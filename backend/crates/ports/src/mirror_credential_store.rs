use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::mirror_credential::MirrorCredential;
use thiserror::Error;

/// Reasons a mirror-credential persistence operation can fail.
#[derive(Debug, Error)]
pub enum MirrorCredentialStoreError {
    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for a mirror's upstream username and secret.
///
/// The secret is stored so FerroBox can send it. Callers that build
/// an API response must not include [`MirrorCredential::secret`].
#[async_trait]
pub trait MirrorCredentialStore: Send + Sync {
    /// Inserts or replaces the credential for one repository.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorCredentialStoreError::Backend`] if the backend fails.
    async fn save(
        &self,
        repository_id: RepositoryId,
        credential: &MirrorCredential,
    ) -> Result<(), MirrorCredentialStoreError>;

    /// Loads the credential, including the secret. `None` if unset.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorCredentialStoreError::Backend`] if the backend fails.
    async fn find(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Option<MirrorCredential>, MirrorCredentialStoreError>;

    /// Removes the credential. Returns `true` if a row was deleted.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorCredentialStoreError::Backend`] if the backend fails.
    async fn delete(&self, repository_id: RepositoryId)
    -> Result<bool, MirrorCredentialStoreError>;
}
