use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::replica::ReplicaPolicy;
use thiserror::Error;

/// Motivos por los que persistir la política de réplica puede fallar.
#[derive(Debug, Error)]
pub enum ReplicaStoreError {
    /// No existe la tabla: falta ejecutar la migración SQL.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_replica is missing)"
    )]
    MissingSchema,

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la política de réplica de un repositorio.
#[async_trait]
pub trait ReplicaStore: Send + Sync {
    /// Devuelve la política, o «sin destino» si nunca se configuró.
    ///
    /// # Errors
    ///
    /// [`ReplicaStoreError::Backend`] si el backend subyacente falla.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPolicy, ReplicaStoreError>;

    /// Inserta o reemplaza la política. Sin destino borra la fila.
    ///
    /// # Errors
    ///
    /// [`ReplicaStoreError::Backend`] si el backend subyacente falla.
    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: &ReplicaPolicy,
    ) -> Result<(), ReplicaStoreError>;
}
