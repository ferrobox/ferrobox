//! Per-repository storage quota.

use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::quota::{StorageQuota, StorageQuotaError};
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::quota_store::{QuotaStore, QuotaStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Current usage and configured cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaSnapshot {
    /// Cap in bytes, or `None` if there is no limit.
    pub limit_bytes: Option<u64>,
    /// Sum of the repository binaries (including unindexed ones).
    pub used_bytes: u64,
}

/// Reasons querying or applying a quota can fail.
#[derive(Debug, Error)]
pub enum QuotaError {
    /// The repository does not exist.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// An `Alloy` does not store its own binaries.
    #[error("quota does not apply to Alloy repositories")]
    AlloyRepository,

    /// Missing SQL migration (`sqlx migrate run` from the backend directory).
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_quota is missing)"
    )]
    MissingSchema,

    /// The requested cap is not valid.
    #[error(transparent)]
    InvalidQuota(#[from] StorageQuotaError),

    /// Storing this binary would exceed the cap.
    #[error(
        "repository quota exceeded: using {used_bytes} of {limit_bytes} bytes, \
         need {additional_bytes} more"
    )]
    Exceeded {
        /// Bytes already occupied.
        used_bytes: u64,
        /// Configured cap.
        limit_bytes: u64,
        /// Size of the binary to add.
        additional_bytes: u64,
    },

    /// Failed to query the repository store.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),

    /// Failed to query artifacts.
    #[error(transparent)]
    Artifacts(#[from] ArtifactStoreError),

    /// Failed to query or persist the quota.
    #[error(transparent)]
    Policy(#[from] QuotaStoreError),
}

/// Use case: storage quota.
#[derive(Clone)]
#[allow(clippy::struct_field_names)]
pub struct QuotaService {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    quota_store: Arc<dyn QuotaStore>,
}

impl QuotaService {
    /// Builds the service from its ports.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        quota_store: Arc<dyn QuotaStore>,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            quota_store,
        }
    }

    /// Returns the cap and current usage.
    ///
    /// # Errors
    ///
    /// [`QuotaError::RepositoryNotFound`] or a port failure.
    pub async fn get_snapshot(
        &self,
        repository_id: RepositoryId,
    ) -> Result<QuotaSnapshot, QuotaError> {
        self.require_target(repository_id).await?;
        let quota = match self.quota_store.find_by_repository(repository_id).await {
            Ok(quota) => quota,
            Err(QuotaStoreError::MissingSchema) => StorageQuota::unlimited(),
            Err(err) => return Err(err.into()),
        };
        Ok(QuotaSnapshot {
            limit_bytes: quota.limit_bytes(),
            used_bytes: self.used_bytes(repository_id).await?,
        })
    }

    /// Saves the cap. Does not delete anything.
    ///
    /// # Errors
    ///
    /// [`QuotaError::RepositoryNotFound`], [`QuotaError::AlloyRepository`]
    /// or [`QuotaError::MissingSchema`].
    pub async fn save(
        &self,
        repository_id: RepositoryId,
        quota: StorageQuota,
    ) -> Result<QuotaSnapshot, QuotaError> {
        self.require_target(repository_id).await?;
        match self.quota_store.save(repository_id, quota).await {
            Ok(()) => {}
            Err(QuotaStoreError::MissingSchema) => return Err(QuotaError::MissingSchema),
            Err(err) => return Err(err.into()),
        }
        self.get_snapshot(repository_id).await
    }

    /// Bytes occupied by every binary on the instance.
    ///
    /// # Errors
    ///
    /// A port failure while summing artifacts.
    pub async fn instance_used_bytes(&self) -> Result<u64, QuotaError> {
        Ok(self.artifact_store.total_size_bytes().await?)
    }

    /// Rejects a `publish` or a cache that would not fit in the cap.
    ///
    /// # Errors
    ///
    /// [`QuotaError::Exceeded`] if `used + additional` exceeds the cap.
    pub async fn ensure_can_store(
        &self,
        repository_id: RepositoryId,
        additional_bytes: u64,
    ) -> Result<(), QuotaError> {
        if additional_bytes == 0 {
            return Ok(());
        }
        let snapshot = self.get_snapshot(repository_id).await?;
        let Some(limit_bytes) = snapshot.limit_bytes else {
            return Ok(());
        };
        if snapshot.used_bytes.saturating_add(additional_bytes) <= limit_bytes {
            return Ok(());
        }
        Err(QuotaError::Exceeded {
            used_bytes: snapshot.used_bytes,
            limit_bytes,
            additional_bytes,
        })
    }

    async fn used_bytes(&self, repository_id: RepositoryId) -> Result<u64, QuotaError> {
        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;
        Ok(artifacts
            .iter()
            .map(ferrobox_domain::artifact::Artifact::size_bytes)
            .sum())
    }

    async fn require_target(&self, repository_id: RepositoryId) -> Result<(), QuotaError> {
        let Some(repository) = self.repository_store.find_by_id(repository_id).await? else {
            return Err(QuotaError::RepositoryNotFound(repository_id));
        };
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(QuotaError::AlloyRepository);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::artifact::Artifact;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{
        InMemoryArtifactStore, InMemoryQuotaStore, InMemoryRepositoryStore,
    };

    use super::*;

    fn checksum() -> Sha256Checksum {
        Sha256Checksum::parse("a".repeat(64)).unwrap()
    }

    #[tokio::test]
    async fn unlimited_by_default_and_blocks_when_set() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let quotas = Arc::new(InMemoryQuotaStore::default());
        let repository = Repository::new(
            RepositoryName::parse("crates").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&repository).await.unwrap();
        let artifact = Artifact::new(repository.id(), checksum(), 80);
        artifacts.save(&artifact).await.unwrap();

        let service = QuotaService::new(repositories, artifacts, quotas);
        assert_eq!(service.instance_used_bytes().await.unwrap(), 80);
        service
            .ensure_can_store(repository.id(), 1_000)
            .await
            .unwrap();

        service
            .save(repository.id(), StorageQuota::new(Some(100)).unwrap())
            .await
            .unwrap();
        service
            .ensure_can_store(repository.id(), 20)
            .await
            .unwrap();
        let err = service
            .ensure_can_store(repository.id(), 21)
            .await
            .unwrap_err();
        assert!(matches!(err, QuotaError::Exceeded { used_bytes: 80, limit_bytes: 100, additional_bytes: 21 }));
    }

    #[tokio::test]
    async fn alloy_is_rejected() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let member = Repository::new(
            RepositoryName::parse("member").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![member.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&member).await.unwrap();
        repositories.save(&alloy).await.unwrap();
        let service = QuotaService::new(
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryQuotaStore::default()),
        );
        let err = service.get_snapshot(alloy.id()).await.unwrap_err();
        assert!(matches!(err, QuotaError::AlloyRepository));
    }
}
