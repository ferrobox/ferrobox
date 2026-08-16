//! Dobles en memoria de los puertos, para tests de aplicación y HTTP.
#![allow(missing_docs, clippy::missing_panics_doc, clippy::must_use_candidate)]

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::assay::Assay;
use ferrobox_domain::ids::{ApiTokenId, ArtifactId, AssayId, RepositoryId, UserId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_domain::retention::RetentionPolicy;
use ferrobox_domain::user::{Role, User, Username};
use ferrobox_ports::api_token_store::{ApiTokenRecord, ApiTokenStore, ApiTokenStoreError};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::assay_store::{AssayStore, AssayStoreError};
use ferrobox_ports::http_client::{HttpClient, HttpClientError, HttpResponse};
use ferrobox_ports::package_index_store::{
    IndexedArtifact, PackageIndexRecord, PackageIndexStore, PackageIndexStoreError,
};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::retention_store::{RetentionStore, RetentionStoreError};
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};
use ferrobox_ports::user_store::{UserStore, UserStoreError};

#[derive(Default)]
pub struct InMemoryRepositoryStore {
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
        Ok(self
            .repositories
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect())
    }

    async fn delete(&self, id: RepositoryId) -> Result<(), RepositoryStoreError> {
        self.repositories.lock().unwrap().remove(&id);
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryArtifactStore {
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
pub struct InMemoryStorage {
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
pub struct InMemoryPackageIndexStore {
    entries: Mutex<Vec<IndexRow>>,
}

struct IndexRow {
    repository_id: RepositoryId,
    coordinate: PackageCoordinate,
    artifact_id: Option<ArtifactId>,
    entry: Bytes,
    created_at_rfc3339: String,
}

impl InMemoryPackageIndexStore {
    pub fn upsert_entry_at(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
        artifact_id: Option<ArtifactId>,
        entry: Bytes,
        created_at_rfc3339: &str,
    ) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(existing) = entries.iter_mut().find(|row| {
            row.repository_id == repository_id && row.coordinate == *coordinate
        }) {
            if artifact_id.is_some() {
                existing.artifact_id = artifact_id;
            }
            existing.entry = entry;
            existing.created_at_rfc3339 = created_at_rfc3339.to_string();
        } else {
            entries.push(IndexRow {
                repository_id,
                coordinate: coordinate.clone(),
                artifact_id,
                entry,
                created_at_rfc3339: created_at_rfc3339.to_string(),
            });
        }
    }
}

#[async_trait]
impl PackageIndexStore for InMemoryPackageIndexStore {
    async fn upsert_entry(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
        artifact_id: Option<ArtifactId>,
        entry: Bytes,
    ) -> Result<(), PackageIndexStoreError> {
        let created_at = format!("2026-01-01T00:00:{:02}Z", {
            let len = self.entries.lock().unwrap().len();
            u32::try_from(len).unwrap_or(0).min(59)
        });
        self.upsert_entry_at(repository_id, coordinate, artifact_id, entry, &created_at);
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
            .filter(|row| {
                row.repository_id == repository_id
                    && row.coordinate.ecosystem() == ecosystem
                    && row.coordinate.name() == name
            })
            .map(|row| row.entry.clone())
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
            .find(|row| row.repository_id == repository_id && row.coordinate == *coordinate)
            .and_then(|row| row.artifact_id))
    }

    async fn delete_by_artifact(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<(), PackageIndexStoreError> {
        self.entries
            .lock()
            .unwrap()
            .retain(|row| row.artifact_id != Some(artifact_id));
        Ok(())
    }

    async fn delete_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<(), PackageIndexStoreError> {
        self.entries
            .lock()
            .unwrap()
            .retain(|row| row.repository_id != repository_id);
        Ok(())
    }

    async fn entries_for_repository(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
    ) -> Result<Vec<Bytes>, PackageIndexStoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|row| {
                row.repository_id == repository_id && row.coordinate.ecosystem() == ecosystem
            })
            .map(|row| row.entry.clone())
            .collect())
    }

    async fn find_indexed_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<IndexedArtifact>, PackageIndexStoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter_map(|row| {
                if row.repository_id != repository_id {
                    return None;
                }
                row.artifact_id.map(|artifact_id| IndexedArtifact {
                    artifact_id,
                    coordinate: row.coordinate.clone(),
                    entry: row.entry.clone(),
                })
            })
            .collect())
    }

    async fn list_entries(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<PackageIndexRecord>, PackageIndexStoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.repository_id == repository_id)
            .map(|row| PackageIndexRecord {
                artifact_id: row.artifact_id,
                coordinate: row.coordinate.clone(),
                entry: row.entry.clone(),
                created_at_rfc3339: row.created_at_rfc3339.clone(),
            })
            .collect())
    }

    async fn delete_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<(), PackageIndexStoreError> {
        self.entries
            .lock()
            .unwrap()
            .retain(|row| !(row.repository_id == repository_id && row.coordinate == *coordinate));
        Ok(())
    }
}

