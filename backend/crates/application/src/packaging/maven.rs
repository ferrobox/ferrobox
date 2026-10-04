//! Packaging strategy for Maven: classic HTTP layout
//! (`/{group}/{artifact}/{version}/{file}`) that `mvn deploy`,
//! `mvn dependency:get`, and Gradle need against a
//! `FerroBox` repository.
//!
//! A single URL serves *releases* and *SNAPSHOT*. The version is classified
//! by the `-SNAPSHOT` suffix or by unique timestamps
//! (`yyyyMMdd.HHmmss-N`). *Release* files and timestamped
//! files are immutable; the floating `-SNAPSHOT` name can be
//! overwritten.
//!
//! Covers **Forge** (PUT/GET, `maven-metadata.xml`, checksums, yank),
//! **Mirror** (*pull-through* cache of a Maven repository), and reads
//! on **Alloy**.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::ArtifactId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::ArtifactStore;
use ferrobox_ports::http_client::HttpClient;
use ferrobox_ports::mirror_credential_store::MirrorCredentialStore;
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::repository_store::RepositoryStore;
use ferrobox_ports::storage::StoragePort;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use url::Url;
use uuid::Uuid;

use super::{
    PackageSearchHit, PackagingError, PackagingStrategy, PromoteOutcome, PublishOutcome,
    copy_stored_artifact, ensure_quota, notify_assay,
};
use crate::assay::AssayService;
use crate::content_hash::sha256_checksum;
use crate::quota::QuotaService;
use crate::storage_key::storage_key_for;

/// Packaging strategy for the Maven ecosystem.
pub struct MavenPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    http_client: Arc<dyn HttpClient>,
    repository_store: Arc<dyn RepositoryStore>,
    assays: Option<AssayService>,
    quota: Option<QuotaService>,
    upstream_credentials: Option<Arc<dyn MirrorCredentialStore>>,
}

