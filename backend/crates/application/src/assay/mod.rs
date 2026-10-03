//! Assay of a package version: inventory and vulnerabilities.

mod cyclonedx;
mod extract;
mod feed;
mod layers;
mod licenses;
mod lockfiles;
mod osv;
mod sync;

pub use feed::{FeedError, OsvFeed};
pub use sync::{SyncError, SyncOutcome};

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use ferrobox_domain::assay::{Assay, AssayComponent, AssayStatus};
use ferrobox_domain::ids::{AssayId, RepositoryId};
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::assay_store::{AssayStore, AssayStoreError};
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use ferrobox_ports::osv_feed_store::{
    OSV_FEED_SOURCE_FILE, OSV_FEED_SOURCE_SYNC, OsvFeedRecord, OsvFeedStore, OsvFeedStoreError,
};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::webhooks::WebhookService;

use self::extract::osv_query_target;
use self::layers::extract_inventory;
use self::osv::query_findings;

pub use self::cyclonedx::to_cyclonedx;
pub use self::extract::{is_exact_version, purl_for};

/// Reasons an assay can fail.
#[derive(Debug, Error)]
pub enum AssayError {
    /// The repository does not exist.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// The assay does not exist.
    #[error("assay {0} was not found")]
    AssayNotFound(AssayId),

    /// There is no published version with that coordinate.
    #[error("{0} was not found")]
    PackageNotFound(PackageCoordinate),

    /// The name or the version is not valid.
    #[error("invalid package coordinate: {0}")]
    InvalidCoordinate(String),