pub fn forge(name: &str) -> Repository {
    Repository::new(
        RepositoryName::parse(name).unwrap(),
        RepositoryKind::Forge,
        PackageEcosystem::Generic,
    )
    .unwrap()
}

#[derive(Default)]
pub struct InMemoryUserStore {
    users: Mutex<HashMap<UserId, (User, String)>>,
}

#[async_trait]
impl UserStore for InMemoryUserStore {
    async fn save_with_password_hash(
        &self,
        user: &User,
        password_hash: &str,
    ) -> Result<(), UserStoreError> {
        let mut users = self.users.lock().unwrap();

        let name_taken_by_another = users.values().any(|(existing, _)| {
            existing.id() != user.id() && existing.username() == user.username()
        });

        if name_taken_by_another {
            return Err(UserStoreError::DuplicateUsername(user.username().clone()));
        }

        users.insert(user.id(), (user.clone(), password_hash.to_string()));
        Ok(())
    }

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, UserStoreError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .get(&id)
            .map(|(user, _)| user.clone()))
    }

    async fn find_by_username_with_password_hash(
        &self,
        username: &Username,
    ) -> Result<Option<(User, String)>, UserStoreError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .values()
            .find(|(user, _)| user.username() == username)
            .map(|(user, hash)| (user.clone(), hash.clone())))
    }

    async fn find_all(&self) -> Result<Vec<User>, UserStoreError> {
        let mut users: Vec<_> = self
            .users
            .lock()
            .unwrap()
            .values()
            .map(|(user, _)| user.clone())
            .collect();
        users.sort_by(|a, b| a.username().as_str().cmp(b.username().as_str()));
        Ok(users)
    }

    async fn delete(&self, id: UserId) -> Result<bool, UserStoreError> {
        Ok(self.users.lock().unwrap().remove(&id).is_some())
    }

    async fn update_role(&self, id: UserId, role: Role) -> Result<bool, UserStoreError> {
        let mut users = self.users.lock().unwrap();
        let Some((user, hash)) = users.remove(&id) else {
            return Ok(false);
        };
        users.insert(id, (user.with_role(role), hash));
        Ok(true)
    }

    async fn count(&self) -> Result<u64, UserStoreError> {
        Ok(self.users.lock().unwrap().len() as u64)
    }

    async fn count_admins(&self) -> Result<u64, UserStoreError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .values()
            .filter(|(user, _)| user.role() == Role::Admin)
            .count() as u64)
    }
}

#[derive(Default)]
pub struct InMemoryApiTokenStore {
    tokens: Mutex<HashMap<ApiTokenId, (ApiToken, String, String)>>,
}

