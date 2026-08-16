use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::retention::RetentionPolicy;
use ferrobox_ports::retention_store::{RetentionStore, RetentionStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Adaptador de [`RetentionStore`] contra `PostgreSQL`.
pub struct PostgresRetentionStore {
    pool: PgPool,
}

impl PostgresRetentionStore {
    /// Construye el adaptador a partir de un `pool` de conexiones.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> RetentionStoreError {
    RetentionStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn policy_from_row(
    keep_last: Option<i32>,
    keep_days: Option<i32>,
) -> Result<RetentionPolicy, RetentionStoreError> {
    let keep_last = keep_last
        .map(|value| u32::try_from(value).map_err(|err| backend_error(err.to_string())))
        .transpose()?;
    let keep_days = keep_days
        .map(|value| u32::try_from(value).map_err(|err| backend_error(err.to_string())))
        .transpose()?;
    RetentionPolicy::new(keep_last, keep_days).map_err(|err| backend_error(err.to_string()))
}

#[async_trait]
impl RetentionStore for PostgresRetentionStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<RetentionPolicy, RetentionStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = sqlx::query(
            r"
            SELECT keep_last, keep_days
            FROM repository_retention
            WHERE repository_id = $1
            ",
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        match row {
            None => Ok(RetentionPolicy::keep_all()),
            Some(row) => {
                let keep_last: Option<i32> = sqlx::Row::try_get(&row, "keep_last")
                    .map_err(|err| backend_error(err.to_string()))?;
                let keep_days: Option<i32> = sqlx::Row::try_get(&row, "keep_days")
                    .map_err(|err| backend_error(err.to_string()))?;
                policy_from_row(keep_last, keep_days)
            }
        }
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<(), RetentionStoreError> {
        let repository_id: Uuid = repository_id.into();
        let keep_last = policy
            .keep_last()
            .map(|value| i32::try_from(value).map_err(|err| backend_error(err.to_string())))
            .transpose()?;
        let keep_days = policy
            .keep_days()
            .map(|value| i32::try_from(value).map_err(|err| backend_error(err.to_string())))
            .transpose()?;

        sqlx::query(
            r"
            INSERT INTO repository_retention (repository_id, keep_last, keep_days)
            VALUES ($1, $2, $3)
            ON CONFLICT (repository_id) DO UPDATE
            SET keep_last = EXCLUDED.keep_last,
                keep_days = EXCLUDED.keep_days,
                updated_at = now()
            ",
        )
        .bind(repository_id)
        .bind(keep_last)
        .bind(keep_days)
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }
}
