//! [`OsvFeedStore`] adapter against `PostgreSQL`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrobox_ports::osv_feed_store::{OsvFeedRecord, OsvFeedStore, OsvFeedStoreError};
use sqlx::PgPool;
use thiserror::Error;

/// [`OsvFeedStore`] adapter against `PostgreSQL`.
pub struct PostgresOsvFeedStore {
    pool: PgPool,
}

impl PostgresOsvFeedStore {
    /// Builds the adapter from a connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> OsvFeedStoreError {
    OsvFeedStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> OsvFeedStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return OsvFeedStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

fn ecosystems_to_column(ecosystems: &[String]) -> String {
    ecosystems.join(",")
}

fn ecosystems_from_column(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

#[async_trait]
impl OsvFeedStore for PostgresOsvFeedStore {
    async fn current(&self) -> Result<Option<OsvFeedRecord>, OsvFeedStoreError> {
        let row = sqlx::query(
            r"
            SELECT dataset, sha256, advisory_count, ecosystems, storage_key, imported_at, source
            FROM osv_feed
            WHERE singleton = TRUE
            ",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let dataset: String =
            sqlx::Row::try_get(&row, "dataset").map_err(|err| backend_error(err.to_string()))?;
        let sha256: String =
            sqlx::Row::try_get(&row, "sha256").map_err(|err| backend_error(err.to_string()))?;
        let advisory_count: i64 = sqlx::Row::try_get(&row, "advisory_count")
            .map_err(|err| backend_error(err.to_string()))?;
        let ecosystems: String =
            sqlx::Row::try_get(&row, "ecosystems").map_err(|err| backend_error(err.to_string()))?;
        let storage_key: String = sqlx::Row::try_get(&row, "storage_key")
            .map_err(|err| backend_error(err.to_string()))?;
        let imported_at: DateTime<Utc> = sqlx::Row::try_get(&row, "imported_at")
            .map_err(|err| backend_error(err.to_string()))?;
        let source: String =
            sqlx::Row::try_get(&row, "source").map_err(|err| backend_error(err.to_string()))?;
        Ok(Some(OsvFeedRecord {
            dataset,
            sha256,
            advisory_count: u64::try_from(advisory_count).unwrap_or(0),
            ecosystems: ecosystems_from_column(&ecosystems),
            storage_key,
            imported_at,
            source,
        }))
    }

    async fn save(&self, record: &OsvFeedRecord) -> Result<(), OsvFeedStoreError> {
        let advisory_count = i64::try_from(record.advisory_count).unwrap_or(i64::MAX);
        sqlx::query(
            r"
            INSERT INTO osv_feed (
                singleton, dataset, sha256, advisory_count, ecosystems, storage_key,
                imported_at, source
            )
            VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (singleton) DO UPDATE SET
                dataset = EXCLUDED.dataset,
                sha256 = EXCLUDED.sha256,
                advisory_count = EXCLUDED.advisory_count,
                ecosystems = EXCLUDED.ecosystems,
                storage_key = EXCLUDED.storage_key,
                imported_at = EXCLUDED.imported_at,
                source = EXCLUDED.source
            ",
        )
        .bind(&record.dataset)
        .bind(&record.sha256)
        .bind(advisory_count)
        .bind(ecosystems_to_column(&record.ecosystems))
        .bind(&record.storage_key)
        .bind(record.imported_at)
        .bind(&record.source)
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }
}
