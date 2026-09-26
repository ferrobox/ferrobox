use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::worm::WormPolicy;
use ferrobox_ports::worm_store::{WormStore, WormStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// [`WormStore`] adapter against PostgreSQL.
pub struct PostgresWormStore {
    pool: PgPool,
}

impl PostgresWormStore {
    /// Builds the adapter from a connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> WormStoreError {
    WormStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> WormStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return WormStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

#[async_trait]
impl WormStore for PostgresWormStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<WormPolicy, WormStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = sqlx::query(
            r"
            SELECT enabled
            FROM repository_worm
            WHERE repository_id = $1
            ",
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err));

        match row {
            Err(WormStoreError::MissingSchema) | Ok(None) => Ok(WormPolicy::disabled()),
            Err(err) => Err(err),
            Ok(Some(row)) => {
                let enabled: bool = sqlx::Row::try_get(&row, "enabled")
                    .map_err(|err| backend_error(err.to_string()))?;
                Ok(WormPolicy::new(enabled))
            }
        }
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: WormPolicy,
    ) -> Result<(), WormStoreError> {
        let repository_id: Uuid = repository_id.into();
        sqlx::query(
            r"
            INSERT INTO repository_worm (repository_id, enabled)
            VALUES ($1, $2)
            ON CONFLICT (repository_id) DO UPDATE
            SET enabled = EXCLUDED.enabled,
                updated_at = now()
            ",
        )
        .bind(repository_id)
        .bind(policy.enabled())
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }
}
