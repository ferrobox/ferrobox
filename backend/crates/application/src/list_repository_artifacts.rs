use std::collections::HashMap;
use std::sync::Arc;

use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

/// Artefacto listado junto con el nombre y la versión de paquete, si el
/// índice de su ecosistema los conoce (por ejemplo, un crate publicado
/// con `cargo publish`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedArtifact {
    artifact: Artifact,
    package_name: Option<String>,
    package_version: Option<String>,
    yanked: bool,
    filename: Option<String>,
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

    /// `true` si el índice marca esta versión como *yanked*.
    #[must_use]
    pub fn yanked(&self) -> bool {
        self.yanked
    }

    /// Nombre de fichero en el índice (receta `Conan`, sdist de `PyPI`, etc.).
    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
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

    /// Fallo al consultar el almacén de repositorios (p. ej. al
    /// resolver los miembros de un `Alloy`).
    #[error(transparent)]
    RepositoryPersistence(#[from] RepositoryStoreError),
}

/// Caso de uso: listar los artefactos de un repositorio.
#[allow(clippy::struct_field_names)]
pub struct ListRepositoryArtifactsUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
}

impl ListRepositoryArtifactsUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            package_index_store,
        }
    }

    /// Lista los artefactos del repositorio indicado, enriquecidos con
    /// nombre y versión cuando el índice de paquetes los conoce. Si el
    /// repositorio es un `Alloy`, une los artefactos de sus miembros.
    ///
    /// # Errors
    ///
    /// Devuelve [`ListRepositoryArtifactsError`] si falla un puerto.
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<ListedArtifact>, ListRepositoryArtifactsError> {
        let ids = match self.repository_store.find_by_id(repository_id).await? {
            Some(repository) => match repository.kind() {
                RepositoryKind::Alloy { members } => members.clone(),
                _ => vec![repository_id],
            },
            None => vec![repository_id],
        };

        let mut listed = Vec::new();
        for id in ids {
            let hide_unindexed = self
                .repository_store
                .find_by_id(id)
                .await?
                .is_some_and(|repository| repository.ecosystem() != PackageEcosystem::Generic);
            listed.extend(self.list_one(id, hide_unindexed).await?);
        }
        sort_listed(&mut listed);
        Ok(listed)
    }

    async fn list_one(
        &self,
        repository_id: RepositoryId,
        hide_unindexed: bool,
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
            apply_index_item(&mut names_by_artifact, &item);
        }

        Ok(artifacts
            .into_iter()
            .filter_map(|artifact| {
                let meta = names_by_artifact.remove(&artifact.id());
                if hide_unindexed && meta.is_none() {
                    return None;
                }
                Some(ListedArtifact {
                    artifact,
                    package_name: meta.as_ref().map(|item| item.name.clone()),
                    package_version: meta.as_ref().map(|item| item.version.clone()),
                    yanked: meta.as_ref().is_some_and(|item| item.yanked),
                    filename: meta.and_then(|item| item.filename),
                })
            })
            .collect())
    }
}

struct ArtifactIndexMeta {
    name: String,
    version: String,
    yanked: bool,
    filename: Option<String>,
}

#[derive(Deserialize)]
struct IndexEntryMeta {
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    files: Vec<IndexFileMeta>,
}

#[derive(Deserialize)]
struct IndexFileMeta {
    artifact_id: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    filename: Option<String>,
}

fn apply_index_item(
    names_by_artifact: &mut HashMap<ArtifactId, ArtifactIndexMeta>,
    item: &ferrobox_ports::package_index_store::IndexedArtifact,
) {
    let name = item.coordinate.name().as_str().to_owned();
    let version = item.coordinate.version().as_str().to_owned();
    let meta = serde_json::from_slice::<IndexEntryMeta>(&item.entry).unwrap_or(IndexEntryMeta {
        yanked: false,
        files: Vec::new(),
    });

    let mut mapped_file = false;
    for file in &meta.files {
        if let Ok(uuid) = Uuid::parse_str(&file.artifact_id) {
            let filename = file
                .filename
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            names_by_artifact.insert(
                ArtifactId::from(uuid),
                ArtifactIndexMeta {
                    name: name.clone(),
                    version: version.clone(),
                    yanked: meta.yanked || file.yanked,
                    filename,
                },
            );
            mapped_file = true;
        }
    }

    if !mapped_file {
        names_by_artifact
            .entry(item.artifact_id)
            .or_insert_with(|| ArtifactIndexMeta {
                name,
                version,
                yanked: meta.yanked,
                filename: None,
            });
    }
}

