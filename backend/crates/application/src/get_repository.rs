use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::Repository;
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Motivos por los que consultar un repositorio puede fallar.
#[derive(Debug, Error)]
pub enum GetRepositoryError {
    /// El repositorio indicado no existe.
    #[error("repository {0} does not exist")]
    NotFound(RepositoryId),

    /// Fallo al consultar el repositorio.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

/// Caso de uso: consultar el detalle de un repositorio existente.
pub struct GetRepositoryUseCase {
    repository_store: Arc<dyn RepositoryStore>,
}

impl GetRepositoryUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self { repository_store }
    }

    /// Busca el repositorio indicado.
    ///
    /// # Errors
    ///
    /// Devuelve [`GetRepositoryError::NotFound`] si `id` no corresponde a
    /// ningún repositorio existente, o [`GetRepositoryError::Persistence`]
    /// si el backend subyacente falla.
    pub async fn execute(&self, id: RepositoryId) -> Result<Repository, GetRepositoryError> {
        self.repository_store
            .find_by_id(id)
            .await?
            .ok_or(GetRepositoryError::NotFound(id))
    }

    /// Persiste un repositorio ya cargado (por ejemplo, tras cambiar el
    /// intervalo de prefetch).
    ///
    /// # Errors
    ///
    /// [`GetRepositoryError::Persistence`] si el backend subyacente falla.
    pub async fn save(&self, repository: &Repository) -> Result<(), GetRepositoryError> {
        self.repository_store.save(repository).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::ids::RepositoryId;

    use crate::test_support::{InMemoryRepositoryStore, forge};

    use super::*;

    #[tokio::test]
    async fn finds_an_existing_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let repository = forge("cargo-releases");
        repository_store.save(&repository).await.unwrap();

        let use_case = GetRepositoryUseCase::new(repository_store);
        let found = use_case.execute(repository.id()).await.unwrap();

        assert_eq!(found, repository);
    }

    #[tokio::test]
    async fn rejects_a_nonexistent_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = GetRepositoryUseCase::new(repository_store);

        let result = use_case.execute(RepositoryId::new()).await;

        assert!(matches!(result, Err(GetRepositoryError::NotFound(_))));
    }

    #[tokio::test]
    async fn save_persists_prefetch_schedule() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let repository = forge("cargo-releases").with_prefetch_schedule(Some(6), None);
        let use_case = GetRepositoryUseCase::new(repository_store);

        use_case.save(&repository).await.unwrap();
        let found = use_case.execute(repository.id()).await.unwrap();

        assert_eq!(found.prefetch_interval_hours(), Some(6));
    }
}
