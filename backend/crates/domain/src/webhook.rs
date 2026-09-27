//! HTTP webhook: a POST to a URL when an event occurs
//! in a repository. Install and publish do not wait for the response.

use std::fmt;

use thiserror::Error;
use url::Url;

use crate::ids::{RepositoryId, WebhookDeliveryId, WebhookId};

const MAX_NAME_LENGTH: usize = 100;
const MAX_SECRET_LENGTH: usize = 256;
const MAX_URL_LENGTH: usize = 2048;

/// Event that can trigger an HTTP webhook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebhookEvent {
    /// An assay has finished (ready, failed, or unsupported).
    AssayCompleted,
    /// A version has become available in the repository (publish or
    /// caching of a Mirror).
    PackagePublished,
}

impl WebhookEvent {
    /// Stable label used in persistence, API, and HTTP header.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AssayCompleted => "assay.completed",
            Self::PackagePublished => "package.published",
        }
    }

    /// Parses the stable label. `ping` is not a persisted event:
    /// it is only used when testing the webhook.
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

/// Result of a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookDeliveryStatus {
    /// The destination responded 2xx.
    Success,
    /// Network, TLS, or a response outside 2xx.
    Failed,
}

impl WebhookDeliveryStatus {
    /// Stable label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
        }
    }

    /// Parses the stable label.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Self::Success),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Reasons why a webhook is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebhookError {
    /// The name cannot be empty.
    #[error("webhook name cannot be empty")]
    EmptyName,

    /// The name exceeds the maximum length.
    #[error("webhook name cannot exceed {max} characters, got {actual}")]
    NameTooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length.
        actual: usize,
    },

    /// The URL is neither `http` nor `https`, or it has no host.
    #[error("webhook URL must be an absolute http or https URL")]
    InvalidUrl,

    /// The URL exceeds the maximum length.
    #[error("webhook URL cannot exceed {max} characters")]
    UrlTooLong {
        /// Maximum allowed length.
        max: usize,
    },

    /// The secret exceeds the maximum length.
    #[error("webhook secret cannot exceed {max} characters")]
    SecretTooLong {
        /// Maximum allowed length.
        max: usize,
    },

    /// At least one event must be chosen.
    #[error("select at least one webhook event")]
    NoEvents,
}

/// An HTTP webhook associated with a repository.
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
    /// Creates a new webhook.
    ///
    /// # Errors
    ///
    /// [`WebhookError`] if the name, URL, secret, or events
    /// are not valid.
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

    /// Reconstitutes an already persisted webhook.
    ///
    /// # Errors
    ///
    /// [`WebhookError`] if any field does not meet the invariants.
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

    /// Identifier.
    #[must_use]
    pub fn id(&self) -> WebhookId {
        self.id
    }

    /// Repository it belongs to.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Descriptive name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Destination URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// HMAC secret, if any.
    #[must_use]
    pub fn secret(&self) -> Option<&str> {
        self.secret.as_deref()
    }

    /// Subscribed events.
    #[must_use]
    pub fn events(&self) -> &[WebhookEvent] {
        &self.events
    }

    /// `true` if it is active.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// `true` if this webhook should fire for `event`.
    #[must_use]
    pub fn listens_to(&self, event: WebhookEvent) -> bool {
        self.enabled && self.events.contains(&event)
    }
}

/// An attempt to deliver to a webhook.
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
    /// Builds a delivery that is already persisted or just performed.
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

    /// Delivery identifier.
    #[must_use]
    pub fn id(&self) -> WebhookDeliveryId {
        self.id
    }

    /// Webhook it belongs to.
    #[must_use]
    pub fn webhook_id(&self) -> WebhookId {
        self.webhook_id
    }

    /// Event name (`assay.completed`, `package.published`, `ping`).
    #[must_use]
    pub fn event(&self) -> &str {
        &self.event
    }

    /// Result.
    #[must_use]
    pub fn status(&self) -> WebhookDeliveryStatus {
        self.status
    }

    /// HTTP status of the destination, if it responded.
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    /// Failure detail, if any.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// RFC 3339 timestamp.
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
