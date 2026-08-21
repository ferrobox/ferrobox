//! Copia una versión publicada (o un binario genérico) de un repositorio
//! `Forge` a otro `Forge` del mismo ecosistema.

use std::sync::Arc;

use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::packaging::{
    copy_stored_artifact, PackagingError, PackagingRegistry, PromoteOutcome as PackagingPromote,
};
use crate::quota::{QuotaError, QuotaService};

/// Resultado de una promoción: recuento de binarios y bytes copiados.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteOutcome {
    /// Nombre de paquete, si el ecosistema indexa por coordenada.
    pub name: Option<String>,
    /// Versión (o etiqueta OCI) copiada.
    pub version: Option<String>,
    /// Binarios nuevos creados en el destino.
    pub artifacts_copied: u32,
    /// Bytes escritos en el almacenamiento del destino.
    pub bytes_copied: u64,
}

/// Motivos por los que promover una versión puede fallar.
#[derive(Debug, Error)]
pub enum PromoteError {
    /// El repositorio de origen no existe.
    #[error("source repository {0} does not exist")]
    SourceNotFound(RepositoryId),

    /// El repositorio de destino no existe.
    #[error("target repository {0} does not exist")]
    TargetNotFound(RepositoryId),

    /// Origen y destino son el mismo repositorio.
    #[error("source and target repositories must be different")]
    SameRepository,

