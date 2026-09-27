//! The Strategy pattern that lets `FerroBox` support several
//! package ecosystems (Cargo, npm, `PyPI`...) without the rest of the
//! application layer needing to know the details of any of them.
//!
//! Each ecosystem has its own incompatible rules for
//! three operations: how the publish request is interpreted
//! (`cargo publish` does not send the same format as `npm publish`), how
//! the index-protocol response is built that the
//! native package manager queries to resolve dependencies, and how
//! the matching binary artifact is located for download.
//! [`PackagingStrategy`] captures those three operations as a single
//! contract; [`PackagingRegistry`] selects, at runtime, which
//! concrete implementation to use based on the [`PackageEcosystem`] of
//! the repository being operated on.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use ferrobox_domain::repository::Repository;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::http_client::HttpClientError;
use ferrobox_ports::package_index_store::PackageIndexStoreError;
use ferrobox_ports::repository_store::RepositoryStoreError;
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::assay::AssayService;

/// The Cargo implementation (sparse index protocol) of the
/// Strategy pattern.
pub mod cargo;

/// The npm implementation (registry compatible with `npm publish` /
/// `npm install`) of the Strategy pattern.
pub mod npm;

/// The `PyPI` implementation (`twine upload` / `pip install`, PEP 503
/// simple index) of the Strategy pattern.
pub mod pypi;

/// Detection of Cosign / Sigstore signatures and other OCI accessories.
pub mod cosign;

/// The OCI implementation (Distribution Spec v2: `docker push` /
/// `docker pull`) of the Strategy pattern.
pub mod oci;

/// The Conan implementation (v2 API with revisions: `conan upload` /
/// `conan install`) of the Strategy pattern.
pub mod conan;

/// The Maven implementation (classic HTTP layout: `mvn deploy` /
/// `mvn dependency:get`) of the Strategy pattern.
pub mod maven;

/// The `NuGet` implementation (V3 API: `dotnet nuget push` /
/// `dotnet restore`) of the Strategy pattern.
pub mod nuget;

/// The Go implementation (`GOPROXY` protocol: `go get` /
/// `go mod download`) of the Strategy pattern.
pub mod golang;

/// Reasons a packaging operation can fail.
#[derive(Debug, Error)]
pub enum PackagingError {
    /// A strategy was invoked with a repository from an ecosystem
    /// other than the one the strategy implements.
    #[error(
        "repository is configured for ecosystem '{actual}', but this strategy handles '{expected}'"
    )]
    EcosystemMismatch {
        /// Ecosystem this strategy knows how to handle.
        expected: &'static str,
        /// Actual ecosystem of the repository that was received.
        actual: &'static str,
    },

    /// The publish request does not have a valid format for this
    /// ecosystem.
    #[error("invalid publish payload: {0}")]
    InvalidPayload(String),

    /// A published version already exists at that same coordinate --
    /// package registries are immutable once published (yank
    /// aside, which does not overwrite the content, only marks it).
    #[error("{0} was already published and cannot be overwritten")]
    AlreadyPublished(PackageCoordinate),

    /// No published version of this package exists in the
    /// repository.
    #[error("no published versions of package '{0}' were found in this repository")]
    PackageNotFound(String),

    /// The requested coordinate does not correspond to any published
    /// version.
    #[error("{0} was not found")]
    VersionNotFound(PackageCoordinate),

    /// No file with that name exists in the repository index
    /// (e.g. a specific `PyPI` wheel or sdist).
    #[error("file '{0}' was not found in this repository")]
    FileNotFound(String),

    /// Failure uploading or downloading the package binary content.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// Failure persisting the binary artifact metadata.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),

    /// Failure reading or writing the package index.
    #[error(transparent)]
    IndexPersistence(#[from] PackageIndexStoreError),

    /// The stored binary content does not match the SHA-256 checksum
    /// persisted at publish time.
    #[error("stored artifact checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Checksum persisted at publish time.
        expected: String,
        /// Checksum recomputed over the stored object.
        actual: String,
    },

    /// The repository is read-only (a `Mirror` or an `Alloy`) and
    /// does not accept publishes.
    #[error("repository is read-only and does not accept publishes")]
    ReadOnlyRepository,

    /// Failure querying the repository store (e.g. when
    /// resolving the members of an `Alloy`).
    #[error(transparent)]
    RepositoryPersistence(#[from] RepositoryStoreError),

    /// Failure querying the *upstream* of a `Mirror` repository.
    #[error(transparent)]
    Upstream(#[from] HttpClientError),

    /// The *upstream* response does not have the expected format.
    #[error("invalid upstream response: {0}")]
    InvalidUpstream(String),

    /// The binary does not fit in the repository storage quota.
    #[error(transparent)]
    Quota(#[from] crate::quota::QuotaError),

    /// An admission policy blocks this operation.
    #[error("{0}")]
    PolicyDenied(String),
}

/// The result of publishing a package: its newly assigned coordinate.
pub type PublishOutcome = PackageCoordinate;

/// Tally of a Forge→Forge promotion: the copied coordinate and the
/// volume of new binaries at the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteOutcome {
    /// Coordinate published in the destination repository.
    pub coordinate: PackageCoordinate,
    /// New binaries created at the destination (not counting OCI blobs already present).
    pub artifacts_copied: u32,
    /// Bytes written to the destination storage.
    pub bytes_copied: u64,
}

/// A `cargo search` match against a repository index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSearchHit {
    /// Package name.
    pub name: String,
    /// Latest non-yanked version (or the latest published one, if all are yanked).
    pub max_version: String,
}

