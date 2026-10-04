//! Replace the upstream URL of a `Mirror` after it has been created.

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::{Repository, RepositoryKind};
use thiserror::Error;
use url::Url;

use crate::get_repository::{GetRepositoryError, GetRepositoryUseCase};

/// Reasons the upstream URL cannot be saved.
#[derive(Debug, Error)]
pub enum SetMirrorUpstreamError {
    /// Only a `Mirror` has an upstream.
    #[error("cannot change the upstream of a {0} repository")]
    NotAMirror(&'static str),

    /// The URL is not absolute `http`/`https`, or it embeds userinfo.
    #[error("{0}")]
    InvalidUpstream(String),

    /// The repository does not exist or persistence failed.
    #[error(transparent)]
    Repository(#[from] GetRepositoryError),
}

/// Accepts an absolute `http` or `https` URL and rejects userinfo.
///
/// A username or password in the URL would be stored next to the
/// repository and could be returned by the API. Those belong in the
/// mirror credential, which is never returned.
///
/// # Errors
///
/// [`SetMirrorUpstreamError::InvalidUpstream`] when `raw` is not a
/// usable upstream URL.
pub fn parse_mirror_upstream(raw: &str) -> Result<Url, SetMirrorUpstreamError> {
    let url = Url::parse(raw.trim()).map_err(|_| {
        SetMirrorUpstreamError::InvalidUpstream("upstream URL is not valid".to_string())
    })?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(SetMirrorUpstreamError::InvalidUpstream(
            "upstream URL must use http or https".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.as_str().contains('@') {
        return Err(SetMirrorUpstreamError::InvalidUpstream(
            "upstream URL must not include a username or password".to_string(),
        ));
    }
    Ok(url)
}

/// Use case: point a `Mirror` at a different upstream.
pub struct SetMirrorUpstreamUseCase;

impl SetMirrorUpstreamUseCase {
    /// Replaces the upstream URL. Identity, ecosystem, prefetch
    /// schedule, stored credential, and cached packages stay.
    ///
    /// # Errors
    ///
    /// [`SetMirrorUpstreamError`] if it is not a `Mirror`, the URL is
    /// not valid, or persistence fails.
    pub async fn execute(
        repositories: &GetRepositoryUseCase,
        id: RepositoryId,
        upstream: &str,
    ) -> Result<Repository, SetMirrorUpstreamError> {
        let repository = repositories.execute(id).await?;
        if !matches!(repository.kind(), RepositoryKind::Mirror { .. }) {
            return Err(SetMirrorUpstreamError::NotAMirror(
                repository.kind().label(),
            ));
        }

        let upstream = parse_mirror_upstream(upstream)?;
        let updated = repository
            .with_kind(RepositoryKind::Mirror { upstream })
            .map_err(|_| {
                SetMirrorUpstreamError::InvalidUpstream("upstream URL is not valid".to_string())
            })?;
        repositories.save(&updated).await?;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{DateTime, Utc};
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};

    use super::*;
    use crate::get_repository::GetRepositoryUseCase;
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::InMemoryRepositoryStore;

    fn cargo_mirror() -> Repository {
        Repository::new(
            RepositoryName::parse("crates-io").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.example/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap()
    }

    fn stamped() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-26T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[tokio::test]
    async fn replaces_the_url_and_keeps_the_schedule() {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let repository = cargo_mirror().with_prefetch_schedule(Some(6), Some(stamped()));
        let id = repository.id();
        store.save(&repository).await.unwrap();
        let get = GetRepositoryUseCase::new(store.clone());

        let updated = SetMirrorUpstreamUseCase::execute(&get, id, "https://other.example/index")
            .await
            .unwrap();

        let RepositoryKind::Mirror { upstream } = updated.kind() else {
            panic!("mirror kind");
        };
        assert_eq!(upstream.as_str(), "https://other.example/index");
        assert_eq!(updated.id(), id);
        assert_eq!(updated.name().as_str(), "crates-io");
        assert_eq!(updated.ecosystem(), PackageEcosystem::Cargo);
        assert_eq!(updated.prefetch_interval_hours(), Some(6));
        assert_eq!(updated.last_prefetch_at(), Some(stamped()));

        let saved = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(saved.prefetch_interval_hours(), Some(6));
    }

    #[tokio::test]
    async fn rejects_userinfo_a_non_http_scheme_and_a_forge() {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let repository = cargo_mirror();
        store.save(&repository).await.unwrap();
        let forge = Repository::new(
            RepositoryName::parse("local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        store.save(&forge).await.unwrap();
        let get = GetRepositoryUseCase::new(store);

        let userinfo = SetMirrorUpstreamUseCase::execute(
            &get,
            repository.id(),
            "https://ci-bot:secret-value@registry.example/npm",
        )
        .await
        .unwrap_err();
        let SetMirrorUpstreamError::InvalidUpstream(message) = userinfo else {
            panic!("expected invalid upstream");
        };
        assert!(!message.contains("secret-value"));
        assert!(message.contains("username or password"));

        let scheme =
            SetMirrorUpstreamUseCase::execute(&get, repository.id(), "ftp://example.com/index")
                .await
                .unwrap_err();
        assert!(matches!(scheme, SetMirrorUpstreamError::InvalidUpstream(_)));

        let forge_err =
            SetMirrorUpstreamUseCase::execute(&get, forge.id(), "https://example.com/index")
                .await
                .unwrap_err();
        assert!(matches!(
            forge_err,
            SetMirrorUpstreamError::NotAMirror("forge")
        ));
    }
}
