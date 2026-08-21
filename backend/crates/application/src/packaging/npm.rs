//! Estrategia de empaquetado para el ecosistema npm: implementa el
//! subconjunto del protocolo de registro que `npm publish` y `npm install`
//! necesitan para publicar paquetes y resolver dependencias contra un
//! repositorio `FerroBox`.
//!
//! Referencia: <https://github.com/npm/registry/blob/main/docs/REGISTRY-API.md>.
//!
//! Cubre **Forge** (publicar, packument, tarball y yank/deprecate),
//! **Mirror** (caché *pull-through* de un registro npm como
//! registry.npmjs.org) y lecturas en **Alloy** (unión de miembros).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::ArtifactStore;
use ferrobox_ports::http_client::HttpClient;
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::repository_store::RepositoryStore;
use ferrobox_ports::storage::StoragePort;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    copy_stored_artifact, ensure_quota, notify_assay, PackageSearchHit, PackagingError,
    PackagingStrategy, PromoteOutcome, PublishOutcome,
};
use crate::assay::AssayService;
use crate::quota::QuotaService;
use crate::content_hash::sha256_checksum;
use crate::storage_key::storage_key_for;

/// Estrategia de empaquetado para el ecosistema npm.
pub struct NpmPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    http_client: Arc<dyn HttpClient>,
    repository_store: Arc<dyn RepositoryStore>,
    public_base_url: String,
    assays: Option<AssayService>,
    quota: Option<QuotaService>,
}

