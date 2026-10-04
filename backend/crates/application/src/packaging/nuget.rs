//! Packaging strategy for NuGet: V3 API that `dotnet nuget push`,
//! `dotnet restore`, and `nuget.exe` need against a
//! `FerroBox` repository.
//!
//! Covers **Forge** (push, flat container, registration, search, unlist),
//! **Mirror** (*pull-through* cache of a V3 origin), and reads on
//! **Alloy**. Versions are immutable; *unlist* (yank) hides them
//! from search and marks them `listed: false` in registration.

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
use ferrobox_ports::http_client::HttpClient;
use ferrobox_ports::mirror_credential_store::MirrorCredentialStore;
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::repository_store::RepositoryStore;
use ferrobox_ports::storage::StoragePort;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

use super::{
    PackageSearchHit, PackagingError, PackagingStrategy, PromoteOutcome, PublishOutcome,
    copy_stored_artifact, ensure_quota, notify_assay,
};
use crate::assay::AssayService;
use crate::content_hash::sha256_checksum;
use crate::quota::QuotaService;
use crate::storage_key::storage_key_for;

/// Packaging strategy for the `NuGet` ecosystem.
pub struct NugetPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    http_client: Arc<dyn HttpClient>,
    repository_store: Arc<dyn RepositoryStore>,
    public_base_url: String,
    assays: Option<AssayService>,
    quota: Option<QuotaService>,
    upstream_credentials: Option<Arc<dyn MirrorCredentialStore>>,
}

