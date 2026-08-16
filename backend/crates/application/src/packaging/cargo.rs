//! Estrategia de empaquetado para el ecosistema Cargo: implementa el
//! subconjunto del protocolo de índice disperso (*sparse index*) que
//! `cargo` necesita para publicar paquetes (`cargo publish`) y resolver
//! sus dependencias (`cargo build`, `cargo add`) contra un repositorio
//! `FerroBox`.
//!
//! Referencia del protocolo:
//! <https://doc.rust-lang.org/cargo/reference/registry-index.html>
//! y <https://doc.rust-lang.org/cargo/reference/registries.html>.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::checksum::Sha256Checksum;
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

use super::{notify_assay, PackageSearchHit, PackagingError, PackagingStrategy, PublishOutcome};
use crate::assay::AssayService;
use crate::content_hash::sha256_checksum;
use crate::storage_key::storage_key_for;

/// Estrategia de empaquetado para el ecosistema Cargo.
pub struct CargoPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    http_client: Arc<dyn HttpClient>,
    repository_store: Arc<dyn RepositoryStore>,
    assays: Option<AssayService>,
}

impl CargoPackagingStrategy {
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
        }
    }

    /// Conecta el ensaye automático al publicar o cachear un crate.
    #[must_use]
    pub fn with_assays(mut self, assays: AssayService) -> Self {
        self.assays = Some(assays);
        self
    }

    fn ensure_cargo_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Cargo {
            return Err(PackagingError::EcosystemMismatch {
                expected: "cargo",
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

    fn merge_index_documents(documents: &[Bytes]) -> Bytes {
        let mut seen_versions = HashSet::new();
        let mut body = BytesMut::new();
        for document in documents {
            for line in document.split(|&byte| byte == b'\n') {
                let line = trim_ascii_whitespace(line);
                if line.is_empty() {
                    continue;
                }
                let Ok(entry) = serde_json::from_slice::<IndexEntry>(line) else {
                    continue;
                };
                if seen_versions.insert(entry.vers) {
                    body.extend_from_slice(line);
                    body.extend_from_slice(b"\n");
                }
            }
        }
        body.freeze()
    }

    async fn refresh_index_from_upstream(
        &self,
        repository: &Repository,
        upstream: &url::Url,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        let index_url = join_upstream(upstream, &cargo_index_shard_path(name));
        let response = self.http_client.get(&index_url).await?;
        let body = response.body;

        for line in body.split(|&byte| byte == b'\n') {
            let line = trim_ascii_whitespace(line);
            if line.is_empty() {
                continue;
            }

            let entry: IndexEntry = serde_json::from_slice(line)
                .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;
            let version = PackageVersion::parse(entry.vers.clone())
                .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;
            let package_name = PackageName::parse(entry.name.clone())
                .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;
            let coordinate = PackageCoordinate::new(PackageEcosystem::Cargo, package_name, version);

            self.package_index_store
                .upsert_entry(
                    repository.id(),
                    &coordinate,
                    None,
                    Bytes::copy_from_slice(line),
                )
                .await?;
        }

        Ok(body)
    }

    async fn upstream_download_url(
        &self,
        upstream: &url::Url,
        coordinate: &PackageCoordinate,
    ) -> Result<String, PackagingError> {
        let config_url = join_upstream(upstream, "config.json");
        let response = self.http_client.get(&config_url).await?;
        let config: UpstreamRegistryConfig = serde_json::from_slice(&response.body)
            .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;

        Ok(expand_dl_template(
            &config.dl,
            coordinate.name().as_str(),
            coordinate.version().as_str(),
        ))
    }

    async fn cache_crate_from_upstream(
        &self,
        repository: &Repository,
        upstream: &url::Url,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Cargo, coordinate.name())
            .await?;

        if entries.is_empty() {
            self.refresh_index_from_upstream(repository, upstream, coordinate.name())
                .await?;
        }

        let entry = self
            .find_local_index_entry(repository, coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;

        let download_url = self.upstream_download_url(upstream, coordinate).await?;
        let response = self.http_client.get(&download_url).await?;
        let crate_bytes = response.body;

        let checksum = sha256_checksum(&crate_bytes);
        if checksum.as_str() != entry.cksum {
            return Err(PackagingError::InvalidUpstream(format!(
                "checksum mismatch for {coordinate}: expected {}, got {checksum}",
                entry.cksum
            )));
        }

        let artifact = Artifact::new(repository.id(), checksum, crate_bytes.len() as u64);

        self.storage
            .put(&storage_key_for(artifact.id()), crate_bytes.clone())
            .await?;
        self.artifact_store.save(&artifact).await?;

        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("an IndexEntry always serializes to valid JSON"),
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

        Ok(crate_bytes)
    }

    async fn find_local_index_entry(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<IndexEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Cargo, coordinate.name())
            .await?;

        for entry_bytes in entries {
            let entry: IndexEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;
            if entry.vers == coordinate.version().as_str() {
                return Ok(Some(entry));
            }
        }

        Ok(None)
    }

    async fn index_one(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        if let Some(upstream) = Self::mirror_upstream(repository) {
            return self
                .refresh_index_from_upstream(repository, upstream, name)
                .await;
        }

        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Cargo, name)
            .await?;

        if entries.is_empty() {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        }

        let mut body = BytesMut::new();
        for entry in entries {
            body.extend_from_slice(&entry);
            body.extend_from_slice(b"\n");
        }

        Ok(body.freeze())
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
                .cache_crate_from_upstream(repository, upstream, coordinate)
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
            .entries_for_repository(repository.id(), PackageEcosystem::Cargo)
            .await?;

        let needle = query.to_ascii_lowercase();
        let mut by_name: BTreeMap<String, Vec<IndexEntry>> = BTreeMap::new();
        for entry_bytes in entries {
            let entry: IndexEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if !entry.name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            by_name.entry(entry.name.clone()).or_default().push(entry);
        }

        let mut hits = Vec::new();
        for (name, versions) in by_name {
            let max_version = versions
                .iter()
                .rev()
                .find(|entry| !entry.yanked)
                .or_else(|| versions.last())
                .expect("grouped versions are never empty")
                .vers
                .clone();
            hits.push(PackageSearchHit { name, max_version });
            if hits.len() >= limit {
                break;
            }
        }

        Ok(hits)
    }
}

