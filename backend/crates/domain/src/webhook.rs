//! Aviso HTTP (`webhook`): un POST a una URL cuando ocurre un evento
//! en un repositorio. El install y el publish no esperan la respuesta.

use std::fmt;

use thiserror::Error;
use url::Url;

use crate::ids::{RepositoryId, WebhookDeliveryId, WebhookId};

const MAX_NAME_LENGTH: usize = 100;
const MAX_SECRET_LENGTH: usize = 256;
const MAX_URL_LENGTH: usize = 2048;

/// Evento que puede disparar un aviso HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebhookEvent {
    /// Un ensaye ha terminado (listo, fallido o no soportado).
    AssayCompleted,
    /// Una versión ha quedado disponible en el repositorio (publish o
    /// cacheo de un Mirror).
    PackagePublished,
}

impl WebhookEvent {
    /// Etiqueta estable usada en persistencia, API y cabecera HTTP.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AssayCompleted => "assay.completed",
            Self::PackagePublished => "package.published",
        }
    }

    /// Parsea la etiqueta estable. `ping` no es un evento persistido:
    /// solo se usa al probar el aviso.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "assay.completed" => Some(Self::AssayCompleted),
            "package.published" => Some(Self::PackagePublished),
            _ => None,
        }
    }
}

impl fmt::Display for WebhookEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Resultado de un envío.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookDeliveryStatus {
    /// El destino respondió 2xx.
    Success,
    /// Red, TLS o respuesta fuera de 2xx.
    Failed,
}

impl WebhookDeliveryStatus {
    /// Etiqueta estable.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
        }
    }

    /// Parsea la etiqueta estable.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Self::Success),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Motivos por los que un aviso no es válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebhookError {
    /// El nombre no puede estar vacío.
    #[error("webhook name cannot be empty")]
    EmptyName,

    /// El nombre supera la longitud máxima.
    #[error("webhook name cannot exceed {max} characters, got {actual}")]
    NameTooLong {
        /// Longitud máxima permitida.
        max: usize,
        /// Longitud real.
        actual: usize,
    },

    /// La URL no es `http` ni `https`, o no tiene host.
    #[error("webhook URL must be an absolute http or https URL")]
    InvalidUrl,

    /// La URL supera la longitud máxima.
    #[error("webhook URL cannot exceed {max} characters")]
    UrlTooLong {
        /// Longitud máxima permitida.
        max: usize,
    },

    /// El secreto supera la longitud máxima.
    #[error("webhook secret cannot exceed {max} characters")]
    SecretTooLong {
        /// Longitud máxima permitida.
        max: usize,
    },

    /// Hay que elegir al menos un evento.
    #[error("select at least one webhook event")]
    NoEvents,
}

/// Un aviso HTTP asociado a un repositorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Webhook {
    id: WebhookId,
    repository_id: RepositoryId,
    name: String,
    url: String,
    secret: Option<String>,
    events: Vec<WebhookEvent>,
    enabled: bool,
}

impl Webhook {
    /// Crea un aviso nuevo.
    ///
    /// # Errors
    ///
    /// [`WebhookError`] si el nombre, la URL, el secreto o los eventos
    /// no son válidos.
    pub fn new(
        repository_id: RepositoryId,
        name: impl Into<String>,
        url: impl Into<String>,
        secret: Option<String>,
        events: Vec<WebhookEvent>,
        enabled: bool,
    ) -> Result<Self, WebhookError> {
        Self::from_parts(
            WebhookId::new(),
            repository_id,
            name,
            url,
            secret,
            events,
            enabled,
        )
    }

    /// Reconstituye un aviso ya persistido.
    ///
    /// # Errors
    ///
    /// [`WebhookError`] si algún campo no cumple las invariantes.
    pub fn from_parts(
        id: WebhookId,
        repository_id: RepositoryId,
        name: impl Into<String>,
        url: impl Into<String>,
        secret: Option<String>,
        events: Vec<WebhookEvent>,
        enabled: bool,
    ) -> Result<Self, WebhookError> {
        let name = validate_name(&name.into())?;
        let url = validate_url(&url.into())?;
        let secret = validate_secret(secret)?;
        if events.is_empty() {
            return Err(WebhookError::NoEvents);
        }
        let mut events = events;
        events.sort_by_key(|event| event.as_str());
        events.dedup();
        Ok(Self {
            id,
            repository_id,
            name,
            url,
            secret,
            events,
            enabled,
        })
    }

    /// Identificador.
    #[must_use]
    pub fn id(&self) -> WebhookId {
        self.id
    }