impl NpmPackagingStrategy {
    /// Construye la estrategia a partir de sus puertos y de la URL
    /// pública con la que se rellenan los `dist.tarball` del packument.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        http_client: Arc<dyn HttpClient>,
        repository_store: Arc<dyn RepositoryStore>,
        public_base_url: String,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
            http_client,
            repository_store,
            public_base_url,
            assays: None,
            quota: None,
        }
    }

    /// Conecta el ensaye automático al publicar o cachear un tarball.
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

    fn ensure_npm_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Npm {
            return Err(PackagingError::EcosystemMismatch {
                expected: "npm",
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

    fn mirror_upstream(repository: &Repository) -> Option<&url::Url> {
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

    fn tarball_url(&self, repository: &Repository, name: &str, version: &str) -> String {
        let file = tarball_filename(name, version);
        let encoded_name = encode_npm_name(name);
        format!(
            "{}/npm/{}/{encoded_name}/-/{file}",
            self.public_base_url.trim_end_matches('/'),
            repository.id()
        )
    }

    async fn find_local_entry(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<VersionEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Npm, coordinate.name())
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

    async fn refresh_packument_from_upstream(
        &self,
        repository: &Repository,
        upstream: &url::Url,
        name: &PackageName,
    ) -> Result<(), PackagingError> {
        let packument_url = join_upstream(upstream, &encode_npm_name(name.as_str()));
        let response = self.http_client.get(&packument_url).await?;
        let packument: Value = serde_json::from_slice(&response.body)
            .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;

        let Some(Value::Object(versions)) = packument.get("versions") else {
            return Err(PackagingError::InvalidUpstream(format!(
                "upstream packument for '{name}' is missing a versions object"
            )));
        };

        let dist_tags = parse_dist_tags(packument.get("dist-tags"));

        for (version_str, manifest) in versions {
            let Ok(version) = PackageVersion::parse(version_str.clone()) else {
                continue;
            };
            let mut entry =
                version_entry_from_upstream_manifest(name.as_str(), version_str, manifest)?;
            entry.dist_tags.clone_from(&dist_tags);
            let coordinate =
                PackageCoordinate::new(PackageEcosystem::Npm, name.clone(), version);
            let entry_bytes = Bytes::from(
                serde_json::to_vec(&entry)
                    .expect("a VersionEntry always serializes to valid JSON"),
            );
            self.package_index_store
                .upsert_entry(repository.id(), &coordinate, None, entry_bytes)
                .await?;
        }

        Ok(())
    }

    async fn cache_tarball_from_upstream(
        &self,
        repository: &Repository,
        upstream: &url::Url,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Npm, coordinate.name())
            .await?;

        if entries.is_empty() {
            self.refresh_packument_from_upstream(repository, upstream, coordinate.name())
                .await?;
        }

        let entry = self
            .find_local_entry(repository, coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;

        let tarball_url = entry
            .manifest
            .get("dist")
            .and_then(|dist| dist.get("tarball"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                PackagingError::InvalidUpstream(format!(
                    "upstream packument for {coordinate} is missing dist.tarball"
                ))
            })?;

        let response = self.http_client.get(tarball_url).await?;
        let tarball = response.body;

        if !entry.shasum.is_empty() {
            let actual = sha1_hex(&tarball);
            if actual != entry.shasum {
                return Err(PackagingError::InvalidUpstream(format!(
                    "shasum mismatch for {coordinate}: expected {}, got {actual}",
                    entry.shasum
                )));
            }
        }
        if entry.integrity.starts_with("sha512-") {
            let actual = sha512_integrity(&tarball);
            if actual != entry.integrity {
                return Err(PackagingError::InvalidUpstream(format!(
                    "integrity mismatch for {coordinate}: expected {}, got {actual}",
                    entry.integrity
                )));
            }
        }

        let checksum = sha256_checksum(&tarball);
        let artifact = Artifact::new(repository.id(), checksum, tarball.len() as u64);

        ensure_quota(self.quota.as_ref(), repository.id(), tarball.len() as u64).await?;
        self.storage
            .put(&storage_key_for(artifact.id()), tarball.clone())
            .await?;
        self.artifact_store.save(&artifact).await?;

        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("a VersionEntry always serializes to valid JSON"),
        );
        self.package_index_store
            .upsert_entry(
                repository.id(),
                coordinate,
                Some(artifact.id()),
                entry_bytes,
            )
            .await?;
        notify_assay(self.assays.as_ref(), repository.id(), coordinate);

        Ok(tarball)
    }

    async fn index_one(
        &self,
        serving: &Repository,
        source: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        if let Some(upstream) = Self::mirror_upstream(source) {
            self.refresh_packument_from_upstream(source, upstream, name)
                .await?;
        }

        let entries = self
            .package_index_store
            .entries_for_package(source.id(), PackageEcosystem::Npm, name)
            .await?;

        if entries.is_empty() {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        }

        packument_from_entries(
            name.as_str(),
            &entries,
            |version| self.tarball_url(serving, name.as_str(), version),
        )
    }

    async fn download_one(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        if let Some(artifact_id) = self
            .package_index_store
            .artifact_for(repository.id(), coordinate)
            .await?
        {
            let artifact = self
                .artifact_store
                .find_by_id(artifact_id)
                .await?
                .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;

            let content = self.storage.get(&storage_key_for(artifact_id)).await?;
            let actual = sha256_checksum(&content);
            if actual != *artifact.checksum() {
                return Err(PackagingError::ChecksumMismatch {
                    expected: artifact.checksum().to_string(),
                    actual: actual.to_string(),
                });
            }

            return Ok(content);
        }

        if let Some(upstream) = Self::mirror_upstream(repository) {
            return self
                .cache_tarball_from_upstream(repository, upstream, coordinate)
                .await;
        }

        Err(PackagingError::VersionNotFound(coordinate.clone()))
    }

    async fn search_one(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let entries = self
            .package_index_store
            .entries_for_repository(repository.id(), PackageEcosystem::Npm)
            .await?;

        let needle = query.to_ascii_lowercase();
        let mut by_name: BTreeMap<String, Vec<VersionEntry>> = BTreeMap::new();
        for entry_bytes in entries {
            let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if !entry.name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            by_name.entry(entry.name.clone()).or_default().push(entry);
        }

        let mut hits = Vec::new();
        for (name, versions) in by_name {
            let max_version = advertised_version(&versions);
            hits.push(PackageSearchHit { name, max_version });
            if hits.len() >= limit {
                break;
            }
        }

        Ok(hits)
    }
}

#[async_trait]
impl PackagingStrategy for NpmPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Npm
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_npm_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }

        let parsed = parse_publish_payload(&payload)?;
        let name = PackageName::parse(parsed.name.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let version = PackageVersion::parse(parsed.version.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let coordinate = PackageCoordinate::new(PackageEcosystem::Npm, name, version);

        if self
            .package_index_store
            .artifact_for(repository.id(), &coordinate)
            .await?
            .is_some()
        {
            return Err(PackagingError::AlreadyPublished(coordinate));
        }

        let checksum = sha256_checksum(&parsed.tarball);
        let artifact = Artifact::new(
            repository.id(),
            checksum,
            parsed.tarball.len() as u64,
        );

        ensure_quota(
            self.quota.as_ref(),
            repository.id(),
            parsed.tarball.len() as u64,
        )
        .await?;
        self.storage
            .put(&storage_key_for(artifact.id()), parsed.tarball.clone())
            .await?;
        self.artifact_store.save(&artifact).await?;

        let entry = VersionEntry {
            name: parsed.name,
            version: parsed.version,
            shasum: sha1_hex(&parsed.tarball),
            integrity: sha512_integrity(&parsed.tarball),
            yanked: false,
            dist_tags: parsed.dist_tags,
            manifest: parsed.manifest,
        };
        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("a VersionEntry always serializes to valid JSON"),
        );

        self.package_index_store
            .upsert_entry(
                repository.id(),
                &coordinate,
                Some(artifact.id()),
                entry_bytes,
            )
            .await?;
        notify_assay(self.assays.as_ref(), repository.id(), &coordinate);

        Ok(coordinate)
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_npm_repository(repository)?;

        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut documents = Vec::new();
            let mut other_error = None;
            for target in &targets {
                match self.index_one(repository, target, name).await {
                    Ok(document) => documents.push(document),
                    Err(PackagingError::PackageNotFound(_)) => {}
                    Err(error) => {
                        if other_error.is_none() {
                            other_error = Some(error);
                        }
                    }
                }
            }
            if documents.is_empty() {
                return Err(other_error
                    .unwrap_or_else(|| PackagingError::PackageNotFound(name.to_string())));
            }
            return merge_packuments(&documents);
        }

        self.index_one(repository, repository, name).await
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_npm_repository(repository)?;

        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut last_error = PackagingError::VersionNotFound(coordinate.clone());
            for target in &targets {
                match self.download_one(target, coordinate).await {
                    Ok(bytes) => return Ok(bytes),
                    Err(PackagingError::VersionNotFound(_) | PackagingError::PackageNotFound(_)) => {
                    }
                    Err(error) => last_error = error,
                }
            }
            return Err(last_error);
        }

        self.download_one(repository, coordinate).await
    }

    async fn promote_version(
        &self,
        source: &Repository,
        target: &Repository,
        coordinate: &PackageCoordinate,
        preserve_yanked: bool,
    ) -> Result<PromoteOutcome, PackagingError> {
        Self::ensure_npm_repository(source)?;
        Self::ensure_npm_repository(target)?;
        if Self::is_read_only(target) {
            return Err(PackagingError::ReadOnlyRepository);
        }

        let mut entry = self
            .find_local_entry(source, coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
        let source_artifact_id = self
            .package_index_store
            .artifact_for(source.id(), coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;

        if self
            .package_index_store
            .artifact_for(target.id(), coordinate)
            .await?
            .is_some()
        {
            return Err(PackagingError::AlreadyPublished(coordinate.clone()));
        }

        if !preserve_yanked {
            entry.yanked = false;
        }
        entry.dist_tags.clear();

        let (artifact_id, bytes_copied) = copy_stored_artifact(
            self.artifact_store.as_ref(),
            self.storage.as_ref(),
            self.quota.as_ref(),
            source_artifact_id,
            target.id(),
        )
        .await?;
        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("a VersionEntry always serializes to valid JSON"),
        );
        self.package_index_store
            .upsert_entry(target.id(), coordinate, Some(artifact_id), entry_bytes)
            .await?;
        notify_assay(self.assays.as_ref(), target.id(), coordinate);

        Ok(PromoteOutcome {
            coordinate: coordinate.clone(),
            artifacts_copied: 1,
            bytes_copied,
        })
    }

    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError> {
        Self::ensure_npm_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }

        let Some(mut entry) = self.find_local_entry(repository, coordinate).await? else {
            return Err(PackagingError::VersionNotFound(coordinate.clone()));
        };

        if entry.yanked == yanked {
            return Ok(());
        }

        entry.yanked = yanked;
        let artifact_id = self
            .package_index_store
            .artifact_for(repository.id(), coordinate)
            .await?;
        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("a VersionEntry always serializes to valid JSON"),
        );
        self.package_index_store
            .upsert_entry(repository.id(), coordinate, artifact_id, entry_bytes)
            .await?;

        Ok(())
    }

    async fn search(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        Self::ensure_npm_repository(repository)?;

        if limit == 0 {
            return Ok(Vec::new());
        }

        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut merged = Vec::new();
            let mut seen = HashSet::new();
            for target in &targets {
                for hit in self.search_one(target, query, usize::MAX).await? {
                    if seen.insert(hit.name.clone()) {
                        merged.push(hit);
                    }
                }
            }
            merged.truncate(limit);
            return Ok(merged);
        }

        self.search_one(repository, query, limit).await
    }
}