    /// El origen no es un `Forge`.
    #[error("cannot promote from a {0} repository")]
    SourceNotForge(&'static str),

    /// El destino no es un `Forge`.
    #[error("cannot promote into a {0} repository")]
    TargetNotForge(&'static str),

    /// Origen y destino no comparten ecosistema.
    #[error("source ecosystem '{source_ecosystem}' does not match target ecosystem '{target_ecosystem}'")]
    EcosystemMismatch {
        /// Ecosistema del origen.
        source_ecosystem: &'static str,
        /// Ecosistema del destino.
        target_ecosystem: &'static str,
    },

    /// Faltan nombre y versión para un ecosistema indexado.
    #[error("name and version are required to promote this package")]
    MissingCoordinate,

    /// Falta el identificador del binario genérico.
    #[error("artifact_id is required to promote a generic artifact")]
    MissingArtifact,

    /// El binario no existe.
    #[error("artifact {0} was not found")]
    ArtifactNotFound(ArtifactId),

    /// El binario no pertenece al repositorio de origen.
    #[error("artifact {0} does not belong to the source repository")]
    ArtifactRepositoryMismatch(ArtifactId),

    /// Fallo al consultar repositorios.
    #[error(transparent)]
    Repository(#[from] RepositoryStoreError),

    /// Fallo al consultar o persistir artefactos.
    #[error(transparent)]
    Artifact(#[from] ArtifactStoreError),

    /// Fallo al copiar bytes.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// El destino no admite más binarios.
    #[error(transparent)]
    Quota(#[from] QuotaError),

    /// Fallo de la estrategia de empaquetado (versión inexistente, ya
    /// publicada, payload inválido, …).
    #[error(transparent)]
    Packaging(#[from] PackagingError),
}

/// Caso de uso: promover una versión (o un binario genérico) entre Forges.
pub struct PromotePackageUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    storage: Arc<dyn StoragePort>,
    quota: QuotaService,
}

impl PromotePackageUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        storage: Arc<dyn StoragePort>,
        quota: QuotaService,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            storage,
            quota,
        }
    }

    /// Copia la versión `name`/`version` (o el binario `artifact_id` en
    /// un repositorio genérico) de `source_id` a `target_id`.
    ///
    /// # Errors
    ///
    /// Devuelve [`PromoteError`] si los repositorios no son dos Forges
    /// distintos del mismo ecosistema, falta la coordenada, la versión
    /// ya existe en el destino, o falla un puerto.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        packaging: &PackagingRegistry,
        source_id: RepositoryId,
        target_id: RepositoryId,
        name: Option<&str>,
        version: Option<&str>,
        artifact_id: Option<ArtifactId>,
        preserve_yanked: bool,
    ) -> Result<PromoteOutcome, PromoteError> {
        if source_id == target_id {
            return Err(PromoteError::SameRepository);
        }

        let source = self
            .repository_store
            .find_by_id(source_id)
            .await?
            .ok_or(PromoteError::SourceNotFound(source_id))?;
        let target = self
            .repository_store
            .find_by_id(target_id)
            .await?
            .ok_or(PromoteError::TargetNotFound(target_id))?;

        ensure_forge(&source, true)?;
        ensure_forge(&target, false)?;

        if source.ecosystem() != target.ecosystem() {
            return Err(PromoteError::EcosystemMismatch {
                source_ecosystem: source.ecosystem().label(),
                target_ecosystem: target.ecosystem().label(),
            });
        }

        if source.ecosystem() == PackageEcosystem::Generic {
            let artifact_id = artifact_id.ok_or(PromoteError::MissingArtifact)?;
            return self.promote_generic(&source, &target, artifact_id).await;
        }

        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(PromoteError::MissingCoordinate)?;
        let version = version
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(PromoteError::MissingCoordinate)?;
        let package_name = PackageName::parse(name.to_string())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let package_version = PackageVersion::parse(version.to_string())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let coordinate =
            PackageCoordinate::new(source.ecosystem(), package_name, package_version);

        let strategy = packaging.strategy_for(source.ecosystem()).ok_or_else(|| {
            PackagingError::InvalidPayload(format!(
                "no packaging strategy registered for {}",
                source.ecosystem().label()
            ))
        })?;
        let copied = strategy
            .promote_version(&source, &target, &coordinate, preserve_yanked)
            .await?;
        Ok(from_packaging(&copied))
    }

    async fn promote_generic(
        &self,
        source: &Repository,
        target: &Repository,
        artifact_id: ArtifactId,
    ) -> Result<PromoteOutcome, PromoteError> {
        let artifact = self
            .artifact_store
            .find_by_id(artifact_id)
            .await?
            .ok_or(PromoteError::ArtifactNotFound(artifact_id))?;
        if artifact.repository_id() != source.id() {
            return Err(PromoteError::ArtifactRepositoryMismatch(artifact_id));
        }

        let (_copied_id, bytes_copied) = copy_stored_artifact(
            self.artifact_store.as_ref(),
            self.storage.as_ref(),
            Some(&self.quota),
            artifact_id,
            target.id(),
        )
        .await?;
        Ok(PromoteOutcome {
            name: None,
            version: None,
            artifacts_copied: 1,
            bytes_copied,
        })
    }
}

fn from_packaging(outcome: &PackagingPromote) -> PromoteOutcome {
    PromoteOutcome {
        name: Some(outcome.coordinate.name().as_str().to_string()),
        version: Some(outcome.coordinate.version().as_str().to_string()),
        artifacts_copied: outcome.artifacts_copied,
        bytes_copied: outcome.bytes_copied,
    }
}

fn ensure_forge(repository: &Repository, source: bool) -> Result<(), PromoteError> {
    match repository.kind() {
        RepositoryKind::Forge => Ok(()),
        RepositoryKind::Mirror { .. } => Err(if source {
            PromoteError::SourceNotForge("Mirror")
        } else {
            PromoteError::TargetNotForge("Mirror")
        }),
        RepositoryKind::Alloy { .. } => Err(if source {
            PromoteError::SourceNotForge("Alloy")
        } else {
            PromoteError::TargetNotForge("Alloy")
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::packaging::cargo::CargoPackagingStrategy;
    use crate::packaging::{PackagingRegistry, PackagingStrategy};
    use crate::publish_artifact::PublishArtifactUseCase;
    use crate::quota::QuotaService;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryQuotaStore, InMemoryRepositoryStore, InMemoryStorage,
    };

    fn quota(
        repositories: Arc<InMemoryRepositoryStore>,
        artifacts: Arc<InMemoryArtifactStore>,
    ) -> QuotaService {
        QuotaService::new(
            repositories,
            artifacts,
            Arc::new(InMemoryQuotaStore::default()),
        )
    }

    fn forge(name: &str, ecosystem: PackageEcosystem) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            ecosystem,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn copies_a_generic_artifact_between_forges() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("generic-dev", PackageEcosystem::Generic);
        let target = forge("generic-prod", PackageEcosystem::Generic);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let artifact_id = PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota.clone(),
        )
        .execute(source.id(), Bytes::from_static(b"blob-bytes"))
        .await
        .unwrap();

        let use_case = PromotePackageUseCase::new(
            repositories,
            artifacts.clone(),
            storage,
            quota,
        );
        let outcome = use_case
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                target.id(),
                None,
                None,
                Some(artifact_id),
                false,
            )
            .await
            .unwrap();

        assert_eq!(outcome.artifacts_copied, 1);
        assert_eq!(outcome.bytes_copied, 10);
        let copied = artifacts.find_by_repository_id(target.id()).await.unwrap();
        assert_eq!(copied.len(), 1);
        assert_ne!(copied[0].id(), artifact_id);
    }