#[async_trait]
impl PackagingStrategy for CargoPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Cargo
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_cargo_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }

        let parsed = parse_publish_payload(payload)?;

        let name = PackageName::parse(parsed.metadata.name.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let version = PackageVersion::parse(parsed.metadata.vers.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let coordinate = PackageCoordinate::new(PackageEcosystem::Cargo, name, version);

        if self
            .package_index_store
            .artifact_for(repository.id(), &coordinate)
            .await?
            .is_some()
        {
            return Err(PackagingError::AlreadyPublished(coordinate));
        }

        let artifact = Artifact::new(
            repository.id(),
            parsed.checksum.clone(),
            parsed.crate_bytes.len() as u64,
        );

        self.storage
            .put(&storage_key_for(artifact.id()), parsed.crate_bytes)
            .await?;
        self.artifact_store.save(&artifact).await?;

        let entry = IndexEntry::from_publish_metadata(&parsed.metadata, parsed.checksum.as_str());
        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("an IndexEntry always serializes to valid JSON"),
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
        Self::ensure_cargo_repository(repository)?;

        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut documents = Vec::new();
            let mut other_error = None;
            for target in &targets {
                match self.index_one(target, name).await {
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
                return Err(other_error.unwrap_or_else(|| {
                    PackagingError::PackageNotFound(name.to_string())
                }));
            }
            return Ok(Self::merge_index_documents(&documents));
        }

        self.index_one(repository, name).await
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_cargo_repository(repository)?;

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

    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError> {
        Self::ensure_cargo_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }

        let Some(mut entry) = self.find_local_index_entry(repository, coordinate).await? else {
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
            serde_json::to_vec(&entry).expect("an IndexEntry always serializes to valid JSON"),
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
        Self::ensure_cargo_repository(repository)?;

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

/// Calcula la ruta relativa -- bajo la raíz del índice disperso -- en la
/// que `cargo` espera encontrar las entradas de índice de un paquete,
/// siguiendo las reglas oficiales de fragmentación (*sharding*) del
/// protocolo de índice disperso: nombres de 1 y 2 caracteres viven en
/// `1/` y `2/` respectivamente; los de 3 caracteres, en `3/{primera
/// letra}/`; y el resto, en `{dos primeras letras}/{siguientes dos
/// letras}/`. `cargo` siempre normaliza el nombre a minúsculas al
/// calcular esta ruta.
#[must_use]
pub fn cargo_index_shard_path(name: &PackageName) -> String {
    let lower = name.as_str().to_ascii_lowercase();
    match lower.len() {
        1 => format!("1/{lower}"),
        2 => format!("2/{lower}"),
        3 => format!("3/{}/{lower}", &lower[0..1]),
        _ => format!("{}/{}/{lower}", &lower[0..2], &lower[2..4]),
    }
}

fn join_upstream(upstream: &url::Url, relative: &str) -> String {
    let base = upstream.as_str().trim_end_matches('/');
    let relative = relative.trim_start_matches('/');
    format!("{base}/{relative}")
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &bytes[start..end]
}

fn expand_dl_template(template: &str, crate_name: &str, version: &str) -> String {
    let lower = crate_name.to_ascii_lowercase();
    let prefix = match lower.len() {
        0 => String::new(),
        1..=2 => lower.clone(),
        3 => lower[0..1].to_string(),
        _ => format!("{}/{}", &lower[0..2], &lower[2..4]),
    };

    // Misma regla que Cargo: si `dl` no trae marcadores, se añade
    // `/{crate}/{version}/download`. crates.io (índice disperso) publica
    // `https://static.crates.io/crates` sin plantilla; sin este paso el
    // Mirror pediría el directorio y crates.io responde 403.
    // https://doc.rust-lang.org/cargo/reference/registry-index.html
    let has_markers = ["{crate}", "{version}", "{prefix}", "{lowerprefix}", "{sha256-checksum}"]
        .iter()
        .any(|marker| template.contains(marker));
    if !has_markers {
        let base = template.trim_end_matches('/');
        return format!("{base}/{crate_name}/{version}/download");
    }

    template
        .replace("{crate}", crate_name)
        .replace("{version}", version)
        .replace("{prefix}", &prefix)
        .replace("{lowerprefix}", &prefix)
}

#[derive(Debug, Deserialize)]
struct UpstreamRegistryConfig {
    dl: String,
}

/// Cuerpo JSON que `cargo publish` envía como primer bloque de la
/// petición de publicación. Solo se modelan los campos que
/// `FerroBox` necesita para construir la entrada de índice; el resto
/// (autores, descripción, licencia...) se ignoran silenciosamente
/// gracias al comportamiento por defecto de `serde` ante campos
/// desconocidos.
#[derive(Debug, Deserialize)]
struct PublishMetadata {
    name: String,
    vers: String,
    #[serde(default)]
    deps: Vec<PublishDependency>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    links: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublishDependency {
    name: String,
    version_req: String,
    #[serde(default)]
    features: Vec<String>,
    optional: bool,
    default_features: bool,
    #[serde(default)]
    target: Option<String>,
    kind: String,
    #[serde(default)]
    registry: Option<String>,
    #[serde(default)]
    explicit_name_in_toml: Option<String>,
}

/// Una entrada del índice disperso de Cargo: una línea JSON que describe
/// una versión publicada de un paquete.
///
/// Formato oficial:
/// <https://doc.rust-lang.org/cargo/reference/registry-index.html#json-schema>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    name: String,
    vers: String,
    #[serde(default)]
    deps: Vec<IndexDependency>,
    cksum: String,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    links: Option<String>,
    #[serde(default = "default_index_schema_version")]
    v: u32,
}

fn default_index_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IndexDependency {
    name: String,
    req: String,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default = "default_true")]
    default_features: bool,
    #[serde(default)]
    target: Option<String>,
    #[serde(default = "default_normal_kind")]
    kind: String,
    #[serde(default)]
    registry: Option<String>,
    #[serde(default)]
    package: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_normal_kind() -> String {
    "normal".to_string()
}

impl IndexEntry {
    fn from_publish_metadata(metadata: &PublishMetadata, cksum: &str) -> Self {
        Self {
            name: metadata.name.clone(),
            vers: metadata.vers.clone(),
            deps: metadata.deps.iter().map(IndexDependency::from).collect(),
            cksum: cksum.to_string(),
            features: metadata.features.clone(),
            yanked: false,
            links: metadata.links.clone(),
            // Se fija deliberadamente en 1: esta versión de esquema le
            // indica a `cargo` que no busque un campo `features2`, que
            // esta estrategia nunca genera (solo es necesario para
            // "weak dependency features", fuera del alcance de esta
            // primera implementación).
            v: 1,
        }
    }
}

impl From<&PublishDependency> for IndexDependency {
    fn from(dep: &PublishDependency) -> Self {
        // Si la dependencia se renombró en el `Cargo.toml` del
        // publicador (`foo = { package = "bar", version = "1" }`), el
        // índice debe reflejar el nombre local bajo `name` y el nombre
        // real del paquete bajo `package` -- así es como `cargo`
        // distingue un alias de un nombre de paquete real al resolver.
        let (name, package) = match &dep.explicit_name_in_toml {
            Some(local_alias) => (local_alias.clone(), Some(dep.name.clone())),
            None => (dep.name.clone(), None),
        };

        Self {
            name,
            req: dep.version_req.clone(),
            features: dep.features.clone(),
            optional: dep.optional,
            default_features: dep.default_features,
            target: dep.target.clone(),
            kind: dep.kind.clone(),
            registry: dep.registry.clone(),
            package,
        }
    }
}

/// Resultado de descomponer una petición `cargo publish`: metadatos,
/// contenido del `.crate` y el SHA-256 de ese contenido, ya listo para
/// persistirse tanto en el artefacto como en la entrada de índice.
struct ParsedPublishPayload {
    metadata: PublishMetadata,
    crate_bytes: Bytes,
    checksum: Sha256Checksum,
}

/// Descompone el cuerpo binario de una petición `cargo publish` en sus
/// dos partes: los metadatos JSON y el contenido del archivo `.crate`.
/// El checksum SHA-256 se calcula aquí, sobre los bytes del `.crate`,
/// para que la entrada de índice (`cksum`) y los metadatos del artefacto
/// compartan exactamente el mismo valor.
///
/// Formato (todos los enteros en *little-endian*):
/// `[u32 longitud de metadatos][metadatos JSON][u32 longitud del
/// `.crate`][contenido del `.crate`]`. Cualquier dato adicional a
/// continuación (extensiones de versiones recientes de `cargo`, como el
/// archivo `.crate` firmado) se ignora.
fn parse_publish_payload(mut payload: Bytes) -> Result<ParsedPublishPayload, PackagingError> {
    let metadata_len = read_u32_le(&mut payload, "metadata length prefix")?;
    let metadata_bytes = split_prefix(&mut payload, metadata_len, "metadata")?;
    let metadata: PublishMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|err| PackagingError::InvalidPayload(format!("invalid metadata JSON: {err}")))?;

    let crate_len = read_u32_le(&mut payload, "crate file length prefix")?;
    let crate_bytes = split_prefix(&mut payload, crate_len, "crate file")?;
    let checksum = sha256_checksum(&crate_bytes);

    Ok(ParsedPublishPayload {
        metadata,
        crate_bytes,
        checksum,
    })
}

fn read_u32_le(payload: &mut Bytes, what: &str) -> Result<usize, PackagingError> {
    if payload.remaining() < 4 {
        return Err(PackagingError::InvalidPayload(format!(
            "payload is too short to contain the {what}"
        )));
    }
    Ok(payload.get_u32_le() as usize)
}

fn split_prefix(payload: &mut Bytes, len: usize, what: &str) -> Result<Bytes, PackagingError> {
    if payload.remaining() < len {
        return Err(PackagingError::InvalidPayload(format!(
            "payload is shorter than the declared {what} length"
        )));
    }
    Ok(payload.split_to(len))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::ids::RepositoryId;
    use ferrobox_domain::repository::{RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    use super::*;

    fn cargo_repository(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap()
    }

    fn strategy() -> CargoPackagingStrategy {
        CargoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        )
    }

    fn encode_publish_payload(metadata_json: &str, crate_bytes: &[u8]) -> Bytes {
        let metadata_bytes = metadata_json.as_bytes();
        let mut payload = BytesMut::new();
        payload.extend_from_slice(&u32::try_from(metadata_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(metadata_bytes);
        payload.extend_from_slice(&u32::try_from(crate_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(crate_bytes);
        payload.freeze()
    }

    fn minimal_metadata(name: &str, vers: &str) -> String {
        format!(r#"{{"name":"{name}","vers":"{vers}","deps":[],"features":{{}}}}"#)
    }

    #[tokio::test]
    async fn publishes_a_crate_with_no_dependencies() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let payload =
            encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"fake tarball");

        let coordinate = strategy.publish(&repository, payload).await.unwrap();

        assert_eq!(coordinate.name().as_str(), "ferrobox-cli");
        assert_eq!(coordinate.version().as_str(), "0.1.0");
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_non_cargo_repository() {
        let strategy = strategy();
        let repository = Repository::new(
            RepositoryName::parse("generic-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Generic,
        )
        .unwrap();
        let payload = encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"data");

        let result = strategy.publish(&repository, payload).await;

        assert!(matches!(
            result,
            Err(PackagingError::EcosystemMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_publishing_the_same_version_twice() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let payload =
            || encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"tarball");

        strategy.publish(&repository, payload()).await.unwrap();
        let result = strategy.publish(&repository, payload()).await;

        assert!(matches!(result, Err(PackagingError::AlreadyPublished(_))));
    }

    #[tokio::test]
    async fn rejects_a_truncated_payload() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");

        let result = strategy
            .publish(&repository, Bytes::from_static(b"\x05\x00"))
            .await;

        assert!(matches!(result, Err(PackagingError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn index_lists_every_published_version_as_a_json_line_per_entry() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"v1"),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.2.0"), b"v2"),
            )
            .await
            .unwrap();

        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();

        let lines: Vec<&str> = std::str::from_utf8(&index).unwrap().lines().collect();
        assert_eq!(lines.len(), 2);
        let first: IndexEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first.vers, "0.1.0");
        assert!(!first.yanked);
        assert_eq!(first.cksum, sha256_checksum(b"v1").as_str());
    }

    #[tokio::test]
    async fn index_for_an_unpublished_package_is_not_found() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");

        let result = strategy
            .index(&repository, &PackageName::parse("nonexistent").unwrap())
            .await;

        assert!(matches!(result, Err(PackagingError::PackageNotFound(_))));
    }

    #[tokio::test]
    async fn publish_stores_sha256_of_the_crate_file_in_the_index_entry() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let crate_bytes = b"tarball-for-checksum";
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), crate_bytes),
            )
            .await
            .unwrap();

        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();
        let entry: IndexEntry =
            serde_json::from_str(std::str::from_utf8(&index).unwrap().trim()).unwrap();

        assert_eq!(entry.cksum, sha256_checksum(crate_bytes).as_str());
        assert_eq!(entry.cksum.len(), 64);
    }

    #[tokio::test]
    async fn downloads_a_previously_published_crate() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let coordinate = strategy
            .publish(
                &repository,
                encode_publish_payload(
                    &minimal_metadata("ferrobox-cli", "0.1.0"),
                    b"tarball bytes",
                ),
            )
            .await
            .unwrap();

        let content = strategy.download(&repository, &coordinate).await.unwrap();

        assert_eq!(content, Bytes::from_static(b"tarball bytes"));
    }

    #[tokio::test]
    async fn downloading_an_unpublished_version_is_not_found() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("ferrobox-cli").unwrap(),
            PackageVersion::parse("9.9.9").unwrap(),
        );

        let result = strategy.download(&repository, &coordinate).await;

        assert!(matches!(result, Err(PackagingError::VersionNotFound(_))));
    }

    #[tokio::test]
    async fn yank_marks_the_index_entry_and_unyank_clears_it() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let coordinate = strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"tarball"),
            )
            .await
            .unwrap();

        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();

        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();
        let entry: IndexEntry =
            serde_json::from_str(std::str::from_utf8(&index).unwrap().trim()).unwrap();
        assert!(entry.yanked);

        strategy
            .set_yanked(&repository, &coordinate, false)
            .await
            .unwrap();
        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();
        let entry: IndexEntry =
            serde_json::from_str(std::str::from_utf8(&index).unwrap().trim()).unwrap();
        assert!(!entry.yanked);
    }

    #[tokio::test]
    async fn yanking_an_unpublished_version_is_not_found() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("ferrobox-cli").unwrap(),
            PackageVersion::parse("9.9.9").unwrap(),
        );

        let result = strategy.set_yanked(&repository, &coordinate, true).await;

        assert!(matches!(result, Err(PackagingError::VersionNotFound(_))));
    }

    #[tokio::test]
    async fn yanking_on_a_mirror_is_rejected() {
        let strategy = strategy();
        let repository = Repository::new(
            RepositoryName::parse("crates-io-mirror").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.crates.io/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );

        let result = strategy.set_yanked(&repository, &coordinate, true).await;

        assert!(matches!(result, Err(PackagingError::ReadOnlyRepository)));
    }

    #[tokio::test]
    async fn search_matches_package_names_case_insensitively() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"v1"),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("other-crate", "1.0.0"), b"v2"),
            )
            .await
            .unwrap();

        let hits = strategy.search(&repository, "FERRO", 10).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "ferrobox-cli");
        assert_eq!(hits[0].max_version, "0.1.0");
    }

    #[tokio::test]
    async fn search_prefers_a_non_yanked_version_as_max_version() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let first = strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"v1"),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.2.0"), b"v2"),
            )
            .await
            .unwrap();
        strategy
            .set_yanked(&repository, &first, true)
            .await
            .unwrap();

        let hits = strategy.search(&repository, "ferrobox", 10).await.unwrap();

        assert_eq!(hits[0].max_version, "0.2.0");
    }

    #[tokio::test]
    async fn maps_a_renamed_dependency_to_name_and_package() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let metadata = r#"{
            "name": "ferrobox-cli",
            "vers": "0.1.0",
            "deps": [{
                "name": "serde_json",
                "version_req": "^1",
                "features": [],
                "optional": false,
                "default_features": true,
                "target": null,
                "kind": "normal",
                "registry": null,
                "explicit_name_in_toml": "json"
            }],
            "features": {}
        }"#;

        strategy
            .publish(&repository, encode_publish_payload(metadata, b"tarball"))
            .await
            .unwrap();

        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();
        let entry: IndexEntry =
            serde_json::from_str(std::str::from_utf8(&index).unwrap().trim()).unwrap();

        assert_eq!(entry.deps[0].name, "json");
        assert_eq!(entry.deps[0].package.as_deref(), Some("serde_json"));
    }

    #[test]
    fn shard_path_for_a_one_character_name() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("a").unwrap()),
            "1/a"
        );
    }

    #[test]
    fn shard_path_for_a_two_character_name() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("ab").unwrap()),
            "2/ab"
        );
    }

    #[test]
    fn shard_path_for_a_three_character_name() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("abc").unwrap()),
            "3/a/abc"
        );
    }

    #[test]
    fn shard_path_for_a_longer_name_uses_the_first_four_characters() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("serde").unwrap()),
            "se/rd/serde"
        );
    }

    #[test]
    fn shard_path_normalizes_case() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("Ferrobox").unwrap()),
            "fe/rr/ferrobox"
        );
    }

    #[test]
    fn dl_template_without_markers_appends_crate_version_download() {
        assert_eq!(
            expand_dl_template("https://static.crates.io/crates", "serde", "1.0.229"),
            "https://static.crates.io/crates/serde/1.0.229/download"
        );
    }

    #[test]
    fn dl_template_with_markers_substitutes_crate_and_version() {
        assert_eq!(
            expand_dl_template(
                "https://static.example/crates/{crate}/{crate}-{version}.crate",
                "demo",
                "1.2.3",
            ),
            "https://static.example/crates/demo/demo-1.2.3.crate"
        );
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_mirror_repository() {
        let strategy = strategy();
        let repository = Repository::new(
            RepositoryName::parse("crates-io-mirror").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.crates.io/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let payload =
            encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"fake tarball");

        let result = strategy.publish(&repository, payload).await;

        assert!(matches!(result, Err(PackagingError::ReadOnlyRepository)));
    }

    #[tokio::test]
    async fn mirror_indexes_and_caches_a_crate_from_upstream() {
        let http = Arc::new(InMemoryHttpClient::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = CargoPackagingStrategy::new(
            artifact_store,
            package_index_store,
            storage,
            http.clone(),
            Arc::new(InMemoryRepositoryStore::default()),
        );

        let upstream = url::Url::parse("https://index.example/").unwrap();
        let repository = Repository::new(
            RepositoryName::parse("example-mirror").unwrap(),
            RepositoryKind::Mirror {
                upstream: upstream.clone(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();

        let crate_bytes = Bytes::from_static(b"cached-crate-bytes");
        let cksum = sha256_checksum(&crate_bytes).to_string();
        let index_line = format!(
            r#"{{"name":"demo","vers":"1.2.3","deps":[],"cksum":"{cksum}","features":{{}},"yanked":false}}"#
        );

        http.stub(
            "https://index.example/de/mo/demo",
            200,
            Bytes::from(format!("{index_line}\n")),
        );
        http.stub(
            "https://index.example/config.json",
            200,
            Bytes::from_static(
                br#"{"dl":"https://static.example/crates/{crate}/{crate}-{version}.crate","api":"https://index.example"}"#,
            ),
        );
        http.stub(
            "https://static.example/crates/demo/demo-1.2.3.crate",
            200,
            crate_bytes.clone(),
        );

        let index = strategy
            .index(&repository, &PackageName::parse("demo").unwrap())
            .await
            .unwrap();
        assert!(
            index
                .as_ref()
                .windows(cksum.len())
                .any(|window| window == cksum.as_bytes())
        );

        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.2.3").unwrap(),
        );
        let downloaded = strategy.download(&repository, &coordinate).await.unwrap();
        assert_eq!(downloaded, crate_bytes);

        // Segunda descarga: debe servirse desde la caché local sin
        // volver a pedir el `.crate` al upstream (el stub sigue ahí,
        // pero el artefacto ya está asociado en el índice).
        let downloaded_again = strategy.download(&repository, &coordinate).await.unwrap();
        assert_eq!(downloaded_again, crate_bytes);
    }

    fn alloy_fixture() -> (
        CargoPackagingStrategy,
        Arc<InMemoryRepositoryStore>,
        Repository,
        Repository,
        Repository,
    ) {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let strategy = CargoPackagingStrategy::new(
            artifact_store,
            package_index_store,
            storage,
            http,
            repository_store.clone(),
        );
        let first = cargo_repository("crates-local");
        let second = cargo_repository("crates-other");
        let alloy = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first.id(), second.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        (strategy, repository_store, first, second, alloy)
    }

    #[tokio::test]
    async fn alloy_index_and_download_see_packages_published_to_a_forge_member() {
        let (strategy, repository_store, first, second, alloy) = alloy_fixture();
        repository_store.save(&first).await.unwrap();
        repository_store.save(&second).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        let coordinate = strategy
            .publish(
                &first,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"from-forge"),
            )
            .await
            .unwrap();

        let index = strategy
            .index(&alloy, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();
        let entry: IndexEntry =
            serde_json::from_str(std::str::from_utf8(&index).unwrap().trim()).unwrap();
        assert_eq!(entry.vers, "0.1.0");
        assert_eq!(entry.cksum, sha256_checksum(b"from-forge").as_str());

        let content = strategy.download(&alloy, &coordinate).await.unwrap();
        assert_eq!(content, Bytes::from_static(b"from-forge"));
    }

    #[tokio::test]
    async fn alloy_index_prefers_the_first_member_on_version_conflict() {
        let (strategy, repository_store, first, second, alloy) = alloy_fixture();
        repository_store.save(&first).await.unwrap();
        repository_store.save(&second).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        strategy
            .publish(
                &first,
                encode_publish_payload(&minimal_metadata("demo", "1.0.0"), b"first-wins"),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &second,
                encode_publish_payload(&minimal_metadata("demo", "1.0.0"), b"second-loses"),
            )
            .await
            .unwrap();

        let index = strategy
            .index(&alloy, &PackageName::parse("demo").unwrap())
            .await
            .unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&index).unwrap().lines().collect();
        assert_eq!(lines.len(), 1);
        let entry: IndexEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry.cksum, sha256_checksum(b"first-wins").as_str());

        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let downloaded = strategy.download(&alloy, &coordinate).await.unwrap();
        assert_eq!(downloaded, Bytes::from_static(b"first-wins"));
    }

    #[tokio::test]
    async fn alloy_search_unions_member_hits_with_first_member_winning() {
        let (strategy, repository_store, first, second, alloy) = alloy_fixture();
        repository_store.save(&first).await.unwrap();
        repository_store.save(&second).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        strategy
            .publish(
                &first,
                encode_publish_payload(&minimal_metadata("shared", "1.0.0"), b"v1"),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &second,
                encode_publish_payload(&minimal_metadata("shared", "2.0.0"), b"v2"),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &second,
                encode_publish_payload(&minimal_metadata("only-second", "0.1.0"), b"v3"),
            )
            .await
            .unwrap();

        let hits = strategy.search(&alloy, "", 10).await.unwrap();
        assert_eq!(hits.len(), 2);
        let shared = hits.iter().find(|hit| hit.name == "shared").unwrap();
        assert_eq!(shared.max_version, "1.0.0");
        assert!(hits.iter().any(|hit| hit.name == "only-second"));
    }

    #[tokio::test]
    async fn rejects_publishing_to_an_alloy_repository() {
        let strategy = strategy();
        let alloy = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![RepositoryId::new()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let payload =
            encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"fake tarball");

        let result = strategy.publish(&alloy, payload).await;

        assert!(matches!(result, Err(PackagingError::ReadOnlyRepository)));
    }

    #[tokio::test]
    async fn yanking_on_an_alloy_is_rejected() {
        let strategy = strategy();
        let alloy = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![RepositoryId::new()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );

        let result = strategy.set_yanked(&alloy, &coordinate, true).await;

        assert!(matches!(result, Err(PackagingError::ReadOnlyRepository)));
    }
}
