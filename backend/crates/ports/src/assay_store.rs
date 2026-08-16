use async_trait::async_trait;
use ferrobox_domain::assay::Assay;
use ferrobox_domain::ids::{AssayId, RepositoryId};
use ferrobox_domain::package_coordinate::PackageCoordinate;
use thiserror::Error;

/// Motivos por los que una operación sobre ensayes puede fallar.
#[derive(Debug, Error)]
pub enum AssayStoreError {
    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la entidad [`Assay`].
#[async_trait]
pub trait AssayStore: Send + Sync {
    /// Inserta o reemplaza el ensaye de una coordenada en un repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayStoreError::Backend`] si el backend subyacente falla.
    async fn upsert(&self, assay: &Assay) -> Result<(), AssayStoreError>;

    /// Busca un ensaye por identificador.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayStoreError::Backend`] si el backend subyacente falla.
    async fn find_by_id(&self, id: AssayId) -> Result<Option<Assay>, AssayStoreError>;

    /// Busca el ensaye de una coordenada en un repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayStoreError::Backend`] si el backend subyacente falla.
    async fn find_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<Assay>, AssayStoreError>;

    /// Lista los ensayes de un repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayStoreError::Backend`] si el backend subyacente falla.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Assay>, AssayStoreError>;

    /// Lista todos los ensayes de la instancia.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayStoreError::Backend`] si el backend subyacente falla.
    async fn find_all(&self) -> Result<Vec<Assay>, AssayStoreError>;
}
