use async_trait::async_trait;
use ferrobox_domain::ids::UserId;
use ferrobox_domain::oidc::OidcIdentity;
use ferrobox_domain::user::{Email, Role, User, Username};
use thiserror::Error;

/// Reasons a user persistence operation can fail.
#[derive(Debug, Error)]
pub enum UserStoreError {
    /// A user with that name already exists.
    #[error("a user named '{0}' already exists")]
    DuplicateUsername(Username),

    /// A user with that email already exists.
    #[error("a user with email '{0}' already exists")]
    DuplicateEmail(Email),

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for the [`User`] entity and its credentials.
///
/// The password hash travels with the user in authentication write /
/// read operations, but it is not part of the domain entity: it is a
/// credentials detail.
#[async_trait]
pub trait UserStore: Send + Sync {
    /// Saves a user together with their password hash.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::DuplicateUsername`] or
    /// [`UserStoreError::DuplicateEmail`] if another user with the same
    /// name or email already exists, or [`UserStoreError::Backend`] if
    /// the underlying backend fails.
    async fn save_with_password_hash(
        &self,
        user: &User,
        password_hash: &str,
    ) -> Result<(), UserStoreError>;

    /// Looks up a user by identifier.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, UserStoreError>;

    /// Looks up a user by name, together with their password hash.
    /// Returns `None` if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_username_with_password_hash(
        &self,
        username: &Username,
    ) -> Result<Option<(User, String)>, UserStoreError>;

    /// Looks up a user by email. Returns `None` if they do not exist or
    /// the email is empty.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_email(&self, email: &Email) -> Result<Option<User>, UserStoreError>;

