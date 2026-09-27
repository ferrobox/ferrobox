//! [`HttpClient`](ferrobox_ports::http_client::HttpClient) adapter
//! backed by `reqwest`.

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_ports::http_client::{HttpClient, HttpClientError, HttpResponse};
use reqwest::Client;

/// Outgoing HTTP client backed by `reqwest`.
pub struct ReqwestHttpClient {
    client: Client,
}

impl ReqwestHttpClient {
    /// Builds a client with `reqwest`'s default configuration
    /// (rustls TLS, redirects followed).
    ///
    /// # Panics
    ///
    /// In practice this should not panic: building the `reqwest` client
    /// only fails if TLS connectors are missing from the environment.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("ferrobox/0.1")
                .build()
                .expect("reqwest client should build with rustls"),
        }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get(&self, url: &str) -> Result<HttpResponse, HttpClientError> {
        let response = self.get_with_headers(url, &[]).await?;
        if response.is_success() {
            Ok(response)
        } else {
            Err(HttpClientError::Status {
                status: response.status,
                url: url.to_string(),
            })
        }
    }

    async fn get_with_headers(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpClientError> {
        let mut request = self.client.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let response = request.send().await.map_err(|err| HttpClientError::Transport {
            url: url.to_string(),
            message: err.to_string(),
        })?;

        let status = response.status().as_u16();
        let header_pairs = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_ascii_lowercase(), text.to_string()))
            })
            .collect();
        let body = response.bytes().await.map_err(|err| HttpClientError::Transport {
            url: url.to_string(),
            message: err.to_string(),
        })?;

        Ok(HttpResponse {
            status,
            body,
            headers: header_pairs,
        })
    }

    async fn post_with_headers(
        &self,
        url: &str,
        body: Bytes,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpClientError> {
        let mut request = self.client.post(url).body(body.to_vec());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let response = request.send().await.map_err(|err| HttpClientError::Transport {
            url: url.to_string(),
            message: err.to_string(),
        })?;

        let status = response.status().as_u16();
        let header_pairs = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_ascii_lowercase(), text.to_string()))
            })
            .collect();
        let body = response.bytes().await.map_err(|err| HttpClientError::Transport {
            url: url.to_string(),
            message: err.to_string(),
        })?;

        Ok(HttpResponse {
            status,
            body,
            headers: header_pairs,
        })
    }
}
