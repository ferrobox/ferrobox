use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use ferrobox_domain::audit::{AuditAction, AuditEvent, AuditTargetKind};
use ferrobox_domain::ids::{AuditEventId, UserId};
use ferrobox_ports::audit_store::{AuditStore, AuditStoreError};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

/// [`AuditStore`] adapter against `PostgreSQL`.
pub struct PostgresAuditStore {
    pool: PgPool,
}

impl PostgresAuditStore {
    /// Builds the adapter from a connection `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> AuditStoreError {
    AuditStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> AuditStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return AuditStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

#[async_trait]
impl AuditStore for PostgresAuditStore {
    async fn record(&self, event: &AuditEvent) -> Result<(), AuditStoreError> {
        let id: Uuid = event.id().into();
        let actor_id: Option<Uuid> = event.actor_id().map(Into::into);
        sqlx::query(
            r"
            INSERT INTO audit_events
                (id, actor_id, actor_username, action, target_kind, target, detail)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(id)
        .bind(actor_id)
        .bind(event.actor_username())
        .bind(event.action().as_str())
        .bind(event.target_kind().as_str())
        .bind(event.target())
        .bind(event.detail())
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;

        sqlx::query(
            r"
            DELETE FROM audit_events
            WHERE id NOT IN (
                SELECT id FROM audit_events
                ORDER BY created_at DESC
                LIMIT 200
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }

    async fn list(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditStoreError> {
        let limit = i64::try_from(limit).unwrap_or(200);
        let rows = sqlx::query(
            r"
            SELECT id, actor_id, actor_username, action, target_kind, target, detail, created_at
            FROM audit_events
            ORDER BY created_at DESC
            LIMIT $1
            ",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        rows.iter().map(row_to_event).collect()
    }
}

fn row_to_event(row: &PgRow) -> Result<AuditEvent, AuditStoreError> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|err| backend_error(err.to_string()))?;
    let actor_id: Option<Uuid> = row
        .try_get("actor_id")
        .map_err(|err| backend_error(err.to_string()))?;
    let actor_username: String = row
        .try_get("actor_username")
        .map_err(|err| backend_error(err.to_string()))?;
    let action: String = row
        .try_get("action")
        .map_err(|err| backend_error(err.to_string()))?;
    let target_kind: String = row
        .try_get("target_kind")
        .map_err(|err| backend_error(err.to_string()))?;
    let target: String = row
        .try_get("target")
        .map_err(|err| backend_error(err.to_string()))?;
    let detail: String = row
        .try_get("detail")
        .map_err(|err| backend_error(err.to_string()))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|err| backend_error(err.to_string()))?;
    let action = AuditAction::parse(&action).map_err(|err| backend_error(err.to_string()))?;
    let target_kind =
        AuditTargetKind::parse(&target_kind).map_err(|err| backend_error(err.to_string()))?;
    Ok(AuditEvent::from_parts(
        AuditEventId::from(id),
        actor_id.map(UserId::from),
        actor_username,
        action,
        target_kind,
        target,
        detail,
        created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
    ))
}
