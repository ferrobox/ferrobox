//! Per-repository write-once / read-many lock.

use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_domain::worm::WormPolicy;
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::worm_store::{WormStore, WormStoreError};
use thiserror::Error;

/// Reasons consulting or applying a WORM lock can fail.
#[derive(Debug, Error)]
pub enum WormError {
    /// The repository does not exist.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// An `Alloy` does not store its own binaries.
    #[error("WORM does not apply to Alloy repositories")]
    AlloyRepository,

    /// Missing SQL migration (`sqlx migrate run` from the backend directory).
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_worm is missing)"
    )]
    MissingSchema,

    /// The repository is locked and this mutation is not allowed.
    #[error("repository {0} is WORM-enabled and does not allow this mutation")]
    Locked(RepositoryId),

    /// Failed to query the repository store.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),

    /// Failed to query or persist the policy.
    #[error(transparent)]
    Policy(#[from] WormStoreError),
}

/// Use case: repository WORM lock.
#[derive(Clone)]
pub struct WormService {
    worm_store: Arc<dyn WormStore>,
    repository_store: Arc<dyn RepositoryStore>,
}

impl WormService {
    /// Builds the service from its ports.
    #[must_use]
    pub fn new(worm_store: Arc<dyn WormStore>, repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self {
            worm_store,
            repository_store,
        }
    }

    /// Returns the saved policy, or disabled if none was stored.
    ///
    /// # Errors
    ///
    /// [`WormError::RepositoryNotFound`] or a port failure.
    pub async fn get(&self, repository_id: RepositoryId) -> Result<WormPolicy, WormError> {
        self.require_target(repository_id).await?;
        match self.worm_store.find_by_repository(repository_id).await {
            Ok(policy) => Ok(policy),
            Err(WormStoreError::MissingSchema) => Ok(WormPolicy::disabled()),
            Err(err) => Err(err.into()),
        }
    }

    /// Saves the lock. Turning it off is allowed in this first cut.
    ///
    /// # Errors
    ///
    /// [`WormError::RepositoryNotFound`], [`WormError::AlloyRepository`],
    /// or [`WormError::MissingSchema`].
    pub async fn save(
        &self,
        repository_id: RepositoryId,
        policy: WormPolicy,
    ) -> Result<WormPolicy, WormError> {
        self.require_target(repository_id).await?;
        match self.worm_store.save(repository_id, policy).await {
            Ok(()) => {}
            Err(WormStoreError::MissingSchema) => return Err(WormError::MissingSchema),
            Err(err) => return Err(err.into()),
        }
        self.get(repository_id).await
    }

    /// Rejects delete, yank, retention apply, and per-repository GC.
    ///
    /// # Errors
    ///
    /// [`WormError::Locked`] when the repository is WORM-enabled.
    pub async fn ensure_mutable(&self, repository_id: RepositoryId) -> Result<(), WormError> {
        if self.get(repository_id).await?.enabled() {
            return Err(WormError::Locked(repository_id));
        }
        Ok(())
    }

    async fn require_target(&self, repository_id: RepositoryId) -> Result<(), WormError> {
        let Some(repository) = self.repository_store.find_by_id(repository_id).await? else {
            return Err(WormError::RepositoryNotFound(repository_id));
        };
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(WormError::AlloyRepository);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{InMemoryRepositoryStore, InMemoryWormStore};

    use super::*;

    async fn forge() -> (WormService, RepositoryId) {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let repository = Repository::new(
            RepositoryName::parse("crates").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&repository).await.unwrap();
        let service = WormService::new(Arc::new(InMemoryWormStore::default()), repositories);
        (service, repository.id())
    }

    #[tokio::test]
    async fn disabled_by_default_and_blocks_when_enabled() {
        let (service, id) = forge().await;
        service.ensure_mutable(id).await.unwrap();
        service.save(id, WormPolicy::new(true)).await.unwrap();
        let err = service.ensure_mutable(id).await.unwrap_err();
        assert!(matches!(err, WormError::Locked(_)));
        service.save(id, WormPolicy::disabled()).await.unwrap();
        service.ensure_mutable(id).await.unwrap();
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
        let service = WormService::new(Arc::new(InMemoryWormStore::default()), repositories);
        let err = service.get(alloy.id()).await.unwrap_err();
        assert!(matches!(err, WormError::AlloyRepository));
    }
}
