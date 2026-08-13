use std::collections::HashMap;
use std::sync::Arc;

use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use thiserror::Error;

/// Artefacto listado junto con el nombre y la versión de paquete, si el
/// índice de su ecosistema los conoce (por ejemplo, un crate publicado
/// con `cargo publish`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedArtifact {
    artifact: Artifact,
    package_name: Option<String>,
    package_version: Option<String>,
}

impl ListedArtifact {
    /// Metadatos binarios del artefacto.
    #[must_use]
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }

    /// Nombre del paquete indexado, si existe.
    #[must_use]
    pub fn package_name(&self) -> Option<&str> {
        self.package_name.as_deref()
    }

    /// Versión del paquete indexado, si existe.
    #[must_use]
    pub fn package_version(&self) -> Option<&str> {
        self.package_version.as_deref()
    }
}

/// Motivos por los que listar los artefactos de un repositorio puede
/// fallar.
#[derive(Debug, Error)]
pub enum ListRepositoryArtifactsError {
    /// Fallo al consultar el almacén de artefactos.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),

    /// Fallo al consultar el índice de paquetes.
    #[error(transparent)]
    IndexPersistence(#[from] PackageIndexStoreError),
}

/// Caso de uso: listar los artefactos de un repositorio.
pub struct ListRepositoryArtifactsUseCase {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
}

impl ListRepositoryArtifactsUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
        }
    }

    /// Lista los artefactos del repositorio indicado, enriquecidos con
    /// nombre y versión cuando el índice de paquetes los conoce.
    ///
    /// # Errors
    ///
    /// Devuelve [`ListRepositoryArtifactsError`] si falla un puerto.
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<ListedArtifact>, ListRepositoryArtifactsError> {
        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;
        let indexed = self
            .package_index_store
            .find_indexed_by_repository(repository_id)
            .await?;

        let mut names_by_artifact = HashMap::new();
        for item in indexed {
            names_by_artifact.entry(item.artifact_id).or_insert((
                item.coordinate.name().as_str().to_owned(),
                item.coordinate.version().as_str().to_owned(),
            ));
        }

        let mut listed: Vec<ListedArtifact> = artifacts
            .into_iter()
            .map(|artifact| {
                let (package_name, package_version) = names_by_artifact
                    .remove(&artifact.id())
                    .map_or((None, None), |(name, version)| (Some(name), Some(version)));
                ListedArtifact {
                    artifact,
                    package_name,
                    package_version,
                }
            })
            .collect();

        listed.sort_by(|left, right| {
            match (left.package_name(), right.package_name()) {
                (Some(left_name), Some(right_name)) => left_name
                    .cmp(right_name)
                    .then_with(|| left.package_version().cmp(&right.package_version())),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left
                    .artifact()
                    .id()
                    .to_string()
                    .cmp(&right.artifact().id().to_string()),
            }
        });

        Ok(listed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::package_index_store::PackageIndexStore;

    use crate::test_support::{InMemoryArtifactStore, InMemoryPackageIndexStore};

    use super::*;

    fn checksum() -> Sha256Checksum {
        Sha256Checksum::parse("a".repeat(64)).unwrap()
    }

    #[tokio::test]
    async fn lists_only_artifacts_belonging_to_the_repository() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let other_repository_id = RepositoryId::new();

        let matching = Artifact::new(repository_id, checksum(), 10);
        let other = Artifact::new(other_repository_id, checksum(), 20);
        artifact_store.save(&matching).await.unwrap();
        artifact_store.save(&other).await.unwrap();

        let use_case =
            ListRepositoryArtifactsUseCase::new(artifact_store, package_index_store);
        let result = use_case.execute(repository_id).await.unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].artifact(), &matching);
        assert_eq!(result[0].package_name(), None);
    }

    #[tokio::test]
    async fn attaches_package_name_and_version_from_the_index() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let artifact = Artifact::new(repository_id, checksum(), 810);
        artifact_store.save(&artifact).await.unwrap();

        package_index_store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("demo-ferrobox").unwrap(),
                    PackageVersion::parse("0.1.0").unwrap(),
                ),
                Some(artifact.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let use_case =
            ListRepositoryArtifactsUseCase::new(artifact_store, package_index_store);
        let result = use_case.execute(repository_id).await.unwrap();

        assert_eq!(result[0].package_name(), Some("demo-ferrobox"));
        assert_eq!(result[0].package_version(), Some("0.1.0"));
    }
}
