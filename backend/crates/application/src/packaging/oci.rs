//! Packaging strategy for the OCI ecosystem: implements the
//! subset of Distribution Spec v2 that `docker push` and
//! `docker pull` need against a `FerroBox` repository.
//!
//! Reference: <https://github.com/opencontainers/distribution-spec>.
//!
//! Covers **Forge** (blobs, manifests, tags, and yank), **Mirror**
//! (*pull-through* cache of an OCI registry such as Docker Hub), and reads
//! on **Alloy**. The same strategy serves the **Helm** ecosystem: charts
//! are published with `helm push` as OCI artifacts.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::ArtifactStore;
use ferrobox_ports::http_client::{HttpClient, HttpClientError, HttpResponse};
use ferrobox_ports::mirror_credential_store::MirrorCredentialStore;
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::repository_store::RepositoryStore;
use ferrobox_ports::storage::StoragePort;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use super::{
    OciManifestDocument, PackageSearchHit, PackagingError, PackagingStrategy, PromoteOutcome,
    PublishOutcome, copy_stored_artifact, cosign, ensure_quota, notify_assay,
};
use crate::admission::AdmissionService;
use crate::assay::AssayService;
use crate::content_hash::sha256_checksum;
use crate::quota::QuotaService;
use crate::storage_key::storage_key_for;

/// Reserved index name for blobs (layers and configs).
const BLOB_PACKAGE: &str = "_blob";

/// Default media type of an OCI manifest.
pub const DEFAULT_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// Media type of an OCI index (multi-arch lists and Referrers API).
pub const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";

const MANIFEST_ACCEPT: &str = "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.index.v1+json, application/vnd.oci.artifact.manifest.v1+json";

/// Packaging strategy for OCI and Helm (charts as OCI artifacts).
pub struct OciPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    repository_store: Arc<dyn RepositoryStore>,
    http_client: Arc<dyn HttpClient>,
    ecosystem: PackageEcosystem,
    assays: Option<AssayService>,
    quota: Option<QuotaService>,
    admission: Option<AdmissionService>,
    upstream_credentials: Option<Arc<dyn MirrorCredentialStore>>,
}

