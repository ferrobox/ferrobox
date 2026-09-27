use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::replica::ReplicaPolicy;
use thiserror::Error;

/// Reasons persisting the replica policy can fail.
#[derive(Debug, Error)]
pub enum ReplicaStoreError {
    /// The table is missing: the SQL migration has not been run.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_replica is missing)"
    )]
    MissingSchema,

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for a repository replica policy.
#[async_trait]
pub trait ReplicaStore: Send + Sync {
    /// Returns the policy, or no target if it was never configured.
    ///
    /// # Errors
    ///
    /// [`ReplicaStoreError::Backend`] if the underlying backend fails.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPolicy, ReplicaStoreError>;

    /// Inserts or replaces the policy. No target deletes the row.
    ///
    /// # Errors
    ///
    /// [`ReplicaStoreError::Backend`] if the underlying backend fails.
    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: &ReplicaPolicy,
    ) -> Result<(), ReplicaStoreError>;

    /// Every configured replica policy (used by the background cron).
    ///
    /// # Errors
    ///
    /// [`ReplicaStoreError::Backend`] if the backend fails.
    async fn list_all(&self) -> Result<Vec<(RepositoryId, ReplicaPolicy)>, ReplicaStoreError>;
}
