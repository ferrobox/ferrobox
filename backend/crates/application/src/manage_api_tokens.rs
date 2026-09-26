use std::sync::Arc;

use chrono::{DateTime, Utc};
use ferrobox_domain::api_token::{ApiToken, ApiTokenName};
use ferrobox_domain::ids::UserId;
use ferrobox_ports::api_token_store::{ApiTokenRecord, ApiTokenStore, ApiTokenStoreError};
use thiserror::Error;

use crate::auth_crypto::{generate_api_token_secret, hash_api_token_secret};

/// Motivos por los que crear un token de API puede fallar.
#[derive(Debug, Error)]
pub enum CreateApiTokenError {
    /// La caducidad está en el pasado.
    #[error("token expiry must be in the future")]
    ExpiryInThePast,

    /// Fallo al persistir el token.
    #[error(transparent)]
    Persistence(#[from] ApiTokenStoreError),
}

/// Resultado de crear un token: metadatos + secreto en claro (una sola
/// vez).
#[derive(Debug, Clone)]
pub struct CreateApiTokenResult {
    /// Token recién creado.
    pub token: ApiToken,
    /// Secreto en claro. Solo se expone aquí.
    pub plaintext_secret: String,
}

/// Caso de uso: emitir un nuevo token de API para un usuario.
pub struct CreateApiTokenUseCase {
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl CreateApiTokenUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self { api_token_store }
    }

    /// Emite un token con el nombre indicado para el usuario dado.
    ///
    /// # Errors
    ///
    /// Devuelve [`CreateApiTokenError::Persistence`] si el backend
    /// falla.
    pub async fn execute(
        &self,
        user_id: UserId,
        name: ApiTokenName,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<CreateApiTokenResult, CreateApiTokenError> {
        if expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(CreateApiTokenError::ExpiryInThePast);
        }
        let (plaintext_secret, prefix) = generate_api_token_secret();
        let token = ApiToken::new(user_id, name, prefix).with_expires_at(expires_at);
        let token_hash = hash_api_token_secret(&plaintext_secret);
        self.api_token_store.save(&token, &token_hash).await?;

        Ok(CreateApiTokenResult {
            token,
            plaintext_secret,
        })
    }
}

/// Motivos por los que listar tokens puede fallar.
#[derive(Debug, Error)]
pub enum ListApiTokensError {
    /// Fallo al consultar el almacén.
    #[error(transparent)]
    Persistence(#[from] ApiTokenStoreError),
}

/// Caso de uso: listar los tokens de API de un usuario.
pub struct ListApiTokensUseCase {
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl ListApiTokensUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self { api_token_store }
    }

    /// Lista los tokens del usuario, sin secretos.
    ///
    /// # Errors
    ///
    /// Devuelve [`ListApiTokensError::Persistence`] si el backend
    /// falla.
    pub async fn execute(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApiTokenRecord>, ListApiTokensError> {
        Ok(self.api_token_store.list_for_user(user_id).await?)
    }
}

/// Motivos por los que revocar un token puede fallar.
#[derive(Debug, Error)]
pub enum RevokeApiTokenError {
    /// El token no existe o no pertenece al usuario.
    #[error("API token not found")]
    NotFound,

    /// Fallo al consultar / actualizar el almacén.
    #[error(transparent)]
    Persistence(#[from] ApiTokenStoreError),
}

/// Caso de uso: revocar (borrar) un token de API propio.
pub struct RevokeApiTokenUseCase {
    api_token_store: Arc<dyn ApiTokenStore>,
}

impl RevokeApiTokenUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(api_token_store: Arc<dyn ApiTokenStore>) -> Self {
        Self { api_token_store }
    }

    /// Revoca el token indicado si pertenece al usuario.
    ///
    /// # Errors
    ///
    /// Devuelve [`RevokeApiTokenError::NotFound`] si el token no
    /// existe o no es del usuario, o un error de persistencia si el
    /// backend falla.
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
    async fn revoke_of_unknown_token_is_not_found() {
        let store = Arc::new(InMemoryApiTokenStore::default());
        let result = RevokeApiTokenUseCase::new(store)
            .execute(UserId::new(), ferrobox_domain::ids::ApiTokenId::new())
            .await;

        assert!(matches!(result, Err(RevokeApiTokenError::NotFound)));
    }
}
