use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};

#[derive(Default)]
pub(crate) struct InMemoryRepositoryStore {
    repositories: Mutex<HashMap<RepositoryId, Repository>>,
}

#[async_trait]
impl RepositoryStore for InMemoryRepositoryStore {
    async fn save(&self, repository: &Repository) -> Result<(), RepositoryStoreError> {
        self.repositories
            .lock()
            .unwrap()
            .insert(repository.id(), repository.clone());
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

pub(crate) fn forge(name: &str) -> Repository {
    Repository::new(RepositoryName::parse(name).unwrap(), RepositoryKind::Forge).unwrap()
}