fn sort_listed(listed: &mut [ListedArtifact]) {
    listed.sort_by(|left, right| {
        match (left.package_name(), right.package_name()) {
            (Some(left_name), Some(right_name)) => left_name
                .cmp(right_name)
                .then_with(|| left.package_version().cmp(&right.package_version()))
                .then_with(|| left.filename().cmp(&right.filename())),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left
                .artifact()
                .id()
                .to_string()
                .cmp(&right.artifact().id().to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{
        InMemoryArtifactStore, InMemoryPackageIndexStore, InMemoryRepositoryStore,
    };

    use super::*;

    fn checksum() -> Sha256Checksum {
        Sha256Checksum::parse("a".repeat(64)).unwrap()
    }

    fn use_case(
        repository_store: Arc<InMemoryRepositoryStore>,
        artifact_store: Arc<InMemoryArtifactStore>,
        package_index_store: Arc<InMemoryPackageIndexStore>,
    ) -> ListRepositoryArtifactsUseCase {
        ListRepositoryArtifactsUseCase::new(repository_store, artifact_store, package_index_store)
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

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].artifact(), &matching);
        assert_eq!(result[0].package_name(), None);
    }

    #[tokio::test]
    async fn hides_unindexed_artifacts_in_package_ecosystems() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("crates").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repository_store.save(&repository).await.unwrap();
        let leftover = Artifact::new(repository.id(), checksum(), 4);
        artifact_store.save(&leftover).await.unwrap();

        let result = use_case(repository_store, artifact_store, package_index_store)
            .execute(repository.id())
            .await
            .unwrap();
        assert!(result.is_empty());
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

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result[0].package_name(), Some("demo-ferrobox"));
        assert_eq!(result[0].package_version(), Some("0.1.0"));
        assert!(!result[0].yanked());
    }

    #[tokio::test]
    async fn attaches_yanked_from_the_index_entry() {
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
                Bytes::from_static(br#"{"yanked":true}"#),
            )
            .await
            .unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert!(result[0].yanked());
    }

    #[tokio::test]
    async fn unions_artifacts_from_alloy_members() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());

        let first = Repository::new(
            RepositoryName::parse("crates-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let second = Repository::new(
            RepositoryName::parse("crates-other").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first.id(), second.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repository_store.save(&first).await.unwrap();
        repository_store.save(&second).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        let from_first = Artifact::new(first.id(), checksum(), 10);
        let from_second = Artifact::new(second.id(), checksum(), 20);
        artifact_store.save(&from_first).await.unwrap();
        artifact_store.save(&from_second).await.unwrap();

        package_index_store
            .upsert_entry(
                first.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("alpha").unwrap(),
                    PackageVersion::parse("1.0.0").unwrap(),
                ),
                Some(from_first.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();
        package_index_store
            .upsert_entry(
                second.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("beta").unwrap(),
                    PackageVersion::parse("2.0.0").unwrap(),
                ),
                Some(from_second.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let result = use_case(repository_store, artifact_store, package_index_store)
            .execute(alloy.id())
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].package_name(), Some("alpha"));
        assert_eq!(result[0].artifact().repository_id(), first.id());
        assert_eq!(result[1].package_name(), Some("beta"));
        assert_eq!(result[1].artifact().repository_id(), second.id());
    }

    #[tokio::test]
    async fn attaches_names_from_every_pypi_file_in_the_index_entry() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let sdist = Artifact::new(repository_id, checksum(), 10);
        let wheel = Artifact::new(repository_id, checksum(), 20);
        artifact_store.save(&sdist).await.unwrap();
        artifact_store.save(&wheel).await.unwrap();

        let entry = serde_json::json!({
            "name": "demo-pypi",
            "version": "1.0.0",
            "yanked": true,
            "files": [
                {
                    "filename": "demo_pypi-1.0.0.tar.gz",
                    "sha256": "aa",
                    "artifact_id": sdist.id().to_string(),
                    "yanked": true
                },
                {
                    "filename": "demo_pypi-1.0.0-py3-none-any.whl",
                    "sha256": "bb",
                    "artifact_id": wheel.id().to_string(),
                    "yanked": true
                }
            ]
        });

        package_index_store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::PyPi,
                    PackageName::parse("demo-pypi").unwrap(),
                    PackageVersion::parse("1.0.0").unwrap(),
                ),
                Some(wheel.id()),
                Bytes::from(entry.to_string()),
            )
            .await
            .unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|item| item.package_name() == Some("demo-pypi")
                    && item.package_version() == Some("1.0.0")
                    && item.yanked())
        );
        let filenames: Vec<_> = result.iter().filter_map(ListedArtifact::filename).collect();
        assert!(filenames.contains(&"demo_pypi-1.0.0.tar.gz"));
        assert!(filenames.contains(&"demo_pypi-1.0.0-py3-none-any.whl"));
    }

    #[tokio::test]
    async fn attaches_conan_recipe_filenames_from_the_index_entry() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let recipe = Artifact::new(repository_id, checksum(), 80);
        let manifest = Artifact::new(repository_id, checksum(), 20);
        artifact_store.save(&recipe).await.unwrap();
        artifact_store.save(&manifest).await.unwrap();

        let entry = serde_json::json!({
            "name": "hello",
            "version": "0.1",
            "user": "_",
            "channel": "_",
            "yanked": false,
            "files": [
                {
                    "filename": "conanfile.py",
                    "artifact_id": recipe.id().to_string()
                },
                {
                    "filename": "conanmanifest.txt",
                    "artifact_id": manifest.id().to_string()
                }
            ]
        });

        package_index_store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Conan,
                    PackageName::parse("hello").unwrap(),
                    PackageVersion::parse("0.1@_:_").unwrap(),
                ),
                Some(manifest.id()),
                Bytes::from(entry.to_string()),
            )
            .await
            .unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|item| item.package_name() == Some("hello")
            && item.package_version() == Some("0.1@_:_")
            && !item.yanked()));
        let filenames: Vec<_> = result.iter().filter_map(ListedArtifact::filename).collect();
        assert!(filenames.contains(&"conanfile.py"));
        assert!(filenames.contains(&"conanmanifest.txt"));
    }
}
