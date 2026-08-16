use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::retention::RetentionPolicy;
use thiserror::Error;

/// Motivos por los que una operación sobre la política de retención puede
/// fallar.
#[derive(Debug, Error)]
pub enum RetentionStoreError {
    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la política de retención de un repositorio.
#[async_trait]
pub trait RetentionStore: Send + Sync {
    /// Devuelve la política del repositorio, o «conservar todo» si nunca
    /// se configuró.
    ///
    /// # Errors
    ///
    /// Devuelve [`RetentionStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<RetentionPolicy, RetentionStoreError>;

    /// Inserta o reemplaza la política del repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`RetentionStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<(), RetentionStoreError>;
}
