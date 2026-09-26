use async_trait::async_trait;
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::ids::{ApiTokenId, UserId};
use thiserror::Error;

/// Vista de un token de API con metadatos de persistencia no sensibles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiTokenRecord {
    /// Entidad de dominio del token.
    pub token: ApiToken,
    /// Momento de creación en formato RFC 3339.
    pub created_at_rfc3339: String,
    /// Caducidad en RFC 3339, o `None` si no caduca.
    pub expires_at_rfc3339: Option<String>,
}

/// Motivos por los que una operación de persistencia de tokens puede
/// fallar.
#[derive(Debug, Error)]
pub enum ApiTokenStoreError {
    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la entidad [`ApiToken`].
///
/// El secreto en claro nunca atraviesa este puerto: solo se persiste su
/// hash criptográfico, y la búsqueda de autenticación se hace por ese
/// hash.
#[async_trait]
pub trait ApiTokenStore: Send + Sync {
    /// Guarda un token junto con el hash de su secreto.
    ///
    /// # Errors
    ///
    /// Devuelve [`ApiTokenStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn save(&self, token: &ApiToken, token_hash: &str) -> Result<(), ApiTokenStoreError>;

    /// Busca un token por el hash de su secreto. Devuelve `None` si no
    /// existe.
    ///
    /// # Errors
    ///
    /// Devuelve [`ApiTokenStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiToken>, ApiTokenStoreError>;

    /// Lista los tokens de un usuario, sin secretos.
    ///
    /// # Errors
    ///
    /// Devuelve [`ApiTokenStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn list_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApiTokenRecord>, ApiTokenStoreError>;

    /// Elimina un token perteneciente al usuario indicado. Devuelve
    /// `true` si existía y se borró.
    ///
    /// # Errors
    ///
    /// Devuelve [`ApiTokenStoreError::Backend`] si el backend
    /// subyacente falla.
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
