//! Configuración del servidor, leída desde variables de entorno.

use std::path::PathBuf;
use std::time::Duration;

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
    /// Emisor `OIDC` (URL del realm). Vacío = SSO desactivado.
    pub oidc_issuer: Option<String>,
    /// `client_id` del cliente en el `IdP`.
    pub oidc_client_id: Option<String>,
    /// Secreto del cliente. Opcional si el cliente es público + PKCE.
    pub oidc_client_secret: Option<String>,
    /// URL de retorno. Si falta, se deriva de `PUBLIC_BASE_URL`.
    pub oidc_redirect_uri: Option<String>,
    /// Destino de la UI tras el login. Si falta, `{PUBLIC_BASE_URL}/login`.
    pub oidc_success_redirect: Option<String>,
    /// Ámbitos (`openid profile email` por defecto).
    pub oidc_scopes: Option<String>,
    /// Roles del `IdP` que conceden Admin, separados por coma.
    pub oidc_admin_roles: Option<String>,
    /// Roles del `IdP` que conceden Developer, separados por coma.
    pub oidc_developer_roles: Option<String>,
    /// Roles del `IdP` que conceden Reader, separados por coma.
    pub oidc_reader_roles: Option<String>,
    /// *Claim* extra de roles (además de `realm_access` / `roles`).
    pub oidc_role_claim: Option<String>,
    /// *Claim* de grupos (`groups` por defecto).
    pub oidc_group_claim: Option<String>,
    /// Si `false`, no se crean grupos que aún no existan.
    pub oidc_auto_create_groups: bool,
    /// Caducidad de los tokens de sesión (login y SSO).
    pub session_ttl: Duration,
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
            oidc_issuer: optional_env("OIDC_ISSUER"),
            oidc_client_id: optional_env("OIDC_CLIENT_ID"),
            oidc_client_secret: optional_env("OIDC_CLIENT_SECRET"),
            oidc_redirect_uri: optional_env("OIDC_REDIRECT_URI"),
            oidc_success_redirect: optional_env("OIDC_SUCCESS_REDIRECT"),
            oidc_scopes: optional_env("OIDC_SCOPES"),
            oidc_admin_roles: optional_env("OIDC_ADMIN_ROLES"),
            oidc_developer_roles: optional_env("OIDC_DEVELOPER_ROLES"),
            oidc_reader_roles: optional_env("OIDC_READER_ROLES"),
            oidc_role_claim: optional_env("OIDC_ROLE_CLAIM"),
            oidc_group_claim: optional_env("OIDC_GROUP_CLAIM"),
            oidc_auto_create_groups: env_flag("OIDC_AUTO_CREATE_GROUPS", true),
            session_ttl: session_ttl_from_env(),
        })
    }
}

/// Carga ficheros `.env` y **pisa** variables ya exportadas en el shell.
///
/// Si `FERROBOX_ENV_FILE` apunta a un fichero (`.env_a`, `.env_b`),
/// solo se carga ese: dos `cargo run` no se pisan el `.env` compartido.
/// Si no, el último candidato gana. `backend/.env` (junto al crate del
/// servidor) se aplica al final para que un `S3_BUCKET` exportado en
/// la terminal no gane. El bucket no se lee de la base de datos.
pub fn load_dotenv() -> Vec<PathBuf> {
    let explicit = optional_env("FERROBOX_ENV_FILE");
    let mut loaded = Vec::new();
    for path in dotenv_candidates() {
        let Some(canonical) = load_env_file(&path) else {
            continue;
        };
        if loaded.iter().any(|seen| seen == &canonical) {
            continue;
        }
        loaded.push(canonical);
    }
    if let Some(path) = explicit {
        if loaded.is_empty() {
            let tried = dotenv_candidates_from(Some(&path))
                .into_iter()
                .map(|candidate| candidate.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            panic!(
                "ferrobox: FERROBOX_ENV_FILE={path} was not found or could not be read (tried {tried})"
            );
        }
    }
    loaded
}

fn dotenv_candidates() -> Vec<PathBuf> {
    dotenv_candidates_from(optional_env("FERROBOX_ENV_FILE").as_deref())
}

fn dotenv_candidates_from(explicit: Option<&str>) -> Vec<PathBuf> {
    if let Some(path) = explicit {
        return explicit_env_paths(path);
    }
    let mut paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".env"));
        paths.push(cwd.join("backend/.env"));
    }
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.env"));
    paths
}

fn explicit_env_paths(path: &str) -> Vec<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return vec![path];
    }
    let crate_backend = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(&path));
        paths.push(cwd.join("backend").join(&path));
        paths.push(cwd.join("..").join(&path));
    }
    paths.push(crate_backend.join(&path));
    paths.push(crate_backend.join("..").join(&path));
    paths
}

fn load_env_file(path: &std::path::Path) -> Option<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        if dotenvy::from_path_override(&canonical).is_ok() {
            return Some(canonical);
        }
    }
    if path.is_file() && dotenvy::from_path_override(path).is_ok() {
        return Some(path.to_path_buf());
    }
    None
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

fn env_flag(key: &str, default: bool) -> bool {
    match optional_env(key) {
        Some(value) => matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => default,
    }
}

fn session_ttl_from_env() -> Duration {
    let hours = optional_env("SESSION_TTL_HOURS")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|hours| *hours > 0)
        .unwrap_or(12);
    Duration::from_secs(hours.saturating_mul(3600))
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
        let last = dotenv_candidates_from(None)
            .pop()
            .expect("the crate-relative backend/.env is always a candidate");
        assert!(
            last.ends_with("../../.env"),
            "expected crates/server/../../.env (backend/.env), got {}",
            last.display()
        );
    }

    #[test]
    fn relative_env_file_also_looks_in_the_repo_root() {
        let paths = dotenv_candidates_from(Some(".env_b"));
        assert!(
            paths.iter().any(|path| path.ends_with("../../.env_b")),
            "expected a crate-relative backend/.env_b, got {paths:?}"
        );
        assert!(
            paths.iter().any(|path| path.ends_with("../../../.env_b")),
            "expected the git-root .env_b next to backend/, got {paths:?}"
        );
    }

    #[test]
    fn absolute_env_file_is_used_as_is() {
        let paths = dotenv_candidates_from(Some("/tmp/.env_a"));
        assert_eq!(paths, vec![PathBuf::from("/tmp/.env_a")]);
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
