//! Persistence port for the active OSV (*Open Source Vulnerabilities*) index.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// An administrator uploaded the index with `POST /api/security/osv-feed`.
pub const OSV_FEED_SOURCE_FILE: &str = "file";

/// A scheduled pull of a signed OCI index installed this copy.
pub const OSV_FEED_SOURCE_SYNC: &str = "sync";

/// Metadata of the vulnerability index currently installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsvFeedRecord {
    /// Dataset label from the index, for example `2026-10-02`.
    pub dataset: String,
    /// Lowercase hex SHA-256 of the uploaded bytes.
    pub sha256: String,
    /// Advisories contained in the index.
    pub advisory_count: u64,
    /// `OSV` ecosystem names covered by the index.
    pub ecosystems: Vec<String>,
    /// Storage key of the uploaded bytes.
    pub storage_key: String,
    /// When this index was accepted.
    pub imported_at: DateTime<Utc>,
    /// [`OSV_FEED_SOURCE_FILE`] or [`OSV_FEED_SOURCE_SYNC`].
    pub source: String,
}

/// Why the feed record could not be read or written.
#[derive(Debug, Error)]
pub enum OsvFeedStoreError {
    /// The table does not exist yet.
    #[error("osv feed schema is missing; run database migrations")]
    MissingSchema,

    /// The database returned an error.
    #[error("osv feed store failure: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Stores which vulnerability index is active. The bytes themselves
/// live in the object store, under [`OsvFeedRecord::storage_key`].
#[async_trait]
pub trait OsvFeedStore: Send + Sync {
    /// The installed index, if an administrator has imported one.
    ///
    /// # Errors
    ///
    /// Returns [`OsvFeedStoreError`] if the database cannot be read.
    async fn current(&self) -> Result<Option<OsvFeedRecord>, OsvFeedStoreError>;

    /// Replaces the installed index metadata.
    ///
    /// # Errors
    ///
    /// Returns [`OsvFeedStoreError`] if the database cannot be written.
    async fn save(&self, record: &OsvFeedRecord) -> Result<(), OsvFeedStoreError>;

    /// Deletes the installed index metadata.
    ///
    /// Returns the removed record so the caller can drop its bytes.
    /// `Ok(None)` means nothing was installed.
    ///
    /// # Errors
    ///
    /// Returns [`OsvFeedStoreError`] if the database cannot be written.
    async fn delete(&self) -> Result<Option<OsvFeedRecord>, OsvFeedStoreError>;
}