/// Codifica un nombre npm para usarlo en una URL de packument/tarball
/// (`@scope/pkg` → `%40scope%2Fpkg`).
#[must_use]
pub fn encode_npm_name(name: &str) -> String {
    name.replace('@', "%40").replace('/', "%2F")
}

/// Nombre del fichero `.tgz` que npm pide en `/{name}/-/{file}`.
#[must_use]
pub fn tarball_filename(name: &str, version: &str) -> String {
    let unscoped = name.rsplit('/').next().unwrap_or(name);
    format!("{unscoped}-{version}.tgz")
}

fn join_upstream(upstream: &url::Url, relative: &str) -> String {
    let base = upstream.as_str().trim_end_matches('/');
    let relative = relative.trim_start_matches('/');
    format!("{base}/{relative}")
}

fn version_entry_from_upstream_manifest(
    name: &str,
    version: &str,
    manifest: &Value,
) -> Result<VersionEntry, PackagingError> {
    let dist = manifest.get("dist").ok_or_else(|| {
        PackagingError::InvalidUpstream(format!(
            "upstream version {name}@{version} is missing dist"
        ))
    })?;
    let shasum = dist
        .get("shasum")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let integrity = dist
        .get("integrity")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if shasum.is_empty() && integrity.is_empty() {
        return Err(PackagingError::InvalidUpstream(format!(
            "upstream version {name}@{version} is missing dist.shasum and dist.integrity"
        )));
    }
    if dist.get("tarball").and_then(Value::as_str).is_none() {
        return Err(PackagingError::InvalidUpstream(format!(
            "upstream version {name}@{version} is missing dist.tarball"
        )));
    }

    Ok(VersionEntry {
        name: name.to_string(),
        version: version.to_string(),
        shasum,
        integrity,
        yanked: manifest.get("deprecated").is_some(),
        dist_tags: BTreeMap::new(),
        manifest: manifest.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VersionEntry {
    name: String,
    version: String,
    shasum: String,
    integrity: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    dist_tags: BTreeMap<String, String>,
    #[serde(default)]
    manifest: Value,
}

#[derive(Debug, Deserialize)]
struct NpmPublishBody {
    name: String,
    #[serde(rename = "dist-tags", default)]
    dist_tags: BTreeMap<String, String>,
    #[serde(default)]
    versions: BTreeMap<String, Value>,
    #[serde(rename = "_attachments", default)]
    attachments: BTreeMap<String, NpmAttachment>,
}

#[derive(Debug, Deserialize)]
struct NpmAttachment {
    data: String,
}

struct ParsedPublish {
    name: String,
    version: String,
    tarball: Bytes,
    dist_tags: BTreeMap<String, String>,
    manifest: Value,
}

fn parse_publish_payload(payload: &[u8]) -> Result<ParsedPublish, PackagingError> {
    let body: NpmPublishBody = serde_json::from_slice(payload)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;

    if body.name.is_empty() {
        return Err(PackagingError::InvalidPayload(
            "publish body is missing package name".to_string(),
        ));
    }

    let version = body
        .dist_tags
        .get("latest")
        .cloned()
        .or_else(|| body.versions.keys().next().cloned())
        .ok_or_else(|| {
            PackagingError::InvalidPayload("publish body has no versions".to_string())
        })?;

    let attachment = body.attachments.into_values().next().ok_or_else(|| {
        PackagingError::InvalidPayload("publish body has no tarball attachment".to_string())
    })?;

    let compact: String = attachment
        .data
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let tarball = BASE64
        .decode(compact.as_bytes())
        .map_err(|err| PackagingError::InvalidPayload(format!("invalid tarball encoding: {err}")))?;

    let manifest = body
        .versions
        .get(&version)
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));

    Ok(ParsedPublish {
        name: body.name,
        version,
        tarball: Bytes::from(tarball),
        dist_tags: body.dist_tags,
        manifest,
    })
}

fn packument_from_entries(
    name: &str,
    entries: &[Bytes],
    tarball_for: impl Fn(&str) -> String,
) -> Result<Bytes, PackagingError> {
    let mut versions = Map::new();
    let mut parsed = Vec::new();

    for entry_bytes in entries {
        let entry: VersionEntry = serde_json::from_slice(entry_bytes)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        versions.insert(entry.version.clone(), version_document(&entry, &tarball_for));
        parsed.push(entry);
    }

    if versions.is_empty() {
        return Err(PackagingError::PackageNotFound(name.to_string()));
    }

    let dist_tags = resolve_dist_tags(&parsed, &versions);
    Ok(packument_document(name, &dist_tags, &versions))
}

fn version_document(entry: &VersionEntry, tarball_for: &impl Fn(&str) -> String) -> Value {
    let mut document = match &entry.manifest {
        Value::Object(map) => Value::Object(map.clone()),
        _ => Value::Object(Map::new()),
    };
    let object = document
        .as_object_mut()
        .expect("version document is an object");
    object.insert("name".to_string(), Value::String(entry.name.clone()));
    object.insert("version".to_string(), Value::String(entry.version.clone()));
    object.insert(
        "dist".to_string(),
        serde_json::json!({
            "tarball": tarball_for(&entry.version),
            "shasum": entry.shasum,
            "integrity": entry.integrity,
        }),
    );
    if entry.yanked {
        object.insert(
            "deprecated".to_string(),
            Value::String("yanked".to_string()),
        );
    }
    document
}

fn merge_packuments(documents: &[Bytes]) -> Result<Bytes, PackagingError> {
    let mut versions = Map::new();
    let mut dist_tags = BTreeMap::new();
    let mut name = String::new();

    for document in documents {
        let packument: Value = serde_json::from_slice(document)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        if name.is_empty()
            && let Some(found) = packument.get("name").and_then(Value::as_str)
        {
            name = found.to_string();
        }
        if let Some(Value::Object(tags)) = packument.get("dist-tags") {
            for (tag, value) in tags {
                if let Some(version) = value.as_str() {
                    dist_tags
                        .entry(tag.clone())
                        .or_insert_with(|| version.to_string());
                }
            }
        }
        let Some(Value::Object(map)) = packument.get("versions") else {
            continue;
        };
        for (version, body) in map {
            versions.entry(version.clone()).or_insert_with(|| body.clone());
        }
    }

    if versions.is_empty() {
        return Err(PackagingError::PackageNotFound(name));
    }

    dist_tags.retain(|_, version| versions.contains_key(version));
    if !dist_tags.contains_key("latest")
        && let Some(latest) = semver_latest(versions.keys().map(String::as_str), &HashSet::new())
    {
        dist_tags.insert("latest".to_string(), latest);
    }

    Ok(packument_document(&name, &dist_tags, &versions))
}

fn packument_document(
    name: &str,
    dist_tags: &BTreeMap<String, String>,
    versions: &Map<String, Value>,
) -> Bytes {
    let packument = serde_json::json!({
        "_id": name,
        "name": name,
        "dist-tags": dist_tags,
        "versions": versions,
    });
    Bytes::from(serde_json::to_vec(&packument).expect("a packument always serializes to valid JSON"))
}

fn parse_dist_tags(value: Option<&Value>) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    let Some(Value::Object(map)) = value else {
        return tags;
    };
    for (tag, version) in map {
        if let Some(version) = version.as_str() {
            tags.insert(tag.clone(), version.to_string());
        }
    }
    tags
}

