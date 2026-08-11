use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;
use url::Url;

/// Motivos por los que crear un repositorio puede fallar.
#[derive(Debug, Error)]
pub enum CreateRepositoryError {
    /// Un `Mirror` solo está soportado para el ecosistema Cargo en esta
    /// versión.
    #[error("mirror repositories currently require the cargo ecosystem")]
    UnsupportedMirrorEcosystem,

    /// La URL *upstream* de un `Mirror` no es válida.
    #[error("invalid upstream URL: {0}")]
    InvalidUpstream(String),

    /// Fallo al persistir el repositorio (incluye el caso de nombre
    /// duplicado).
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

/// Descripción del tipo de repositorio a crear.
#[derive(Debug, Clone)]
pub enum CreateRepositoryKind {
    /// Almacenamiento propio.
    Forge,
    /// Réplica cacheada de un *upstream*.
    Mirror {
        /// URL base del índice disperso remoto.
        upstream: String,
    },
}

/// Caso de uso: crear un nuevo repositorio (`Forge` o `Mirror`).
pub struct CreateRepositoryUseCase {
    repository_store: Arc<dyn RepositoryStore>,
}

impl CreateRepositoryUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self { repository_store }
    }

    /// Crea un repositorio con el nombre, ecosistema y tipo indicados.
    ///
    /// # Errors
    ///
    /// Devuelve [`CreateRepositoryError`] si el tipo no está soportado,
    /// la URL *upstream* es inválida, el nombre ya está en uso, o el
    /// backend falla.
    ///
    /// # Panics
    ///
    /// En la práctica, nunca entra en pánico: ni `Forge` ni un `Mirror`
    /// con *upstream* válido pueden violar el invariante de "Alloy sin
    /// miembros" que `Repository::new` valida.
    pub async fn execute(
        &self,
        name: RepositoryName,
        ecosystem: PackageEcosystem,
        kind: CreateRepositoryKind,
    ) -> Result<RepositoryId, CreateRepositoryError> {
        let repository_kind = match kind {
            CreateRepositoryKind::Forge => RepositoryKind::Forge,
            CreateRepositoryKind::Mirror { upstream } => {
                if ecosystem != PackageEcosystem::Cargo {
                    return Err(CreateRepositoryError::UnsupportedMirrorEcosystem);
                }
                let upstream = Url::parse(upstream.trim()).map_err(|err| {
                    CreateRepositoryError::InvalidUpstream(err.to_string())
                })?;
                RepositoryKind::Mirror { upstream }
            }
        };

        let repository = Repository::new(name, repository_kind, ecosystem)
            .expect("Forge/Mirror never violate the Alloy non-empty invariant");

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
    async fn creates_a_forge_repository_by_default_shape() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("cargo-releases").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        assert!(matches!(repository.kind(), RepositoryKind::Forge));
    }

    #[tokio::test]
    async fn creates_a_cargo_mirror_with_upstream() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("crates-io").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Mirror {
                    upstream: "https://index.crates.io/".to_string(),
                },
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        match repository.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://index.crates.io/");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_a_non_cargo_mirror() {
        let use_case = CreateRepositoryUseCase::new(Arc::new(InMemoryRepositoryStore::default()));

        let result = use_case
            .execute(
                RepositoryName::parse("npm-mirror").unwrap(),
                PackageEcosystem::Npm,
                CreateRepositoryKind::Mirror {
                    upstream: "https://registry.npmjs.org/".to_string(),
                },
            )
            .await;

        assert!(matches!(
            result,
            Err(CreateRepositoryError::UnsupportedMirrorEcosystem)
        ));
    }

    #[tokio::test]
    async fn rejects_a_duplicate_name() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store);

        use_case
            .execute(
                RepositoryName::parse("cargo-releases").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        let result = use_case
            .execute(
                RepositoryName::parse("cargo-releases").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await;

        assert!(matches!(
            result,
            Err(CreateRepositoryError::Persistence(
                RepositoryStoreError::DuplicateName(_)
            ))
        ));
    }
}
