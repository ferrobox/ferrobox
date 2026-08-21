//! Avisos HTTP de un repositorio: CRUD y envío en segundo plano.

use std::sync::Arc;

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use ferrobox_domain::assay::Assay;
use ferrobox_domain::ids::{RepositoryId, WebhookDeliveryId, WebhookId};
use ferrobox_domain::package_coordinate::PackageCoordinate;
use ferrobox_domain::repository::Repository;
use ferrobox_domain::webhook::{
    Webhook, WebhookDelivery, WebhookDeliveryStatus, WebhookError, WebhookEvent,
};
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::webhook_store::{WebhookStore, WebhookStoreError};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Motivos por los que gestionar o enviar un aviso puede fallar.
#[derive(Debug, Error)]
pub enum ManageWebhookError {
    /// El repositorio no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// El aviso no existe.
    #[error("webhook {0} was not found")]
    WebhookNotFound(WebhookId),

    /// El aviso no pertenece a ese repositorio.
    #[error("webhook does not belong to this repository")]
    WebhookMismatch,

    /// Un `Alloy` no dispara avisos propios: configúralos en un Forge o Mirror miembro.
    #[error("webhooks do not apply to Alloy repositories")]
    AlloyRepository,

    /// Los campos del aviso no son válidos.
    #[error(transparent)]
    Invalid(#[from] WebhookError),

    /// Fallo al persistir avisos.
    #[error(transparent)]
    Store(#[from] WebhookStoreError),

    /// Fallo al consultar repositorios.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),
}

/// Caso de uso: avisos HTTP por repositorio.
#[derive(Clone)]
pub struct WebhookService {
    store: Arc<dyn WebhookStore>,
    http_client: Arc<dyn HttpClient>,
    repository_store: Arc<dyn RepositoryStore>,
}

impl WebhookService {
    /// Construye el servicio a partir de sus puertos.
    #[must_use]
    pub fn new(
        store: Arc<dyn WebhookStore>,
        http_client: Arc<dyn HttpClient>,
        repository_store: Arc<dyn RepositoryStore>,
    ) -> Self {
        Self {
            store,
            http_client,
            repository_store,
        }
    }

    /// Lista los avisos de un repositorio.
    ///
    /// # Errors
    ///
    /// [`ManageWebhookError::RepositoryNotFound`] o fallo de persistencia.
    pub async fn list(&self, repository_id: RepositoryId) -> Result<Vec<Webhook>, ManageWebhookError> {
        self.require_writable_repository(repository_id).await?;
        Ok(self.store.find_by_repository(repository_id).await?)
    }

    /// Crea un aviso.
    ///
    /// # Errors
    ///
    /// [`ManageWebhookError::Invalid`] o el repositorio no existe.
    pub async fn create(
        &self,
        repository_id: RepositoryId,
        name: String,
        url: String,
        secret: Option<String>,
        events: Vec<WebhookEvent>,
        enabled: bool,
    ) -> Result<Webhook, ManageWebhookError> {
        self.require_writable_repository(repository_id).await?;
        let webhook = Webhook::new(repository_id, name, url, secret, events, enabled)?;
        self.store.save(&webhook).await?;
        Ok(webhook)
    }

    /// Actualiza un aviso. Si `secret` es `None`, conserva el anterior.
    ///
    /// # Errors
    ///
    /// [`ManageWebhookError::WebhookNotFound`] o validación.
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        repository_id: RepositoryId,
        webhook_id: WebhookId,
        name: String,
        url: String,
        secret: Option<String>,
        events: Vec<WebhookEvent>,
        enabled: bool,
    ) -> Result<Webhook, ManageWebhookError> {
        self.require_writable_repository(repository_id).await?;
        let existing = self.require_webhook(repository_id, webhook_id).await?;
        let secret = secret.or_else(|| existing.secret().map(str::to_string));
        let webhook = Webhook::from_parts(
            existing.id(),
            repository_id,
            name,
            url,
            secret,
            events,
            enabled,
        )?;
        self.store.save(&webhook).await?;
        Ok(webhook)
    }

    /// Elimina un aviso.
    ///
    /// # Errors
    ///
    /// [`ManageWebhookError::WebhookNotFound`].
    pub async fn delete(
        &self,
        repository_id: RepositoryId,
        webhook_id: WebhookId,
    ) -> Result<(), ManageWebhookError> {
        self.require_writable_repository(repository_id).await?;
        self.require_webhook(repository_id, webhook_id).await?;
        self.store.delete(webhook_id).await?;
        Ok(())
    }

    /// Últimos envíos de un aviso.
    ///
    /// # Errors
    ///
    /// [`ManageWebhookError::WebhookNotFound`].
    pub async fn deliveries(
        &self,
        repository_id: RepositoryId,
        webhook_id: WebhookId,
    ) -> Result<Vec<WebhookDelivery>, ManageWebhookError> {
        self.require_writable_repository(repository_id).await?;
        self.require_webhook(repository_id, webhook_id).await?;
        Ok(self.store.deliveries(webhook_id, 20).await?)
    }

    /// Envía un `ping` síncrono para comprobar el destino.
    ///
    /// # Errors
    ///
    /// [`ManageWebhookError::WebhookNotFound`].
    pub async fn ping(
        &self,
        repository_id: RepositoryId,
        webhook_id: WebhookId,
    ) -> Result<WebhookDelivery, ManageWebhookError> {
        let webhook = {
            self.require_writable_repository(repository_id).await?;
            self.require_webhook(repository_id, webhook_id).await?
        };
        let repository = self.require_repository(repository_id).await?;
        let payload = json!({
            "event": "ping",
            "occurred_at": now_rfc3339(),
            "repository": {
                "id": repository.id().to_string(),
                "name": repository.name().as_str(),
            },
        });
        Ok(self.deliver(&webhook, "ping", payload).await)
    }

    /// Dispara `package.published` en segundo plano. Nunca bloquea al
    /// llamador: un fallo queda en el historial de envíos.
    pub fn notify_package_published(
        &self,
        repository_id: RepositoryId,
        coordinate: PackageCoordinate,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            let _ = service
                .deliver_package_published(repository_id, &coordinate)
                .await;
        });
    }

    /// Dispara `assay.completed` en segundo plano.
    pub fn notify_assay_completed(&self, assay: Assay) {
        let service = self.clone();
        tokio::spawn(async move {
            let _ = service.deliver_assay_completed(&assay).await;
        });
    }

    async fn deliver_package_published(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<(), ManageWebhookError> {
        let Some(repository) = self.repository_store.find_by_id(repository_id).await? else {
            return Ok(());
        };
        let payload = json!({
            "event": WebhookEvent::PackagePublished.as_str(),
            "occurred_at": now_rfc3339(),
            "repository": {
                "id": repository.id().to_string(),
                "name": repository.name().as_str(),
            },
            "package": package_json(coordinate),
        });
        self.deliver_event(
            repository_id,
            WebhookEvent::PackagePublished,
            payload,
        )
        .await;
        Ok(())
    }

    async fn deliver_assay_completed(&self, assay: &Assay) -> Result<(), ManageWebhookError> {
        let Some(repository) = self
            .repository_store
            .find_by_id(assay.repository_id())
            .await?
        else {
            return Ok(());
        };
        let counts = assay.counts();
        let payload = json!({
            "event": WebhookEvent::AssayCompleted.as_str(),
            "occurred_at": now_rfc3339(),
            "repository": {
                "id": repository.id().to_string(),
                "name": repository.name().as_str(),
            },
            "package": package_json(assay.coordinate()),
            "assay": {
                "id": assay.id().to_string(),
                "status": assay.status().as_str(),
                "counts": {
                    "critical": counts.critical,
                    "high": counts.high,
                    "medium": counts.medium,
                    "low": counts.low,
                    "unknown": counts.unknown,
                },
            },
        });
        self.deliver_event(
            assay.repository_id(),
            WebhookEvent::AssayCompleted,
            payload,
        )
        .await;
        Ok(())
    }

    async fn deliver_event(&self, repository_id: RepositoryId, event: WebhookEvent, payload: Value) {
        let Ok(webhooks) = self.store.find_by_repository(repository_id).await else {
            return;
        };
        for webhook in webhooks {
            if webhook.listens_to(event) {
                let _ = self.deliver(&webhook, event.as_str(), payload.clone()).await;
            }
        }
    }

    async fn deliver(&self, webhook: &Webhook, event: &str, payload: Value) -> WebhookDelivery {
        let body = Bytes::from(payload.to_string());
        let mut headers = vec![
            ("content-type", "application/json"),
            ("user-agent", "ferrobox/0.1"),
            ("x-ferrobox-event", event),
        ];
        let signature = webhook.secret().map(|secret| sign(secret, &body));
        if let Some(value) = signature.as_ref() {
            headers.push(("x-ferrobox-signature", value.as_str()));
        }

        let (status, http_status, error) = match self
            .http_client
            .post_with_headers(webhook.url(), body, &headers)
            .await
        {
            Ok(response) if response.is_success() => {
                (WebhookDeliveryStatus::Success, Some(response.status), None)
            }
            Ok(response) => (
                WebhookDeliveryStatus::Failed,
                Some(response.status),
                Some(format!("destination responded HTTP {}", response.status)),
            ),
            Err(HttpClientError::Transport { message, .. }) => {
                (WebhookDeliveryStatus::Failed, None, Some(message))
            }
            Err(HttpClientError::Status { status, .. }) => (
                WebhookDeliveryStatus::Failed,
                Some(status),
                Some(format!("destination responded HTTP {status}")),
            ),
        };

        let delivery = WebhookDelivery::from_parts(
            WebhookDeliveryId::new(),
            webhook.id(),
            event,
            status,
            http_status,
            error,
            now_rfc3339(),
        );
        let _ = self.store.record_delivery(&delivery).await;
        delivery
    }

    async fn require_writable_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Repository, ManageWebhookError> {
        let repository = self.require_repository(repository_id).await?;
        if matches!(
            repository.kind(),
            ferrobox_domain::repository::RepositoryKind::Alloy { .. }
        ) {
            return Err(ManageWebhookError::AlloyRepository);
        }
        Ok(repository)
    }

    async fn require_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Repository, ManageWebhookError> {
        self.repository_store
            .find_by_id(repository_id)
            .await?
            .ok_or(ManageWebhookError::RepositoryNotFound(repository_id))
    }

    async fn require_webhook(
        &self,
        repository_id: RepositoryId,
        webhook_id: WebhookId,
    ) -> Result<Webhook, ManageWebhookError> {
        let webhook = self
            .store
            .find_by_id(webhook_id)
            .await?
            .ok_or(ManageWebhookError::WebhookNotFound(webhook_id))?;
        if webhook.repository_id() != repository_id {
            return Err(ManageWebhookError::WebhookMismatch);
        }
        Ok(webhook)
    }
}

