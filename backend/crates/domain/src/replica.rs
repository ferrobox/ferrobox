//! Réplica push de un repositorio hacia otra instancia FerroBox.
//!
//! Una política por repositorio: URL remota, UUID del Forge destino y
//! un token de API. El pull y el cron quedan para un corte posterior.

use thiserror::Error;
use url::Url;

use crate::ids::RepositoryId;

const MAX_URL_LENGTH: usize = 2048;
const MAX_TOKEN_LENGTH: usize = 512;

/// Destino de un push de réplica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaTarget {
    remote_url: Url,
    destination_id: RepositoryId,
    token: Option<String>,
}

/// Resultado del último push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaRun {
    occurred_at: String,
    packages_imported: u32,
    artifacts_imported: u32,
    skipped: u32,
    error: Option<String>,
}

/// Política persistida de un repositorio. Sin destino no hay réplica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaPolicy {
    target: Option<ReplicaTarget>,
    last_run: Option<ReplicaRun>,
}

/// Motivos por los que una política de réplica no es válida.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReplicaPolicyError {
    /// La URL no es `http` ni `https`, o no tiene host.
    #[error("replica URL must be an absolute http or https URL")]
    InvalidUrl,

    /// La URL supera el máximo permitido.
    #[error("replica URL must be at most {max} characters")]
    UrlTooLong {
        /// Máximo permitido.
        max: usize,
    },

    /// El token supera el máximo permitido.
    #[error("replica token must be at most {max} characters")]
    TokenTooLong {
        /// Máximo permitido.
        max: usize,
    },

    /// El destino es el mismo repositorio.
    #[error("replica destination cannot be the source repository")]
    SameRepository,
}

impl ReplicaTarget {
    /// Construye un destino a partir de URL, UUID y token opcional.
    ///
    /// # Errors
    ///
    /// [`ReplicaPolicyError`] si la URL no es HTTP(S) o el destino es
    /// el propio origen.
    pub fn new(
        remote_url: impl AsRef<str>,
        destination_id: RepositoryId,
        token: Option<String>,
        source_id: RepositoryId,
    ) -> Result<Self, ReplicaPolicyError> {
        if destination_id == source_id {
            return Err(ReplicaPolicyError::SameRepository);
        }
        let remote_url = parse_remote_url(remote_url.as_ref())?;
        let token = parse_token(token)?;
        Ok(Self {
            remote_url,
            destination_id,
            token,
        })
    }

    /// URL de la instancia remota (origen, sin `/api`).
    #[must_use]
    pub fn remote_url(&self) -> &Url {
        &self.remote_url
    }

    /// Forge destino en la instancia remota.
    #[must_use]
    pub fn destination_id(&self) -> RepositoryId {
        self.destination_id
    }

    /// Token de API con escritura en el destino. `None` si aún no se
    /// guardó (o se borró).
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Sustituye el token. `None` conserva el anterior.
    #[must_use]
    pub fn with_token(mut self, token: Option<String>) -> Self {
        if token.is_some() {
            self.token = token;
        }
        self
    }

    /// URLs del `POST` de import en la instancia remota.
    ///
    /// Primero `{origen}/api/repositories/{id}/import` (compose / `FRONTEND_DIR`).
    /// Después `{origen}/repositories/{id}/import` (`cargo run` + Vite).
    #[must_use]
    pub fn import_urls(&self) -> [String; 2] {
        let origin = replica_origin(&self.remote_url);
        let dest = self.destination_id;
        [
            format!("{origin}/api/repositories/{dest}/import"),
            format!("{origin}/repositories/{dest}/import"),
        ]
    }
}

impl ReplicaRun {
    /// Construye el recuento de un push.
    #[must_use]
    pub fn new(
        occurred_at: impl Into<String>,
        packages_imported: u32,
        artifacts_imported: u32,
        skipped: u32,
        error: Option<String>,
    ) -> Self {
        Self {
            occurred_at: occurred_at.into(),
            packages_imported,
            artifacts_imported,
            skipped,
            error,
        }
    }

