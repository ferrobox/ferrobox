//! Dobles en memoria de los puertos, para tests de aplicación y HTTP.
#![allow(missing_docs, clippy::missing_panics_doc, clippy::must_use_candidate)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::admission::{AdmissionEvent, AdmissionPolicy};
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::assay::Assay;
use ferrobox_domain::audit::AuditEvent;
use ferrobox_domain::group::Group;
use ferrobox_domain::group::GroupName;
use ferrobox_domain::ids::{
    ApiTokenId, ArtifactId, AssayId, GroupId, RepositoryId, UserId, WebhookId,
};
use ferrobox_domain::oidc::OidcIdentity;
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use ferrobox_domain::quota::StorageQuota;
use ferrobox_domain::replica::ReplicaPolicy;
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_domain::retention::RetentionPolicy;
use ferrobox_domain::user::{Email, Role, User, Username};
use ferrobox_domain::webhook::{Webhook, WebhookDelivery};
use ferrobox_domain::worm::WormPolicy;
use ferrobox_ports::admission_store::{AdmissionRecord, AdmissionStore, AdmissionStoreError};
use ferrobox_ports::api_token_store::{ApiTokenRecord, ApiTokenStore, ApiTokenStoreError};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::assay_store::{AssayStore, AssayStoreError};
use ferrobox_ports::audit_store::{AuditStore, AuditStoreError};
use ferrobox_ports::group_store::{GroupStore, GroupStoreError, RepositoryGroupGrant};
use ferrobox_ports::http_client::{HttpClient, HttpClientError, HttpResponse};
use ferrobox_ports::package_index_store::{
    IndexedArtifact, PackageIndexRecord, PackageIndexStore, PackageIndexStoreError,
};
use ferrobox_ports::quota_store::{QuotaStore, QuotaStoreError};
use ferrobox_ports::replica_store::{ReplicaStore, ReplicaStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::retention_store::{RetentionStore, RetentionStoreError};
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use ferrobox_ports::webhook_store::{WebhookStore, WebhookStoreError};
use ferrobox_ports::worm_store::{WormStore, WormStoreError};

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

    async fn total_size_bytes(&self) -> Result<u64, ArtifactStoreError> {
        Ok(self
            .artifacts
            .lock()
            .unwrap()
            .values()
            .map(Artifact::size_bytes)
            .sum())
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
        if let Some(existing) = entries
            .iter_mut()
            .find(|row| row.repository_id == repository_id && row.coordinate == *coordinate)
        {
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

        if let Some(email) = user.email() {
            let email_taken_by_another = users
                .values()
                .any(|(existing, _)| existing.id() != user.id() && existing.email() == Some(email));
            if email_taken_by_another {
                return Err(UserStoreError::DuplicateEmail(email.clone()));
            }
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

    async fn find_by_email(&self, email: &Email) -> Result<Option<User>, UserStoreError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .values()
            .find(|(user, _)| user.email() == Some(email))
            .map(|(user, _)| user.clone()))
    }

    async fn find_by_oidc(&self, identity: &OidcIdentity) -> Result<Option<User>, UserStoreError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .values()
            .find(|(user, _)| user.oidc() == Some(identity))
            .map(|(user, _)| user.clone()))
    }

    async fn find_by_id_with_password_hash(
        &self,
        id: UserId,
    ) -> Result<Option<(User, String)>, UserStoreError> {
        Ok(self.users.lock().unwrap().get(&id).cloned())
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
                expires_at_rfc3339: token.expires_at().map(|at| at.to_rfc3339()),
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

#[derive(Debug, Clone)]
pub struct RecordedHttpPost {
    pub url: String,
    pub body: Bytes,
    pub headers: Vec<(String, String)>,
}

#[derive(Default)]
pub struct InMemoryHttpClient {
    responses: Mutex<HashMap<String, VecDeque<HttpResponse>>>,
    posts: Mutex<Vec<RecordedHttpPost>>,
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

    pub fn take_posts(&self) -> Vec<RecordedHttpPost> {
        std::mem::take(&mut *self.posts.lock().unwrap())
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
        body: Bytes,
        content_type: &str,
    ) -> Result<HttpResponse, HttpClientError> {
        self.post_with_headers(url, body, &[("content-type", content_type)])
            .await
            .and_then(|response| {
                if response.is_success() {
                    Ok(response)
                } else {
                    Err(HttpClientError::Status {
                        status: response.status,
                        url: url.to_string(),
                    })
                }
            })
    }

    async fn post_with_headers(
        &self,
        url: &str,
        body: Bytes,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpClientError> {
        self.posts.lock().unwrap().push(RecordedHttpPost {
            url: url.to_string(),
            body: body.clone(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
        });
        self.get_with_headers(url, headers).await
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
pub struct InMemoryAdmissionStore {
    policies: Mutex<HashMap<RepositoryId, AdmissionPolicy>>,
    keys: Mutex<HashMap<RepositoryId, String>>,
    events: Mutex<HashMap<RepositoryId, Vec<AdmissionEvent>>>,
}

#[async_trait]
impl AdmissionStore for InMemoryAdmissionStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<AdmissionRecord, AdmissionStoreError> {
        let policy = self
            .policies
            .lock()
            .unwrap()
            .get(&repository_id)
            .cloned()
            .unwrap_or_else(AdmissionPolicy::inactive);
        let public_keys_pem = self
            .keys
            .lock()
            .unwrap()
            .get(&repository_id)
            .cloned()
            .unwrap_or_default();
        Ok(AdmissionRecord {
            policy,
            public_keys_pem,
        })
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
        public_keys_pem: &str,
    ) -> Result<(), AdmissionStoreError> {
        self.policies.lock().unwrap().insert(repository_id, policy);
        self.keys
            .lock()
            .unwrap()
            .insert(repository_id, public_keys_pem.to_string());
        Ok(())
    }

    async fn record_event(&self, event: &AdmissionEvent) -> Result<(), AdmissionStoreError> {
        let mut events = self.events.lock().unwrap();
        let list = events.entry(event.repository_id()).or_default();
        list.insert(0, event.clone());
        list.truncate(50);
        Ok(())
    }

    async fn list_events(
        &self,
        repository_id: RepositoryId,
        limit: usize,
    ) -> Result<Vec<AdmissionEvent>, AdmissionStoreError> {
        Ok(self
            .events
            .lock()
            .unwrap()
            .get(&repository_id)
            .map(|events| events.iter().take(limit).cloned().collect())
            .unwrap_or_default())
    }
}

#[derive(Default)]
pub struct InMemoryAuditStore {
    events: Mutex<Vec<AuditEvent>>,
}

#[async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn record(&self, event: &AuditEvent) -> Result<(), AuditStoreError> {
        let mut events = self.events.lock().unwrap();
        events.insert(0, event.clone());
        events.truncate(200);
        Ok(())
    }

    async fn list(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditStoreError> {
        Ok(self
            .events
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .collect())
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

#[derive(Default)]
pub struct InMemoryReplicaStore {
    policies: Mutex<HashMap<RepositoryId, ReplicaPolicy>>,
}

#[async_trait]
impl ReplicaStore for InMemoryReplicaStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPolicy, ReplicaStoreError> {
        Ok(self
            .policies
            .lock()
            .unwrap()
            .get(&repository_id)
            .cloned()
            .unwrap_or_else(ReplicaPolicy::unconfigured))
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: &ReplicaPolicy,
    ) -> Result<(), ReplicaStoreError> {
        let mut policies = self.policies.lock().unwrap();
        if policy.target().is_none() {
            policies.remove(&repository_id);
        } else {
            policies.insert(repository_id, policy.clone());
        }
        Ok(())
    }

    async fn list_all(&self) -> Result<Vec<(RepositoryId, ReplicaPolicy)>, ReplicaStoreError> {
        Ok(self
            .policies
            .lock()
            .unwrap()
            .iter()
            .map(|(id, policy)| (*id, policy.clone()))
            .collect())
    }
}

#[derive(Default)]
pub struct InMemoryQuotaStore {
    quotas: Mutex<HashMap<RepositoryId, StorageQuota>>,
}

#[async_trait]
impl QuotaStore for InMemoryQuotaStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<StorageQuota, QuotaStoreError> {
        Ok(self
            .quotas
            .lock()
            .unwrap()
            .get(&repository_id)
            .copied()
            .unwrap_or_else(StorageQuota::unlimited))
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        quota: StorageQuota,
    ) -> Result<(), QuotaStoreError> {
        self.quotas.lock().unwrap().insert(repository_id, quota);
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryWormStore {
    policies: Mutex<HashMap<RepositoryId, WormPolicy>>,
}

#[async_trait]
impl WormStore for InMemoryWormStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<WormPolicy, WormStoreError> {
        Ok(self
            .policies
            .lock()
            .unwrap()
            .get(&repository_id)
            .copied()
            .unwrap_or_else(WormPolicy::disabled))
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: WormPolicy,
    ) -> Result<(), WormStoreError> {
        self.policies.lock().unwrap().insert(repository_id, policy);
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryGroupStore {
    groups: Mutex<HashMap<GroupId, Group>>,
    members: Mutex<HashMap<GroupId, HashSet<UserId>>>,
    grants: Mutex<Vec<RepositoryGroupGrant>>,
    sso_memberships: Mutex<HashMap<UserId, HashSet<GroupId>>>,
}

#[async_trait]
impl GroupStore for InMemoryGroupStore {
    async fn save(&self, group: &Group) -> Result<(), GroupStoreError> {
        let mut groups = self.groups.lock().unwrap();
        let name_taken = groups
            .values()
            .any(|existing| existing.id() != group.id() && existing.name() == group.name());
        if name_taken {
            return Err(GroupStoreError::DuplicateName(group.name().clone()));
        }
        groups.insert(group.id(), group.clone());
        Ok(())
    }

    async fn find_by_id(&self, id: GroupId) -> Result<Option<Group>, GroupStoreError> {
        Ok(self.groups.lock().unwrap().get(&id).cloned())
    }

    async fn find_by_name(&self, name: &GroupName) -> Result<Option<Group>, GroupStoreError> {
        Ok(self
            .groups
            .lock()
            .unwrap()
            .values()
            .find(|group| group.name() == name)
            .cloned())
    }

    async fn find_all(&self) -> Result<Vec<Group>, GroupStoreError> {
        let mut groups: Vec<_> = self.groups.lock().unwrap().values().cloned().collect();
        groups.sort_by(|a, b| a.name().as_str().cmp(b.name().as_str()));
        Ok(groups)
    }

    async fn delete(&self, id: GroupId) -> Result<bool, GroupStoreError> {
        let removed = self.groups.lock().unwrap().remove(&id).is_some();
        self.members.lock().unwrap().remove(&id);
        self.grants
            .lock()
            .unwrap()
            .retain(|grant| grant.group_id != id);
        for links in self.sso_memberships.lock().unwrap().values_mut() {
            links.remove(&id);
        }
        Ok(removed)
    }

    async fn set_members(
        &self,
        group_id: GroupId,
        user_ids: &[UserId],
    ) -> Result<(), GroupStoreError> {
        self.members
            .lock()
            .unwrap()
            .insert(group_id, user_ids.iter().copied().collect());
        Ok(())
    }

    async fn members(&self, group_id: GroupId) -> Result<Vec<UserId>, GroupStoreError> {
        Ok(self
            .members
            .lock()
            .unwrap()
            .get(&group_id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default())
    }

    async fn groups_for_user(&self, user_id: UserId) -> Result<Vec<GroupId>, GroupStoreError> {
        Ok(self
            .members
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, members)| members.contains(&user_id))
            .map(|(group_id, _)| *group_id)
            .collect())
    }

    async fn add_member(&self, group_id: GroupId, user_id: UserId) -> Result<(), GroupStoreError> {
        self.members
            .lock()
            .unwrap()
            .entry(group_id)
            .or_default()
            .insert(user_id);
        Ok(())
    }

    async fn remove_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<(), GroupStoreError> {
        if let Some(members) = self.members.lock().unwrap().get_mut(&group_id) {
            members.remove(&user_id);
        }
        Ok(())
    }

    async fn sso_memberships(&self, user_id: UserId) -> Result<Vec<GroupId>, GroupStoreError> {
        Ok(self
            .sso_memberships
            .lock()
            .unwrap()
            .get(&user_id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default())
    }

    async fn set_sso_memberships(
        &self,
        user_id: UserId,
        group_ids: &[GroupId],
    ) -> Result<(), GroupStoreError> {
        self.sso_memberships
            .lock()
            .unwrap()
            .insert(user_id, group_ids.iter().copied().collect());
        Ok(())
    }

    async fn set_group_repositories(
        &self,
        group_id: GroupId,
        grants: &[(RepositoryId, Role)],
    ) -> Result<(), GroupStoreError> {
        let mut all = self.grants.lock().unwrap();
        all.retain(|grant| grant.group_id != group_id);
        all.extend(
            grants
                .iter()
                .map(|(repository_id, role)| RepositoryGroupGrant {
                    repository_id: *repository_id,
                    group_id,
                    role: *role,
                }),
        );
        Ok(())
    }

    async fn set_repository_groups(
        &self,
        repository_id: RepositoryId,
        grants: &[(GroupId, Role)],
    ) -> Result<(), GroupStoreError> {
        let mut all = self.grants.lock().unwrap();
        all.retain(|grant| grant.repository_id != repository_id);
        all.extend(grants.iter().map(|(group_id, role)| RepositoryGroupGrant {
            repository_id,
            group_id: *group_id,
            role: *role,
        }));
        Ok(())
    }

    async fn grants_for_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError> {
        Ok(self
            .grants
            .lock()
            .unwrap()
            .iter()
            .filter(|grant| grant.repository_id == repository_id)
            .cloned()
            .collect())
    }

    async fn grants_for_group(
        &self,
        group_id: GroupId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError> {
        Ok(self
            .grants
            .lock()
            .unwrap()
            .iter()
            .filter(|grant| grant.group_id == group_id)
            .cloned()
            .collect())
    }

    async fn all_grants(&self) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError> {
        Ok(self.grants.lock().unwrap().clone())
    }
}

#[derive(Default)]
pub struct InMemoryWebhookStore {
    webhooks: Mutex<HashMap<WebhookId, Webhook>>,
    deliveries: Mutex<HashMap<WebhookId, Vec<WebhookDelivery>>>,
}

#[async_trait]
impl WebhookStore for InMemoryWebhookStore {
    async fn save(&self, webhook: &Webhook) -> Result<(), WebhookStoreError> {
        self.webhooks
            .lock()
            .unwrap()
            .insert(webhook.id(), webhook.clone());
        Ok(())
    }

    async fn find_by_id(&self, id: WebhookId) -> Result<Option<Webhook>, WebhookStoreError> {
        Ok(self.webhooks.lock().unwrap().get(&id).cloned())
    }

    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Webhook>, WebhookStoreError> {
        let mut webhooks: Vec<_> = self
            .webhooks
            .lock()
            .unwrap()
            .values()
            .filter(|webhook| webhook.repository_id() == repository_id)
            .cloned()
            .collect();
        webhooks.sort_by_key(|webhook| std::cmp::Reverse(webhook.id().to_string()));
        Ok(webhooks)
    }

    async fn delete(&self, id: WebhookId) -> Result<bool, WebhookStoreError> {
        self.deliveries.lock().unwrap().remove(&id);
        Ok(self.webhooks.lock().unwrap().remove(&id).is_some())
    }

    async fn record_delivery(&self, delivery: &WebhookDelivery) -> Result<(), WebhookStoreError> {
        let mut deliveries = self.deliveries.lock().unwrap();
        let list = deliveries.entry(delivery.webhook_id()).or_default();
        list.insert(0, delivery.clone());
        list.truncate(20);
        Ok(())
    }

    async fn deliveries(
        &self,
        webhook_id: WebhookId,
        limit: usize,
    ) -> Result<Vec<WebhookDelivery>, WebhookStoreError> {
        Ok(self
            .deliveries
            .lock()
            .unwrap()
            .get(&webhook_id)
            .map(|list| list.iter().take(limit).cloned().collect())
            .unwrap_or_default())
    }
}
