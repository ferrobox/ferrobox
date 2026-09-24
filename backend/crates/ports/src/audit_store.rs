use async_trait::async_trait;
use ferrobox_domain::audit::AuditEvent;
use thiserror::Error;

/// Motivos por los que una operación sobre el registro de auditoría
/// puede fallar.
#[derive(Debug, Error)]
pub enum AuditStoreError {
    /// No existe la tabla: falta ejecutar la migración SQL.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table audit_events is missing)"
    )]
    MissingSchema,

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia del registro de auditoría.
#[async_trait]
pub trait AuditStore: Send + Sync {
    /// Inserta un evento y recorta el historial a las 200 filas más
    /// recientes.
    ///
    /// # Errors
    ///
    /// Devuelve [`AuditStoreError::Backend`] si el backend falla.
    async fn record(&self, event: &AuditEvent) -> Result<(), AuditStoreError>;

    /// Últimos eventos de la instancia, más recientes primero.
    ///
    /// # Errors
    ///
    /// Devuelve [`AuditStoreError::Backend`] si el backend falla.
    async fn list(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditStoreError>;
}
