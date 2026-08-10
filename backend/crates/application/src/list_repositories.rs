use std::sync::Arc;

use ferrobox_domain::repository::Repository;
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};

/// Caso de uso: listar todos los repositorios existentes.
pub struct ListRepositoriesUseCase {
    repository_store: Arc<dyn RepositoryStore>,
}

impl ListRepositoriesUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self { repository_store }
    }

    /// Lista todos los repositorios existentes.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryStoreError::Backend`] si el backend subyacente
    /// falla.
    pub async fn execute(&self) -> Result<Vec<Repository>, RepositoryStoreError> {
        self.repository_store.find_all().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::test_support::{InMemoryRepositoryStore, forge};

    use super::*;

    #[tokio::test]
    async fn lists_every_existing_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        repository_store.save(&forge("cargo-releases")).await.unwrap();
        repository_store.save(&forge("npm-releases")).await.unwrap();

        let use_case = ListRepositoriesUseCase::new(repository_store);
        let repositories = use_case.execute().await.unwrap();

        assert_eq!(repositories.len(), 2);
    }

    #[tokio::test]
    async fn returns_an_empty_list_when_there_are_no_repositories() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = ListRepositoriesUseCase::new(repository_store);

        assert_eq!(use_case.execute().await.unwrap(), Vec::new());
    }
}
