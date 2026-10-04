use std::sync::Arc;

use chrono::{DateTime, Utc};
use ferrobox_domain::api_token::{ApiToken, ApiTokenName, TokenRepositories, TokenScopes};
use ferrobox_domain::ids::UserId;
use ferrobox_ports::api_token_store::{ApiTokenRecord, ApiTokenStore, ApiTokenStoreError};
use thiserror::Error;

use crate::auth_crypto::{generate_api_token_secret, hash_api_token_secret};

/// Reasons creating an API token can fail.
#[derive(Debug, Error)]
pub enum CreateApiTokenError {
    /// The expiry is in the past.
    #[error("token expiry must be in the future")]
    ExpiryInThePast,

    /// Failed to persist the token.
    #[error(transparent)]
    Persistence(#[from] ApiTokenStoreError),
}

/// Result of creating a token: metadata + plaintext secret (once only).
#[derive(Debug, Clone)]
pub struct CreateApiTokenResult {
    /// Newly created token.
    pub token: ApiToken,
    /// Plaintext secret. Exposed only here.
    pub plaintext_secret: String,
}

/// Use case: issue a new API token for a user.
pub struct CreateApiTokenUseCase {
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl CreateApiTokenUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self { api_token_store }
    }

    /// Issues an unrestricted token (login, robots, existing callers).
    ///
    /// # Errors
    ///
    /// [`CreateApiTokenError::Persistence`] if the store fails.
    pub async fn execute(
        &self,
        user_id: UserId,
        name: ApiTokenName,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<CreateApiTokenResult, CreateApiTokenError> {
        self.execute_with_scopes(user_id, name, expires_at, TokenScopes::unrestricted())
            .await
    }

    /// Issues a token with the given scopes. Empty scopes inherit the
    /// user's full role.
    ///
    /// # Errors
    ///
    /// [`CreateApiTokenError::ExpiryInThePast`] if `expires_at` is not in
    /// the future, or [`CreateApiTokenError::Persistence`] if the store fails.
    pub async fn execute_with_scopes(
        &self,
        user_id: UserId,
        name: ApiTokenName,
        expires_at: Option<DateTime<Utc>>,
        scopes: TokenScopes,
    ) -> Result<CreateApiTokenResult, CreateApiTokenError> {
        self.execute_limited(
            user_id,
            name,
            expires_at,
            scopes,
            TokenRepositories::unrestricted(),
        )
        .await
    }

    /// Issues a token with scopes and an optional repository allow-list.
    /// Empty repositories inherit every repository the user can access.
    ///
    /// # Errors
    ///
    /// [`CreateApiTokenError::ExpiryInThePast`] if `expires_at` is not in
    /// the future, or [`CreateApiTokenError::Persistence`] if the store fails.
    pub async fn execute_limited(
        &self,
        user_id: UserId,
        name: ApiTokenName,
        expires_at: Option<DateTime<Utc>>,
        scopes: TokenScopes,
        repositories: TokenRepositories,
    ) -> Result<CreateApiTokenResult, CreateApiTokenError> {
        if expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(CreateApiTokenError::ExpiryInThePast);
        }
        let (plaintext_secret, prefix) = generate_api_token_secret();
        let token = ApiToken::new(user_id, name, prefix)
            .with_expires_at(expires_at)
            .with_scopes(scopes)
            .with_repositories(repositories);
        let token_hash = hash_api_token_secret(&plaintext_secret);
        self.api_token_store.save(&token, &token_hash).await?;

        Ok(CreateApiTokenResult {
            token,
            plaintext_secret,
        })
    }
}

/// Reasons listing tokens can fail.
#[derive(Debug, Error)]
pub enum ListApiTokensError {
    /// Failed to query the store.
    #[error(transparent)]
    Persistence(#[from] ApiTokenStoreError),
}

/// Use case: list a user's API tokens.
pub struct ListApiTokensUseCase {
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl ListApiTokensUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self { api_token_store }
    }