fn collect_dist_tags(entries: &[VersionEntry]) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    for entry in entries {
        tags.extend(entry.dist_tags.iter().map(|(tag, version)| (tag.clone(), version.clone())));
    }
    tags
}

fn advertised_version(entries: &[VersionEntry]) -> String {
    let stored = collect_dist_tags(entries);
    if let Some(latest) = stored.get("latest")
        && entries.iter().any(|entry| entry.version == *latest)
    {
        return latest.clone();
    }
    let yanked = entries
        .iter()
        .filter(|entry| entry.yanked)
        .map(|entry| entry.version.clone())
        .collect();
    semver_latest(entries.iter().map(|entry| entry.version.as_str()), &yanked)
        .or_else(|| entries.last().map(|entry| entry.version.clone()))
        .expect("grouped versions are never empty")
}

fn resolve_dist_tags(
    entries: &[VersionEntry],
    versions: &Map<String, Value>,
) -> BTreeMap<String, String> {
    let yanked = entries
        .iter()
        .filter(|entry| entry.yanked)
        .map(|entry| entry.version.clone())
        .collect();
    let mut tags = collect_dist_tags(entries);
    tags.retain(|_, version| versions.contains_key(version));
    if !tags.contains_key("latest")
        && let Some(latest) = semver_latest(versions.keys().map(String::as_str), &yanked)
    {
        tags.insert("latest".to_string(), latest);
    }
    tags
}