    /// Failed to persist the assay.
    #[error(transparent)]
    Persistence(#[from] AssayStoreError),

    /// Failed to read the package index.
    #[error(transparent)]
    Index(#[from] PackageIndexStoreError),

    /// Failed to read repositories.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),

    /// Failed to read blobs or manifests from storage.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Why importing or restoring a vulnerability index failed.
#[derive(Debug, Error)]
pub enum FeedImportError {
    /// This process was not given a place to record the import.
    #[error("vulnerability feed import is not configured")]
    NotConfigured,

    /// The declared SHA-256 does not match the body.
    #[error("sha256 mismatch: declared {expected}, body is {actual}")]
    ChecksumMismatch {
        /// Hex digest sent by the caller.
        expected: String,
        /// Hex digest of the body.
        actual: String,
    },

    /// The body is not a `ferrobox-osv-index` v1 document.
    #[error(transparent)]
    Invalid(#[from] feed::FeedError),

    /// The object store rejected the bytes.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// The metadata row could not be saved or read.
    #[error(transparent)]
    Persistence(#[from] OsvFeedStoreError),
}

/// Use case: list, get, and run assays.
#[derive(Clone)]
pub struct AssayService {
    assays: Arc<dyn AssayStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    repository_store: Arc<dyn RepositoryStore>,
    storage: Arc<dyn StoragePort>,
    http_client: Arc<dyn HttpClient>,
    webhooks: Option<WebhookService>,
    /// When set, vulnerability lookup uses this index and does not call
    /// `api.osv.dev`. Shared across clones so an import is visible at once.
    feed: Arc<RwLock<Option<Arc<OsvFeed>>>>,
    /// Metadata of an imported index. Absent in unit tests that only
    /// inject a feed with [`Self::with_feed`].
    feed_store: Option<Arc<dyn OsvFeedStore>>,
}

impl AssayService {
    /// Builds the service from its ports.
    #[must_use]
    pub fn new(
        assays: Arc<dyn AssayStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        repository_store: Arc<dyn RepositoryStore>,
        storage: Arc<dyn StoragePort>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            assays,
            package_index_store,
            repository_store,
            storage,
            http_client,
            webhooks: None,
            feed: Arc::new(RwLock::new(None)),
            feed_store: None,
        }
    }

    /// Connects HTTP webhook delivery. A destination failure does not
    /// affect the assay or the publish.
    #[must_use]
    pub fn with_webhooks(mut self, webhooks: WebhookService) -> Self {
        self.webhooks = Some(webhooks);
        self
    }

    /// Uses a local vulnerability index instead of `api.osv.dev`.
    #[must_use]
    pub fn with_feed(self, feed: Arc<OsvFeed>) -> Self {
        self.install_feed(feed);
        self
    }

    /// Remembers imported indexes in `store`.
    #[must_use]
    pub fn with_feed_store(mut self, store: Arc<dyn OsvFeedStore>) -> Self {
        self.feed_store = Some(store);
        self
    }

    /// The index an administrator imported, if any.
    ///
    /// # Errors
    ///
    /// Returns [`FeedImportError`] if the metadata cannot be read.
    pub async fn imported_feed(&self) -> Result<Option<OsvFeedRecord>, FeedImportError> {
        let Some(store) = &self.feed_store else {
            return Ok(None);
        };
        match store.current().await {
            Ok(record) => Ok(record),
            Err(OsvFeedStoreError::MissingSchema) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Checks the SHA-256, parses the index, stores the bytes, and makes
    /// this process use them. A mismatch or a bad document leaves the
    /// previous index in place. A new checksum marks every stored assay
    /// `Running` and re-runs it in the background against this index.
    ///
    /// # Errors
    ///
    /// Returns [`FeedImportError`] if the checksum does not match, the
    /// document is not a v1 index, or the bytes cannot be saved.
    pub async fn import_feed(
        &self,
        bytes: Bytes,
        expected_sha256: &str,
    ) -> Result<OsvFeedRecord, FeedImportError> {
        self.import_feed_with_source(bytes, expected_sha256, OSV_FEED_SOURCE_FILE)
            .await
    }

    /// Pulls `reference`, verifies its Cosign signature, and installs the
    /// index layer when the checksum is new. A rejected pull leaves the
    /// active index untouched. The same checksum does not re-run assays.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the registry or the signature cannot
    /// be trusted, or the layer is not a v1 index.
    pub async fn sync_signed_feed(
        &self,
        reference: &str,
        public_key_pem: &str,
        token: Option<&str>,
    ) -> Result<SyncOutcome, SyncError> {
        let bytes =
            sync::pull_signed_index(self.http_client.as_ref(), reference, public_key_pem, token)
                .await?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if let Some(current) = self.imported_feed().await?
            && current.sha256 == digest
        {
            return Ok(SyncOutcome::Unchanged { sha256: digest });
        }
        let record = self
            .import_feed_with_source(bytes, &digest, OSV_FEED_SOURCE_SYNC)
            .await?;
        Ok(SyncOutcome::Imported(record))
    }

    async fn import_feed_with_source(
        &self,
        bytes: Bytes,
        expected_sha256: &str,
        source: &str,
    ) -> Result<OsvFeedRecord, FeedImportError> {
        let Some(store) = &self.feed_store else {
            return Err(FeedImportError::NotConfigured);
        };
        let actual = format!("{:x}", Sha256::digest(&bytes));
        let expected = expected_sha256.trim().to_ascii_lowercase();
        if expected != actual {
            return Err(FeedImportError::ChecksumMismatch { expected, actual });
        }
        let parsed = OsvFeed::from_bytes(&bytes)?;
        let storage_key = format!("system/osv-feed/{actual}");
        let previous = match store.current().await {
            Ok(record) => record,
            Err(OsvFeedStoreError::MissingSchema) => None,
            Err(err) => return Err(err.into()),
        };
        self.storage
            .put(&StorageKey::new(&storage_key), bytes)
            .await?;
        let record = OsvFeedRecord {
            dataset: parsed.dataset().to_string(),
            sha256: actual,
            advisory_count: u64::try_from(parsed.advisory_count()).unwrap_or(u64::MAX),
            ecosystems: parsed.ecosystems(),
            storage_key: storage_key.clone(),
            imported_at: Utc::now(),
            source: source.to_string(),
        };
        if let Err(err) = store.save(&record).await {
            return Err(err.into());
        }
        let feed_changed = previous
            .as_ref()
            .is_none_or(|previous| previous.sha256 != record.sha256);
        if let Some(previous) = previous
            && previous.storage_key != storage_key
        {
            let _ = self
                .storage
                .delete(&StorageKey::new(previous.storage_key))
                .await;
        }
        self.install_feed(Arc::new(parsed));
        if feed_changed {
            let _ = self.rerun_all().await;
        }
        Ok(record)
    }

    /// Loads the imported index from storage into this process.
    ///
    /// A file previously passed to [`Self::with_feed`] stays in place
    /// when nothing has been imported. A stored index replaces it.
    ///
    /// # Errors
    ///
    /// Returns [`FeedImportError`] if the stored bytes are missing or
    /// are no longer a valid index.
    pub async fn restore_persisted_feed(&self) -> Result<(), FeedImportError> {
        let Some(record) = self.imported_feed().await? else {
            return Ok(());
        };
        let bytes = self
            .storage
            .get(&StorageKey::new(&record.storage_key))
            .await?;
        let parsed = OsvFeed::from_bytes(&bytes)?;
        self.install_feed(Arc::new(parsed));
        Ok(())
    }

    fn install_feed(&self, feed: Arc<OsvFeed>) {
        let mut slot = self
            .feed
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(feed);
    }

    fn current_feed(&self) -> Option<Arc<OsvFeed>> {
        self.feed
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Lists every assay of the instance.
    ///
    /// # Errors
    ///
    /// Returns [`AssayError::Persistence`] if the store fails.
    pub async fn list_all(&self) -> Result<Vec<Assay>, AssayError> {
        Ok(self.assays.find_all().await?)
    }

    /// Lists the assays of a repository. In an `Alloy`, unions those of
    /// its members.
    ///
    /// # Errors
    ///
    /// Returns [`AssayError::RepositoryNotFound`] if it does not exist.
    pub async fn list_for_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Assay>, AssayError> {
        let repository = self.require_repository(repository_id).await?;
        let mut assays = self.assays.find_by_repository(repository_id).await?;
        if let RepositoryKind::Alloy { members } = repository.kind() {
            for member_id in members {
                assays.extend(self.assays.find_by_repository(*member_id).await?);
            }
        }
        Ok(assays)
    }

    /// Returns the assay of a coordinate. If it does not exist, runs it.
    ///
    /// # Errors
    ///
    /// Returns [`AssayError::PackageNotFound`] if the package is not
    /// indexed, or [`AssayError::RepositoryNotFound`].
    pub async fn get_or_run(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &str,
        version: &str,
    ) -> Result<Assay, AssayError> {
        let (source, coordinate, _entry) = self
            .resolve_indexed(repository_id, ecosystem, name, version)
            .await?;
        if let Some(existing) = self
            .assays
            .find_by_coordinate(source.id(), &coordinate)
            .await?
        {
            return Ok(existing);
        }
        self.run_on(source.id(), &coordinate).await
    }

    /// Re-assays a coordinate, replacing the previous result.
    ///
    /// # Errors
    ///
    /// Returns [`AssayError::PackageNotFound`] if the package is not
    /// indexed.
    pub async fn run(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &str,
        version: &str,
    ) -> Result<Assay, AssayError> {
        let (source, coordinate, _entry) = self
            .resolve_indexed(repository_id, ecosystem, name, version)
            .await?;
        self.run_on(source.id(), &coordinate).await
    }

    /// Looks up an assay by identifier.
    ///
    /// # Errors
    ///
    /// Returns [`AssayError::PackageNotFound`] if it does not exist
    /// (the same HTTP 404 is reused).
    pub async fn get_by_id(&self, id: AssayId) -> Result<Assay, AssayError> {
        self.assays
            .find_by_id(id)
            .await?
            .ok_or(AssayError::AssayNotFound(id))
    }

    /// Starts an assay in the background. Does not block `publish` or
    /// `install`: a failure is persisted as `Failed`.
    pub fn schedule(&self, repository_id: RepositoryId, coordinate: PackageCoordinate) {
        if !should_auto_assay(&coordinate) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            let _ = service
                .run(
                    repository_id,
                    coordinate.ecosystem(),
                    coordinate.name().as_str(),
                    coordinate.version().as_str(),
                )
                .await;
        });
    }

    /// Like [`schedule`], and also notifies `package.published` without
    /// waiting for the destination.
    pub fn schedule_after_publish(
        &self,
        repository_id: RepositoryId,
        coordinate: PackageCoordinate,
    ) {
        if let Some(webhooks) = &self.webhooks {
            webhooks.notify_package_published(repository_id, coordinate.clone());
        }
        self.schedule(repository_id, coordinate);
    }

    /// Re-assays the coordinates of an OCI/Helm image after caching a
    /// layer. Used to go from `unsupported` (manifest without blobs) to
    /// a real inventory without blocking the `pull`.
    pub fn reschedule_for_name(&self, repository_id: RepositoryId, name: &str) {
        let name = name.to_string();
        let service = self.clone();
        tokio::spawn(async move {
            let Ok(assays) = service.assays.find_by_repository(repository_id).await else {
                return;
            };
            for assay in assays {
                if assay.coordinate().name().as_str() == name {
                    let _ = service.assays.upsert(&assay.mark_running()).await;
                    let _ = service.run_on(repository_id, assay.coordinate()).await;
                }
            }
        });
    }

    /// Re-assays in the background every coordinate already assayed.
    ///
    /// # Errors
    ///
    /// Returns [`AssayError::Persistence`] if the store fails.
    pub async fn rerun_all(&self) -> Result<u32, AssayError> {
        let assays = self.assays.find_all().await?;
        let mut seen = HashSet::new();
        let mut scheduled = 0_u32;
        for assay in assays {
            let key = (assay.repository_id(), assay.coordinate().clone());
            if !seen.insert(key) {
                continue;
            }
            self.assays.upsert(&assay.mark_running()).await?;
            self.schedule(assay.repository_id(), assay.coordinate().clone());
            scheduled += 1;
        }
        Ok(scheduled)
    }

    async fn run_on(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Assay, AssayError> {
        let entry = self
            .load_entry(repository_id, coordinate)
            .await?
            .ok_or_else(|| AssayError::PackageNotFound(coordinate.clone()))?;

        let existing_id = self
            .assays
            .find_by_coordinate(repository_id, coordinate)
            .await?
            .map_or_else(AssayId::new, |assay| assay.id());

        let components = extract_inventory(
            self.storage.as_ref(),
            self.package_index_store.as_ref(),
            repository_id,
            coordinate,
            &entry,
        )
        .await;
        let scanned_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let can_query = components
            .iter()
            .any(|component| osv_query_target(component, coordinate.ecosystem()).is_some());

        let assay = if can_query {
            if let Some(feed) = self.current_feed() {
                let findings = feed.query(coordinate.ecosystem(), &components);
                Assay::from_parts(
                    existing_id,
                    repository_id,
                    coordinate.clone(),
                    AssayStatus::Ready,
                    Some(scanned_at.clone()),
                    None,
                    components,
                    findings,
                )
            } else {
                self.assay_from_osv_http(
                    existing_id,
                    repository_id,
                    coordinate,
                    components,
                    &scanned_at,
                )
                .await
            }
        } else if components.len() > 1 {
            Assay::from_parts(
                existing_id,
                repository_id,
                coordinate.clone(),
                AssayStatus::Ready,
                Some(scanned_at),
                composition_only_message(coordinate.ecosystem()),
                components,
                Vec::new(),
            )
        } else {
            Assay::from_parts(
                existing_id,
                repository_id,
                coordinate.clone(),
                AssayStatus::Unsupported,
                Some(scanned_at),
                Some(unsupported_message(coordinate.ecosystem())),
                components,
                Vec::new(),
            )
        };

        self.assays.upsert(&assay).await?;
        if let Some(webhooks) = &self.webhooks {
            webhooks.notify_assay_completed(assay.clone());
        }
        Ok(assay)
    }

    async fn assay_from_osv_http(
        &self,
        existing_id: AssayId,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
        components: Vec<AssayComponent>,
        scanned_at: &str,
    ) -> Assay {
        match query_findings(
            self.http_client.as_ref(),
            coordinate.ecosystem(),
            &components,
        )
        .await
        {
            Ok(findings) => Assay::from_parts(
                existing_id,
                repository_id,
                coordinate.clone(),
                AssayStatus::Ready,
                Some(scanned_at.to_string()),
                None,
                components,
                findings,
            ),
            Err(HttpClientError::Status { status, url }) => Assay::from_parts(
                existing_id,
                repository_id,
                coordinate.clone(),
                AssayStatus::Failed,
                Some(scanned_at.to_string()),
                Some(format!(
                    "OSV (Open Source Vulnerabilities) respondió HTTP {status} para {url}"
                )),
                components,
                Vec::new(),
            ),
            Err(HttpClientError::Transport { url, message }) => Assay::from_parts(
                existing_id,
                repository_id,
                coordinate.clone(),
                AssayStatus::Failed,
                Some(scanned_at.to_string()),
                Some(format!("no se pudo consultar OSV en {url}: {message}")),
                components,
                Vec::new(),
            ),
        }
    }

    async fn require_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Repository, AssayError> {
        self.repository_store
            .find_by_id(repository_id)
            .await?
            .ok_or(AssayError::RepositoryNotFound(repository_id))
    }

    async fn resolve_indexed(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &str,
        version: &str,
    ) -> Result<(Repository, PackageCoordinate, Bytes), AssayError> {
        let name = PackageName::parse(name.to_string())
            .map_err(|err| AssayError::InvalidCoordinate(err.to_string()))?;
        let version = PackageVersion::parse(version.to_string())
            .map_err(|err| AssayError::InvalidCoordinate(err.to_string()))?;
        let coordinate = PackageCoordinate::new(ecosystem, name, version);
        let repository = self.require_repository(repository_id).await?;

        let mut targets = vec![repository.clone()];
        if let RepositoryKind::Alloy { members } = repository.kind() {
            for member_id in members {
                if let Some(member) = self.repository_store.find_by_id(*member_id).await? {
                    targets.push(member);
                }
            }
        }

        for target in targets {
            if let Some(entry) = self.load_entry(target.id(), &coordinate).await? {
                return Ok((target, coordinate, entry));
            }
        }
        Err(AssayError::PackageNotFound(coordinate))
    }

    async fn load_entry(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<Bytes>, AssayError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository_id, coordinate.ecosystem(), coordinate.name())
            .await?;
        for entry in entries {
            if entry_version_matches(&entry, coordinate.version().as_str()) {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }
}

/// Skips OCI blobs and manifests indexed by digest: they are not a
/// version the UI assays.
/// Tells admission whether a local vulnerability index is loaded.
pub trait VulnerabilityFeedSource: Send + Sync {
    /// `true` when assays can answer from a local index.
    fn vulnerability_feed_loaded(&self) -> bool;
}

impl VulnerabilityFeedSource for AssayService {
    fn vulnerability_feed_loaded(&self) -> bool {
        self.current_feed().is_some()
    }
}

pub(crate) fn should_auto_assay(coordinate: &PackageCoordinate) -> bool {
    let name = coordinate.name().as_str();
    let version = coordinate.version().as_str();
    name != "_blob" && !name.is_empty() && !version.is_empty() && !version.starts_with("sha256:")
}

fn composition_only_message(ecosystem: PackageEcosystem) -> Option<String> {
    match ecosystem {
        PackageEcosystem::Helm => Some(
            "OSV (Open Source Vulnerabilities) no indexa charts Helm. Se muestra la composición de Chart.yaml.".to_string(),
        ),
        PackageEcosystem::Conan => Some(
            "OSV (Open Source Vulnerabilities) no indexa Conan. Se muestra la composición de la receta.".to_string(),
        ),
        _ => None,
    }
}

fn unsupported_message(ecosystem: PackageEcosystem) -> String {
    match ecosystem {
        PackageEcosystem::Oci | PackageEcosystem::Helm => {
            "No hay capas locales para inventariar (apk/dpkg o Chart.yaml). Haz pull o push de la imagen/chart para cachear los blobs y vuelve a ensayar.".to_string()
        }
        PackageEcosystem::Conan => {
            "No se encontró conanfile.py ni conanfile.txt con requires en esta receta.".to_string()
        }
        _ => {
            "Assay cubre npm, PyPI, Cargo y el sistema de ficheros de imágenes OCI (Alpine/Debian/Ubuntu).".to_string()
        }
    }
}

fn entry_version_matches(entry: &Bytes, version: &str) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(entry) else {
        return false;
    };
    if ["version", "vers", "reference"]
        .iter()
        .any(|key| value.get(*key).and_then(serde_json::Value::as_str) == Some(version))
    {
        return true;
    }
    // Conan indexes `version`/`user`/`channel` separately; the UI sends
    // the `0.1@_:_` coordinate.
    match (
        value.get("version").and_then(serde_json::Value::as_str),
        value.get("user").and_then(serde_json::Value::as_str),
        value.get("channel").and_then(serde_json::Value::as_str),
    ) {
        (Some(recipe_version), Some(user), Some(channel)) => {
            format!("{recipe_version}@{user}:{channel}") == version
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_key::storage_key_for;
    use crate::test_support::{
        InMemoryAssayStore, InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryRepositoryStore,
        InMemoryStorage,
    };
    use ferrobox_domain::ids::ArtifactId;
    use ferrobox_domain::package_coordinate::PackageName;
    use ferrobox_domain::repository::{RepositoryKind, RepositoryName};
    use ferrobox_ports::storage::StoragePort;
    use sha2::{Digest, Sha256};

    fn npm_forge() -> Repository {
        Repository::new(
            RepositoryName::parse("npm-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Npm,
        )
        .unwrap()
    }

    async fn seed_lodash(index: &InMemoryPackageIndexStore, repository: &Repository) {
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("lodash").unwrap(),
            PackageVersion::parse("4.17.20").unwrap(),
        );
        let entry = Bytes::from(
            serde_json::json!({
                "name": "lodash",
                "version": "4.17.20",
                "manifest": { "license": "MIT", "dependencies": { "foo": "^1.0.0" } }
            })
            .to_string(),
        );
        index
            .upsert_entry(repository.id(), &coordinate, None, entry)
            .await
            .unwrap();
    }

    async fn seed_npm(
        index: &InMemoryPackageIndexStore,
        repository: &Repository,
        name: &str,
        version: &str,
    ) {
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse(name).unwrap(),
            PackageVersion::parse(version).unwrap(),
        );
        let entry = Bytes::from(
            serde_json::json!({
                "name": name,
                "version": version,
                "manifest": { "license": "MIT" }
            })
            .to_string(),
        );
        index
            .upsert_entry(repository.id(), &coordinate, None, entry)
            .await
            .unwrap();
    }

    fn osv_high_lodash_vuln() -> serde_json::Value {
        serde_json::json!({
            "id": "GHSA-35jh-r3h4-6jhm",
            "aliases": ["CVE-2021-23337"],
            "summary": "Command Injection in lodash",
            "database_specific": { "severity": "HIGH" },
            "affected": [{ "ranges": [{ "events": [{ "fixed": "4.17.21" }] }] }],
            "references": [{ "url": "https://github.com/advisories/GHSA-35jh-r3h4-6jhm" }]
        })
    }

    fn osv_high_lodash() -> Bytes {
        Bytes::from(
            serde_json::json!({
                "results": [{
                    "vulns": [osv_high_lodash_vuln()]
                }]
            })
            .to_string(),
        )
    }

    #[test]
    fn entry_version_matches_oci_reference_and_conan_recipe() {
        assert!(entry_version_matches(
            &Bytes::from(r#"{"reference":"latest"}"#),
            "latest",
        ));
        assert!(entry_version_matches(
            &Bytes::from(r#"{"version":"0.1","user":"_","channel":"_"}"#),
            "0.1@_:_",
        ));
        assert!(entry_version_matches(
            &Bytes::from(r#"{"vers":"1.0.0"}"#),
            "1.0.0",
        ));
        assert!(!entry_version_matches(
            &Bytes::from(r#"{"reference":"latest"}"#),
            "1.0.0",
        ));
    }

    #[tokio::test]
    async fn run_records_inventory_and_osv_findings() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub("https://api.osv.dev/v1/querybatch", 200, osv_high_lodash());

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();

        assert_eq!(assay.status(), AssayStatus::Ready);
        assert!(assay.components().iter().any(|c| c.name() == "lodash"));
        assert!(assay.components().iter().any(|c| c.name() == "foo"));
        assert_eq!(
            assay
                .components()
                .iter()
                .find(|component| component.name() == "lodash")
                .map(ferrobox_domain::assay::AssayComponent::licenses),
            Some(["MIT".to_string()].as_slice())
        );
        assert_eq!(assay.counts().high, 1);
        assert_eq!(assay.findings()[0].fixed_version(), Some("4.17.21"));
        let document: serde_json::Value = serde_json::from_slice(&to_cyclonedx(&assay)).unwrap();
        assert_eq!(document["bomFormat"], "CycloneDX");
        assert_eq!(document["vulnerabilities"][0]["id"], "GHSA-35jh-r3h4-6jhm");
    }

    #[tokio::test]
    async fn get_or_run_reuses_existing_assay() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub("https://api.osv.dev/v1/querybatch", 200, osv_high_lodash());

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        let first = service
            .get_or_run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        let second = service
            .get_or_run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(first.id(), second.id());
    }

    #[tokio::test]
    async fn thin_osv_batch_is_hydrated_from_vuln_endpoint() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            Bytes::from(
                serde_json::json!({
                    "results": [{ "vulns": [{ "id": "GHSA-35jh-r3h4-6jhm" }] }]
                })
                .to_string(),
            ),
        );
        http.stub(
            "https://api.osv.dev/v1/vulns/GHSA-35jh-r3h4-6jhm",
            200,
            Bytes::from(osv_high_lodash_vuln().to_string()),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(assay.counts().high, 1);
        assert_eq!(assay.findings()[0].title(), "Command Injection in lodash");
        assert_eq!(assay.findings()[0].fixed_version(), Some("4.17.21"));
    }

    #[tokio::test]
    async fn local_feed_answers_without_calling_osv() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        let feed = super::OsvFeed::from_bytes(
            br#"{
                "format": "ferrobox-osv-index",
                "format_version": 1,
                "dataset": "2026-10-02",
                "advisories": [{
                    "ecosystem": "npm",
                    "name": "lodash",
                    "id": "GHSA-35jh-r3h4-6jhm",
                    "aliases": ["CVE-2021-23337"],
                    "summary": "Command Injection in lodash",
                    "severity": "HIGH",
                    "fixed": "4.17.21"
                }]
            }"#,
        )
        .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        )
        .with_feed(Arc::new(feed));
        let assay = service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Ready);
        assert_eq!(assay.counts().high, 1);
        assert_eq!(assay.findings()[0].fixed_version(), Some("4.17.21"));
    }

    struct MemoryOsvFeedStore {
        record: std::sync::Mutex<Option<OsvFeedRecord>>,
    }

    impl MemoryOsvFeedStore {
        fn new() -> Self {
            Self {
                record: std::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl OsvFeedStore for MemoryOsvFeedStore {
        async fn current(&self) -> Result<Option<OsvFeedRecord>, OsvFeedStoreError> {
            Ok(self.record.lock().unwrap().clone())
        }

        async fn save(&self, record: &OsvFeedRecord) -> Result<(), OsvFeedStoreError> {
            *self.record.lock().unwrap() = Some(record.clone());
            Ok(())
        }
    }

    fn lodash_feed_bytes() -> Bytes {
        Bytes::from_static(
            br#"{
                "format": "ferrobox-osv-index",
                "format_version": 1,
                "dataset": "2026-10-02",
                "advisories": [{
                    "ecosystem": "npm",
                    "name": "lodash",
                    "id": "GHSA-35jh-r3h4-6jhm",
                    "aliases": ["CVE-2021-23337"],
                    "summary": "Command Injection in lodash",
                    "severity": "HIGH",
                    "fixed": "4.17.21"
                }]
            }"#,
        )
    }

    fn sha256_hex(bytes: &Bytes) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    async fn assay_with_feed_store(
        storage: Arc<InMemoryStorage>,
        store: Arc<dyn OsvFeedStore>,
    ) -> (AssayService, Repository) {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            storage,
            Arc::new(InMemoryHttpClient::default()),
        )
        .with_feed_store(store);
        (service, repository)
    }

    #[tokio::test]
    async fn import_with_matching_sha256_answers_without_calling_osv() {
        let storage = Arc::new(InMemoryStorage::default());
        let store = Arc::new(MemoryOsvFeedStore::new());
        let (service, repository) = assay_with_feed_store(storage, store).await;
        let bytes = lodash_feed_bytes();
        let digest = sha256_hex(&bytes);
        let record = service
            .import_feed(bytes, &digest.to_ascii_uppercase())
            .await
            .unwrap();
        assert_eq!(record.dataset, "2026-10-02");
        assert_eq!(record.sha256, digest);
        assert_eq!(record.advisory_count, 1);
        assert_eq!(record.ecosystems, vec!["npm".to_string()]);
        assert_eq!(record.source, "file");

        let assay = service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Ready);
        assert_eq!(assay.counts().high, 1);
        assert_eq!(assay.findings()[0].fixed_version(), Some("4.17.21"));
    }

    #[tokio::test]
    async fn rejected_import_keeps_the_previous_index() {
        let storage = Arc::new(InMemoryStorage::default());
        let store = Arc::new(MemoryOsvFeedStore::new());
        let (service, repository) = assay_with_feed_store(storage.clone(), store).await;
        let bytes = lodash_feed_bytes();
        let digest = sha256_hex(&bytes);
        service.import_feed(bytes, &digest).await.unwrap();

        let other = Bytes::from_static(b"different-bytes");
        let mismatch = service.import_feed(other, &digest).await.unwrap_err();
        assert!(matches!(
            mismatch,
            FeedImportError::ChecksumMismatch { .. }
        ));

        let broken = Bytes::from_static(b"not-an-index");
        let broken_digest = sha256_hex(&broken);
        let invalid = service
            .import_feed(broken, &broken_digest)
            .await
            .unwrap_err();
        assert!(matches!(invalid, FeedImportError::Invalid(_)));
        assert!(
            !storage
                .exists(&StorageKey::new(format!("system/osv-feed/{broken_digest}")))
                .await
                .unwrap()
        );

        let current = service.imported_feed().await.unwrap().unwrap();
        assert_eq!(current.dataset, "2026-10-02");
        assert_eq!(current.sha256, digest);
        let assay = service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Ready);
        assert_eq!(assay.counts().high, 1);
    }

    #[tokio::test]
    async fn restored_feed_survives_a_new_process() {
        let storage = Arc::new(InMemoryStorage::default());
        let store = Arc::new(MemoryOsvFeedStore::new());
        let (service, _) = assay_with_feed_store(Arc::clone(&storage), store.clone()).await;
        let bytes = lodash_feed_bytes();
        let digest = sha256_hex(&bytes);
        service.import_feed(bytes, &digest).await.unwrap();

        let (restarted, repository) = assay_with_feed_store(storage, store).await;
        let before = restarted
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(before.status(), AssayStatus::Failed);

        restarted.restore_persisted_feed().await.unwrap();
        let after = restarted
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(after.status(), AssayStatus::Ready);
        assert_eq!(after.counts().high, 1);
        assert_eq!(after.findings()[0].fixed_version(), Some("4.17.21"));
    }

    fn npm_index(dataset: &str, fixed: &str) -> Bytes {
        Bytes::from(
            serde_json::json!({
                "format": "ferrobox-osv-index",
                "format_version": 1,
                "dataset": dataset,
                "advisories": [{
                    "ecosystem": "npm",
                    "name": "lodash",
                    "id": "GHSA-35jh-r3h4-6jhm",
                    "aliases": ["CVE-2021-23337"],
                    "summary": "Command Injection in lodash",
                    "severity": "HIGH",
                    "fixed": fixed
                }]
            })
            .to_string(),
        )
    }

    fn stored_assay<'a>(assays: &'a [Assay], name: &str) -> &'a Assay {
        assays
            .iter()
            .find(|assay| assay.coordinate().name().as_str() == name)
            .unwrap()
    }

    #[tokio::test]
    async fn importing_a_new_feed_reruns_stored_assays() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_npm(&index, &repository, "lodash", "4.17.21").await;
        seed_npm(&index, &repository, "ms", "2.1.3").await;
        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        )
        .with_feed_store(Arc::new(MemoryOsvFeedStore::new()));

