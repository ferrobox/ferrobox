use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;
use url::Url;

use crate::alloy_members::{resolve_alloy_members, ResolveAlloyMembersError};

/// Motivos por los que crear un repositorio puede fallar.
#[derive(Debug, Error)]
pub enum CreateRepositoryError {
    /// Un `Mirror` solo está soportado para Cargo, npm, `PyPI`, OCI y Helm.
    #[error(
        "mirror repositories currently require the cargo, npm, pypi, oci or helm ecosystem"
    )]
    UnsupportedMirrorEcosystem,

    /// La URL *upstream* de un `Mirror` no es válida.
    #[error("invalid upstream URL: {0}")]
    InvalidUpstream(String),

    /// Un `Alloy` necesita al menos un repositorio miembro.
    #[error("an Alloy repository must aggregate at least one member repository")]
    EmptyAlloy,

    /// Uno de los miembros indicados no existe.
    #[error("alloy member repository does not exist")]
    MemberNotFound,

    /// Un miembro no comparte el ecosistema del `Alloy`.
    #[error("alloy members must use the same package ecosystem")]
    MemberEcosystemMismatch,

    /// Un `Alloy` no puede agregar a otro `Alloy` (evita ciclos).
    #[error("alloy members must be Forge or Mirror repositories")]
    NestedAlloy,

    /// Fallo al persistir el repositorio (incluye el caso de nombre
    /// duplicado).
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

impl From<ResolveAlloyMembersError> for CreateRepositoryError {
    fn from(err: ResolveAlloyMembersError) -> Self {
        match err {
            ResolveAlloyMembersError::EmptyAlloy => Self::EmptyAlloy,
            ResolveAlloyMembersError::MemberNotFound => Self::MemberNotFound,
            ResolveAlloyMembersError::MemberEcosystemMismatch => Self::MemberEcosystemMismatch,
            ResolveAlloyMembersError::NestedAlloy => Self::NestedAlloy,
            ResolveAlloyMembersError::Persistence(inner) => Self::Persistence(inner),
        }
    }
}

/// Descripción del tipo de repositorio a crear.
#[derive(Debug, Clone)]
pub enum CreateRepositoryKind {
    /// Almacenamiento propio.
    Forge,
    /// Réplica cacheada de un *upstream*.
    Mirror {
        /// URL base del registro remoto (índice sparse de Cargo,
        /// registro npm, índice simple de `PyPI`, registro OCI o charts Helm).
        upstream: String,
    },
    /// Agregación de otros repositorios `Forge` o `Mirror`.
    Alloy {
        /// Identificadores de los repositorios miembro, en orden de
        /// resolución.
        members: Vec<RepositoryId>,
    },
}

