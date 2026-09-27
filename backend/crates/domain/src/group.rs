use std::fmt;

use thiserror::Error;

use crate::ids::GroupId;
use crate::user::Role;

const MAX_GROUP_NAME_LENGTH: usize = 64;

/// Effective access of a user to a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepositoryAccess {
    /// Can list and download.
    Read,
    /// Can publish, delete, and configure the repository.
    Write,
}

impl RepositoryAccess {
    /// Converts the role assigned to a group into repository access.
    /// `Admin` is not used at group level: it is an instance role.
    #[must_use]
    pub fn from_group_role(role: Role) -> Option<Self> {
        match role {
            Role::Reader => Some(Self::Read),
            Role::Developer => Some(Self::Write),
            Role::Admin => None,
        }
    }

    /// Stable label used in the HTTP API.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    /// `true` if it includes write access.
    #[must_use]
    pub fn can_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// Validated group name: non-empty, with a bounded length, and
/// restricted to characters safe for identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupName(String);

/// Reasons why a string is not a valid [`GroupName`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GroupNameError {
    /// The name cannot be empty.
    #[error("group name cannot be empty")]
    Empty,

    /// The name exceeds the maximum allowed length.
    #[error("group name cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },

    /// The name contains a character outside the allowed alphabet.
    #[error(
        "group name contains an invalid character: '{0}' \
         (only ASCII letters, digits, '-' and '_' are allowed)"
    )]
    InvalidCharacter(char),
}

impl GroupName {
    /// Validates and builds a group name.
    ///
    /// # Errors
    ///
    /// Returns [`GroupNameError`] if `name` is empty, exceeds `64`
    /// characters, or contains a character outside the allowed alphabet.
    pub fn parse(name: impl Into<String>) -> Result<Self, GroupNameError> {
        let name = name.into();

        if name.is_empty() {
            return Err(GroupNameError::Empty);
        }

        if name.len() > MAX_GROUP_NAME_LENGTH {
            return Err(GroupNameError::TooLong {
                max: MAX_GROUP_NAME_LENGTH,
                actual: name.len(),
            });
        }

        if let Some(invalid) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(GroupNameError::InvalidCharacter(invalid));
        }

        Ok(Self(name))
    }

    /// Returns the name as a text string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GroupName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A group of users to which an Admin assigns repositories.
///
/// Members and repository access do not live on this entity:
/// they are associations persisted by the group store.
#[derive(Debug, Clone)]
pub struct Group {
    id: GroupId,
    name: GroupName,
}

impl Group {
    /// Creates a new group.
    #[must_use]
    pub fn new(name: GroupName) -> Self {
        Self {
            id: GroupId::new(),
            name,
        }
    }

    /// Reconstitutes an already persisted group.
    #[must_use]
    pub fn from_parts(id: GroupId, name: GroupName) -> Self {
        Self { id, name }
    }

    /// Unique identifier.
    #[must_use]
    pub fn id(&self) -> GroupId {
        self.id
    }

    /// Group name.
    #[must_use]
    pub fn name(&self) -> &GroupName {
        &self.name
    }
}

impl PartialEq for Group {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Group {}

impl std::hash::Hash for Group {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_empty_group_name() {
        assert_eq!(GroupName::parse(""), Err(GroupNameError::Empty));
    }

    #[test]
    fn group_role_reader_is_read_and_developer_is_write() {
        assert_eq!(
            RepositoryAccess::from_group_role(Role::Reader),
            Some(RepositoryAccess::Read)
        );
        assert_eq!(
            RepositoryAccess::from_group_role(Role::Developer),
            Some(RepositoryAccess::Write)
        );
        assert_eq!(RepositoryAccess::from_group_role(Role::Admin), None);
        assert!(RepositoryAccess::Read < RepositoryAccess::Write);
    }
}
