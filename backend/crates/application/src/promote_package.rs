//! Copies a published version (or a generic binary) from one `Forge`
//! repository to another `Forge` of the same ecosystem.

use std::sync::Arc;

use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::packaging::{
    copy_stored_artifact, PackagingError, PackagingRegistry, PromoteOutcome as PackagingPromote,
};
use crate::quota::{QuotaError, QuotaService};

/// Result of a promotion: count of binaries and bytes copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteOutcome {
    /// Package name, if the ecosystem indexes by coordinate.
    pub name: Option<String>,
    /// Copied version (or OCI tag).
    pub version: Option<String>,
    /// New binaries created in the destination.
    pub artifacts_copied: u32,
    /// Bytes written to the destination storage.
    pub bytes_copied: u64,
}

/// Reasons promoting a version can fail.
#[derive(Debug, Error)]
pub enum PromoteError {
    /// The source repository does not exist.
    #[error("source repository {0} does not exist")]
    SourceNotFound(RepositoryId),

    /// The destination repository does not exist.
    #[error("target repository {0} does not exist")]
    TargetNotFound(RepositoryId),

    /// Source and destination are the same repository.
    #[error("source and target repositories must be different")]
    SameRepository,

    /// The source is not a `Forge`.
    #[error("cannot promote from a {0} repository")]
    SourceNotForge(&'static str),

    /// The destination is not a `Forge`.
    #[error("cannot promote into a {0} repository")]
    TargetNotForge(&'static str),

    /// Source and destination do not share an ecosystem.
    #[error("source ecosystem '{source_ecosystem}' does not match target ecosystem '{target_ecosystem}'")]
    EcosystemMismatch {
        /// Source ecosystem.
        source_ecosystem: &'static str,
        /// Destination ecosystem.
        target_ecosystem: &'static str,
    },

    /// Name and version are missing for an indexed ecosystem.
    #[error("name and version are required to promote this package")]
    MissingCoordinate,

    /// The generic binary identifier is missing.
    #[error("artifact_id is required to promote a generic artifact")]
    MissingArtifact,

    /// The binary does not exist.
    #[error("artifact {0} was not found")]
    ArtifactNotFound(ArtifactId),

    /// The binary does not belong to the source repository.
    #[error("artifact {0} does not belong to the source repository")]
    ArtifactRepositoryMismatch(ArtifactId),

    /// Failed to query repositories.
    #[error(transparent)]
    Repository(#[from] RepositoryStoreError),

    /// Failed to query or persist artifacts.
    #[error(transparent)]
    Artifact(#[from] ArtifactStoreError),

    /// Failed to copy bytes.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// The destination cannot accept more binaries.
    #[error(transparent)]
    Quota(#[from] QuotaError),

    /// Packaging strategy failure (missing version, already
    /// published, invalid payload, …).
    #[error(transparent)]
    Packaging(#[from] PackagingError),
}

/// Use case: promote a version (or a generic binary) between Forges.
pub struct PromotePackageUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    storage: Arc<dyn StoragePort>,
    quota: QuotaService,
}

impl PromotePackageUseCase {
    /// Builds the use case from its ports.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        storage: Arc<dyn StoragePort>,
        quota: QuotaService,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            storage,
            quota,
        }
    }

    /// Copies the `name`/`version` version (or the `artifact_id` binary
    /// in a generic repository) from `source_id` to `target_id`.
    ///
    /// # Errors
    ///
    /// Returns [`PromoteError`] if the repositories are not two
    /// distinct Forges of the same ecosystem, the coordinate is
    /// missing, the version already exists in the destination, or a
    /// port fails.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        packaging: &PackagingRegistry,
        source_id: RepositoryId,
        target_id: RepositoryId,
        name: Option<&str>,
        version: Option<&str>,
        artifact_id: Option<ArtifactId>,
        preserve_yanked: bool,
    ) -> Result<PromoteOutcome, PromoteError> {
        if source_id == target_id {
            return Err(PromoteError::SameRepository);
        }

        let source = self
            .repository_store
            .find_by_id(source_id)
            .await?
            .ok_or(PromoteError::SourceNotFound(source_id))?;
        let target = self
            .repository_store
            .find_by_id(target_id)
            .await?
            .ok_or(PromoteError::TargetNotFound(target_id))?;

        ensure_forge(&source, true)?;
        ensure_forge(&target, false)?;

        if source.ecosystem() != target.ecosystem() {
            return Err(PromoteError::EcosystemMismatch {
                source_ecosystem: source.ecosystem().label(),
                target_ecosystem: target.ecosystem().label(),
            });
        }

        if source.ecosystem() == PackageEcosystem::Generic {
            let artifact_id = artifact_id.ok_or(PromoteError::MissingArtifact)?;
            return self.promote_generic(&source, &target, artifact_id).await;
        }

        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(PromoteError::MissingCoordinate)?;
        let version = version
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(PromoteError::MissingCoordinate)?;
        let package_name = PackageName::parse(name.to_string())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let package_version = PackageVersion::parse(version.to_string())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let coordinate =
            PackageCoordinate::new(source.ecosystem(), package_name, package_version);

        let strategy = packaging.strategy_for(source.ecosystem()).ok_or_else(|| {
            PackagingError::InvalidPayload(format!(
                "no packaging strategy registered for {}",
                source.ecosystem().label()
            ))
        })?;
        let copied = strategy
            .promote_version(&source, &target, &coordinate, preserve_yanked)
            .await?;
        Ok(from_packaging(&copied))
    }

    async fn promote_generic(
        &self,
        source: &Repository,
        target: &Repository,
        artifact_id: ArtifactId,
    ) -> Result<PromoteOutcome, PromoteError> {
        let artifact = self
            .artifact_store
            .find_by_id(artifact_id)
            .await?
            .ok_or(PromoteError::ArtifactNotFound(artifact_id))?;
        if artifact.repository_id() != source.id() {
            return Err(PromoteError::ArtifactRepositoryMismatch(artifact_id));
        }

        let (_copied_id, bytes_copied) = copy_stored_artifact(
            self.artifact_store.as_ref(),
            self.storage.as_ref(),
            Some(&self.quota),
            artifact_id,
            target.id(),
        )
        .await?;
        Ok(PromoteOutcome {
            name: None,
            version: None,
            artifacts_copied: 1,
            bytes_copied,
        })
    }
}

fn from_packaging(outcome: &PackagingPromote) -> PromoteOutcome {
    PromoteOutcome {
        name: Some(outcome.coordinate.name().as_str().to_string()),
        version: Some(outcome.coordinate.version().as_str().to_string()),
        artifacts_copied: outcome.artifacts_copied,
        bytes_copied: outcome.bytes_copied,
    }
}

fn ensure_forge(repository: &Repository, source: bool) -> Result<(), PromoteError> {
    match repository.kind() {
        RepositoryKind::Forge => Ok(()),
        RepositoryKind::Mirror { .. } => Err(if source {
            PromoteError::SourceNotForge("Mirror")
        } else {
            PromoteError::TargetNotForge("Mirror")
        }),
        RepositoryKind::Alloy { .. } => Err(if source {
            PromoteError::SourceNotForge("Alloy")
        } else {
            PromoteError::TargetNotForge("Alloy")
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::packaging::cargo::CargoPackagingStrategy;
    use crate::packaging::{PackagingRegistry, PackagingStrategy};
    use crate::publish_artifact::PublishArtifactUseCase;
    use crate::quota::QuotaService;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryQuotaStore, InMemoryRepositoryStore, InMemoryStorage,
    };

    fn quota(
        repositories: Arc<InMemoryRepositoryStore>,
        artifacts: Arc<InMemoryArtifactStore>,
    ) -> QuotaService {
        QuotaService::new(
            repositories,
            artifacts,
            Arc::new(InMemoryQuotaStore::default()),
        )
    }

    fn forge(name: &str, ecosystem: PackageEcosystem) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            ecosystem,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn copies_a_generic_artifact_between_forges() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("generic-dev", PackageEcosystem::Generic);
        let target = forge("generic-prod", PackageEcosystem::Generic);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let artifact_id = PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota.clone(),
        )
        .execute(source.id(), Bytes::from_static(b"blob-bytes"))
        .await
        .unwrap();

        let use_case = PromotePackageUseCase::new(
            repositories,
            artifacts.clone(),
            storage,
            quota,
        );
        let outcome = use_case
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                target.id(),
                None,
                None,
                Some(artifact_id),
                false,
            )
            .await
            .unwrap();

        assert_eq!(outcome.artifacts_copied, 1);
        assert_eq!(outcome.bytes_copied, 10);
        let copied = artifacts.find_by_repository_id(target.id()).await.unwrap();
        assert_eq!(copied.len(), 1);
        assert_ne!(copied[0].id(), artifact_id);
    }

    #[tokio::test]
    async fn rejects_the_same_repository() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("only", PackageEcosystem::Generic);
        repositories.save(&source).await.unwrap();

        let err = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                source.id(),
                None,
                None,
                None,
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PromoteError::SameRepository));
    }

    #[tokio::test]
    async fn rejects_a_mirror_target() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("crates-dev", PackageEcosystem::Cargo);
        let target = Repository::new(
            RepositoryName::parse("crates-mirror").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.crates.io/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let err = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                target.id(),
                Some("demo"),
                Some("1.0.0"),
                None,
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PromoteError::TargetNotForge("Mirror")));
    }

    #[tokio::test]
    async fn rejects_distinct_ecosystems() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("crates-dev", PackageEcosystem::Cargo);
        let target = forge("npm-prod", PackageEcosystem::Npm);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let err = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                target.id(),
                Some("demo"),
                Some("1.0.0"),
                None,
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PromoteError::EcosystemMismatch { .. }));
    }

    #[tokio::test]
    async fn promotes_a_cargo_crate_through_the_use_case() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("crates-dev", PackageEcosystem::Cargo);
        let target = forge("crates-prod", PackageEcosystem::Cargo);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let strategy = CargoPackagingStrategy::new(
            artifacts.clone(),
            index.clone(),
            storage.clone(),
            Arc::new(InMemoryHttpClient::default()),
            repositories.clone(),
        );
        let payload = encode_cargo_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"crate-bytes",
        );
        strategy.publish(&source, payload).await.unwrap();

        let packaging = PackagingRegistry::new().register(Arc::new(strategy));
        let outcome = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &packaging,
                source.id(),
                target.id(),
                Some("ferrobox-cli"),
                Some("0.1.0"),
                None,
                false,
            )
            .await
            .unwrap();

        assert_eq!(outcome.name.as_deref(), Some("ferrobox-cli"));
        assert_eq!(outcome.version.as_deref(), Some("0.1.0"));
        assert_eq!(outcome.artifacts_copied, 1);
        assert_eq!(outcome.bytes_copied, 11);
    }

    fn encode_cargo_payload(metadata_json: &str, crate_bytes: &[u8]) -> Bytes {
        let metadata_bytes = metadata_json.as_bytes();
        let mut payload = Vec::new();
        payload.extend_from_slice(&u32::try_from(metadata_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(metadata_bytes);
        payload.extend_from_slice(&u32::try_from(crate_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(crate_bytes);
        Bytes::from(payload)
    }
}
