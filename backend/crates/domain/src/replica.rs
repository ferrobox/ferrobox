//! Push or pull replica of a repository to another FerroBox instance.
//!
//! One policy per repository: direction, remote URL, remote Forge UUID,
//! API token, and an optional interval for the background cron.

use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;
use url::Url;

use crate::ids::RepositoryId;

const MAX_URL_LENGTH: usize = 2048;
const MAX_TOKEN_LENGTH: usize = 512;
/// Minimum minutes between scheduled replica runs.
pub const MIN_INTERVAL_MINUTES: u32 = 1;
/// Maximum minutes between scheduled replica runs (one week).
pub const MAX_INTERVAL_MINUTES: u32 = 10_080;

/// Direction of the replica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaDirection {
    /// This instance exports and `POST`s to the remote import.
    Push,
    /// This instance requests the remote export and imports it here.
    Pull,
}

/// Target of a replica push or pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaTarget {
    remote_url: Url,
    destination_id: RepositoryId,
    token: Option<String>,
    direction: ReplicaDirection,
}

/// Result of the last replica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaRun {
    occurred_at: String,
    packages_imported: u32,
    artifacts_imported: u32,
    skipped: u32,
    error: Option<String>,
}

/// Persisted policy for a repository. No target means no replica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaPolicy {
    target: Option<ReplicaTarget>,
    last_run: Option<ReplicaRun>,
    interval_minutes: Option<u32>,
}

/// Reasons why a replica policy is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReplicaPolicyError {
    /// The URL is neither `http` nor `https`, or it has no host.
    #[error("replica URL must be an absolute http or https URL")]
    InvalidUrl,

    /// The URL exceeds the allowed maximum.
    #[error("replica URL must be at most {max} characters")]
    UrlTooLong {
        /// Maximum allowed.
        max: usize,
    },

    /// The token exceeds the allowed maximum.
    #[error("replica token must be at most {max} characters")]
    TokenTooLong {
        /// Maximum allowed.
        max: usize,
    },

    /// The destination is the same repository.
    #[error("replica destination cannot be the source repository")]
    SameRepository,

    /// Direction is not `push` or `pull`.
    #[error("replica direction must be push or pull")]
    InvalidDirection,

    /// Interval is outside 1..=10080 (0 / missing disables the cron).
    #[error(
        "replica interval must be between {MIN_INTERVAL_MINUTES} and {MAX_INTERVAL_MINUTES} minutes, or 0 to disable"
    )]
    InvalidInterval,
}

impl ReplicaTarget {
    /// Builds a target from a URL, UUID, and optional token.
    ///
    /// # Errors
    ///
    /// [`ReplicaPolicyError`] if the URL is not HTTP(S) or the destination is
    /// the source itself.
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
            direction: ReplicaDirection::Push,
        })
    }

    /// URL of the remote instance (origin, without `/api`).
    #[must_use]
    pub fn remote_url(&self) -> &Url {
        &self.remote_url
    }

    /// Destination Forge on the remote instance.
    #[must_use]
    pub fn destination_id(&self) -> RepositoryId {
        self.destination_id
    }

    /// API token with write access on the destination. `None` if it has not
    /// been saved yet (or was deleted).
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Replaces the token. `None` keeps the previous one.
    #[must_use]
    pub fn with_token(mut self, token: Option<String>) -> Self {
        if token.is_some() {
            self.token = token;
        }
        self
    }

    /// Stored direction. Defaults to push.
    #[must_use]
    pub fn direction(&self) -> ReplicaDirection {
        self.direction
    }

    /// Replaces the direction.
    #[must_use]
    pub fn with_direction(mut self, direction: ReplicaDirection) -> Self {
        self.direction = direction;
        self
    }

    /// URLs of the import `POST` on the remote instance.
    ///
    /// First `{origin}/api/repositories/{id}/import` (compose / `FRONTEND_DIR`).
    /// Then `{origin}/repositories/{id}/import` (`cargo run` + Vite).
    #[must_use]
    pub fn import_urls(&self) -> [String; 2] {
        let origin = replica_origin(&self.remote_url);
        let dest = self.destination_id;
        [
            format!("{origin}/api/repositories/{dest}/import"),
            format!("{origin}/repositories/{dest}/import"),
        ]
    }

    /// URLs of the export `GET` on the remote instance.
    #[must_use]
    pub fn export_urls(&self) -> [String; 2] {
        let origin = replica_origin(&self.remote_url);
        let dest = self.destination_id;
        [
            format!("{origin}/api/repositories/{dest}/export"),
            format!("{origin}/repositories/{dest}/export"),
        ]
    }
}

impl ReplicaDirection {
    /// Persisted label (`push` / `pull`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Pull => "pull",
        }
    }

    /// Interprets the label. Empty = push.
    ///
    /// # Errors
    ///
    /// [`ReplicaPolicyError::InvalidDirection`] if it is neither `push` nor `pull`.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, ReplicaPolicyError> {
        match raw.as_ref().trim().to_ascii_lowercase().as_str() {
            "" | "push" => Ok(Self::Push),
            "pull" => Ok(Self::Pull),
            _ => Err(ReplicaPolicyError::InvalidDirection),
        }
    }
}

