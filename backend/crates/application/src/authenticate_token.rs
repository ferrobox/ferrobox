use std::sync::Arc;

use chrono::Utc;
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::user::User;
use ferrobox_ports::api_token_store::{ApiTokenStore, ApiTokenStoreError};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::hash_api_token_secret;

/// Reasons authenticating a token secret can fail.
#[derive(Debug, Error)]
pub enum AuthenticateTokenError {
    /// The secret does not match any known token.
    #[error("invalid or revoked API token")]
    InvalidToken,

    /// Failed to query the token store.
    #[error(transparent)]
    TokenPersistence(#[from] ApiTokenStoreError),

    /// Failed to query the user store.
    #[error(transparent)]
    UserPersistence(#[from] UserStoreError),
}

/// Result of authenticating a token secret: the associated user and
/// token.
#[derive(Debug, Clone)]
pub struct AuthenticatedPrincipal {
    /// User who owns the token.
    pub user: User,
    /// Token that authenticated the request.
    pub token: ApiToken,
}

/// Use case: resolve a Bearer / Token secret to an authenticated
/// principal.
pub struct AuthenticateTokenUseCase {
    user_store: Arc<dyn UserStore>,
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl AuthenticateTokenUseCase {
    /// Builds the use case from its ports.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>, api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self {
            user_store,
            api_token_store,
        }
    }

    /// Hashes the secret, looks up the token, and loads its user.
    ///
    /// # Errors
    ///
    /// Returns [`AuthenticateTokenError::InvalidToken`] if the secret
    /// does not exist or the associated user has disappeared, or a
    /// persistence error if the backend fails.
    pub async fn execute(
        &self,
        plaintext_secret: &str,
    ) -> Result<AuthenticatedPrincipal, AuthenticateTokenError> {
        let token_hash = hash_api_token_secret(plaintext_secret);
        let Some(token) = self.api_token_store.find_by_token_hash(&token_hash).await? else {
            return Err(AuthenticateTokenError::InvalidToken);
        };
        if token.is_expired(Utc::now()) {
            return Err(AuthenticateTokenError::InvalidToken);
        }

        let Some(user) = self.user_store.find_by_id(token.user_id()).await? else {
            return Err(AuthenticateTokenError::InvalidToken);
        };

        Ok(AuthenticatedPrincipal { user, token })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::user::{Role, User, Username};

    use crate::auth_crypto::{generate_api_token_secret, hash_api_token_secret, hash_password};
    use crate::test_support::{InMemoryApiTokenStore, InMemoryUserStore};

    use super::*;

    #[tokio::test]
    async fn authenticates_a_valid_secret() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);
        user_store
            .save_with_password_hash(&user, &hash_password("admin").unwrap())
            .await
            .unwrap();

        let (secret, prefix) = generate_api_token_secret();
        let token = ApiToken::new(
            user.id(),
            ApiTokenName::parse("session").unwrap(),
            prefix,
        );
        api_token_store
            .save(&token, &hash_api_token_secret(&secret))
            .await
            .unwrap();

        let principal = AuthenticateTokenUseCase::new(user_store, api_token_store)
            .execute(&secret)
            .await
            .unwrap();

        assert_eq!(principal.user, user);
        assert_eq!(principal.token, token);
    }

    #[tokio::test]
    async fn rejects_an_unknown_secret() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());

        let result = AuthenticateTokenUseCase::new(user_store, api_token_store)
            .execute("fb_deadbeef")
            .await;

        assert!(matches!(result, Err(AuthenticateTokenError::InvalidToken)));
    }

    #[tokio::test]
    async fn rejects_an_expired_secret() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);
        user_store
            .save_with_password_hash(&user, &hash_password("admin").unwrap())
            .await
            .unwrap();

        let (secret, prefix) = generate_api_token_secret();
        let token = ApiToken::new(
            user.id(),
            ApiTokenName::parse("old").unwrap(),
            prefix,
        )
        .with_expires_at(Some(Utc::now() - chrono::TimeDelta::hours(1)));
        api_token_store
            .save(&token, &hash_api_token_secret(&secret))
            .await
            .unwrap();

        let result = AuthenticateTokenUseCase::new(user_store, api_token_store)
            .execute(&secret)
            .await;

        assert!(matches!(result, Err(AuthenticateTokenError::InvalidToken)));
    }
}
