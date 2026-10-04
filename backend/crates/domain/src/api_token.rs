use std::fmt;

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::ids::{ApiTokenId, RepositoryId, UserId};

const MAX_NAME_LENGTH: usize = 100;

/// Validated descriptive name of an API token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApiTokenName(String);

/// Reasons why a string is not a valid [`ApiTokenName`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApiTokenNameError {
    /// The name cannot be empty.
    #[error("API token name cannot be empty")]
    Empty,

    /// The name exceeds the maximum allowed length.
    #[error("API token name cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },
}

impl ApiTokenName {
    /// Validates and builds an API token name.
    ///
    /// # Errors
    ///
    /// Returns [`ApiTokenNameError`] if `name` is empty or exceeds
    /// `100` characters.
    pub fn parse(name: impl Into<String>) -> Result<Self, ApiTokenNameError> {
        let name = name.into();

        if name.is_empty() {
            return Err(ApiTokenNameError::Empty);
        }

        if name.len() > MAX_NAME_LENGTH {
            return Err(ApiTokenNameError::TooLong {
                max: MAX_NAME_LENGTH,
                actual: name.len(),
            });
        }

        Ok(Self(name))
    }

    /// Returns the name as a text string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ApiTokenName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Action a token may be limited to. Empty scopes mean unrestricted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenScope {
    /// List, download, export, and other reads.
    Read,
    /// Publish, delete, prefetch, replica, and other writes. Implies read.
    Write,
}

/// Reasons why a scope is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenScopeError {
    /// Unknown label.
    #[error("token scope must be read or write")]
    Unknown,
}

impl TokenScope {
    /// Persist / API label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    /// Parse one label.
    ///
    /// # Errors
    ///
    /// [`TokenScopeError::Unknown`] if the label is not `read` or `write`.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, TokenScopeError> {
        match raw.as_ref().trim().to_ascii_lowercase().as_str() {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            _ => Err(TokenScopeError::Unknown),
        }
    }
}

/// Repositories a token may touch. Empty = every repository the user can
/// already access.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenRepositories(Vec<RepositoryId>);

/// Reasons why a stored repository list is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenRepositoryError {
    /// A stored id is not a UUID.
    #[error("repository id must be a UUID")]
    Invalid,
}

impl TokenRepositories {
    /// No repository limit beyond the user's role and groups.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self(Vec::new())
    }

    /// Builds a list, dropping duplicates and keeping the first occurrence.
    #[must_use]
    pub fn from_ids(ids: impl IntoIterator<Item = RepositoryId>) -> Self {
        let mut repositories = Vec::new();
        for id in ids {
            if !repositories.contains(&id) {
                repositories.push(id);
            }
        }
        Self(repositories)
    }

    /// Parses the stored comma-separated form. Empty is unrestricted.
    ///
    /// # Errors
    ///
    /// [`TokenRepositoryError::Invalid`] if any id is not a UUID.
    pub fn parse_stored(raw: &str) -> Result<Self, TokenRepositoryError> {
        if raw.is_empty() {
            return Ok(Self::unrestricted());
        }
        let mut ids = Vec::new();
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let id = uuid::Uuid::parse_str(part)
                .map(RepositoryId::from)
                .map_err(|_| TokenRepositoryError::Invalid)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(Self(ids))
    }

    /// Stored form: empty, or comma-separated repository ids.
    #[must_use]
    pub fn as_stored(&self) -> String {
        self.0
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Repository ids. Empty means unrestricted.
    #[must_use]
    pub fn as_ids(&self) -> &[RepositoryId] {
        &self.0
    }

    /// `true` when no repositories were set.
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.0.is_empty()
    }

    /// `true` for an unrestricted token or one that lists `id`.
    #[must_use]
    pub fn allows(&self, id: RepositoryId) -> bool {
        self.is_unrestricted() || self.0.contains(&id)
    }
}

/// Set of scopes on a token. Empty = the token inherits the user's full role.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenScopes(Vec<TokenScope>);