/// An OCI manifest read from the index: media type, digest, and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciManifestDocument {
    /// OCI or Docker media type of the manifest.
    pub media_type: String,
    /// `sha256:…` digest of the body.
    pub digest: String,
    /// Raw manifest body.
    pub body: Bytes,
}

/// Packaging strategy for a specific ecosystem.
///
/// Each implementation encapsulates the three operations that its
/// ecosystem's native package manager expects from a registry: publish a
/// new package, respond to the index protocol used to
/// resolve dependencies, and serve the binary content of an
/// already published version.
#[async_trait]
pub trait PackagingStrategy: Send + Sync {
    /// The ecosystem this strategy knows how to handle.
    fn ecosystem(&self) -> PackageEcosystem;

    /// Publishes a new package in `repository` from the raw
    /// request sent by the ecosystem's native client (for example,
    /// the body of a `cargo publish` request).
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::EcosystemMismatch`] if `repository` does not
    /// belong to this ecosystem, [`PackagingError::InvalidPayload`]
    /// if `payload` does not have the expected format,
    /// [`PackagingError::AlreadyPublished`] if the resulting coordinate
    /// already existed, or any other error if the corresponding
    /// port fails.
    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError>;

    /// Builds this ecosystem's index-protocol response
    /// for package `name` inside `repository`.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::EcosystemMismatch`] if `repository` does not
    /// belong to this ecosystem, or any other error if the corresponding
    /// port fails.
    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError>;

    /// Downloads the binary content of an already published version.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::EcosystemMismatch`] if `repository` does not
    /// belong to this ecosystem, [`PackagingError::PackageNotFound`]
    /// if `coordinate` does not correspond to any published version, or
    /// any other error if the corresponding port fails.
    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError>;

    /// Downloads a file from the repository by filename
    /// (e.g. a `PyPI` wheel or sdist). The default implementation
    /// indicates that the ecosystem does not resolve artifacts by name.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::FileNotFound`] if the ecosystem does not
    /// support this operation or the file does not exist.
    async fn download_file(
        &self,
        _repository: &Repository,
        filename: &str,
    ) -> Result<Bytes, PackagingError> {
        Err(PackagingError::FileNotFound(filename.to_string()))
    }

    /// Uploads a file identified by the native protocol's relative
    /// path (e.g. a Conan recipe or package).
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::InvalidPayload`] if the ecosystem does not
    /// accept this kind of upload.
    async fn put_protocol_file(
        &self,
        _repository: &Repository,
        _path: &str,
        _body: Bytes,
    ) -> Result<(), PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not accept protocol file uploads".to_string(),
        ))
    }

    /// Downloads a file identified by the native protocol's relative
    /// path.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::FileNotFound`] if it does not exist.
    async fn get_protocol_file(
        &self,
        _repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Err(PackagingError::FileNotFound(path.to_string()))
    }

    /// Native-protocol JSON metadata for `path` (latest,
    /// revisions, file listing, binary search).
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::PackageNotFound`] if there is no data.
    async fn protocol_metadata(
        &self,
        _repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Err(PackagingError::PackageNotFound(path.to_string()))
    }

    /// Stores an OCI blob identified by its `sha256:…` digest.
    /// The default implementation indicates that the ecosystem does not use blobs.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::InvalidPayload`] if the ecosystem does not
    /// support blobs, or another error if the corresponding port fails.
    async fn put_blob(
        &self,
        _repository: &Repository,
        _digest: &str,
        _body: Bytes,
    ) -> Result<u64, PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not store OCI blobs".to_string(),
        ))
    }

    /// Publishes an OCI manifest (by tag or by digest).
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::InvalidPayload`] if the ecosystem does not
    /// support manifests, or another error if the corresponding port fails.
    async fn put_manifest(
        &self,
        _repository: &Repository,
        _name: &str,
        _reference: &str,
        _media_type: &str,
        _body: Bytes,
    ) -> Result<String, PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not store OCI manifests".to_string(),
        ))
    }

    /// Reads an OCI manifest by tag or digest.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::PackageNotFound`] or
    /// [`PackagingError::VersionNotFound`] if it does not exist.
    async fn get_manifest(
        &self,
        _repository: &Repository,
        _name: &str,
        _reference: &str,
    ) -> Result<OciManifestDocument, PackagingError> {
        Err(PackagingError::PackageNotFound(_name.to_string()))
    }

    /// Downloads an OCI blob by digest. `name` is the image name
    /// (a `Mirror` needs it to build the *upstream* URL).
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::FileNotFound`] if the blob does not exist.
    async fn get_blob(
        &self,
        repository: &Repository,
        _name: &str,
        digest: &str,
    ) -> Result<Bytes, PackagingError> {
        self.download_file(repository, digest).await
    }

    /// Lists the tags of an OCI image.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::PackageNotFound`] if the image does not exist.
    async fn list_tags(
        &self,
        _repository: &Repository,
        _name: &str,
    ) -> Result<Vec<String>, PackagingError> {
        Err(PackagingError::PackageNotFound(_name.to_string()))
    }

    /// Lists the manifests that point to `digest` as `subject`
    /// (Distribution Spec Referrers API).
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::PackageNotFound`] if the ecosystem does not
    /// implement referrers or the image does not exist.
    async fn list_referrers(
        &self,
        _repository: &Repository,
        _name: &str,
        _digest: &str,
        _artifact_type: Option<&str>,
    ) -> Result<OciManifestDocument, PackagingError> {
        Err(PackagingError::PackageNotFound(_name.to_string()))
    }

    /// Marks (or unmarks) an already published version as *yanked*. Does not
    /// delete the binary: `cargo` can still download it if it is
    /// pinned in a `Cargo.lock`, but stops considering it for
    /// new resolutions.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::EcosystemMismatch`] if `repository` does not
    /// belong to this ecosystem, [`PackagingError::ReadOnlyRepository`]
    /// if it is a `Mirror` or an `Alloy`, [`PackagingError::VersionNotFound`]
    /// if that coordinate does not exist, or any other error if the
    /// corresponding port fails.
    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError>;

    /// Copies an already published version from `source` to `target` (both
    /// `Forge` of the same ecosystem). Binaries are rewritten with
    /// new identifiers; by default the copy does not inherit yank.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::InvalidPayload`] if the ecosystem does not
    /// support this operation, [`PackagingError::VersionNotFound`] if
    /// the coordinate does not exist at the source,
    /// [`PackagingError::AlreadyPublished`] if it is already at the destination, or
    /// another error if a port fails.
    async fn promote_version(
        &self,
        _source: &Repository,
        _target: &Repository,
        _coordinate: &PackageCoordinate,
        _preserve_yanked: bool,
    ) -> Result<PromoteOutcome, PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not support promoting versions".to_string(),
        ))
    }

    /// Searches for packages whose name contains `query` (case
    /// insensitive), up to `limit` matches.
    ///
    /// # Errors
    ///
    /// Returns [`PackagingError::EcosystemMismatch`] if `repository` does not
    /// belong to this ecosystem, or any other error if the corresponding
    /// port fails.
    async fn search(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError>;
}

