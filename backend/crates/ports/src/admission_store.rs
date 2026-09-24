use async_trait::async_trait;
use ferrobox_domain::admission::{AdmissionEvent, AdmissionPolicy};
use ferrobox_domain::ids::RepositoryId;
use thiserror::Error;

/// Motivos por los que una operación sobre la política de admisión puede
/// fallar.
#[derive(Debug, Error)]
pub enum AdmissionStoreError {
    /// No existe la tabla de políticas: falta ejecutar la migración SQL.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_admission is missing)"
    )]
    MissingSchema,

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la política de admisión de un repositorio.
#[async_trait]
pub trait AdmissionStore: Send + Sync {
    /// Devuelve la política del repositorio, o la inactiva por defecto.
    ///
    /// # Errors
    ///
    /// Devuelve [`AdmissionStoreError::Backend`] si el backend falla.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<AdmissionPolicy, AdmissionStoreError>;

    /// Inserta o reemplaza la política del repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`AdmissionStoreError::Backend`] si el backend falla.
    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
    ) -> Result<(), AdmissionStoreError>;

    /// Registra un aviso o una denegación y recorta el historial a 50
    /// filas por repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`AdmissionStoreError::Backend`] si el backend falla.
    async fn record_event(&self, event: &AdmissionEvent) -> Result<(), AdmissionStoreError>;

    /// Últimos eventos del repositorio, más recientes primero.
    ///
    /// # Errors
    ///
    /// Devuelve [`AdmissionStoreError::Backend`] si el backend falla.
    async fn list_events(
        &self,
        repository_id: RepositoryId,
        limit: usize,
    ) -> Result<Vec<AdmissionEvent>, AdmissionStoreError>;
}
