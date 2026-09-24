use async_trait::async_trait;
use ferrobox_domain::admission::AdmissionPolicy;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_ports::admission_store::{AdmissionStore, AdmissionStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Adaptador de [`AdmissionStore`] contra `PostgreSQL`.
pub struct PostgresAdmissionStore {
    pool: PgPool,
}

impl PostgresAdmissionStore {
    /// Construye el adaptador a partir de un `pool` de conexiones.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> AdmissionStoreError {
    AdmissionStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> AdmissionStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return AdmissionStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

#[async_trait]
impl AdmissionStore for PostgresAdmissionStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<AdmissionPolicy, AdmissionStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = sqlx::query(
            r"
            SELECT enabled, moment, predicate, effect
            FROM repository_admission
            WHERE repository_id = $1
            ",
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err));

        match row {
            Err(AdmissionStoreError::MissingSchema) | Ok(None) => Ok(AdmissionPolicy::inactive()),
            Err(err) => Err(err),
            Ok(Some(row)) => {
                let enabled: bool = sqlx::Row::try_get(&row, "enabled")
                    .map_err(|err| backend_error(err.to_string()))?;
                let moment: String = sqlx::Row::try_get(&row, "moment")
                    .map_err(|err| backend_error(err.to_string()))?;
                let predicate: String = sqlx::Row::try_get(&row, "predicate")
                    .map_err(|err| backend_error(err.to_string()))?;
                let effect: String = sqlx::Row::try_get(&row, "effect")
                    .map_err(|err| backend_error(err.to_string()))?;
                AdmissionPolicy::parse(enabled, &moment, &predicate, &effect)
                    .map_err(|err| backend_error(err.to_string()))
            }
        }
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
    ) -> Result<(), AdmissionStoreError> {
        let repository_id: Uuid = repository_id.into();
        sqlx::query(
            r"
            INSERT INTO repository_admission (repository_id, enabled, moment, predicate, effect)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (repository_id) DO UPDATE
            SET enabled = EXCLUDED.enabled,
                moment = EXCLUDED.moment,
                predicate = EXCLUDED.predicate,
                effect = EXCLUDED.effect,
                updated_at = now()
            ",
        )
        .bind(repository_id)
        .bind(policy.enabled())
        .bind(policy.when().as_str())
        .bind(policy.predicate().as_str())
        .bind(policy.effect().as_str())
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }
}
