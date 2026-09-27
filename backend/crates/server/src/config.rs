//! Server configuration, read from environment variables.

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

/// Reasons the server configuration is invalid.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A required environment variable is missing.
    #[error("missing required environment variable: {0}")]
    MissingVar(&'static str),
}

/// Server configuration, assembled once at startup.
pub struct Config {
    /// `PostgreSQL` connection string.
    pub database_url: String,
    /// S3 *endpoint* URL (Garage in development, AWS S3 in production).
    pub s3_endpoint_url: String,
    /// S3 region to use.
    pub s3_region: String,
    /// S3 access-key identifier.
    pub s3_access_key_id: String,
    /// S3 access-key secret.
    pub s3_secret_access_key: String,
    /// Name of the S3 *bucket* where artifacts are stored.
    pub s3_bucket: String,
    /// Address and port the HTTP server listens on.
    pub bind_address: String,
    /// Public URL (scheme + host + port, no trailing slash) at which
    /// this server is reachable. Used to build absolute URLs in
    /// protocols that require them, such as Cargo sparse-index
    /// `config.json`.
    pub public_base_url: String,
    /// Initial administrator username (used only if there are no users).
    pub admin_username: String,
    /// Initial administrator password (used only if there are no users).
    pub admin_password: String,
    /// Static UI directory. When set, the server nests the API under
    /// `/api` and serves the SPA on the remaining routes.
    pub frontend_dir: Option<String>,
    /// `OIDC` issuer (realm URL). Empty = SSO disabled.
    pub oidc_issuer: Option<String>,
    /// `client_id` of the client in the `IdP`.
    pub oidc_client_id: Option<String>,
    /// Client secret. Optional if the client is public + PKCE.
    pub oidc_client_secret: Option<String>,
    /// Return URL. If missing, it is derived from `PUBLIC_BASE_URL`.
    pub oidc_redirect_uri: Option<String>,
    /// UI destination after login. If missing, `{PUBLIC_BASE_URL}/login`.
    pub oidc_success_redirect: Option<String>,
    /// Scopes (`openid profile email` by default).
    pub oidc_scopes: Option<String>,
    /// `IdP` roles that grant Admin, comma-separated.
    pub oidc_admin_roles: Option<String>,
    /// `IdP` roles that grant Developer, comma-separated.
    pub oidc_developer_roles: Option<String>,
    /// `IdP` roles that grant Reader, comma-separated.
    pub oidc_reader_roles: Option<String>,
    /// Extra roles *claim* (in addition to `realm_access` / `roles`).
    pub oidc_role_claim: Option<String>,
    /// Groups *claim* (`groups` by default).
    pub oidc_group_claim: Option<String>,
    /// If `false`, groups that do not yet exist are not created.
    pub oidc_auto_create_groups: bool,
    /// Session token expiry (login and SSO).
    pub session_ttl: Duration,
}

impl Config {
    /// Reads the full configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingVar`] if a required variable is
    /// missing.
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

/// Loads `.env` files and **overrides** variables already exported in the shell.
///
/// If `FERROBOX_ENV_FILE` points to a file (`.env_a`, `.env_b`),
/// only that one is loaded: two `cargo run` processes do not overwrite
/// each other's shared `.env`. Otherwise the last candidate wins.
/// `backend/.env` (next to the server crate) is applied last so an
/// `S3_BUCKET` exported in the terminal does not win. The bucket is
/// not read from the database.
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
