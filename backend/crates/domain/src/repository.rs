use std::fmt;

use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;
use url::Url;

use crate::ids::RepositoryId;
use crate::package_coordinate::PackageEcosystem;

const MAX_NAME_LENGTH: usize = 100;

/// Validated repository name: non-empty, with a bounded length, and
/// restricted to characters safe to appear in a URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepositoryName(String);

/// Reasons why a string is not a valid [`RepositoryName`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RepositoryNameError {
    /// The name cannot be empty.
    #[error("repository name cannot be empty")]
    Empty,

    /// The name exceeds the maximum allowed length.
    #[error("repository name cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },

    /// The name contains a character outside the allowed alphabet.
    #[error(
        "repository name contains an invalid character: '{0}' \
         (only ASCII letters, digits, '-' and '_' are allowed)"
    )]
    InvalidCharacter(char),
}

impl RepositoryName {
    /// Validates and builds a repository name.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryNameError`] if `name` is empty, exceeds
    /// `100` characters, or contains any character outside the
    /// allowed alphabet (ASCII letters and digits, `-` and `_`).
    pub fn parse(name: impl Into<String>) -> Result<Self, RepositoryNameError> {
        let name = name.into();

        if name.is_empty() {
            return Err(RepositoryNameError::Empty);
        }

        if name.len() > MAX_NAME_LENGTH {
            return Err(RepositoryNameError::TooLong {
                max: MAX_NAME_LENGTH,
                actual: name.len(),
            });
        }

        if let Some(invalid) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(RepositoryNameError::InvalidCharacter(invalid));
        }

        Ok(Self(name))
    }

    /// Returns the name as a text string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for RepositoryName {
    type Error = RepositoryNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

/// The origin and storage strategy of a repository.
///
/// Unlike a C-style enumeration (a simple list of
/// labels), each variant of this type carries its own distinct
/// data -- it is an *algebraic data type*: the compiler knows,
/// for each variant, exactly which fields exist, and requires every
/// variant to be handled explicitly in any `match` on this
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryKind {
    /// Owned storage: `FerroBox` is the source of truth of the
    /// content published directly here.
    Forge,

    /// Cached replica of an external source. The first request for a
    /// missing artifact is resolved against `upstream`; later
    /// requests are served from the copy already stored locally.
    Mirror {
        /// Base URL of the external repository being replicated.
        upstream: Url,
    },

    /// Single access point that aggregates several repositories (`Forge`
    /// and/or `Mirror`) under one URL, resolving internally against
    /// which of them to respond.
    Alloy {
        /// Aggregated repositories, in the order they are consulted.
        members: Vec<RepositoryId>,
    },
}

impl RepositoryKind {
    /// Brief one-word description, useful for logs and debugging.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Forge => "forge",
            Self::Mirror { .. } => "mirror",
            Self::Alloy { .. } => "alloy",
        }
    }
}

/// A repository of artifacts: groups an origin strategy
/// (`RepositoryKind`) and a package ecosystem (`PackageEcosystem`)
/// under a unique name.
///
/// `RepositoryKind` and `PackageEcosystem` are deliberately two
/// independent fields, not a single hierarchy: the first answers "where
/// does the content come from and how is it stored?" (owned storage,
/// cached replica, or aggregation of other repositories), while the
/// second answers "what package format does it contain?" (Cargo,
/// npm, generic...). Both questions are orthogonal -- a `Mirror`
/// repository can replicate a Cargo registry as well as an npm one, and
/// a Cargo `Forge` behaves, as far as storage is concerned, the same
/// as a generic `Forge`. `FerroBox` models this same distinction with
/// two independent axes (repository kind and package format) and its
/// own vocabulary.
#[derive(Debug, Clone)]
pub struct Repository {
    id: RepositoryId,
    name: RepositoryName,
    kind: RepositoryKind,
    ecosystem: PackageEcosystem,
    prefetch_interval_hours: Option<u32>,
    last_prefetch_at: Option<DateTime<Utc>>,
}

/// Reasons why a combination of name and kind does not form a
/// valid [`Repository`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RepositoryError {
    /// An `Alloy` repository with no members aggregates nothing -- it is an
    /// empty promise, so it is rejected at construction.
    #[error("an Alloy repository must aggregate at least one member repository")]
    EmptyAlloy,
}