impl NugetPackagingStrategy {
    /// Builds the strategy from its ports.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        http_client: Arc<dyn HttpClient>,
        repository_store: Arc<dyn RepositoryStore>,
        public_base_url: impl Into<String>,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
            http_client,
            repository_store,
            public_base_url: public_base_url.into(),
            assays: None,
            quota: None,
            upstream_credentials: None,
        }
    }

    /// Connects automatic assay when publishing or caching a package.
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

    fn ensure_nuget_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Nuget {
            return Err(PackagingError::EcosystemMismatch {
                expected: "nuget",
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

    fn resource_base(&self, repository: &Repository) -> String {
        format!(
            "{}/nuget/{}",
            self.public_base_url.trim_end_matches('/'),
            repository.id()
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

    async fn load_version_entry(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<VersionEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Nuget, coordinate.name())
            .await?;
        for entry_bytes in entries {
            let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if entry
                .version
                .eq_ignore_ascii_case(coordinate.version().as_str())
            {
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
            .entries_for_package(repository.id(), PackageEcosystem::Nuget, name)
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

    async fn collect_package_entries(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Vec<VersionEntry>, PackagingError> {
        let mut versions = Vec::new();
        for target in self.resolve_read_targets(repository).await? {
            versions.extend(self.load_package_entries(&target, name).await?);
        }
        Ok(versions)
    }

    async fn find_version(
        &self,
        repository: &Repository,
        name: &PackageName,
        version: &str,
    ) -> Result<Option<(Repository, VersionEntry)>, PackagingError> {
        let coordinate = nuget_coordinate(name.as_str(), version)?;
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

    async fn store_nupkg(
        &self,
        repository: &Repository,
        parsed: &ParsedNupkg,
        body: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        let coordinate = nuget_coordinate(&parsed.id, &parsed.version)?;
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
        let filename = nupkg_filename(&parsed.id, &parsed.version);
        let entry = VersionEntry {
            id: parsed.id.clone(),
            version: parsed.version.clone(),
            description: parsed.description.clone(),
            authors: parsed.authors.clone(),
            yanked: false,
            files: vec![FileEntry {
                filename,
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
        name: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        let Some(upstream) = Self::mirror_upstream(repository) else {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        };
        let flat = self.upstream_flat_base(repository, upstream).await?;
        let id_lower = name.to_ascii_lowercase();
        let ver_lower = version.to_ascii_lowercase();
        let url = format!("{flat}{id_lower}/{ver_lower}/{id_lower}.{ver_lower}.nupkg");
        let response = super::upstream::upstream_get(
            self.http_client.as_ref(),
            self.upstream_credentials.as_deref(),
            repository.id(),
            &url,
        )
        .await?;
        if !response.is_success() {
            return Err(PackagingError::FileNotFound(nupkg_filename(name, version)));
        }
        let parsed = parse_nupkg(&response.body)?;
        self.store_nupkg(repository, &parsed, response.body.clone())
            .await?;
        Ok(response.body)
    }

    async fn upstream_flat_base(
        &self,
        repository: &Repository,
        upstream: &Url,
    ) -> Result<String, PackagingError> {
        let index_url = nuget_service_index_url(upstream);
        let response = super::upstream::upstream_get(
            self.http_client.as_ref(),
            self.upstream_credentials.as_deref(),
            repository.id(),
            &index_url,
        )
        .await?;
        if !response.is_success() {
            return Err(PackagingError::InvalidUpstream(format!(
                "service index HTTP {}",
                response.status
            )));
        }
        let index: Value = serde_json::from_slice(&response.body)
            .map_err(|err| PackagingError::InvalidUpstream(err.to_string()))?;
        let resources = index
            .get("resources")
            .and_then(Value::as_array)
            .ok_or_else(|| PackagingError::InvalidUpstream("missing resources".to_string()))?;
        for resource in resources {
            let type_name = resource
                .get("@type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if type_name.starts_with("PackageBaseAddress")
                && let Some(id) = resource.get("@id").and_then(Value::as_str)
            {
                let mut base = id.to_string();
                if !base.ends_with('/') {
                    base.push('/');
                }
                return Ok(base);
            }
        }
        Err(PackagingError::InvalidUpstream(
            "upstream service index has no PackageBaseAddress".to_string(),
        ))
    }

    async fn nupkg_bytes(
        &self,
        repository: &Repository,
        name: &str,
        version: &str,
    ) -> Result<Bytes, PackagingError> {
        let package_name = PackageName::parse(name.to_ascii_lowercase())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        if let Some((_target, entry)) = self
            .find_version(repository, &package_name, version)
            .await?
            && let Some(file) = entry.files.first()
            && let Ok(artifact_id) = parse_artifact_id(&file.artifact_id)
        {
            return self.download_stored(artifact_id).await;
        }
        for target in self.resolve_read_targets(repository).await? {
            if Self::mirror_upstream(&target).is_some() {
                match self.cache_from_upstream(&target, name, version).await {
                    Ok(body) => return Ok(body),
                    Err(PackagingError::FileNotFound(_) | PackagingError::PackageNotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Err(PackagingError::VersionNotFound(nuget_coordinate(
            name, version,
        )?))
    }

    fn service_index(&self, repository: &Repository) -> Bytes {
        let base = self.resource_base(repository);
        Bytes::from(
            serde_json::to_vec(&json!({
                "version": "3.0.0",
                "resources": [
                    {
                        "@id": format!("{base}/v3/flat/"),
                        "@type": "PackageBaseAddress/3.0.0"
                    },
                    {
                        "@id": format!("{base}/v3/registration/"),
                        "@type": "RegistrationsBaseUrl/3.6.0"
                    },
                    {
                        "@id": format!("{base}/v3/registration/"),
                        "@type": "RegistrationsBaseUrl"
                    },
                    {
                        "@id": format!("{base}/v3/query"),
                        "@type": "SearchQueryService"
                    },
                    {
                        "@id": format!("{base}/v3/query"),
                        "@type": "SearchQueryService/3.0.0-rc"
                    },
                    {
                        "@id": format!("{base}/v3/autocomplete"),
                        "@type": "SearchAutocompleteService"
                    },
                    {
                        "@id": format!("{base}/v3/package"),
                        "@type": "PackagePublish/2.0.0"
                    }
                ]
            }))
            .expect("service index serializes"),
        )
    }

    fn flat_index(versions: &[VersionEntry]) -> Bytes {
        let mut seen = Vec::new();
        for entry in versions {
            if !seen
                .iter()
                .any(|version: &String| version.eq_ignore_ascii_case(&entry.version))
            {
                seen.push(entry.version.clone());
            }
        }
        Bytes::from(
            serde_json::to_vec(&json!({ "versions": seen })).expect("flat index serializes"),
        )
    }

    fn registration_index(&self, repository: &Repository, versions: &[VersionEntry]) -> Bytes {
        let Some(first) = versions.first() else {
            return Bytes::from_static(b"{}");
        };
        let id_lower = first.id.to_ascii_lowercase();
        let base = self.resource_base(repository);
        let registration = format!("{base}/v3/registration/{id_lower}/index.json");
        let items: Vec<Value> = versions
            .iter()
            .map(|entry| {
                let ver_lower = entry.version.to_ascii_lowercase();
                let leaf = format!("{base}/v3/registration/{id_lower}/{ver_lower}.json");
                let content = format!(
                    "{base}/v3/flat/{id_lower}/{ver_lower}/{}",
                    nupkg_filename(&entry.id, &entry.version)
                );
                json!({
                    "@id": leaf,
                    "catalogEntry": {
                        "@id": leaf,
                        "id": entry.id,
                        "version": entry.version,
                        "description": entry.description,
                        "authors": entry.authors,
                        "listed": !entry.yanked,
                        "packageContent": content
                    },
                    "packageContent": content,
                    "registration": registration
                })
            })
            .collect();
        let lower = versions
            .iter()
            .map(|entry| entry.version.as_str())
            .min()
            .unwrap_or("");
        let upper = versions
            .iter()
            .map(|entry| entry.version.as_str())
            .max()
            .unwrap_or("");
        Bytes::from(
            serde_json::to_vec(&json!({
                "count": 1,
                "items": [{
                    "@id": format!("{registration}#page/{lower}/{upper}"),
                    "count": items.len(),
                    "lower": lower,
                    "upper": upper,
                    "items": items
                }]
            }))
            .expect("registration serializes"),
        )
    }

    async fn search_json(
        &self,
        repository: &Repository,
        query: &str,
        skip: usize,
        take: usize,
    ) -> Result<Bytes, PackagingError> {
        let hits = self.search(repository, query, skip + take).await?;
        let page: Vec<&PackageSearchHit> = hits.iter().skip(skip).take(take).collect();
        let base = self.resource_base(repository);
        let mut data = Vec::new();
        for hit in page {
            let id_lower = hit.name.to_ascii_lowercase();
            data.push(json!({
                "id": hit.name,
                "version": hit.max_version,
                "description": "",
                "totalDownloads": 0,
                "verified": false,
                "versions": [{
                    "version": hit.max_version,
                    "downloads": 0,
                    "@id": format!(
                        "{base}/v3/registration/{id_lower}/{}.json",
                        hit.max_version.to_ascii_lowercase()
                    )
                }]
            }));
        }
        Ok(Bytes::from(
            serde_json::to_vec(&json!({
                "totalHits": hits.len(),
                "data": data
            }))
            .expect("search serializes"),
        ))
    }

    async fn autocomplete_json(
        &self,
        repository: &Repository,
        query: &str,
        take: usize,
    ) -> Result<Bytes, PackagingError> {
        let hits = self.search(repository, query, take).await?;
        let data: Vec<String> = hits.into_iter().map(|hit| hit.name).collect();
        Ok(Bytes::from(
            serde_json::to_vec(&json!({
                "totalHits": data.len(),
                "data": data
            }))
            .expect("autocomplete serializes"),
        ))
    }
}

#[async_trait]
impl PackagingStrategy for NugetPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Nuget
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_nuget_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let parsed = parse_nupkg(&payload)?;
        self.store_nupkg(repository, &parsed, payload).await
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_nuget_repository(repository)?;
        let versions = self.collect_package_entries(repository, name).await?;
        if versions.is_empty() {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        }
        Ok(self.registration_index(repository, &versions))
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_nuget_repository(repository)?;
        self.nupkg_bytes(
            repository,
            coordinate.name().as_str(),
            coordinate.version().as_str(),
        )
        .await
    }

    async fn download_file(
        &self,
        repository: &Repository,
        filename: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_nuget_repository(repository)?;
        if let Some((id, version)) = split_nupkg_filename(filename) {
            return self.nupkg_bytes(repository, &id, &version).await;
        }
        Err(PackagingError::FileNotFound(filename.to_string()))
    }

    async fn get_protocol_file(
        &self,
        repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_nuget_repository(repository)?;
        match parse_nuget_path(path)? {
            NugetResource::ServiceIndex => Ok(self.service_index(repository)),
            NugetResource::FlatIndex { id } => {
                let name = PackageName::parse(id.to_ascii_lowercase())
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
                let versions = self.collect_package_entries(repository, &name).await?;
                if versions.is_empty() {
                    return Err(PackagingError::PackageNotFound(id));
                }
                Ok(Self::flat_index(&versions))
            }
            NugetResource::Nupkg { id, version } => {
                self.nupkg_bytes(repository, &id, &version).await
            }
            NugetResource::Nuspec { id, version } => {
                let nupkg = self.nupkg_bytes(repository, &id, &version).await?;
                let parsed = parse_nupkg(&nupkg)?;
                Ok(Bytes::from(parsed.nuspec))
            }
            NugetResource::Registration { id } => {
                let name = PackageName::parse(id.to_ascii_lowercase())
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
                let versions = self.collect_package_entries(repository, &name).await?;
                if versions.is_empty() {
                    return Err(PackagingError::PackageNotFound(id));
                }
                Ok(self.registration_index(repository, &versions))
            }
            NugetResource::Query { query, skip, take } => {
                self.search_json(repository, &query, skip, take).await
            }
            NugetResource::Autocomplete { query, take } => {
                self.autocomplete_json(repository, &query, take).await
            }
        }
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
        Self::ensure_nuget_repository(repository)?;
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
        Self::ensure_nuget_repository(source)?;
        Self::ensure_nuget_repository(target)?;
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
        if self.load_version_entry(target, coordinate).await?.is_some() {
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
        Self::ensure_nuget_repository(repository)?;
        let needle = query.trim().to_ascii_lowercase();
        let mut hits = BTreeMap::new();
        for target in self.resolve_read_targets(repository).await? {
            let entries = self
                .package_index_store
                .entries_for_repository(target.id(), PackageEcosystem::Nuget)
                .await?;
            for entry_bytes in entries {
                let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
                if entry.yanked {
                    continue;
                }
                if !needle.is_empty() && !entry.id.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                hits.entry(entry.id).or_insert(entry.version);
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
    id: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    authors: String,
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

struct ParsedNupkg {
    id: String,
    version: String,
    description: String,
    authors: String,
    nuspec: String,
}

enum NugetResource {
    ServiceIndex,
    FlatIndex {
        id: String,
    },
    Nupkg {
        id: String,
        version: String,
    },
    Nuspec {
        id: String,
        version: String,
    },
    Registration {
        id: String,
    },
    Query {
        query: String,
        skip: usize,
        take: usize,
    },
    Autocomplete {
        query: String,
        take: usize,
    },
}

fn nuget_coordinate(id: &str, version: &str) -> Result<PackageCoordinate, PackagingError> {
    Ok(PackageCoordinate::new(
        PackageEcosystem::Nuget,
        PackageName::parse(id.to_ascii_lowercase())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
        PackageVersion::parse(version)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?,
    ))
}

fn nupkg_filename(id: &str, version: &str) -> String {
    format!(
        "{}.{}.nupkg",
        id.to_ascii_lowercase(),
        version.to_ascii_lowercase()
    )
}

fn split_nupkg_filename(filename: &str) -> Option<(String, String)> {
    let stem = filename.strip_suffix(".nupkg")?;
    let dot = stem.rfind('.')?;
    let version = stem.get(dot + 1..)?;
    let id = stem.get(..dot)?;
    if id.is_empty() || version.is_empty() {
        return None;
    }
    Some((id.to_string(), version.to_string()))
}

fn parse_artifact_id(value: &str) -> Result<ArtifactId, PackagingError> {
    Uuid::parse_str(value)
        .map(ArtifactId::from)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))
}

fn nuget_service_index_url(upstream: &Url) -> String {
    let text = upstream.as_str().trim_end_matches('/');
    if text.ends_with("index.json") {
        text.to_string()
    } else if text.ends_with("/v3") {
        format!("{text}/index.json")
    } else {
        format!("{text}/v3/index.json")
    }
}

/// Id and version of a `.nupkg`. The index and `.nuspec` do not trigger.
#[must_use]
pub fn admission_download_target(path: &str) -> Option<(String, String)> {
    match parse_nuget_path(path).ok()? {
        NugetResource::Nupkg { id, version } => Some((id.to_ascii_lowercase(), version)),
        _ => None,
    }
}

fn parse_nuget_path(path: &str) -> Result<NugetResource, PackagingError> {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() || trimmed == "index.json" || trimmed == "v3/index.json" {
        return Ok(NugetResource::ServiceIndex);
    }
    if trimmed == "v3/query" || trimmed == "query" {
        return Ok(NugetResource::Query {
            query: query_param(query, "q").unwrap_or_default(),
            skip: query_param(query, "skip")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            take: query_param(query, "take")
                .and_then(|value| value.parse().ok())
                .unwrap_or(20),
        });
    }
    if trimmed == "v3/autocomplete" || trimmed == "autocomplete" {
        return Ok(NugetResource::Autocomplete {
            query: query_param(query, "q").unwrap_or_default(),
            take: query_param(query, "take")
                .and_then(|value| value.parse().ok())
                .unwrap_or(20),
        });
    }
    if let Some(rest) = trimmed
        .strip_prefix("v3/flat/")
        .or_else(|| trimmed.strip_prefix("flat/"))
    {
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() == 2 && segments[1] == "index.json" {
            return Ok(NugetResource::FlatIndex {
                id: segments[0].to_string(),
            });
        }
        if segments.len() == 3 {
            let filename = segments[2];
            if extension_eq(filename, "nupkg") {
                return Ok(NugetResource::Nupkg {
                    id: segments[0].to_string(),
                    version: segments[1].to_string(),
                });
            }
            if extension_eq(filename, "nuspec") {
                return Ok(NugetResource::Nuspec {
                    id: segments[0].to_string(),
                    version: segments[1].to_string(),
                });
            }
        }
    }
    if let Some(rest) = trimmed
        .strip_prefix("v3/registration/")
        .or_else(|| trimmed.strip_prefix("registration/"))
    {
        let id = rest
            .trim_end_matches('/')
            .strip_suffix("/index.json")
            .or_else(|| rest.strip_suffix(".json"))
            .unwrap_or(rest)
            .trim_end_matches('/')
            .to_string();
        if !id.is_empty() {
            return Ok(NugetResource::Registration { id });
        }
    }
    Err(PackagingError::FileNotFound(path.to_string()))
}

fn extension_eq(filename: &str, extension: &str) -> bool {
    std::path::Path::new(filename)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        if pair == key {
            return Some(String::new());
        }
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name == key {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            )
        {
            out.push(char::from(byte));
            index += 3;
            continue;
        }
        if bytes[index] == b'+' {
            out.push(' ');
        } else {
            out.push(char::from(bytes[index]));
        }
        index += 1;
    }
    out
}

/// Extracts the `.nupkg` from a raw body or `multipart/form-data`.
///
/// # Errors
///
/// Returns [`PackagingError::InvalidPayload`] if the body is
/// `multipart` and the first part cannot be extracted.
pub fn extract_nupkg_bytes(
    content_type: Option<&str>,
    body: Bytes,
) -> Result<Bytes, PackagingError> {
    let Some(content_type) = content_type else {
        return Ok(body);
    };
    if !content_type.to_ascii_lowercase().contains("multipart/") {
        return Ok(body);
    }
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .map(str::trim)
        .map(|value| value.trim_matches('"'))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PackagingError::InvalidPayload("multipart is missing a boundary".to_string())
        })?;
    extract_first_multipart_part(&body, boundary)
}

fn extract_first_multipart_part(body: &[u8], boundary: &str) -> Result<Bytes, PackagingError> {
    let delim = format!("--{boundary}").into_bytes();
    let start = find_subslice(body, &delim).ok_or_else(|| {
        PackagingError::InvalidPayload("multipart body is missing the boundary".to_string())
    })?;
    let after = body.get(start + delim.len()..).unwrap_or(&[]);
    let after = after
        .strip_prefix(b"\r\n")
        .or_else(|| after.strip_prefix(b"\n"))
        .unwrap_or(after);
    let header_end = find_subslice(after, b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| find_subslice(after, b"\n\n").map(|index| index + 2))
        .ok_or_else(|| {
            PackagingError::InvalidPayload("multipart part is missing headers".to_string())
        })?;
    let content = &after[header_end..];
    let next = format!("\r\n--{boundary}").into_bytes();
    let alt = format!("\n--{boundary}").into_bytes();
    let end = find_subslice(content, &next)
        .or_else(|| find_subslice(content, &alt))
        .unwrap_or(content.len());
    Ok(Bytes::copy_from_slice(&content[..end]))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_nupkg(body: &[u8]) -> Result<ParsedNupkg, PackagingError> {
    let cursor = Cursor::new(body);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|err| PackagingError::InvalidPayload(format!("nupkg is not a zip: {err}")))?;
    let mut nuspec = None;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let name = file.name().to_string();
        if !name.to_ascii_lowercase().ends_with(".nuspec") {
            continue;
        }
        let mut buf = String::new();
        file.read_to_string(&mut buf)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        nuspec = Some(buf);
        break;
    }
    let nuspec = nuspec.ok_or_else(|| {
        PackagingError::InvalidPayload("nupkg does not contain a .nuspec".to_string())
    })?;
    let metadata = xml_section(&nuspec, "metadata").unwrap_or(nuspec.as_str());
    let id = xml_text(metadata, "id")
        .ok_or_else(|| PackagingError::InvalidPayload("nuspec is missing <id>".to_string()))?;
    let version = xml_text(metadata, "version")
        .ok_or_else(|| PackagingError::InvalidPayload("nuspec is missing <version>".to_string()))?;
    validate_package_id(&id)?;
    Ok(ParsedNupkg {
        id,
        version,
        description: xml_text(metadata, "description").unwrap_or_default(),
        authors: xml_text(metadata, "authors").unwrap_or_default(),
        nuspec,
    })
}

fn validate_package_id(id: &str) -> Result<(), PackagingError> {
    if id.is_empty() || id.len() > 100 {
        return Err(PackagingError::InvalidPayload(
            "package id must be 1-100 characters".to_string(),
        ));
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return Err(PackagingError::InvalidPayload(
            "package id may only contain letters, digits, '.', '-' and '_'".to_string(),
        ));
    }
    if !id.chars().any(char::is_alphabetic) {
        return Err(PackagingError::InvalidPayload(
            "package id must contain a letter".to_string(),
        ));
    }
    Ok(())
}

fn xml_section<'a>(body: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{tag}");
    let start = body
        .to_ascii_lowercase()
        .find(&start_tag.to_ascii_lowercase())?;
    let after = body.get(start..)?.find('>')? + start + 1;
    let close = format!("</{tag}>");
    let end_rel = body
        .get(after..)?
        .to_ascii_lowercase()
        .find(&close.to_ascii_lowercase())?;
    body.get(after..after + end_rel)
}

fn xml_text(body: &str, tag: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let patterns = [format!("<{tag}"), format!(":{tag}")];
    for pattern in patterns {
        let mut search_from = 0;
        while let Some(rel) = lower.get(search_from..)?.find(&pattern) {
            let pos = search_from + rel;
            let Some(gt) = body.get(pos..)?.find('>') else {
                break;
            };
            let start = pos + gt + 1;
            let Some(end_rel) = body.get(start..)?.find('<') else {
                break;
            };
            let value = body.get(start..start + end_rel)?.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
            search_from = start;
        }
    }
    None
}

/// `true` if the `NuGet` version is a `SemVer` prerelease (`1.0.0-beta`).
#[must_use]
pub fn is_prerelease_version(version: &str) -> bool {
    let core = version.split_once('+').map_or(version, |(core, _)| core);
    core.contains('-')
}

/// Builds a minimal `.nupkg` (zip + `.nuspec`) for tests.
///
/// # Panics
///
/// Panics if the zip cannot be written in memory.
#[must_use]
pub fn build_nupkg(id: &str, version: &str) -> Bytes {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        zip.start_file(
            format!("{id}.nuspec"),
            zip::write::SimpleFileOptions::default(),
        )
        .expect("nuspec entry");
        zip.write_all(
            format!(
                r#"<?xml version="1.0"?>
<package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
  <metadata>
    <id>{id}</id>
    <version>{version}</version>
    <authors>FerroBox</authors>
    <description>test package</description>
  </metadata>
</package>"#
            )
            .as_bytes(),
        )
        .expect("nuspec bytes");
        zip.finish().expect("nupkg zip");
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

    fn strategy() -> NugetPackagingStrategy {
        NugetPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000",
        )
    }

    fn nuget_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Nuget,
        )
        .unwrap()
    }

    #[test]
    fn detects_prerelease_versions() {
        assert!(is_prerelease_version("1.0.0-beta"));
        assert!(is_prerelease_version("1.0.0-rc.1+build"));
        assert!(!is_prerelease_version("1.0.0"));
        assert!(!is_prerelease_version("1.0.0+build"));
    }

    #[test]
    fn admission_download_target_is_nupkg_only() {
        assert_eq!(
            admission_download_target("v3/flat/hello.world/1.0.0/hello.world.1.0.0.nupkg"),
            Some(("hello.world".into(), "1.0.0".into()))
        );
        assert_eq!(
            admission_download_target("v3/flat/hello.world/index.json"),
            None
        );
    }

    #[test]
    fn parses_a_minimal_nupkg() {
        let parsed = parse_nupkg(&build_nupkg("Hello.World", "1.0.0")).unwrap();
        assert_eq!(parsed.id, "Hello.World");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(parsed.authors, "FerroBox");
    }

    #[test]
    fn extracts_nupkg_from_multipart() {
        let nupkg = build_nupkg("Hello.World", "1.0.0");
        let mut body = Vec::new();
        body.extend_from_slice(
            b"--abc\r\nContent-Disposition: form-data; name=\"package\"\r\n\r\n",
        );
        body.extend_from_slice(&nupkg);
        body.extend_from_slice(b"\r\n--abc--\r\n");
        let extracted =
            extract_nupkg_bytes(Some("multipart/form-data; boundary=abc"), Bytes::from(body))
                .unwrap();
        let parsed = parse_nupkg(&extracted).unwrap();
        assert_eq!(parsed.id, "Hello.World");
    }

    #[tokio::test]
    async fn publish_then_flat_and_registration() {
        let strategy = strategy();
        let repository = nuget_forge("nuget-local");
        let outcome = strategy
            .publish(&repository, build_nupkg("Hello.World", "1.0.0"))
            .await
            .unwrap();
        assert_eq!(outcome.name().as_str(), "hello.world");

        let flat = strategy
            .get_protocol_file(&repository, "v3/flat/hello.world/index.json")
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&flat).unwrap();
        assert_eq!(json["versions"][0], "1.0.0");

        let nupkg = strategy
            .get_protocol_file(
                &repository,
                "v3/flat/hello.world/1.0.0/hello.world.1.0.0.nupkg",
            )
            .await
            .unwrap();
        assert_eq!(parse_nupkg(&nupkg).unwrap().id, "Hello.World");

        let registration = strategy
            .get_protocol_file(&repository, "v3/registration/hello.world/index.json")
            .await
            .unwrap();
        let reg: Value = serde_json::from_slice(&registration).unwrap();
        assert_eq!(
            reg["items"][0]["items"][0]["catalogEntry"]["id"],
            "Hello.World"
        );
        assert_eq!(reg["items"][0]["items"][0]["catalogEntry"]["listed"], true);

        let index = strategy
            .get_protocol_file(&repository, "v3/index.json")
            .await
            .unwrap();
        let service: Value = serde_json::from_slice(&index).unwrap();
        assert_eq!(service["version"], "3.0.0");
    }

    #[tokio::test]
    async fn rejects_overwrite_and_unlists_from_search() {
        let strategy = strategy();
        let repository = nuget_forge("nuget-local");
        strategy
            .publish(&repository, build_nupkg("Hello.World", "1.0.0"))
            .await
            .unwrap();
        let err = strategy
            .publish(&repository, build_nupkg("Hello.World", "1.0.0"))
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::AlreadyPublished(_)));

        let coordinate = nuget_coordinate("Hello.World", "1.0.0").unwrap();
        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();
        let hits = strategy.search(&repository, "hello", 10).await.unwrap();
        assert!(hits.is_empty());
        let nupkg = strategy.download(&repository, &coordinate).await.unwrap();
        assert_eq!(parse_nupkg(&nupkg).unwrap().version, "1.0.0");
    }

    #[tokio::test]
    async fn mirror_caches_upstream_nupkg() {
        let nupkg = build_nupkg("Newtonsoft.Json", "13.0.3");
        let http = InMemoryHttpClient::default();
        http.stub(
            "https://api.nuget.org/v3/index.json",
            200,
            Bytes::from(
                serde_json::to_vec(&json!({
                    "version": "3.0.0",
                    "resources": [{
                        "@id": "https://api.nuget.org/v3-flatcontainer/",
                        "@type": "PackageBaseAddress/3.0.0"
                    }]
                }))
                .unwrap(),
            ),
        );
        http.stub(
            "https://api.nuget.org/v3-flatcontainer/newtonsoft.json/13.0.3/newtonsoft.json.13.0.3.nupkg",
            200,
            nupkg,
        );
        let strategy = NugetPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(http),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000",
        );
        let repository = Repository::new(
            RepositoryName::parse("nuget-gallery").unwrap(),
            RepositoryKind::Mirror {
                upstream: Url::parse("https://api.nuget.org/v3/index.json").unwrap(),
            },
            PackageEcosystem::Nuget,
        )
        .unwrap();
        let body = strategy
            .get_protocol_file(
                &repository,
                "v3/flat/newtonsoft.json/13.0.3/newtonsoft.json.13.0.3.nupkg",
            )
            .await
            .unwrap();
        assert_eq!(parse_nupkg(&body).unwrap().id, "Newtonsoft.Json");
    }

    #[tokio::test]
    async fn alloy_reads_first_member_and_unions_search() {
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let strategy = NugetPackagingStrategy::new(
            artifacts,
            index,
            storage,
            Arc::new(InMemoryHttpClient::default()),
            repos.clone(),
            "http://127.0.0.1:3000",
        );
        let first = nuget_forge("nuget-one");
        let second = nuget_forge("nuget-two");
        repos.save(&first).await.unwrap();
        repos.save(&second).await.unwrap();
        strategy
            .publish(&first, build_nupkg("Hello.World", "1.0.0"))
            .await
            .unwrap();
        strategy
            .publish(&second, build_nupkg("Other.Lib", "2.0.0"))
            .await
            .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("nuget-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first.id(), second.id()],
            },
            PackageEcosystem::Nuget,
        )
        .unwrap();
        let hits = strategy.search(&alloy, "", 10).await.unwrap();
        assert_eq!(hits.len(), 2);
        let nupkg = strategy
            .get_protocol_file(&alloy, "v3/flat/hello.world/1.0.0/hello.world.1.0.0.nupkg")
            .await
            .unwrap();
        assert_eq!(parse_nupkg(&nupkg).unwrap().id, "Hello.World");
    }

    #[tokio::test]
    async fn promote_copies_the_nupkg() {
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = NugetPackagingStrategy::new(
            artifacts,
            index,
            storage,
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000",
        );
        let source = nuget_forge("nuget-dev");
        let target = nuget_forge("nuget-prod");
        strategy
            .publish(&source, build_nupkg("Hello.World", "1.0.0"))
            .await
            .unwrap();
        let coordinate = nuget_coordinate("Hello.World", "1.0.0").unwrap();
        let outcome = strategy
            .promote_version(&source, &target, &coordinate, false)
            .await
            .unwrap();
        assert_eq!(outcome.artifacts_copied, 1);
        let copied = strategy.download(&target, &coordinate).await.unwrap();
        assert_eq!(parse_nupkg(&copied).unwrap().id, "Hello.World");
    }
}
