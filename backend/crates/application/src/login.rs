use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ferrobox_domain::api_token::{ApiToken, ApiTokenName};
use ferrobox_domain::user::{User, Username};
use ferrobox_ports::api_token_store::{ApiTokenStore, ApiTokenStoreError};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::{
    generate_api_token_secret, hash_api_token_secret, verify_password,
};

/// Reasons sign-in can fail.
#[derive(Debug, Error)]
pub enum LoginError {
    /// Nonexistent user or incorrect password. A single message is used
    /// so as not to leak whether the username exists.
    #[error("invalid username or password")]
    InvalidCredentials,

    /// Failed to persist the issued session token.
    #[error(transparent)]
    TokenPersistence(#[from] ApiTokenStoreError),

    /// Failed to query the user store.
    #[error(transparent)]
    UserPersistence(#[from] UserStoreError),
}

/// Result of a successful sign-in: the authenticated user and the
/// session token secret (shown only once).
#[derive(Debug, Clone)]
pub struct LoginResult {
    /// Authenticated user.
    pub user: User,
    /// Newly issued API token (plaintext secret).
    pub token: ApiToken,
    /// Plaintext token secret. Exposed only here.
    pub plaintext_secret: String,
}

/// Use case: authenticate with username and password, issuing a session
/// API token.
pub struct LoginUseCase {
    user_store: Arc<dyn UserStore>,
    api_token_store: Arc<dyn ApiTokenStore>,
    session_ttl: Duration,
}

impl LoginUseCase {
    /// Builds the use case from its ports. The session expires after
    /// 12 hours.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>, api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self::with_session_ttl(user_store, api_token_store, Duration::from_hours(12))
    }

    /// Same as [`Self::new`] with a configurable session TTL.
    #[must_use]
    pub fn with_session_ttl(
        user_store: Arc<dyn UserStore>,
        api_token_store: Arc<dyn ApiTokenStore>,
        session_ttl: Duration,
    ) -> Self {
        Self {
            user_store,
            api_token_store,
            session_ttl,
        }
    }

    /// Verifies the credentials and, if they are valid, issues a
    /// session token named `session`.
    ///
    /// # Errors
    ///
    /// Returns [`LoginError::InvalidCredentials`] if the user does not
    /// exist or the password does not match, or a persistence error if
    /// the store fails.
    ///
    /// # Panics
    ///
    /// In practice this never panics: the `"session"` literal is always
    /// a valid [`ApiTokenName`].
    pub async fn execute(
        &self,
        username: Username,
        password: &str,
    ) -> Result<LoginResult, LoginError> {
        let Some((user, password_hash)) = self
            .user_store
            .find_by_username_with_password_hash(&username)
            .await?
        else {
            return Err(LoginError::InvalidCredentials);
        };

        if user.is_robot() || !verify_password(password, &password_hash) {
            return Err(LoginError::InvalidCredentials);
        }

        let (plaintext_secret, prefix) = generate_api_token_secret();
        let expires_at = Utc::now() + self.session_ttl;
        let token = ApiToken::new(
            user.id(),
            ApiTokenName::parse("session").expect("literal 'session' is a valid token name"),
            prefix,
        )
        .with_expires_at(Some(expires_at));
        let token_hash = hash_api_token_secret(&plaintext_secret);
        self.api_token_store.save(&token, &token_hash).await?;

        Ok(LoginResult {
            user,
            token,
            plaintext_secret,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::user::Role;

    use crate::auth_crypto::hash_password;
    use crate::test_support::{InMemoryApiTokenStore, InMemoryUserStore};

    use super::*;

    async fn seeded_stores() -> (Arc<InMemoryUserStore>, Arc<InMemoryApiTokenStore>, User) {
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);
        let hash = hash_password("admin").unwrap();
        user_store
            .save_with_password_hash(&user, &hash)
            .await
            .unwrap();
        (user_store, api_token_store, user)
    }

    #[tokio::test]
    async fn login_with_valid_credentials_issues_a_session_token() {
        let (user_store, api_token_store, user) = seeded_stores().await;
        let use_case = LoginUseCase::new(user_store, api_token_store.clone());

        let result = use_case
            .execute(Username::parse("admin").unwrap(), "admin")
            .await
            .unwrap();

        assert_eq!(result.user, user);
        assert!(result.plaintext_secret.starts_with("fb_"));
        assert_eq!(
            api_token_store.list_for_user(user.id()).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn login_with_wrong_password_is_rejected() {
        let (user_store, api_token_store, _) = seeded_stores().await;
        let use_case = LoginUseCase::new(user_store, api_token_store);

        let result = use_case
            .execute(Username::parse("admin").unwrap(), "wrong")
            .await;

        assert!(matches!(result, Err(LoginError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn login_with_unknown_user_is_rejected() {
        let (user_store, api_token_store, _) = seeded_stores().await;
        let use_case = LoginUseCase::new(user_store, api_token_store);

        let result = use_case
            .execute(Username::parse("nobody").unwrap(), "admin")
            .await;

        assert!(matches!(result, Err(LoginError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn login_rejects_a_robot_account() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let robot = User::new(Username::parse("ci").unwrap(), Role::Developer).with_robot(true);
        let hash = hash_password("Secret1a").unwrap();
        user_store
            .save_with_password_hash(&robot, &hash)
            .await
            .unwrap();
        let use_case = LoginUseCase::new(user_store, api_token_store);

        let result = use_case
            .execute(Username::parse("ci").unwrap(), "Secret1a")
            .await;

        assert!(matches!(result, Err(LoginError::InvalidCredentials)));
    }
}
