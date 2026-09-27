use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::quota::StorageQuota;
use thiserror::Error;

/// Reasons a quota operation can fail.
#[derive(Debug, Error)]
pub enum QuotaStoreError {
    /// The quota table is missing: the SQL migration has not been run.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_quota is missing)"
    )]
    MissingSchema,

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for a repository storage quota.
#[async_trait]
pub trait QuotaStore: Send + Sync {
    /// Returns the repository quota, or unlimited if it was never configured.
    ///
    /// # Errors
    ///
    /// Returns [`QuotaStoreError::Backend`] if the underlying backend fails.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<StorageQuota, QuotaStoreError>;

    /// Inserts or replaces the repository quota.
    ///
    /// # Errors
    ///
    /// Returns [`QuotaStoreError::Backend`] if the underlying backend fails.
    async fn save(
        &self,
        repository_id: RepositoryId,
        quota: StorageQuota,
    ) -> Result<(), QuotaStoreError>;
}