    /// Looks up the user linked to an `OIDC` identity.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_oidc(
        &self,
        identity: &OidcIdentity,
    ) -> Result<Option<User>, UserStoreError>;

    /// Looks up a user by identifier, together with their password hash.
    /// Returns `None` if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_by_id_with_password_hash(
        &self,
        id: UserId,
    ) -> Result<Option<(User, String)>, UserStoreError>;

    /// Lists every user, ordered by name.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn find_all(&self) -> Result<Vec<User>, UserStoreError>;

    /// Deletes a user. Deleting a missing identifier is not an error;
    /// in that case it returns `false`.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn delete(&self, id: UserId) -> Result<bool, UserStoreError>;

    /// Replaces the role of an existing user. Returns `false` if the
    /// identifier does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn update_role(&self, id: UserId, role: Role) -> Result<bool, UserStoreError>;

    /// Counts how many users exist. Used at startup to decide whether
    /// the initial administrator must be created.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn count(&self) -> Result<u64, UserStoreError>;

    /// Counts how many users have the administrator role.
    ///
    /// # Errors
    ///
    /// Returns [`UserStoreError::Backend`] if the underlying backend
    /// fails.
    async fn count_admins(&self) -> Result<u64, UserStoreError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ferrobox_domain::user::{Email, Role};

    use super::*;

    #[derive(Default)]
    struct InMemoryUserStore {
        users: Mutex<HashMap<UserId, (User, String)>>,
    }

    #[async_trait]
    impl UserStore for InMemoryUserStore {
        async fn save_with_password_hash(
            &self,
            user: &User,
            password_hash: &str,
        ) -> Result<(), UserStoreError> {
            let mut users = self.users.lock().unwrap();

            let name_taken_by_another = users.values().any(|(existing, _)| {
                existing.id() != user.id() && existing.username() == user.username()
            });

            if name_taken_by_another {
                return Err(UserStoreError::DuplicateUsername(user.username().clone()));
            }

            if let Some(email) = user.email() {
                let email_taken_by_another = users.values().any(|(existing, _)| {
                    existing.id() != user.id() && existing.email() == Some(email)
                });
                if email_taken_by_another {
                    return Err(UserStoreError::DuplicateEmail(email.clone()));
                }
            }

            users.insert(user.id(), (user.clone(), password_hash.to_string()));
            Ok(())
        }

        async fn find_by_id(&self, id: UserId) -> Result<Option<User>, UserStoreError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .get(&id)
                .map(|(user, _)| user.clone()))
        }

        async fn find_by_username_with_password_hash(
            &self,
            username: &Username,
        ) -> Result<Option<(User, String)>, UserStoreError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .values()
                .find(|(user, _)| user.username() == username)
                .map(|(user, hash)| (user.clone(), hash.clone())))
        }

        async fn find_by_email(&self, email: &Email) -> Result<Option<User>, UserStoreError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .values()
                .find(|(user, _)| user.email() == Some(email))
                .map(|(user, _)| user.clone()))
        }

        async fn find_by_oidc(
            &self,
            identity: &OidcIdentity,
        ) -> Result<Option<User>, UserStoreError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .values()
                .find(|(user, _)| user.oidc() == Some(identity))
                .map(|(user, _)| user.clone()))
        }

        async fn find_by_id_with_password_hash(
            &self,
            id: UserId,
        ) -> Result<Option<(User, String)>, UserStoreError> {
            Ok(self.users.lock().unwrap().get(&id).cloned())
        }

        async fn find_all(&self) -> Result<Vec<User>, UserStoreError> {
            let mut users: Vec<_> = self
                .users
                .lock()
                .unwrap()
                .values()
                .map(|(user, _)| user.clone())
                .collect();
            users.sort_by(|a, b| a.username().as_str().cmp(b.username().as_str()));
            Ok(users)
        }

        async fn delete(&self, id: UserId) -> Result<bool, UserStoreError> {
            Ok(self.users.lock().unwrap().remove(&id).is_some())
        }

        async fn update_role(&self, id: UserId, role: Role) -> Result<bool, UserStoreError> {
            let mut users = self.users.lock().unwrap();
            let Some((user, hash)) = users.remove(&id) else {
                return Ok(false);
            };
            users.insert(id, (user.with_role(role), hash));
            Ok(true)
        }

        async fn count(&self) -> Result<u64, UserStoreError> {
            Ok(self.users.lock().unwrap().len() as u64)
        }

        async fn count_admins(&self) -> Result<u64, UserStoreError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .values()
                .filter(|(user, _)| user.role() == Role::Admin)
                .count() as u64)
        }
    }

    #[tokio::test]
    async fn save_then_find_by_username_returns_user_and_hash() {
        let store = InMemoryUserStore::default();
        let user = User::new(Username::parse("admin").unwrap(), Role::Admin);

        store
            .save_with_password_hash(&user, "hash-admin")
            .await
            .unwrap();

        let found = store
            .find_by_username_with_password_hash(user.username())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found.0, user);
        assert_eq!(found.0.role(), Role::Admin);
        assert_eq!(found.1, "hash-admin");
    }

    #[tokio::test]
    async fn saving_a_duplicate_username_is_rejected() {
        let store = InMemoryUserStore::default();
        store
            .save_with_password_hash(
                &User::new(Username::parse("admin").unwrap(), Role::Admin),
                "a",
            )
            .await
            .unwrap();

        let result = store
            .save_with_password_hash(
                &User::new(Username::parse("admin").unwrap(), Role::Developer),
                "b",
            )
            .await;

        assert!(matches!(result, Err(UserStoreError::DuplicateUsername(_))));
    }

    #[tokio::test]
    async fn saving_a_duplicate_email_is_rejected() {
        let store = InMemoryUserStore::default();
        let email = Email::parse("ada@example.com").unwrap();
        store
            .save_with_password_hash(
                &User::new(Username::parse("ada").unwrap(), Role::Admin).with_email(Some(email.clone())),
                "a",
            )
            .await
            .unwrap();

        let result = store
            .save_with_password_hash(
                &User::new(Username::parse("other").unwrap(), Role::Developer)
                    .with_email(Some(email)),
                "b",
            )
            .await;

        assert!(matches!(result, Err(UserStoreError::DuplicateEmail(_))));
    }

    #[tokio::test]
    async fn count_and_count_admins_reflect_saved_users() {
        let store = InMemoryUserStore::default();
        assert_eq!(store.count().await.unwrap(), 0);

        store
            .save_with_password_hash(
                &User::new(Username::parse("admin").unwrap(), Role::Admin),
                "a",
            )
            .await
            .unwrap();
        store
            .save_with_password_hash(
                &User::new(Username::parse("dev").unwrap(), Role::Developer),
                "b",
            )
            .await
            .unwrap();

        assert_eq!(store.count().await.unwrap(), 2);
        assert_eq!(store.count_admins().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn find_all_orders_by_username() {
        let store = InMemoryUserStore::default();
        store
            .save_with_password_hash(
                &User::new(Username::parse("zoe").unwrap(), Role::Reader),
                "a",
            )
            .await
            .unwrap();
        store
            .save_with_password_hash(
                &User::new(Username::parse("ada").unwrap(), Role::Admin),
                "b",
            )
            .await
            .unwrap();

        let all = store.find_all().await.unwrap();
        assert_eq!(all[0].username().as_str(), "ada");
        assert_eq!(all[1].username().as_str(), "zoe");
    }

    #[tokio::test]
    async fn update_role_replaces_authorization_and_preserves_password_hash() {
        let store = InMemoryUserStore::default();
        let user = User::new(Username::parse("dev").unwrap(), Role::Developer);
        store
            .save_with_password_hash(&user, "hash-dev")
            .await
            .unwrap();

        assert!(store.update_role(user.id(), Role::Reader).await.unwrap());

        let found = store.find_by_id(user.id()).await.unwrap().unwrap();
        assert_eq!(found.role(), Role::Reader);

        let with_hash = store
            .find_by_username_with_password_hash(user.username())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(with_hash.1, "hash-dev");
    }

    #[tokio::test]
    async fn update_role_missing_user_returns_false() {
        let store = InMemoryUserStore::default();
        assert!(!store.update_role(UserId::new(), Role::Admin).await.unwrap());
    }
}
