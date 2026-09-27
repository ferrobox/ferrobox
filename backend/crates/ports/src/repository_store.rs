use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::{Repository, RepositoryName};
use thiserror::Error;

/// Reasons a repository persistence operation can fail.
#[derive(Debug, Error)]
pub enum RepositoryStoreError {
    /// A repository with that name already exists.
    #[error("a repository named '{0}' already exists")]
    DuplicateName(RepositoryName),

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for the [`Repository`] entity.
///
/// Deliberately named `RepositoryStore`, not `RepositoryRepository`,
/// to avoid colliding the data-access "Repository" pattern (Martin
/// Fowler, *Patterns of Enterprise Application Architecture*, 2002)
/// with our own `Repository` domain entity.
#[async_trait]
pub trait RepositoryStore: Send + Sync {
    /// Saves a repository, inserting it if new or updating its data if
    /// one with the same identifier already existed.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryStoreError::DuplicateName`] if another
    /// repository with the same name already exists, or
    /// [`RepositoryStoreError::Backend`] if the underlying backend fails.
    async fn save(&self, repository: &Repository) -> Result<(), RepositoryStoreError>;

    /// Looks up a repository by identifier. Returns `None` if it does
    /// not exist -- unlike `StoragePort::get`, not finding a
    /// repository is not, by itself, an error condition.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryStoreError::Backend`] if the underlying
    /// backend fails.
    async fn find_by_id(
        &self,
        id: RepositoryId,
    ) -> Result<Option<Repository>, RepositoryStoreError>;

    /// Looks up a repository by name.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryStoreError::Backend`] if the underlying
    /// backend fails.
    async fn find_by_name(
        &self,
        name: &RepositoryName,
    ) -> Result<Option<Repository>, RepositoryStoreError>;

    /// Lists every existing repository.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryStoreError::Backend`] if the underlying
    /// backend fails.
    async fn find_all(&self) -> Result<Vec<Repository>, RepositoryStoreError>;

    /// Deletes a repository. Deleting a missing identifier is not an
    /// error.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryStoreError::Backend`] if the underlying
    /// backend fails.
    async fn delete(&self, id: RepositoryId) -> Result<(), RepositoryStoreError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::RepositoryKind;

    use super::*;

    #[derive(Default)]
    struct InMemoryRepositoryStore {
        repositories: Mutex<HashMap<RepositoryId, Repository>>,
    }

    #[async_trait]
    impl RepositoryStore for InMemoryRepositoryStore {
        async fn save(&self, repository: &Repository) -> Result<(), RepositoryStoreError> {
            let mut repositories = self.repositories.lock().unwrap();

            let name_taken_by_another = repositories.values().any(|existing| {
                existing.id() != repository.id() && existing.name() == repository.name()
            });

            if name_taken_by_another {
                return Err(RepositoryStoreError::DuplicateName(
                    repository.name().clone(),
                ));
            }

            repositories.insert(repository.id(), repository.clone());
            Ok(())
        }

        async fn find_by_id(
            &self,
            id: RepositoryId,
        ) -> Result<Option<Repository>, RepositoryStoreError> {
            Ok(self.repositories.lock().unwrap().get(&id).cloned())
        }

        async fn find_by_name(
            &self,
            name: &RepositoryName,
        ) -> Result<Option<Repository>, RepositoryStoreError> {
            Ok(self
                .repositories
                .lock()
                .unwrap()
                .values()
                .find(|r| r.name() == name)
                .cloned())
        }

        async fn find_all(&self) -> Result<Vec<Repository>, RepositoryStoreError> {
            Ok(self.repositories.lock().unwrap().values().cloned().collect())
        }

        async fn delete(&self, id: RepositoryId) -> Result<(), RepositoryStoreError> {
            self.repositories.lock().unwrap().remove(&id);
            Ok(())
        }
    }

    fn forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Generic,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn save_then_find_by_id_returns_the_same_repository() {
        let store = InMemoryRepositoryStore::default();
        let repository = forge("cargo-releases");

        store.save(&repository).await.unwrap();
        let found = store.find_by_id(repository.id()).await.unwrap();

        assert_eq!(found, Some(repository));
    }

    #[tokio::test]
    async fn find_by_id_on_a_missing_repository_returns_none() {
        let store = InMemoryRepositoryStore::default();

        let found = store.find_by_id(RepositoryId::new()).await.unwrap();

        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn saving_a_duplicate_name_is_rejected() {
        let store = InMemoryRepositoryStore::default();
        store.save(&forge("cargo-releases")).await.unwrap();

        let result = store.save(&forge("cargo-releases")).await;

        assert!(matches!(
            result,
            Err(RepositoryStoreError::DuplicateName(_))
        ));
    }

    #[tokio::test]
    async fn find_all_returns_every_saved_repository() {
        let store = InMemoryRepositoryStore::default();
        store.save(&forge("cargo-releases")).await.unwrap();
        store.save(&forge("npm-releases")).await.unwrap();

        let all = store.find_all().await.unwrap();

        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn find_all_on_an_empty_store_returns_an_empty_list() {
        let store = InMemoryRepositoryStore::default();

        assert_eq!(store.find_all().await.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn delete_removes_the_repository() {
        let store = InMemoryRepositoryStore::default();
        let repository = forge("cargo-releases");
        store.save(&repository).await.unwrap();

        store.delete(repository.id()).await.unwrap();

        assert_eq!(store.find_by_id(repository.id()).await.unwrap(), None);
    }
}
