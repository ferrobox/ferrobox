//! [`OsvSyncStore`] adapter against `PostgreSQL`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrobox_ports::osv_sync_store::{OsvSyncSettings, OsvSyncStore, OsvSyncStoreError};
use sqlx::PgPool;
use thiserror::Error;

/// [`OsvSyncStore`] adapter against `PostgreSQL`.
pub struct PostgresOsvSyncStore {
    pool: PgPool,
}

impl PostgresOsvSyncStore {
    /// Builds the adapter from a connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> OsvSyncStoreError {
    OsvSyncStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> OsvSyncStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return OsvSyncStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

fn row_to_settings(row: &sqlx::postgres::PgRow) -> Result<OsvSyncSettings, OsvSyncStoreError> {
    let reference: String =
        sqlx::Row::try_get(row, "reference").map_err(|err| backend_error(err.to_string()))?;
    let public_key_pem: String =
        sqlx::Row::try_get(row, "public_key_pem").map_err(|err| backend_error(err.to_string()))?;
    let last_outcome: Option<String> =
        sqlx::Row::try_get(row, "last_outcome").map_err(|err| backend_error(err.to_string()))?;
    let last_detail: Option<String> =
        sqlx::Row::try_get(row, "last_detail").map_err(|err| backend_error(err.to_string()))?;
    let last_attempt_at: Option<DateTime<Utc>> =
        sqlx::Row::try_get(row, "last_attempt_at").map_err(|err| backend_error(err.to_string()))?;
    let retry_after: Option<DateTime<Utc>> =
        sqlx::Row::try_get(row, "retry_after").map_err(|err| backend_error(err.to_string()))?;
    Ok(OsvSyncSettings {
        reference,
        public_key_pem,
        last_outcome,
        last_detail,
        last_attempt_at,
        retry_after,
    })
}

const SELECT_ROW: &str = "
    SELECT reference, public_key_pem, last_outcome, last_detail, last_attempt_at, retry_after
    FROM osv_sync
    WHERE singleton = TRUE
";

#[async_trait]
impl OsvSyncStore for PostgresOsvSyncStore {
    async fn current(&self) -> Result<Option<OsvSyncSettings>, OsvSyncStoreError> {
        let row = sqlx::query(SELECT_ROW)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| map_sqlx(&err))?;
        row.map(|row| row_to_settings(&row)).transpose()
    }

    async fn save(
        &self,
        reference: &str,
        public_key_pem: &str,
    ) -> Result<OsvSyncSettings, OsvSyncStoreError> {
        let row = sqlx::query(
            "
            INSERT INTO osv_sync (singleton, reference, public_key_pem)
            VALUES (TRUE, $1, $2)
            ON CONFLICT (singleton) DO UPDATE SET
                reference = EXCLUDED.reference,
                public_key_pem = EXCLUDED.public_key_pem,
                last_outcome = NULL,
                last_detail = NULL,
                last_attempt_at = NULL,
                retry_after = NULL
            RETURNING reference, public_key_pem, last_outcome, last_detail, last_attempt_at, retry_after
            ",
        )
        .bind(reference)
        .bind(public_key_pem)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        row_to_settings(&row)
    }

    async fn record_attempt(
        &self,
        outcome: &str,
        detail: &str,
        attempted_at: DateTime<Utc>,
        retry_after: DateTime<Utc>,
    ) -> Result<OsvSyncSettings, OsvSyncStoreError> {
        let row = sqlx::query(
            "
            UPDATE osv_sync SET
                last_outcome = $1,
                last_detail = $2,
                last_attempt_at = $3,
                retry_after = $4
            WHERE singleton = TRUE
            RETURNING reference, public_key_pem, last_outcome, last_detail, last_attempt_at, retry_after
            ",
        )
        .bind(outcome)
        .bind(detail)
        .bind(attempted_at)
        .bind(retry_after)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        let Some(row) = row else {
            return Err(OsvSyncStoreError::Backend(Box::new(RowConversionError(
                "osv sync settings are not configured".to_string(),
            ))));
        };
        row_to_settings(&row)
    }
}
