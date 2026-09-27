use async_trait::async_trait;
use ferrobox_domain::audit::AuditEvent;
use thiserror::Error;

/// Reasons an audit-log operation can fail.
#[derive(Debug, Error)]
pub enum AuditStoreError {
    /// The table is missing: the SQL migration has not been run.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table audit_events is missing)"
    )]
    MissingSchema,

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for the audit log.
#[async_trait]
pub trait AuditStore: Send + Sync {
    /// Inserts an event and trims history to the 200 most recent rows.
    ///
    /// # Errors
    ///
    /// Returns [`AuditStoreError::Backend`] if the backend fails.
    async fn record(&self, event: &AuditEvent) -> Result<(), AuditStoreError>;

    /// Latest events for the instance, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`AuditStoreError::Backend`] if the backend fails.
    async fn list(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditStoreError>;
}
