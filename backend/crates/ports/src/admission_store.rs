use async_trait::async_trait;
use ferrobox_domain::admission::{AdmissionEvent, AdmissionPolicy};
use ferrobox_domain::ids::RepositoryId;
use thiserror::Error;

/// Persisted policy together with the repository Cosign keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionRecord {
    /// Admission rule.
    pub policy: AdmissionPolicy,
    /// PEM of Cosign public keys. Empty: only the signature is detected.
    pub public_keys_pem: String,
}

/// Reasons an admission-policy operation can fail.
#[derive(Debug, Error)]
pub enum AdmissionStoreError {
    /// The policy table is missing: the SQL migration has not been run.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_admission is missing)"
    )]
    MissingSchema,

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for a repository admission policy.
#[async_trait]
pub trait AdmissionStore: Send + Sync {
    /// Returns the repository policy, or the inactive default.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStoreError::Backend`] if the backend fails.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<AdmissionRecord, AdmissionStoreError>;

    /// Inserts or replaces the repository policy.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStoreError::Backend`] if the backend fails.
    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
        public_keys_pem: &str,
    ) -> Result<(), AdmissionStoreError>;

    /// Records a warning or a denial and trims history to 50 rows per
    /// repository.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStoreError::Backend`] if the backend fails.
    async fn record_event(&self, event: &AdmissionEvent) -> Result<(), AdmissionStoreError>;

    /// Latest events for the repository, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStoreError::Backend`] if the backend fails.
    async fn list_events(
        &self,
        repository_id: RepositoryId,
        limit: usize,
    ) -> Result<Vec<AdmissionEvent>, AdmissionStoreError>;
}
