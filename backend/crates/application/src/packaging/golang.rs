//! Estrategia de empaquetado para módulos Go: protocolo `GOPROXY`
//! (`go get`, `go mod download`) contra un repositorio `FerroBox`.
//!
//! Cubre **Forge** (subida del zip del módulo, listado, `.info` / `.mod`
//! / `.zip`, `@latest` y yank), **Mirror** (caché *pull-through* de un
//! proxy V3) y lecturas en **Alloy**. Las versiones son inmutables; el
//! yank las oculta de `@v/list` y `@latest` pero el zip sigue
//! descargable.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::ArtifactId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::ArtifactStore;
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::repository_store::RepositoryStore;
use ferrobox_ports::storage::StoragePort;
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

use super::{
    copy_stored_artifact, ensure_quota, notify_assay, PackageSearchHit, PackagingError,
    PackagingStrategy, PromoteOutcome, PublishOutcome,
};
use crate::assay::AssayService;
use crate::content_hash::sha256_checksum;
use crate::quota::QuotaService;
use crate::storage_key::storage_key_for;

/// Estrategia de empaquetado para el ecosistema Go.
pub struct GoPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    http_client: Arc<dyn HttpClient>,
    repository_store: Arc<dyn RepositoryStore>,
    assays: Option<AssayService>,
    quota: Option<QuotaService>,
}