fn package_json(coordinate: &PackageCoordinate) -> Value {
    json!({
        "ecosystem": coordinate.ecosystem().label(),
        "name": coordinate.name().as_str(),
        "version": coordinate.version().as_str(),
    })
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC-SHA256 accepts a secret of any length");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrobox_domain::package_coordinate::{PackageEcosystem, PackageName, PackageVersion};
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{
        InMemoryHttpClient, InMemoryRepositoryStore, InMemoryWebhookStore, forge,
    };

    fn coordinate() -> PackageCoordinate {
        PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("lodash").unwrap(),
            PackageVersion::parse("4.17.21").unwrap(),
        )
    }

    #[tokio::test]
    async fn delivers_published_event_with_hmac_and_skips_other_events() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let store = Arc::new(InMemoryWebhookStore::default());
        let repo = forge("npm-all");
        repos.save(&repo).await.unwrap();
        http.stub("https://example.test/hook", 204, "");

        let service = WebhookService::new(store.clone(), http.clone(), repos);
        let webhook = service
            .create(
                repo.id(),
                "ci".into(),
                "https://example.test/hook".into(),
                Some("topsecret".into()),
                vec![WebhookEvent::PackagePublished],
                true,
            )
            .await
            .unwrap();

        service
            .deliver_package_published(repo.id(), &coordinate())
            .await
            .unwrap();

        let posts = http.take_posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].url, "https://example.test/hook");
        assert!(
            posts[0]
                .headers
                .iter()
                .any(|(name, value)| name == "x-ferrobox-event" && value == "package.published")
        );
        let signature = posts[0]
            .headers
            .iter()
            .find(|(name, _)| name == "x-ferrobox-signature")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(signature, sign("topsecret", &posts[0].body));
        let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
        assert_eq!(body["package"]["name"], "lodash");

        let deliveries = service.deliveries(repo.id(), webhook.id()).await.unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].status(), WebhookDeliveryStatus::Success);
        assert_eq!(deliveries[0].http_status(), Some(204));
    }

    #[tokio::test]
    async fn failed_destination_is_recorded_and_does_not_panic() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let store = Arc::new(InMemoryWebhookStore::default());
        let repo = forge("npm-all");
        repos.save(&repo).await.unwrap();
        http.stub("https://example.test/hook", 500, "nope");

        let service = WebhookService::new(store, http, repos);
        let webhook = service
            .create(
                repo.id(),
                "ci".into(),
                "https://example.test/hook".into(),
                None,
                vec![WebhookEvent::AssayCompleted],
                true,
            )
            .await
            .unwrap();

        let ping = service.ping(repo.id(), webhook.id()).await.unwrap();
        assert_eq!(ping.status(), WebhookDeliveryStatus::Failed);
        assert_eq!(ping.http_status(), Some(500));
    }
}
