use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;
use url::Url;

use crate::alloy_members::{resolve_alloy_members, ResolveAlloyMembersError};

/// Reasons creating a repository can fail.
#[derive(Debug, Error)]
pub enum CreateRepositoryError {
    /// A `Mirror` is not supported for `generic`.
    #[error(
        "mirror repositories currently require the cargo, npm, pypi, oci, helm, conan, maven, nuget or go ecosystem"
    )]
    UnsupportedMirrorEcosystem,

    /// The *upstream* URL of a `Mirror` is not valid.
    #[error("invalid upstream URL: {0}")]
    InvalidUpstream(String),

    /// An `Alloy` needs at least one member repository.
    #[error("an Alloy repository must aggregate at least one member repository")]
    EmptyAlloy,

    /// One of the given members does not exist.
    #[error("alloy member repository does not exist")]
    MemberNotFound,

    /// A member does not share the `Alloy` ecosystem.
    #[error("alloy members must use the same package ecosystem")]
    MemberEcosystemMismatch,

    /// An `Alloy` cannot aggregate another `Alloy` (avoids cycles).
    #[error("alloy members must be Forge or Mirror repositories")]
    NestedAlloy,

    /// Failed to persist the repository (includes a duplicate-name
    /// case).
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

/// Description of the repository kind to create.
#[derive(Debug, Clone)]
pub enum CreateRepositoryKind {
    /// Own storage.
    Forge,
    /// Cached replica of an *upstream*.
    Mirror {
        /// Base URL of the remote registry (Cargo sparse index, npm
        /// registry, `PyPI` simple index, OCI registry, or Helm charts).
        upstream: String,
    },
    /// Aggregation of other `Forge` or `Mirror` repositories.
    Alloy {
        /// Identifiers of the member repositories, in resolution
        /// order.
        members: Vec<RepositoryId>,
    },
}

/// Use case: create a new repository (`Forge`, `Mirror`, or `Alloy`).
pub struct CreateRepositoryUseCase {
    repository_store: Arc<dyn RepositoryStore>,
}

impl CreateRepositoryUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self { repository_store }
    }

    /// Creates a repository with the given name, ecosystem, and kind.
    ///
    /// # Errors
    ///
    /// Returns [`CreateRepositoryError`] if the kind is not supported,
    /// the *upstream* URL is invalid, an `Alloy` has no valid members,
    /// the name is already in use, or the backend fails.
    ///
    /// # Panics
    ///
    /// In practice this never panics: a `Forge`, a `Mirror` with a
    /// valid *upstream*, and an `Alloy` with at least one member cannot
    /// violate the "Alloy without members" invariant that
    /// `Repository::new` validates.
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
                        | PackageEcosystem::Conan
                        | PackageEcosystem::Maven
                        | PackageEcosystem::Nuget
                        | PackageEcosystem::Go
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
    async fn creates_a_conan_mirror() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let id = use_case
            .execute(
                RepositoryName::parse("conan-center").unwrap(),
                PackageEcosystem::Conan,
                CreateRepositoryKind::Mirror {
                    upstream: "https://center2.conan.io".to_string(),
                },
            )
            .await
            .unwrap();

        let repository = repository_store.find_by_id(id).await.unwrap().unwrap();
        match repository.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://center2.conan.io/");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(repository.ecosystem(), PackageEcosystem::Conan);
    }

    #[tokio::test]
    async fn creates_a_maven_forge_and_mirror() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let forge_id = use_case
            .execute(
                RepositoryName::parse("maven-local").unwrap(),
                PackageEcosystem::Maven,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let forge = repository_store.find_by_id(forge_id).await.unwrap().unwrap();
        assert!(matches!(forge.kind(), RepositoryKind::Forge));
        assert_eq!(forge.ecosystem(), PackageEcosystem::Maven);

        let mirror_id = use_case
            .execute(
                RepositoryName::parse("maven-central").unwrap(),
                PackageEcosystem::Maven,
                CreateRepositoryKind::Mirror {
                    upstream: "https://repo1.maven.org/maven2/".to_string(),
                },
            )
            .await
            .unwrap();
        let mirror = repository_store
            .find_by_id(mirror_id)
            .await
            .unwrap()
            .unwrap();
        match mirror.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://repo1.maven.org/maven2/");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(mirror.ecosystem(), PackageEcosystem::Maven);
    }

    #[tokio::test]
    async fn creates_a_nuget_forge_and_mirror() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let forge_id = use_case
            .execute(
                RepositoryName::parse("nuget-local").unwrap(),
                PackageEcosystem::Nuget,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let forge = repository_store.find_by_id(forge_id).await.unwrap().unwrap();
        assert!(matches!(forge.kind(), RepositoryKind::Forge));
        assert_eq!(forge.ecosystem(), PackageEcosystem::Nuget);

        let mirror_id = use_case
            .execute(
                RepositoryName::parse("nuget-gallery").unwrap(),
                PackageEcosystem::Nuget,
                CreateRepositoryKind::Mirror {
                    upstream: "https://api.nuget.org/v3/index.json".to_string(),
                },
            )
            .await
            .unwrap();
        let mirror = repository_store
            .find_by_id(mirror_id)
            .await
            .unwrap()
            .unwrap();
        match mirror.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(upstream.as_str(), "https://api.nuget.org/v3/index.json");
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(mirror.ecosystem(), PackageEcosystem::Nuget);
    }

    #[tokio::test]
    async fn creates_a_go_forge_and_mirror() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = CreateRepositoryUseCase::new(repository_store.clone());

        let forge_id = use_case
            .execute(
                RepositoryName::parse("go-local").unwrap(),
                PackageEcosystem::Go,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let forge = repository_store.find_by_id(forge_id).await.unwrap().unwrap();
        assert!(matches!(forge.kind(), RepositoryKind::Forge));
        assert_eq!(forge.ecosystem(), PackageEcosystem::Go);

        let mirror_id = use_case
            .execute(
                RepositoryName::parse("go-proxy").unwrap(),
                PackageEcosystem::Go,
                CreateRepositoryKind::Mirror {
                    upstream: "https://proxy.golang.org".to_string(),
                },
            )
            .await
            .unwrap();
        let mirror = repository_store
            .find_by_id(mirror_id)
            .await
            .unwrap()
            .unwrap();
        match mirror.kind() {
            RepositoryKind::Mirror { upstream } => {
                assert_eq!(
                    upstream.as_str().trim_end_matches('/'),
                    "https://proxy.golang.org"
                );
            }
            other => panic!("expected mirror, got {other:?}"),
        }
        assert_eq!(mirror.ecosystem(), PackageEcosystem::Go);
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
