use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;

/// Motivos por los que una petición HTTP saliente puede fallar.
#[derive(Debug, Error)]
pub enum HttpClientError {
    /// El servidor remoto respondió con un código de error HTTP.
    #[error("upstream HTTP {status} for {url}")]
    Status {
        /// Código de estado HTTP recibido.
        status: u16,
        /// URL solicitada.
        url: String,
    },

    /// Fallo de red, TLS o de la librería HTTP concreta.
    #[error("HTTP transport failure for {url}: {message}")]
    Transport {
        /// URL solicitada.
        url: String,
        /// Detalle del fallo.
        message: String,
    },
}

/// Respuesta HTTP mínima que necesita la capa de aplicación.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// Código de estado HTTP.
    pub status: u16,
    /// Cuerpo de la respuesta.
    pub body: Bytes,
    /// Cabeceras de respuesta. Los nombres se guardan en minúsculas.
    pub headers: Vec<(String, String)>,
}

impl HttpResponse {
    /// Construye una respuesta sin cabeceras.
    #[must_use]
    pub fn new(status: u16, body: Bytes) -> Self {
        Self {
            status,
            body,
            headers: Vec::new(),
        }
    }

    /// `true` si el código de estado está en el rango 2xx.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Primera cabecera cuyo nombre coincide sin distinguir mayúsculas.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find_map(|(key, value)| {
            key.eq_ignore_ascii_case(name).then_some(value.as_str())
        })
    }
}

/// Puerto de cliente HTTP saliente (por ejemplo, para consultar el
/// *upstream* de un repositorio `Mirror`).
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Realiza una petición `GET` a `url`.
    ///
    /// # Errors
    ///
    /// Devuelve [`HttpClientError`] si la red falla o el remoto
    /// responde fuera de 2xx (según la política del adaptador).
    async fn get(&self, url: &str) -> Result<HttpResponse, HttpClientError>;

    /// `GET` con cabeceras extra. A diferencia de [`get`], devuelve el
    /// cuerpo y las cabeceras también fuera de 2xx (hace falta para el
    /// desafío `Bearer` de un registro OCI). Solo falla de transporte.
    ///
    /// # Errors
    ///
    /// Devuelve [`HttpClientError::Transport`] si la red o TLS fallan.
    async fn get_with_headers(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpClientError>;

    /// Realiza una petición `POST` con cuerpo y `Content-Type`.
    ///
    /// Como [`get`], solo devuelve `Ok` en el rango 2xx.
    ///
    /// # Errors
    ///
    /// Devuelve [`HttpClientError`] si la red falla o el remoto responde
    /// fuera de 2xx.
    async fn post(
        &self,
        url: &str,
        body: Bytes,
        content_type: &str,
    ) -> Result<HttpResponse, HttpClientError>;
}
