use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::quota::StorageQuota;
use thiserror::Error;

/// Motivos por los que una operación sobre la cuota puede fallar.
#[derive(Debug, Error)]
pub enum QuotaStoreError {
    /// No existe la tabla de cuotas: falta ejecutar la migración SQL.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_quota is missing)"
    )]
    MissingSchema,

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la cuota de almacenamiento de un repositorio.
#[async_trait]
pub trait QuotaStore: Send + Sync {
    /// Devuelve la cuota del repositorio, o «ilimitada» si nunca se configuró.
    ///
    /// # Errors
    ///
    /// Devuelve [`QuotaStoreError::Backend`] si el backend subyacente falla.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<StorageQuota, QuotaStoreError>;

    /// Inserta o reemplaza la cuota del repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`QuotaStoreError::Backend`] si el backend subyacente falla.
    async fn save(
        &self,
        repository_id: RepositoryId,
        quota: StorageQuota,
    ) -> Result<(), QuotaStoreError>;
}
