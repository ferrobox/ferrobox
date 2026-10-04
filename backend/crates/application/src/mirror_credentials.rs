//! Save and clear the username and secret a mirror sends to its upstream.

use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::mirror_credential::MirrorCredential;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::http_client::HttpClient;
use ferrobox_ports::mirror_credential_store::{MirrorCredentialStore, MirrorCredentialStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Public view of a mirror credential. The secret is absent on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorCredentialStatus {
    /// `true` when a secret is stored.
    pub configured: bool,
    /// Username, or empty when the secret is a bearer token or nothing is stored.
    pub username: String,
}

/// HTTP status of one GET against the saved upstream. The body is absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorUpstreamProbe {
    /// Status code returned by the upstream.
    pub status: u16,
    /// `true` when `status` is 2xx.
    pub ok: bool,
    /// `true` when a stored credential was attached to the request.
    pub authenticated: bool,
}

/// Reasons a mirror-credential operation can fail.
#[derive(Debug, Error)]
pub enum MirrorCredentialError {
    /// The repository does not exist.
    #[error("repository not found")]
    RepositoryNotFound,

    /// Only a mirror calls an upstream.
    #[error("upstream credentials apply only to a mirror repository")]
    NotAMirror,

    /// The username or secret is not valid.
    #[error(transparent)]
    Invalid(#[from] ferrobox_domain::mirror_credential::MirrorCredentialError),

    /// The outbound request failed before an HTTP status was available.
    #[error("{0}")]
    Upstream(String),

    /// Production wires an HTTP client. A probe without one cannot run.
    #[error("upstream probe is not configured")]
    ProbeUnavailable,

    /// Failed to read the repository.
    #[error(transparent)]
    RepositoryPersistence(#[from] RepositoryStoreError),

    /// Failed to read or write the credential.
    #[error(transparent)]
    Persistence(#[from] MirrorCredentialStoreError),
}

/// Use case: read, replace, or remove a mirror's upstream credential.
pub struct MirrorCredentialService {
    repositories: Arc<dyn RepositoryStore>,
    credentials: Arc<dyn MirrorCredentialStore>,
    http: Option<Arc<dyn HttpClient>>,
}

impl MirrorCredentialService {
    /// Builds the service from its ports.
    #[must_use]
    pub fn new(
        repositories: Arc<dyn RepositoryStore>,
        credentials: Arc<dyn MirrorCredentialStore>,
    ) -> Self {
        Self {
            repositories,
            credentials,
            http: None,
        }
    }

    /// Attaches the client used by [`Self::probe`].
    ///
    /// Kept off [`Self::new`] so tests that never probe do not need a client.
    #[must_use]
    pub fn with_http(mut self, http: Arc<dyn HttpClient>) -> Self {
        self.http = Some(http);
        self
    }

    /// The store strategies use when they call an upstream.
    #[must_use]
    pub fn store(&self) -> Arc<dyn MirrorCredentialStore> {
        self.credentials.clone()
    }

    /// Reports whether a secret is stored. Does not return the secret.
    ///
    /// # Errors
    ///
    /// [`MirrorCredentialError::RepositoryNotFound`] if the repository
    /// does not exist, or a persistence error if a port fails.
    pub async fn status(
        &self,
        repository_id: RepositoryId,
    ) -> Result<MirrorCredentialStatus, MirrorCredentialError> {
        self.require_repository(repository_id).await?;
        let found = self.credentials.find(repository_id).await?;
        Ok(match found {
            Some(credential) => MirrorCredentialStatus {
                configured: true,
                username: credential.username().to_string(),
            },
            None => MirrorCredentialStatus {
                configured: false,
                username: String::new(),
            },
        })
    }

    /// Replaces the upstream credential. The secret is required every time.
    ///
    /// # Errors
    ///
    /// [`MirrorCredentialError`] if the repository is missing, is not a
    /// mirror, the credential is invalid, or a port fails.
    pub async fn save(
        &self,
        repository_id: RepositoryId,
        username: impl Into<String>,
        secret: impl Into<String>,
    ) -> Result<MirrorCredentialStatus, MirrorCredentialError> {
        self.require_mirror(repository_id).await?;
        let credential = MirrorCredential::parse(username, secret)?;
        let username = credential.username().to_string();
        self.credentials.save(repository_id, &credential).await?;
        Ok(MirrorCredentialStatus {
            configured: true,
            username,
        })
    }

    /// `GET`s the saved upstream URL with the saved credential.
    ///
    /// The response body is dropped. A non-2xx status is a result, not
    /// an error. A transport failure is [`MirrorCredentialError::Upstream`].
    ///
    /// # Errors
    ///
    /// [`MirrorCredentialError`] if the repository is missing, is not a
    /// mirror, the HTTP client was not configured, or the request fails
    /// before a status is available.
    pub async fn probe(
        &self,
        repository_id: RepositoryId,
    ) -> Result<MirrorUpstreamProbe, MirrorCredentialError> {
        let repository = self.require_mirror(repository_id).await?;
        let RepositoryKind::Mirror { upstream } = repository.kind() else {
            return Err(MirrorCredentialError::NotAMirror);
        };
        let http = self
            .http
            .as_ref()
            .ok_or(MirrorCredentialError::ProbeUnavailable)?;
        let authenticated = self.credentials.find(repository_id).await?.is_some();
        let response = crate::packaging::upstream::upstream_get_with_headers(
            http.as_ref(),
            Some(self.credentials.as_ref()),
            repository_id,
            upstream.as_str(),
            &[],
        )
        .await
        .map_err(|err| {
            MirrorCredentialError::Upstream(format!("could not reach the upstream: {err}"))
        })?;
        Ok(MirrorUpstreamProbe {
            status: response.status,
            ok: response.is_success(),
            authenticated,
        })
    }

    /// Removes the credential. Idempotent.
    ///
    /// # Errors
    ///
    /// [`MirrorCredentialError::RepositoryNotFound`] if the repository
    /// does not exist, or a persistence error if a port fails.
    pub async fn clear(&self, repository_id: RepositoryId) -> Result<(), MirrorCredentialError> {
        self.require_repository(repository_id).await?;
        self.credentials.delete(repository_id).await?;
        Ok(())
    }

    async fn require_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ferrobox_domain::repository::Repository, MirrorCredentialError> {
        self.repositories
            .find_by_id(repository_id)
            .await?
            .ok_or(MirrorCredentialError::RepositoryNotFound)
    }

    async fn require_mirror(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ferrobox_domain::repository::Repository, MirrorCredentialError> {
        let repository = self.require_repository(repository_id).await?;
        if matches!(repository.kind(), RepositoryKind::Mirror { .. }) {
            Ok(repository)
        } else {
            Err(MirrorCredentialError::NotAMirror)
        }
    }
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};

    use crate::test_support::{
        InMemoryHttpClient, InMemoryMirrorCredentialStore, InMemoryRepositoryStore,
    };

    use super::*;

    fn mirror() -> Repository {
        Repository::new(
            RepositoryName::parse("npm-private").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://registry.npmjs.org").unwrap(),
            },
            PackageEcosystem::Npm,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn save_reports_the_username_and_hides_the_secret() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let credentials = Arc::new(InMemoryMirrorCredentialStore::default());
        let repository = mirror();
        repositories.save(&repository).await.unwrap();
        let service = MirrorCredentialService::new(repositories, credentials.clone());

        let status = service
            .save(repository.id(), "ci-bot", "secret-value")
            .await
            .unwrap();
        assert!(status.configured);
        assert_eq!(status.username, "ci-bot");

        let stored = credentials.find(repository.id()).await.unwrap().unwrap();
        assert_eq!(stored.secret(), "secret-value");
        let again = service.status(repository.id()).await.unwrap();
        assert_eq!(again.username, "ci-bot");
        assert!(!format!("{again:?}").contains("secret-value"));

        service.clear(repository.id()).await.unwrap();
        assert!(!service.status(repository.id()).await.unwrap().configured);
    }

    #[tokio::test]
    async fn a_forge_cannot_store_an_upstream_secret() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let repository = Repository::new(
            RepositoryName::parse("npm-forge").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Npm,
        )
        .unwrap();
        repositories.save(&repository).await.unwrap();
        let service = MirrorCredentialService::new(
            repositories,
            Arc::new(InMemoryMirrorCredentialStore::default()),
        );
        let result = service.save(repository.id(), "", "token").await;
        assert!(matches!(result, Err(MirrorCredentialError::NotAMirror)));
    }

    #[tokio::test]
    async fn probe_reports_status_and_drops_the_body() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let credentials = Arc::new(InMemoryMirrorCredentialStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = mirror();
        let upstream = match repository.kind() {
            RepositoryKind::Mirror { upstream } => upstream.as_str().to_string(),
            RepositoryKind::Forge | RepositoryKind::Alloy { .. } => unreachable!(),
        };
        repositories.save(&repository).await.unwrap();
        let service =
            MirrorCredentialService::new(repositories, credentials).with_http(http.clone());
        service
            .save(repository.id(), "", "secret-value")
            .await
            .unwrap();
        http.stub(
            &upstream,
            200,
            bytes::Bytes::from_static(br#"{"token":"secret-value"}"#),
        );

        let probe = service.probe(repository.id()).await.unwrap();

        assert_eq!(probe, MirrorUpstreamProbe {
            status: 200,
            ok: true,
            authenticated: true,
        });
        assert!(!format!("{probe:?}").contains("secret-value"));
        let gets = http.take_gets();
        assert_eq!(gets.len(), 1);
        assert_eq!(gets[0].url, upstream);
        assert!(
            gets[0]
                .headers
                .iter()
                .any(|(name, value)| name == "authorization" && value == "Bearer secret-value")
        );
    }

    #[tokio::test]
    async fn probe_reports_401_when_basic_is_not_a_bearer_token() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let credentials = Arc::new(InMemoryMirrorCredentialStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = mirror();
        let upstream = match repository.kind() {
            RepositoryKind::Mirror { upstream } => upstream.as_str().to_string(),
            RepositoryKind::Forge | RepositoryKind::Alloy { .. } => unreachable!(),
        };
        repositories.save(&repository).await.unwrap();
        let service =
            MirrorCredentialService::new(repositories, credentials).with_http(http.clone());
        service
            .save(repository.id(), "wrong", "secret-value")
            .await
            .unwrap();
        http.stub(&upstream, 401, bytes::Bytes::from_static(b"nope"));

        let probe = service.probe(repository.id()).await.unwrap();

        assert_eq!(probe.status, 401);
        assert!(!probe.ok);
        assert!(probe.authenticated);
        assert!(!format!("{probe:?}").contains("secret-value"));
        let gets = http.take_gets();
        assert!(
            gets[0]
                .headers
                .iter()
                .any(|(name, value)| name == "authorization" && value.starts_with("Basic "))
        );
    }

    #[tokio::test]
    async fn changing_the_upstream_url_keeps_the_saved_secret() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let credentials = Arc::new(InMemoryMirrorCredentialStore::default());
        let repository = mirror();
        repositories.save(&repository).await.unwrap();
        let service = MirrorCredentialService::new(repositories.clone(), credentials.clone());
        service
            .save(repository.id(), "", "secret-value")
            .await
            .unwrap();

        let moved = repository
            .with_kind(RepositoryKind::Mirror {
                upstream: url::Url::parse("https://other.example/bearer").unwrap(),
            })
            .unwrap();
        repositories.save(&moved).await.unwrap();

        let status = service.status(moved.id()).await.unwrap();
        assert!(status.configured);
        assert_eq!(
            credentials
                .find(moved.id())
                .await
                .unwrap()
                .unwrap()
                .secret(),
            "secret-value"
        );
    }
}
