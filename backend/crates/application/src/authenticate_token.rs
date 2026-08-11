use std::sync::Arc;

use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::user::User;
use ferrobox_ports::api_token_store::{ApiTokenStore, ApiTokenStoreError};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::hash_api_token_secret;

/// Motivos por los que autenticar un secreto de token puede fallar.
#[derive(Debug, Error)]
pub enum AuthenticateTokenError {
    /// El secreto no corresponde a ningún token conocido.
    #[error("invalid or revoked API token")]
    InvalidToken,

    /// Fallo al consultar el almacén de tokens.
    #[error(transparent)]
    TokenPersistence(#[from] ApiTokenStoreError),

    /// Fallo al consultar el almacén de usuarios.
    #[error(transparent)]
    UserPersistence(#[from] UserStoreError),
}

/// Resultado de autenticar un secreto de token: el usuario y el token
/// asociados.
#[derive(Debug, Clone)]
pub struct AuthenticatedPrincipal {
    /// Usuario dueño del token.
    pub user: User,
    /// Token que autenticó la petición.
    pub token: ApiToken,
}

/// Caso de uso: resolver un secreto Bearer / Token a un principal
/// autenticado.
pub struct AuthenticateTokenUseCase {
    user_store: Arc<dyn UserStore>,
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl AuthenticateTokenUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>, api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self {
            user_store,
            api_token_store,
        }
    }

    /// Hashea el secreto, busca el token y carga su usuario.
    ///
    /// # Errors
    ///
    /// Devuelve [`AuthenticateTokenError::InvalidToken`] si el secreto
    /// no existe o el usuario asociado ha desaparecido, o un error de
    /// persistencia si el backend falla.
    pub async fn execute(
        &self,
        plaintext_secret: &str,
    ) -> Result<AuthenticatedPrincipal, AuthenticateTokenError> {
        let token_hash = hash_api_token_secret(plaintext_secret);
        let Some(token) = self.api_token_store.find_by_token_hash(&token_hash).await? else {
            return Err(AuthenticateTokenError::InvalidToken);
        };

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
    use ferrobox_domain::user::Username;

    use crate::auth_crypto::{generate_api_token_secret, hash_api_token_secret, hash_password};
    use crate::test_support::{InMemoryApiTokenStore, InMemoryUserStore};

    use super::*;

    #[tokio::test]
    async fn authenticates_a_valid_secret() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap());
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
}
