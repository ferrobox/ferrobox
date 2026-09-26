use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::worm::WormPolicy;
use thiserror::Error;

/// Reasons a WORM policy read or write can fail.
#[derive(Debug, Error)]
pub enum WormStoreError {
    /// The WORM table is missing: run the SQL migration.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_worm is missing)"
    )]
    MissingSchema,

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for a repository WORM lock.
#[async_trait]
pub trait WormStore: Send + Sync {
    /// Returns the policy, or disabled if it was never configured.
    ///
    /// # Errors
    ///
    /// [`WormStoreError::Backend`] if the underlying backend fails.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<WormPolicy, WormStoreError>;

    /// Inserts or replaces the policy.
    ///
    /// # Errors
    ///
    /// [`WormStoreError::Backend`] if the underlying backend fails.
    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: WormPolicy,
    ) -> Result<(), WormStoreError>;
}