        let clean = npm_index("2026-10-02", "4.17.21");
        service
            .import_feed(clean.clone(), &sha256_hex(&clean))
            .await
            .unwrap();
        service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.21")
            .await
            .unwrap();
        service
            .run(repository.id(), PackageEcosystem::Npm, "ms", "2.1.3")
            .await
            .unwrap();
        let before = service.list_all().await.unwrap();
        let lodash_scanned = stored_assay(&before, "lodash")
            .scanned_at()
            .unwrap()
            .to_string();
        let ms_scanned = stored_assay(&before, "ms").scanned_at().unwrap().to_string();
        assert_eq!(stored_assay(&before, "lodash").counts().high, 0);
        assert_eq!(stored_assay(&before, "ms").counts().high, 0);

        let rejected = service
            .import_feed(Bytes::from_static(b"nope"), &sha256_hex(&clean))
            .await
            .unwrap_err();
        assert!(matches!(
            rejected,
            FeedImportError::ChecksumMismatch { .. }
        ));
        service
            .import_feed(clean.clone(), &sha256_hex(&clean))
            .await
            .unwrap();
        let unchanged = service.list_all().await.unwrap();
        assert_eq!(
            stored_assay(&unchanged, "lodash").scanned_at(),
            Some(lodash_scanned.as_str())
        );
        assert_eq!(
            stored_assay(&unchanged, "ms").scanned_at(),
            Some(ms_scanned.as_str())
        );
        assert_eq!(stored_assay(&unchanged, "lodash").status(), AssayStatus::Ready);