/// Fires a background assay if the strategy has
/// [`AssayService`]. Does not block publish or install.
pub(crate) fn notify_assay(
    assays: Option<&AssayService>,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
) {
    if let Some(assays) = assays {
        assays.schedule_after_publish(repository_id, coordinate.clone());
    }
}

pub(crate) async fn ensure_quota(
    quota: Option<&crate::quota::QuotaService>,
    repository_id: RepositoryId,
    additional_bytes: u64,
) -> Result<(), PackagingError> {
    if let Some(quota) = quota {
        quota
            .ensure_can_store(repository_id, additional_bytes)
            .await?;
    }
    Ok(())
}

/// Copies the stored object from `source_artifact_id` to a new
/// artifact in `target_repository_id`. Applies the destination quota.
pub(crate) async fn copy_stored_artifact(
    artifact_store: &dyn ArtifactStore,
    storage: &dyn StoragePort,
    quota: Option<&crate::quota::QuotaService>,
    source_artifact_id: ArtifactId,
    target_repository_id: RepositoryId,
) -> Result<(ArtifactId, u64), PackagingError> {
    let source = artifact_store
        .find_by_id(source_artifact_id)
        .await?
        .ok_or_else(|| PackagingError::FileNotFound(source_artifact_id.to_string()))?;
    let storage_key = crate::storage_key::storage_key_for(source_artifact_id);
    let content = storage.get(&storage_key).await?;
    let actual = crate::content_hash::sha256_checksum(&content);
    if actual != *source.checksum() {
        return Err(PackagingError::ChecksumMismatch {
            expected: source.checksum().to_string(),
            actual: actual.to_string(),
        });
    }

    let size_bytes = source.size_bytes();
    ensure_quota(quota, target_repository_id, size_bytes).await?;
    let copied = Artifact::new(target_repository_id, source.checksum().clone(), size_bytes)
        .with_filename(source.filename().map(ToOwned::to_owned));
    storage
        .put(&crate::storage_key::storage_key_for(copied.id()), content)
        .await?;
    artifact_store.save(&copied).await?;
    Ok((copied.id(), size_bytes))
}

