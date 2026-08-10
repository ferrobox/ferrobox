use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};

#[derive(Default)]
pub(crate) struct InMemoryRepositoryStore {
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

#[derive(Default)]
pub(crate) struct InMemoryArtifactStore {
    artifacts: Mutex<HashMap<ArtifactId, Artifact>>,
}

#[async_trait]
impl ArtifactStore for InMemoryArtifactStore {
    async fn save(&self, artifact: &Artifact) -> Result<(), ArtifactStoreError> {
        self.artifacts
            .lock()
            .unwrap()
            .insert(artifact.id(), artifact.clone());
        Ok(())
    }

    async fn find_by_id(&self, id: ArtifactId) -> Result<Option<Artifact>, ArtifactStoreError> {
        Ok(self.artifacts.lock().unwrap().get(&id).cloned())
    }

    async fn find_by_repository_id(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Artifact>, ArtifactStoreError> {
        Ok(self
            .artifacts
            .lock()
            .unwrap()
            .values()
            .filter(|artifact| artifact.repository_id() == repository_id)
            .cloned()
            .collect())
    }

    async fn delete(&self, id: ArtifactId) -> Result<(), ArtifactStoreError> {
        self.artifacts.lock().unwrap().remove(&id);
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct InMemoryStorage {
    objects: Mutex<HashMap<StorageKey, Bytes>>,
}

#[async_trait]
impl StoragePort for InMemoryStorage {
    async fn put(&self, key: &StorageKey, content: Bytes) -> Result<(), StorageError> {
        self.objects.lock().unwrap().insert(key.clone(), content);
        Ok(())
    }

    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError> {
        self.objects
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(key.clone()))
    }

    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }

    async fn exists(&self, key: &StorageKey) -> Result<bool, StorageError> {
        Ok(self.objects.lock().unwrap().contains_key(key))
    }
}

/// Doble en memoria de [`PackageIndexStore`]. Usa un `Vec`, no un
/// `HashMap`, deliberadamente: preserva el orden de publicación, tal y
/// como exige el contrato del puerto y tal y como lo garantiza el
/// adaptador real (`ORDER BY created_at`).
#[derive(Default)]
pub(crate) struct InMemoryPackageIndexStore {
    entries: Mutex<Vec<(RepositoryId, PackageCoordinate, ArtifactId, Bytes)>>,
}

#[async_trait]
impl PackageIndexStore for InMemoryPackageIndexStore {
    async fn upsert_entry(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
        artifact_id: ArtifactId,
        entry: Bytes,
    ) -> Result<(), PackageIndexStoreError> {
        let mut entries = self.entries.lock().unwrap();

        if let Some(existing) = entries
            .iter_mut()
            .find(|(repo_id, existing_coordinate, ..)| {
                *repo_id == repository_id && existing_coordinate == coordinate
            })
        {
            existing.2 = artifact_id;
            existing.3 = entry;
        } else {
            entries.push((repository_id, coordinate.clone(), artifact_id, entry));
        }

        Ok(())
    }

    async fn entries_for_package(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &PackageName,
    ) -> Result<Vec<Bytes>, PackageIndexStoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(repo_id, coordinate, ..)| {
                *repo_id == repository_id
                    && coordinate.ecosystem() == ecosystem
                    && coordinate.name() == name
            })
            .map(|(.., entry)| entry.clone())
            .collect())
    }

    async fn artifact_for(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<ArtifactId>, PackageIndexStoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|(repo_id, existing_coordinate, ..)| {
                *repo_id == repository_id && existing_coordinate == coordinate
            })
            .map(|(_, _, artifact_id, _)| *artifact_id))
    }
}

pub(crate) fn forge(name: &str) -> Repository {
    Repository::new(
        RepositoryName::parse(name).unwrap(),
        RepositoryKind::Forge,
        PackageEcosystem::Generic,
    )
    .unwrap()
}
