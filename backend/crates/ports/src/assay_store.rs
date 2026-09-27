use async_trait::async_trait;
use ferrobox_domain::assay::Assay;
use ferrobox_domain::ids::{AssayId, RepositoryId};
use ferrobox_domain::package_coordinate::PackageCoordinate;
use thiserror::Error;

/// Reasons an Assay operation can fail.
#[derive(Debug, Error)]
pub enum AssayStoreError {
    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for the [`Assay`] entity.
#[async_trait]
pub trait AssayStore: Send + Sync {
    /// Inserts or replaces the Assay of a coordinate in a repository.
    ///
    /// # Errors
    ///
    /// Returns [`AssayStoreError::Backend`] if the underlying backend fails.
    async fn upsert(&self, assay: &Assay) -> Result<(), AssayStoreError>;

    /// Looks up an Assay by identifier.
    ///
    /// # Errors
    ///
    /// Returns [`AssayStoreError::Backend`] if the underlying backend fails.
    async fn find_by_id(&self, id: AssayId) -> Result<Option<Assay>, AssayStoreError>;

    /// Looks up the Assay of a coordinate in a repository.
    ///
    /// # Errors
    ///
    /// Returns [`AssayStoreError::Backend`] if the underlying backend fails.
    async fn find_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<Assay>, AssayStoreError>;

    /// Lists the Assays of a repository.
    ///
    /// # Errors
    ///
    /// Returns [`AssayStoreError::Backend`] if the underlying backend fails.
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Assay>, AssayStoreError>;

    /// Lists every Assay on the instance.
    ///
    /// # Errors
    ///
    /// Returns [`AssayStoreError::Backend`] if the underlying backend fails.
    async fn find_all(&self) -> Result<Vec<Assay>, AssayStoreError>;

    /// Deletes the Assay of a coordinate. It is not an error if none exists.
    ///
    /// # Errors
    ///
    /// Returns [`AssayStoreError::Backend`] if the underlying backend fails.
    async fn delete_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<(), AssayStoreError>;
}