/// Selects, at runtime, the appropriate [`PackagingStrategy`]
/// for a repository's [`PackageEcosystem`].
///
/// This is the "context" of the Strategy pattern: the rest of the application
/// (HTTP routes, future use cases) depends only on this
/// registry, not on any concrete strategy -- adding a new
/// ecosystem means implementing `PackagingStrategy` and calling
/// [`PackagingRegistry::register`], without touching any other point in the
/// system.
#[derive(Clone, Default)]
pub struct PackagingRegistry {
    strategies: HashMap<PackageEcosystem, Arc<dyn PackagingStrategy>>,
}

impl PackagingRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a strategy for the ecosystem it declares it
    /// handles, replacing any strategy previously
    /// registered for that same ecosystem.
    #[must_use]
    pub fn register(mut self, strategy: Arc<dyn PackagingStrategy>) -> Self {
        self.strategies.insert(strategy.ecosystem(), strategy);
        self
    }

    /// Looks up the strategy registered for the given ecosystem.
    /// Returns `None` if no strategy was registered for that
    /// ecosystem -- for example, because its implementation does not
    /// exist yet.
    #[must_use]
    pub fn strategy_for(&self, ecosystem: PackageEcosystem) -> Option<Arc<dyn PackagingStrategy>> {
        self.strategies.get(&ecosystem).cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::PackageEcosystem;

    use super::cargo::CargoPackagingStrategy;
    use super::conan::ConanPackagingStrategy;
    use super::golang::GoPackagingStrategy;
    use super::maven::MavenPackagingStrategy;
    use super::npm::NpmPackagingStrategy;
    use super::nuget::NugetPackagingStrategy;
    use super::oci::OciPackagingStrategy;
    use super::pypi::PypiPackagingStrategy;
    use super::*;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn cargo_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(CargoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        ))
    }

    fn npm_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        ))
    }

    fn pypi_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(PypiPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        ))
    }

    fn oci_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(OciPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryHttpClient::default()),
        ))
    }

    fn helm_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(OciPackagingStrategy::for_ecosystem(
            PackageEcosystem::Helm,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryHttpClient::default()),
        ))
    }

    fn conan_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(ConanPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        ))
    }

    fn maven_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(MavenPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        ))
    }

    fn nuget_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(NugetPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000",
        ))
    }

    fn go_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(GoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        ))
    }

    #[test]
    fn registers_and_finds_a_strategy_by_ecosystem() {
        let registry = PackagingRegistry::new()
            .register(cargo_strategy())
            .register(npm_strategy())
            .register(pypi_strategy())
            .register(oci_strategy())
            .register(helm_strategy())
            .register(conan_strategy())
            .register(maven_strategy())
            .register(nuget_strategy())
            .register(go_strategy());

        assert!(registry.strategy_for(PackageEcosystem::Cargo).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Npm).is_some());
        assert!(registry.strategy_for(PackageEcosystem::PyPi).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Oci).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Helm).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Conan).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Maven).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Nuget).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Go).is_some());
    }

    #[test]
    fn returns_none_for_an_unregistered_ecosystem() {
        let registry = PackagingRegistry::new().register(cargo_strategy());

        assert!(registry.strategy_for(PackageEcosystem::Npm).is_none());
    }
}
