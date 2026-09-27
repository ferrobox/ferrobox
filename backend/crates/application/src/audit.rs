//! Audit log: persist and list business writes.

use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use ferrobox_domain::audit::{AuditAction, AuditEvent, AuditTargetKind};
use ferrobox_domain::ids::AuditEventId;
use ferrobox_domain::user::User;
use ferrobox_ports::audit_store::{AuditStore, AuditStoreError};
use thiserror::Error;

/// How many rows are kept in the log.
pub const AUDIT_HISTORY_LIMIT: usize = 200;

/// Reasons querying the log can fail.
#[derive(Debug, Error)]
pub enum AuditError {
    /// Failed to read or write events.
    #[error(transparent)]
    Store(#[from] AuditStoreError),
}

/// Use case: record and list audit events.
#[derive(Clone)]
pub struct AuditService {
    store: Arc<dyn AuditStore>,
}

impl AuditService {
    /// Builds the service from its port.
    #[must_use]
    pub fn new(store: Arc<dyn AuditStore>) -> Self {
        Self { store }
    }

    /// Records a write. A persistence failure is not propagated: the
    /// business operation has already finished.
    pub async fn record(
        &self,
        actor: &User,
        action: AuditAction,
        target_kind: AuditTargetKind,
        target: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let event = AuditEvent::from_parts(
            AuditEventId::new(),
            Some(actor.id()),
            actor.username().to_string(),
            action,
            target_kind,
            target,
            detail,
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        );
        let _ = self.store.record(&event).await;
    }

    /// Latest events of the instance, newest first.
    ///
    /// # Errors
    ///
    /// [`AuditError::Store`] if the backend fails (except a missing schema).
    pub async fn list(&self) -> Result<Vec<AuditEvent>, AuditError> {
        match self.store.list(AUDIT_HISTORY_LIMIT).await {
            Ok(events) => Ok(events),
            Err(AuditStoreError::MissingSchema) => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::user::{Role, Username};

    use super::*;
    use crate::test_support::InMemoryAuditStore;

    fn actor(name: &str) -> User {
        User::new(Username::parse(name).unwrap(), Role::Admin)
    }

    #[tokio::test]
    async fn records_and_lists_newest_first() {
        let service = AuditService::new(Arc::new(InMemoryAuditStore::default()));
        let admin = actor("admin");
        service
            .record(
                &admin,
                AuditAction::UserCreated,
                AuditTargetKind::User,
                "developer",
                "developer",
            )
            .await;
        service
            .record(
                &admin,
                AuditAction::PackageYanked,
                AuditTargetKind::Package,
                "serde",
                "1.0.0",
            )
            .await;

        let events = service.list().await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].action(), AuditAction::PackageYanked);
        assert_eq!(events[0].actor_username(), "admin");
        assert_eq!(events[0].target(), "serde");
        assert_eq!(events[1].action(), AuditAction::UserCreated);
    }

    #[tokio::test]
    async fn caps_history_at_two_hundred() {
        let store = Arc::new(InMemoryAuditStore::default());
        let service = AuditService::new(store);
        let admin = actor("admin");
        for index in 0..210 {
            service
                .record(
                    &admin,
                    AuditAction::TokenCreated,
                    AuditTargetKind::Token,
                    format!("token-{index}"),
                    "",
                )
                .await;
        }
        let events = service.list().await.unwrap();
        assert_eq!(events.len(), AUDIT_HISTORY_LIMIT);
        assert_eq!(events[0].target(), "token-209");
    }
}
