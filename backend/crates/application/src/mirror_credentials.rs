//! Save and clear the username and secret a mirror sends to its upstream.

use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::mirror_credential::MirrorCredential;
use ferrobox_domain::repository::RepositoryKind;
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
        }
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
    ) -> Result<(), MirrorCredentialError> {
        let repository = self.require_repository(repository_id).await?;
        if matches!(repository.kind(), RepositoryKind::Mirror { .. }) {
            Ok(())
        } else {
            Err(MirrorCredentialError::NotAMirror)
        }
    }
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};

    use crate::test_support::{InMemoryMirrorCredentialStore, InMemoryRepositoryStore};

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
}
