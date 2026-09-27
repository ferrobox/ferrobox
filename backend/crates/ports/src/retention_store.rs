use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::retention::RetentionPolicy;
use thiserror::Error;

/// Reasons a retention-policy operation can fail.
#[derive(Debug, Error)]
pub enum RetentionStoreError {
    /// The policy table is missing: the SQL migration has not been run.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_retention is missing)"
    )]
    MissingSchema,

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for a repository retention policy.
#[async_trait]
pub trait RetentionStore: Send + Sync {
    /// Returns the repository policy, or "keep everything" if it was never
    /// configured.
    ///
    /// # Errors
    ///
    /// Returns [`RetentionStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<RetentionPolicy, RetentionStoreError>;

    /// Inserts or replaces the repository policy.
    ///
    /// # Errors
    ///
    /// Returns [`RetentionStoreError::Backend`] if the underlying backend
    /// fails.
    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<(), RetentionStoreError>;
}
