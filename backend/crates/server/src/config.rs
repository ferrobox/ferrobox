//! Configuración del servidor, leída desde variables de entorno.

use std::path::PathBuf;

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
    /// URL pública (esquema + host + puerto, sin barra final) bajo la
    /// que este servidor es alcanzable. Se usa para construir URLs
    /// absolutas en protocolos que las requieren, como el `config.json`
    /// del índice disperso de Cargo.
    pub public_base_url: String,
    /// Nombre del administrador inicial (solo se usa si no hay usuarios).
    pub admin_username: String,
    /// Contraseña del administrador inicial (solo se usa si no hay usuarios).
    pub admin_password: String,
    /// Directorio de la UI estática. Si está, el servidor anida la API
    /// en `/api` y sirve el SPA en el resto de rutas.
    pub frontend_dir: Option<String>,
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
            public_base_url: env_or("PUBLIC_BASE_URL", "http://127.0.0.1:3000"),
            admin_username: env_or("ADMIN_USERNAME", "admin"),
            admin_password: env_or("ADMIN_PASSWORD", "admin"),
            frontend_dir: optional_env("FRONTEND_DIR"),
        })
    }
}

/// Carga ficheros `.env` y **pisa** variables ya exportadas en el shell.
///
/// El último fichero gana. `backend/.env` (junto al crate del servidor)
/// se aplica al final para que `cargo run` no se quede con un
/// `S3_BUCKET` antiguo exportado en la terminal. El bucket no se lee
/// de la base de datos.
pub fn load_dotenv() -> Vec<PathBuf> {
    let mut loaded = Vec::new();
    for path in dotenv_candidates() {
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if loaded.iter().any(|seen| seen == &canonical) {
            continue;
        }
        if dotenvy::from_path_override(&canonical).is_ok() {
            loaded.push(canonical);
        }
    }
    loaded
}

fn dotenv_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".env"));
        paths.push(cwd.join("backend/.env"));
    }
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.env"));
    paths
}

fn require_env(key: &'static str) -> Result<String, ConfigError> {
    std::env::var(key)
        .ok()
        .and_then(normalize_env)
        .ok_or(ConfigError::MissingVar(key))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .and_then(normalize_env)
        .unwrap_or_else(|| default.to_string())
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(normalize_env)
}

fn normalize_env(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_dotenv_is_backend_env() {
        let last = dotenv_candidates()
            .pop()
            .expect("the crate-relative backend/.env is always a candidate");
        assert!(
            last.ends_with("../../.env"),
            "expected crates/server/../../.env (backend/.env), got {}",
            last.display()
        );
    }

    #[test]
    fn normalize_env_trims_and_rejects_blank() {
        assert_eq!(normalize_env("  ferrobox  ").as_deref(), Some("ferrobox"));
        assert_eq!(
            normalize_env("ferrobox-artifacts").as_deref(),
            Some("ferrobox-artifacts")
        );
        assert_eq!(normalize_env("   "), None);
        assert_eq!(normalize_env(""), None);
    }
}
