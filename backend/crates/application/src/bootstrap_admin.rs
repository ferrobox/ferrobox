use std::sync::Arc;

use ferrobox_domain::user::{Role, User, Username};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::{hash_password, PasswordHashError};

/// Reasons bootstrapping the initial administrator can fail.
#[derive(Debug, Error)]
pub enum BootstrapAdminError {
    /// Failed to hash the administrator password.
    #[error(transparent)]
    PasswordHashing(#[from] PasswordHashError),

    /// Failed to persist the administrator user.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Result of attempting to create the initial administrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapAdminOutcome {
    /// At least one user already existed: nothing was created.
    AlreadyInitialized,
    /// The administrator was created with the given credentials.
    Created,
}

/// Use case: create the administrator user the first time the system
/// starts with no users.
pub struct BootstrapAdminUseCase {
    user_store: Arc<dyn UserStore>,
}

impl BootstrapAdminUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// If there are no users, creates one with role [`Role::Admin`] and
    /// the given credentials. If users already exist, does nothing.
    ///
    /// # Errors
    ///
    /// Returns [`BootstrapAdminError`] if hashing or persistence fails.
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
