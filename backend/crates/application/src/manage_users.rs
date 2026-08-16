use std::sync::Arc;

use ferrobox_domain::ids::UserId;
use ferrobox_domain::user::{Email, Role, User, Username, validate_password_policy};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

use crate::auth_crypto::{PasswordHashError, hash_password};

/// Motivos por los que crear un usuario puede fallar.
#[derive(Debug, Error)]
pub enum CreateUserError {
    /// La contraseña no cumple la política de la instancia.
    #[error(transparent)]
    InvalidPassword(#[from] ferrobox_domain::user::PasswordPolicyError),

    /// Fallo al hashear la contraseña.
    #[error(transparent)]
    PasswordHashing(#[from] PasswordHashError),

    /// Fallo al persistir el usuario (incluye nombre o correo duplicado).
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Caso de uso: crear un usuario con rol, correo y contraseña.
pub struct CreateUserUseCase {
    user_store: Arc<dyn UserStore>,
}

impl CreateUserUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Crea un usuario nuevo. El correo es obligatorio y la contraseña
    /// debe cumplir la política de la instancia.
    ///
    /// # Errors
    ///
    /// Devuelve [`CreateUserError`] si la contraseña es débil, o si el
    /// hashing o la persistencia fallan.
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

    /// Crea un usuario de prueba con correo `{username}@example.com` y
    /// una contraseña que cumple la política.
    ///
    /// # Errors
    ///
    /// Propaga [`CreateUserError`] si el nombre no es válido como
    /// correo derivado o si falla la persistencia.
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

/// Motivos por los que listar usuarios puede fallar.
#[derive(Debug, Error)]
pub enum ListUsersError {
    /// Fallo al consultar el almacén.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Caso de uso: listar todos los usuarios.
pub struct ListUsersUseCase {
    user_store: Arc<dyn UserStore>,
}

impl ListUsersUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Lista los usuarios ordenados por nombre.
    ///
    /// # Errors
    ///
    /// Devuelve [`ListUsersError::Persistence`] si el backend falla.
    pub async fn execute(&self) -> Result<Vec<User>, ListUsersError> {
        Ok(self.user_store.find_all().await?)
    }
}

/// Motivos por los que eliminar un usuario puede fallar.
#[derive(Debug, Error)]
pub enum DeleteUserError {
    /// El usuario no existe.
    #[error("user not found")]
    NotFound,

    /// No se puede eliminar el propio usuario autenticado.
    #[error("cannot delete your own account")]
    CannotDeleteSelf,

    /// No se puede eliminar el último administrador del sistema.
    #[error("cannot delete the last admin user")]
    CannotDeleteLastAdmin,

    /// Fallo al consultar / actualizar el almacén.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Caso de uso: eliminar un usuario.
pub struct DeleteUserUseCase {
    user_store: Arc<dyn UserStore>,
}

impl DeleteUserUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Elimina el usuario indicado, con salvaguardas para no dejar el
    /// sistema sin administradores ni borrar la propia cuenta.
    ///
    /// # Errors
    ///
    /// Devuelve [`DeleteUserError`] si el usuario no existe, es el
    /// propio actor, es el último admin, o si falla la persistencia.
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

/// Motivos por los que cambiar el rol de un usuario puede fallar.
#[derive(Debug, Error)]
pub enum ChangeUserRoleError {
    /// El usuario no existe.
    #[error("user not found")]
    NotFound,

    /// No se puede degradar el último administrador del sistema.
    #[error("cannot demote the last admin user")]
    CannotDemoteLastAdmin,

    /// Fallo al consultar / actualizar el almacén.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Caso de uso: cambiar el rol de un usuario.
pub struct ChangeUserRoleUseCase {
    user_store: Arc<dyn UserStore>,
}

impl ChangeUserRoleUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Cambia el rol del usuario indicado. Degradar al último
    /// administrador se rechaza para no dejar el sistema sin gestión.
    ///
    /// # Errors
    ///
    /// Devuelve [`ChangeUserRoleError`] si el usuario no existe, es el
    /// último admin y se intenta degradarlo, o si falla la persistencia.
    pub async fn execute(
        &self,
        target_id: UserId,
        new_role: Role,
    ) -> Result<User, ChangeUserRoleError> {
        let Some(target) = self.user_store.find_by_id(target_id).await? else {
            return Err(ChangeUserRoleError::NotFound);
        };

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

/// Motivos por los que restablecer la contraseña de otro usuario puede
/// fallar.
#[derive(Debug, Error)]
pub enum ResetUserPasswordError {
    /// No se puede restablecer la propia contraseña por esta vía.
    #[error("cannot reset your own password; change it in settings")]
    CannotResetSelf,

    /// El usuario no existe.
    #[error("user not found")]
    NotFound,

    /// La nueva contraseña no cumple la política de la instancia.
    #[error(transparent)]
    InvalidPassword(#[from] ferrobox_domain::user::PasswordPolicyError),

    /// Fallo al hashear la nueva contraseña.
    #[error(transparent)]
    PasswordHashing(#[from] PasswordHashError),

    /// Fallo al consultar / actualizar el almacén.
    #[error(transparent)]
    Persistence(#[from] UserStoreError),
}

/// Caso de uso: un Admin restablece la contraseña de otra cuenta.
pub struct ResetUserPasswordUseCase {
    user_store: Arc<dyn UserStore>,
}

impl ResetUserPasswordUseCase {
    /// Construye el caso de uso a partir de su puerto.
    #[must_use]
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }

    /// Sustituye el hash de contraseña del usuario indicado. El actor
    /// no puede restablecerse a sí mismo: para eso está el cambio de
    /// contraseña en Configuración.
    ///
    /// # Errors
    ///
    /// Devuelve [`ResetUserPasswordError`] si el objetivo es el propio
    /// actor, no existe, la contraseña es débil, o falla la
    /// persistencia.
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
}