impl Repository {
    /// Registers a new repository, assigning it a new identifier.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError`] if `kind` is an `Alloy` with no members.
    pub fn new(
        name: RepositoryName,
        kind: RepositoryKind,
        ecosystem: PackageEcosystem,
    ) -> Result<Self, RepositoryError> {
        Self::validate(&kind)?;
        Ok(Self {
            id: RepositoryId::new(),
            name,
            kind,
            ecosystem,
            prefetch_interval_hours: None,
            last_prefetch_at: None,
        })
    }

    /// Reconstitutes an already existing repository from a
    /// known identifier (for example, when loading it from
    /// persistence).
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError`] if `kind` is an `Alloy` with no
    /// members -- even when reconstituting, we do not blindly trust
    /// that persisted data satisfies the invariant.
    pub fn from_parts(
        id: RepositoryId,
        name: RepositoryName,
        kind: RepositoryKind,
        ecosystem: PackageEcosystem,
    ) -> Result<Self, RepositoryError> {
        Self::validate(&kind)?;
        Ok(Self {
            id,
            name,
            kind,
            ecosystem,
            prefetch_interval_hours: None,
            last_prefetch_at: None,
        })
    }

    fn validate(kind: &RepositoryKind) -> Result<(), RepositoryError> {
        if let RepositoryKind::Alloy { members } = kind
            && members.is_empty()
        {
            return Err(RepositoryError::EmptyAlloy);
        }
        Ok(())
    }

    /// Unique identifier of this repository.
    #[must_use]
    pub fn id(&self) -> RepositoryId {
        self.id
    }

    /// Repository name.
    #[must_use]
    pub fn name(&self) -> &RepositoryName {
        &self.name
    }

    /// Origin and storage strategy of this repository.
    #[must_use]
    pub fn kind(&self) -> &RepositoryKind {
        &self.kind
    }

    /// Package ecosystem this repository indexes.
    #[must_use]
    pub fn ecosystem(&self) -> PackageEcosystem {
        self.ecosystem
    }

    /// Replaces the origin strategy, keeping identity, name,
    /// and ecosystem. Used to update the members of an `Alloy`.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError`] if `kind` is an `Alloy` with no members.
    pub fn with_kind(self, kind: RepositoryKind) -> Result<Self, RepositoryError> {
        Self::validate(&kind)?;
        Ok(Self { kind, ..self })
    }

    /// Hours between scheduled *upstream* refreshes. `None` = there is no
    /// cron.
    #[must_use]
    pub fn prefetch_interval_hours(&self) -> Option<u32> {
        self.prefetch_interval_hours
    }

    /// Last scheduled refresh, if it has already run once.
    #[must_use]
    pub fn last_prefetch_at(&self) -> Option<DateTime<Utc>> {
        self.last_prefetch_at
    }

    /// Interval and last-refresh mark. Only has effect on a
    /// [`RepositoryKind::Mirror`].
    #[must_use]
    pub fn with_prefetch_schedule(
        self,
        interval_hours: Option<u32>,
        last_prefetch_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            prefetch_interval_hours: interval_hours,
            last_prefetch_at,
            ..self
        }
    }

    /// `true` if it is a Mirror with an interval and it is time to refresh.
    #[must_use]
    pub fn prefetch_is_due(&self, now: DateTime<Utc>) -> bool {
        if !matches!(self.kind, RepositoryKind::Mirror { .. }) {
            return false;
        }
        let Some(hours) = self.prefetch_interval_hours.filter(|hours| *hours > 0) else {
            return false;
        };
        match self.last_prefetch_at {
            None => true,
            Some(last) => now >= last + TimeDelta::hours(i64::from(hours)),
        }
    }
}

impl PartialEq for Repository {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Repository {}

impl std::hash::Hash for Repository {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;

    #[test]
    fn rejects_an_empty_name() {
        assert_eq!(RepositoryName::parse(""), Err(RepositoryNameError::Empty));
    }

    #[test]
    fn rejects_an_invalid_character() {
        assert_eq!(
            RepositoryName::parse("my repo"),
            Err(RepositoryNameError::InvalidCharacter(' '))
        );
    }

