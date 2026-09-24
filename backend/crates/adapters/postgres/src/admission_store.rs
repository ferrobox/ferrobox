use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use ferrobox_domain::admission::{AdmissionEffect, AdmissionEvent, AdmissionPolicy};
use ferrobox_domain::ids::{AdmissionEventId, RepositoryId};
use ferrobox_ports::admission_store::{AdmissionRecord, AdmissionStore, AdmissionStoreError};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
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
    let missing = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01" || code == "42703");
    if missing {
        return AdmissionStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

#[async_trait]
impl AdmissionStore for PostgresAdmissionStore {
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<AdmissionRecord, AdmissionStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = sqlx::query(
            r"
            SELECT enabled, moment, predicate, effect, public_keys_pem
            FROM repository_admission
            WHERE repository_id = $1
            ",
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err));

        match row {
            Err(AdmissionStoreError::MissingSchema) | Ok(None) => Ok(AdmissionRecord {
                policy: AdmissionPolicy::inactive(),
                public_keys_pem: String::new(),
            }),
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
                let public_keys_pem: String = sqlx::Row::try_get(&row, "public_keys_pem")
                    .map_err(|err| backend_error(err.to_string()))?;
                Ok(AdmissionRecord {
                    policy: AdmissionPolicy::parse(enabled, &moment, &predicate, &effect)
                        .map_err(|err| backend_error(err.to_string()))?,
                    public_keys_pem,
                })
            }
        }
    }

    async fn save(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
        public_keys_pem: &str,
    ) -> Result<(), AdmissionStoreError> {
        let repository_id: Uuid = repository_id.into();
        sqlx::query(
            r"
            INSERT INTO repository_admission
                (repository_id, enabled, moment, predicate, effect, public_keys_pem)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (repository_id) DO UPDATE
            SET enabled = EXCLUDED.enabled,
                moment = EXCLUDED.moment,
                predicate = EXCLUDED.predicate,
                effect = EXCLUDED.effect,
                public_keys_pem = EXCLUDED.public_keys_pem,
                updated_at = now()
            ",
        )
        .bind(repository_id)
        .bind(policy.enabled())
        .bind(policy.when().as_str())
        .bind(policy.predicate().as_str())
        .bind(policy.effect().as_str())
        .bind(public_keys_pem)
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }

    async fn record_event(&self, event: &AdmissionEvent) -> Result<(), AdmissionStoreError> {
        let id: Uuid = event.id().into();
        let repository_id: Uuid = event.repository_id().into();
        sqlx::query(
            r"
            INSERT INTO repository_admission_events
                (id, repository_id, name, reference, effect, reason)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(id)
        .bind(repository_id)
        .bind(event.name())
        .bind(event.reference())
        .bind(event.effect().as_str())
        .bind(event.reason())
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;

        sqlx::query(
            r"
            DELETE FROM repository_admission_events
            WHERE repository_id = $1
              AND id NOT IN (
                  SELECT id FROM repository_admission_events
                  WHERE repository_id = $1
                  ORDER BY created_at DESC
                  LIMIT 50
              )
            ",
        )
        .bind(repository_id)
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }

    async fn list_events(
        &self,
        repository_id: RepositoryId,
        limit: usize,
    ) -> Result<Vec<AdmissionEvent>, AdmissionStoreError> {
        let repository_id: Uuid = repository_id.into();
        let limit = i64::try_from(limit).unwrap_or(50);
        let rows = sqlx::query(
            r"
            SELECT id, repository_id, name, reference, effect, reason, created_at
            FROM repository_admission_events
            WHERE repository_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            ",
        )
        .bind(repository_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        rows.iter().map(row_to_event).collect()
    }
}

fn row_to_event(row: &PgRow) -> Result<AdmissionEvent, AdmissionStoreError> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|err| backend_error(err.to_string()))?;
    let repository_id: Uuid = row
        .try_get("repository_id")
        .map_err(|err| backend_error(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| backend_error(err.to_string()))?;
    let reference: String = row
        .try_get("reference")
        .map_err(|err| backend_error(err.to_string()))?;
    let effect: String = row
        .try_get("effect")
        .map_err(|err| backend_error(err.to_string()))?;
    let reason: String = row
        .try_get("reason")
        .map_err(|err| backend_error(err.to_string()))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|err| backend_error(err.to_string()))?;
    let effect = match effect.as_str() {
        "deny" => AdmissionEffect::Deny,
        "warn" => AdmissionEffect::Warn,
        other => return Err(backend_error(format!("unknown admission effect '{other}'"))),
    };
    Ok(AdmissionEvent::from_parts(
        AdmissionEventId::from(id),
        RepositoryId::from(repository_id),
        name,
        reference,
        effect,
        reason,
        created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
    ))
}
