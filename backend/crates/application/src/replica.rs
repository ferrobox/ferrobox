//! Push or pull a repository to another FerroBox instance.
//!
//! Push: export the catalog and `POST` it to the remote import.
//! Pull: `GET` the remote export and import it here.
//! The background cron runs due policies on an interval.

use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::replica::{
    ReplicaDirection, ReplicaPolicy, ReplicaPolicyError, ReplicaRun, ReplicaTarget,
};
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use ferrobox_ports::replica_store::{ReplicaStore, ReplicaStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::get_repository::{GetRepositoryError, GetRepositoryUseCase};
use crate::repository_bundle::{BundleError, RepositoryBundleService};

/// Recuento que devuelve el import remoto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaPushOutcome {
    /// Coordenadas nuevas en el destino.
    pub packages_imported: u32,
    /// Binarios nuevos en el destino.
    pub artifacts_imported: u32,
    /// Paquetes o binarios que ya estaban.
    pub skipped: u32,
}

/// Motivos por los que configurar o empujar una réplica puede fallar.
#[derive(Debug, Error)]
pub enum ReplicaError {
    /// El repositorio no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// Un `Alloy` no se replica: no guarda binarios propios.
    #[error("replica does not apply to Alloy repositories")]
    AlloyRepository,

    /// Falta la migración SQL.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_replica is missing)"
    )]
    MissingSchema,

    /// No hay destino guardado.
    #[error("replica target is not configured")]
    NotConfigured,

    /// Hay destino pero no hay token.
    #[error("replica token is required")]
    MissingToken,

    /// La política pedida no es válida.
    #[error(transparent)]
    Invalid(#[from] ReplicaPolicyError),

    /// El remoto rechazó el bundle o no respondió.
    #[error("replica failed: {0}")]
    Remote(String),

    /// Fallo al consultar el repositorio.
    #[error(transparent)]
    Repository(#[from] GetRepositoryError),

    /// Fallo al listar repositorios.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),

    /// Fallo al persistir la política.
    #[error(transparent)]
    Store(#[from] ReplicaStoreError),

    /// Fallo al exportar el bundle.
    #[error(transparent)]
    Bundle(#[from] BundleError),
}

/// Use case: replica policy, manual push/pull, and the scheduled cron.
#[derive(Clone)]
pub struct ReplicaService {
    store: Arc<dyn ReplicaStore>,
    repositories: GetRepositoryUseCase,
    bundles: RepositoryBundleService,
    http_client: Arc<dyn HttpClient>,
}

impl ReplicaService {
    /// Construye el servicio a partir de sus puertos.
    #[must_use]
    pub fn new(
        store: Arc<dyn ReplicaStore>,
        repository_store: Arc<dyn RepositoryStore>,
        bundles: RepositoryBundleService,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            store,
            repositories: GetRepositoryUseCase::new(repository_store),
            bundles,
            http_client,
        }
    }

    /// Devuelve la política, o «sin destino» si nunca se configuró.
    ///
    /// # Errors
    ///
    /// [`ReplicaError::AlloyRepository`] o fallo de persistencia.
    pub async fn get_policy(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPolicy, ReplicaError> {
        self.require_pushable(repository_id).await?;
        Ok(self.load_policy(repository_id).await?)
    }

    /// Guarda el destino. Un token vacío conserva el anterior.
    ///
    /// # Errors
    ///
    /// [`ReplicaError::Invalid`] o el repositorio no admite réplica.
    pub async fn save_policy(
        &self,
        repository_id: RepositoryId,
        remote_url: Option<String>,
        destination_id: Option<Uuid>,
        token: Option<String>,
        direction: Option<String>,
        interval_hours: Option<u32>,
    ) -> Result<ReplicaPolicy, ReplicaError> {
        self.require_pushable(repository_id).await?;
        let existing = self.load_policy(repository_id).await?;
        let direction = match direction.as_deref() {
            Some(raw) if !raw.trim().is_empty() => ReplicaDirection::parse(raw)?,
            _ => existing
                .target()
                .map(ReplicaTarget::direction)
                .unwrap_or(ReplicaDirection::Push),
        };
        let interval = match interval_hours {
            Some(hours) => Some(hours),
            None => existing.interval_hours(),
        };
        let target = match (remote_url, destination_id) {
            (Some(url), Some(destination)) if !url.trim().is_empty() => {
                let incoming =
                    ReplicaTarget::new(url, RepositoryId::from(destination), token, repository_id)?
                        .with_direction(direction);
                Some(if incoming.token().is_some() {
                    incoming
                } else {
                    incoming.with_token(
                        existing
                            .target()
                            .and_then(|current| current.token().map(str::to_string)),
                    )
                })
            }
            _ => None,
        };
        let policy =
            ReplicaPolicy::new(target, existing.last_run().cloned()).with_interval(interval)?;
        self.store.save(repository_id, &policy).await?;
        Ok(policy)
    }

    /// Exporta el repositorio y lo importa en el Forge remoto.
    ///
    /// # Errors
    ///
    /// [`ReplicaError::NotConfigured`], [`ReplicaError::MissingToken`]
    /// o el remoto rechaza el bundle.
    pub async fn push_now(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPushOutcome, ReplicaError> {
        self.require_pushable(repository_id).await?;
        let policy = self.load_policy(repository_id).await?;
        let target = policy
            .target()
            .cloned()
            .ok_or(ReplicaError::NotConfigured)?;
        let token = target
            .token()
            .ok_or(ReplicaError::MissingToken)?
            .to_string();
        let bundle = self.bundles.export(repository_id).await?;
        let (body, content_type) = multipart_bundle(&bundle.bytes, &bundle.filename);
        let auth = format!("Bearer {token}");
        let response = self.post_import(&target, body, &content_type, &auth).await;

        let run = match response {
            Ok(response) if response.is_success() => match parse_import_body(&response.body) {
                Ok(outcome) => ReplicaRun::new(
                    Utc::now().to_rfc3339(),
                    outcome.packages_imported,
                    outcome.artifacts_imported,
                    outcome.skipped,
                    None,
                ),
                Err(message) => ReplicaRun::new(Utc::now().to_rfc3339(), 0, 0, 0, Some(message)),
            },
            Ok(response) => ReplicaRun::new(
                Utc::now().to_rfc3339(),
                0,
                0,
                0,
                Some(remote_status_message(response.status, &response.body)),
            ),
            Err(err) => ReplicaRun::new(Utc::now().to_rfc3339(), 0, 0, 0, Some(err.to_string())),
        };
        let saved = policy.with_last_run(run.clone());
        self.store.save(repository_id, &saved).await?;
        if let Some(error) = run.error() {
            return Err(ReplicaError::Remote(error.to_string()));
        }
        Ok(ReplicaPushOutcome {
            packages_imported: run.packages_imported(),
            artifacts_imported: run.artifacts_imported(),
            skipped: run.skipped(),
        })
    }

    /// Pide el export remoto y lo importa en este repositorio.
    ///
    /// # Errors
    ///
    /// [`ReplicaError::NotConfigured`], [`ReplicaError::MissingToken`]
    /// o el remoto / el import local fallan.
    pub async fn pull_now(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPushOutcome, ReplicaError> {
        self.require_pushable(repository_id).await?;
        let policy = self.load_policy(repository_id).await?;
        let target = policy
            .target()
            .cloned()
            .ok_or(ReplicaError::NotConfigured)?;
        let token = target
            .token()
            .ok_or(ReplicaError::MissingToken)?
            .to_string();
        let auth = format!("Bearer {token}");
        let response = self.get_export(&target, &auth).await;

        let run = match response {
            Ok(response) if response.is_success() => {
                match self.bundles.import(repository_id, response.body).await {
                    Ok(outcome) => ReplicaRun::new(
                        Utc::now().to_rfc3339(),
                        outcome.packages_imported,
                        outcome.artifacts_imported,
                        outcome.skipped,
                        None,
                    ),
                    Err(err) => {
                        ReplicaRun::new(Utc::now().to_rfc3339(), 0, 0, 0, Some(err.to_string()))
                    }
                }
            }
            Ok(response) => ReplicaRun::new(
                Utc::now().to_rfc3339(),
                0,
                0,
                0,
                Some(remote_status_message(response.status, &response.body)),
            ),
            Err(err) => ReplicaRun::new(Utc::now().to_rfc3339(), 0, 0, 0, Some(err.to_string())),
        };
        let saved = policy.with_last_run(run.clone());
        self.store.save(repository_id, &saved).await?;
        if let Some(error) = run.error() {
            return Err(ReplicaError::Remote(error.to_string()));
        }
        Ok(ReplicaPushOutcome {
            packages_imported: run.packages_imported(),
            artifacts_imported: run.artifacts_imported(),
            skipped: run.skipped(),
        })
    }

    /// Run every configured policy whose interval is due.
    ///
    /// One repository failing does not abort the rest. `push_now` /
    /// `pull_now` already persist `last_run` on success and on remote
    /// errors, so the next tick waits for the interval.
    ///
    /// # Errors
    ///
    /// [`ReplicaError::MissingSchema`] or a store failure while listing.
    pub async fn run_due(&self, now: DateTime<Utc>) -> Result<u32, ReplicaError> {
        let policies = match self.store.list_all().await {
            Ok(policies) => policies,
            Err(ReplicaStoreError::MissingSchema) => return Err(ReplicaError::MissingSchema),
            Err(err) => return Err(err.into()),
        };
        let mut ran = 0;
        for (repository_id, policy) in policies {
            if !policy.is_due(now) {
                continue;
            }
            ran += 1;
            let _ = match policy.target().map(ReplicaTarget::direction) {
                Some(ReplicaDirection::Pull) => self.pull_now(repository_id).await,
                _ => self.push_now(repository_id).await,
            };
        }
        Ok(ran)
    }

    async fn require_pushable(&self, repository_id: RepositoryId) -> Result<(), ReplicaError> {
        let repository = self.repositories.execute(repository_id).await?;
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(ReplicaError::AlloyRepository);
        }
        Ok(())
    }

    async fn post_import(
        &self,
        target: &ReplicaTarget,
        body: Bytes,
        content_type: &str,
        auth: &str,
    ) -> Result<ferrobox_ports::http_client::HttpResponse, ReplicaError> {
        let urls = target.import_urls();
        let mut last_not_found = None;
        for (index, url) in urls.iter().enumerate() {
            let more = index + 1 < urls.len();
            match self
                .http_client
                .post_with_headers(url, body.clone(), &[
                    ("content-type", content_type),
                    ("authorization", auth),
                ])
                .await
            {
                Ok(response) if response.is_success() || response.status != 404 || !more => {
                    return Ok(response);
                }
                Ok(not_found) => last_not_found = Some(not_found),
                Err(HttpClientError::Status { status: 404, .. }) if more => {}
                Err(err) => return Err(remote_error(err)),
            }
        }
        Ok(last_not_found.expect("import_urls is not empty"))
    }

    async fn get_export(
        &self,
        target: &ReplicaTarget,
        auth: &str,
    ) -> Result<ferrobox_ports::http_client::HttpResponse, ReplicaError> {
        let urls = target.export_urls();
        let mut last_not_found = None;
        for (index, url) in urls.iter().enumerate() {
            let more = index + 1 < urls.len();
            match self
                .http_client
                .get_with_headers(url, &[("authorization", auth)])
                .await
            {
                Ok(response) if response.is_success() || response.status != 404 || !more => {
                    return Ok(response);
                }
                Ok(not_found) => last_not_found = Some(not_found),
                Err(HttpClientError::Status { status: 404, .. }) if more => {}
                Err(err) => return Err(remote_error(err)),
            }
        }
        Ok(last_not_found.expect("export_urls is not empty"))
    }

    async fn load_policy(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPolicy, ReplicaError> {
        match self.store.find_by_repository(repository_id).await {
            Ok(policy) => Ok(policy),
            Err(ReplicaStoreError::MissingSchema) => Err(ReplicaError::MissingSchema),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Deserialize)]
struct RemoteImportBody {
    packages_imported: u32,
    artifacts_imported: u32,
    skipped: u32,
}

fn parse_import_body(body: &[u8]) -> Result<ReplicaPushOutcome, String> {
    let parsed: RemoteImportBody =
        serde_json::from_slice(body).map_err(|err| format!("invalid import response: {err}"))?;
    Ok(ReplicaPushOutcome {
        packages_imported: parsed.packages_imported,
        artifacts_imported: parsed.artifacts_imported,
        skipped: parsed.skipped,
    })
}

fn remote_status_message(status: u16, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {text}")
    }
}

fn remote_error(err: HttpClientError) -> ReplicaError {
    ReplicaError::Remote(err.to_string())
}

fn multipart_bundle(bytes: &Bytes, filename: &str) -> (Bytes, String) {
    let boundary = format!("ferrobox-replica-{}", Uuid::now_v7());
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"bundle\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/gzip\r\n\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (
        Bytes::from(body),
        format!("multipart/form-data; boundary={boundary}"),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::http_client::HttpResponse;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::publish_artifact::PublishArtifactUseCase;
    use crate::quota::QuotaService;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore,
        InMemoryReplicaStore, InMemoryRepositoryStore, InMemoryStorage,
    };

    fn forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Generic,
        )
        .unwrap()
    }

    fn service(
        repositories: Arc<InMemoryRepositoryStore>,
        artifacts: Arc<InMemoryArtifactStore>,
        index: Arc<InMemoryPackageIndexStore>,
        storage: Arc<InMemoryStorage>,
        http: Arc<InMemoryHttpClient>,
    ) -> (ReplicaService, Arc<InMemoryReplicaStore>) {
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        let bundles =
            RepositoryBundleService::new(repositories.clone(), artifacts, index, storage, quota);
        let store = Arc::new(InMemoryReplicaStore::default());
        (
            ReplicaService::new(store.clone(), repositories, bundles, http),
            store,
        )
    }

    #[tokio::test]
    async fn push_posts_the_bundle_to_the_remote_import() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let source = forge("src");
        let destination = forge("dst");
        repositories.save(&source).await.unwrap();
        repositories.save(&destination).await.unwrap();
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota,
        )
        .execute(source.id(), Bytes::from_static(b"replica-bytes"))
        .await
        .unwrap();

        let import_url = format!(
            "http://peer.example/api/repositories/{}/import",
            destination.id()
        );
        http.stub(
            &import_url,
            200,
            Bytes::from_static(
                br#"{"packages_imported":0,"artifacts_imported":1,"skipped":0,"bytes_copied":13}"#,
            ),
        );

        let (replica, _) = service(
            repositories,
            artifacts.clone(),
            index,
            storage,
            http.clone(),
        );
        replica
            .save_policy(
                source.id(),
                Some("http://peer.example".into()),
                Some(destination.id().into()),
                Some("peer-token".into()),
                None,
                None,
            )
            .await
            .unwrap();

        let outcome = replica.push_now(source.id()).await.unwrap();
        assert_eq!(outcome.artifacts_imported, 1);

        let posts = http.take_posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].url, import_url);
        assert!(
            posts[0]
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("authorization")
                    && value == "Bearer peer-token")
        );
        assert!(
            posts[0]
                .body
                .windows(b"name=\"bundle\"".len())
                .any(|w| w == b"name=\"bundle\"")
        );
        assert!(posts[0].body.len() > 13);
        assert_eq!(
            artifacts
                .find_by_repository_id(source.id())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn push_falls_back_to_the_cargo_run_import_path() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let source = forge("src");
        let destination = forge("dst");
        repositories.save(&source).await.unwrap();
        repositories.save(&destination).await.unwrap();
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub_response(
            &format!(
                "http://peer.example/api/repositories/{}/import",
                destination.id()
            ),
            ferrobox_ports::http_client::HttpResponse::new(404, Bytes::new()),
        );
        http.stub(
            &format!(
                "http://peer.example/repositories/{}/import",
                destination.id()
            ),
            200,
            Bytes::from_static(
                br#"{"packages_imported":0,"artifacts_imported":0,"skipped":0,"bytes_copied":0}"#,
            ),
        );
        let (replica, _) = service(
            repositories,
            artifacts,
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
        );
        replica
            .save_policy(
                source.id(),
                Some("http://peer.example".into()),
                Some(destination.id().into()),
                Some("tok".into()),
                None,
                None,
            )
            .await
            .unwrap();

        replica.push_now(source.id()).await.unwrap();
        let posts = http.take_posts();
        assert_eq!(posts.len(), 2);
        assert!(posts[0].url.contains("/api/repositories/"));
        assert!(
            posts[1]
                .url
                .ends_with(&format!("/repositories/{}/import", destination.id()))
        );
        assert!(!posts[1].url.contains("/api/"));
    }

    #[tokio::test]
    async fn rejects_an_alloy_and_a_missing_token() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let member = forge("member");
        let alloy = Repository::new(
            RepositoryName::parse("all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![member.id()],
            },
            PackageEcosystem::Generic,
        )
        .unwrap();
        repositories.save(&member).await.unwrap();
        repositories.save(&alloy).await.unwrap();
        let (replica, _) = service(
            repositories.clone(),
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let err = replica.get_policy(alloy.id()).await.unwrap_err();
        assert!(matches!(err, ReplicaError::AlloyRepository));

        let source = forge("bare");
        repositories.save(&source).await.unwrap();
        replica
            .save_policy(
                source.id(),
                Some("http://peer.example".into()),
                Some(member.id().into()),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let err = replica.push_now(source.id()).await.unwrap_err();
        assert!(matches!(err, ReplicaError::MissingToken));
    }

    #[tokio::test]
    async fn records_a_failed_remote_status() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let source = forge("src");
        let destination = forge("dst");
        repositories.save(&source).await.unwrap();
        repositories.save(&destination).await.unwrap();
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub_response(
            &format!(
                "http://peer.example/api/repositories/{}/import",
                destination.id()
            ),
            HttpResponse::new(403, Bytes::from_static(br#"{"error":"forbidden"}"#)),
        );
        let (replica, store) = service(
            repositories,
            artifacts,
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http,
        );
        replica
            .save_policy(
                source.id(),
                Some("http://peer.example".into()),
                Some(destination.id().into()),
                Some("tok".into()),
                None,
                None,
            )
            .await
            .unwrap();
        let err = replica.push_now(source.id()).await.unwrap_err();
        assert!(matches!(err, ReplicaError::Remote(_)));
        let saved = store.find_by_repository(source.id()).await.unwrap();
        assert!(saved.last_run().unwrap().error().unwrap().contains("403"));
    }

    #[tokio::test]
    async fn pull_imports_the_remote_export_into_this_repository() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let source = forge("src");
        let destination = forge("dst");
        repositories.save(&source).await.unwrap();
        repositories.save(&destination).await.unwrap();
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota.clone(),
        )
        .execute(source.id(), Bytes::from_static(b"pulled-bytes"))
        .await
        .unwrap();
        let bundles = RepositoryBundleService::new(
            repositories.clone(),
            artifacts.clone(),
            index.clone(),
            storage.clone(),
            quota,
        );
        let exported = bundles.export(source.id()).await.unwrap();
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            &format!(
                "http://peer.example/api/repositories/{}/export",
                source.id()
            ),
            200,
            exported.bytes,
        );

        let (replica, _) = service(repositories, artifacts.clone(), index, storage, http);
        replica
            .save_policy(
                destination.id(),
                Some("http://peer.example".into()),
                Some(source.id().into()),
                Some("tok".into()),
                Some("pull".into()),
                None,
            )
            .await
            .unwrap();

        let outcome = replica.pull_now(destination.id()).await.unwrap();
        assert_eq!(outcome.artifacts_imported, 1);
        assert_eq!(
            artifacts
                .find_by_repository_id(destination.id())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn pull_falls_back_to_the_cargo_run_export_path() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let source = forge("src");
        let destination = forge("dst");
        repositories.save(&source).await.unwrap();
        repositories.save(&destination).await.unwrap();
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota.clone(),
        )
        .execute(source.id(), Bytes::from_static(b"pulled-bytes"))
        .await
        .unwrap();
        let bundles = RepositoryBundleService::new(
            repositories.clone(),
            artifacts.clone(),
            index.clone(),
            storage.clone(),
            quota,
        );
        let exported = bundles.export(source.id()).await.unwrap();
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub_response(
            &format!(
                "http://peer.example/api/repositories/{}/export",
                source.id()
            ),
            HttpResponse::new(404, Bytes::new()),
        );
        http.stub(
            &format!("http://peer.example/repositories/{}/export", source.id()),
            200,
            exported.bytes,
        );

        let (replica, _) = service(repositories, artifacts.clone(), index, storage, http);
        replica
            .save_policy(
                destination.id(),
                Some("http://peer.example".into()),
                Some(source.id().into()),
                Some("tok".into()),
                Some("pull".into()),
                None,
            )
            .await
            .unwrap();

        let outcome = replica.pull_now(destination.id()).await.unwrap();
        assert_eq!(outcome.artifacts_imported, 1);
    }

    #[tokio::test]
    async fn run_due_pushes_a_policy_that_never_ran() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let source = forge("src");
        let destination = forge("dst");
        repositories.save(&source).await.unwrap();
        repositories.save(&destination).await.unwrap();
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            &format!(
                "http://peer.example/api/repositories/{}/import",
                destination.id()
            ),
            200,
            Bytes::from_static(
                br#"{"packages_imported":0,"artifacts_imported":0,"skipped":0,"bytes_copied":0}"#,
            ),
        );
        let (replica, _) = service(
            repositories,
            artifacts,
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
        );
        replica
            .save_policy(
                source.id(),
                Some("http://peer.example".into()),
                Some(destination.id().into()),
                Some("tok".into()),
                None,
                Some(1),
            )
            .await
            .unwrap();

        let ran = replica.run_due(Utc::now()).await.unwrap();
        assert_eq!(ran, 1);
        assert_eq!(http.take_posts().len(), 1);
        assert_eq!(replica.run_due(Utc::now()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn run_due_skips_a_policy_without_an_interval() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let source = forge("src");
        let destination = forge("dst");
        repositories.save(&source).await.unwrap();
        repositories.save(&destination).await.unwrap();
        let http = Arc::new(InMemoryHttpClient::default());
        let (replica, _) = service(
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http.clone(),
        );
        replica
            .save_policy(
                source.id(),
                Some("http://peer.example".into()),
                Some(destination.id().into()),
                Some("tok".into()),
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(replica.run_due(Utc::now()).await.unwrap(), 0);
        assert!(http.take_posts().is_empty());
    }
}