    /// Lists the user's tokens, without secrets.
    ///
    /// # Errors
    ///
    /// Returns [`ListApiTokensError::Persistence`] if the backend
    /// fails.
    pub async fn execute(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApiTokenRecord>, ListApiTokensError> {
        Ok(self.api_token_store.list_for_user(user_id).await?)
    }
}

/// Reasons revoking a token can fail.
#[derive(Debug, Error)]
pub enum RevokeApiTokenError {
    /// The token does not exist or does not belong to the user.
    #[error("API token not found")]
    NotFound,

    /// Failed to query / update the store.
    #[error(transparent)]
    Persistence(#[from] ApiTokenStoreError),
}

/// Use case: revoke (delete) one of the caller's own API tokens.
pub struct RevokeApiTokenUseCase {
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl RevokeApiTokenUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self { api_token_store }
    }

    /// Revokes the given token if it belongs to the user.
    ///
    /// # Errors
    ///
    /// Returns [`RevokeApiTokenError::NotFound`] if the token does not
    /// exist or is not the user's, or a persistence error if the
    /// backend fails.
    pub async fn execute(
        &self,
        user_id: UserId,
        token_id: ferrobox_domain::ids::ApiTokenId,
    ) -> Result<(), RevokeApiTokenError> {
        if self
            .api_token_store
            .delete_for_user(token_id, user_id)
            .await?
        {
            Ok(())
        } else {
            Err(RevokeApiTokenError::NotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::ids::UserId;
    use ferrobox_domain::user::{Role, User, Username};

    use crate::test_support::InMemoryApiTokenStore;

    use super::*;

    #[tokio::test]
    async fn create_list_and_revoke_a_token() {
        let store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);

        let created = CreateApiTokenUseCase::new(store.clone())
            .execute(
                user.id(),
                ApiTokenName::parse("cargo-publish").unwrap(),
                None,
            )
            .await
            .unwrap();

        assert!(created.plaintext_secret.starts_with("fb_"));

        let listed = ListApiTokensUseCase::new(store.clone())
            .execute(user.id())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].token, created.token);

        RevokeApiTokenUseCase::new(store.clone())
            .execute(user.id(), created.token.id())
            .await
            .unwrap();

        assert!(
            ListApiTokensUseCase::new(store)
                .execute(user.id())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn create_persists_read_scope() {
        let store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);

        let created = CreateApiTokenUseCase::new(store.clone())
            .execute_with_scopes(
                user.id(),
                ApiTokenName::parse("readonly").unwrap(),
                None,
                TokenScopes::parse(["read"]).unwrap(),
            )
            .await
            .unwrap();

        assert!(created.token.scopes().allows_read());
        assert!(!created.token.scopes().allows_write());

        let listed = ListApiTokensUseCase::new(store)
            .execute(user.id())
            .await
            .unwrap();
        assert_eq!(listed[0].token.scopes().as_stored(), "read");
        assert!(listed[0].token.repositories().is_unrestricted());
    }

    #[tokio::test]
    async fn create_persists_repository_allow_list() {
        let store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);
        let repository_id = ferrobox_domain::ids::RepositoryId::new();

        let created = CreateApiTokenUseCase::new(store.clone())
            .execute_limited(
                user.id(),
                ApiTokenName::parse("ci").unwrap(),
                None,
                TokenScopes::parse(["write"]).unwrap(),
                TokenRepositories::from_ids([repository_id]),
            )
            .await
            .unwrap();

        assert!(created.token.repositories().allows(repository_id));
        assert!(
            !created
                .token
                .repositories()
                .allows(ferrobox_domain::ids::RepositoryId::new())
        );

        let listed = ListApiTokensUseCase::new(store)
            .execute(user.id())
            .await
            .unwrap();
        assert_eq!(
            listed[0].token.repositories().as_stored(),
            repository_id.to_string()
        );
    }

    #[tokio::test]
    async fn revoke_of_unknown_token_is_not_found() {
        let store = Arc::new(InMemoryApiTokenStore::default());
        let result = RevokeApiTokenUseCase::new(store)
            .execute(UserId::new(), ferrobox_domain::ids::ApiTokenId::new())
            .await;

        assert!(matches!(result, Err(RevokeApiTokenError::NotFound)));
    }
}