    /// Repositorio al que pertenece.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Nombre descriptivo.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// URL de destino.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Secreto HMAC, si hay.
    #[must_use]
    pub fn secret(&self) -> Option<&str> {
        self.secret.as_deref()
    }

    /// Eventos suscritos.
    #[must_use]
    pub fn events(&self) -> &[WebhookEvent] {
        &self.events
    }

    /// `true` si está activo.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// `true` si este aviso debe dispararse para `event`.
    #[must_use]
    pub fn listens_to(&self, event: WebhookEvent) -> bool {
        self.enabled && self.events.contains(&event)
    }
}

/// Un intento de envío a un aviso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookDelivery {
    id: WebhookDeliveryId,
    webhook_id: WebhookId,
    event: String,
    status: WebhookDeliveryStatus,
    http_status: Option<u16>,
    error: Option<String>,
    created_at: String,
}

impl WebhookDelivery {
    /// Construye un envío ya persistido o recién realizado.
    #[must_use]
    pub fn from_parts(
        id: WebhookDeliveryId,
        webhook_id: WebhookId,
        event: impl Into<String>,
        status: WebhookDeliveryStatus,
        http_status: Option<u16>,
        error: Option<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id,
            webhook_id,
            event: event.into(),
            status,
            http_status,
            error,
            created_at: created_at.into(),
        }
    }

    /// Identificador del envío.
    #[must_use]
    pub fn id(&self) -> WebhookDeliveryId {
        self.id
    }

    /// Aviso al que pertenece.
    #[must_use]
    pub fn webhook_id(&self) -> WebhookId {
        self.webhook_id
    }

    /// Nombre del evento (`assay.completed`, `package.published`, `ping`).
    #[must_use]
    pub fn event(&self) -> &str {
        &self.event
    }

    /// Resultado.
    #[must_use]
    pub fn status(&self) -> WebhookDeliveryStatus {
        self.status
    }

    /// Código HTTP del destino, si llegó a responder.
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    /// Detalle del fallo, si lo hubo.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Instante RFC 3339.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.created_at
    }
}

fn validate_name(name: &str) -> Result<String, WebhookError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(WebhookError::EmptyName);
    }
    if name.len() > MAX_NAME_LENGTH {
        return Err(WebhookError::NameTooLong {
            max: MAX_NAME_LENGTH,
            actual: name.len(),
        });
    }
    Ok(name)
}

fn validate_url(url: &str) -> Result<String, WebhookError> {
    let url = url.trim().to_string();
    if url.len() > MAX_URL_LENGTH {
        return Err(WebhookError::UrlTooLong {
            max: MAX_URL_LENGTH,
        });
    }
    let parsed = Url::parse(&url).map_err(|_| WebhookError::InvalidUrl)?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(WebhookError::InvalidUrl);
    }
    if parsed.host_str().is_none() {
        return Err(WebhookError::InvalidUrl);
    }
    Ok(url)
}

fn validate_secret(secret: Option<String>) -> Result<Option<String>, WebhookError> {
    let Some(secret) = secret else {
        return Ok(None);
    };
    let secret = secret.trim().to_string();
    if secret.is_empty() {
        return Ok(None);
    }
    if secret.len() > MAX_SECRET_LENGTH {
        return Err(WebhookError::SecretTooLong {
            max: MAX_SECRET_LENGTH,
        });
    }
    Ok(Some(secret))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RepositoryId;

    #[test]
    fn rejects_non_http_url_and_empty_events() {
        let repo = RepositoryId::new();
        assert_eq!(
            Webhook::new(
                repo,
                "ci",
                "ftp://example.test",
                None,
                vec![WebhookEvent::AssayCompleted],
                true,
            )
                .unwrap_err(),
            WebhookError::InvalidUrl
        );
        assert_eq!(
            Webhook::new(repo, "ci", "https://example.test/hook", None, vec![], true).unwrap_err(),
            WebhookError::NoEvents
        );
    }

    #[test]
    fn accepts_https_and_deduplicates_events() {
        let webhook = Webhook::new(
            RepositoryId::new(),
            " Slack CI ",
            "https://example.test/ferrobox",
            Some("  s3cret  ".into()),
            vec![WebhookEvent::AssayCompleted, WebhookEvent::AssayCompleted],
            true,
        )
        .unwrap();
        assert_eq!(webhook.name(), "Slack CI");
        assert_eq!(webhook.secret(), Some("s3cret"));
        assert_eq!(webhook.events(), &[WebhookEvent::AssayCompleted]);
        assert!(webhook.listens_to(WebhookEvent::AssayCompleted));
        assert!(!webhook.listens_to(WebhookEvent::PackagePublished));
    }
}