    /// Instante RFC 3339.
    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    /// Coordenadas nuevas en el destino.
    #[must_use]
    pub fn packages_imported(&self) -> u32 {
        self.packages_imported
    }

    /// Binarios nuevos en el destino.
    #[must_use]
    pub fn artifacts_imported(&self) -> u32 {
        self.artifacts_imported
    }

    /// Paquetes o binarios que ya estaban.
    #[must_use]
    pub fn skipped(&self) -> u32 {
        self.skipped
    }

    /// Error del remoto, si el push falló.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `true` si el remoto aceptó el bundle.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

impl ReplicaPolicy {
    /// Sin destino: el repositorio no replica.
    #[must_use]
    pub fn unconfigured() -> Self {
        Self {
            target: None,
            last_run: None,
        }
    }

    /// Política con destino y, opcionalmente, el último push.
    #[must_use]
    pub fn new(target: Option<ReplicaTarget>, last_run: Option<ReplicaRun>) -> Self {
        Self { target, last_run }
    }

    /// Destino, si está configurado.
    #[must_use]
    pub fn target(&self) -> Option<&ReplicaTarget> {
        self.target.as_ref()
    }

    /// Último push, si se ejecutó alguna vez.
    #[must_use]
    pub fn last_run(&self) -> Option<&ReplicaRun> {
        self.last_run.as_ref()
    }

    /// Sustituye el recuento del último push.
    #[must_use]
    pub fn with_last_run(mut self, last_run: ReplicaRun) -> Self {
        self.last_run = Some(last_run);
        self
    }
}

fn parse_remote_url(raw: &str) -> Result<Url, ReplicaPolicyError> {
    let raw = raw.trim();
    if raw.len() > MAX_URL_LENGTH {
        return Err(ReplicaPolicyError::UrlTooLong {
            max: MAX_URL_LENGTH,
        });
    }
    let parsed = Url::parse(raw).map_err(|_| ReplicaPolicyError::InvalidUrl)?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ReplicaPolicyError::InvalidUrl);
    }
    if parsed.host_str().is_none() {
        return Err(ReplicaPolicyError::InvalidUrl);
    }
    Ok(parsed)
}

fn parse_token(token: Option<String>) -> Result<Option<String>, ReplicaPolicyError> {
    let Some(token) = token else {
        return Ok(None);
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        return Ok(None);
    }
    if token.len() > MAX_TOKEN_LENGTH {
        return Err(ReplicaPolicyError::TokenTooLong {
            max: MAX_TOKEN_LENGTH,
        });
    }
    Ok(Some(token))
}

fn replica_origin(remote: &Url) -> String {
    let mut base = remote.as_str().trim_end_matches('/').to_string();
    if let Some(stripped) = base.strip_suffix("/api") {
        base = stripped.trim_end_matches('/').to_string();
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(label: u8) -> RepositoryId {
        RepositoryId::from(uuid::Uuid::from_u128(u128::from(label)))
    }

    #[test]
    fn rejects_same_repository_and_non_http() {
        assert!(matches!(
            ReplicaTarget::new("http://127.0.0.1:3000", repo(1), None, repo(1)),
            Err(ReplicaPolicyError::SameRepository)
        ));
        assert!(matches!(
            ReplicaTarget::new("ftp://files.example", repo(2), None, repo(1)),
            Err(ReplicaPolicyError::InvalidUrl)
        ));
    }

    #[test]
    fn import_url_always_uses_the_api_prefix() {
        let target = ReplicaTarget::new(
            "http://127.0.0.1:3000/api",
            repo(2),
            Some("tok".into()),
            repo(1),
        )
        .unwrap();
        assert_eq!(
            target.import_urls(),
            [
                format!("http://127.0.0.1:3000/api/repositories/{}/import", repo(2)),
                format!("http://127.0.0.1:3000/repositories/{}/import", repo(2)),
            ]
        );
    }
}