#[async_trait]
impl ApiTokenStore for InMemoryApiTokenStore {
    async fn save(&self, token: &ApiToken, token_hash: &str) -> Result<(), ApiTokenStoreError> {
        self.tokens.lock().unwrap().insert(
            token.id(),
            (
                token.clone(),
                token_hash.to_string(),
                "2026-01-01T00:00:00Z".to_string(),
            ),
        );
        Ok(())
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiToken>, ApiTokenStoreError> {
        Ok(self
            .tokens
            .lock()
            .unwrap()
            .values()
            .find(|(_, hash, _)| hash == token_hash)
            .map(|(token, _, _)| token.clone()))
    }

    async fn list_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApiTokenRecord>, ApiTokenStoreError> {
        Ok(self
            .tokens
            .lock()
            .unwrap()
            .values()
            .filter(|(token, _, _)| token.user_id() == user_id)
            .map(|(token, _, created_at)| ApiTokenRecord {
                token: token.clone(),
                created_at_rfc3339: created_at.clone(),
            })
            .collect())
    }

    async fn delete_for_user(
        &self,
        token_id: ApiTokenId,
        user_id: UserId,
    ) -> Result<bool, ApiTokenStoreError> {
        let mut tokens = self.tokens.lock().unwrap();
        match tokens.get(&token_id) {
            Some((token, _, _)) if token.user_id() == user_id => {
                tokens.remove(&token_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

#[derive(Default)]
pub struct InMemoryHttpClient {
    responses: Mutex<HashMap<String, VecDeque<HttpResponse>>>,
}

impl InMemoryHttpClient {
    pub fn stub(&self, url: &str, status: u16, body: impl Into<Bytes>) {
        self.stub_response(url, HttpResponse::new(status, body.into()));
    }

    pub fn stub_response(&self, url: &str, response: HttpResponse) {
        self.stub_sequence(url, vec![response]);
    }

    pub fn stub_sequence(&self, url: &str, responses: Vec<HttpResponse>) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.to_string(), VecDeque::from(responses));
    }
}

#[async_trait]
impl HttpClient for InMemoryHttpClient {
    async fn get(&self, url: &str) -> Result<HttpResponse, HttpClientError> {
        let response = self.get_with_headers(url, &[]).await?;
        if response.is_success() {
            Ok(response)
        } else {
            Err(HttpClientError::Status {
                status: response.status,
                url: url.to_string(),
            })
        }
    }

    async fn get_with_headers(
        &self,
        url: &str,
        _headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpClientError> {
        let mut responses = self.responses.lock().unwrap();
        let Some(queue) = responses.get_mut(url) else {
            return Err(HttpClientError::Status {
                status: 404,
                url: url.to_string(),
            });
        };
        if queue.is_empty() {
            return Err(HttpClientError::Status {
                status: 404,
                url: url.to_string(),
            });
        }
        if queue.len() == 1 {
            return Ok(queue[0].clone());
        }
        Ok(queue.pop_front().expect("queue length was checked"))
    }

    async fn post(
        &self,
        url: &str,
        _body: Bytes,
        _content_type: &str,
    ) -> Result<HttpResponse, HttpClientError> {
        self.get(url).await
    }
}

#[derive(Default)]
pub struct InMemoryAssayStore {
    assays: Mutex<HashMap<AssayId, Assay>>,
}

#[async_trait]
impl AssayStore for InMemoryAssayStore {
    async fn upsert(&self, assay: &Assay) -> Result<(), AssayStoreError> {
        let mut assays = self.assays.lock().unwrap();
        assays.retain(|_, existing| {
            !(existing.repository_id() == assay.repository_id()
                && existing.coordinate() == assay.coordinate())
        });
        assays.insert(assay.id(), assay.clone());
        Ok(())
    }

    async fn find_by_id(&self, id: AssayId) -> Result<Option<Assay>, AssayStoreError> {
        Ok(self.assays.lock().unwrap().get(&id).cloned())
    }

    async fn find_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<Assay>, AssayStoreError> {
        Ok(self
            .assays
            .lock()
            .unwrap()
            .values()
            .find(|assay| {
                assay.repository_id() == repository_id && assay.coordinate() == coordinate
            })
            .cloned())
    }

    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Assay>, AssayStoreError> {
        Ok(self
            .assays
            .lock()
            .unwrap()
            .values()
            .filter(|assay| assay.repository_id() == repository_id)
            .cloned()
            .collect())
    }

    async fn find_all(&self) -> Result<Vec<Assay>, AssayStoreError> {
        Ok(self.assays.lock().unwrap().values().cloned().collect())
    }

    async fn delete_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<(), AssayStoreError> {
        self.assays.lock().unwrap().retain(|_, assay| {
            !(assay.repository_id() == repository_id && assay.coordinate() == coordinate)
        });
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryRetentionStore {
    policies: Mutex<HashMap<RepositoryId, RetentionPolicy>>,
}

#[async_trait]
impl RetentionStore for InMemoryRetentionStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<RetentionPolicy, RetentionStoreError> {
        Ok(self
            .policies
            .lock()
            .unwrap()
            .get(&repository_id)
            .copied()
            .unwrap_or_else(RetentionPolicy::keep_all))
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<(), RetentionStoreError> {
        self.policies.lock().unwrap().insert(repository_id, policy);
        Ok(())
    }
}
