use std::sync::Arc;

use ferrobox_domain::user::{Username, validate_password_policy};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::{PasswordHashError, hash_password, verify_password};

/// Motivos por los que cambiar la contraseña puede fallar.
#[derive(Debug, Error)]
pub enum ChangePasswordError {
    /// La nueva contraseña no cumple la política de la instancia.
    #[error(transparent)]
    InvalidPassword(#[from] ferrobox_domain::user::PasswordPolicyError),

    /// La contraseña actual no coincide.
    #[error("current password is incorrect")]
    InvalidCurrentPassword,

    /// Las cuentas robot no tienen contraseña.
    #[error("a robot account has no password")]
    RobotAccount,

    /// El usuario autenticado ya no existe.
    #[error("user not found")]
    NotFound,

    /// Fallo al hashear la nueva contraseña.
    #[error(transparent)]
    PasswordHashing(#[from] PasswordHashError),

    /// Fallo al consultar o persistir el almacén de usuarios.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Caso de uso: el usuario autenticado cambia su propia contraseña.
pub struct ChangePasswordUseCase {
    user_store: Arc<dyn UserStore>,
}

impl ChangePasswordUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Verifica la contraseña actual y sustituye el hash almacenado.
    ///
    /// # Errors
    ///
    /// Devuelve [`ChangePasswordError`] si la nueva contraseña no
    /// cumple la política, la actual no coincide, el usuario no
    /// existe, o fallan el hashing o la persistencia.
    pub async fn execute(
        &self,
        username: &Username,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), ChangePasswordError> {
        validate_password_policy(new_password)?;

        let Some((user, password_hash)) = self
            .user_store
            .find_by_username_with_password_hash(username)
            .await?
        else {
            return Err(ChangePasswordError::NotFound);
        };

        if user.is_robot() {
            return Err(ChangePasswordError::RobotAccount);
        }

        if !verify_password(current_password, &password_hash) {
            return Err(ChangePasswordError::InvalidCurrentPassword);
        }

        let new_hash = hash_password(new_password)?;
        self.user_store
            .save_with_password_hash(&user, &new_hash)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::user::{Role, User, Username};

    use crate::auth_crypto::hash_password;
    use crate::login::{LoginError, LoginUseCase};
    use crate::test_support::{InMemoryApiTokenStore, InMemoryUserStore};

    use super::*;

    async fn seeded_user(password: &str) -> (Arc<InMemoryUserStore>, Username) {
        let user_store = Arc::new(InMemoryUserStore::default());
        let username = Username::parse("admin").unwrap();
        let user = User::new(username.clone(), Role::Admin);
        let hash = hash_password(password).unwrap();
        user_store
            .save_with_password_hash(&user, &hash)
            .await
            .unwrap();
        (user_store, username)
    }

    #[tokio::test]
    async fn changing_password_allows_login_with_the_new_one() {
        let (user_store, username) = seeded_user("old-secret").await;
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());

        ChangePasswordUseCase::new(user_store.clone())
            .execute(&username, "old-secret", "NewSecret1")
            .await
            .unwrap();

        let login = LoginUseCase::new(user_store, api_token_store);
        login
            .execute(username.clone(), "NewSecret1")
            .await
            .unwrap();

        let err = login.execute(username, "old-secret").await.unwrap_err();
        assert!(matches!(err, LoginError::InvalidCredentials));
    }

    #[tokio::test]
    async fn wrong_current_password_is_rejected_and_hash_is_unchanged() {
        let (user_store, username) = seeded_user("old-secret").await;

        let err = ChangePasswordUseCase::new(user_store.clone())
            .execute(&username, "nope", "NewSecret1")
            .await
            .unwrap_err();

        assert!(matches!(err, ChangePasswordError::InvalidCurrentPassword));

        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        LoginUseCase::new(user_store, api_token_store)
            .execute(username, "old-secret")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn weak_new_password_is_rejected() {
        let (user_store, username) = seeded_user("old-secret").await;

        let err = ChangePasswordUseCase::new(user_store)
            .execute(&username, "old-secret", "")
            .await
            .unwrap_err();

        assert!(matches!(err, ChangePasswordError::InvalidPassword(_)));
    }

    #[tokio::test]
    async fn unknown_user_is_not_found() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let err = ChangePasswordUseCase::new(user_store)
            .execute(&Username::parse("ghost").unwrap(), "x", "NewSecret1")
            .await
            .unwrap_err();

        assert!(matches!(err, ChangePasswordError::NotFound));
    }
}
