use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::quota::StorageQuota;
use ferrobox_ports::quota_store::{QuotaStore, QuotaStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Adaptador de [`QuotaStore`] contra `PostgreSQL`.
pub struct PostgresQuotaStore {
    pool: PgPool,
}

impl PostgresQuotaStore {
    /// Construye el adaptador a partir de un `pool` de conexiones.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> QuotaStoreError {
    QuotaStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> QuotaStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return QuotaStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

fn quota_from_row(limit_bytes: Option<i64>) -> Result<StorageQuota, QuotaStoreError> {
    let limit_bytes = limit_bytes
        .map(|value| u64::try_from(value).map_err(|err| backend_error(err.to_string())))
        .transpose()?;
    StorageQuota::new(limit_bytes).map_err(|err| backend_error(err.to_string()))
}

#[async_trait]
impl QuotaStore for PostgresQuotaStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<StorageQuota, QuotaStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = sqlx::query(
            r"
            SELECT limit_bytes
            FROM repository_quota
            WHERE repository_id = $1
            ",
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err));

        match row {
            Err(QuotaStoreError::MissingSchema) | Ok(None) => Ok(StorageQuota::unlimited()),
            Err(err) => Err(err),
            Ok(Some(row)) => {
                let limit_bytes: Option<i64> = sqlx::Row::try_get(&row, "limit_bytes")
                    .map_err(|err| backend_error(err.to_string()))?;
                quota_from_row(limit_bytes)
            }
        }
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        quota: StorageQuota,
    ) -> Result<(), QuotaStoreError> {
        let repository_id: Uuid = repository_id.into();
        let limit_bytes = quota
            .limit_bytes()
            .map(|value| i64::try_from(value).map_err(|err| backend_error(err.to_string())))
            .transpose()?;

        sqlx::query(
            r"
            INSERT INTO repository_quota (repository_id, limit_bytes)
            VALUES ($1, $2)
            ON CONFLICT (repository_id) DO UPDATE
            SET limit_bytes = EXCLUDED.limit_bytes,
                updated_at = now()
            ",
        )
        .bind(repository_id)
        .bind(limit_bytes)
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }
}