impl OciPackagingStrategy {
    /// Builds the OCI strategy from its ports.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        repository_store: Arc<dyn RepositoryStore>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self::for_ecosystem(
            PackageEcosystem::Oci,
            artifact_store,
            package_index_store,
            storage,
            repository_store,
            http_client,
        )
    }

    /// Builds the strategy for OCI or Helm.
    #[must_use]
    pub fn for_ecosystem(
        ecosystem: PackageEcosystem,
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        repository_store: Arc<dyn RepositoryStore>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
            repository_store,
            http_client,
            ecosystem,
            assays: None,
            quota: None,
            admission: None,
            upstream_credentials: None,
        }
    }

    /// Connects automatic assay when publishing or caching a manifest.
    #[must_use]
    pub fn with_assays(mut self, assays: AssayService) -> Self {
        self.assays = Some(assays);
        self
    }

    /// Applies the storage quota when publishing or caching.
    #[must_use]
    pub fn with_quota(mut self, quota: QuotaService) -> Self {
        self.quota = Some(quota);
        self
    }

    /// Applies the admission policy when reading a manifest.
    #[must_use]
    pub fn with_admission(mut self, admission: AdmissionService) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Sends this store's secret when the mirror calls its upstream.
    #[must_use]
    pub fn with_upstream_credentials(mut self, store: Arc<dyn MirrorCredentialStore>) -> Self {
        self.upstream_credentials = Some(store);
        self
    }

    fn ensure_repository(&self, repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != self.ecosystem {
            return Err(PackagingError::EcosystemMismatch {
                expected: self.ecosystem.label(),
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
            _ => None,
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

        let coordinate = blob_coordinate(self.ecosystem, &digest)?;
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
        ensure_quota(self.quota.as_ref(), repository.id(), body.len() as u64).await?;
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
        let coordinate = blob_coordinate(self.ecosystem, &digest)?;
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
            .entries_for_package(repository.id(), self.ecosystem, name)
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
        if !matches!(repository.kind(), RepositoryKind::Mirror { .. }) {
            self.ensure_referenced_blobs(repository, &body).await?;
        }

        let package_name = PackageName::parse(name.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let tag_version = PackageVersion::parse(reference.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let tag_coordinate =
            PackageCoordinate::new(self.ecosystem, package_name.clone(), tag_version);

        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(repository.id(), checksum, body.len() as u64);
        ensure_quota(self.quota.as_ref(), repository.id(), body.len() as u64).await?;
        self.storage
            .put(&storage_key_for(artifact.id()), body.clone())
            .await?;
        self.artifact_store.save(&artifact).await?;

        let media_type = if media_type.trim().is_empty() {
            DEFAULT_MANIFEST_MEDIA_TYPE.to_string()
        } else {
            media_type.trim().to_string()
        };
        let accessory = cosign::inspect(&reference, &media_type, &body);

        let tag_entry = ManifestEntry {
            name: name.clone(),
            reference: reference.clone(),
            digest: digest.clone(),
            media_type: media_type.clone(),
            size: body.len() as u64,
            yanked: false,
            artifact_id: artifact.id().to_string(),
            subject: accessory.subject.clone(),
            accessory: accessory.kind.map(|kind| kind.as_str().to_string()),
            artifact_type: accessory.artifact_type.clone(),
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
                PackageCoordinate::new(self.ecosystem, package_name, digest_version);
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
            if accessory.kind.is_none() {
                notify_assay(self.assays.as_ref(), repository.id(), &tag_coordinate);
            }
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
            .entries_for_package(repository.id(), self.ecosystem, &package_name)
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

    async fn enforce_pull_policy(
        &self,
        repository: &Repository,
        name: &str,
        reference: &str,
        document: &OciManifestDocument,
    ) -> Result<(), PackagingError> {
        let Some(admission) = &self.admission else {
            return Ok(());
        };
        if !matches!(
            repository.kind(),
            RepositoryKind::Forge | RepositoryKind::Mirror { .. }
        ) {
            return Ok(());
        }
        if cosign::is_accessory_tag(reference)
            || self
                .manifest_is_accessory(repository, name, reference)
                .await?
        {
            return Ok(());
        }
        let signed = self
            .image_is_signed(repository, name, &document.digest)
            .await?;
        let keys = admission.public_keys_pem(repository.id()).await;
        let verified = self
            .image_is_verified(repository, name, &document.digest, &keys)
            .await?;
        admission
            .enforce_pull(repository.id(), name, reference, signed, verified)
            .await
    }

    async fn manifest_is_accessory(
        &self,
        repository: &Repository,
        name: &str,
        reference: &str,
    ) -> Result<bool, PackagingError> {
        let Ok(package_name) = PackageName::parse(name.to_string()) else {
            return Ok(false);
        };
        let Some(entry) = self
            .load_manifest_entry(repository, &package_name, reference)
            .await?
        else {
            return Ok(false);
        };
        Ok(entry.accessory.is_some() || cosign::is_accessory_tag(&entry.reference))
    }

    async fn image_is_signed(
        &self,
        repository: &Repository,
        name: &str,
        digest: &str,
    ) -> Result<bool, PackagingError> {
        let Ok(package_name) = PackageName::parse(name.to_string()) else {
            return Ok(false);
        };
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), self.ecosystem, &package_name)
            .await?;
        for entry_bytes in entries {
            let Ok(entry) = serde_json::from_slice::<ManifestEntry>(&entry_bytes) else {
                continue;
            };
            if !cosign::is_signature_accessory(entry.accessory.as_deref(), &entry.reference) {
                continue;
            }
            if cosign::signature_subject(entry.subject.as_deref(), &entry.reference).as_deref()
                == Some(digest)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn image_is_verified(
        &self,
        repository: &Repository,
        name: &str,
        digest: &str,
        keys_pem: &str,
    ) -> Result<bool, PackagingError> {
        if keys_pem.trim().is_empty() {
            return Ok(false);
        }
        let Ok(package_name) = PackageName::parse(name.to_string()) else {
            return Ok(false);
        };
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), self.ecosystem, &package_name)
            .await?;
        for entry_bytes in entries {
            let Ok(entry) = serde_json::from_slice::<ManifestEntry>(&entry_bytes) else {
                continue;
            };
            if !cosign::is_signature_accessory(entry.accessory.as_deref(), &entry.reference) {
                continue;
            }
            if cosign::signature_subject(entry.subject.as_deref(), &entry.reference).as_deref()
                != Some(digest)
            {
                continue;
            }
            let Ok(artifact_id) = parse_artifact_id(&entry.artifact_id) else {
                continue;
            };
            let Ok(manifest) = self.download_stored(artifact_id).await else {
                continue;
            };
            let Some(signature) = cosign::extract_simple_signature(&manifest) else {
                continue;
            };
            let Ok(payload) = self
                .get_blob_one(repository, &signature.payload_digest)
                .await
            else {
                continue;
            };
            if cosign::verify_simple(&payload, &signature.signature_b64, keys_pem, Some(digest)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn list_referrers_one(
        &self,
        repository: &Repository,
        name: &str,
        digest: &str,
        artifact_type: Option<&str>,
    ) -> Result<OciManifestDocument, PackagingError> {
        let name = normalize_oci_name(name)?;
        let digest = parse_oci_digest(digest)?;
        let package_name = PackageName::parse(name)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), self.ecosystem, &package_name)
            .await?;
        let mut manifests = Vec::new();
        let mut seen = HashSet::new();
        for entry_bytes in entries {
            let Ok(entry) = serde_json::from_slice::<ManifestEntry>(&entry_bytes) else {
                continue;
            };
            let Some(subject) =
                cosign::signature_subject(entry.subject.as_deref(), &entry.reference)
            else {
                continue;
            };
            if subject != digest || !seen.insert(entry.digest.clone()) {
                continue;
            }
            if let Some(filter) = artifact_type
                && entry
                    .artifact_type
                    .as_deref()
                    .is_none_or(|value| !value.eq_ignore_ascii_case(filter))
            {
                continue;
            }
            let mut descriptor = serde_json::json!({
                "mediaType": entry.media_type,
                "digest": entry.digest,
                "size": entry.size,
            });
            if let Some(artifact) = entry.artifact_type {
                descriptor
                    .as_object_mut()
                    .expect("descriptor is an object")
                    .insert(
                        "artifactType".to_string(),
                        serde_json::Value::String(artifact),
                    );
            }
            manifests.push(descriptor);
        }
        Ok(referrers_index(manifests))
    }

    async fn refresh_referrers_from_upstream(
        &self,
        repository: &Repository,
        name: &str,
        digest: &str,
        artifact_type: Option<&str>,
    ) -> Result<OciManifestDocument, PackagingError> {
        let Some(upstream) = Self::mirror_upstream(repository) else {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        };
        let local_name = normalize_oci_name(name)?;
        let digest = parse_oci_digest(digest)?;
        let image = upstream_image_name(upstream, &local_name, self.ecosystem);
        let mut url = join_v2_path(upstream, &format!("{image}/referrers/{digest}"));
        if let Some(filter) = artifact_type {
            url.push_str("?artifactType=");
            url.push_str(&query_escape(filter));
        }
        let response = self
            .registry_get_authed(
                repository.id(),
                &url,
                vec![("Accept".to_string(), INDEX_MEDIA_TYPE.to_string())],
            )
            .await?;
        Ok(OciManifestDocument {
            media_type: content_type_of(&response, INDEX_MEDIA_TYPE),
            digest: content_digest(&response.body),
            body: response.body,
        })
    }

    async fn copy_accessories_for(
        &self,
        source: &Repository,
        target: &Repository,
        image_name: &str,
        subject_digest: &str,
    ) -> Result<(u32, u64), PackagingError> {
        let package_name = PackageName::parse(image_name.to_string())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let entries = self
            .package_index_store
            .entries_for_package(source.id(), self.ecosystem, &package_name)
            .await?;
        let mut artifacts_copied = 0_u32;
        let mut bytes_copied = 0_u64;
        let mut seen = HashSet::new();
        for entry_bytes in entries {
            let Ok(entry) = serde_json::from_slice::<ManifestEntry>(&entry_bytes) else {
                continue;
            };
            let Some(subject) =
                cosign::signature_subject(entry.subject.as_deref(), &entry.reference)
            else {
                continue;
            };
            if subject != subject_digest || !seen.insert(entry.digest.clone()) {
                continue;
            }
            let (artifact_id, mut copied, copied_count, copied_bytes) = self
                .copy_image_into(source, target, image_name, &entry.digest)
                .await?;
            artifacts_copied += copied_count;
            bytes_copied += copied_bytes;
            if is_digest_reference(&entry.reference) {
                continue;
            }
            copied.reference = entry.reference.clone();
            copied.artifact_id = artifact_id.to_string();
            copied.yanked = false;
            let tag_version = PackageVersion::parse(entry.reference)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            let tag_coordinate =
                PackageCoordinate::new(self.ecosystem, package_name.clone(), tag_version);
            self.package_index_store
                .upsert_entry(
                    target.id(),
                    &tag_coordinate,
                    Some(artifact_id),
                    encode_entry(&copied),
                )
                .await?;
        }
        Ok((artifacts_copied, bytes_copied))
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
                .artifact_for(
                    repository.id(),
                    &blob_coordinate(self.ecosystem, &config.digest)?,
                )
                .await?
                .is_none()
        {
            missing.push(config.digest);
        }
        for layer in manifest
            .layers
            .into_iter()
            .flatten()
            .chain(manifest.blobs.into_iter().flatten())
        {
            if self
                .package_index_store
                .artifact_for(
                    repository.id(),
                    &blob_coordinate(self.ecosystem, &layer.digest)?,
                )
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

    async fn copy_blob_if_missing(
        &self,
        source_id: ArtifactId,
        target: &Repository,
        digest: &str,
        blob_coord: &PackageCoordinate,
    ) -> Result<(u32, u64), PackagingError> {
        if self
            .package_index_store
            .artifact_for(target.id(), blob_coord)
            .await?
            .is_some()
        {
            return Ok((0, 0));
        }
        let (new_id, size) = copy_stored_artifact(
            self.artifact_store.as_ref(),
            self.storage.as_ref(),
            self.quota.as_ref(),
            source_id,
            target.id(),
        )
        .await?;
        let blob_entry = BlobEntry {
            digest: digest.to_string(),
            artifact_id: new_id.to_string(),
            size,
        };
        self.package_index_store
            .upsert_entry(
                target.id(),
                blob_coord,
                Some(new_id),
                encode_entry(&blob_entry),
            )
            .await?;
        Ok((1, size))
    }

    async fn copy_manifest_if_missing(
        &self,
        source_id: ArtifactId,
        target: &Repository,
        digest: &str,
        entry: &ManifestEntry,
        digest_coord: &PackageCoordinate,
    ) -> Result<(ArtifactId, u32, u64), PackagingError> {
        if let Some(existing) = self
            .package_index_store
            .artifact_for(target.id(), digest_coord)
            .await?
        {
            return Ok((existing, 0, 0));
        }
        let (new_id, size) = copy_stored_artifact(
            self.artifact_store.as_ref(),
            self.storage.as_ref(),
            self.quota.as_ref(),
            source_id,
            target.id(),
        )
        .await?;
        let digest_entry = ManifestEntry {
            reference: digest.to_string(),
            yanked: false,
            artifact_id: new_id.to_string(),
            ..entry.clone()
        };
        self.package_index_store
            .upsert_entry(
                target.id(),
                digest_coord,
                Some(new_id),
                encode_entry(&digest_entry),
            )
            .await?;
        Ok((new_id, 1, size))
    }

    async fn copy_image_into(
        &self,
        source: &Repository,
        target: &Repository,
        image_name: &str,
        root_digest: &str,
    ) -> Result<(ArtifactId, ManifestEntry, u32, u64), PackagingError> {
        let mut pending = vec![root_digest.to_string()];
        let mut seen = HashSet::new();
        let mut artifacts_copied = 0_u32;
        let mut bytes_copied = 0_u64;
        let mut root_artifact = None;
        let mut root_entry = None;

        while let Some(digest) = pending.pop() {
            if !seen.insert(digest.clone()) {
                continue;
            }

            let blob_coord = blob_coordinate(self.ecosystem, &digest)?;
            if let Some(source_id) = self
                .package_index_store
                .artifact_for(source.id(), &blob_coord)
                .await?
            {
                let (copied, size) = self
                    .copy_blob_if_missing(source_id, target, &digest, &blob_coord)
                    .await?;
                artifacts_copied += copied;
                bytes_copied += size;
                continue;
            }

            let package_name = PackageName::parse(image_name.to_string())
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            let Some(entry) = self
                .load_manifest_entry(source, &package_name, &digest)
                .await?
            else {
                return Err(PackagingError::FileNotFound(digest));
            };
            let source_id = parse_artifact_id(&entry.artifact_id)?;
            let body = self.download_stored(source_id).await?;
            enqueue_manifest_digests(&body, &mut pending);

            let digest_version = PackageVersion::parse(digest.clone())
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            let digest_coord = PackageCoordinate::new(self.ecosystem, package_name, digest_version);
            let (artifact_id, copied, size) = self
                .copy_manifest_if_missing(source_id, target, &digest, &entry, &digest_coord)
                .await?;
            artifacts_copied += copied;
            bytes_copied += size;
            if digest == root_digest {
                root_artifact = Some(artifact_id);
                root_entry = Some(entry);
            }
        }

        let artifact_id =
            root_artifact.ok_or_else(|| PackagingError::FileNotFound(root_digest.to_string()))?;
        let entry =
            root_entry.ok_or_else(|| PackagingError::FileNotFound(root_digest.to_string()))?;
        Ok((artifact_id, entry, artifacts_copied, bytes_copied))
    }

    async fn search_one(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_repository(repository.id(), self.ecosystem)
            .await?;
        let query = query.to_ascii_lowercase();
        let mut hits = Vec::new();
        let mut seen = HashSet::new();
        for entry_bytes in entries {
            let Ok(entry) = serde_json::from_slice::<ManifestEntry>(&entry_bytes) else {
                continue;
            };
            if entry.name == BLOB_PACKAGE
                || is_digest_reference(&entry.reference)
                || entry.accessory.is_some()
                || cosign::is_accessory_tag(&entry.reference)
            {
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

    async fn refresh_manifest_from_upstream(
        &self,
        repository: &Repository,
        name: &str,
        reference: &str,
    ) -> Result<OciManifestDocument, PackagingError> {
        let Some(upstream) = Self::mirror_upstream(repository) else {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        };
        let local_name = normalize_oci_name(name)?;
        let reference = normalize_oci_reference(reference)?;
        let image = upstream_image_name(upstream, &local_name, self.ecosystem);
        let url = join_v2_path(upstream, &format!("{image}/manifests/{reference}"));
        let response = self
            .registry_get_authed(
                repository.id(),
                &url,
                vec![("Accept".to_string(), MANIFEST_ACCEPT.to_string())],
            )
            .await
            .map_err(|err| match err {
                PackagingError::FileNotFound(_) => {
                    PackagingError::PackageNotFound(local_name.clone())
                }
                other => other,
            })?;
        if response.body.is_empty() {
            return Err(PackagingError::InvalidUpstream(
                "upstream manifest is empty".to_string(),
            ));
        }
        let media_type = content_type_of(&response, DEFAULT_MANIFEST_MEDIA_TYPE);
        let digest = self
            .put_manifest_one(
                repository,
                &local_name,
                &reference,
                &media_type,
                response.body.clone(),
            )
            .await?;
        Ok(OciManifestDocument {
            media_type,
            digest,
            body: response.body,
        })
    }

    async fn refresh_blob_from_upstream(
        &self,
        repository: &Repository,
        name: &str,
        digest: &str,
    ) -> Result<Bytes, PackagingError> {
        let Some(upstream) = Self::mirror_upstream(repository) else {
            return Err(PackagingError::FileNotFound(digest.to_string()));
        };
        let local_name = normalize_oci_name(name)?;
        let digest = parse_oci_digest(digest)?;
        let image = upstream_image_name(upstream, &local_name, self.ecosystem);
        let url = join_v2_path(upstream, &format!("{image}/blobs/{digest}"));
        let response = self
            .registry_get_authed(repository.id(), &url, Vec::new())
            .await?;
        self.put_blob_one(repository, &digest, response.body.clone())
            .await?;
        if let Some(assays) = &self.assays {
            assays.reschedule_for_name(repository.id(), &local_name);
        }
        Ok(response.body)
    }

    async fn registry_get_authed(
        &self,
        repository_id: RepositoryId,
        url: &str,
        extra_headers: Vec<(String, String)>,
    ) -> Result<HttpResponse, PackagingError> {
        let stored = super::upstream::upstream_authorization(
            self.upstream_credentials.as_deref(),
            repository_id,
        )
        .await?;
        let response = self
            .exchange(url, extra_headers.clone(), None, stored.as_deref())
            .await?;
        if response.is_success() {
            return Ok(response);
        }
        if response.status != 401 {
            return Err(registry_status_error(url, response.status));
        }
        let challenge = bearer_challenge(&response).ok_or_else(|| {
            PackagingError::InvalidUpstream(
                "upstream 401 without a Bearer WWW-Authenticate challenge".to_string(),
            )
        })?;
        let token = self
            .fetch_bearer_token(challenge, stored.as_deref())
            .await?;
        let retry = self
            .exchange(url, extra_headers, Some(&token), None)
            .await?;
        if retry.is_success() {
            return Ok(retry);
        }
        Err(registry_status_error(url, retry.status))
    }

    async fn exchange(
        &self,
        url: &str,
        extra_headers: Vec<(String, String)>,
        bearer: Option<&str>,
        stored_authorization: Option<&str>,
    ) -> Result<HttpResponse, PackagingError> {
        let mut headers = extra_headers;
        if let Some(token) = bearer {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        } else if let Some(authorization) = stored_authorization
            && !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        {
            headers.push(("Authorization".to_string(), authorization.to_string()));
        }
        let refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        Ok(self.http_client.get_with_headers(url, &refs).await?)
    }

    async fn fetch_bearer_token(
        &self,
        challenge: &str,
        stored: Option<&str>,
    ) -> Result<String, PackagingError> {
        let parsed = parse_bearer_challenge(challenge)?;
        let token_url = token_request_url(&parsed.realm, &parsed.service, parsed.scope.as_deref())?;
        if stored.is_some()
            && let Ok(token) = self.token_endpoint(&token_url, stored).await
        {
            return Ok(token);
        }
        self.token_endpoint(&token_url, None).await
    }

    async fn token_endpoint(
        &self,
        token_url: &str,
        authorization: Option<&str>,
    ) -> Result<String, PackagingError> {
        let headers: Vec<(&str, &str)> = match authorization {
            Some(value) => vec![("authorization", value)],
            None => Vec::new(),
        };
        let response = self
            .http_client
            .get_with_headers(token_url, &headers)
            .await?;
        if !response.is_success() {
            return Err(PackagingError::InvalidUpstream(format!(
                "token endpoint returned HTTP {}",
                response.status
            )));
        }
        parse_registry_token(&response.body)
    }
}

#[async_trait]
impl PackagingStrategy for OciPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        self.ecosystem
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        self.ensure_repository(repository)?;
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
        Ok(PackageCoordinate::new(self.ecosystem, name, version))
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
        self.get_blob_from(repository, "", filename).await
    }

    async fn put_blob(
        &self,
        repository: &Repository,
        digest: &str,
        body: Bytes,
    ) -> Result<u64, PackagingError> {
        self.ensure_repository(repository)?;
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
        self.ensure_repository(repository)?;
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
        self.ensure_repository(repository)?;
        let targets = if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            self.resolve_read_targets(repository).await?
        } else {
            vec![repository.clone()]
        };
        let mut last_error = PackagingError::PackageNotFound(name.to_string());
        for target in &targets {
            match self.get_manifest_one(target, name, reference).await {
                Ok(document) => {
                    self.enforce_pull_policy(target, name, reference, &document)
                        .await?;
                    return Ok(document);
                }
                Err(
                    PackagingError::PackageNotFound(_)
                    | PackagingError::VersionNotFound(_)
                    | PackagingError::FileNotFound(_),
                ) => {
                    if Self::mirror_upstream(target).is_some() {
                        match self
                            .refresh_manifest_from_upstream(target, name, reference)
                            .await
                        {
                            Ok(document) => {
                                self.enforce_pull_policy(target, name, reference, &document)
                                    .await?;
                                return Ok(document);
                            }
                            Err(
                                PackagingError::PackageNotFound(_)
                                | PackagingError::VersionNotFound(_)
                                | PackagingError::FileNotFound(_),
                            ) => {}
                            Err(error) => last_error = error,
                        }
                    }
                }
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    async fn get_blob(
        &self,
        repository: &Repository,
        name: &str,
        digest: &str,
    ) -> Result<Bytes, PackagingError> {
        self.get_blob_from(repository, name, digest).await
    }

    async fn list_tags(
        &self,
        repository: &Repository,
        name: &str,
    ) -> Result<Vec<String>, PackagingError> {
        self.ensure_repository(repository)?;
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

    async fn list_referrers(
        &self,
        repository: &Repository,
        name: &str,
        digest: &str,
        artifact_type: Option<&str>,
    ) -> Result<OciManifestDocument, PackagingError> {
        self.ensure_repository(repository)?;
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut manifests = Vec::new();
            let mut seen = HashSet::new();
            for target in &targets {
                match self
                    .list_referrers_one(target, name, digest, artifact_type)
                    .await
                {
                    Ok(document) => {
                        append_referrer_descriptors(&document.body, &mut manifests, &mut seen);
                    }
                    Err(PackagingError::PackageNotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            return Ok(referrers_index(manifests));
        }
        match self
            .list_referrers_one(repository, name, digest, artifact_type)
            .await
        {
            Ok(document) if referrer_count(&document.body) > 0 => Ok(document),
            Ok(local) => {
                if Self::mirror_upstream(repository).is_some() {
                    match self
                        .refresh_referrers_from_upstream(repository, name, digest, artifact_type)
                        .await
                    {
                        Ok(upstream) => Ok(upstream),
                        Err(
                            PackagingError::PackageNotFound(_)
                            | PackagingError::FileNotFound(_)
                            | PackagingError::InvalidUpstream(_)
                            | PackagingError::Upstream(_),
                        ) => Ok(local),
                        Err(error) => Err(error),
                    }
                } else {
                    Ok(local)
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn promote_version(
        &self,
        source: &Repository,
        target: &Repository,
        coordinate: &PackageCoordinate,
        preserve_yanked: bool,
    ) -> Result<PromoteOutcome, PackagingError> {
        self.ensure_repository(source)?;
        self.ensure_repository(target)?;
        if Self::is_read_only(target) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        if coordinate.name().as_str() == BLOB_PACKAGE {
            return Err(PackagingError::InvalidPayload(
                "OCI blobs cannot be promoted on their own".to_string(),
            ));
        }

        let reference = coordinate.version().as_str();
        let Some(source_entry) = self
            .load_manifest_entry(source, coordinate.name(), reference)
            .await?
        else {
            return Err(PackagingError::VersionNotFound(coordinate.clone()));
        };

        if self
            .package_index_store
            .artifact_for(target.id(), coordinate)
            .await?
            .is_some()
        {
            return Err(PackagingError::AlreadyPublished(coordinate.clone()));
        }

        let (artifact_id, mut entry, mut artifacts_copied, mut bytes_copied) = self
            .copy_image_into(
                source,
                target,
                coordinate.name().as_str(),
                &source_entry.digest,
            )
            .await?;

        if !is_digest_reference(reference) {
            entry.reference = reference.to_string();
            entry.artifact_id = artifact_id.to_string();
            if !preserve_yanked {
                entry.yanked = false;
            }
            self.package_index_store
                .upsert_entry(
                    target.id(),
                    coordinate,
                    Some(artifact_id),
                    encode_entry(&entry),
                )
                .await?;
        }

        if source_entry.accessory.is_none() {
            let (copied, size) = self
                .copy_accessories_for(
                    source,
                    target,
                    coordinate.name().as_str(),
                    &source_entry.digest,
                )
                .await?;
            artifacts_copied += copied;
            bytes_copied += size;
            notify_assay(self.assays.as_ref(), target.id(), coordinate);
        }

        Ok(PromoteOutcome {
            coordinate: coordinate.clone(),
            artifacts_copied,
            bytes_copied,
        })
    }

    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError> {
        self.ensure_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let Some(mut entry) = self
            .load_manifest_entry(repository, coordinate.name(), coordinate.version().as_str())
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
        self.ensure_repository(repository)?;
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
        name: &str,
        digest: &str,
    ) -> Result<Bytes, PackagingError> {
        self.ensure_repository(repository)?;
        let targets = if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            self.resolve_read_targets(repository).await?
        } else {
            vec![repository.clone()]
        };
        let mut last_error = PackagingError::FileNotFound(digest.to_string());
        for target in &targets {
            match self.get_blob_one(target, digest).await {
                Ok(body) => return Ok(body),
                Err(PackagingError::FileNotFound(_) | PackagingError::PackageNotFound(_)) => {
                    if !name.is_empty() && Self::mirror_upstream(target).is_some() {
                        match self.refresh_blob_from_upstream(target, name, digest).await {
                            Ok(body) => return Ok(body),
                            Err(
                                PackagingError::FileNotFound(_)
                                | PackagingError::PackageNotFound(_),
                            ) => {}
                            Err(error) => last_error = error,
                        }
                    }
                }
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }
}

/// Normalizes an OCI image name: lowercase, components
/// `[a-z0-9]+([._-][a-z0-9]+)*` separated by `/`.
///
/// # Errors
///
/// Returns [`PackagingError::InvalidPayload`] if the name is empty,
/// is the reserved `_blob`, or a component is invalid.
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

/// Validates a `sha256:` digest of 64 hex digits.
///
/// # Errors
///
/// Returns [`PackagingError::InvalidPayload`] if the format does not match.
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
        && rest.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn content_digest(body: &[u8]) -> String {
    format!("sha256:{}", sha256_checksum(body))
}

fn query_escape(value: &str) -> String {
    let mut escaped = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                escaped.push(char::from(byte));
            }
            _ => {
                escaped.push('%');
                escaped.push(char::from(hex_digit(byte >> 4)));
                escaped.push(char::from(hex_digit(byte & 0x0f)));
            }
        }
    }
    escaped
}

fn hex_digit(value: u8) -> u8 {
    if value < 10 {
        b'0' + value
    } else {
        b'A' + (value - 10)
    }
}

fn referrers_index(manifests: Vec<serde_json::Value>) -> OciManifestDocument {
    let body = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": INDEX_MEDIA_TYPE,
        "manifests": serde_json::Value::Array(manifests),
    }))
    .expect("referrers index always serializes");
    OciManifestDocument {
        media_type: INDEX_MEDIA_TYPE.to_string(),
        digest: content_digest(&body),
        body: Bytes::from(body),
    }
}

fn referrer_count(body: &[u8]) -> usize {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("manifests")?.as_array().map(Vec::len))
        .unwrap_or(0)
}

fn append_referrer_descriptors(
    body: &[u8],
    manifests: &mut Vec<serde_json::Value>,
    seen: &mut HashSet<String>,
) {
    let Some(items) = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("manifests")?.as_array().cloned())
    else {
        return;
    };
    for item in items {
        let Some(digest) = item.get("digest").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if seen.insert(digest.to_string()) {
            manifests.push(item);
        }
    }
}

fn join_v2_path(upstream: &Url, path: &str) -> String {
    let mut root = upstream.as_str().trim_end_matches('/').to_string();
    if let Some(stripped) = root.strip_suffix("/v2") {
        root = stripped.trim_end_matches('/').to_string();
    }
    format!("{root}/v2/{path}")
}

fn is_docker_hub(upstream: &Url) -> bool {
    matches!(
        upstream.host_str(),
        Some("registry-1.docker.io" | "docker.io" | "index.docker.io")
    )
}

fn upstream_image_name(upstream: &Url, name: &str, ecosystem: PackageEcosystem) -> String {
    if ecosystem == PackageEcosystem::Oci && is_docker_hub(upstream) && !name.contains('/') {
        format!("library/{name}")
    } else {
        name.to_string()
    }
}

fn content_type_of(response: &HttpResponse, fallback: &str) -> String {
    response
        .header("content-type")
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn bearer_challenge(response: &HttpResponse) -> Option<&str> {
    response.headers.iter().find_map(|(name, value)| {
        (name.eq_ignore_ascii_case("www-authenticate") && value.trim().starts_with("Bearer"))
            .then_some(value.as_str())
    })
}

struct BearerChallenge {
    realm: String,
    service: String,
    scope: Option<String>,
}

fn parse_bearer_challenge(header: &str) -> Result<BearerChallenge, PackagingError> {
    let rest = header
        .trim()
        .strip_prefix("Bearer")
        .ok_or_else(|| {
            PackagingError::InvalidUpstream(
                "WWW-Authenticate is not a Bearer challenge".to_string(),
            )
        })?
        .trim();
    let mut realm = None;
    let mut service = None;
    let mut scope = None;
    for part in rest.split(',') {
        let part = part.trim();
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "realm" => realm = Some(value.to_string()),
            "service" => service = Some(value.to_string()),
            "scope" => scope = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(BearerChallenge {
        realm: realm.ok_or_else(|| {
            PackagingError::InvalidUpstream("Bearer challenge is missing realm".to_string())
        })?,
        service: service.ok_or_else(|| {
            PackagingError::InvalidUpstream("Bearer challenge is missing service".to_string())
        })?,
        scope,
    })
}

fn token_request_url(
    realm: &str,
    service: &str,
    scope: Option<&str>,
) -> Result<String, PackagingError> {
    let mut url =
        Url::parse(realm).map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("service", service);
        if let Some(scope) = scope {
            pairs.append_pair("scope", scope);
        }
    }
    Ok(url.to_string())
}

fn parse_registry_token(body: &[u8]) -> Result<String, PackagingError> {
    let parsed: RegistryTokenResponse = serde_json::from_slice(body)
        .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;
    parsed.token.or(parsed.access_token).ok_or_else(|| {
        PackagingError::InvalidUpstream("token response is missing token".to_string())
    })
}

fn registry_status_error(url: &str, status: u16) -> PackagingError {
    if status == 404 {
        PackagingError::FileNotFound(url.to_string())
    } else {
        PackagingError::Upstream(HttpClientError::Status {
            status,
            url: url.to_string(),
        })
    }
}

fn blob_coordinate(
    ecosystem: PackageEcosystem,
    digest: &str,
) -> Result<PackageCoordinate, PackagingError> {
    let name = PackageName::parse(BLOB_PACKAGE)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    let version = PackageVersion::parse(digest)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    Ok(PackageCoordinate::new(ecosystem, name, version))
}

fn parse_artifact_id(value: &str) -> Result<ArtifactId, PackagingError> {
    Uuid::parse_str(value)
        .map(ArtifactId::from)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))
}

fn encode_entry<T: Serialize>(entry: &T) -> Bytes {
    Bytes::from(serde_json::to_vec(entry).expect("index entry always serializes"))
}

fn enqueue_manifest_digests(body: &[u8], pending: &mut Vec<String>) {
    let Ok(loose) = serde_json::from_slice::<LooseManifest>(body) else {
        return;
    };
    if let Some(config) = loose.config {
        pending.push(config.digest);
    }
    for layer in loose.layers.unwrap_or_default() {
        pending.push(layer.digest);
    }
    for nested in loose.manifests.unwrap_or_default() {
        pending.push(nested.digest);
    }
    for blob in loose.blobs.unwrap_or_default() {
        pending.push(blob.digest);
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accessory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifact_type: Option<String>,
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
    blobs: Option<Vec<LooseDescriptor>>,
}

#[derive(Debug, Deserialize)]
struct LooseDescriptor {
    digest: String,
}

#[derive(Debug, Deserialize)]
struct RegistryTokenResponse {
    token: Option<String>,
    access_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::{PackageName, PackageVersion};
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::admission_store::AdmissionStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::admission::AdmissionService;
    use crate::list_repository_artifacts::ListRepositoryArtifactsUseCase;
    use crate::test_support::{
        InMemoryAdmissionStore, InMemoryArtifactStore, InMemoryHttpClient,
        InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryStorage,
    };
    use ferrobox_domain::admission::AdmissionPolicy;

    fn strategy() -> OciPackagingStrategy {
        strategy_with_http(Arc::new(InMemoryHttpClient::default()))
    }

    fn strategy_with_http(http: Arc<InMemoryHttpClient>) -> OciPackagingStrategy {
        OciPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            http,
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

    fn oci_mirror(name: &str, upstream: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse(upstream).unwrap(),
            },
            PackageEcosystem::Oci,
        )
        .unwrap()
    }

    fn helm_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Helm,
        )
        .unwrap()
    }

    fn helm_strategy() -> OciPackagingStrategy {
        OciPackagingStrategy::for_ecosystem(
            PackageEcosystem::Helm,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryHttpClient::default()),
        )
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
        let again = strategy.put_blob(&repository, &digest, body).await.unwrap();
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
        let manifest = manifest_for(&config_digest, &layer_digest, config.len(), layer.len());
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

    fn signature_manifest(subject: &str, payload: &str, payload_size: usize) -> Bytes {
        Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
                "artifactType": crate::packaging::cosign::COSIGN_SIMPLE_MEDIA_TYPE,
                "config": {
                    "mediaType": "application/vnd.oci.empty.v1+json",
                    "digest": payload,
                    "size": payload_size
                },
                "layers": [{
                    "mediaType": crate::packaging::cosign::COSIGN_SIMPLE_MEDIA_TYPE,
                    "digest": payload,
                    "size": payload_size
                }],
                "subject": {
                    "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
                    "digest": subject,
                    "size": 100
                }
            }))
            .unwrap(),
        )
    }

    fn signed_cosign(subject: &str) -> (String, Bytes, Bytes, String) {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;
        use p256::ecdsa::signature::Signer;
        use p256::ecdsa::{Signature, SigningKey};
        use p256::pkcs8::EncodePublicKey;

        let signing = SigningKey::random(&mut rand::rngs::OsRng);
        let pem = signing
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .unwrap();
        let payload = Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "critical": {
                    "identity": { "docker-reference": "demo" },
                    "image": { "docker-manifest-digest": subject },
                    "type": "cosign container image signature"
                }
            }))
            .unwrap(),
        );
        let payload_digest = content_digest(&payload);
        let signature: Signature = signing.sign(&payload);
        let encoded = STANDARD.encode(signature.to_der());
        let manifest = Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
                "artifactType": crate::packaging::cosign::COSIGN_SIMPLE_MEDIA_TYPE,
                "config": {
                    "mediaType": "application/vnd.oci.empty.v1+json",
                    "digest": payload_digest,
                    "size": payload.len()
                },
                "layers": [{
                    "mediaType": crate::packaging::cosign::COSIGN_SIMPLE_MEDIA_TYPE,
                    "digest": payload_digest,
                    "size": payload.len(),
                    "annotations": {
                        crate::packaging::cosign::COSIGN_SIGNATURE_ANNOTATION: encoded
                    }
                }],
                "subject": {
                    "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
                    "digest": subject,
                    "size": 100
                }
            }))
            .unwrap(),
        );
        (payload_digest, payload, manifest, pem)
    }

    #[tokio::test]
    async fn put_cosign_signature_lists_referrers_and_keeps_sig_tag() {
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
        let manifest = manifest_for(&config_digest, &layer_digest, config.len(), layer.len());
        let image_digest = strategy
            .put_manifest(
                &repository,
                "demo",
                "latest",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest,
            )
            .await
            .unwrap();

        let (payload_digest, payload) =
            blob(br#"{"critical":{"identity":{"docker-reference":"demo"}}}"#);
        strategy
            .put_blob(&repository, &payload_digest, payload.clone())
            .await
            .unwrap();
        let sig_tag = format!("{}.sig", image_digest.replace(':', "-"));
        let signature = signature_manifest(&image_digest, &payload_digest, payload.len());
        strategy
            .put_manifest(
                &repository,
                "demo",
                &sig_tag,
                DEFAULT_MANIFEST_MEDIA_TYPE,
                signature,
            )
            .await
            .unwrap();

        let referrers = strategy
            .list_referrers(&repository, "demo", &image_digest, None)
            .await
            .unwrap();
        assert_eq!(referrers.media_type, INDEX_MEDIA_TYPE);
        let parsed: serde_json::Value = serde_json::from_slice(&referrers.body).unwrap();
        let manifests = parsed["manifests"].as_array().unwrap();
        assert_eq!(manifests.len(), 1);
        assert_eq!(
            manifests[0]["artifactType"].as_str(),
            Some(crate::packaging::cosign::COSIGN_SIMPLE_MEDIA_TYPE)
        );

        let filtered = strategy
            .list_referrers(
                &repository,
                "demo",
                &image_digest,
                Some(crate::packaging::cosign::COSIGN_SIMPLE_MEDIA_TYPE),
            )
            .await
            .unwrap();
        let filtered: serde_json::Value = serde_json::from_slice(&filtered.body).unwrap();
        assert_eq!(filtered["manifests"].as_array().unwrap().len(), 1);

        let tags = strategy.list_tags(&repository, "demo").await.unwrap();
        assert!(tags.contains(&"latest".to_string()));
        assert!(tags.contains(&sig_tag));

        let hits = strategy.search(&repository, "demo", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].max_version, "latest");
    }

    #[tokio::test]
    async fn admission_denies_unsigned_pull_and_allows_signed() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let admission_store = Arc::new(InMemoryAdmissionStore::default());
        let repository = oci_forge("oci-local");
        repositories.save(&repository).await.unwrap();
        admission_store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
                "",
            )
            .await
            .unwrap();
        let admission = AdmissionService::new(
            admission_store,
            repositories.clone(),
            ListRepositoryArtifactsUseCase::new(
                repositories.clone(),
                artifact_store.clone(),
                index.clone(),
            ),
        );
        let strategy = OciPackagingStrategy::new(
            artifact_store,
            index,
            storage,
            repositories,
            Arc::new(InMemoryHttpClient::default()),
        )
        .with_admission(admission);

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
        let image_digest = strategy
            .put_manifest(
                &repository,
                "demo",
                "latest",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest_for(&config_digest, &layer_digest, config.len(), layer.len()),
            )
            .await
            .unwrap();
        let error = strategy
            .get_manifest(&repository, "demo", "latest")
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));

        let (payload_digest, payload) = blob(br#"{"critical":{}}"#);
        strategy
            .put_blob(&repository, &payload_digest, payload.clone())
            .await
            .unwrap();
        let sig_tag = format!("{}.sig", image_digest.replace(':', "-"));
        strategy
            .put_manifest(
                &repository,
                "demo",
                &sig_tag,
                DEFAULT_MANIFEST_MEDIA_TYPE,
                signature_manifest(&image_digest, &payload_digest, payload.len()),
            )
            .await
            .unwrap();
        strategy
            .get_manifest(&repository, "demo", "latest")
            .await
            .unwrap();
        strategy
            .get_manifest(&repository, "demo", &sig_tag)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn admission_denies_unsigned_mirror_pull() {
        let http = Arc::new(InMemoryHttpClient::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let admission_store = Arc::new(InMemoryAdmissionStore::default());
        let repository = oci_mirror("oci-proxy", "https://registry-1.docker.io");
        repositories.save(&repository).await.unwrap();
        admission_store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
                "",
            )
            .await
            .unwrap();
        let admission = AdmissionService::new(
            admission_store,
            repositories.clone(),
            ListRepositoryArtifactsUseCase::new(
                repositories.clone(),
                artifact_store.clone(),
                index.clone(),
            ),
        );
        let strategy =
            OciPackagingStrategy::new(artifact_store, index, storage, repositories, http.clone())
                .with_admission(admission);
        let manifest = Bytes::from_static(br#"{"schemaVersion":2,"layers":[]}"#);
        http.stub(
            "https://registry-1.docker.io/v2/library/alpine/manifests/latest",
            200,
            manifest,
        );
        let error = strategy
            .get_manifest(&repository, "alpine", "latest")
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
    }

    #[tokio::test]
    async fn admission_denies_unsigned_alloy_member_pull() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let admission_store = Arc::new(InMemoryAdmissionStore::default());
        let forge = oci_forge("oci-member");
        repositories.save(&forge).await.unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("oci-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge.id()],
            },
            PackageEcosystem::Oci,
        )
        .unwrap();
        repositories.save(&alloy).await.unwrap();
        admission_store
            .save(
                forge.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
                "",
            )
            .await
            .unwrap();
        let admission = AdmissionService::new(
            admission_store,
            repositories.clone(),
            ListRepositoryArtifactsUseCase::new(
                repositories.clone(),
                artifact_store.clone(),
                index.clone(),
            ),
        );
        let strategy = OciPackagingStrategy::new(
            artifact_store,
            index,
            storage,
            repositories,
            Arc::new(InMemoryHttpClient::default()),
        )
        .with_admission(admission);
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
        let error = strategy
            .get_manifest(&alloy, "demo", "latest")
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
    }

    #[tokio::test]
    async fn admission_denies_unverified_pull_and_allows_verified() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let admission_store = Arc::new(InMemoryAdmissionStore::default());
        let repository = oci_forge("oci-local");
        repositories.save(&repository).await.unwrap();
        let admission = AdmissionService::new(
            admission_store.clone(),
            repositories.clone(),
            ListRepositoryArtifactsUseCase::new(
                repositories.clone(),
                artifact_store.clone(),
                index.clone(),
            ),
        );
        let strategy = OciPackagingStrategy::new(
            artifact_store,
            index,
            storage,
            repositories,
            Arc::new(InMemoryHttpClient::default()),
        )
        .with_admission(admission);

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
        let image_digest = strategy
            .put_manifest(
                &repository,
                "demo",
                "latest",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest_for(&config_digest, &layer_digest, config.len(), layer.len()),
            )
            .await
            .unwrap();

        let (payload_digest, payload, signature, pem) = signed_cosign(&image_digest);
        admission_store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_verified", "deny").unwrap(),
                &pem,
            )
            .await
            .unwrap();

        let (detect_digest, detect_payload) = blob(br#"{"critical":{}}"#);
        strategy
            .put_blob(&repository, &detect_digest, detect_payload.clone())
            .await
            .unwrap();
        let sig_tag = format!("{}.sig", image_digest.replace(':', "-"));
        strategy
            .put_manifest(
                &repository,
                "demo",
                &sig_tag,
                DEFAULT_MANIFEST_MEDIA_TYPE,
                signature_manifest(&image_digest, &detect_digest, detect_payload.len()),
            )
            .await
            .unwrap();
        let error = strategy
            .get_manifest(&repository, "demo", "latest")
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));

        strategy
            .put_blob(&repository, &payload_digest, payload.clone())
            .await
            .unwrap();
        strategy
            .put_manifest(
                &repository,
                "demo",
                &sig_tag,
                DEFAULT_MANIFEST_MEDIA_TYPE,
                signature,
            )
            .await
            .unwrap();
        strategy
            .get_manifest(&repository, "demo", "latest")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn promote_copies_manifest_and_blobs() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = OciPackagingStrategy::new(
            artifact_store,
            index,
            storage,
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let source = oci_forge("oci-dev");
        let target = oci_forge("oci-prod");
        let (config_digest, config) = blob(b"{\"architecture\":\"amd64\"}");
        let (layer_digest, layer) = blob(b"layer");
        strategy
            .put_blob(&source, &config_digest, config.clone())
            .await
            .unwrap();
        strategy
            .put_blob(&source, &layer_digest, layer.clone())
            .await
            .unwrap();
        let manifest = manifest_for(&config_digest, &layer_digest, config.len(), layer.len());
        strategy
            .put_manifest(
                &source,
                "demo",
                "latest",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest.clone(),
            )
            .await
            .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("latest").unwrap(),
        );

        let outcome = strategy
            .promote_version(&source, &target, &coordinate, false)
            .await
            .unwrap();
        assert!(outcome.artifacts_copied >= 3);
        let copied = strategy
            .get_manifest(&target, "demo", "latest")
            .await
            .unwrap();
        assert_eq!(copied.body, manifest);
        assert_eq!(
            strategy
                .get_blob(&target, "demo", &layer_digest)
                .await
                .unwrap(),
            layer
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
            Arc::new(InMemoryHttpClient::default()),
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
            Arc::new(InMemoryHttpClient::default()),
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
        let layer_body = strategy.download_file(&alloy, &layer_digest).await.unwrap();
        assert_eq!(layer_body.as_ref(), b"lyr");
    }

    #[test]
    fn join_v2_path_strips_a_trailing_v2_prefix() {
        let upstream = Url::parse("https://registry-1.docker.io/v2/").unwrap();
        assert_eq!(
            join_v2_path(&upstream, "library/alpine/manifests/latest"),
            "https://registry-1.docker.io/v2/library/alpine/manifests/latest"
        );
    }

    #[test]
    fn docker_hub_official_images_use_the_library_prefix() {
        let hub = Url::parse("https://registry-1.docker.io").unwrap();
        assert_eq!(
            upstream_image_name(&hub, "alpine", PackageEcosystem::Oci),
            "library/alpine"
        );
        assert_eq!(
            upstream_image_name(&hub, "bitnami/nginx", PackageEcosystem::Oci),
            "bitnami/nginx"
        );
        let ghcr = Url::parse("https://ghcr.io").unwrap();
        assert_eq!(
            upstream_image_name(&ghcr, "alpine", PackageEcosystem::Oci),
            "alpine"
        );
        assert_eq!(
            upstream_image_name(&hub, "wordpress", PackageEcosystem::Helm),
            "wordpress"
        );
    }

    #[test]
    fn parse_bearer_challenge_reads_docker_hub_header() {
        let parsed = parse_bearer_challenge(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/alpine:pull""#,
        )
        .unwrap();
        assert_eq!(parsed.realm, "https://auth.docker.io/token");
        assert_eq!(parsed.service, "registry.docker.io");
        assert_eq!(
            parsed.scope.as_deref(),
            Some("repository:library/alpine:pull")
        );
    }

    #[tokio::test]
    async fn mirror_caches_a_manifest_and_blob_from_upstream() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = strategy_with_http(http.clone());
        let repository = oci_mirror("oci-proxy", "https://registry-1.docker.io");
        let layer = Bytes::from_static(b"cached-layer");
        let layer_digest = content_digest(&layer);
        let manifest = Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
                "config": {
                    "mediaType": "application/vnd.oci.image.config.v1+json",
                    "digest": layer_digest,
                    "size": layer.len()
                },
                "layers": []
            }))
            .unwrap(),
        );
        let manifest_url = "https://registry-1.docker.io/v2/library/alpine/manifests/latest";
        let blob_url =
            format!("https://registry-1.docker.io/v2/library/alpine/blobs/{layer_digest}");
        http.stub(manifest_url, 200, manifest.clone());
        http.stub(&blob_url, 200, layer.clone());

        let pulled = strategy
            .get_manifest(&repository, "alpine", "latest")
            .await
            .unwrap();
        assert_eq!(pulled.body, manifest);
        assert_eq!(
            strategy.list_tags(&repository, "alpine").await.unwrap(),
            vec!["latest".to_string()]
        );

        http.stub(manifest_url, 500, Bytes::from_static(b"should-not-hit"));
        let cached = strategy
            .get_manifest(&repository, "alpine", "latest")
            .await
            .unwrap();
        assert_eq!(cached.body, manifest);

        let blob = strategy
            .get_blob(&repository, "alpine", &layer_digest)
            .await
            .unwrap();
        assert_eq!(blob, layer);
        http.stub(&blob_url, 500, Bytes::from_static(b"should-not-hit"));
        let blob_again = strategy
            .get_blob(&repository, "alpine", &layer_digest)
            .await
            .unwrap();
        assert_eq!(blob_again, layer);
    }

    #[tokio::test]
    async fn mirror_follows_a_docker_hub_bearer_challenge() {
        let http = Arc::new(InMemoryHttpClient::default());
        let strategy = strategy_with_http(http.clone());
        let repository = oci_mirror("oci-proxy", "https://registry-1.docker.io");
        let manifest = Bytes::from_static(br#"{"schemaVersion":2,"layers":[]}"#);
        let manifest_url = "https://registry-1.docker.io/v2/library/alpine/manifests/latest";
        let challenge = r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/alpine:pull""#;
        let token_url = token_request_url(
            "https://auth.docker.io/token",
            "registry.docker.io",
            Some("repository:library/alpine:pull"),
        )
        .unwrap();

        let mut unauthorized = HttpResponse::new(401, Bytes::from_static(b"unauthorized"));
        unauthorized
            .headers
            .push(("www-authenticate".to_string(), challenge.to_string()));
        let mut ok = HttpResponse::new(200, manifest.clone());
        ok.headers.push((
            "content-type".to_string(),
            DEFAULT_MANIFEST_MEDIA_TYPE.to_string(),
        ));
        http.stub_sequence(manifest_url, vec![unauthorized, ok]);
        http.stub(
            &token_url,
            200,
            Bytes::from_static(br#"{"token":"test-token"}"#),
        );

        let pulled = strategy
            .get_manifest(&repository, "alpine", "latest")
            .await
            .unwrap();
        assert_eq!(pulled.body, manifest);
        assert_eq!(pulled.media_type, DEFAULT_MANIFEST_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_mirror() {
        let error = strategy()
            .put_blob(
                &oci_mirror("oci-proxy", "https://registry-1.docker.io"),
                &content_digest(b"nope"),
                Bytes::from_static(b"nope"),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::ReadOnlyRepository));
    }

    #[tokio::test]
    async fn indexes_a_nested_image_name() {
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
                "bitnami/nginx",
                "latest",
                DEFAULT_MANIFEST_MEDIA_TYPE,
                manifest_for(&config_digest, &layer_digest, config.len(), layer.len()),
            )
            .await
            .unwrap();
        assert_eq!(
            strategy
                .list_tags(&repository, "bitnami/nginx")
                .await
                .unwrap(),
            vec!["latest".to_string()]
        );
    }

    #[tokio::test]
    async fn helm_forge_indexes_charts_under_the_helm_ecosystem() {
        let helm = helm_strategy();
        let repository = helm_forge("charts-local");
        let (config_digest, config) = blob(b"helm-config");
        let (layer_digest, layer) = blob(b"chart-tgz");
        helm.put_blob(&repository, &config_digest, config.clone())
            .await
            .unwrap();
        helm.put_blob(&repository, &layer_digest, layer.clone())
            .await
            .unwrap();
        helm.put_manifest(
            &repository,
            "demo",
            "0.1.0",
            "application/vnd.cncf.helm.chart.manifest.v1+json",
            manifest_for(&config_digest, &layer_digest, config.len(), layer.len()),
        )
        .await
        .unwrap();
        let pulled = helm
            .get_manifest(&repository, "demo", "0.1.0")
            .await
            .unwrap();
        assert!(!pulled.body.is_empty());
        let mismatch = strategy()
            .get_manifest(&repository, "demo", "0.1.0")
            .await
            .unwrap_err();
        assert!(matches!(mismatch, PackagingError::EcosystemMismatch { .. }));
    }
}
