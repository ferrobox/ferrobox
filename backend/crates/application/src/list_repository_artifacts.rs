use std::sync::Arc;

use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};

/// Caso de uso: listar los artefactos de un repositorio.
pub struct ListRepositoryArtifactsUseCase {
    artifact_store: Arc<dyn ArtifactStore>,
}

impl ListRepositoryArtifactsUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(artifact_store: Arc<dyn ArtifactStore>) -> Self {
        Self { artifact_store }
    }

    /// Lista los artefactos del repositorio indicado.
    ///
    /// # Errors
    ///
    /// Devuelve [`ArtifactStoreError::Backend`] si el backend subyacente
    /// falla.
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Artifact>, ArtifactStoreError> {
        self.artifact_store
            .find_by_repository_id(repository_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::checksum::Sha256Checksum;

    use crate::test_support::InMemoryArtifactStore;

    use super::*;

    #[tokio::test]
    async fn lists_only_artifacts_belonging_to_the_repository() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let repository_id = RepositoryId::new();
        let other_repository_id = RepositoryId::new();

        let checksum = Sha256Checksum::parse("a".repeat(64)).unwrap();
        let matching = Artifact::new(repository_id, checksum.clone(), 10);
        let other = Artifact::new(other_repository_id, checksum, 20);
        artifact_store.save(&matching).await.unwrap();
        artifact_store.save(&other).await.unwrap();

        let use_case = ListRepositoryArtifactsUseCase::new(artifact_store);
        let result = use_case.execute(repository_id).await.unwrap();

        assert_eq!(result, vec![matching]);
    }
}