impl TokenScopes {
    /// No restriction beyond the user's role.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self(Vec::new())
    }

    /// Parse labels. Duplicates are dropped. Empty input is unrestricted.
    ///
    /// # Errors
    ///
    /// [`TokenScopeError::Unknown`] if any label is not `read` or `write`.
    pub fn parse<I, S>(labels: I) -> Result<Self, TokenScopeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut scopes = Vec::new();
        for label in labels {
            let scope = TokenScope::parse(label)?;
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        Ok(Self(scopes))
    }

    /// Stored form: empty, `read`, `write`, or `read,write`.
    #[must_use]
    pub fn as_stored(&self) -> String {
        self.0
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Labels for the API.
    #[must_use]
    pub fn as_labels(&self) -> Vec<&'static str> {
        self.0.iter().map(|scope| scope.as_str()).collect()
    }

    /// `true` when no scopes were set.
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.0.is_empty()
    }

    /// `true` for unrestricted tokens or those with `read` or `write`.
    #[must_use]
    pub fn allows_read(&self) -> bool {
        self.is_unrestricted() || self.0.contains(&TokenScope::Read) || self.allows_write()
    }

    /// `true` for unrestricted tokens or those with `write`.
    #[must_use]
    pub fn allows_write(&self) -> bool {
        self.is_unrestricted() || self.0.contains(&TokenScope::Write)
    }
}

/// An API token issued to a user.
///
/// The plaintext secret is shown only once at creation; in
/// persistence only its hash is stored. This entity keeps a
/// non-sensitive `prefix` so the user can recognize the token
/// in later listings.
#[derive(Debug, Clone)]
pub struct ApiToken {
    id: ApiTokenId,
    user_id: UserId,
    name: ApiTokenName,
    prefix: String,
    expires_at: Option<DateTime<Utc>>,
    scopes: TokenScopes,
    repositories: TokenRepositories,
}

impl ApiToken {
    /// Issues a new token, assigning it a new identifier.
    #[must_use]
    pub fn new(user_id: UserId, name: ApiTokenName, prefix: String) -> Self {
        Self {
            id: ApiTokenId::new(),
            user_id,
            name,
            prefix,
            expires_at: None,
            scopes: TokenScopes::unrestricted(),
            repositories: TokenRepositories::unrestricted(),
        }
    }

    /// Reconstitutes an already existing token from a known
    /// identifier (for example, when loading it from persistence).
    #[must_use]
    pub fn from_parts(id: ApiTokenId, user_id: UserId, name: ApiTokenName, prefix: String) -> Self {
        Self {
            id,
            user_id,
            name,
            prefix,
            expires_at: None,
            scopes: TokenScopes::unrestricted(),
            repositories: TokenRepositories::unrestricted(),
        }
    }

    /// Unique identifier of this token.
    #[must_use]
    pub fn id(&self) -> ApiTokenId {
        self.id
    }

    /// Identifier of the owning user.
    #[must_use]
    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    /// Descriptive name of the token.
    #[must_use]
    pub fn name(&self) -> &ApiTokenName {
        &self.name
    }

    /// Non-sensitive prefix of the secret, useful for recognizing it in
    /// listings.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Instant at which the token stops being valid, if it has
    /// an expiry.
    #[must_use]
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }

    /// `true` if `now` is after or equal to [`Self::expires_at`].
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|at| now >= at)
    }

    /// Returns this token with a different expiry.
    #[must_use]
    pub fn with_expires_at(self, expires_at: Option<DateTime<Utc>>) -> Self {
        Self { expires_at, ..self }
    }

    /// Scopes stored on this token. Empty is unrestricted.
    #[must_use]
    pub fn scopes(&self) -> &TokenScopes {
        &self.scopes
    }

    /// Replace the scopes.
    #[must_use]
    pub fn with_scopes(self, scopes: TokenScopes) -> Self {
        Self { scopes, ..self }
    }

    /// Repositories this token may touch. Empty is unrestricted.
    #[must_use]
    pub fn repositories(&self) -> &TokenRepositories {
        &self.repositories
    }

    /// Replace the repository list.
    #[must_use]
    pub fn with_repositories(self, repositories: TokenRepositories) -> Self {
        Self {
            repositories,
            ..self
        }
    }
}

