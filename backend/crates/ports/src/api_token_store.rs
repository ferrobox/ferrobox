use async_trait::async_trait;
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::ids::{ApiTokenId, UserId};
use thiserror::Error;

/// View of an API token with non-sensitive persistence metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiTokenRecord {
    /// Domain entity of the token.
    pub token: ApiToken,
    /// Creation time in RFC 3339 format.
    pub created_at_rfc3339: String,
    /// Expiry in RFC 3339, or `None` if it does not expire.
    pub expires_at_rfc3339: Option<String>,
}

/// Reasons a token persistence operation can fail.
#[derive(Debug, Error)]
pub enum ApiTokenStoreError {
    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for the [`ApiToken`] entity.
///
/// The plaintext secret never crosses this port: only its cryptographic
/// hash is persisted, and authentication lookup is done by that hash.
#[async_trait]
pub trait ApiTokenStore: Send + Sync {
    /// Saves a token together with the hash of its secret.
    ///
    /// # Errors
    ///
    /// Returns [`ApiTokenStoreError::Backend`] if the underlying
    /// backend fails.
    async fn save(&self, token: &ApiToken, token_hash: &str) -> Result<(), ApiTokenStoreError>;

    /// Looks up a token by the hash of its secret. Returns `None` if it
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`ApiTokenStoreError::Backend`] if the underlying
    /// backend fails.
    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiToken>, ApiTokenStoreError>;

    /// Lists a user's tokens, without secrets.
    ///
    /// # Errors
    ///
    /// Returns [`ApiTokenStoreError::Backend`] if the underlying
    /// backend fails.
    async fn list_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApiTokenRecord>, ApiTokenStoreError>;

    /// Deletes a token belonging to the given user. Returns `true` if
    /// it existed and was removed.
    ///
    /// # Errors
    ///
    /// Returns [`ApiTokenStoreError::Backend`] if the underlying
    /// backend fails.
    async fn delete_for_user(
        &self,
        token_id: ApiTokenId,
        user_id: UserId,
    ) -> Result<bool, ApiTokenStoreError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::user::{Role, User, Username};

    use super::*;

    #[derive(Default)]
    struct InMemoryApiTokenStore {
        tokens: Mutex<HashMap<ApiTokenId, (ApiToken, String, String)>>,
    }

    #[async_trait]
    impl ApiTokenStore for InMemoryApiTokenStore {
        async fn save(
            &self,
            token: &ApiToken,
            token_hash: &str,
        ) -> Result<(), ApiTokenStoreError> {
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

    #[tokio::test]
    async fn save_then_find_by_hash_returns_the_token() {
        let store = InMemoryApiTokenStore::default();
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);
        let token = ApiToken::new(
            user.id(),
            ApiTokenName::parse("session").unwrap(),
            "fb_abcd".to_string(),
        );

        store.save(&token, "hash-1").await.unwrap();

        assert_eq!(
            store.find_by_token_hash("hash-1").await.unwrap(),
            Some(token)
        );
    }

    #[tokio::test]
    async fn delete_for_user_only_removes_owned_tokens() {
        let store = InMemoryApiTokenStore::default();
        let owner = User::new(Username::parse("admin").unwrap(), Role::Admin);
        let other = User::new(Username::parse("other").unwrap(), Role::Developer);
        let token = ApiToken::new(
            owner.id(),
            ApiTokenName::parse("session").unwrap(),
            "fb_abcd".to_string(),
        );

        store.save(&token, "hash-1").await.unwrap();

        assert!(!store
            .delete_for_user(token.id(), other.id())
            .await
            .unwrap());
        assert!(store
            .delete_for_user(token.id(), owner.id())
            .await
            .unwrap());
        assert_eq!(store.find_by_token_hash("hash-1").await.unwrap(), None);
    }
}