        let next = npm_index("2026-10-03", "4.17.22");
        service
            .import_feed(next.clone(), &sha256_hex(&next))
            .await
            .unwrap();
        let finished = wait_until_not_running(&service).await;
        let lodash = stored_assay(&finished, "lodash");
        let ms = stored_assay(&finished, "ms");
        assert_eq!(lodash.status(), AssayStatus::Ready);
        assert_eq!(lodash.counts().high, 1);
        assert_eq!(lodash.findings()[0].fixed_version(), Some("4.17.22"));
        assert_eq!(ms.status(), AssayStatus::Ready);
        assert_eq!(ms.counts().high, 0);
        assert!(ms.findings().is_empty());
    }

    #[tokio::test]
    async fn oci_tag_entry_is_unsupported_without_layer_inventory() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse("latest").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(
                    r#"{"name":"alpine","reference":"latest","digest":"sha256:abc","media_type":"application/vnd.oci.image.manifest.v1+json","size":527,"artifact_id":"00000000-0000-0000-0000-000000000001"}"#,
                ),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Oci, "alpine", "latest")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Unsupported);
        assert!(assay.findings().is_empty());
        assert!(assay.error_message().is_some());
    }

    #[tokio::test]
    async fn conan_recipe_is_unsupported_without_lock_inventory() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("conan-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Conan,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Conan,
            PackageName::parse("hello").unwrap(),
            PackageVersion::parse("0.1@_:_").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(
                    r#"{"name":"hello","version":"0.1","user":"_","channel":"_","yanked":false,"files":[],"revisions":[]}"#,
                ),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Conan, "hello", "0.1@_:_")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Unsupported);
    }

    #[tokio::test]
    async fn oci_is_unsupported_without_layer_inventory() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("library/alpine").unwrap(),
            PackageVersion::parse("3.19").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(r#"{"name":"library/alpine","version":"3.19"}"#),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(
                repository.id(),
                PackageEcosystem::Oci,
                "library/alpine",
                "3.19",
            )
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Unsupported);
        assert!(assay.findings().is_empty());
    }

    fn gzip_tar_file(path: &str, content: &str) -> Bytes {
        use std::io::Write;
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let data = content.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, data).unwrap();
            builder.finish().unwrap();
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_buf).unwrap();
        Bytes::from(encoder.finish().unwrap())
    }

    fn content_digest(body: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(body))
    }

    #[tokio::test]
    async fn oci_alpine_layer_queries_osv_for_apk_packages() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();

        let layer = gzip_tar_file("lib/apk/db/installed", "P:busybox\nV:1.36.1-r19\n\n");
        let layer_digest = content_digest(&layer);
        let layer_id = ArtifactId::new();
        storage
            .put(&storage_key_for(layer_id), layer.clone())
            .await
            .unwrap();
        let blob_coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("_blob").unwrap(),
            PackageVersion::parse(layer_digest.clone()).unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &blob_coordinate,
                Some(layer_id),
                Bytes::from(format!(
                    r#"{{"digest":"{layer_digest}","artifact_id":"{layer_id}","size":{}}}"#,
                    layer.len()
                )),
            )
            .await
            .unwrap();

        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "layers": [{
                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                "digest": layer_digest,
                "size": layer.len()
            }]
        });
        let manifest_bytes = Bytes::from(manifest.to_string());
        let manifest_id = ArtifactId::new();
        storage
            .put(&storage_key_for(manifest_id), manifest_bytes)
            .await
            .unwrap();
        let tag_coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse("latest").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &tag_coordinate,
                Some(manifest_id),
                Bytes::from(format!(
                    r#"{{"name":"alpine","reference":"latest","digest":"sha256:dead","media_type":"application/vnd.oci.image.manifest.v1+json","size":1,"artifact_id":"{manifest_id}"}}"#
                )),
            )
            .await
            .unwrap();

        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            Bytes::from(
                serde_json::json!({
                    "results": [{
                        "vulns": [{
                            "id": "CVE-2022-28391",
                            "summary": "busybox issue",
                            "database_specific": { "severity": "MEDIUM" },
                            "affected": [{ "ranges": [{ "events": [{ "fixed": "1.36.2-r0" }] }] }]
                        }]
                    }]
                })
                .to_string(),
            ),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            storage,
            http,
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Oci, "alpine", "latest")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Ready);
        assert!(assay.components().iter().any(|c| c.name() == "busybox"));
        assert_eq!(assay.counts().medium, 1);
        assert_eq!(assay.findings()[0].component_name(), "busybox");
    }

    #[tokio::test]
    async fn conan_recipe_lists_requires_without_osv() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let repository = Repository::new(
            RepositoryName::parse("conan-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Conan,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let file_id = ArtifactId::new();
        storage
            .put(
                &storage_key_for(file_id),
                Bytes::from("[requires]\nzlib/1.2.13\n"),
            )
            .await
            .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Conan,
            PackageName::parse("hello").unwrap(),
            PackageVersion::parse("0.1@_:_").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(format!(
                    r#"{{"name":"hello","version":"0.1","user":"_","channel":"_","yanked":false,"files":[{{"filename":"conanfile.txt","artifact_id":"{file_id}"}}],"revisions":[]}}"#
                )),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            storage,
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Conan, "hello", "0.1@_:_")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Ready);
        assert!(assay.components().iter().any(|c| c.name() == "zlib"));
        assert!(assay.findings().is_empty());
        assert!(assay.error_message().unwrap().contains("Conan"));
    }

    #[tokio::test]
    async fn missing_package_is_not_found() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let err = service
            .run(repository.id(), PackageEcosystem::Npm, "missing", "1.0.0")
            .await
            .unwrap_err();
        assert!(matches!(err, AssayError::PackageNotFound(_)));
    }

    #[test]
    fn auto_assay_skips_oci_blobs_and_digest_references() {
        let blob = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("_blob").unwrap(),
            PackageVersion::parse(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
        );
        assert!(!should_auto_assay(&blob));
        let digest_tag = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
        );
        assert!(!should_auto_assay(&digest_tag));
        let tag = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse("latest").unwrap(),
        );
        assert!(should_auto_assay(&tag));
        let npm = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("lodash").unwrap(),
            PackageVersion::parse("4.17.20").unwrap(),
        );
        assert!(should_auto_assay(&npm));
    }

    async fn wait_for_assays(service: &AssayService, expected: usize) -> Vec<Assay> {
        for _ in 0..50 {
            let assays = service.list_all().await.unwrap();
            if assays.len() >= expected {
                return assays;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        service.list_all().await.unwrap()
    }

    async fn wait_until_not_running(service: &AssayService) -> Vec<Assay> {
        for _ in 0..50 {
            let assays = service.list_all().await.unwrap();
            if assays
                .iter()
                .all(|assay| assay.status() != AssayStatus::Running)
            {
                return assays;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        service.list_all().await.unwrap()
    }

    #[tokio::test]
    async fn schedule_runs_in_the_background_without_failing_the_caller() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub("https://api.osv.dev/v1/querybatch", 200, osv_high_lodash());

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        service.schedule(
            repository.id(),
            PackageCoordinate::new(
                PackageEcosystem::Npm,
                PackageName::parse("lodash").unwrap(),
                PackageVersion::parse("4.17.20").unwrap(),
            ),
        );
        let assays = wait_for_assays(&service, 1).await;
        assert_eq!(assays.len(), 1);
        assert_eq!(assays[0].counts().high, 1);
    }

    #[tokio::test]
    async fn rerun_all_schedules_each_distinct_coordinate() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub("https://api.osv.dev/v1/querybatch", 200, osv_high_lodash());

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        let scheduled = service.rerun_all().await.unwrap();
        assert_eq!(scheduled, 1);
        let assays = wait_until_not_running(&service).await;
        assert_eq!(assays.len(), 1);
        assert_ne!(assays[0].status(), AssayStatus::Running);
    }
}