impl PartialEq for ApiToken {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for ApiToken {}

impl std::hash::Hash for ApiToken {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_empty_name() {
        assert_eq!(ApiTokenName::parse(""), Err(ApiTokenNameError::Empty));
    }

    #[test]
    fn accepts_a_valid_name() {
        assert!(ApiTokenName::parse("cargo-publish").is_ok());
    }

    #[test]
    fn a_token_without_expiry_is_never_expired() {
        let token = ApiToken::new(
            UserId::new(),
            ApiTokenName::parse("ci").unwrap(),
            "fb_ab".into(),
        );
        assert!(!token.is_expired(Utc::now()));
    }

    #[test]
    fn a_token_is_expired_at_or_after_its_deadline() {
        let deadline = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let token = ApiToken::new(
            UserId::new(),
            ApiTokenName::parse("ci").unwrap(),
            "fb_ab".into(),
        )
        .with_expires_at(Some(deadline));
        assert!(token.is_expired(deadline));
        assert!(!token.is_expired(deadline - chrono::TimeDelta::seconds(1)));
    }

    #[test]
    fn empty_scopes_are_unrestricted() {
        let scopes = TokenScopes::parse(Vec::<&str>::new()).unwrap();
        assert!(scopes.is_unrestricted());
        assert!(scopes.allows_read());
        assert!(scopes.allows_write());
        assert_eq!(scopes.as_stored(), "");
        assert!(scopes.as_labels().is_empty());
    }

    #[test]
    fn read_scope_forbids_write() {
        let scopes = TokenScopes::parse(["read"]).unwrap();
        assert!(!scopes.is_unrestricted());
        assert!(scopes.allows_read());
        assert!(!scopes.allows_write());
        assert_eq!(scopes.as_stored(), "read");
    }

    #[test]
    fn write_scope_implies_read() {
        let scopes = TokenScopes::parse(["write"]).unwrap();
        assert!(scopes.allows_read());
        assert!(scopes.allows_write());
        assert_eq!(scopes.as_stored(), "write");
    }

    #[test]
    fn parse_drops_duplicates_and_rejects_unknown_labels() {
        let scopes = TokenScopes::parse(["read", "READ", "write"]).unwrap();
        assert_eq!(scopes.as_labels(), vec!["read", "write"]);
        assert_eq!(TokenScope::parse("admin"), Err(TokenScopeError::Unknown));
        assert_eq!(
            TokenScopes::parse(["read", "admin"]),
            Err(TokenScopeError::Unknown)
        );
    }

    #[test]
    fn empty_repositories_allow_every_repository() {
        let repositories = TokenRepositories::unrestricted();
        let id = RepositoryId::new();
        assert!(repositories.is_unrestricted());
        assert!(repositories.allows(id));
        assert_eq!(repositories.as_stored(), "");
        assert!(repositories.as_ids().is_empty());
    }

    #[test]
    fn listed_repositories_are_an_allow_list() {
        let allowed = RepositoryId::new();
        let other = RepositoryId::new();
        let repositories = TokenRepositories::from_ids([allowed, allowed, other]);
        assert!(!repositories.is_unrestricted());
        assert!(repositories.allows(allowed));
        assert!(repositories.allows(other));
        assert!(!repositories.allows(RepositoryId::new()));
        assert_eq!(repositories.as_ids().len(), 2);

        let parsed = TokenRepositories::parse_stored(&repositories.as_stored()).unwrap();
        assert_eq!(parsed, repositories);
        assert_eq!(
            TokenRepositories::parse_stored("not-a-uuid"),
            Err(TokenRepositoryError::Invalid)
        );
    }

    #[test]
    fn token_carries_scopes() {
        let token = ApiToken::new(
            UserId::new(),
            ApiTokenName::parse("ci").unwrap(),
            "fb_ab".into(),
        )
        .with_scopes(TokenScopes::parse(["read"]).unwrap());
        assert!(token.scopes().allows_read());
        assert!(!token.scopes().allows_write());
    }
}