/// Compara versiones npm por `major.minor.patch`, no por orden
/// lexicográfico (`"4.9.0"` > `"4.17.21"` como cadenas).
fn parse_release_tuple(version: &str) -> Option<(u64, u64, u64, bool)> {
    let version = version.split_once('+').map_or(version, |(core, _)| core);
    let (core, prerelease) = version
        .split_once('-')
        .map_or((version, false), |(core, pre)| (core, !pre.is_empty()));
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch, prerelease))
}

fn semver_latest<'a>(
    versions: impl IntoIterator<Item = &'a str>,
    yanked: &HashSet<String>,
) -> Option<String> {
    let mut best_release: Option<((u64, u64, u64), &'a str)> = None;
    let mut best_any: Option<((u64, u64, u64), &'a str)> = None;
    for version in versions {
        if yanked.contains(version) {
            continue;
        }
        let Some((major, minor, patch, prerelease)) = parse_release_tuple(version) else {
            continue;
        };
        let key = (major, minor, patch);
        if best_any.is_none_or(|(current, _)| key > current) {
            best_any = Some((key, version));
        }
        if !prerelease && best_release.is_none_or(|(current, _)| key > current) {
            best_release = Some((key, version));
        }
    }
    best_release
        .or(best_any)
        .map(|(_, version)| version.to_string())
}

fn sha1_hex(content: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    format!("{:x}", Sha1::digest(content))
}

fn sha512_integrity(content: &[u8]) -> String {
    use sha2::{Digest, Sha512};
    format!("sha512-{}", BASE64.encode(Sha512::digest(content)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::assay::AssayService;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryAssayStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn strategy() -> NpmPackagingStrategy {
        NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        )
    }

    fn npm_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Npm,
        )
        .unwrap()
    }

    fn publish_body(name: &str, version: &str, tarball: &[u8]) -> Bytes {
        let encoded = BASE64.encode(tarball);
        Bytes::from(
            serde_json::json!({
                "name": name,
                "dist-tags": { "latest": version },
                "versions": {
                    version: { "name": name, "version": version, "description": "demo" }
                },
                "_attachments": {
                    format!("{name}-{version}.tgz"): {
                        "content_type": "application/octet-stream",
                        "data": encoded,
                        "length": tarball.len()
                    }
                }
            })
            .to_string(),
        )
    }

    #[test]
    fn tarball_filename_strips_scope() {
        assert_eq!(tarball_filename("demo-pkg", "1.0.0"), "demo-pkg-1.0.0.tgz");
        assert_eq!(
            tarball_filename("@acme/demo-pkg", "1.0.0"),
            "demo-pkg-1.0.0.tgz"
        );
    }

    #[tokio::test]
    async fn publish_then_index_and_download() {
        let strategy = strategy();
        let repository = npm_forge("npm-local");
        let tarball = b"tarball-bytes";

        let coordinate = strategy
            .publish(&repository, publish_body("demo-pkg", "1.0.0", tarball))
            .await
            .unwrap();

        assert_eq!(coordinate.name().as_str(), "demo-pkg");
        assert_eq!(coordinate.version().as_str(), "1.0.0");

        let packument: Value =
            serde_json::from_slice(&strategy.index(&repository, coordinate.name()).await.unwrap())
                .unwrap();
        assert_eq!(packument["name"], "demo-pkg");
        assert_eq!(packument["dist-tags"]["latest"], "1.0.0");
        assert_eq!(
            packument["versions"]["1.0.0"]["dist"]["tarball"],
            format!(
                "http://127.0.0.1:3000/npm/{}/demo-pkg/-/demo-pkg-1.0.0.tgz",
                repository.id()
            )
        );

        let downloaded = strategy.download(&repository, &coordinate).await.unwrap();
        assert_eq!(downloaded.as_ref(), tarball);
    }

    #[tokio::test]
    async fn publish_schedules_an_assay_without_blocking() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            Bytes::from(r#"{"results":[{}]}"#),
        );
        let assays = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index.clone(),
            repos.clone(),
            storage.clone(),
            http.clone(),
        );
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            index,
            storage,
            http,
            repos.clone(),
            "http://127.0.0.1:3000".to_string(),
        )
        .with_assays(assays.clone());
        let repository = npm_forge("npm-local");
        repos.save(&repository).await.unwrap();

        strategy
            .publish(&repository, publish_body("demo-pkg", "1.0.0", b"tarball"))
            .await
            .unwrap();

        let mut found = Vec::new();
        for _ in 0..50 {
            found = assays.list_all().await.unwrap();
            if !found.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].coordinate().name().as_str(), "demo-pkg");
    }

    #[tokio::test]
    async fn rejects_publishing_the_same_version_twice() {
        let strategy = strategy();
        let repository = npm_forge("npm-local");
        let body = publish_body("demo-pkg", "1.0.0", b"tarball");

        strategy.publish(&repository, body.clone()).await.unwrap();
        let err = strategy.publish(&repository, body).await.unwrap_err();
        assert!(matches!(err, PackagingError::AlreadyPublished(_)));
    }

    #[tokio::test]
    async fn rejects_publishing_to_an_alloy() {
        let first = npm_forge("npm-a");
        let alloy = Repository::new(
            RepositoryName::parse("npm-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first.id()],
            },
            PackageEcosystem::Npm,
        )
        .unwrap();

        let err = strategy()
            .publish(&alloy, publish_body("demo-pkg", "1.0.0", b"tarball"))
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::ReadOnlyRepository));
    }

    #[tokio::test]
    async fn yank_marks_the_version_as_deprecated_in_the_packument() {
        let strategy = strategy();
        let repository = npm_forge("npm-local");
        let coordinate = strategy
            .publish(&repository, publish_body("demo-pkg", "1.0.0", b"tarball"))
            .await
            .unwrap();

        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();

        let packument: Value =
            serde_json::from_slice(&strategy.index(&repository, coordinate.name()).await.unwrap())
                .unwrap();
        assert_eq!(packument["versions"]["1.0.0"]["deprecated"], "yanked");
    }

    #[tokio::test]
    async fn promote_copies_tarball_to_another_forge() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = NpmPackagingStrategy::new(
            artifact_store,
            index,
            storage,
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        );
        let source = npm_forge("npm-dev");
        let target = npm_forge("npm-prod");
        let coordinate = strategy
            .publish(&source, publish_body("demo-pkg", "1.0.0", b"tarball"))
            .await
            .unwrap();

        let outcome = strategy
            .promote_version(&source, &target, &coordinate, false)
            .await
            .unwrap();
        assert_eq!(outcome.artifacts_copied, 1);
        assert_eq!(
            strategy.download(&target, &coordinate).await.unwrap().as_ref(),
            b"tarball"
        );
    }

    #[tokio::test]
    async fn search_matches_package_names() {
        let strategy = strategy();
        let repository = npm_forge("npm-local");
        strategy
            .publish(&repository, publish_body("demo-pkg", "1.0.0", b"tarball"))
            .await
            .unwrap();

        let hits = strategy.search(&repository, "demo", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "demo-pkg");
        assert_eq!(hits[0].max_version, "1.0.0");
    }

    #[tokio::test]
    async fn alloy_index_and_download_see_packages_published_to_a_forge_member() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            repository_store.clone(),
            "http://127.0.0.1:3000".to_string(),
        );

        let forge = npm_forge("npm-local");
        let alloy = Repository::new(
            RepositoryName::parse("npm-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge.id()],
            },
            PackageEcosystem::Npm,
        )
        .unwrap();
        repository_store.save(&forge).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        let coordinate = strategy
            .publish(&forge, publish_body("demo-pkg", "1.0.0", b"tarball"))
            .await
            .unwrap();

        let packument: Value =
            serde_json::from_slice(&strategy.index(&alloy, coordinate.name()).await.unwrap())
                .unwrap();
        assert_eq!(packument["name"], "demo-pkg");
        assert!(
            packument["versions"]["1.0.0"]["dist"]["tarball"]
                .as_str()
                .unwrap()
                .contains(&alloy.id().to_string())
        );

        let downloaded = strategy.download(&alloy, &coordinate).await.unwrap();
        assert_eq!(downloaded.as_ref(), b"tarball");
    }

    #[tokio::test]
    async fn rejects_a_truncated_payload() {
        let err = strategy()
            .publish(&npm_forge("npm-local"), Bytes::from_static(b"{"))
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::InvalidPayload(_)));
    }

    fn npm_mirror(name: &str, upstream: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse(upstream).unwrap(),
            },
            PackageEcosystem::Npm,
        )
        .unwrap()
    }

    fn upstream_packument(name: &str, version: &str, tarball_url: &str, tarball: &[u8]) -> Bytes {
        Bytes::from(
            serde_json::json!({
                "name": name,
                "dist-tags": { "latest": version },
                "versions": {
                    version: {
                        "name": name,
                        "version": version,
                        "dist": {
                            "tarball": tarball_url,
                            "shasum": sha1_hex(tarball),
                            "integrity": sha512_integrity(tarball)
                        }
                    }
                }
            })
            .to_string(),
        )
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_mirror() {
        let err = strategy()
            .publish(
                &npm_mirror("npm-proxy", "https://registry.example/"),
                publish_body("demo-pkg", "1.0.0", b"tarball"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::ReadOnlyRepository));
    }

    #[tokio::test]
    async fn yanking_on_a_mirror_is_rejected() {
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("demo-pkg").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let err = strategy()
            .set_yanked(
                &npm_mirror("npm-proxy", "https://registry.example/"),
                &coordinate,
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::ReadOnlyRepository));
    }

    #[tokio::test]
    async fn mirror_indexes_and_caches_a_tarball_from_upstream() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        );

        let repository = npm_mirror("npm-proxy", "https://registry.example/");
        let tarball = Bytes::from_static(b"cached-npm-tarball");
        let tarball_url = "https://registry.example/demo-pkg/-/demo-pkg-1.0.0.tgz";
        http.stub(
            "https://registry.example/demo-pkg",
            200,
            upstream_packument("demo-pkg", "1.0.0", tarball_url, &tarball),
        );
        http.stub(tarball_url, 200, tarball.clone());

        let packument: Value = serde_json::from_slice(
            &strategy
                .index(&repository, &PackageName::parse("demo-pkg").unwrap())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(packument["name"], "demo-pkg");
        assert_eq!(
            packument["versions"]["1.0.0"]["dist"]["tarball"],
            format!(
                "http://127.0.0.1:3000/npm/{}/demo-pkg/-/demo-pkg-1.0.0.tgz",
                repository.id()
            )
        );

        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("demo-pkg").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let downloaded = strategy.download(&repository, &coordinate).await.unwrap();
        assert_eq!(downloaded, tarball);

        let downloaded_again = strategy.download(&repository, &coordinate).await.unwrap();
        assert_eq!(downloaded_again, tarball);
    }

    #[tokio::test]
    async fn mirror_download_refreshes_the_packument_when_the_index_is_empty() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        );

        let repository = npm_mirror("npm-proxy", "https://registry.example/");
        let tarball = Bytes::from_static(b"lazy-cache");
        let tarball_url = "https://registry.example/demo-pkg/-/demo-pkg-1.0.0.tgz";
        http.stub(
            "https://registry.example/demo-pkg",
            200,
            upstream_packument("demo-pkg", "1.0.0", tarball_url, &tarball),
        );
        http.stub(tarball_url, 200, tarball.clone());

        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("demo-pkg").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let downloaded = strategy.download(&repository, &coordinate).await.unwrap();
        assert_eq!(downloaded, tarball);
    }

    #[tokio::test]
    async fn mirror_marks_upstream_deprecated_versions_as_yanked() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        );

        let repository = npm_mirror("npm-proxy", "https://registry.example/");
        let tarball = b"deprecated-tarball";
        http.stub(
            "https://registry.example/demo-pkg",
            200,
            Bytes::from(
                serde_json::json!({
                    "name": "demo-pkg",
                    "versions": {
                        "1.0.0": {
                            "name": "demo-pkg",
                            "version": "1.0.0",
                            "deprecated": "use 2.x",
                            "dist": {
                                "tarball": "https://registry.example/demo-pkg/-/demo-pkg-1.0.0.tgz",
                                "shasum": sha1_hex(tarball),
                                "integrity": sha512_integrity(tarball)
                            }
                        }
                    }
                })
                .to_string(),
            ),
        );

        let packument: Value = serde_json::from_slice(
            &strategy
                .index(&repository, &PackageName::parse("demo-pkg").unwrap())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(packument["versions"]["1.0.0"]["deprecated"], "yanked");
    }

    #[tokio::test]
    async fn mirror_fetches_scoped_packuments_with_encoded_names() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        );

        let repository = npm_mirror("npm-proxy", "https://registry.example/");
        let tarball = b"scoped-tarball";
        let tarball_url = "https://registry.example/@acme/demo-pkg/-/demo-pkg-1.0.0.tgz";
        http.stub(
            "https://registry.example/%40acme%2Fdemo-pkg",
            200,
            upstream_packument("@acme/demo-pkg", "1.0.0", tarball_url, tarball),
        );

        let packument: Value = serde_json::from_slice(
            &strategy
                .index(
                    &repository,
                    &PackageName::parse("@acme/demo-pkg").unwrap(),
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(packument["name"], "@acme/demo-pkg");
        assert!(
            packument["versions"]["1.0.0"]["dist"]["tarball"]
                .as_str()
                .unwrap()
                .contains("%40acme%2Fdemo-pkg/-/demo-pkg-1.0.0.tgz")
        );
    }

    #[tokio::test]
    async fn alloy_index_and_download_see_packages_from_a_mirror_member() {
        let http = Arc::new(InMemoryHttpClient::default());
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
            repository_store.clone(),
            "http://127.0.0.1:3000".to_string(),
        );

        let mirror = npm_mirror("npm-proxy", "https://registry.example/");
        let alloy = Repository::new(
            RepositoryName::parse("npm-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![mirror.id()],
            },
            PackageEcosystem::Npm,
        )
        .unwrap();
        repository_store.save(&mirror).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        let tarball = Bytes::from_static(b"from-mirror");
        let tarball_url = "https://registry.example/demo-pkg/-/demo-pkg-1.0.0.tgz";
        http.stub(
            "https://registry.example/demo-pkg",
            200,
            upstream_packument("demo-pkg", "1.0.0", tarball_url, &tarball),
        );
        http.stub(tarball_url, 200, tarball.clone());

        let packument: Value = serde_json::from_slice(
            &strategy
                .index(&alloy, &PackageName::parse("demo-pkg").unwrap())
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            packument["versions"]["1.0.0"]["dist"]["tarball"]
                .as_str()
                .unwrap()
                .contains(&alloy.id().to_string())
        );

        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("demo-pkg").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let downloaded = strategy.download(&alloy, &coordinate).await.unwrap();
        assert_eq!(downloaded, tarball);
    }

    #[test]
    fn join_upstream_encodes_against_the_registry_base() {
        let upstream = url::Url::parse("https://registry.npmjs.org/").unwrap();
        assert_eq!(
            join_upstream(&upstream, "lodash"),
            "https://registry.npmjs.org/lodash"
        );
        assert_eq!(
            join_upstream(&upstream, &encode_npm_name("@types/node")),
            "https://registry.npmjs.org/%40types%2Fnode"
        );
    }

    #[test]
    fn semver_latest_prefers_numeric_order_over_lexicographic() {
        let none = HashSet::new();
        assert_eq!(
            semver_latest(["4.9.0", "4.17.21", "4.0.0"], &none).as_deref(),
            Some("4.17.21")
        );
    }

    #[test]
    fn semver_latest_prefers_a_release_over_a_newer_prerelease() {
        let none = HashSet::new();
        assert_eq!(
            semver_latest(["2.0.0-beta.1", "1.9.0"], &none).as_deref(),
            Some("1.9.0")
        );
    }

    fn version_dist(name: &str, version: &str, tarball_url: &str, tarball: &[u8]) -> Value {
        serde_json::json!({
            "name": name,
            "version": version,
            "dist": {
                "tarball": tarball_url,
                "shasum": sha1_hex(tarball),
                "integrity": sha512_integrity(tarball)
            }
        })
    }

    #[tokio::test]
    async fn mirror_packument_uses_upstream_dist_tags_latest() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        );

        let repository = npm_mirror("npm-proxy", "https://registry.example/");
        let tarball = b"lodash-bytes";
        http.stub(
            "https://registry.example/lodash",
            200,
            Bytes::from(
                serde_json::json!({
                    "name": "lodash",
                    "dist-tags": { "latest": "4.17.21" },
                    "versions": {
                        "4.9.0": version_dist(
                            "lodash",
                            "4.9.0",
                            "https://registry.example/lodash/-/lodash-4.9.0.tgz",
                            tarball,
                        ),
                        "4.17.21": version_dist(
                            "lodash",
                            "4.17.21",
                            "https://registry.example/lodash/-/lodash-4.17.21.tgz",
                            tarball,
                        )
                    }
                })
                .to_string(),
            ),
        );

        let packument: Value = serde_json::from_slice(
            &strategy
                .index(&repository, &PackageName::parse("lodash").unwrap())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(packument["dist-tags"]["latest"], "4.17.21");
        assert!(packument["versions"].get("4.9.0").is_some());
        assert!(packument["versions"].get("4.17.21").is_some());
    }

    #[tokio::test]
    async fn mirror_packument_falls_back_to_semver_when_dist_tags_are_missing() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        );

        let repository = npm_mirror("npm-proxy", "https://registry.example/");
        let tarball = b"lodash-bytes";
        http.stub(
            "https://registry.example/lodash",
            200,
            Bytes::from(
                serde_json::json!({
                    "name": "lodash",
                    "versions": {
                        "4.9.0": version_dist(
                            "lodash",
                            "4.9.0",
                            "https://registry.example/lodash/-/lodash-4.9.0.tgz",
                            tarball,
                        ),
                        "4.17.21": version_dist(
                            "lodash",
                            "4.17.21",
                            "https://registry.example/lodash/-/lodash-4.17.21.tgz",
                            tarball,
                        )
                    }
                })
                .to_string(),
            ),
        );

        let packument: Value = serde_json::from_slice(
            &strategy
                .index(&repository, &PackageName::parse("lodash").unwrap())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(packument["dist-tags"]["latest"], "4.17.21");
    }
}
