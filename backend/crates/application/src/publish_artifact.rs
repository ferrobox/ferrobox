use std::sync::Arc;

use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::content_hash::sha256_checksum;
use crate::storage_key::storage_key_for;

/// Motivos por los que publicar un artefacto puede fallar.
#[derive(Debug, Error)]
pub enum PublishArtifactError {
    /// El repositorio indicado no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// Fallo al consultar el repositorio.
    #[error(transparent)]
    RepositoryLookup(#[from] RepositoryStoreError),

    /// Fallo al subir el contenido binario.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// Fallo al persistir los metadatos del artefacto.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),
}

/// Caso de uso: publicar un artefacto binario en un repositorio
/// existente.
pub struct PublishArtifactUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    storage: Arc<dyn StoragePort>,
}

impl PublishArtifactUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        storage: Arc<dyn StoragePort>,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            storage,
        }
    }

    /// Ejecuta la publicación: verifica que el repositorio existe,
    /// calcula el checksum del contenido, lo sube al almacenamiento
    /// binario, y persiste los metadatos del artefacto.
    ///
    /// # Errors
    ///
    /// Devuelve [`PublishArtifactError::RepositoryNotFound`] si
    /// `repository_id` no corresponde a ningún repositorio existente, o
    /// cualquiera de las demás variantes si falla el puerto
    /// correspondiente.
    ///
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
        content: Bytes,
    ) -> Result<ArtifactId, PublishArtifactError> {
        if self
            .repository_store
            .find_by_id(repository_id)
            .await?
            .is_none()
        {
            return Err(PublishArtifactError::RepositoryNotFound(repository_id));
        }

        let checksum = sha256_checksum(&content);
        let artifact = Artifact::new(repository_id, checksum, content.len() as u64);

        self.storage
            .put(&storage_key_for(artifact.id()), content)
            .await?;
        self.artifact_store.save(&artifact).await?;

        Ok(artifact.id())
    }
}

use ferrobox_domain::ids::ArtifactId;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;

    use crate::test_support::{
        InMemoryArtifactStore, InMemoryRepositoryStore, InMemoryStorage, forge,
    };

    use super::*;

    #[tokio::test]
    async fn publishes_an_artifact_to_an_existing_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());

        let repository = forge("cargo-releases");
        repository_store.save(&repository).await.unwrap();

        let use_case =
            PublishArtifactUseCase::new(repository_store, artifact_store.clone(), storage);

        let artifact_id = use_case
            .execute(repository.id(), Bytes::from_static(b"hello, ferrobox"))
            .await
            .unwrap();

        let stored = artifact_store
            .find_by_id(artifact_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.checksum().as_str(),
            crate::content_hash::sha256_checksum(b"hello, ferrobox").as_str()
        );
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_nonexistent_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let use_case = PublishArtifactUseCase::new(repository_store, artifact_store, storage);

        let result = use_case
            .execute(RepositoryId::new(), Bytes::from_static(b"data"))
            .await;

        assert!(matches!(
            result,
            Err(PublishArtifactError::RepositoryNotFound(_))
        ));
    }
}
