use std::sync::Arc;

use ferrobox_domain::ids::UserId;
use ferrobox_domain::user::{Email, Role, User, Username, validate_password_policy};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::{PasswordHashError, hash_password};

/// Reasons creating a user can fail.
#[derive(Debug, Error)]
pub enum CreateUserError {
    /// The password does not meet the instance policy.
    #[error(transparent)]
    InvalidPassword(#[from] ferrobox_domain::user::PasswordPolicyError),

    /// Failed to hash the password.
    #[error(transparent)]
    PasswordHashing(#[from] PasswordHashError),

    /// Failed to persist the user (includes a duplicate name or email).
    #[error(transparent)]
    Persistence(#[from] UserStoreError),

    /// A robot account cannot be an instance administrator.
    #[error("a robot account cannot be an admin")]
    RobotCannotBeAdmin,
}

/// Use case: create a user with role, email, and password.
pub struct CreateUserUseCase {
    user_store: Arc<dyn UserStore>,
}

impl CreateUserUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Creates a new user. Email is required and the password must
    /// meet the instance policy.
    ///
    /// # Errors
    ///
    /// Returns [`CreateUserError`] if the password is weak, or if
    /// hashing or persistence fails.
    pub async fn execute(
        &self,
        username: Username,
        email: Email,
        password: &str,
        role: Role,
    ) -> Result<User, CreateUserError> {
        validate_password_policy(password)?;
        let user = User::new(username, role).with_email(Some(email));
        let password_hash = hash_password(password)?;
        self.user_store
            .save_with_password_hash(&user, &password_hash)
            .await?;
        Ok(user)
    }

    /// Creates a robot account: no email, no usable password, API
    /// tokens only. Cannot be [`Role::Admin`].
    ///
    /// # Errors
    ///
    /// [`CreateUserError::RobotCannotBeAdmin`] if the role is admin, or
    /// a hashing / persistence error.
    pub async fn execute_robot(
        &self,
        username: Username,
        role: Role,
    ) -> Result<User, CreateUserError> {
        if role == Role::Admin {
            return Err(CreateUserError::RobotCannotBeAdmin);
        }
        let user = User::new(username, role).with_robot(true);
        let random = format!("R{}aA1", uuid::Uuid::now_v7().simple());
        let password_hash = hash_password(&random)?;
        self.user_store
            .save_with_password_hash(&user, &password_hash)
            .await?;
        Ok(user)
    }

    /// Creates a test user with email `{username}@example.com` and a
    /// password that meets the policy.
    ///
    /// # Errors
    ///
    /// Propagates [`CreateUserError`] if the name is not valid as a
    /// derived email or if persistence fails.
    #[cfg(any(test, feature = "test-utils"))]
    pub async fn seed(&self, username: &str, role: Role) -> Result<User, CreateUserError> {
        let email = Email::parse(format!("{username}@example.com")).map_err(|err| {
            CreateUserError::Persistence(UserStoreError::Backend(Box::new(err)))
        })?;
        let parsed = Username::parse(username).map_err(|err| {
            CreateUserError::Persistence(UserStoreError::Backend(Box::new(err)))
        })?;
        self.execute(parsed, email, "Secret1a", role).await
    }
}

/// Reasons listing users can fail.
#[derive(Debug, Error)]
pub enum ListUsersError {
    /// Failed to query the store.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Use case: list every user.
pub struct ListUsersUseCase {
    user_store: Arc<dyn UserStore>,
}

impl ListUsersUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Lists users ordered by name.
    ///
    /// # Errors
    ///
    /// Returns [`ListUsersError::Persistence`] if the backend fails.
    pub async fn execute(&self) -> Result<Vec<User>, ListUsersError> {
        Ok(self.user_store.find_all().await?)
    }
}

/// Reasons deleting a user can fail.
#[derive(Debug, Error)]
pub enum DeleteUserError {
    /// The user does not exist.
    #[error("user not found")]
    NotFound,

    /// The authenticated user cannot delete themselves.
    #[error("cannot delete your own account")]
    CannotDeleteSelf,

    /// The last administrator of the system cannot be deleted.
    #[error("cannot delete the last admin user")]
    CannotDeleteLastAdmin,

    /// Failed to query / update the store.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Use case: delete a user.
pub struct DeleteUserUseCase {
    user_store: Arc<dyn UserStore>,
}

impl DeleteUserUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Deletes the given user, with safeguards so the system is not
    /// left without administrators and the caller's own account is not
    /// deleted.
    ///
    /// # Errors
    ///
    /// Returns [`DeleteUserError`] if the user does not exist, is the
    /// actor themselves, is the last admin, or if persistence fails.
    pub async fn execute(
        &self,
        actor_id: UserId,
        target_id: UserId,
    ) -> Result<(), DeleteUserError> {
        if actor_id == target_id {
            return Err(DeleteUserError::CannotDeleteSelf);
        }

        let Some(target) = self.user_store.find_by_id(target_id).await? else {
            return Err(DeleteUserError::NotFound);
        };

        if target.role() == Role::Admin && self.user_store.count_admins().await? <= 1 {
            return Err(DeleteUserError::CannotDeleteLastAdmin);
        }

        if self.user_store.delete(target_id).await? {
            Ok(())
        } else {
            Err(DeleteUserError::NotFound)
        }
    }
}

/// Reasons changing a user's role can fail.
#[derive(Debug, Error)]
pub enum ChangeUserRoleError {
    /// The user does not exist.
    #[error("user not found")]
    NotFound,

    /// The last administrator of the system cannot be demoted.
    #[error("cannot demote the last admin user")]
    CannotDemoteLastAdmin,

    /// A robot account cannot be an instance administrator.
    #[error("a robot account cannot be an admin")]
    RobotCannotBeAdmin,

    /// Failed to query / update the store.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Use case: change a user's role.
pub struct ChangeUserRoleUseCase {
    user_store: Arc<dyn UserStore>,
}

impl ChangeUserRoleUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Changes the role of the given user. Demoting the last
    /// administrator is rejected so the system is not left unmanaged.
    ///
    /// # Errors
    ///
    /// Returns [`ChangeUserRoleError`] if the user does not exist, is
    /// the last admin and a demotion is attempted, or if persistence
    /// fails.
    pub async fn execute(
        &self,
        target_id: UserId,
        new_role: Role,
    ) -> Result<User, ChangeUserRoleError> {
        let Some(target) = self.user_store.find_by_id(target_id).await? else {
            return Err(ChangeUserRoleError::NotFound);
        };

        if target.is_robot() && new_role == Role::Admin {
            return Err(ChangeUserRoleError::RobotCannotBeAdmin);
        }

        if target.role() == new_role {
            return Ok(target);
        }

        if target.role() == Role::Admin && self.user_store.count_admins().await? <= 1 {
            return Err(ChangeUserRoleError::CannotDemoteLastAdmin);
        }

        let updated = target.with_role(new_role);
        if self.user_store.update_role(target_id, new_role).await? {
            Ok(updated)
        } else {
            Err(ChangeUserRoleError::NotFound)
        }
    }
}

/// Reasons resetting another user's password can fail.
#[derive(Debug, Error)]
pub enum ResetUserPasswordError {
    /// The caller cannot reset their own password this way.
    #[error("cannot reset your own password; change it in settings")]
    CannotResetSelf,

    /// The user does not exist.
    #[error("user not found")]
    NotFound,

    /// The new password does not meet the instance policy.
    #[error(transparent)]
    InvalidPassword(#[from] ferrobox_domain::user::PasswordPolicyError),

    /// Robot accounts have no password.
    #[error("a robot account has no password")]
    RobotAccount,

    /// Failed to hash the new password.
    #[error(transparent)]
    PasswordHashing(#[from] PasswordHashError),

    /// Failed to query / update the store.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Use case: an Admin resets another account's password.
pub struct ResetUserPasswordUseCase {
    user_store: Arc<dyn UserStore>,
}

impl ResetUserPasswordUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Replaces the password hash of the given user. The actor cannot
    /// reset themselves: that is what the password change in Settings
    /// is for.
    ///
    /// # Errors
    ///
    /// Returns [`ResetUserPasswordError`] if the target is the actor
    /// themselves, does not exist, the password is weak, or
    /// persistence fails.
    pub async fn execute(
        &self,
        actor_id: UserId,
        target_id: UserId,
        new_password: &str,
    ) -> Result<(), ResetUserPasswordError> {
        if actor_id == target_id {
            return Err(ResetUserPasswordError::CannotResetSelf);
        }

        validate_password_policy(new_password)?;

        let Some(target) = self.user_store.find_by_id(target_id).await? else {
            return Err(ResetUserPasswordError::NotFound);
        };
        if target.is_robot() {
            return Err(ResetUserPasswordError::RobotAccount);
        }

        let password_hash = hash_password(new_password)?;
        self.user_store
            .save_with_password_hash(&target, &password_hash)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::test_support::InMemoryUserStore;

    use super::*;

    #[tokio::test]
    async fn create_and_list_users() {
        let store = Arc::new(InMemoryUserStore::default());
        let created = CreateUserUseCase::new(store.clone())
            .seed("dev", Role::Developer)
            .await
            .unwrap();

        assert_eq!(created.role(), Role::Developer);

        let listed = ListUsersUseCase::new(store).execute().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], created);
    }

    #[tokio::test]
    async fn cannot_delete_self() {
        let store = Arc::new(InMemoryUserStore::default());
        let admin = CreateUserUseCase::new(store.clone())
            .seed("admin", Role::Admin)
            .await
            .unwrap();

        let result = DeleteUserUseCase::new(store)
            .execute(admin.id(), admin.id())
            .await;

        assert!(matches!(result, Err(DeleteUserError::CannotDeleteSelf)));
    }

    #[tokio::test]
    async fn cannot_delete_last_admin() {
        let store = Arc::new(InMemoryUserStore::default());
        let create = CreateUserUseCase::new(store.clone());
        let admin = create
            .seed("admin", Role::Admin)
            .await
            .unwrap();
        let other = create
            .seed("dev", Role::Developer)
            .await
            .unwrap();

        let result = DeleteUserUseCase::new(store)
            .execute(other.id(), admin.id())
            .await;

        assert!(matches!(
            result,
            Err(DeleteUserError::CannotDeleteLastAdmin)
        ));
    }

    #[tokio::test]
    async fn deletes_a_non_last_admin_or_developer() {
        let store = Arc::new(InMemoryUserStore::default());
        let create = CreateUserUseCase::new(store.clone());
        let admin = create
            .seed("admin", Role::Admin)
            .await
            .unwrap();
        let other_admin = create
            .seed("admin2", Role::Admin)
            .await
            .unwrap();

        DeleteUserUseCase::new(store.clone())
            .execute(admin.id(), other_admin.id())
            .await
            .unwrap();

        assert_eq!(store.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn change_role_updates_user() {
        let store = Arc::new(InMemoryUserStore::default());
        let created = CreateUserUseCase::new(store.clone())
            .seed("dev", Role::Developer)
            .await
            .unwrap();

        let updated = ChangeUserRoleUseCase::new(store.clone())
            .execute(created.id(), Role::Reader)
            .await
            .unwrap();

        assert_eq!(updated.role(), Role::Reader);
        assert_eq!(
            store
                .find_by_id(created.id())
                .await
                .unwrap()
                .unwrap()
                .role(),
            Role::Reader
        );
    }

    #[tokio::test]
    async fn change_role_same_role_is_a_noop() {
        let store = Arc::new(InMemoryUserStore::default());
        let created = CreateUserUseCase::new(store.clone())
            .seed("dev", Role::Developer)
            .await
            .unwrap();

        let updated = ChangeUserRoleUseCase::new(store)
            .execute(created.id(), Role::Developer)
            .await
            .unwrap();

        assert_eq!(updated.role(), Role::Developer);
    }

    #[tokio::test]
    async fn cannot_demote_last_admin() {
        let store = Arc::new(InMemoryUserStore::default());
        let created = CreateUserUseCase::new(store.clone())
            .seed("only-admin", Role::Admin)
            .await
            .unwrap();

        let err = ChangeUserRoleUseCase::new(store.clone())
            .execute(created.id(), Role::Reader)
            .await
            .unwrap_err();

        assert!(matches!(err, ChangeUserRoleError::CannotDemoteLastAdmin));
        assert_eq!(
            store
                .find_by_id(created.id())
                .await
                .unwrap()
                .unwrap()
                .role(),
            Role::Admin
        );
    }

    #[tokio::test]
    async fn can_demote_admin_when_another_remains() {
        let store = Arc::new(InMemoryUserStore::default());
        let create = CreateUserUseCase::new(store.clone());
        let first = create
            .seed("admin-one", Role::Admin)
            .await
            .unwrap();
        create
            .seed("admin-two", Role::Admin)
            .await
            .unwrap();

        let updated = ChangeUserRoleUseCase::new(store)
            .execute(first.id(), Role::Developer)
            .await
            .unwrap();

        assert_eq!(updated.role(), Role::Developer);
    }

    #[tokio::test]
    async fn change_role_missing_user_is_not_found() {
        let store = Arc::new(InMemoryUserStore::default());
        let err = ChangeUserRoleUseCase::new(store)
            .execute(UserId::new(), Role::Reader)
            .await
            .unwrap_err();

        assert!(matches!(err, ChangeUserRoleError::NotFound));
    }

    #[tokio::test]
    async fn create_rejects_a_weak_password() {
        let store = Arc::new(InMemoryUserStore::default());
        let err = CreateUserUseCase::new(store)
            .execute(
                Username::parse("dev").unwrap(),
                Email::parse("dev@example.com").unwrap(),
                "secret",
                Role::Developer,
            )
            .await
            .unwrap_err();

        assert!(matches!(err, CreateUserError::InvalidPassword(_)));
    }

    #[tokio::test]
    async fn create_rejects_a_duplicate_email() {
        let store = Arc::new(InMemoryUserStore::default());
        let create = CreateUserUseCase::new(store);
        create.seed("ada", Role::Developer).await.unwrap();

        let err = create
            .execute(
                Username::parse("other").unwrap(),
                Email::parse("ada@example.com").unwrap(),
                "Secret1a",
                Role::Reader,
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            CreateUserError::Persistence(UserStoreError::DuplicateEmail(_))
        ));
    }

    #[tokio::test]
    async fn reset_password_allows_login_with_the_new_one() {
        let store = Arc::new(InMemoryUserStore::default());
        let create = CreateUserUseCase::new(store.clone());
        let admin = create.seed("admin", Role::Admin).await.unwrap();
        let dev = create.seed("dev", Role::Developer).await.unwrap();

        ResetUserPasswordUseCase::new(store.clone())
            .execute(admin.id(), dev.id(), "NewPass1a")
            .await
            .unwrap();

        let tokens = Arc::new(crate::test_support::InMemoryApiTokenStore::default());
        let login = crate::login::LoginUseCase::new(store, tokens);
        login
            .execute(Username::parse("dev").unwrap(), "NewPass1a")
            .await
            .unwrap();
        let err = login
            .execute(Username::parse("dev").unwrap(), "Secret1a")
            .await
            .unwrap_err();
        assert!(matches!(err, crate::login::LoginError::InvalidCredentials));
    }

    #[tokio::test]
    async fn cannot_reset_own_password() {
        let store = Arc::new(InMemoryUserStore::default());
        let admin = CreateUserUseCase::new(store.clone())
            .seed("admin", Role::Admin)
            .await
            .unwrap();

        let err = ResetUserPasswordUseCase::new(store)
            .execute(admin.id(), admin.id(), "NewPass1a")
            .await
            .unwrap_err();

        assert!(matches!(err, ResetUserPasswordError::CannotResetSelf));
    }

    #[tokio::test]
    async fn reset_rejects_a_weak_password() {
        let store = Arc::new(InMemoryUserStore::default());
        let create = CreateUserUseCase::new(store.clone());
        let admin = create.seed("admin", Role::Admin).await.unwrap();
        let dev = create.seed("dev", Role::Developer).await.unwrap();

        let err = ResetUserPasswordUseCase::new(store)
            .execute(admin.id(), dev.id(), "weak")
            .await
            .unwrap_err();

        assert!(matches!(err, ResetUserPasswordError::InvalidPassword(_)));
    }

    #[tokio::test]
    async fn create_robot_rejects_admin_role() {
        let store = Arc::new(InMemoryUserStore::default());
        let err = CreateUserUseCase::new(store)
            .execute_robot(Username::parse("ci").unwrap(), Role::Admin)
            .await
            .unwrap_err();
        assert!(matches!(err, CreateUserError::RobotCannotBeAdmin));
    }

    #[tokio::test]
    async fn create_robot_persists_a_developer_without_email() {
        let store = Arc::new(InMemoryUserStore::default());
        let robot = CreateUserUseCase::new(store)
            .execute_robot(Username::parse("ci").unwrap(), Role::Developer)
            .await
            .unwrap();
        assert!(robot.is_robot());
        assert_eq!(robot.role(), Role::Developer);
        assert!(robot.email().is_none());
    }
}
