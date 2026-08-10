use async_trait::async_trait;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use thiserror::Error;

/// Motivos por los que una operación de persistencia de artefactos puede
/// fallar.
#[derive(Debug, Error)]
pub enum ArtifactStoreError {
    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la entidad [`Artifact`].
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Guarda un artefacto, insertándolo si es nuevo o actualizando sus
    /// datos si ya existía uno con el mismo identificador.
    ///
    /// # Errors
    ///
    /// Devuelve [`ArtifactStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn save(&self, artifact: &Artifact) -> Result<(), ArtifactStoreError>;

    /// Busca un artefacto por su identificador. Devuelve `None` si no
    /// existe.
    ///
    /// # Errors
    ///
    /// Devuelve [`ArtifactStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn find_by_id(&self, id: ArtifactId) -> Result<Option<Artifact>, ArtifactStoreError>;

    /// Lista todos los artefactos de un repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`ArtifactStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn find_by_repository_id(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Artifact>, ArtifactStoreError>;

    /// Elimina un artefacto. No es un error eliminar un identificador que
    /// no existe.
    ///
    /// # Errors
    ///
    /// Devuelve [`ArtifactStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn delete(&self, id: ArtifactId) -> Result<(), ArtifactStoreError>;
}
