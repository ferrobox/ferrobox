//! Registro de auditoría: persistir y listar escrituras de negocio.

use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use ferrobox_domain::audit::{AuditAction, AuditEvent, AuditTargetKind};
use ferrobox_domain::ids::AuditEventId;
use ferrobox_domain::user::User;
use ferrobox_ports::audit_store::{AuditStore, AuditStoreError};
use thiserror::Error;

/// Cuántas filas se conservan en el registro.
pub const AUDIT_HISTORY_LIMIT: usize = 200;

/// Motivos por los que consultar el registro puede fallar.
#[derive(Debug, Error)]
pub enum AuditError {
    /// Fallo al leer o escribir eventos.
    #[error(transparent)]
    Store(#[from] AuditStoreError),
}

/// Caso de uso: registrar y listar eventos de auditoría.
#[derive(Clone)]
pub struct AuditService {
    store: Arc<dyn AuditStore>,
}

impl AuditService {
    /// Construye el servicio a partir de su puerto.
    #[must_use]
    pub fn new(store: Arc<dyn AuditStore>) -> Self {
        Self { store }
    }

    /// Deja constancia de una escritura. Un fallo de persistencia no se
    /// propaga: la operación de negocio ya ha terminado.
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

    /// Últimos eventos de la instancia, más recientes primero.
    ///
    /// # Errors
    ///
    /// [`AuditError::Store`] si el backend falla, incluida la tabla
    /// ausente (`MissingSchema`): no se oculta como lista vacía.
    pub async fn list(&self) -> Result<Vec<AuditEvent>, AuditError> {
        Ok(self.store.list(AUDIT_HISTORY_LIMIT).await?)
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

    struct MissingSchemaStore;

    #[async_trait::async_trait]
    impl ferrobox_ports::audit_store::AuditStore for MissingSchemaStore {
        async fn record(
            &self,
            _event: &AuditEvent,
        ) -> Result<(), ferrobox_ports::audit_store::AuditStoreError> {
            Err(ferrobox_ports::audit_store::AuditStoreError::MissingSchema)
        }

        async fn list(
            &self,
            _limit: usize,
        ) -> Result<Vec<AuditEvent>, ferrobox_ports::audit_store::AuditStoreError> {
            Err(ferrobox_ports::audit_store::AuditStoreError::MissingSchema)
        }
    }

    #[tokio::test]
    async fn list_surfaces_a_missing_table() {
        let service = AuditService::new(Arc::new(MissingSchemaStore));
        let error = service.list().await.unwrap_err();
        assert!(matches!(
            error,
            AuditError::Store(ferrobox_ports::audit_store::AuditStoreError::MissingSchema)
        ));
    }
}
