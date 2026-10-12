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

    /// An environment variable is set to a value this server does not accept.
    #[error("invalid value for {name}: {value}")]
    InvalidValue {
        /// Environment variable name.
        name: &'static str,
        /// The rejected value.
        value: String,
    },
}

/// Where artifact bytes are stored.
///
/// This is not a repository kind. Forge, Mirror, and Alloy stay the
/// same; only the shelf under them changes. Switching from one variant
/// to the other does not copy objects.
pub enum StorageConfig {
    /// S3-compatible bucket. Garage in the default quickstart.
    S3 {
        /// Endpoint URL, for example `http://garage:3900`.
        endpoint_url: String,
        /// Region sent to the S3 client.
        region: String,
        /// Access-key identifier.
        access_key_id: String,
        /// Access-key secret.
        secret_access_key: String,
        /// Bucket name.
        bucket: String,
    },
    /// Directory on the local filesystem.
    Filesystem {
        /// Directory that will hold every object. Created at startup.
        path: PathBuf,
    },
}

impl StorageConfig {
    /// `s3` or `filesystem`.
    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::S3 { .. } => "s3",
            Self::Filesystem { .. } => "filesystem",
        }
    }

    /// Bucket name, or the configured directory. No credentials.
    #[must_use]
    pub fn location(&self) -> String {
        match self {
            Self::S3 { bucket, .. } => bucket.clone(),
            Self::Filesystem { path } => path.display().to_string(),
        }
    }
}

impl std::fmt::Debug for StorageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::S3 {
                endpoint_url,
                region,
                access_key_id,
                bucket,
                ..
            } => f
                .debug_struct("S3")
                .field("endpoint_url", endpoint_url)
                .field("region", region)
                .field("access_key_id", access_key_id)
                .field("secret_access_key", &"***")
                .field("bucket", bucket)
                .finish(),
            Self::Filesystem { path } => f.debug_struct("Filesystem").field("path", path).finish(),
        }
    }
}

/// Server configuration, assembled once at startup.
pub struct Config {
    /// `PostgreSQL` connection string.
    pub database_url: String,
    /// Where artifact bytes are stored. See [`StorageConfig`].
    pub storage: StorageConfig,
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
    /// Optional path to a local `ferrobox-osv-index` file (plain or gzipped).
    /// When set, assays consult this index instead of `api.osv.dev`.
    pub osv_feed_path: Option<PathBuf>,
    /// OCI reference of a Cosign-signed index, for example
    /// `ghcr.io/ferrobox/osv-db:2026-10-03`. Seeds the settings row on
    /// first boot when that row is empty. A row saved from the card wins.
    pub osv_sync_ref: Option<String>,
    /// Path of the PEM public key used to verify [`Self::osv_sync_ref`].
    pub osv_sync_key: Option<PathBuf>,
    /// Base64 PEM public key. Used when [`Self::osv_sync_key`] is unset.
    pub osv_sync_pubkey_b64: Option<String>,
    /// Optional bearer token for a private registry.
    pub osv_sync_token: Option<String>,
    /// Delay after a successful or rejected pull before the next one.
    pub osv_sync_interval: Duration,
}

impl Config {
    /// Reads the full configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingVar`] if a required variable is
    /// missing, or [`ConfigError::InvalidValue`] when `STORAGE_BACKEND`
    /// is neither `s3` nor `filesystem`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: require_env("DATABASE_URL")?,
            storage: storage_from_env()?,
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
            osv_feed_path: optional_env("OSV_FEED_PATH").map(PathBuf::from),
            osv_sync_ref: optional_env("OSV_SYNC_REF"),
            osv_sync_key: optional_env("OSV_SYNC_KEY").map(PathBuf::from),
            osv_sync_pubkey_b64: optional_env("OSV_SYNC_PUBKEY_B64"),
            osv_sync_token: optional_env("OSV_SYNC_TOKEN"),
            osv_sync_interval: osv_sync_interval_from_env(),
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

fn storage_from_env() -> Result<StorageConfig, ConfigError> {
    storage_from_values(
        optional_env("STORAGE_BACKEND").as_deref(),
        optional_env("STORAGE_PATH").as_deref(),
        optional_env("S3_ENDPOINT_URL").as_deref(),
        optional_env("S3_REGION").as_deref(),
        optional_env("S3_ACCESS_KEY_ID").as_deref(),
        optional_env("S3_SECRET_ACCESS_KEY").as_deref(),
        optional_env("S3_BUCKET").as_deref(),
    )
}

fn storage_from_values(
    backend: Option<&str>,
    path: Option<&str>,
    endpoint_url: Option<&str>,
    region: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
    bucket: Option<&str>,
) -> Result<StorageConfig, ConfigError> {
    let raw = backend.unwrap_or("s3");
    match raw.to_ascii_lowercase().as_str() {
        "s3" => Ok(StorageConfig::S3 {
            endpoint_url: required("S3_ENDPOINT_URL", endpoint_url)?,
            region: region.unwrap_or("garage").to_string(),
            access_key_id: required("S3_ACCESS_KEY_ID", access_key_id)?,
            secret_access_key: required("S3_SECRET_ACCESS_KEY", secret_access_key)?,
            bucket: required("S3_BUCKET", bucket)?,
        }),
        "filesystem" => Ok(StorageConfig::Filesystem {
            path: PathBuf::from(required("STORAGE_PATH", path)?),
        }),
        _ => Err(ConfigError::InvalidValue {
            name: "STORAGE_BACKEND",
            value: raw.to_string(),
        }),
    }
}

fn required(name: &'static str, value: Option<&str>) -> Result<String, ConfigError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(ConfigError::MissingVar(name))
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

fn osv_sync_interval_from_env() -> Duration {
    let hours = optional_env("OSV_SYNC_INTERVAL_HOURS")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|hours| *hours > 0)
        .unwrap_or(24);
    Duration::from_secs(hours.saturating_mul(3600))
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
    fn missing_storage_backend_is_s3_and_still_requires_s3() {
        let storage = storage_from_values(
            None,
            None,
            Some("http://garage:3900"),
            None,
            Some("GK0123456789abcdef01234567"),
            Some("secret"),
            Some("ferrobox"),
        )
        .unwrap();
        assert_eq!(storage.backend_name(), "s3");
        assert_eq!(storage.location(), "ferrobox");
    }

    #[test]
    fn filesystem_backend_does_not_require_s3() {
        let storage = storage_from_values(
            Some("FileSystem"),
            Some("/var/lib/ferrobox/blobs"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(storage.backend_name(), "filesystem");
        assert_eq!(storage.location(), "/var/lib/ferrobox/blobs");
    }

    #[test]
    fn filesystem_backend_requires_a_path() {
        let error = storage_from_values(Some("filesystem"), None, None, None, None, None, None)
            .unwrap_err();
        assert!(matches!(error, ConfigError::MissingVar("STORAGE_PATH")));
    }

    #[test]
    fn unknown_storage_backend_is_rejected() {
        let error =
            storage_from_values(Some("blob"), None, None, None, None, None, None).unwrap_err();
        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                name: "STORAGE_BACKEND",
                ..
            }
        ));
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
