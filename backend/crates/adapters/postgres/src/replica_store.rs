use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::replica::{ReplicaPolicy, ReplicaRun, ReplicaTarget};
use ferrobox_ports::replica_store::{ReplicaStore, ReplicaStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Adaptador de [`ReplicaStore`] contra `PostgreSQL`.
pub struct PostgresReplicaStore {
    pool: PgPool,
}

impl PostgresReplicaStore {
    /// Construye el adaptador a partir de un `pool` de conexiones.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> ReplicaStoreError {
    ReplicaStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> ReplicaStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return ReplicaStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

#[async_trait]
impl ReplicaStore for PostgresReplicaStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<ReplicaPolicy, ReplicaStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = sqlx::query(
            r"
            SELECT remote_url, destination_id, token, last_run_at,
                   last_packages_imported, last_artifacts_imported,
                   last_skipped, last_error
            FROM repository_replica
            WHERE repository_id = $1
            ",
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err));

        match row {
            Err(ReplicaStoreError::MissingSchema) | Ok(None) => Ok(ReplicaPolicy::unconfigured()),
            Err(err) => Err(err),
            Ok(Some(row)) => policy_from_row(repository_id, &row),
        }
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: &ReplicaPolicy,
    ) -> Result<(), ReplicaStoreError> {
        let repository_id: Uuid = repository_id.into();
        let Some(target) = policy.target() else {
            sqlx::query("DELETE FROM repository_replica WHERE repository_id = $1")
                .bind(repository_id)
                .execute(&self.pool)
                .await
                .map_err(|err| map_sqlx(&err))?;
            return Ok(());
        };

        let last = policy.last_run();
        let last_run_at = last
            .map(ReplicaRun::occurred_at)
            .map(|value| {
                chrono::DateTime::parse_from_rfc3339(value)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|err| backend_error(err.to_string()))
            })
            .transpose()?;

        sqlx::query(
            r"
            INSERT INTO repository_replica (
                repository_id, remote_url, destination_id, token,
                last_run_at, last_packages_imported, last_artifacts_imported,
                last_skipped, last_error
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (repository_id) DO UPDATE
            SET remote_url = EXCLUDED.remote_url,
                destination_id = EXCLUDED.destination_id,
                token = EXCLUDED.token,
                last_run_at = EXCLUDED.last_run_at,
                last_packages_imported = EXCLUDED.last_packages_imported,
                last_artifacts_imported = EXCLUDED.last_artifacts_imported,
                last_skipped = EXCLUDED.last_skipped,
                last_error = EXCLUDED.last_error,
                updated_at = now()
            ",
        )
        .bind(repository_id)
        .bind(target.remote_url().as_str())
        .bind(Uuid::from(target.destination_id()))
        .bind(target.token())
        .bind(last_run_at)
        .bind(last.map(|run| i32::try_from(run.packages_imported()).unwrap_or(i32::MAX)))
        .bind(last.map(|run| i32::try_from(run.artifacts_imported()).unwrap_or(i32::MAX)))
        .bind(last.map(|run| i32::try_from(run.skipped()).unwrap_or(i32::MAX)))
        .bind(last.and_then(ReplicaRun::error))
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }
}

fn policy_from_row(
    source_id: Uuid,
    row: &sqlx::postgres::PgRow,
) -> Result<ReplicaPolicy, ReplicaStoreError> {
    use sqlx::Row;

    let remote_url: String = row
        .try_get("remote_url")
        .map_err(|err| backend_error(err.to_string()))?;
    let destination_id: Uuid = row
        .try_get("destination_id")
        .map_err(|err| backend_error(err.to_string()))?;
    let token: Option<String> = row
        .try_get("token")
        .map_err(|err| backend_error(err.to_string()))?;
    let target = ReplicaTarget::new(
        remote_url,
        RepositoryId::from(destination_id),
        token,
        RepositoryId::from(source_id),
    )
    .map_err(|err| backend_error(err.to_string()))?;

    let last_run_at: Option<chrono::DateTime<chrono::Utc>> = row
        .try_get("last_run_at")
        .map_err(|err| backend_error(err.to_string()))?;
    let last_run = last_run_at.map(|occurred_at| {
        let packages: Option<i32> = row.try_get("last_packages_imported").unwrap_or(None);
        let artifacts: Option<i32> = row.try_get("last_artifacts_imported").unwrap_or(None);
        let skipped: Option<i32> = row.try_get("last_skipped").unwrap_or(None);
        let error: Option<String> = row.try_get("last_error").unwrap_or(None);
        ReplicaRun::new(
            occurred_at.to_rfc3339(),
            u32::try_from(packages.unwrap_or(0)).unwrap_or(0),
            u32::try_from(artifacts.unwrap_or(0)).unwrap_or(0),
            u32::try_from(skipped.unwrap_or(0)).unwrap_or(0),
            error,
        )
    });

    Ok(ReplicaPolicy::new(Some(target), last_run))
}
