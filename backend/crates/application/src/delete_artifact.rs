use std::sync::Arc;

use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::storage_key::storage_key_for;

/// Motivos por los que eliminar un artefacto puede fallar.
#[derive(Debug, Error)]
pub enum DeleteArtifactError {
    /// El repositorio no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// El artefacto no existe en ese repositorio.
    #[error("artifact {0} does not exist in repository {1}")]
    ArtifactNotFound(ArtifactId, RepositoryId),

    /// Fallo al consultar el almacén de repositorios.
    #[error(transparent)]
    RepositoryPersistence(#[from] RepositoryStoreError),

    /// Fallo al consultar o actualizar el almacén de artefactos.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),

    /// Fallo al consultar o actualizar el índice de paquetes.
    #[error(transparent)]
    IndexPersistence(#[from] PackageIndexStoreError),

    /// Fallo al eliminar el objeto binario.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Caso de uso: eliminar un artefacto de un repositorio, incluyendo su
/// objeto en almacenamiento y cualquier entrada de índice asociada.
pub struct DeleteArtifactUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
}

impl DeleteArtifactUseCase {
    /// Construye el caso de uso a partir de sus puertos.
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

    /// Elimina el artefacto: primero el objeto binario, después el
    /// índice y por último los metadatos.
    ///
    /// # Errors
    ///
    /// Devuelve [`DeleteArtifactError::RepositoryNotFound`] o
    /// [`DeleteArtifactError::ArtifactNotFound`] si no existen, o
    /// cualquiera de las demás variantes si falla un puerto.
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
        artifact_id: ArtifactId,
    ) -> Result<(), DeleteArtifactError> {
        if self
            .repository_store
            .find_by_id(repository_id)
            .await?
            .is_none()
        {
            return Err(DeleteArtifactError::RepositoryNotFound(repository_id));
        }

        let Some(artifact) = self.artifact_store.find_by_id(artifact_id).await? else {
            return Err(DeleteArtifactError::ArtifactNotFound(
                artifact_id,
                repository_id,
            ));
        };

        if artifact.repository_id() != repository_id {
            return Err(DeleteArtifactError::ArtifactNotFound(
                artifact_id,
                repository_id,
            ));
        }

        self.storage.delete(&storage_key_for(artifact.id())).await?;
        self.package_index_store
            .delete_by_artifact(artifact_id)
            .await?;
        self.artifact_store.delete(artifact_id).await?;
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
    async fn deletes_storage_index_and_metadata() {
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
                artifact.id(),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        DeleteArtifactUseCase::new(
            repository_store,
            artifact_store.clone(),
            package_index_store.clone(),
            storage.clone(),
        )
        .execute(repository.id(), artifact.id())
        .await
        .unwrap();

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
    async fn rejects_artifact_from_another_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());

        let repository = forge("one");
        let other = forge("two");
        repository_store.save(&repository).await.unwrap();
        repository_store.save(&other).await.unwrap();

        let artifact = ferrobox_domain::artifact::Artifact::new(
            other.id(),
            Sha256Checksum::parse("a".repeat(64)).unwrap(),
            1,
        );
        artifact_store.save(&artifact).await.unwrap();

        let err = DeleteArtifactUseCase::new(
            repository_store,
            artifact_store,
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
        )
        .execute(repository.id(), artifact.id())
        .await
        .unwrap_err();

        assert!(matches!(err, DeleteArtifactError::ArtifactNotFound(_, _)));
    }

    #[tokio::test]
    async fn missing_repository_is_not_found() {
        let err = DeleteArtifactUseCase::new(
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
        )
        .execute(RepositoryId::new(), ArtifactId::new())
        .await
        .unwrap_err();

        assert!(matches!(err, DeleteArtifactError::RepositoryNotFound(_)));
    }
}
