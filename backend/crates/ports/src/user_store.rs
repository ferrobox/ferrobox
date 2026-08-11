use async_trait::async_trait;
use ferrobox_domain::ids::UserId;
use ferrobox_domain::user::{User, Username};
use thiserror::Error;

/// Motivos por los que una operación de persistencia de usuarios puede
/// fallar.
#[derive(Debug, Error)]
pub enum UserStoreError {
    /// Ya existe un usuario con ese nombre.
    #[error("a user named '{0}' already exists")]
    DuplicateUsername(Username),

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de la entidad [`User`] y de sus credenciales.
///
/// El hash de la contraseña viaja junto al usuario en las operaciones
/// de escritura / lectura de autenticación, pero no forma parte de la
/// entidad de dominio: es un detalle de credenciales.
#[async_trait]
pub trait UserStore: Send + Sync {
    /// Guarda un usuario junto con el hash de su contraseña.
    ///
    /// # Errors
    ///
    /// Devuelve [`UserStoreError::DuplicateUsername`] si ya existe otro
    /// usuario con el mismo nombre, o
    /// [`UserStoreError::Backend`] si el backend subyacente falla.
    async fn save_with_password_hash(
        &self,
        user: &User,
        password_hash: &str,
    ) -> Result<(), UserStoreError>;

    /// Busca un usuario por su identificador.
    ///
    /// # Errors
    ///
    /// Devuelve [`UserStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, UserStoreError>;

    /// Busca un usuario por su nombre, junto con el hash de su
    /// contraseña. Devuelve `None` si no existe.
    ///
    /// # Errors
    ///
    /// Devuelve [`UserStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn find_by_username_with_password_hash(
        &self,
        username: &Username,
    ) -> Result<Option<(User, String)>, UserStoreError>;

    /// Lista todos los usuarios, ordenados por nombre.
    ///
    /// # Errors
    ///
    /// Devuelve [`UserStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn find_all(&self) -> Result<Vec<User>, UserStoreError>;

    /// Elimina un usuario. No es un error eliminar un identificador
    /// que no existe; en ese caso devuelve `false`.
    ///
    /// # Errors
    ///
    /// Devuelve [`UserStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn delete(&self, id: UserId) -> Result<bool, UserStoreError>;

    /// Cuenta cuántos usuarios existen. Se usa al arrancar para decidir
    /// si hay que crear el administrador inicial.
    ///
    /// # Errors
    ///
    /// Devuelve [`UserStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn count(&self) -> Result<u64, UserStoreError>;

    /// Cuenta cuántos usuarios tienen rol de administrador.
    ///
    /// # Errors
    ///
    /// Devuelve [`UserStoreError::Backend`] si el backend subyacente
    /// falla.
    async fn count_admins(&self) -> Result<u64, UserStoreError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ferrobox_domain::user::Role;

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
}
