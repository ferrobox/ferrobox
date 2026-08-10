use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::{Repository, RepositoryName};
use thiserror::Error;

/// Motivos por los que una operación de persistencia de repositorios
/// puede fallar.
#[derive(Debug, Error)]
pub enum RepositoryStoreError {
    /// Ya existe un repositorio con ese nombre.
    #[error("a repository named '{0}' already exists")]
    DuplicateName(RepositoryName),

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la entidad [`Repository`].
///
/// Se llama deliberadamente `RepositoryStore`, y no `RepositoryRepository`,
/// para evitar la colisión de nombres entre el patrón de acceso a datos
/// "Repository" (Martin Fowler, *Patterns of Enterprise Application
/// Architecture*, 2002) y nuestra propia entidad de dominio `Repository`.
#[async_trait]
pub trait RepositoryStore: Send + Sync {
    /// Guarda un repositorio, insertándolo si es nuevo o actualizando sus
    /// datos si ya existía uno con el mismo identificador.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryStoreError::DuplicateName`] si ya existe otro
    /// repositorio con el mismo nombre, o
    /// [`RepositoryStoreError::Backend`] si el backend subyacente falla.
    async fn save(&self, repository: &Repository) -> Result<(), RepositoryStoreError>;

    /// Busca un repositorio por su identificador. Devuelve `None` si no
    /// existe -- a diferencia de `StoragePort::get`, no encontrar un
    /// repositorio no es, en sí mismo, una condición de error.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn find_by_id(
        &self,
        id: RepositoryId,
    ) -> Result<Option<Repository>, RepositoryStoreError>;

    /// Busca un repositorio por su nombre.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn find_by_name(
        &self,
        name: &RepositoryName,
    ) -> Result<Option<Repository>, RepositoryStoreError>;

    /// Lista todos los repositorios existentes.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn find_all(&self) -> Result<Vec<Repository>, RepositoryStoreError>;

    /// Elimina un repositorio. No es un error eliminar un identificador
    /// que no existe.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryStoreError::Backend`] si el backend
    /// subyacente falla.
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
