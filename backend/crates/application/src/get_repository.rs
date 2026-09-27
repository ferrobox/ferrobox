use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::Repository;
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Reasons looking up a repository can fail.
#[derive(Debug, Error)]
pub enum GetRepositoryError {
    /// The given repository does not exist.
    #[error("repository {0} does not exist")]
    NotFound(RepositoryId),

    /// Failed to query the repository.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

/// Use case: look up the details of an existing repository.
#[derive(Clone)]
pub struct GetRepositoryUseCase {
    repository_store: Arc<dyn RepositoryStore>,
}

impl GetRepositoryUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self { repository_store }
    }

    /// Looks up the given repository.
    ///
    /// # Errors
    ///
    /// Returns [`GetRepositoryError::NotFound`] if `id` does not match
    /// any existing repository, or [`GetRepositoryError::Persistence`]
    /// if the underlying backend fails.
    pub async fn execute(&self, id: RepositoryId) -> Result<Repository, GetRepositoryError> {
        self.repository_store
            .find_by_id(id)
            .await?
            .ok_or(GetRepositoryError::NotFound(id))
    }

    /// Persists an already loaded repository (for example, after
    /// changing the prefetch interval).
    ///
    /// # Errors
    ///
    /// [`GetRepositoryError::Persistence`] if the underlying backend fails.
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
