use async_trait::async_trait;
use ferrobox_domain::ids::UserId;
use ferrobox_domain::user::{Role, User, Username};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Adaptador de [`UserStore`] contra `PostgreSQL`.
pub struct PostgresUserStore {
    pool: PgPool,
}

impl PostgresUserStore {
    /// Construye el adaptador a partir de un `pool` de conexiones ya
    /// configurado.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> UserStoreError {
    UserStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn translate_save_error(username: &Username, err: &sqlx::Error) -> UserStoreError {
    if let sqlx::Error::Database(db_err) = err
        && db_err.constraint() == Some("users_username_key")
    {
        return UserStoreError::DuplicateUsername(username.clone());
    }

    backend_error(err.to_string())
}

fn row_to_user(id: Uuid, username: String, role: &str) -> Result<User, UserStoreError> {
    let username = Username::parse(username).map_err(|err| backend_error(err.to_string()))?;
    let role = Role::parse(role).map_err(|err| backend_error(err.to_string()))?;
    Ok(User::from_parts(UserId::from(id), username, role))
}

#[async_trait]
impl UserStore for PostgresUserStore {
    async fn save_with_password_hash(
        &self,
        user: &User,
        password_hash: &str,
    ) -> Result<(), UserStoreError> {
        let id: Uuid = user.id().into();

        sqlx::query!(
            r#"
            INSERT INTO users (id, username, password_hash, role)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO UPDATE
            SET username = EXCLUDED.username,
                password_hash = EXCLUDED.password_hash,
                role = EXCLUDED.role
            "#,
            id,
            user.username().as_str(),
            password_hash,
            user.role().as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(|err| translate_save_error(user.username(), &err))?;

        Ok(())
    }

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, UserStoreError> {
        let id: Uuid = id.into();

        let row = sqlx::query!(
            r#"
            SELECT id, username, role
            FROM users
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|row| row_to_user(row.id, row.username, &row.role))
            .transpose()
    }

    async fn find_by_username_with_password_hash(
        &self,
        username: &Username,
    ) -> Result<Option<(User, String)>, UserStoreError> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, password_hash, role
            FROM users
            WHERE username = $1
            "#,
            username.as_str(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|row| {
            let user = row_to_user(row.id, row.username, &row.role)?;
            Ok((user, row.password_hash))
        })
        .transpose()
    }

    async fn find_all(&self) -> Result<Vec<User>, UserStoreError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, username, role
            FROM users
            ORDER BY username ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| row_to_user(row.id, row.username, &row.role))
            .collect()
    }

    async fn delete(&self, id: UserId) -> Result<bool, UserStoreError> {
        let id: Uuid = id.into();

        let result = sqlx::query!(
            r#"
            DELETE FROM users
            WHERE id = $1
            "#,
            id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_role(&self, id: UserId, role: Role) -> Result<bool, UserStoreError> {
        let id: Uuid = id.into();

        let result = sqlx::query!(
            r#"
            UPDATE users
            SET role = $2
            WHERE id = $1
            "#,
            id,
            role.as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn count(&self) -> Result<u64, UserStoreError> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM users
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(u64::try_from(row.count).unwrap_or(0))
    }

    async fn count_admins(&self) -> Result<u64, UserStoreError> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM users
            WHERE role = 'admin'
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(u64::try_from(row.count).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::user::{Role, Username};
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    async fn pool_from_env() -> PgPool {
        let database_url =
            std::env::var("DATABASE_URL").expect("set DATABASE_URL to run this integration test");
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to connect to PostgreSQL")
    }

    #[tokio::test]
    #[ignore = "requires a running PostgreSQL instance; run with `cargo test -- --ignored`"]
    async fn save_then_find_by_username() {
        let store = PostgresUserStore::new(pool_from_env().await);
        let user = User::new(
            Username::parse("integration-auth-user").unwrap(),
            Role::Developer,
        );

        store
            .save_with_password_hash(&user, "hash-integration")
            .await
            .unwrap();

        let found = store
            .find_by_username_with_password_hash(user.username())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found.0, user);
        assert_eq!(found.0.role(), Role::Developer);
        assert_eq!(found.1, "hash-integration");
    }
}
