use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use ferrobox_domain::ids::{RepositoryId, WebhookDeliveryId, WebhookId};
use ferrobox_domain::webhook::{
    Webhook, WebhookDelivery, WebhookDeliveryStatus, WebhookEvent,
};
use ferrobox_ports::webhook_store::{WebhookStore, WebhookStoreError};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

/// [`WebhookStore`] adapter against `PostgreSQL`.
pub struct PostgresWebhookStore {
    pool: PgPool,
}

impl PostgresWebhookStore {
    /// Builds the adapter from a connection `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> WebhookStoreError {
    WebhookStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn map_sqlx(err: &sqlx::Error) -> WebhookStoreError {
    let undefined_table = err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01");
    if undefined_table {
        return WebhookStoreError::MissingSchema;
    }
    backend_error(err.to_string())
}

fn row_to_webhook(row: &PgRow) -> Result<Webhook, WebhookStoreError> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|err| backend_error(err.to_string()))?;
    let repository_id: Uuid = row
        .try_get("repository_id")
        .map_err(|err| backend_error(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| backend_error(err.to_string()))?;
    let url: String = row
        .try_get("url")
        .map_err(|err| backend_error(err.to_string()))?;
    let secret: Option<String> = row
        .try_get("secret")
        .map_err(|err| backend_error(err.to_string()))?;
    let event_labels: Vec<String> = row
        .try_get("events")
        .map_err(|err| backend_error(err.to_string()))?;
    let enabled: bool = row
        .try_get("enabled")
        .map_err(|err| backend_error(err.to_string()))?;
    let events = event_labels
        .iter()
        .map(|label| {
            WebhookEvent::parse(label)
                .ok_or_else(|| backend_error(format!("unknown webhook event '{label}'")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Webhook::from_parts(
        WebhookId::from(id),
        RepositoryId::from(repository_id),
        name,
        url,
        secret,
        events,
        enabled,
    )
    .map_err(|err| backend_error(err.to_string()))
}

fn row_to_delivery(row: &PgRow) -> Result<WebhookDelivery, WebhookStoreError> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|err| backend_error(err.to_string()))?;
    let webhook_id: Uuid = row
        .try_get("webhook_id")
        .map_err(|err| backend_error(err.to_string()))?;
    let event: String = row
        .try_get("event")
        .map_err(|err| backend_error(err.to_string()))?;
    let status: String = row
        .try_get("status")
        .map_err(|err| backend_error(err.to_string()))?;
    let http_status: Option<i32> = row
        .try_get("http_status")
        .map_err(|err| backend_error(err.to_string()))?;
    let error: Option<String> = row
        .try_get("error")
        .map_err(|err| backend_error(err.to_string()))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|err| backend_error(err.to_string()))?;
    let status = WebhookDeliveryStatus::parse(&status)
        .ok_or_else(|| backend_error(format!("unknown delivery status '{status}'")))?;
    let http_status = http_status
        .map(|value| u16::try_from(value).map_err(|err| backend_error(err.to_string())))
        .transpose()?;
    Ok(WebhookDelivery::from_parts(
        WebhookDeliveryId::from(id),
        WebhookId::from(webhook_id),
        event,
        status,
        http_status,
        error,
        created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
    ))
}

#[async_trait]
impl WebhookStore for PostgresWebhookStore {
    async fn save(&self, webhook: &Webhook) -> Result<(), WebhookStoreError> {
        let id: Uuid = webhook.id().into();
        let repository_id: Uuid = webhook.repository_id().into();
        let events: Vec<String> = webhook
            .events()
            .iter()
            .map(|event| event.as_str().to_string())
            .collect();
        sqlx::query(
            r"
            INSERT INTO webhooks (id, repository_id, name, url, secret, events, enabled)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                url = EXCLUDED.url,
                secret = EXCLUDED.secret,
                events = EXCLUDED.events,
                enabled = EXCLUDED.enabled
            ",
        )
        .bind(id)
        .bind(repository_id)
        .bind(webhook.name())
        .bind(webhook.url())
        .bind(webhook.secret())
        .bind(&events)
        .bind(webhook.enabled())
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }

    async fn find_by_id(&self, id: WebhookId) -> Result<Option<Webhook>, WebhookStoreError> {
        let id: Uuid = id.into();
        let row = sqlx::query(
            r"
            SELECT id, repository_id, name, url, secret, events, enabled
            FROM webhooks
            WHERE id = $1
            ",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        row.as_ref().map(row_to_webhook).transpose()
    }

    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Webhook>, WebhookStoreError> {
        let repository_id: Uuid = repository_id.into();
        let rows = sqlx::query(
            r"
            SELECT id, repository_id, name, url, secret, events, enabled
            FROM webhooks
            WHERE repository_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(repository_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        rows.iter().map(row_to_webhook).collect()
    }

    async fn delete(&self, id: WebhookId) -> Result<bool, WebhookStoreError> {
        let id: Uuid = id.into();
        let result = sqlx::query("DELETE FROM webhooks WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|err| map_sqlx(&err))?;
        Ok(result.rows_affected() > 0)
    }

    async fn record_delivery(&self, delivery: &WebhookDelivery) -> Result<(), WebhookStoreError> {
        let id: Uuid = delivery.id().into();
        let webhook_id: Uuid = delivery.webhook_id().into();
        let http_status = delivery.http_status().map(i32::from);
        sqlx::query(
            r"
            INSERT INTO webhook_deliveries
                (id, webhook_id, event, status, http_status, error)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(id)
        .bind(webhook_id)
        .bind(delivery.event())
        .bind(delivery.status().as_str())
        .bind(http_status)
        .bind(delivery.error())
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;

        sqlx::query(
            r"
            DELETE FROM webhook_deliveries
            WHERE webhook_id = $1
              AND id NOT IN (
                  SELECT id FROM webhook_deliveries
                  WHERE webhook_id = $1
                  ORDER BY created_at DESC
                  LIMIT 20
              )
            ",
        )
        .bind(webhook_id)
        .execute(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        Ok(())
    }

    async fn deliveries(
        &self,
        webhook_id: WebhookId,
        limit: usize,
    ) -> Result<Vec<WebhookDelivery>, WebhookStoreError> {
        let webhook_id: Uuid = webhook_id.into();
        let limit = i64::try_from(limit).unwrap_or(20);
        let rows = sqlx::query(
            r"
            SELECT id, webhook_id, event, status, http_status, error, created_at
            FROM webhook_deliveries
            WHERE webhook_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            ",
        )
        .bind(webhook_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| map_sqlx(&err))?;
        rows.iter().map(row_to_delivery).collect()
    }
}
