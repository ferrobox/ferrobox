//! Persistence port for the signed vulnerability-index pull.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// The last pull installed a new index.
pub const OSV_SYNC_IMPORTED: &str = "imported";

/// The last pull matched the index already installed.
pub const OSV_SYNC_UNCHANGED: &str = "unchanged";

/// The last pull was rejected and the active index was kept.
pub const OSV_SYNC_REJECTED: &str = "rejected";

/// Reference, `Cosign` public key, and the last pull attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsvSyncSettings {
    /// OCI reference, for example `ghcr.io/ferrobox/osv-db:2026-10-03`.
    pub reference: String,
    /// PEM public key. A private key is not stored.
    pub public_key_pem: String,
    /// [`OSV_SYNC_IMPORTED`], [`OSV_SYNC_UNCHANGED`], [`OSV_SYNC_REJECTED`], or none yet.
    pub last_outcome: Option<String>,
    /// Dataset, checksum, or the rejection message.
    pub last_detail: Option<String>,
    /// When the last pull was attempted.
    pub last_attempt_at: Option<DateTime<Utc>>,
    /// The scheduler waits until this instant before pulling again.
    pub retry_after: Option<DateTime<Utc>>,
}

/// Why the sync settings could not be read or written.
#[derive(Debug, Error)]
pub enum OsvSyncStoreError {
    /// The table does not exist yet.
    #[error("osv sync schema is missing; run database migrations")]
    MissingSchema,

    /// The database returned an error.
    #[error("osv sync store failure: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Stores the signed-pull configuration. One row for the instance.
#[async_trait]
pub trait OsvSyncStore: Send + Sync {
    /// The saved pull, if an administrator has configured one.
    ///
    /// # Errors
    ///
    /// Returns [`OsvSyncStoreError`] if the database cannot be read.
    async fn current(&self) -> Result<Option<OsvSyncSettings>, OsvSyncStoreError>;

    /// Replaces the reference and public key and clears the last attempt.
    ///
    /// # Errors
    ///
    /// Returns [`OsvSyncStoreError`] if the database cannot be written.
    async fn save(
        &self,
        reference: &str,
        public_key_pem: &str,
    ) -> Result<OsvSyncSettings, OsvSyncStoreError>;

    /// Records the result of a pull. The active index is updated elsewhere.
    ///
    /// # Errors
    ///
    /// Returns [`OsvSyncStoreError`] if no row exists or the database cannot be written.
    async fn record_attempt(
        &self,
        outcome: &str,
        detail: &str,
        attempted_at: DateTime<Utc>,
        retry_after: DateTime<Utc>,
    ) -> Result<OsvSyncSettings, OsvSyncStoreError>;

    /// Deletes the saved pull. Does nothing when no row exists.
    ///
    /// # Errors
    ///
    /// Returns [`OsvSyncStoreError`] if the database cannot be written.
    async fn delete(&self) -> Result<(), OsvSyncStoreError>;
}
