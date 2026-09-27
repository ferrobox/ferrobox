use async_trait::async_trait;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use thiserror::Error;

/// Reasons an artifact persistence operation can fail.
#[derive(Debug, Error)]
pub enum ArtifactStoreError {
    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for the [`Artifact`] entity.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Saves an artifact, inserting it if new or updating its data if one
    /// with the same identifier already existed.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactStoreError::Backend`] if the underlying backend
    /// fails.
    async fn save(&self, artifact: &Artifact) -> Result<(), ArtifactStoreError>;

    /// Looks up an artifact by identifier. Returns `None` if it does not
    /// exist.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_id(&self, id: ArtifactId) -> Result<Option<Artifact>, ArtifactStoreError>;

    /// Lists every artifact in a repository.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_repository_id(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Artifact>, ArtifactStoreError>;

    /// Deletes an artifact. Deleting a missing identifier is not an
    /// error.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactStoreError::Backend`] if the underlying backend
    /// fails.
    async fn delete(&self, id: ArtifactId) -> Result<(), ArtifactStoreError>;

    /// Sum of every stored binary on the instance, including blobs that
    /// are no longer in a catalog until garbage collection runs.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::Backend`] if the underlying backend fails.
    async fn total_size_bytes(&self) -> Result<u64, ArtifactStoreError>;
}
