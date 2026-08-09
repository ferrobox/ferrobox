use std::sync::Arc;

use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::ArtifactId;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::storage_key::storage_key_for;

/// Motivos por los que descargar un artefacto puede fallar.
#[derive(Debug, Error)]
pub enum DownloadArtifactError {
    /// El artefacto indicado no existe.
    #[error("artifact {0} does not exist")]
    NotFound(ArtifactId),

    /// Fallo al consultar los metadatos del artefacto.
    #[error(transparent)]
    ArtifactLookup(#[from] ArtifactStoreError),

    /// Fallo al descargar el contenido binario.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Caso de uso: descargar un artefacto ya publicado.
pub struct DownloadArtifactUseCase {
    artifact_store: Arc<dyn ArtifactStore>,
    storage: Arc<dyn StoragePort>,
}

impl DownloadArtifactUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(artifact_store: Arc<dyn ArtifactStore>, storage: Arc<dyn StoragePort>) -> Self {
        Self {
            artifact_store,
            storage,
        }
    }

    /// Ejecuta la descarga: busca los metadatos del artefacto y luego su
    /// contenido binario.
    ///
    /// # Errors
    ///
    /// Devuelve [`DownloadArtifactError::NotFound`] si `artifact_id` no
    /// corresponde a ningún artefacto existente, o cualquiera de las
    /// demás variantes si falla el puerto correspondiente.
    pub async fn execute(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<(Artifact, Bytes), DownloadArtifactError> {
        let artifact = self
            .artifact_store
            .find_by_id(artifact_id)
            .await?
            .ok_or(DownloadArtifactError::NotFound(artifact_id))?;

        let content = self.storage.get(&storage_key_for(artifact.id())).await?;

        Ok((artifact, content))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::ids::RepositoryId;

    use crate::test_support::{InMemoryArtifactStore, InMemoryStorage};

    use super::*;

    #[tokio::test]
    async fn downloads_a_previously_stored_artifact() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());

        let artifact = Artifact::new(
            RepositoryId::new(),
            Sha256Checksum::parse("a".repeat(64)).unwrap(),
            5,
        );
        artifact_store.save(&artifact).await.unwrap();
        storage
            .put(
                &storage_key_for(artifact.id()),
                Bytes::from_static(b"hello"),
            )
            .await
            .unwrap();

        let use_case = DownloadArtifactUseCase::new(artifact_store, storage);
        let (found, content) = use_case.execute(artifact.id()).await.unwrap();

        assert_eq!(found.id(), artifact.id());
        assert_eq!(content, Bytes::from_static(b"hello"));
    }

    #[tokio::test]
    async fn rejects_downloading_a_nonexistent_artifact() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let use_case = DownloadArtifactUseCase::new(artifact_store, storage);

        let result = use_case.execute(ArtifactId::new()).await;

        assert!(matches!(result, Err(DownloadArtifactError::NotFound(_))));
    }
}