impl MavenPackagingStrategy {
    /// Builds the strategy from its ports.
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
            upstream_credentials: None,
        }
    }

    /// Connects automatic assay when publishing or caching a file.
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

    /// Sends this store's secret when the mirror calls its upstream.
    #[must_use]
    pub fn with_upstream_credentials(mut self, store: Arc<dyn MirrorCredentialStore>) -> Self {
        self.upstream_credentials = Some(store);
        self
    }

    fn ensure_maven_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Maven {
            return Err(PackagingError::EcosystemMismatch {
                expected: "maven",
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
            .entries_for_package(repository.id(), PackageEcosystem::Maven, coordinate.name())
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

    async fn load_package_entries(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Vec<VersionEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Maven, name)
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

    async fn download_stored(&self, artifact_id: ArtifactId) -> Result<Bytes, PackagingError> {
        Ok(self.storage.get(&storage_key_for(artifact_id)).await?)
    }

    async fn find_file_in(
        &self,
        repository: &Repository,
        filename: &str,
    ) -> Result<Option<(VersionEntry, usize)>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_repository(repository.id(), PackageEcosystem::Maven)
            .await?;
        for entry_bytes in entries {
            let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if let Some(index) = entry
                .files
                .iter()
                .position(|file| file.filename == filename)
            {
                return Ok(Some((entry, index)));
            }
        }
        Ok(None)
    }

    async fn serve_or_cache_file(
        &self,
        repository: &Repository,
        filename: &str,
        upstream_url: Option<String>,
        group_id: &str,
        artifact_id: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        if let Some((entry, index)) = self.find_file_in(repository, filename).await? {
            let file = &entry.files[index];
            if !file.artifact_id.is_empty()
                && let Ok(stored) = parse_artifact_id(&file.artifact_id)
            {
                return self.download_stored(stored).await;
            }
            if let Some(url) = file.url.clone() {
                return self
                    .cache_file_from_upstream(repository, &entry, index, &url)
                    .await;
            }
        }

        let Some(url) = upstream_url else {
            return Err(PackagingError::FileNotFound(filename.to_string()));
        };
        self.cache_new_upstream_file(repository, group_id, artifact_id, version, filename, &url)
            .await
    }

    async fn cache_file_from_upstream(
        &self,
        repository: &Repository,
        entry: &VersionEntry,
        file_index: usize,
        url: &str,
    ) -> Result<Bytes, PackagingError> {
        let response = super::upstream::upstream_get(
            self.http_client.as_ref(),
            self.upstream_credentials.as_deref(),
            repository.id(),
            url,
        )
        .await?;
        if !response.is_success() {
            return Err(PackagingError::FileNotFound(
                entry.files[file_index].filename.clone(),
            ));
        }
        self.store_cached_file(repository, entry, file_index, response.body)
            .await
    }

    async fn cache_new_upstream_file(
        &self,
        repository: &Repository,
        group_id: &str,
        artifact_id: &str,
        version: &str,
        filename: &str,
        url: &str,
    ) -> Result<Bytes, PackagingError> {
        let response = super::upstream::upstream_get(
            self.http_client.as_ref(),
            self.upstream_credentials.as_deref(),
            repository.id(),
            url,
        )
        .await?;
        if !response.is_success() {
            return Err(PackagingError::FileNotFound(filename.to_string()));
        }
        let coordinate = maven_coordinate(group_id, artifact_id, version)?;
        let mut entry = self
            .load_version_entry(repository, &coordinate)
            .await?
            .unwrap_or_else(|| VersionEntry::new(group_id, artifact_id, version));
        if entry.files.iter().any(|file| file.filename == filename) {
            let index = entry
                .files
                .iter()
                .position(|file| file.filename == filename)
                .expect("filename was just found");
            return self
                .store_cached_file(repository, &entry, index, response.body)
                .await;
        }
        let parsed = parse_maven_filename(artifact_id, version, filename);
        entry.files.push(FileEntry {
            filename: filename.to_string(),
            sha256: String::new(),
            artifact_id: String::new(),
            yanked: false,
            classifier: parsed.classifier,
            extension: parsed.extension,
            url: Some(url.to_string()),
            unique_version: parsed.unique_version,
        });
        let index = entry.files.len() - 1;
        self.store_cached_file(repository, &entry, index, response.body)
            .await
    }

    async fn store_cached_file(
        &self,
        repository: &Repository,
        entry: &VersionEntry,
        file_index: usize,
        body: Bytes,
    ) -> Result<Bytes, PackagingError> {
        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(repository.id(), checksum.clone(), body.len() as u64);
        ensure_quota(self.quota.as_ref(), repository.id(), body.len() as u64).await?;
        self.storage
            .put(&storage_key_for(artifact.id()), body.clone())
            .await?;
        self.artifact_store.save(&artifact).await?;

        let mut entry = entry.clone();
        entry.files[file_index].sha256 = checksum.to_string();
        entry.files[file_index].artifact_id = artifact.id().to_string();
        entry.updated = maven_last_updated();
        let coordinate = maven_coordinate(&entry.group_id, &entry.artifact_id, &entry.version)?;
        self.save_version_entry(repository, &coordinate, &entry, Some(artifact.id()))
            .await?;
        notify_assay(self.assays.as_ref(), repository.id(), &coordinate);
        Ok(body)
    }

    async fn metadata_for_package(
        &self,
        repository: &Repository,
        group_id: &str,
        artifact_id: &str,
    ) -> Result<Bytes, PackagingError> {
        let name = package_name(group_id, artifact_id)?;
        let mut versions = Vec::new();
        for target in self.resolve_read_targets(repository).await? {
            versions.extend(self.load_package_entries(&target, &name).await?);
            if let Some(upstream) = Self::mirror_upstream(&target)
                && let Ok(remote) = self
                    .fetch_upstream_metadata(&target, upstream, group_id, artifact_id, None)
                    .await
            {
                merge_remote_versions(&mut versions, &remote);
            }
        }
        if versions.is_empty() {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        }
        Ok(Bytes::from(render_artifact_metadata(
            group_id,
            artifact_id,
            &versions,
        )))
    }

    async fn metadata_for_version(
        &self,
        repository: &Repository,
        group_id: &str,
        artifact_id: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        let coordinate = maven_coordinate(group_id, artifact_id, version)?;
        for target in self.resolve_read_targets(repository).await? {
            if let Some(entry) = self.load_version_entry(&target, &coordinate).await? {
                return Ok(Bytes::from(render_version_metadata(&entry)));
            }
            if let Some(upstream) = Self::mirror_upstream(&target)
                && let Ok(body) = self
                    .fetch_upstream_metadata(
                        &target,
                        upstream,
                        group_id,
                        artifact_id,
                        Some(version),
                    )
                    .await
            {
                return Ok(body);
            }
        }
        Err(PackagingError::VersionNotFound(coordinate))
    }

    async fn fetch_upstream_metadata(
        &self,
        repository: &Repository,
        upstream: &Url,
        group_id: &str,
        artifact_id: &str,
        version: Option<&str>,
    ) -> Result<Bytes, PackagingError> {
        let path = match version {
            Some(version) => format!(
                "{}/{version}/maven-metadata.xml",
                maven_layout_prefix(group_id, artifact_id)
            ),
            None => format!(
                "{}/maven-metadata.xml",
                maven_layout_prefix(group_id, artifact_id)
            ),
        };
        let url = join_upstream(upstream, &path);
        let response = super::upstream::upstream_get(
            self.http_client.as_ref(),
            self.upstream_credentials.as_deref(),
            repository.id(),
            &url,
        )
        .await?;
        if !response.is_success() {
            return Err(PackagingError::FileNotFound(path));
        }
        Ok(response.body)
    }

    async fn get_resource(
        &self,
        repository: &Repository,
        resource: &MavenResource,
    ) -> Result<Bytes, PackagingError> {
        match resource {
            MavenResource::ArtifactMetadata {
                group_id,
                artifact_id,
                checksum,
            } => {
                let body = self
                    .metadata_for_package(repository, group_id, artifact_id)
                    .await?;
                Ok(maybe_checksum(body, *checksum))
            }
            MavenResource::VersionMetadata {
                group_id,
                artifact_id,
                version,
                checksum,
            } => {
                let body = self
                    .metadata_for_version(repository, group_id, artifact_id, version)
                    .await?;
                Ok(maybe_checksum(body, *checksum))
            }
            MavenResource::Artifact {
                group_id,
                artifact_id,
                version,
                filename,
                checksum,
            } => {
                let body = self
                    .get_artifact_bytes(repository, group_id, artifact_id, version, filename)
                    .await?;
                Ok(maybe_checksum(body, *checksum))
            }
        }
    }

    async fn get_artifact_bytes(
        &self,
        repository: &Repository,
        group_id: &str,
        artifact_id: &str,
        version: &str,
        filename: &str,
    ) -> Result<Bytes, PackagingError> {
        let mut last_error = PackagingError::FileNotFound(filename.to_string());
        for target in self.resolve_read_targets(repository).await? {
            let upstream = Self::mirror_upstream(&target).map(|url| {
                join_upstream(
                    url,
                    &format!(
                        "{}/{version}/{filename}",
                        maven_layout_prefix(group_id, artifact_id)
                    ),
                )
            });
            match self
                .serve_or_cache_file(&target, filename, upstream, group_id, artifact_id, version)
                .await
            {
                Ok(body) => return Ok(body),
                Err(PackagingError::FileNotFound(_) | PackagingError::PackageNotFound(_)) => {}
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    #[allow(clippy::too_many_lines)]
    async fn put_artifact(
        &self,
        repository: &Repository,
        group_id: &str,
        artifact_id: &str,
        version: &str,
        filename: &str,
        body: Bytes,
    ) -> Result<(), PackagingError> {
        let coordinate = maven_coordinate(group_id, artifact_id, version)?;
        let mut entry = self
            .load_version_entry(repository, &coordinate)
            .await?
            .unwrap_or_else(|| VersionEntry::new(group_id, artifact_id, version));

        let existing = entry
            .files
            .iter()
            .position(|file| file.filename == filename);
        if let Some(index) = existing {
            if !may_overwrite(version, filename) {
                return Err(PackagingError::AlreadyPublished(coordinate));
            }
            let checksum = sha256_checksum(&body);
            let artifact = Artifact::new(repository.id(), checksum.clone(), body.len() as u64);
            ensure_quota(self.quota.as_ref(), repository.id(), body.len() as u64).await?;
            self.storage
                .put(&storage_key_for(artifact.id()), body)
                .await?;
            self.artifact_store.save(&artifact).await?;
            let parsed = parse_maven_filename(artifact_id, version, filename);
            entry.files[index] = FileEntry {
                filename: filename.to_string(),
                sha256: checksum.to_string(),
                artifact_id: artifact.id().to_string(),
                yanked: entry.files[index].yanked,
                classifier: parsed.classifier,
                extension: parsed.extension,
                url: None,
                unique_version: parsed.unique_version,
            };
            entry.updated = maven_last_updated();
            self.save_version_entry(repository, &coordinate, &entry, Some(artifact.id()))
                .await?;
            notify_assay(self.assays.as_ref(), repository.id(), &coordinate);
            return Ok(());
        }

        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(repository.id(), checksum.clone(), body.len() as u64);
        ensure_quota(self.quota.as_ref(), repository.id(), body.len() as u64).await?;
        self.storage
            .put(&storage_key_for(artifact.id()), body)
            .await?;
        self.artifact_store.save(&artifact).await?;
        let parsed = parse_maven_filename(artifact_id, version, filename);
        entry.files.push(FileEntry {
            filename: filename.to_string(),
            sha256: checksum.to_string(),
            artifact_id: artifact.id().to_string(),
            yanked: false,
            classifier: parsed.classifier,
            extension: parsed.extension,
            url: None,
            unique_version: parsed.unique_version,
        });
        entry.updated = maven_last_updated();
        self.save_version_entry(repository, &coordinate, &entry, Some(artifact.id()))
            .await?;
        notify_assay(self.assays.as_ref(), repository.id(), &coordinate);
        Ok(())
    }
}

#[async_trait]
impl PackagingStrategy for MavenPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Maven
    }

    async fn publish(
        &self,
        repository: &Repository,
        _payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_maven_repository(repository)?;
        Err(PackagingError::InvalidPayload(
            "maven publishes via PUT to /maven/<repository>/{group}/{artifact}/{version}/{file}"
                .to_string(),
        ))
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_maven_repository(repository)?;
        let (group_id, artifact_id) = split_package_name(name.as_str())?;
        self.metadata_for_package(repository, &group_id, &artifact_id)
            .await
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_maven_repository(repository)?;
        let (group_id, artifact_id) = split_package_name(coordinate.name().as_str())?;
        let entry = self
            .load_version_entry(repository, coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
        let file = preferred_download(&entry)
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
        self.get_artifact_bytes(
            repository,
            &group_id,
            &artifact_id,
            coordinate.version().as_str(),
            &file.filename,
        )
        .await
    }

    async fn download_file(
        &self,
        repository: &Repository,
        filename: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_maven_repository(repository)?;
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let mut last_error = PackagingError::FileNotFound(filename.to_string());
            for target in self.resolve_read_targets(repository).await? {
                match self.download_file(&target, filename).await {
                    Ok(bytes) => return Ok(bytes),
                    Err(PackagingError::FileNotFound(_) | PackagingError::PackageNotFound(_)) => {}
                    Err(error) => last_error = error,
                }
            }
            return Err(last_error);
        }
        let Some((entry, index)) = self.find_file_in(repository, filename).await? else {
            return Err(PackagingError::FileNotFound(filename.to_string()));
        };
        let file = &entry.files[index];
        if !file.artifact_id.is_empty()
            && let Ok(artifact_id) = parse_artifact_id(&file.artifact_id)
        {
            return self.download_stored(artifact_id).await;
        }
        if let Some(url) = file.url.clone() {
            return self
                .cache_file_from_upstream(repository, &entry, index, &url)
                .await;
        }
        Err(PackagingError::FileNotFound(filename.to_string()))
    }

    async fn put_protocol_file(
        &self,
        repository: &Repository,
        path: &str,
        body: Bytes,
    ) -> Result<(), PackagingError> {
        Self::ensure_maven_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let resource = parse_maven_path(path)?;
        match resource {
            MavenResource::ArtifactMetadata { .. } | MavenResource::VersionMetadata { .. } => {
                Ok(())
            }
            MavenResource::Artifact {
                checksum: Some(_), ..
            } => Ok(()),
            MavenResource::Artifact {
                group_id,
                artifact_id,
                version,
                filename,
                checksum: None,
            } => {
                self.put_artifact(
                    repository,
                    &group_id,
                    &artifact_id,
                    &version,
                    &filename,
                    body,
                )
                .await
            }
        }
    }

    async fn get_protocol_file(
        &self,
        repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_maven_repository(repository)?;
        let resource = parse_maven_path(path)?;
        self.get_resource(repository, &resource).await
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
        Self::ensure_maven_repository(repository)?;
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
        Self::ensure_maven_repository(source)?;
        Self::ensure_maven_repository(target)?;
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
            .is_some_and(|existing| !existing.files.is_empty())
        {
            return Err(PackagingError::AlreadyPublished(coordinate.clone()));
        }
        if !preserve_yanked {
            entry.yanked = false;
        }

        let mut artifacts_copied = 0_u32;
        let mut bytes_copied = 0_u64;
        let mut last_artifact_id = None;
        for file in &mut entry.files {
            if file.artifact_id.is_empty() {
                return Err(PackagingError::FileNotFound(file.filename.clone()));
            }
            let source_id = parse_artifact_id(&file.artifact_id)?;
            let (new_id, size) = copy_stored_artifact(
                self.artifact_store.as_ref(),
                self.storage.as_ref(),
                self.quota.as_ref(),
                source_id,
                target.id(),
            )
            .await?;
            file.artifact_id = new_id.to_string();
            file.url = None;
            if !preserve_yanked {
                file.yanked = false;
            }
            artifacts_copied += 1;
            bytes_copied += size;
            last_artifact_id = Some(new_id);
        }
        entry.updated = maven_last_updated();
        self.save_version_entry(target, coordinate, &entry, last_artifact_id)
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
        Self::ensure_maven_repository(repository)?;
        let needle = query.trim().to_ascii_lowercase();
        let mut hits = BTreeMap::new();
        for target in self.resolve_read_targets(repository).await? {
            let entries = self
                .package_index_store
                .entries_for_repository(target.id(), PackageEcosystem::Maven)
                .await?;
            for entry_bytes in entries {
                let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
                if entry.yanked {
                    continue;
                }
                let name = format!("{}:{}", entry.group_id, entry.artifact_id);
                if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                hits.entry(name).or_insert(entry.version);
            }
        }
        Ok(hits
            .into_iter()
            .take(limit)
            .map(|(name, max_version)| PackageSearchHit { name, max_version })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MavenResource {
    Artifact {
        group_id: String,
        artifact_id: String,
        version: String,
        filename: String,
        checksum: Option<ChecksumKind>,
    },
    ArtifactMetadata {
        group_id: String,
        artifact_id: String,
        checksum: Option<ChecksumKind>,
    },
    VersionMetadata {
        group_id: String,
        artifact_id: String,
        version: String,
        checksum: Option<ChecksumKind>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChecksumKind {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VersionEntry {
    group_id: String,
    artifact_id: String,
    version: String,
    #[serde(default)]
    snapshot: bool,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    updated: String,
    #[serde(default)]
    files: Vec<FileEntry>,
}

impl VersionEntry {
    fn new(group_id: &str, artifact_id: &str, version: &str) -> Self {
        Self {
            group_id: group_id.to_string(),
            artifact_id: artifact_id.to_string(),
            version: version.to_string(),
            snapshot: is_snapshot_version(version),
            yanked: false,
            updated: maven_last_updated(),
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FileEntry {
    filename: String,
    sha256: String,
    #[serde(default)]
    artifact_id: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    classifier: Option<String>,
    #[serde(default)]
    extension: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    unique_version: Option<String>,
}

struct ParsedFilename {
    classifier: Option<String>,
    extension: String,
    unique_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UniqueSnapshot {
    timestamp: String,
    build_number: String,
    value: String,
}

/// `groupId:artifactId` and version of a jar/pom, not of metadata or checksums.
#[must_use]
pub fn admission_download_target(path: &str) -> Option<(String, String)> {
    match parse_maven_path(path.trim_matches('/')).ok()? {
        MavenResource::Artifact {
            group_id,
            artifact_id,
            version,
            checksum: None,
            ..
        } => Some((format!("{group_id}:{artifact_id}"), version)),
        _ => None,
    }
}

fn parse_maven_path(path: &str) -> Result<MavenResource, PackagingError> {
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 2 {
        return Err(PackagingError::InvalidPayload(
            "maven path must be {group}/{artifact}/…".to_string(),
        ));
    }
    let filename = *segments.last().expect("path has at least two segments");
    let (stem, checksum) = strip_checksum_suffix(filename);
    if is_metadata_filename(stem) {
        let rest = &segments[..segments.len() - 1];
        if rest.len() >= 3 && looks_like_version(rest[rest.len() - 1]) {
            let version = rest[rest.len() - 1].to_string();
            let artifact_id = rest[rest.len() - 2].to_string();
            let group_id = rest[..rest.len() - 2].join(".");
            return Ok(MavenResource::VersionMetadata {
                group_id,
                artifact_id,
                version,
                checksum,
            });
        }
        if rest.len() < 2 {
            return Err(PackagingError::InvalidPayload(
                "maven-metadata.xml needs groupId and artifactId".to_string(),
            ));
        }
        let artifact_id = rest[rest.len() - 1].to_string();
        let group_id = rest[..rest.len() - 1].join(".");
        return Ok(MavenResource::ArtifactMetadata {
            group_id,
            artifact_id,
            checksum,
        });
    }

    if segments.len() < 4 {
        return Err(PackagingError::InvalidPayload(
            "maven artifact path must be {group}/{artifact}/{version}/{file}".to_string(),
        ));
    }
    let version = segments[segments.len() - 2];
    if !looks_like_version(version) {
        return Err(PackagingError::InvalidPayload(format!(
            "invalid maven version segment '{version}'"
        )));
    }
    let artifact_id = segments[segments.len() - 3].to_string();
    let group_id = segments[..segments.len() - 3].join(".");
    if group_id.is_empty() {
        return Err(PackagingError::InvalidPayload(
            "maven groupId cannot be empty".to_string(),
        ));
    }
    Ok(MavenResource::Artifact {
        group_id,
        artifact_id,
        version: version.to_string(),
        filename: stem.to_string(),
        checksum,
    })
}

fn strip_checksum_suffix(filename: &str) -> (&str, Option<ChecksumKind>) {
    if let Some(stem) = filename.strip_suffix(".md5") {
        (stem, Some(ChecksumKind::Md5))
    } else if let Some(stem) = filename.strip_suffix(".sha1") {
        (stem, Some(ChecksumKind::Sha1))
    } else if let Some(stem) = filename.strip_suffix(".sha256") {
        (stem, Some(ChecksumKind::Sha256))
    } else if let Some(stem) = filename.strip_suffix(".sha512") {
        (stem, Some(ChecksumKind::Sha512))
    } else {
        (filename, None)
    }
}

fn is_metadata_filename(filename: &str) -> bool {
    filename == "maven-metadata.xml"
}

fn looks_like_version(segment: &str) -> bool {
    if segment.ends_with("-SNAPSHOT") {
        return true;
    }
    if parse_unique_snapshot(segment).is_some() {
        return true;
    }
    segment
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
}

/// `true` if the version is a floating SNAPSHOT (`1.0-SNAPSHOT`).
#[must_use]
pub fn is_snapshot_version(version: &str) -> bool {
    version.ends_with("-SNAPSHOT")
}

fn may_overwrite(version: &str, filename: &str) -> bool {
    is_snapshot_version(version) && is_floating_snapshot_filename(filename)
}

fn is_floating_snapshot_filename(filename: &str) -> bool {
    filename.contains("-SNAPSHOT.") || filename.contains("-SNAPSHOT-")
}

fn parse_unique_snapshot(value: &str) -> Option<UniqueSnapshot> {
    let build_dash = value.rfind('-')?;
    let build = value.get(build_dash + 1..)?;
    if build.is_empty() || !build.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let rest = value.get(..build_dash)?;
    let ts_dash = rest.rfind('-')?;
    let timestamp = rest.get(ts_dash + 1..)?;
    if timestamp.len() != 15 || timestamp.as_bytes().get(8) != Some(&b'.') {
        return None;
    }
    if !timestamp[..8].bytes().all(|byte| byte.is_ascii_digit())
        || !timestamp[9..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let base = rest.get(..ts_dash)?;
    if base.is_empty() {
        return None;
    }
    Some(UniqueSnapshot {
        timestamp: timestamp.to_string(),
        build_number: build.to_string(),
        value: value.to_string(),
    })
}

fn parse_maven_filename(artifact_id: &str, version: &str, filename: &str) -> ParsedFilename {
    let prefix = format!("{artifact_id}-");
    let Some(rest) = filename.strip_prefix(&prefix) else {
        return ParsedFilename {
            classifier: None,
            extension: extension_of(filename).to_string(),
            unique_version: None,
        };
    };

    let unique = rest
        .find('.')
        .and_then(|dot| parse_unique_snapshot(&rest[..dot]));
    if let Some(unique) = unique {
        let after = rest
            .strip_prefix(&unique.value)
            .unwrap_or(rest)
            .trim_start_matches('-');
        return ParsedFilename {
            classifier: classifier_from_remainder(after),
            extension: extension_of(after).to_string(),
            unique_version: Some(unique.value),
        };
    }

    if let Some(after) = rest.strip_prefix(version) {
        let after = after.trim_start_matches('-');
        return ParsedFilename {
            classifier: classifier_from_remainder(after),
            extension: extension_of(after).to_string(),
            unique_version: None,
        };
    }

    ParsedFilename {
        classifier: None,
        extension: extension_of(filename).to_string(),
        unique_version: None,
    }
}

fn classifier_from_remainder(remainder: &str) -> Option<String> {
    let stem = remainder
        .rsplit_once('.')
        .map_or(remainder, |(stem, _)| stem);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

fn extension_of(filename: &str) -> &str {
    filename.rsplit_once('.').map_or("", |(_, ext)| ext)
}

fn maven_coordinate(
    group_id: &str,
    artifact_id: &str,
    version: &str,
) -> Result<PackageCoordinate, PackagingError> {
    Ok(PackageCoordinate::new(
        PackageEcosystem::Maven,
        package_name(group_id, artifact_id)?,
        PackageVersion::parse(version)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
    ))
}

fn package_name(group_id: &str, artifact_id: &str) -> Result<PackageName, PackagingError> {
    PackageName::parse(format!("{group_id}:{artifact_id}"))
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))
}

fn split_package_name(name: &str) -> Result<(String, String), PackagingError> {
    name.split_once(':')
        .map(|(group, artifact)| (group.to_string(), artifact.to_string()))
        .ok_or_else(|| {
            PackagingError::InvalidPayload(
                "maven package name must be groupId:artifactId".to_string(),
            )
        })
}

fn parse_artifact_id(value: &str) -> Result<ArtifactId, PackagingError> {
    Uuid::parse_str(value)
        .map(ArtifactId::from)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))
}

fn maven_layout_prefix(group_id: &str, artifact_id: &str) -> String {
    format!("{}/{artifact_id}", group_id.replace('.', "/"))
}

fn join_upstream(upstream: &Url, path: &str) -> String {
    let base = upstream.as_str().trim_end_matches('/');
    format!("{base}/{path}")
}

fn maven_last_updated() -> String {
    Utc::now().format("%Y%m%d%H%M%S").to_string()
}

fn maybe_checksum(body: Bytes, checksum: Option<ChecksumKind>) -> Bytes {
    match checksum {
        Some(kind) => Bytes::from(format!("{}\n", digest(&body, kind))),
        None => body,
    }
}

fn digest(body: &[u8], kind: ChecksumKind) -> String {
    match kind {
        ChecksumKind::Md5 => format!("{:x}", Md5::digest(body)),
        ChecksumKind::Sha1 => format!("{:x}", Sha1::digest(body)),
        ChecksumKind::Sha256 => format!("{:x}", Sha256::digest(body)),
        ChecksumKind::Sha512 => format!("{:x}", Sha512::digest(body)),
    }
}

fn preferred_download(entry: &VersionEntry) -> Option<&FileEntry> {
    entry
        .files
        .iter()
        .find(|file| file.extension == "jar" && file.classifier.is_none())
        .or_else(|| {
            entry
                .files
                .iter()
                .find(|file| file.extension == "pom" && file.classifier.is_none())
        })
        .or_else(|| entry.files.first())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_artifact_metadata(
    group_id: &str,
    artifact_id: &str,
    versions: &[VersionEntry],
) -> String {
    let mut seen = Vec::new();
    for entry in versions {
        if entry.yanked {
            continue;
        }
        if !seen.iter().any(|version| version == &entry.version) {
            seen.push(entry.version.clone());
        }
    }
    let latest = versions
        .iter()
        .filter(|entry| !entry.yanked)
        .max_by(|left, right| left.updated.cmp(&right.updated))
        .map(|entry| entry.version.as_str())
        .or_else(|| seen.last().map(String::as_str))
        .unwrap_or("");
    let release = versions
        .iter()
        .filter(|entry| !entry.yanked && !entry.snapshot)
        .max_by(|left, right| left.updated.cmp(&right.updated))
        .map_or("", |entry| entry.version.as_str());
    let last_updated = versions
        .iter()
        .map(|entry| entry.updated.as_str())
        .max()
        .unwrap_or("");
    let version_xml = seen
        .iter()
        .map(|version| format!("      <version>{}</version>", xml_escape(version)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<metadata>\n\
  <groupId>{}</groupId>\n\
  <artifactId>{}</artifactId>\n\
  <versioning>\n\
    <latest>{}</latest>\n\
    <release>{}</release>\n\
    <versions>\n\
{version_xml}\n\
    </versions>\n\
    <lastUpdated>{}</lastUpdated>\n\
  </versioning>\n\
</metadata>\n",
        xml_escape(group_id),
        xml_escape(artifact_id),
        xml_escape(latest),
        xml_escape(release),
        xml_escape(last_updated)
    )
}

fn render_version_metadata(entry: &VersionEntry) -> String {
    let unique = entry.files.iter().find_map(|file| {
        file.unique_version
            .as_deref()
            .and_then(parse_unique_snapshot)
    });
    let snapshot_xml = unique.as_ref().map_or_else(
        || {
            format!(
                "    <snapshot>\n      <timestamp>{}</timestamp>\n      <buildNumber>1</buildNumber>\n    </snapshot>",
                entry.updated.get(..8).unwrap_or(&entry.updated)
            )
        },
        |snapshot| {
            format!(
                "    <snapshot>\n      <timestamp>{}</timestamp>\n      <buildNumber>{}</buildNumber>\n    </snapshot>",
                xml_escape(&snapshot.timestamp),
                xml_escape(&snapshot.build_number)
            )
        },
    );
    let mut snapshot_versions = String::new();
    for file in &entry.files {
        if file.yanked {
            continue;
        }
        let value = file
            .unique_version
            .clone()
            .unwrap_or_else(|| entry.version.clone());
        let classifier = file
            .classifier
            .as_ref()
            .map_or(String::new(), |classifier| {
                format!(
                    "        <classifier>{}</classifier>\n",
                    xml_escape(classifier)
                )
            });
        snapshot_versions.push_str("      <snapshotVersion>\n");
        snapshot_versions.push_str(&classifier);
        snapshot_versions.push_str("        <extension>");
        snapshot_versions.push_str(&xml_escape(&file.extension));
        snapshot_versions.push_str("</extension>\n        <value>");
        snapshot_versions.push_str(&xml_escape(&value));
        snapshot_versions.push_str("</value>\n        <updated>");
        snapshot_versions.push_str(&xml_escape(&entry.updated));
        snapshot_versions.push_str("</updated>\n      </snapshotVersion>\n");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<metadata>\n\
  <groupId>{}</groupId>\n\
  <artifactId>{}</artifactId>\n\
  <version>{}</version>\n\
  <versioning>\n\
{snapshot_xml}\n\
    <lastUpdated>{}</lastUpdated>\n\
    <snapshotVersions>\n\
{snapshot_versions}    </snapshotVersions>\n\
  </versioning>\n\
</metadata>\n",
        xml_escape(&entry.group_id),
        xml_escape(&entry.artifact_id),
        xml_escape(&entry.version),
        xml_escape(&entry.updated)
    )
}

fn merge_remote_versions(versions: &mut Vec<VersionEntry>, metadata: &Bytes) {
    let Ok(text) = std::str::from_utf8(metadata) else {
        return;
    };
    for version in extract_xml_versions(text) {
        if versions.iter().any(|entry| entry.version == version) {
            continue;
        }
        let snapshot = is_snapshot_version(&version);
        versions.push(VersionEntry {
            group_id: String::new(),
            artifact_id: String::new(),
            version,
            snapshot,
            yanked: false,
            updated: String::new(),
            files: Vec::new(),
        });
    }
}

fn extract_xml_versions(xml: &str) -> Vec<String> {
    let mut versions = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<version>") {
        let after = &rest[start + "<version>".len()..];
        if let Some(end) = after.find("</version>") {
            let value = after[..end].trim();
            if !value.is_empty() {
                versions.push(value.to_string());
            }
            rest = &after[end + "</version>".len()..];
        } else {
            break;
        }
    }
    versions
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;
    use url::Url;

    use super::*;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn strategy() -> MavenPackagingStrategy {
        MavenPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        )
    }

    fn maven_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Maven,
        )
        .unwrap()
    }

    fn artifact_path(file: &str) -> String {
        format!("org/example/hello/1.0.0/{file}")
    }

    #[test]
    fn admission_download_target_skips_metadata_and_checksums() {
        assert_eq!(
            admission_download_target("org/example/hello/1.0.0/hello-1.0.0.jar"),
            Some(("org.example:hello".into(), "1.0.0".into()))
        );
        assert_eq!(
            admission_download_target("org/example/hello/1.0.0/hello-1.0.0.jar.sha1"),
            None
        );
        assert_eq!(
            admission_download_target("org/example/hello/maven-metadata.xml"),
            None
        );
    }

    #[test]
    fn parses_release_artifact_and_metadata_paths() {
        let artifact = parse_maven_path("org/example/hello/1.0.0/hello-1.0.0.jar").unwrap();
        assert_eq!(
            artifact,
            MavenResource::Artifact {
                group_id: "org.example".to_string(),
                artifact_id: "hello".to_string(),
                version: "1.0.0".to_string(),
                filename: "hello-1.0.0.jar".to_string(),
                checksum: None,
            }
        );
        let checksum = parse_maven_path("org/example/hello/1.0.0/hello-1.0.0.jar.sha1").unwrap();
        assert!(matches!(
            checksum,
            MavenResource::Artifact {
                checksum: Some(ChecksumKind::Sha1),
                ..
            }
        ));
        let metadata = parse_maven_path("org/example/hello/maven-metadata.xml").unwrap();
        assert_eq!(
            metadata,
            MavenResource::ArtifactMetadata {
                group_id: "org.example".to_string(),
                artifact_id: "hello".to_string(),
                checksum: None,
            }
        );
        let snapshot_meta =
            parse_maven_path("org/example/hello/1.0-SNAPSHOT/maven-metadata.xml").unwrap();
        assert!(matches!(
            snapshot_meta,
            MavenResource::VersionMetadata {
                version,
                ..
            } if version == "1.0-SNAPSHOT"
        ));
    }

    #[test]
    fn detects_snapshot_and_unique_timestamps() {
        assert!(is_snapshot_version("1.0-SNAPSHOT"));
        assert!(!is_snapshot_version("1.0.0"));
        let unique = parse_unique_snapshot("1.0-20240101.120000-3").unwrap();
        assert_eq!(unique.timestamp, "20240101.120000");
        assert_eq!(unique.build_number, "3");
        assert!(may_overwrite("1.0-SNAPSHOT", "hello-1.0-SNAPSHOT.jar"));
        assert!(!may_overwrite(
            "1.0-SNAPSHOT",
            "hello-1.0-20240101.120000-1.jar"
        ));
        assert!(!may_overwrite("1.0.0", "hello-1.0.0.jar"));
    }

    #[tokio::test]
    async fn put_then_get_release_and_metadata() {
        let strategy = strategy();
        let repository = maven_forge("maven-local");
        strategy
            .put_protocol_file(
                &repository,
                &artifact_path("hello-1.0.0.pom"),
                Bytes::from_static(b"<project/>"),
            )
            .await
            .unwrap();
        strategy
            .put_protocol_file(
                &repository,
                &artifact_path("hello-1.0.0.jar"),
                Bytes::from_static(b"jar-bytes"),
            )
            .await
            .unwrap();

        let jar = strategy
            .get_protocol_file(&repository, &artifact_path("hello-1.0.0.jar"))
            .await
            .unwrap();
        assert_eq!(jar.as_ref(), b"jar-bytes");

        let sha1 = strategy
            .get_protocol_file(&repository, &artifact_path("hello-1.0.0.jar.sha1"))
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&sha1).unwrap().trim(),
            &digest(b"jar-bytes", ChecksumKind::Sha1)
        );

        let metadata = strategy
            .index(
                &repository,
                &PackageName::parse("org.example:hello").unwrap(),
            )
            .await
            .unwrap();
        let xml = std::str::from_utf8(&metadata).unwrap();
        assert!(xml.contains("<version>1.0.0</version>"));
        assert!(xml.contains("<release>1.0.0</release>"));
        assert!(!xml.contains("SNAPSHOT"));
    }

    #[tokio::test]
    async fn rejects_release_overwrite_and_allows_snapshot() {
        let strategy = strategy();
        let repository = maven_forge("maven-local");
        strategy
            .put_protocol_file(
                &repository,
                &artifact_path("hello-1.0.0.jar"),
                Bytes::from_static(b"one"),
            )
            .await
            .unwrap();
        let err = strategy
            .put_protocol_file(
                &repository,
                &artifact_path("hello-1.0.0.jar"),
                Bytes::from_static(b"two"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::AlreadyPublished(_)));

        strategy
            .put_protocol_file(
                &repository,
                "org/example/hello/1.0-SNAPSHOT/hello-1.0-SNAPSHOT.jar",
                Bytes::from_static(b"snap-1"),
            )
            .await
            .unwrap();
        strategy
            .put_protocol_file(
                &repository,
                "org/example/hello/1.0-SNAPSHOT/hello-1.0-SNAPSHOT.jar",
                Bytes::from_static(b"snap-2"),
            )
            .await
            .unwrap();
        let body = strategy
            .get_protocol_file(
                &repository,
                "org/example/hello/1.0-SNAPSHOT/hello-1.0-SNAPSHOT.jar",
            )
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"snap-2");
    }

    #[tokio::test]
    async fn unique_snapshot_files_are_immutable() {
        let strategy = strategy();
        let repository = maven_forge("maven-local");
        let path = "org/example/hello/1.0-SNAPSHOT/hello-1.0-20240101.120000-1.jar";
        strategy
            .put_protocol_file(&repository, path, Bytes::from_static(b"one"))
            .await
            .unwrap();
        let err = strategy
            .put_protocol_file(&repository, path, Bytes::from_static(b"two"))
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::AlreadyPublished(_)));
    }

    #[tokio::test]
    async fn promote_copies_every_file_of_the_version() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = MavenPackagingStrategy::new(
            artifact_store,
            index,
            storage,
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        );
        let source = maven_forge("maven-dev");
        let target = maven_forge("maven-prod");
        strategy
            .put_protocol_file(
                &source,
                &artifact_path("hello-1.0.0.pom"),
                Bytes::from_static(b"<project/>"),
            )
            .await
            .unwrap();
        strategy
            .put_protocol_file(
                &source,
                &artifact_path("hello-1.0.0.jar"),
                Bytes::from_static(b"jar"),
            )
            .await
            .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Maven,
            PackageName::parse("org.example:hello").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let outcome = strategy
            .promote_version(&source, &target, &coordinate, false)
            .await
            .unwrap();
        assert_eq!(outcome.artifacts_copied, 2);
        assert_eq!(
            strategy
                .download_file(&target, "hello-1.0.0.jar")
                .await
                .unwrap()
                .as_ref(),
            b"jar"
        );
    }

    #[tokio::test]
    async fn alloy_reads_first_member_files() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = MavenPackagingStrategy::new(
            artifact_store,
            index,
            storage,
            Arc::new(InMemoryHttpClient::default()),
            repositories.clone(),
        );
        let member = maven_forge("maven-member");
        repositories.save(&member).await.unwrap();
        strategy
            .put_protocol_file(
                &member,
                &artifact_path("hello-1.0.0.jar"),
                Bytes::from_static(b"from-member"),
            )
            .await
            .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("maven-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![member.id()],
            },
            PackageEcosystem::Maven,
        )
        .unwrap();
        let body = strategy
            .get_protocol_file(&alloy, &artifact_path("hello-1.0.0.jar"))
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"from-member");
    }

    #[tokio::test]
    async fn mirror_caches_upstream_artifact() {
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            "https://repo1.example/maven2/org/example/hello/1.0.0/hello-1.0.0.jar",
            200,
            Bytes::from_static(b"upstream-jar"),
        );
        let strategy = MavenPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http,
            Arc::new(InMemoryRepositoryStore::default()),
        );
        let repository = Repository::new(
            RepositoryName::parse("maven-central").unwrap(),
            RepositoryKind::Mirror {
                upstream: Url::parse("https://repo1.example/maven2/").unwrap(),
            },
            PackageEcosystem::Maven,
        )
        .unwrap();
        let body = strategy
            .get_protocol_file(&repository, &artifact_path("hello-1.0.0.jar"))
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"upstream-jar");
        let cached = strategy
            .download_file(&repository, "hello-1.0.0.jar")
            .await
            .unwrap();
        assert_eq!(cached.as_ref(), b"upstream-jar");
    }

    #[tokio::test]
    async fn yank_omits_version_from_metadata() {
        let strategy = strategy();
        let repository = maven_forge("maven-local");
        strategy
            .put_protocol_file(
                &repository,
                &artifact_path("hello-1.0.0.jar"),
                Bytes::from_static(b"jar"),
            )
            .await
            .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Maven,
            PackageName::parse("org.example:hello").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();
        let metadata = strategy
            .index(
                &repository,
                &PackageName::parse("org.example:hello").unwrap(),
            )
            .await
            .unwrap();
        let xml = std::str::from_utf8(&metadata).unwrap();
        assert!(!xml.contains("<version>1.0.0</version>"));
        assert!(xml.contains("<versions>"));
    }
}