impl GoPackagingStrategy {
    /// Construye la estrategia a partir de sus puertos.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        http_client: Arc<dyn HttpClient>,
        repository_store: Arc<dyn RepositoryStore>,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
            http_client,
            repository_store,
            assays: None,
            quota: None,
        }
    }

    /// Conecta el ensaye automático al publicar o cachear un módulo.
    #[must_use]
    pub fn with_assays(mut self, assays: AssayService) -> Self {
        self.assays = Some(assays);
        self
    }

    /// Aplica la cuota de almacenamiento al publicar o cachear.
    #[must_use]
    pub fn with_quota(mut self, quota: QuotaService) -> Self {
        self.quota = Some(quota);
        self
    }

    fn ensure_go_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Go {
            return Err(PackagingError::EcosystemMismatch {
                expected: "go",
                actual: repository.ecosystem().label(),
            });
        }
        Ok(())
    }

    fn is_read_only(repository: &Repository) -> bool {
        matches!(
            repository.kind(),
            RepositoryKind::Mirror { .. } | RepositoryKind::Alloy { .. }
        )
    }

    fn mirror_upstream(repository: &Repository) -> Option<&Url> {
        match repository.kind() {
            RepositoryKind::Mirror { upstream } => Some(upstream),
            RepositoryKind::Forge | RepositoryKind::Alloy { .. } => None,
        }
    }

    async fn resolve_read_targets(
        &self,
        repository: &Repository,
    ) -> Result<Vec<Repository>, PackagingError> {
        match repository.kind() {
            RepositoryKind::Alloy { members } => {
                let mut targets = Vec::new();
                for member_id in members {
                    let Some(member) = self.repository_store.find_by_id(*member_id).await? else {
                        continue;
                    };
                    if matches!(member.kind(), RepositoryKind::Alloy { .. }) {
                        continue;
                    }
                    if member.ecosystem() != repository.ecosystem() {
                        continue;
                    }
                    targets.push(member);
                }
                Ok(targets)
            }
            _ => Ok(vec![repository.clone()]),
        }
    }

    async fn load_version_entry(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<VersionEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Go, coordinate.name())
            .await?;
        for entry_bytes in entries {
            let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if entry.version == coordinate.version().as_str() {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    async fn load_module_entries(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Vec<VersionEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Go, name)
            .await?;
        let mut versions = Vec::new();
        for entry_bytes in entries {
            versions.push(
                serde_json::from_slice(&entry_bytes)
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
            );
        }
        Ok(versions)
    }

    async fn save_version_entry(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        entry: &VersionEntry,
        artifact_id: Option<ArtifactId>,
    ) -> Result<(), PackagingError> {
        let entry_bytes = Bytes::from(
            serde_json::to_vec(entry).expect("a VersionEntry always serializes to valid JSON"),
        );
        self.package_index_store
            .upsert_entry(repository.id(), coordinate, artifact_id, entry_bytes)
            .await?;
        Ok(())
    }

    async fn collect_module_entries(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Vec<VersionEntry>, PackagingError> {
        let mut versions = Vec::new();
        for target in self.resolve_read_targets(repository).await? {
            versions.extend(self.load_module_entries(&target, name).await?);
        }
        Ok(versions)
    }

    async fn find_version(
        &self,
        repository: &Repository,
        name: &PackageName,
        version: &str,
    ) -> Result<Option<(Repository, VersionEntry)>, PackagingError> {
        let coordinate = go_coordinate(name.as_str(), version)?;
        for target in self.resolve_read_targets(repository).await? {
            if let Some(entry) = self.load_version_entry(&target, &coordinate).await? {
                return Ok(Some((target, entry)));
            }
        }
        Ok(None)
    }

    async fn download_stored(&self, artifact_id: ArtifactId) -> Result<Bytes, PackagingError> {
        Ok(self.storage.get(&storage_key_for(artifact_id)).await?)
    }

    async fn store_module(
        &self,
        repository: &Repository,
        parsed: &ParsedModule,
        body: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        let coordinate = go_coordinate(&parsed.module, &parsed.version)?;
        if self
            .load_version_entry(repository, &coordinate)
            .await?
            .is_some()
        {
            return Err(PackagingError::AlreadyPublished(coordinate));
        }
        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(repository.id(), checksum, body.len() as u64);
        ensure_quota(self.quota.as_ref(), repository.id(), body.len() as u64).await?;
        self.storage
            .put(&storage_key_for(artifact.id()), body)
            .await?;
        self.artifact_store.save(&artifact).await?;
        let entry = VersionEntry {
            module: parsed.module.clone(),
            version: parsed.version.clone(),
            time: parsed.time.clone(),
            go_mod: parsed.go_mod.clone(),
            yanked: false,
            files: vec![FileEntry {
                filename: zip_filename(&parsed.module, &parsed.version),
                artifact_id: artifact.id().to_string(),
                yanked: false,
            }],
        };
        self.save_version_entry(repository, &coordinate, &entry, Some(artifact.id()))
            .await?;
        notify_assay(self.assays.as_ref(), repository.id(), &coordinate);
        Ok(coordinate)
    }

    async fn cache_from_upstream(
        &self,
        repository: &Repository,
        module: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        let Some(upstream) = Self::mirror_upstream(repository) else {
            return Err(PackagingError::PackageNotFound(module.to_string()));
        };
        let url = upstream_url(upstream, &escaped_module_path(module), &format!("@v/{version}.zip"));
        let body = match self.http_client.get(&url).await {
            Ok(response) => response.body,
            Err(HttpClientError::Status { status: 404 | 410, .. }) => {
                return Err(PackagingError::VersionNotFound(go_coordinate(
                    module, version,
                )?));
            }
            Err(error) => return Err(error.into()),
        };
        let parsed = parse_module_zip(&body, Some(version))?;
        self.store_module(repository, &parsed, body.clone()).await?;
        Ok(body)
    }

    async fn zip_bytes(
        &self,
        repository: &Repository,
        module: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        let name = PackageName::parse(module)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        if let Some((_target, entry)) = self.find_version(repository, &name, version).await?
            && let Some(file) = entry.files.first()
            && let Ok(artifact_id) = parse_artifact_id(&file.artifact_id)
        {
            return self.download_stored(artifact_id).await;
        }
        for target in self.resolve_read_targets(repository).await? {
            if Self::mirror_upstream(&target).is_some() {
                match self.cache_from_upstream(&target, module, version).await {
                    Ok(body) => return Ok(body),
                    Err(PackagingError::VersionNotFound(_) | PackagingError::PackageNotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Err(PackagingError::VersionNotFound(go_coordinate(
            module, version,
        )?))
    }

    async fn list_text(
        &self,
        repository: &Repository,
        module: &str,
    ) -> Result<Bytes, PackagingError> {
        let name = PackageName::parse(module)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let mut versions: Vec<String> = self
            .collect_module_entries(repository, &name)
            .await?
            .into_iter()
            .filter(|entry| !entry.yanked)
            .map(|entry| entry.version)
            .collect();
        if versions.is_empty() {
            for target in self.resolve_read_targets(repository).await? {
                if let Some(upstream) = Self::mirror_upstream(&target) {
                    let url = upstream_url(upstream, &escaped_module_path(module), "@v/list");
                    match self.http_client.get(&url).await {
                        Ok(response) => {
                            return Ok(response.body);
                        }
                        Err(HttpClientError::Status { status: 404 | 410, .. }) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            return Err(PackagingError::PackageNotFound(module.to_string()));
        }
        versions.sort_by(|left, right| cmp_go_version(left, right));
        versions.dedup();
        Ok(Bytes::from(format!("{}\n", versions.join("\n"))))
    }

    async fn info_json(
        &self,
        repository: &Repository,
        module: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        let name = PackageName::parse(module)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        if let Some((_target, entry)) = self.find_version(repository, &name, version).await? {
            return Ok(Bytes::from(
                serde_json::to_vec(&json!({
                    "Version": entry.version,
                    "Time": entry.time
                }))
                .expect("info serializes"),
            ));
        }
        let zip = self.zip_bytes(repository, module, version).await?;
        let parsed = parse_module_zip(&zip, Some(version))?;
        Ok(Bytes::from(
            serde_json::to_vec(&json!({
                "Version": parsed.version,
                "Time": parsed.time
            }))
            .expect("info serializes"),
        ))
    }

    async fn go_mod(
        &self,
        repository: &Repository,
        module: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        let name = PackageName::parse(module)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        if let Some((_target, entry)) = self.find_version(repository, &name, version).await? {
            return Ok(Bytes::from(entry.go_mod));
        }
        let zip = self.zip_bytes(repository, module, version).await?;
        let parsed = parse_module_zip(&zip, Some(version))?;
        Ok(Bytes::from(parsed.go_mod))
    }

    async fn latest_info(
        &self,
        repository: &Repository,
        module: &str,
    ) -> Result<Bytes, PackagingError> {
        let list = self.list_text(repository, module).await?;
        let versions: Vec<String> = String::from_utf8_lossy(&list)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        let latest = pick_latest(&versions)
            .ok_or_else(|| PackagingError::PackageNotFound(module.to_string()))?;
        self.info_json(repository, module, latest).await
    }
}

#[async_trait]
impl PackagingStrategy for GoPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Go
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_go_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let parsed = parse_module_zip(&payload, None)?;
        self.store_module(repository, &parsed, payload).await
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_go_repository(repository)?;
        self.list_text(repository, name.as_str()).await
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_go_repository(repository)?;
        self.zip_bytes(
            repository,
            coordinate.name().as_str(),
            coordinate.version().as_str(),
        )
        .await
    }

    async fn get_protocol_file(
        &self,
        repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_go_repository(repository)?;
        match parse_go_path(path)? {
            GoResource::List { module } => self.list_text(repository, &module).await,
            GoResource::Latest { module } => self.latest_info(repository, &module).await,
            GoResource::Info { module, version } => {
                self.info_json(repository, &module, &version).await
            }
            GoResource::Mod { module, version } => self.go_mod(repository, &module, &version).await,
            GoResource::Zip { module, version } => {
                self.zip_bytes(repository, &module, &version).await
            }
        }
    }

    async fn put_protocol_file(
        &self,
        repository: &Repository,
        path: &str,
        body: Bytes,
    ) -> Result<(), PackagingError> {
        Self::ensure_go_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let GoResource::Zip { module, version } = parse_go_path(path)? else {
            return Err(PackagingError::InvalidPayload(
                "only PUT of module@version.zip is accepted".to_string(),
            ));
        };
        let parsed = parse_module_zip(&body, Some(&version))?;
        if parsed.module != module {
            return Err(PackagingError::InvalidPayload(format!(
                "zip module '{}' does not match path '{module}'",
                parsed.module
            )));
        }
        self.store_module(repository, &parsed, body).await?;
        Ok(())
    }

    async fn protocol_metadata(
        &self,
        repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        self.get_protocol_file(repository, path).await
    }

    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError> {
        Self::ensure_go_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let mut entry = self
            .load_version_entry(repository, coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
        entry.yanked = yanked;
        for file in &mut entry.files {
            file.yanked = yanked;
        }
        let last_id = entry
            .files
            .iter()
            .rev()
            .find_map(|file| parse_artifact_id(&file.artifact_id).ok());
        self.save_version_entry(repository, coordinate, &entry, last_id)
            .await
    }

    async fn promote_version(
        &self,
        source: &Repository,
        target: &Repository,
        coordinate: &PackageCoordinate,
        preserve_yanked: bool,
    ) -> Result<PromoteOutcome, PackagingError> {
        Self::ensure_go_repository(source)?;
        Self::ensure_go_repository(target)?;
        if Self::is_read_only(target) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let mut entry = self
            .load_version_entry(source, coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
        if entry.files.is_empty() {
            return Err(PackagingError::VersionNotFound(coordinate.clone()));
        }
        if self
            .load_version_entry(target, coordinate)
            .await?
            .is_some()
        {
            return Err(PackagingError::AlreadyPublished(coordinate.clone()));
        }
        if !preserve_yanked {
            entry.yanked = false;
            for file in &mut entry.files {
                file.yanked = false;
            }
        }
        let mut artifacts_copied = 0;
        let mut bytes_copied = 0;
        for file in &mut entry.files {
            let source_id = parse_artifact_id(&file.artifact_id)?;
            let (copied_id, size) = copy_stored_artifact(
                self.artifact_store.as_ref(),
                self.storage.as_ref(),
                self.quota.as_ref(),
                source_id,
                target.id(),
            )
            .await?;
            file.artifact_id = copied_id.to_string();
            artifacts_copied += 1;
            bytes_copied += size;
        }
        let last_id = entry
            .files
            .iter()
            .rev()
            .find_map(|file| parse_artifact_id(&file.artifact_id).ok());
        self.save_version_entry(target, coordinate, &entry, last_id)
            .await?;
        notify_assay(self.assays.as_ref(), target.id(), coordinate);
        Ok(PromoteOutcome {
            coordinate: coordinate.clone(),
            artifacts_copied,
            bytes_copied,
        })
    }

    async fn search(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        Self::ensure_go_repository(repository)?;
        let needle = query.trim().to_ascii_lowercase();
        let mut hits = BTreeMap::new();
        for target in self.resolve_read_targets(repository).await? {
            let entries = self
                .package_index_store
                .entries_for_repository(target.id(), PackageEcosystem::Go)
                .await?;
            for entry_bytes in entries {
                let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
                if entry.yanked {
                    continue;
                }
                if !needle.is_empty() && !entry.module.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                hits.entry(entry.module).or_insert(entry.version);
            }
        }
        Ok(hits
            .into_iter()
            .take(limit)
            .map(|(name, max_version)| PackageSearchHit { name, max_version })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VersionEntry {
    module: String,
    version: String,
    #[serde(default)]
    time: String,
    #[serde(default)]
    go_mod: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FileEntry {
    filename: String,
    artifact_id: String,
    #[serde(default)]
    yanked: bool,
}

struct ParsedModule {
    module: String,
    version: String,
    time: String,
    go_mod: String,
}

#[derive(Debug)]
enum GoResource {
    List { module: String },
    Latest { module: String },
    Info { module: String, version: String },
    Mod { module: String, version: String },
    Zip { module: String, version: String },
}

fn go_coordinate(module: &str, version: &str) -> Result<PackageCoordinate, PackagingError> {
    Ok(PackageCoordinate::new(
        PackageEcosystem::Go,
        PackageName::parse(module).map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
        PackageVersion::parse(version)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
    ))
}

fn zip_filename(module: &str, version: &str) -> String {
    format!("{module}@{version}.zip")
}

fn parse_artifact_id(value: &str) -> Result<ArtifactId, PackagingError> {
    Uuid::parse_str(value)
        .map(ArtifactId::from)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))
}

fn upstream_url(upstream: &Url, escaped_module: &str, suffix: &str) -> String {
    let base = upstream.as_str().trim_end_matches('/');
    format!("{base}/{escaped_module}/{suffix}")
}

/// Codifica mayúsculas como `!` + minúscula (`Azure` → `!azure`).
#[must_use]
pub fn escape_module_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn escaped_module_path(path: &str) -> String {
    escape_module_path(path)
}

/// Decodifica un módulo o versión del protocolo `GOPROXY`.
///
/// # Errors
///
/// Devuelve [`PackagingError::InvalidPayload`] si un `!` no va seguido
/// de una letra minúscula.
pub fn unescape_module_path(path: &str) -> Result<String, PackagingError> {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars();
    while let Some(ch) = chars.next() {
        if ch == '!' {
            let next = chars.next().ok_or_else(|| {
                PackagingError::InvalidPayload("escaped path ends with '!'".to_string())
            })?;
            if !next.is_ascii_lowercase() {
                return Err(PackagingError::InvalidPayload(
                    "escaped path has '!' not followed by a lowercase letter".to_string(),
                ));
            }
            out.push(next.to_ascii_uppercase());
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

fn parse_go_path(path: &str) -> Result<GoResource, PackagingError> {
    let trimmed = path.trim_matches('/');
    if let Some(module) = trimmed.strip_suffix("/@latest") {
        return Ok(GoResource::Latest {
            module: unescape_module_path(module)?,
        });
    }
    if let Some(module) = trimmed.strip_suffix("/@v/list") {
        return Ok(GoResource::List {
            module: unescape_module_path(module)?,
        });
    }
    let Some((module, rest)) = trimmed.split_once("/@v/") else {
        return Err(PackagingError::FileNotFound(path.to_string()));
    };
    let module = unescape_module_path(module)?;
    if let Some(version) = rest.strip_suffix(".zip") {
        return Ok(GoResource::Zip {
            module,
            version: unescape_module_path(version)?,
        });
    }
    if let Some(version) = rest.strip_suffix(".mod") {
        return Ok(GoResource::Mod {
            module,
            version: unescape_module_path(version)?,
        });
    }
    if let Some(version) = rest.strip_suffix(".info") {
        return Ok(GoResource::Info {
            module,
            version: unescape_module_path(version)?,
        });
    }
    Err(PackagingError::FileNotFound(path.to_string()))
}

fn parse_module_zip(
    body: &[u8],
    expected_version: Option<&str>,
) -> Result<ParsedModule, PackagingError> {
    let mut archive = ZipArchive::new(Cursor::new(body))
        .map_err(|err| PackagingError::InvalidPayload(format!("module zip is invalid: {err}")))?;
    let mut go_mod = None;
    let mut inferred_version = None;
    let mut inferred_module = None;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let name = file.name().to_string();
        if !name.ends_with("go.mod") || name.contains("/testdata/") {
            continue;
        }
        let mut buf = String::new();
        file.read_to_string(&mut buf)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        if let Some((prefix, _)) = name.rsplit_once("/go.mod")
            && let Some((module, version)) = prefix.rsplit_once('@')
        {
            inferred_module = Some(module.trim_matches('/').to_string());
            inferred_version = Some(version.to_string());
        }
        go_mod = Some(buf);
        break;
    }
    let go_mod = go_mod.ok_or_else(|| {
        PackagingError::InvalidPayload("module zip does not contain go.mod".to_string())
    })?;
    let module = parse_module_directive(&go_mod)
        .or(inferred_module)
        .ok_or_else(|| PackagingError::InvalidPayload("go.mod is missing module".to_string()))?;
    let version = expected_version
        .map(ToOwned::to_owned)
        .or(inferred_version)
        .ok_or_else(|| {
            PackagingError::InvalidPayload("cannot infer module version from zip".to_string())
        })?;
    validate_module_path(&module)?;
    validate_go_version(&version)?;
    Ok(ParsedModule {
        module,
        version,
        time: chrono::Utc::now().to_rfc3339(),
        go_mod,
    })
}

fn parse_module_directive(go_mod: &str) -> Option<String> {
    for line in go_mod.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("module ") {
            let value = rest.trim().trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn validate_module_path(module: &str) -> Result<(), PackagingError> {
    if module.is_empty() || module.len() > 214 {
        return Err(PackagingError::InvalidPayload(
            "module path must be 1-214 characters".to_string(),
        ));
    }
    if module.starts_with('/') || module.ends_with('/') || module.contains("//") {
        return Err(PackagingError::InvalidPayload(
            "module path is not a valid import path".to_string(),
        ));
    }
    if !module
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '/' | '-' | '_' | '~'))
    {
        return Err(PackagingError::InvalidPayload(
            "module path contains an invalid character".to_string(),
        ));
    }
    Ok(())
}

fn validate_go_version(version: &str) -> Result<(), PackagingError> {
    if !version.starts_with('v') {
        return Err(PackagingError::InvalidPayload(
            "Go module versions must start with 'v'".to_string(),
        ));
    }
    Ok(())
}

/// `true` si la versión Go es una prerelease (`v1.0.0-rc.1`).
#[must_use]
pub fn is_prerelease_version(version: &str) -> bool {
    let core = version
        .strip_suffix("+incompatible")
        .unwrap_or(version)
        .split_once('+')
        .map_or(version, |(core, _)| core);
    core.contains('-')
}

fn pick_latest(versions: &[String]) -> Option<&str> {
    let releases: Vec<&str> = versions
        .iter()
        .map(String::as_str)
        .filter(|version| !is_prerelease_version(version))
        .collect();
    let pool = if releases.is_empty() {
        versions.iter().map(String::as_str).collect()
    } else {
        releases
    };
    pool.into_iter().max_by(|left, right| cmp_go_version(left, right))
}

fn cmp_go_version(left: &str, right: &str) -> std::cmp::Ordering {
    version_key(left).cmp(&version_key(right))
}

fn version_key(version: &str) -> (u64, u64, u64, bool, String) {
    let without_v = version.strip_prefix('v').unwrap_or(version);
    let trimmed = without_v
        .strip_suffix("+incompatible")
        .unwrap_or(without_v);
    let (core, pre) = trimmed.split_once('-').unwrap_or((trimmed, ""));
    let mut parts = core.split('.');
    let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    (major, minor, patch, pre.is_empty(), pre.to_string())
}

/// Construye un zip de módulo mínimo para pruebas.
///
/// # Panics
///
/// Entra en pánico si no se puede escribir el zip en memoria.
#[must_use]
pub fn build_module_zip(module: &str, version: &str) -> Bytes {
    let prefix = format!("{module}@{version}");
    let go_mod = format!("module {module}\n\ngo 1.22\n");
    let source = format!("package hello\n\nfunc Version() string {{ return \"{version}\" }}\n");
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(format!("{prefix}/go.mod"), options)
            .expect("go.mod entry");
        zip.write_all(go_mod.as_bytes()).expect("go.mod bytes");
        zip.start_file(format!("{prefix}/hello.go"), options)
            .expect("hello.go entry");
        zip.write_all(source.as_bytes()).expect("hello.go bytes");
        zip.finish().expect("module zip");
    }
    Bytes::from(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::repository::RepositoryName;

    use super::*;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn strategy() -> GoPackagingStrategy {
        GoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        )
    }

    fn go_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Go,
        )
        .unwrap()
    }

    #[test]
    fn escapes_uppercase_module_paths() {
        assert_eq!(
            escape_module_path("github.com/Azure/go-autorest"),
            "github.com/!azure/go-autorest"
        );
        assert_eq!(
            unescape_module_path("github.com/!azure/go-autorest").unwrap(),
            "github.com/Azure/go-autorest"
        );
    }

    #[test]
    fn parses_proxy_paths() {
        match parse_go_path("github.com/example/hello/@v/list").unwrap() {
            GoResource::List { module } => assert_eq!(module, "github.com/example/hello"),
            other => panic!("unexpected {other:?}"),
        }
        match parse_go_path("github.com/!azure/go-autorest/@v/v1.0.0.zip").unwrap() {
            GoResource::Zip { module, version } => {
                assert_eq!(module, "github.com/Azure/go-autorest");
                assert_eq!(version, "v1.0.0");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn latest_prefers_releases() {
        let versions = ["v1.0.0-rc.1", "v1.0.0", "v0.9.0"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(pick_latest(&versions), Some("v1.0.0"));
    }

    #[tokio::test]
    async fn publish_then_list_mod_and_zip() {
        let strategy = strategy();
        let repository = go_forge("go-local");
        strategy
            .put_protocol_file(
                &repository,
                "github.com/example/hello/@v/v1.0.0.zip",
                build_module_zip("github.com/example/hello", "v1.0.0"),
            )
            .await
            .unwrap();
        let list = strategy
            .get_protocol_file(&repository, "github.com/example/hello/@v/list")
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&list).unwrap().trim(), "v1.0.0");
        let go_mod = strategy
            .get_protocol_file(&repository, "github.com/example/hello/@v/v1.0.0.mod")
            .await
            .unwrap();
        assert!(std::str::from_utf8(&go_mod).unwrap().contains("module github.com/example/hello"));
        let zip = strategy
            .get_protocol_file(&repository, "github.com/example/hello/@v/v1.0.0.zip")
            .await
            .unwrap();
        assert_eq!(
            parse_module_zip(&zip, Some("v1.0.0")).unwrap().module,
            "github.com/example/hello"
        );
        let latest = strategy
            .get_protocol_file(&repository, "github.com/example/hello/@latest")
            .await
            .unwrap();
        let info: serde_json::Value = serde_json::from_slice(&latest).unwrap();
        assert_eq!(info["Version"], "v1.0.0");
    }

    #[tokio::test]
    async fn rejects_overwrite_and_omits_yanked_from_list() {
        let strategy = strategy();
        let repository = go_forge("go-local");
        strategy
            .publish(
                &repository,
                build_module_zip("github.com/example/hello", "v1.0.0"),
            )
            .await
            .unwrap();
        let err = strategy
            .publish(
                &repository,
                build_module_zip("github.com/example/hello", "v1.0.0"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::AlreadyPublished(_)));
        let coordinate = go_coordinate("github.com/example/hello", "v1.0.0").unwrap();
        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();
        let list = strategy
            .index(&repository, coordinate.name())
            .await
            .unwrap_err();
        assert!(matches!(list, PackagingError::PackageNotFound(_)));
        let zip = strategy.download(&repository, &coordinate).await.unwrap();
        assert!(!zip.is_empty());
    }

    #[tokio::test]
    async fn mirror_caches_upstream_zip() {
        let zip = build_module_zip("rsc.io/quote", "v1.5.2");
        let http = InMemoryHttpClient::default();
        http.stub(
            "https://proxy.golang.org/rsc.io/quote/@v/v1.5.2.zip",
            200,
            zip.clone(),
        );
        let strategy = GoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(http),
            Arc::new(InMemoryRepositoryStore::default()),
        );
        let repository = Repository::new(
            RepositoryName::parse("go-proxy").unwrap(),
            RepositoryKind::Mirror {
                upstream: Url::parse("https://proxy.golang.org").unwrap(),
            },
            PackageEcosystem::Go,
        )
        .unwrap();
        let body = strategy
            .get_protocol_file(&repository, "rsc.io/quote/@v/v1.5.2.zip")
            .await
            .unwrap();
        assert_eq!(parse_module_zip(&body, Some("v1.5.2")).unwrap().module, "rsc.io/quote");
    }

    #[tokio::test]
    async fn alloy_unions_search_and_reads_first_member() {
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let strategy = GoPackagingStrategy::new(
            artifacts,
            index,
            storage,
            Arc::new(InMemoryHttpClient::default()),
            repos.clone(),
        );
        let first = go_forge("go-one");
        let second = go_forge("go-two");
        repos.save(&first).await.unwrap();
        repos.save(&second).await.unwrap();
        strategy
            .publish(
                &first,
                build_module_zip("github.com/example/hello", "v1.0.0"),
            )
            .await
            .unwrap();
        strategy
            .publish(&second, build_module_zip("rsc.io/quote", "v1.5.2"))
            .await
            .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("go-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first.id(), second.id()],
            },
            PackageEcosystem::Go,
        )
        .unwrap();
        let hits = strategy.search(&alloy, "", 10).await.unwrap();
        assert_eq!(hits.len(), 2);
        let zip = strategy
            .get_protocol_file(&alloy, "github.com/example/hello/@v/v1.0.0.zip")
            .await
            .unwrap();
        assert_eq!(
            parse_module_zip(&zip, Some("v1.0.0")).unwrap().module,
            "github.com/example/hello"
        );
    }

    #[tokio::test]
    async fn promote_copies_the_zip() {
        let strategy = strategy();
        let source = go_forge("go-dev");
        let target = go_forge("go-prod");
        strategy
            .publish(
                &source,
                build_module_zip("github.com/example/hello", "v1.0.0"),
            )
            .await
            .unwrap();
        let coordinate = go_coordinate("github.com/example/hello", "v1.0.0").unwrap();
        let outcome = strategy
            .promote_version(&source, &target, &coordinate, false)
            .await
            .unwrap();
        assert_eq!(outcome.artifacts_copied, 1);
        let copied = strategy.download(&target, &coordinate).await.unwrap();
        assert_eq!(
            parse_module_zip(&copied, Some("v1.0.0")).unwrap().module,
            "github.com/example/hello"
        );
    }
}
