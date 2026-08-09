//! Configuración del servidor, leída desde variables de entorno.

use thiserror::Error;

/// Motivos por los que la configuración del servidor es inválida.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Falta una variable de entorno obligatoria.
    #[error("missing required environment variable: {0}")]
    MissingVar(&'static str),
}

/// Configuración del servidor, ensamblada una sola vez al arrancar.
pub struct Config {
    /// Cadena de conexión a `PostgreSQL`.
    pub database_url: String,
    /// URL del *endpoint* S3 (Garage en desarrollo, AWS S3 en producción).
    pub s3_endpoint_url: String,
    /// Región S3 a usar.
    pub s3_region: String,
    /// Identificador de la clave de acceso S3.
    pub s3_access_key_id: String,
    /// Secreto de la clave de acceso S3.
    pub s3_secret_access_key: String,
    /// Nombre del *bucket* S3 donde se almacenan los artefactos.
    pub s3_bucket: String,
    /// Dirección y puerto en los que escucha el servidor HTTP.
    pub bind_address: String,
}

impl Config {
    /// Lee la configuración completa desde variables de entorno.
    ///
    /// # Errors
    ///
    /// Devuelve [`ConfigError::MissingVar`] si falta alguna variable
    /// obligatoria.
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: require_env("DATABASE_URL")?,
            s3_endpoint_url: require_env("S3_ENDPOINT_URL")?,
            s3_region: env_or("S3_REGION", "garage"),
            s3_access_key_id: require_env("S3_ACCESS_KEY_ID")?,
            s3_secret_access_key: require_env("S3_SECRET_ACCESS_KEY")?,
            s3_bucket: require_env("S3_BUCKET")?,
            bind_address: env_or("BIND_ADDRESS", "127.0.0.1:3000"),
        })
    }
}

fn require_env(key: &'static str) -> Result<String, ConfigError> {
    std::env::var(key).map_err(|_| ConfigError::MissingVar(key))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
