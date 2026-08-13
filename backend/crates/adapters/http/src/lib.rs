//! Adaptador de [`HttpClient`](ferrobox_ports::http_client::HttpClient)
//! basado en `reqwest`.

use async_trait::async_trait;
use ferrobox_ports::http_client::{HttpClient, HttpClientError, HttpResponse};
use reqwest::Client;

/// Cliente HTTP saliente respaldado por `reqwest`.
pub struct ReqwestHttpClient {
    client: Client,
}

impl ReqwestHttpClient {
    /// Construye un cliente con la configuración por defecto de
    /// `reqwest` (TLS rustls, redirecciones seguidas).
    ///
    /// # Panics
    ///
    /// En la práctica no debería entrar en pánico: fallar al construir
    /// el cliente de `reqwest` solo ocurre si faltan conectores TLS en
    /// el entorno.
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
        let response = self.client.get(url).send().await.map_err(|err| {
            HttpClientError::Transport {
                url: url.to_string(),
                message: err.to_string(),
            }
        })?;

        let status = response.status().as_u16();
        let body = response.bytes().await.map_err(|err| HttpClientError::Transport {
            url: url.to_string(),
            message: err.to_string(),
        })?;

        if !(200..300).contains(&status) {
            return Err(HttpClientError::Status {
                status,
                url: url.to_string(),
            });
        }

        Ok(HttpResponse {
            status,
            body,
        })
    }
}
