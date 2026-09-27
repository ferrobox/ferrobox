use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;

/// Reasons an outbound HTTP request can fail.
#[derive(Debug, Error)]
pub enum HttpClientError {
    /// The remote server responded with an HTTP error status.
    #[error("upstream HTTP {status} for {url}")]
    Status {
        /// HTTP status code received.
        status: u16,
        /// Requested URL.
        url: String,
    },

    /// Network, TLS, or concrete HTTP library failure.
    #[error("HTTP transport failure for {url}: {message}")]
    Transport {
        /// Requested URL.
        url: String,
        /// Failure detail.
        message: String,
    },
}

/// Minimal HTTP response needed by the application layer.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body.
    pub body: Bytes,
    /// Response headers. Names are stored in lowercase.
    pub headers: Vec<(String, String)>,
}

impl HttpResponse {
    /// Builds a response with no headers.
    #[must_use]
    pub fn new(status: u16, body: Bytes) -> Self {
        Self {
            status,
            body,
            headers: Vec::new(),
        }
    }

    /// `true` if the status code is in the 2xx range.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// First header whose name matches case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find_map(|(key, value)| {
            key.eq_ignore_ascii_case(name).then_some(value.as_str())
        })
    }
}

/// Outbound HTTP client port (for example, to query the *upstream* of
/// a `Mirror` repository).
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Performs a `GET` request to `url`.
    ///
    /// # Errors
    ///
    /// Returns [`HttpClientError`] if the network fails or the remote
    /// responds outside 2xx (according to the adapter policy).
    async fn get(&self, url: &str) -> Result<HttpResponse, HttpClientError>;

    /// `GET` with extra headers. Unlike [`get`], returns the body and
    /// headers even outside 2xx (needed for the `Bearer` challenge of
    /// an OCI registry). Only transport failures.
    ///
    /// # Errors
    ///
    /// Returns [`HttpClientError::Transport`] if the network or TLS fail.
    async fn get_with_headers(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpClientError>;

    /// Performs a `POST` request with a body and `Content-Type`.
    ///
    /// Like [`get`], only returns `Ok` in the 2xx range.
    ///
    /// # Errors
    ///
    /// Returns [`HttpClientError`] if the network fails or the remote
    /// responds outside 2xx.
    async fn post(
        &self,
        url: &str,
        body: Bytes,
        content_type: &str,
    ) -> Result<HttpResponse, HttpClientError> {
        let response = self
            .post_with_headers(url, body, &[("content-type", content_type)])
            .await?;
        if response.is_success() {
            Ok(response)
        } else {
            Err(HttpClientError::Status {
                status: response.status,
                url: url.to_string(),
            })
        }
    }

    /// `POST` with extra headers. Like [`get_with_headers`], returns
    /// the body even outside 2xx: needed to record the result of an
    /// HTTP webhook. Only transport failures.
    ///
    /// # Errors
    ///
    /// Returns [`HttpClientError::Transport`] if the network or TLS fail.
    async fn post_with_headers(
        &self,
        url: &str,
        body: Bytes,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpClientError>;
}