impl ReplicaRun {
    /// Builds the counts of a push.
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

    /// RFC 3339 timestamp.
    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    /// New coordinates on the destination.
    #[must_use]
    pub fn packages_imported(&self) -> u32 {
        self.packages_imported
    }

    /// New binaries on the destination.
    #[must_use]
    pub fn artifacts_imported(&self) -> u32 {
        self.artifacts_imported
    }

    /// Packages or binaries that were already present.
    #[must_use]
    pub fn skipped(&self) -> u32 {
        self.skipped
    }

    /// Remote error, if the replica failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `true` if the remote or the local import accepted the bundle.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

impl ReplicaPolicy {
    /// No target: this repository does not replicate.
    #[must_use]
    pub fn unconfigured() -> Self {
        Self {
            target: None,
            last_run: None,
            interval_minutes: None,
        }
    }

    /// Policy with a target and, optionally, the last run.
    #[must_use]
    pub fn new(target: Option<ReplicaTarget>, last_run: Option<ReplicaRun>) -> Self {
        Self {
            target,
            last_run,
            interval_minutes: None,
        }
    }

    /// Target, if configured.
    #[must_use]
    pub fn target(&self) -> Option<&ReplicaTarget> {
        self.target.as_ref()
    }

    /// Last run, if one has executed.
    #[must_use]
    pub fn last_run(&self) -> Option<&ReplicaRun> {
        self.last_run.as_ref()
    }

    /// Minutes between scheduled runs. `None` disables the cron.
    #[must_use]
    pub fn interval_minutes(&self) -> Option<u32> {
        self.interval_minutes
    }

    /// Replaces the last-run counts.
    #[must_use]
    pub fn with_last_run(mut self, last_run: ReplicaRun) -> Self {
        self.last_run = Some(last_run);
        self
    }

    /// Sets the cron interval. `None` or `0` disables it.
    ///
    /// # Errors
    ///
    /// [`ReplicaPolicyError::InvalidInterval`] if the value is outside 1..=10080.
    pub fn with_interval(mut self, minutes: Option<u32>) -> Result<Self, ReplicaPolicyError> {
        self.interval_minutes = normalize_interval(minutes)?;
        Ok(self)
    }

    /// `true` when a target, token, and interval are set and the interval elapsed.
    ///
    /// A policy that never ran is due immediately. A last-run timestamp that
    /// cannot be parsed is treated as due so the cron can retry.
    #[must_use]
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        let Some(target) = self.target.as_ref() else {
            return false;
        };
        if target.token().is_none() {
            return false;
        }
        let Some(minutes) = self.interval_minutes.filter(|minutes| *minutes > 0) else {
            return false;
        };
        match self.last_run.as_ref() {
            None => true,
            Some(run) => match DateTime::parse_from_rfc3339(run.occurred_at()) {
                Ok(last) => {
                    now >= last.with_timezone(&Utc) + TimeDelta::minutes(i64::from(minutes))
                }
                Err(_) => true,
            },
        }
    }
}

/// `None` or `0` disables the cron; 1..=10080 keeps it on.
///
/// # Errors
///
/// [`ReplicaPolicyError::InvalidInterval`] if the value is positive and out of range.
pub fn normalize_interval(minutes: Option<u32>) -> Result<Option<u32>, ReplicaPolicyError> {
    match minutes {
        None | Some(0) => Ok(None),
        Some(minutes) if (MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&minutes) => {
            Ok(Some(minutes))
        }
        Some(_) => Err(ReplicaPolicyError::InvalidInterval),
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
        assert_eq!(
            target.export_urls(),
            [
                format!("http://127.0.0.1:3000/api/repositories/{}/export", repo(2)),
                format!("http://127.0.0.1:3000/repositories/{}/export", repo(2)),
            ]
        );
        assert_eq!(
            ReplicaDirection::parse("pull").unwrap(),
            ReplicaDirection::Pull
        );
    }

    #[test]
    fn scheduled_policy_is_due_until_the_interval_elapses() {
        let target = ReplicaTarget::new(
            "http://127.0.0.1:3000",
            repo(2),
            Some("tok".into()),
            repo(1),
        )
        .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-26T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let idle = ReplicaPolicy::new(Some(target.clone()), None)
            .with_interval(Some(30))
            .unwrap();
        assert!(idle.is_due(now));

        let just_ran = idle.with_last_run(ReplicaRun::new("2026-09-26T11:45:00Z", 0, 0, 0, None));
        assert!(!just_ran.is_due(now));
        assert!(just_ran.is_due(now + TimeDelta::minutes(30)));

        let no_cron = ReplicaPolicy::new(Some(target), None);
        assert!(!no_cron.is_due(now));
        assert!(matches!(
            ReplicaPolicy::unconfigured().with_interval(Some(20_000)),
            Err(ReplicaPolicyError::InvalidInterval)
        ));
    }
}
