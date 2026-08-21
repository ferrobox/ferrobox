use async_trait::async_trait;
use ferrobox_domain::ids::{RepositoryId, WebhookId};
use ferrobox_domain::webhook::{Webhook, WebhookDelivery};
use thiserror::Error;

/// Motivos por los que una operación de persistencia de avisos puede
/// fallar.
#[derive(Debug, Error)]
pub enum WebhookStoreError {
    /// No existe la tabla: falta ejecutar la migración SQL.
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table webhooks is missing)"
    )]
    MissingSchema,

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia de avisos HTTP y de sus envíos.
#[async_trait]
pub trait WebhookStore: Send + Sync {
    /// Inserta o actualiza un aviso.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] o [`WebhookStoreError::MissingSchema`].
    async fn save(&self, webhook: &Webhook) -> Result<(), WebhookStoreError>;

    /// Busca un aviso por identificador.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] o [`WebhookStoreError::MissingSchema`].
    async fn find_by_id(&self, id: WebhookId) -> Result<Option<Webhook>, WebhookStoreError>;

    /// Lista los avisos de un repositorio, más recientes primero.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] o [`WebhookStoreError::MissingSchema`].
    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Webhook>, WebhookStoreError>;

    /// Elimina un aviso y sus envíos. Devuelve `false` si no existía.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] o [`WebhookStoreError::MissingSchema`].
    async fn delete(&self, id: WebhookId) -> Result<bool, WebhookStoreError>;

    /// Registra un envío y recorta el historial del aviso a 20 filas.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] o [`WebhookStoreError::MissingSchema`].
    async fn record_delivery(&self, delivery: &WebhookDelivery) -> Result<(), WebhookStoreError>;

    /// Últimos envíos de un aviso, más recientes primero.
    ///
    /// # Errors
    ///
    /// [`WebhookStoreError::Backend`] o [`WebhookStoreError::MissingSchema`].
    async fn deliveries(
        &self,
        webhook_id: WebhookId,
        limit: usize,
    ) -> Result<Vec<WebhookDelivery>, WebhookStoreError>;
}
