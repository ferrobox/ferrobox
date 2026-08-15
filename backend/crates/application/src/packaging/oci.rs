//! Estrategia de empaquetado para el ecosistema OCI: implementa el
//! subconjunto del Distribution Spec v2 que `docker push` y
//! `docker pull` necesitan contra un repositorio `FerroBox`.
//!
//! Referencia: <https://github.com/opencontainers/distribution-spec>.
//!
//! Cubre **Forge** (blobs, manifiestos, etiquetas y yank) y lecturas en
//! **Alloy**. Un `Mirror` OCI queda para un corte posterior.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::ArtifactId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::ArtifactStore;
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::repository_store::RepositoryStore;
use ferrobox_ports::storage::StoragePort;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{OciManifestDocument, PackageSearchHit, PackagingError, PackagingStrategy, PublishOutcome};
use crate::content_hash::sha256_checksum;
use crate::storage_key::storage_key_for;

/// Nombre reservado en el índice para los blobs (capas y configs).
const BLOB_PACKAGE: &str = "_blob";

/// Media type por defecto de un manifiesto OCI.
pub const DEFAULT_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// Estrategia de empaquetado para el ecosistema OCI.
pub struct OciPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    repository_store: Arc<dyn RepositoryStore>,
}

impl OciPackagingStrategy {
    /// Construye la estrategia a partir de sus puertos.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        repository_store: Arc<dyn RepositoryStore>,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
            repository_store,
        }
    }

    fn ensure_oci_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Oci {
            return Err(PackagingError::EcosystemMismatch {
                expected: "oci",
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

    async fn put_blob_one(
        &self,
        repository: &Repository,
        digest: &str,
        body: Bytes,
    ) -> Result<u64, PackagingError> {
        let digest = parse_oci_digest(digest)?;
        let actual = content_digest(&body);
        if actual != digest {
            return Err(PackagingError::InvalidPayload(format!(
                "blob digest mismatch: expected {digest}, got {actual}"
            )));
        }

        let coordinate = blob_coordinate(&digest)?;
        if let Some(existing) = self
            .package_index_store
            .artifact_for(repository.id(), &coordinate)
            .await?
        {
            return Ok(self
                .artifact_store
                .find_by_id(existing)
                .await?
                .map_or(body.len() as u64, |artifact| artifact.size_bytes()));
        }

        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(repository.id(), checksum, body.len() as u64);
        self.storage
            .put(&storage_key_for(artifact.id()), body.clone())
            .await?;
        self.artifact_store.save(&artifact).await?;

        let entry = BlobEntry {
            digest: digest.clone(),
            artifact_id: artifact.id().to_string(),
            size: body.len() as u64,
        };
        self.package_index_store
            .upsert_entry(
                repository.id(),
                &coordinate,
                Some(artifact.id()),
                encode_entry(&entry),
            )
            .await?;
        Ok(body.len() as u64)
    }

    async fn get_blob_one(
        &self,
        repository: &Repository,
        digest: &str,
    ) -> Result<Bytes, PackagingError> {
        let digest = parse_oci_digest(digest)?;
        let coordinate = blob_coordinate(&digest)?;
        let artifact_id = self
            .package_index_store
            .artifact_for(repository.id(), &coordinate)
            .await?
            .ok_or_else(|| PackagingError::FileNotFound(digest.clone()))?;
        self.download_stored(artifact_id).await
    }

    async fn load_manifest_entry(
        &self,
        repository: &Repository,
        name: &PackageName,
        reference: &str,
    ) -> Result<Option<ManifestEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Oci, name)
            .await?;
        for entry_bytes in entries {
            let entry: ManifestEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if entry.reference == reference || entry.digest == reference {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    async fn put_manifest_one(
        &self,
        repository: &Repository,
        name: &str,
        reference: &str,
        media_type: &str,
        body: Bytes,
    ) -> Result<String, PackagingError> {
        let name = normalize_oci_name(name)?;
        let reference = normalize_oci_reference(reference)?;
        let digest = content_digest(&body);
        if is_digest_reference(&reference) && reference != digest {
            return Err(PackagingError::InvalidPayload(format!(
                "manifest digest mismatch: expected {reference}, got {digest}"
            )));
        }
        self.ensure_referenced_blobs(repository, &body).await?;

        let package_name = PackageName::parse(name.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let tag_version = PackageVersion::parse(reference.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let tag_coordinate =
            PackageCoordinate::new(PackageEcosystem::Oci, package_name.clone(), tag_version);

        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(repository.id(), checksum, body.len() as u64);
        self.storage
            .put(&storage_key_for(artifact.id()), body.clone())
            .await?;
        self.artifact_store.save(&artifact).await?;

        let media_type = if media_type.trim().is_empty() {
            DEFAULT_MANIFEST_MEDIA_TYPE.to_string()
        } else {
            media_type.trim().to_string()
        };

        let tag_entry = ManifestEntry {
            name: name.clone(),
            reference: reference.clone(),
            digest: digest.clone(),
            media_type: media_type.clone(),
            size: body.len() as u64,
            yanked: false,
            artifact_id: artifact.id().to_string(),
        };
        self.package_index_store
            .upsert_entry(
                repository.id(),
                &tag_coordinate,
                Some(artifact.id()),
                encode_entry(&tag_entry),
            )
            .await?;

        if !is_digest_reference(&reference) {
            let digest_version = PackageVersion::parse(digest.clone())
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            let digest_coordinate =
                PackageCoordinate::new(PackageEcosystem::Oci, package_name, digest_version);
            let digest_entry = ManifestEntry {
                reference: digest.clone(),
                ..tag_entry
            };
            self.package_index_store
                .upsert_entry(
                    repository.id(),
                    &digest_coordinate,
                    Some(artifact.id()),
                    encode_entry(&digest_entry),
                )
                .await?;
        }

        Ok(digest)
    }

    async fn get_manifest_one(
        &self,
        repository: &Repository,
        name: &str,
        reference: &str,
    ) -> Result<OciManifestDocument, PackagingError> {
        let name = normalize_oci_name(name)?;
        let reference = normalize_oci_reference(reference)?;
        let package_name = PackageName::parse(name.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let Some(entry) = self
            .load_manifest_entry(repository, &package_name, &reference)
            .await?
        else {
            return Err(PackagingError::PackageNotFound(name));
        };
        let artifact_id = parse_artifact_id(&entry.artifact_id)?;
        let body = self.download_stored(artifact_id).await?;
        Ok(OciManifestDocument {
            media_type: entry.media_type,
            digest: entry.digest,
            body,
        })
    }

    async fn list_tags_one(
        &self,
        repository: &Repository,
        name: &str,
    ) -> Result<Vec<String>, PackagingError> {
        let name = normalize_oci_name(name)?;
        let package_name = PackageName::parse(name.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Oci, &package_name)
            .await?;
        let mut tags = Vec::new();
        let mut seen = HashSet::new();
        for entry_bytes in entries {
            let entry: ManifestEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if is_digest_reference(&entry.reference) || !seen.insert(entry.reference.clone()) {
                continue;
            }
            tags.push(entry.reference);
        }
        if tags.is_empty() {
            return Err(PackagingError::PackageNotFound(name));
        }
        tags.sort();
        Ok(tags)
    }

    async fn ensure_referenced_blobs(
        &self,
        repository: &Repository,
        body: &[u8],
    ) -> Result<(), PackagingError> {
        let Ok(manifest) = serde_json::from_slice::<LooseManifest>(body) else {
            return Ok(());
        };
        if manifest.manifests.is_some() {
            return Ok(());
        }
        let mut missing = Vec::new();
        if let Some(config) = manifest.config
            && self
                .package_index_store
                .artifact_for(repository.id(), &blob_coordinate(&config.digest)?)
                .await?
                .is_none()
        {
            missing.push(config.digest);
        }
        for layer in manifest.layers.unwrap_or_default() {
            if self
                .package_index_store
                .artifact_for(repository.id(), &blob_coordinate(&layer.digest)?)
                .await?
                .is_none()
            {
                missing.push(layer.digest);
            }
        }
        if let Some(digest) = missing.into_iter().next() {
            return Err(PackagingError::FileNotFound(digest));
        }
        Ok(())
    }

    async fn download_stored(&self, artifact_id: ArtifactId) -> Result<Bytes, PackagingError> {
        let artifact = self
            .artifact_store
            .find_by_id(artifact_id)
            .await?
            .ok_or_else(|| PackagingError::FileNotFound(artifact_id.to_string()))?;
        let content = self.storage.get(&storage_key_for(artifact_id)).await?;
        let actual = sha256_checksum(&content);
        if actual != *artifact.checksum() {
            return Err(PackagingError::ChecksumMismatch {
                expected: artifact.checksum().to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(content)
    }

    async fn search_one(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_repository(repository.id(), PackageEcosystem::Oci)
            .await?;
        let query = query.to_ascii_lowercase();
        let mut hits = Vec::new();
        let mut seen = HashSet::new();
        for entry_bytes in entries {
            let Ok(entry) = serde_json::from_slice::<ManifestEntry>(&entry_bytes) else {
                continue;
            };
            if entry.name == BLOB_PACKAGE || is_digest_reference(&entry.reference) {
                continue;
            }
            if !query.is_empty() && !entry.name.to_ascii_lowercase().contains(&query) {
                continue;
            }
            if !seen.insert(entry.name.clone()) {
                continue;
            }
            hits.push(PackageSearchHit {
                name: entry.name,
                max_version: entry.reference,
            });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }
}

#[async_trait]
impl PackagingStrategy for OciPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Oci
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_oci_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let parsed: OciPublishBody = serde_json::from_slice(&payload)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        for blob in parsed.blobs {
            let content = BASE64
                .decode(blob.content.as_bytes())
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            self.put_blob_one(repository, &blob.digest, Bytes::from(content))
                .await?;
        }
        let manifest = match parsed.manifest {
            serde_json::Value::String(raw) => Bytes::from(raw),
            other => Bytes::from(
                serde_json::to_vec(&other)
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
            ),
        };
        let media_type = parsed
            .media_type
            .unwrap_or_else(|| DEFAULT_MANIFEST_MEDIA_TYPE.to_string());
        self.put_manifest_one(
            repository,
            &parsed.name,
            &parsed.reference,
            &media_type,
            manifest,
        )
        .await?;
        let name = PackageName::parse(normalize_oci_name(&parsed.name)?)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let version = PackageVersion::parse(normalize_oci_reference(&parsed.reference)?)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        Ok(PackageCoordinate::new(PackageEcosystem::Oci, name, version))
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        let tags = self.list_tags(repository, name.as_str()).await?;
        Ok(Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "name": normalize_oci_name(name.as_str())?,
                "tags": tags,
            }))
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
        ))
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Ok(self
            .get_manifest(
                repository,
                coordinate.name().as_str(),
                coordinate.version().as_str(),
            )
            .await?
            .body)
    }

    async fn download_file(
        &self,
        repository: &Repository,
        filename: &str,
    ) -> Result<Bytes, PackagingError> {
        self.get_blob_from(repository, filename).await
    }

    async fn put_blob(
        &self,
        repository: &Repository,
        digest: &str,
        body: Bytes,
    ) -> Result<u64, PackagingError> {
        Self::ensure_oci_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        self.put_blob_one(repository, digest, body).await
    }

    async fn put_manifest(
        &self,
        repository: &Repository,
        name: &str,
        reference: &str,
        media_type: &str,
        body: Bytes,
    ) -> Result<String, PackagingError> {
        Self::ensure_oci_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        self.put_manifest_one(repository, name, reference, media_type, body)
            .await
    }

    async fn get_manifest(
        &self,
        repository: &Repository,
        name: &str,
        reference: &str,
    ) -> Result<OciManifestDocument, PackagingError> {
        Self::ensure_oci_repository(repository)?;
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut last_error = PackagingError::PackageNotFound(name.to_string());
            for target in &targets {
                match self.get_manifest_one(target, name, reference).await {
                    Ok(document) => return Ok(document),
                    Err(
                        PackagingError::PackageNotFound(_)
                        | PackagingError::VersionNotFound(_)
                        | PackagingError::FileNotFound(_),
                    ) => {}
                    Err(error) => last_error = error,
                }
            }
            return Err(last_error);
        }
        self.get_manifest_one(repository, name, reference).await
    }

    async fn list_tags(
        &self,
        repository: &Repository,
        name: &str,
    ) -> Result<Vec<String>, PackagingError> {
        Self::ensure_oci_repository(repository)?;
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut tags = Vec::new();
            let mut seen = HashSet::new();
            let mut found = false;
            for target in &targets {
                match self.list_tags_one(target, name).await {
                    Ok(member_tags) => {
                        found = true;
                        for tag in member_tags {
                            if seen.insert(tag.clone()) {
                                tags.push(tag);
                            }
                        }
                    }
                    Err(PackagingError::PackageNotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            if !found {
                return Err(PackagingError::PackageNotFound(name.to_string()));
            }
            tags.sort();
            return Ok(tags);
        }
        self.list_tags_one(repository, name).await
    }

    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError> {
        Self::ensure_oci_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let Some(mut entry) = self
            .load_manifest_entry(
                repository,
                coordinate.name(),
                coordinate.version().as_str(),
            )
            .await?
        else {
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
        self.package_index_store
            .upsert_entry(
                repository.id(),
                coordinate,
                artifact_id,
                encode_entry(&entry),
            )
            .await?;
        Ok(())
    }

    async fn search(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        Self::ensure_oci_repository(repository)?;
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

impl OciPackagingStrategy {
    async fn get_blob_from(
        &self,
        repository: &Repository,
        digest: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_oci_repository(repository)?;
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut last_error = PackagingError::FileNotFound(digest.to_string());
            for target in &targets {
                match self.get_blob_one(target, digest).await {
                    Ok(body) => return Ok(body),
                    Err(PackagingError::FileNotFound(_) | PackagingError::PackageNotFound(_)) => {}
                    Err(error) => last_error = error,
                }
            }
            return Err(last_error);
        }
        self.get_blob_one(repository, digest).await
    }
}

/// Normaliza un nombre de imagen OCI: minúsculas, componentes
/// `[a-z0-9]+([._-][a-z0-9]+)*` separados por `/`.
///
/// # Errors
///
/// Devuelve [`PackagingError::InvalidPayload`] si el nombre está vacío,
/// es el reservado `_blob`, o un componente no es válido.
pub fn normalize_oci_name(name: &str) -> Result<String, PackagingError> {
    let name = name.trim().trim_matches('/').to_ascii_lowercase();
    if name.is_empty() || name == BLOB_PACKAGE {
        return Err(PackagingError::InvalidPayload(
            "invalid OCI image name".to_string(),
        ));
    }
    if !name.split('/').all(is_valid_name_component) {
        return Err(PackagingError::InvalidPayload(format!(
            "invalid OCI image name '{name}'"
        )));
    }
    Ok(name)
}

/// Valida un digest `sha256:` de 64 hexadecimales.
///
/// # Errors
///
/// Devuelve [`PackagingError::InvalidPayload`] si el formato no coincide.
pub fn parse_oci_digest(value: &str) -> Result<String, PackagingError> {
    let lower = value.trim().to_ascii_lowercase();
    let Some(hex) = lower.strip_prefix("sha256:") else {
        return Err(PackagingError::InvalidPayload(
            "digest must be sha256:<hex>".to_string(),
        ));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PackagingError::InvalidPayload(format!(
            "invalid sha256 digest '{value}'"
        )));
    }
    Ok(lower)
}

fn normalize_oci_reference(reference: &str) -> Result<String, PackagingError> {
    let reference = reference.trim();
    if is_digest_reference(reference) {
        return parse_oci_digest(reference);
    }
    if is_valid_tag(reference) {
        return Ok(reference.to_string());
    }
    Err(PackagingError::InvalidPayload(format!(
        "invalid OCI tag '{reference}'"
    )))
}

fn is_digest_reference(value: &str) -> bool {
    value.to_ascii_lowercase().starts_with("sha256:")
}

fn is_valid_name_component(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    let mut previous_separator = false;
    for character in characters {
        if matches!(character, '.' | '_' | '-') {
            if previous_separator {
                return false;
            }
            previous_separator = true;
        } else if character.is_ascii_lowercase() || character.is_ascii_digit() {
            previous_separator = false;
        } else {
            return false;
        }
    }
    !previous_separator
}

fn is_valid_tag(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() && first != '_' {
        return false;
    }
    let rest: String = characters.collect();
    rest.len() < 128
        && rest
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

fn content_digest(body: &[u8]) -> String {
    format!("sha256:{}", sha256_checksum(body))
}

fn blob_coordinate(digest: &str) -> Result<PackageCoordinate, PackagingError> {
    let name = PackageName::parse(BLOB_PACKAGE)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    let version = PackageVersion::parse(digest)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    Ok(PackageCoordinate::new(PackageEcosystem::Oci, name, version))
}

fn parse_artifact_id(value: &str) -> Result<ArtifactId, PackagingError> {
    Uuid::parse_str(value)
        .map(ArtifactId::from)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))
}

fn encode_entry<T: Serialize>(entry: &T) -> Bytes {
    Bytes::from(serde_json::to_vec(entry).expect("index entry always serializes"))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ManifestEntry {
    name: String,
    reference: String,
    digest: String,
    media_type: String,
    size: u64,
    #[serde(default)]
    yanked: bool,
    artifact_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BlobEntry {
    digest: String,
    artifact_id: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct OciPublishBody {
    name: String,
    reference: String,
    media_type: Option<String>,
    manifest: serde_json::Value,
    #[serde(default)]
    blobs: Vec<OciPublishBlob>,
}

#[derive(Debug, Deserialize)]
struct OciPublishBlob {
    digest: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct LooseManifest {
    config: Option<LooseDescriptor>,
    layers: Option<Vec<LooseDescriptor>>,
    manifests: Option<Vec<LooseDescriptor>>,
}

#[derive(Debug, Deserialize)]
struct LooseDescriptor {
    digest: String,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::{PackageName, PackageVersion};
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryStorage,
    };

    fn strategy() -> OciPackagingStrategy {
        OciPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        )
    }

    fn oci_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap()
    }

    fn blob(content: &[u8]) -> (String, Bytes) {
        let body = Bytes::from(content.to_vec());
        (content_digest(&body), body)
    }

    fn manifest_for(config: &str, layer: &str, config_size: usize, layer_size: usize) -> Bytes {
        Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
                "config": {
                    "mediaType": "application/vnd.oci.image.config.v1+json",
                    "digest": config,
                    "size": config_size
                },
                "layers": [{
                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                    "digest": layer,
                    "size": layer_size
                }]
            }))
            .unwrap(),
        )
    }

    #[test]
    fn normalize_oci_name_lowercases_and_rejects_invalid() {
        assert_eq!(normalize_oci_name("Demo").unwrap(), "demo");
        assert!(normalize_oci_name("_blob").is_err());
        assert!(normalize_oci_name("Not Valid").is_err());
    }

    #[tokio::test]
    async fn put_blob_then_download_by_digest() {
        let strategy = strategy();
        let repository = oci_forge("oci-local");
        let (digest, body) = blob(b"layer-bytes");
        strategy
            .put_blob(&repository, &digest, body.clone())
            .await
            .unwrap();
        let downloaded = strategy.download_file(&repository, &digest).await.unwrap();
        assert_eq!(downloaded, body);
        let again = strategy
            .put_blob(&repository, &digest, body)
            .await
            .unwrap();
        assert_eq!(again, 11);
    }

    #[tokio::test]
    async fn put_manifest_indexes_tag_and_digest() {
        let strategy = strategy();
        let repository = oci_forge("oci-local");
        let (config_digest, config) = blob(b"{\"architecture\":\"amd64\"}");
        let (layer_digest, layer) = blob(b"layer");
        strategy
            .put_blob(&repository, &config_digest, config.clone())
            .await
            .unwrap();
        strategy
            .put_blob(&repository, &layer_digest, layer.clone())
            .await
            .unwrap();
        let manifest = manifest_for(
            &config_digest,
            &layer_digest,
            config.len(),
            layer.len(),
        );
        let digest = strategy
            .put_manifest(
                &repository,
                "demo",
                "latest",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest.clone(),
            )
            .await
            .unwrap();
        let by_tag = strategy
            .get_manifest(&repository, "demo", "latest")
            .await
            .unwrap();
        assert_eq!(by_tag.body, manifest);
        assert_eq!(by_tag.digest, digest);
        let by_digest = strategy
            .get_manifest(&repository, "demo", &digest)
            .await
            .unwrap();
        assert_eq!(by_digest.body, manifest);
        assert_eq!(
            strategy.list_tags(&repository, "demo").await.unwrap(),
            vec!["latest".to_string()]
        );
    }

    #[tokio::test]
    async fn yank_marks_the_tag() {
        let strategy = strategy();
        let repository = oci_forge("oci-local");
        let (config_digest, config) = blob(b"cfg");
        let (layer_digest, layer) = blob(b"lyr");
        strategy
            .put_blob(&repository, &config_digest, config.clone())
            .await
            .unwrap();
        strategy
            .put_blob(&repository, &layer_digest, layer.clone())
            .await
            .unwrap();
        strategy
            .put_manifest(
                &repository,
                "demo",
                "1.0.0",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest_for(&config_digest, &layer_digest, config.len(), layer.len()),
            )
            .await
            .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();
        let entry = strategy
            .load_manifest_entry(&repository, coordinate.name(), "1.0.0")
            .await
            .unwrap()
            .unwrap();
        assert!(entry.yanked);
    }

    #[tokio::test]
    async fn rejects_publishing_to_an_alloy() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let forge = oci_forge("oci-member");
        repository_store.save(&forge).await.unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("oci-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge.id()],
            },
            PackageEcosystem::Oci,
        )
        .unwrap();
        let strategy = OciPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            repository_store,
        );
        let (digest, body) = blob(b"nope");
        let error = strategy.put_blob(&alloy, &digest, body).await.unwrap_err();
        assert!(matches!(error, PackagingError::ReadOnlyRepository));
    }

    #[tokio::test]
    async fn alloy_index_and_download_see_forge_member_files() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let forge = oci_forge("oci-member");
        repository_store.save(&forge).await.unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("oci-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge.id()],
            },
            PackageEcosystem::Oci,
        )
        .unwrap();
        repository_store.save(&alloy).await.unwrap();
        let strategy = OciPackagingStrategy::new(
            artifact_store,
            package_index_store,
            storage,
            repository_store,
        );
        let (config_digest, config) = blob(b"cfg");
        let (layer_digest, layer) = blob(b"lyr");
        strategy
            .put_blob(&forge, &config_digest, config.clone())
            .await
            .unwrap();
        strategy
            .put_blob(&forge, &layer_digest, layer.clone())
            .await
            .unwrap();
        strategy
            .put_manifest(
                &forge,
                "demo",
                "latest",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest_for(&config_digest, &layer_digest, config.len(), layer.len()),
            )
            .await
            .unwrap();
        assert_eq!(
            strategy.list_tags(&alloy, "demo").await.unwrap(),
            vec!["latest".to_string()]
        );
        let pulled = strategy
            .get_manifest(&alloy, "demo", "latest")
            .await
            .unwrap();
        assert!(!pulled.body.is_empty());
        let layer_body = strategy
            .download_file(&alloy, &layer_digest)
            .await
            .unwrap();
        assert_eq!(layer_body.as_ref(), b"lyr");
    }
}
