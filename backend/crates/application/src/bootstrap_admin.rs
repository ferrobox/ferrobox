use std::sync::Arc;

use ferrobox_domain::user::{Role, User, Username};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::{hash_password, PasswordHashError};

/// Motivos por los que el arranque del administrador inicial puede
/// fallar.
#[derive(Debug, Error)]
pub enum BootstrapAdminError {
    /// Fallo al hashear la contraseña del administrador.
    #[error(transparent)]
    PasswordHashing(#[from] PasswordHashError),

    /// Fallo al persistir el usuario administrador.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Resultado de intentar crear el administrador inicial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapAdminOutcome {
    /// Ya existía al menos un usuario: no se creó nada.
    AlreadyInitialized,
    /// Se creó el administrador con las credenciales indicadas.
    Created,
}

/// Caso de uso: crear el usuario administrador la primera vez que el
/// sistema arranca sin usuarios.
pub struct BootstrapAdminUseCase {
    user_store: Arc<dyn UserStore>,
}

impl BootstrapAdminUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Si no hay usuarios, crea uno con rol [`Role::Admin`] y las
    /// credenciales indicadas. Si ya hay usuarios, no hace nada.
    ///
    /// # Errors
    ///
    /// Devuelve [`BootstrapAdminError`] si el hashing o la
    /// persistencia fallan.
    pub async fn execute(
        &self,
        username: Username,
        password: &str,
    ) -> Result<BootstrapAdminOutcome, BootstrapAdminError> {
        if self.user_store.count().await? > 0 {
            return Ok(BootstrapAdminOutcome::AlreadyInitialized);
        }

        let user = User::new(username, Role::Admin);
        let password_hash = hash_password(password)?;
        self.user_store
            .save_with_password_hash(&user, &password_hash)
            .await?;

        Ok(BootstrapAdminOutcome::Created)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::test_support::InMemoryUserStore;

    use super::*;

    #[tokio::test]
    async fn creates_admin_when_store_is_empty() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let use_case = BootstrapAdminUseCase::new(user_store.clone());

        let outcome = use_case
            .execute(Username::parse("admin").unwrap(), "admin")
            .await
            .unwrap();

        assert_eq!(outcome, BootstrapAdminOutcome::Created);
        assert_eq!(user_store.count().await.unwrap(), 1);
        let admin = user_store.find_all().await.unwrap().pop().unwrap();
        assert_eq!(admin.role(), Role::Admin);
    }

    #[tokio::test]
    async fn does_nothing_when_users_already_exist() {
        let user_store = Arc::new(InMemoryUserStore::default());
        let use_case = BootstrapAdminUseCase::new(user_store.clone());

        use_case
            .execute(Username::parse("admin").unwrap(), "admin")
            .await
            .unwrap();

        let outcome = use_case
            .execute(Username::parse("other").unwrap(), "other")
            .await
            .unwrap();

        assert_eq!(outcome, BootstrapAdminOutcome::AlreadyInitialized);
        assert_eq!(user_store.count().await.unwrap(), 1);
    }
}
