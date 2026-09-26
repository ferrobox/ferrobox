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
/// Un path relativo, o un absoluto que no existe, se busca subiendo
/// desde el cwd por el nombre del fichero. Si no hay variable, el
/// último candidato gana. `backend/.env` se aplica al final para que
/// un `S3_BUCKET` exportado en la terminal no gane.
pub fn load_dotenv() -> Vec<PathBuf> {
    let explicit = optional_env("FERROBOX_ENV_FILE");
    let mut loaded = Vec::new();
    for path in dotenv_candidates() {
        if let Probe::Loaded(canonical) = probe_env_file(&path) {
            if loaded.iter().any(|seen| seen == &canonical) {
                continue;
            }
            loaded.push(canonical);
        }
    }
    if let Some(path) = explicit {
        if loaded.is_empty() {
            panic!("{}", env_file_not_found(&path));
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
    let given = PathBuf::from(path);
    let name = given
        .file_name()
        .map_or_else(|| given.clone(), PathBuf::from);
    let mut paths = Vec::new();
    if given.is_absolute() {
        paths.push(given);
    }
    paths.extend(walk_named(&name));
    let crate_backend = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    paths.push(crate_backend.join(&name));
    paths.push(crate_backend.join("..").join(&name));
    paths
}

fn walk_named(name: &std::path::Path) -> Vec<PathBuf> {
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    let mut dir = cwd;
    let mut paths = Vec::new();
    for _ in 0..8 {
        paths.push(dir.join(name));
        if !dir.pop() {
            break;
        }
    }
    paths
}

enum Probe {
    Loaded(PathBuf),
    Missing,
    NotFile,
    Error(String),
}

fn probe_env_file(path: &std::path::Path) -> Probe {
    match path.metadata() {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
        Err(err) => Probe::Error(err.to_string()),
        Ok(meta) if !meta.is_file() => Probe::NotFile,
        Ok(_) => match dotenvy::from_path_override(path) {
            Ok(()) => Probe::Loaded(path.canonicalize().unwrap_or_else(|_| path.to_path_buf())),
            Err(err) => Probe::Error(err.to_string()),
        },
    }
}

fn env_file_not_found(requested: &str) -> String {
    let mut lines = vec![format!("ferrobox: FERROBOX_ENV_FILE={requested}")];
    let mut seen = Vec::new();
    for path in dotenv_candidates_from(Some(requested)) {
        let display = path.display().to_string();
        if seen.iter().any(|previous: &String| previous == &display) {
            continue;
        }
        seen.push(display.clone());
        let detail = match probe_env_file(&path) {
            Probe::Loaded(_) => continue,
            Probe::Missing => "missing".to_string(),
            Probe::NotFile => "not a file".to_string(),
            Probe::Error(err) => err,
        };
        lines.push(format!("  {display}: {detail}"));
    }
    lines.push(
        "Save .env_a and .env_b to disk, then from backend/: FERROBOX_ENV_FILE=.env_a cargo run -p ferrobox-server".to_string(),
    );
    lines.join("\n")
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
    fn absolute_env_file_is_tried_first_then_the_basename() {
        let paths = dotenv_candidates_from(Some("/tmp/.env_a"));
        assert_eq!(paths.first(), Some(&PathBuf::from("/tmp/.env_a")));
        assert!(
            paths
                .iter()
                .any(|path| path.file_name().and_then(|name| name.to_str()) == Some(".env_a")),
            "expected to also search for the basename .env_a, got {paths:?}"
        );
    }

    #[test]
    fn missing_env_file_lists_each_probe() {
        let message = env_file_not_found("/tmp/ferrobox-no-such-env-file");
        assert!(message.contains("missing"), "{message}");
        assert!(
            message.contains("/tmp/ferrobox-no-such-env-file"),
            "{message}"
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
