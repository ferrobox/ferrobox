use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Motivos por los que crear un repositorio puede fallar.
#[derive(Debug, Error)]
pub enum CreateRepositoryError {
    /// Fallo al persistir el repositorio (incluye el caso de nombre
    /// duplicado).
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

/// Caso de uso: crear un nuevo repositorio de tipo `Forge`.
pub struct CreateRepositoryUseCase {
    repository_store: Arc<dyn RepositoryStore>,
}

impl CreateRepositoryUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self { repository_store }
    }

    /// Crea un repositorio `Forge` con el nombre indicado.
    ///
    /// # Errors
    ///
    /// Devuelve [`CreateRepositoryError::Persistence`] si el nombre ya
    /// está en uso, o si el backend subyacente falla.
    ///
    /// # Panics
    ///
    /// En la práctica, nunca entra en pánico: `RepositoryKind::Forge`
    /// nunca puede violar el invariante de "Alloy sin miembros" que
    /// `Repository::new` valida.
    pub async fn execute(
        &self,
        name: RepositoryName,
    ) -> Result<RepositoryId, CreateRepositoryError> {
        let repository = Repository::new(name, RepositoryKind::Forge)
            .expect("RepositoryKind::Forge never violates the Alloy non-empty invariant");

        self.repository_store.save(&repository).await?;

        Ok(repository.id())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::test_support::InMemoryRepositoryStore;

    use super::*;

    #[tokio::test]
    async fn creates_a_repository_with_a_new_identity() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(RepositoryName::parse("cargo-releases").unwrap())
            .await
            .unwrap();

        assert!(repository_store.find_by_id(id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn rejects_a_duplicate_name() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store);

        use_case
            .execute(RepositoryName::parse("cargo-releases").unwrap())
            .await
            .unwrap();

        let result = use_case
            .execute(RepositoryName::parse("cargo-releases").unwrap())
            .await;

        assert!(matches!(
            result,
            Err(CreateRepositoryError::Persistence(
                RepositoryStoreError::DuplicateName(_)
            ))
        ));
    }
}
