use std::sync::Arc;

use ferrobox_domain::api_token::{ApiToken, ApiTokenName};
use ferrobox_domain::user::{User, Username};
use ferrobox_ports::api_token_store::{ApiTokenStore, ApiTokenStoreError};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::{
    generate_api_token_secret, hash_api_token_secret, verify_password,
};

/// Motivos por los que el inicio de sesión puede fallar.
#[derive(Debug, Error)]
pub enum LoginError {
    /// Usuario inexistente o contraseña incorrecta. Se usa un único
    /// mensaje para no filtrar si el nombre de usuario existe.
    #[error("invalid username or password")]
    InvalidCredentials,

    /// Fallo al persistir el token de sesión emitido.
    #[error(transparent)]
    TokenPersistence(#[from] ApiTokenStoreError),

    /// Fallo al consultar el almacén de usuarios.
    #[error(transparent)]
    UserPersistence(#[from] UserStoreError),
}

/// Resultado de un inicio de sesión exitoso: el usuario autenticado y
/// el secreto del token de sesión (mostrado una sola vez).
#[derive(Debug, Clone)]
pub struct LoginResult {
    /// Usuario autenticado.
    pub user: User,
    /// Token de API recién emitido (secreto en claro).
    pub token: ApiToken,
    /// Secreto en claro del token. Solo se expone aquí.
    pub plaintext_secret: String,
}

/// Caso de uso: autenticar con usuario y contraseña, emitiendo un
/// token de API de sesión.
pub struct LoginUseCase {
    user_store: Arc<dyn UserStore>,
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl LoginUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>, api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self {
            user_store,
            api_token_store,
        }
    }

    /// Verifica las credenciales y, si son válidas, emite un token de
    /// sesión llamado `session`.
    ///
    /// # Errors
    ///
    /// Devuelve [`LoginError::InvalidCredentials`] si el usuario no
    /// existe o la contraseña no coincide, o un error de persistencia
    /// si falla el almacén.
    ///
    /// # Panics
    ///
    /// En la práctica, nunca entra en pánico: el literal `"session"`
    /// siempre es un [`ApiTokenName`] válido.
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

        if !verify_password(password, &password_hash) {
            return Err(LoginError::InvalidCredentials);
        }

        let (plaintext_secret, prefix) = generate_api_token_secret();
        let token = ApiToken::new(
            user.id(),
            ApiTokenName::parse("session").expect("literal 'session' is a valid token name"),
            prefix,
        );
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

    use crate::auth_crypto::hash_password;
    use crate::test_support::{InMemoryApiTokenStore, InMemoryUserStore};

    use super::*;

    async fn seeded_stores() -> (Arc<InMemoryUserStore>, Arc<InMemoryApiTokenStore>, User) {
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let user = User::new(Username::parse("admin").unwrap());
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
}
