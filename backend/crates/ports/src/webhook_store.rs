use async_trait::async_trait;
use ferrobox_domain::ids::{RepositoryId, WebhookId};
use ferrobox_domain::webhook::{Webhook, WebhookDelivery};
use thiserror::Error;

/// Reasons a webhook persistence operation can fail.
#[derive(Debug, Error)]
pub enum WebhookStoreError {
    /// The table is missing: the SQL migration has not been run.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table webhooks is missing)"
    )]
    MissingSchema,

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence port for HTTP webhooks and their deliveries.
#[async_trait]
pub trait WebhookStore: Send + Sync {
    /// Inserts or updates a webhook.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] or [`WebhookStoreError::MissingSchema`].
    async fn save(&self, webhook: &Webhook) -> Result<(), WebhookStoreError>;

    /// Looks up a webhook by identifier.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] or [`WebhookStoreError::MissingSchema`].
    async fn find_by_id(&self, id: WebhookId) -> Result<Option<Webhook>, WebhookStoreError>;

    /// Lists the webhooks of a repository, newest first.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] or [`WebhookStoreError::MissingSchema`].
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Webhook>, WebhookStoreError>;

    /// Deletes a webhook and its deliveries. Returns `false` if it did not
    /// exist.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] or [`WebhookStoreError::MissingSchema`].
    async fn delete(&self, id: WebhookId) -> Result<bool, WebhookStoreError>;

    /// Records a delivery and trims the webhook history to 20 rows.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] or [`WebhookStoreError::MissingSchema`].
    async fn record_delivery(&self, delivery: &WebhookDelivery) -> Result<(), WebhookStoreError>;

    /// Latest deliveries of a webhook, newest first.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] or [`WebhookStoreError::MissingSchema`].
    async fn deliveries(
        &self,
        webhook_id: WebhookId,
        limit: usize,
    ) -> Result<Vec<WebhookDelivery>, WebhookStoreError>;
}
