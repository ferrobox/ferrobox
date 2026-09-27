use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::storage_key::storage_key_for;

/// Reasons deleting a repository can fail.
#[derive(Debug, Error)]
pub enum DeleteRepositoryError {
    /// The repository does not exist.
    #[error("repository {0} does not exist")]
    NotFound(RepositoryId),

    /// Failed to query or update the repository store.
    #[error(transparent)]
    RepositoryPersistence(#[from] RepositoryStoreError),

    /// Failed to query or update the artifact store.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),

    /// Failed to query or update the package index.
    #[error(transparent)]
    IndexPersistence(#[from] PackageIndexStoreError),

    /// Failed to delete a binary object.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Use case: delete a repository and all of its content.
///
/// The order is deliberate: first the binary objects in storage, then
/// the index and artifact rows (which reference the repository without
/// `ON DELETE CASCADE`), and finally the repository itself.
pub struct DeleteRepositoryUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
}

impl DeleteRepositoryUseCase {
    /// Builds the use case from its ports.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            package_index_store,
            storage,
        }
    }

    /// Deletes the repository and everything hanging off it.
    ///
    /// # Errors
    ///
    /// Returns [`DeleteRepositoryError::NotFound`] if the repository
    /// does not exist, or any of the other variants if a port fails.
    pub async fn execute(&self, repository_id: RepositoryId) -> Result<(), DeleteRepositoryError> {
        if self
            .repository_store
            .find_by_id(repository_id)
            .await?
            .is_none()
        {
            return Err(DeleteRepositoryError::NotFound(repository_id));
        }

        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;

        for artifact in &artifacts {
            self.storage.delete(&storage_key_for(artifact.id())).await?;
        }

        self.package_index_store
            .delete_by_repository(repository_id)
            .await?;

        for artifact in artifacts {
            self.artifact_store.delete(artifact.id()).await?;
        }

        self.repository_store.delete(repository_id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };

    use crate::storage_key::storage_key_for;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryStorage,
        forge,
    };

    use super::*;

    #[tokio::test]
    async fn deletes_repository_artifacts_index_and_storage() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());

        let repository = forge("cargo-releases");
        repository_store.save(&repository).await.unwrap();

        let artifact = ferrobox_domain::artifact::Artifact::new(
            repository.id(),
            Sha256Checksum::parse("a".repeat(64)).unwrap(),
            4,
        );
        artifact_store.save(&artifact).await.unwrap();
        storage
            .put(&storage_key_for(artifact.id()), Bytes::from_static(b"data"))
            .await
            .unwrap();

        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("ferrobox-cli").unwrap(),
            PackageVersion::parse("0.1.0").unwrap(),
        );
        package_index_store
            .upsert_entry(
                repository.id(),
                &coordinate,
                Some(artifact.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        DeleteRepositoryUseCase::new(
            repository_store.clone(),
            artifact_store.clone(),
            package_index_store.clone(),
            storage.clone(),
        )
        .execute(repository.id())
        .await
        .unwrap();

        assert!(
            repository_store
                .find_by_id(repository.id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            artifact_store
                .find_by_id(artifact.id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !storage
                .exists(&storage_key_for(artifact.id()))
                .await
                .unwrap()
        );
        assert_eq!(
            package_index_store
                .artifact_for(repository.id(), &coordinate)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn missing_repository_is_not_found() {
        let err = DeleteRepositoryUseCase::new(
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
        )
        .execute(RepositoryId::new())
        .await
        .unwrap_err();

        assert!(matches!(err, DeleteRepositoryError::NotFound(_)));
    }

    #[tokio::test]
    async fn deleting_an_empty_repository_succeeds() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let repository = forge("empty");
        repository_store.save(&repository).await.unwrap();

        DeleteRepositoryUseCase::new(
            repository_store.clone(),
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
        )
        .execute(repository.id())
        .await
        .unwrap();

        assert!(
            repository_store
                .find_by_id(repository.id())
                .await
                .unwrap()
                .is_none()
        );
    }
}