    #[test]
    fn accepts_a_valid_name() {
        assert!(RepositoryName::parse("cargo-releases").is_ok());
    }

    #[test]
    fn a_forge_repository_can_be_created() {
        let name = RepositoryName::parse("cargo-releases").unwrap();
        assert!(Repository::new(name, RepositoryKind::Forge, PackageEcosystem::Cargo).is_ok());
    }

    #[test]
    fn an_alloy_without_members_is_rejected() {
        let name = RepositoryName::parse("public-cargo").unwrap();
        let result = Repository::new(
            name,
            RepositoryKind::Alloy { members: vec![] },
            PackageEcosystem::Cargo,
        );

        assert_eq!(result, Err(RepositoryError::EmptyAlloy));
    }

    #[test]
    fn an_alloy_with_at_least_one_member_is_accepted() {
        let name = RepositoryName::parse("public-cargo").unwrap();
        let kind = RepositoryKind::Alloy {
            members: vec![RepositoryId::new()],
        };

        assert!(Repository::new(name, kind, PackageEcosystem::Cargo).is_ok());
    }

    #[test]
    fn equality_is_based_on_identity_not_on_name_or_kind() {
        let id = RepositoryId::new();
        let original = Repository::from_parts(
            id,
            RepositoryName::parse("cargo-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let renamed = Repository::from_parts(
            id,
            RepositoryName::parse("cargo-releases-v2").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();

        assert_eq!(original, renamed);
    }

    #[test]
    fn exposes_the_package_ecosystem_it_indexes() {
        let name = RepositoryName::parse("npm-releases").unwrap();
        let repository =
            Repository::new(name, RepositoryKind::Forge, PackageEcosystem::Npm).unwrap();

        assert_eq!(repository.ecosystem(), PackageEcosystem::Npm);
    }

    #[test]
    fn with_kind_replaces_alloy_members_and_keeps_identity() {
        let first = RepositoryId::new();
        let second = RepositoryId::new();
        let original = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let id = original.id();

        let updated = original
            .with_kind(RepositoryKind::Alloy {
                members: vec![second],
            })
            .unwrap();

        assert_eq!(updated.id(), id);
        assert_eq!(updated.name().as_str(), "crates-alloy");
        match updated.kind() {
            RepositoryKind::Alloy { members } => assert_eq!(members, &vec![second]),
            other => panic!("expected alloy, got {other:?}"),
        }
    }

    #[test]
    fn with_kind_rejects_an_empty_alloy() {
        let original = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![RepositoryId::new()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();

        let result = original.with_kind(RepositoryKind::Alloy { members: vec![] });
        assert_eq!(result.err(), Some(RepositoryError::EmptyAlloy));
    }

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

    #[test]
    fn prefetch_is_due_when_a_mirror_has_never_run() {
        let now = DateTime::parse_from_rfc3339("2026-09-26T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let repository = cargo_mirror().with_prefetch_schedule(Some(1), None);

        assert!(repository.prefetch_is_due(now));
    }

    #[test]
    fn prefetch_is_due_after_the_interval_elapses() {
        let last = DateTime::parse_from_rfc3339("2026-09-26T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = DateTime::parse_from_rfc3339("2026-09-26T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let repository = cargo_mirror().with_prefetch_schedule(Some(1), Some(last));

        assert!(repository.prefetch_is_due(now));
    }

    #[test]
    fn prefetch_is_not_due_before_the_interval() {
        let last = DateTime::parse_from_rfc3339("2026-09-26T09:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = DateTime::parse_from_rfc3339("2026-09-26T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let repository = cargo_mirror().with_prefetch_schedule(Some(1), Some(last));

        assert!(!repository.prefetch_is_due(now));
    }

    #[test]
    fn prefetch_is_not_due_without_an_interval() {
        let now = DateTime::parse_from_rfc3339("2026-09-26T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let forge = Repository::new(
            RepositoryName::parse("local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap()
        .with_prefetch_schedule(Some(1), None);
        let disabled = cargo_mirror().with_prefetch_schedule(Some(0), None);

        assert!(!forge.prefetch_is_due(now));
        assert!(!disabled.prefetch_is_due(now));
        assert!(!cargo_mirror().prefetch_is_due(now));
    }
}