    #[tokio::test]
    async fn rejects_the_same_repository() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("only", PackageEcosystem::Generic);
        repositories.save(&source).await.unwrap();

        let err = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                source.id(),
                None,
                None,
                None,
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PromoteError::SameRepository));
    }

    #[tokio::test]
    async fn rejects_a_mirror_target() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("crates-dev", PackageEcosystem::Cargo);
        let target = Repository::new(
            RepositoryName::parse("crates-mirror").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.crates.io/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let err = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                target.id(),
                Some("demo"),
                Some("1.0.0"),
                None,
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PromoteError::TargetNotForge("Mirror")));
    }

    #[tokio::test]
    async fn rejects_distinct_ecosystems() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("crates-dev", PackageEcosystem::Cargo);
        let target = forge("npm-prod", PackageEcosystem::Npm);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let err = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &PackagingRegistry::new(),
                source.id(),
                target.id(),
                Some("demo"),
                Some("1.0.0"),
                None,
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PromoteError::EcosystemMismatch { .. }));
    }

    #[tokio::test]
    async fn promotes_a_cargo_crate_through_the_use_case() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quota = quota(repositories.clone(), artifacts.clone());
        let source = forge("crates-dev", PackageEcosystem::Cargo);
        let target = forge("crates-prod", PackageEcosystem::Cargo);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let strategy = CargoPackagingStrategy::new(
            artifacts.clone(),
            index.clone(),
            storage.clone(),
            Arc::new(InMemoryHttpClient::default()),
            repositories.clone(),
        );
        let payload = encode_cargo_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"crate-bytes",
        );
        strategy.publish(&source, payload).await.unwrap();

        let packaging = PackagingRegistry::new().register(Arc::new(strategy));
        let outcome = PromotePackageUseCase::new(repositories, artifacts, storage, quota)
            .execute(
                &packaging,
                source.id(),
                target.id(),
                Some("ferrobox-cli"),
                Some("0.1.0"),
                None,
                false,
            )
            .await
            .unwrap();

        assert_eq!(outcome.name.as_deref(), Some("ferrobox-cli"));
        assert_eq!(outcome.version.as_deref(), Some("0.1.0"));
        assert_eq!(outcome.artifacts_copied, 1);
        assert_eq!(outcome.bytes_copied, 11);
    }

    fn encode_cargo_payload(metadata_json: &str, crate_bytes: &[u8]) -> Bytes {
        let metadata_bytes = metadata_json.as_bytes();
        let mut payload = Vec::new();
        payload.extend_from_slice(&u32::try_from(metadata_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(metadata_bytes);
        payload.extend_from_slice(&u32::try_from(crate_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(crate_bytes);
        Bytes::from(payload)
    }
}