/// Caso de uso: crear un nuevo repositorio (`Forge`, `Mirror` o `Alloy`).
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
    /// la URL *upstream* es inválida, un `Alloy` no tiene miembros
    /// válidos, el nombre ya está en uso, o el backend falla.
    ///
    /// # Panics
    ///
    /// En la práctica, nunca entra en pánico: `Forge`, un `Mirror` con
    /// *upstream* válido y un `Alloy` con al menos un miembro no pueden
    /// violar el invariante de "Alloy sin miembros" que `Repository::new`
    /// valida.
    pub async fn execute(
        &self,
        name: RepositoryName,
        ecosystem: PackageEcosystem,
        kind: CreateRepositoryKind,
    ) -> Result<RepositoryId, CreateRepositoryError> {
        let repository_kind = match kind {
            CreateRepositoryKind::Forge => RepositoryKind::Forge,
            CreateRepositoryKind::Mirror { upstream } => {
                if !matches!(
                    ecosystem,
                    PackageEcosystem::Cargo
                        | PackageEcosystem::Npm
                        | PackageEcosystem::PyPi
                        | PackageEcosystem::Oci
                        | PackageEcosystem::Helm
                ) {
                    return Err(CreateRepositoryError::UnsupportedMirrorEcosystem);
                }
                let upstream = Url::parse(upstream.trim()).map_err(|err| {
                    CreateRepositoryError::InvalidUpstream(err.to_string())
                })?;
                RepositoryKind::Mirror { upstream }
            }
            CreateRepositoryKind::Alloy { members } => {
                let members = resolve_alloy_members(
                    self.repository_store.as_ref(),
                    ecosystem,
                    members,
                )
                .await?;
                RepositoryKind::Alloy { members }
            }
        };

        let repository = Repository::new(name, repository_kind, ecosystem)
            .expect("validated Forge/Mirror/Alloy never violate the empty-Alloy invariant");

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
    async fn creates_an_npm_mirror_with_upstream() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("npm-proxy").unwrap(),
                PackageEcosystem::Npm,
                CreateRepositoryKind::Mirror {
                    upstream: "https://registry.npmjs.org/".to_string(),
                },
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        match repository.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://registry.npmjs.org/");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(repository.ecosystem(), PackageEcosystem::Npm);
    }

    #[tokio::test]
    async fn creates_a_pypi_mirror_with_upstream() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("pypi-proxy").unwrap(),
                PackageEcosystem::PyPi,
                CreateRepositoryKind::Mirror {
                    upstream: "https://pypi.org/simple/".to_string(),
                },
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        match repository.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://pypi.org/simple/");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(repository.ecosystem(), PackageEcosystem::PyPi);
    }

    #[tokio::test]
    async fn creates_an_oci_mirror_with_upstream() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("oci-proxy").unwrap(),
                PackageEcosystem::Oci,
                CreateRepositoryKind::Mirror {
                    upstream: "https://registry-1.docker.io".to_string(),
                },
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        match repository.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://registry-1.docker.io/");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(repository.ecosystem(), PackageEcosystem::Oci);
    }

    #[tokio::test]
    async fn creates_a_helm_mirror_with_upstream() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("helm-proxy").unwrap(),
                PackageEcosystem::Helm,
                CreateRepositoryKind::Mirror {
                    upstream: "https://registry-1.docker.io".to_string(),
                },
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        match repository.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://registry-1.docker.io/");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(repository.ecosystem(), PackageEcosystem::Helm);
    }

    #[tokio::test]
    async fn creates_a_conan_forge() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("conan-local").unwrap(),
                PackageEcosystem::Conan,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        assert!(matches!(repository.kind(), RepositoryKind::Forge));
        assert_eq!(repository.ecosystem(), PackageEcosystem::Conan);
    }

    #[tokio::test]
    async fn rejects_a_generic_ecosystem_mirror() {
        let use_case = CreateRepositoryUseCase::new(Arc::new(InMemoryRepositoryStore::default()));

        let result = use_case
            .execute(
                RepositoryName::parse("generic-mirror").unwrap(),
                PackageEcosystem::Generic,
                CreateRepositoryKind::Mirror {
                    upstream: "https://example.invalid/".to_string(),
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

    #[tokio::test]
    async fn creates_an_alloy_from_forge_members() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());
        let forge_id = use_case
            .execute(
                RepositoryName::parse("crates-local").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        let alloy_id = use_case
            .execute(
                RepositoryName::parse("crates-virtual").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Alloy {
                    members: vec![forge_id],
                },
            )
            .await
            .unwrap();

        let alloy = repository_store
            .find_by_id(alloy_id)
            .await
            .unwrap()
            .unwrap();
        match alloy.kind() {
            RepositoryKind::Alloy { members } => assert_eq!(members, &vec![forge_id]),
            other => panic!("expected alloy, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_an_empty_alloy() {
        let use_case = CreateRepositoryUseCase::new(Arc::new(InMemoryRepositoryStore::default()));
        let result = use_case
            .execute(
                RepositoryName::parse("empty-alloy").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Alloy { members: vec![] },
            )
            .await;
        assert!(matches!(result, Err(CreateRepositoryError::EmptyAlloy)));
    }

    #[tokio::test]
    async fn rejects_a_missing_alloy_member() {
        let use_case = CreateRepositoryUseCase::new(Arc::new(InMemoryRepositoryStore::default()));
        let result = use_case
            .execute(
                RepositoryName::parse("broken-alloy").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Alloy {
                    members: vec![RepositoryId::new()],
                },
            )
            .await;
        assert!(matches!(result, Err(CreateRepositoryError::MemberNotFound)));
    }

    #[tokio::test]
    async fn rejects_a_nested_alloy_member() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());
        let forge_id = use_case
            .execute(
                RepositoryName::parse("crates-local").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let inner_alloy = use_case
            .execute(
                RepositoryName::parse("inner-alloy").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Alloy {
                    members: vec![forge_id],
                },
            )
            .await
            .unwrap();

        let result = use_case
            .execute(
                RepositoryName::parse("outer-alloy").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Alloy {
                    members: vec![inner_alloy],
                },
            )
            .await;
        assert!(matches!(result, Err(CreateRepositoryError::NestedAlloy)));
    }

    #[tokio::test]
    async fn rejects_an_alloy_member_from_another_ecosystem() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store);
        let npm_id = use_case
            .execute(
                RepositoryName::parse("npm-local").unwrap(),
                PackageEcosystem::Npm,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        let result = use_case
            .execute(
                RepositoryName::parse("cargo-alloy").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Alloy {
                    members: vec![npm_id],
                },
            )
            .await;
        assert!(matches!(
            result,
            Err(CreateRepositoryError::MemberEcosystemMismatch)
        ));
    }
}
