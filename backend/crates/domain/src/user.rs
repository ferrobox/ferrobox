use std::fmt;

use thiserror::Error;

use crate::ids::UserId;
use crate::oidc::OidcIdentity;

const MAX_USERNAME_LENGTH: usize = 64;
const MAX_EMAIL_LENGTH: usize = 254;
const MIN_PASSWORD_LENGTH: usize = 8;

/// Authorization role of a user in `FerroBox`.
///
/// Roles are deliberately few and ordered by privilege:
/// `Admin` manages accounts (create, delete, change role, reset
/// other people's passwords) and can write; `Developer` can publish and
/// create repositories; `Reader` only reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Full management: users, repositories, and publication.
    Admin,
    /// Can create repositories and publish artifacts; does not manage
    /// users.
    Developer,
    /// Read-only access to repositories and artifacts.
    Reader,
}

impl Role {
    /// Stable label used in persistence and in the HTTP API.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Developer => "developer",
            Self::Reader => "reader",
        }
    }

    /// Parses the stable label of a role.
    ///
    /// # Errors
    ///
    /// Returns [`RoleError::Unknown`] if the label is not known.
    pub fn parse(value: &str) -> Result<Self, RoleError> {
        match value {
            "admin" => Ok(Self::Admin),
            "developer" => Ok(Self::Developer),
            "reader" => Ok(Self::Reader),
            other => Err(RoleError::Unknown(other.to_string())),
        }
    }

    /// `true` if this role can manage users.
    #[must_use]
    pub fn can_manage_users(self) -> bool {
        matches!(self, Self::Admin)
    }

    /// `true` if this role can create repositories and publish
    /// artifacts.
    #[must_use]
    pub fn can_write_artifacts(self) -> bool {
        matches!(self, Self::Admin | Self::Developer)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Reasons why a string is not a valid [`Role`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RoleError {
    /// Unknown role label.
    #[error("unknown role: '{0}'")]
    Unknown(String),
}

/// Validated username: non-empty, with a bounded length, and
/// restricted to characters safe for identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Username(String);

/// Reasons why a string is not a valid [`Username`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsernameError {
    /// The username cannot be empty.
    #[error("username cannot be empty")]
    Empty,

    /// The username exceeds the maximum allowed length.
    #[error("username cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },

    /// The name contains a character outside the allowed alphabet.
    #[error(
        "username contains an invalid character: '{0}' \
         (only ASCII letters, digits, '-' and '_' are allowed)"
    )]
    InvalidCharacter(char),
}

impl Username {
    /// Validates and builds a username.
    ///
    /// # Errors
    ///
    /// Returns [`UsernameError`] if `username` is empty, exceeds
    /// `64` characters, or contains any character outside the
    /// allowed alphabet (ASCII letters and digits, `-` and `_`).
    pub fn parse(username: impl Into<String>) -> Result<Self, UsernameError> {
        let username = username.into();

        if username.is_empty() {
            return Err(UsernameError::Empty);
        }

        if username.len() > MAX_USERNAME_LENGTH {
            return Err(UsernameError::TooLong {
                max: MAX_USERNAME_LENGTH,
                actual: username.len(),
            });
        }

        if let Some(invalid) = username
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(UsernameError::InvalidCharacter(invalid));
        }

        Ok(Self(username))
    }

    /// Returns the username as a text string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for Username {
    type Error = UsernameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

/// Validated email address, normalized to ASCII lowercase.
///
/// The initial administrator created at startup may have no email
/// (`None` on [`User`]). Accounts created by an Admin do require one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

/// Reasons why a string is not a valid [`Email`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EmailError {
    /// The email cannot be empty.
    #[error("email cannot be empty")]
    Empty,

    /// The email exceeds the maximum allowed length.
    #[error("email cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },

    /// The email does not have the form `local@domain.tld`.
    #[error("email is not a valid address")]
    Invalid,
}

impl Email {
    /// Validates and normalizes an email address.
    ///
    /// # Errors
    ///
    /// Returns [`EmailError`] if it is empty, too long, or does not
    /// have the form of an address.
    pub fn parse(email: impl Into<String>) -> Result<Self, EmailError> {
        let email = email.into();
        let trimmed = email.trim();

        if trimmed.is_empty() {
            return Err(EmailError::Empty);
        }

        if trimmed.len() > MAX_EMAIL_LENGTH {
            return Err(EmailError::TooLong {
                max: MAX_EMAIL_LENGTH,
                actual: trimmed.len(),
            });
        }

        if trimmed.chars().any(char::is_whitespace) {
            return Err(EmailError::Invalid);
        }

        let normalized = trimmed.to_ascii_lowercase();
        let Some((local, domain)) = normalized.split_once('@') else {
            return Err(EmailError::Invalid);
        };

        if local.is_empty()
            || domain.is_empty()
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
            || domain.contains("..")
        {
            return Err(EmailError::Invalid);
        }

        Ok(Self(normalized))
    }

    /// Returns the email as a text string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Reasons why a password does not meet the instance
/// policy.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PasswordPolicyError {
    /// Fewer than eight characters, or missing a lowercase, uppercase, or
    /// digit.
    #[error(
        "password must be at least {min} characters and include a lowercase letter, \
         an uppercase letter, and a digit"
    )]
    TooWeak {
        /// Required minimum length.
        min: usize,
    },
}

/// Checks that `password` has at least 8 characters, a lowercase,
/// an uppercase, and a digit. Applies to creating accounts, resetting, and
/// changing one's own password; not to the initial
/// bootstrap administrator.
///
/// # Errors
///
/// Returns [`PasswordPolicyError::TooWeak`] if it does not meet the policy.
pub fn validate_password_policy(password: &str) -> Result<(), PasswordPolicyError> {
    let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());

    if password.len() >= MIN_PASSWORD_LENGTH && has_lower && has_upper && has_digit {
        Ok(())
    } else {
        Err(PasswordPolicyError::TooWeak {
            min: MIN_PASSWORD_LENGTH,
        })
    }
}

/// A `FerroBox` user: an authenticable identity with an authorization
/// role, which may own API tokens.
///
/// The password hash is not part of this entity: it is a
/// credential detail that lives in the application /
/// persistence layer, not a concept of the business model.
#[derive(Debug, Clone)]
pub struct User {
    id: UserId,
    username: Username,
    email: Option<Email>,
    role: Role,
    oidc: Option<OidcIdentity>,
    robot: bool,
}

impl User {
    /// Registers a new user, assigning it a new identifier.
    /// The email is left empty: use this for the bootstrap administrator
    /// or call [`User::with_email`].
    #[must_use]
    pub fn new(username: Username, role: Role) -> Self {
        Self {
            id: UserId::new(),
            username,
            email: None,
            role,
            oidc: None,
            robot: false,
        }
    }

    /// Reconstitutes an already existing user from a
    /// known identifier (for example, when loading it from
    /// persistence).
    #[must_use]
    pub fn from_parts(
        id: UserId,
        username: Username,
        role: Role,
        email: Option<Email>,
    ) -> Self {
        Self {
            id,
            username,
            email,
            role,
            oidc: None,
            robot: false,
        }
    }

    /// Unique identifier of this user.
    #[must_use]
    pub fn id(&self) -> UserId {
        self.id
    }

    /// Username.
    #[must_use]
    pub fn username(&self) -> &Username {
        &self.username
    }

    /// Email address, if the account has one.
    #[must_use]
    pub fn email(&self) -> Option<&Email> {
        self.email.as_ref()
    }

    /// Authorization role of this user.
    #[must_use]
    pub fn role(&self) -> Role {
        self.role
    }

    /// Returns this user with a different email. Identity does not
    /// change.
    #[must_use]
    pub fn with_email(self, email: Option<Email>) -> Self {
        Self { email, ..self }
    }

    /// Returns this user with a different role. Identity does not change.
    #[must_use]
    pub fn with_role(self, role: Role) -> Self {
        Self { role, ..self }
    }

    /// Federated identity, if the account has been linked to an IdP.
    #[must_use]
    pub fn oidc(&self) -> Option<&OidcIdentity> {
        self.oidc.as_ref()
    }

    /// `true` if the account is linked to an `OIDC` issuer.
    #[must_use]
    pub fn is_sso_linked(&self) -> bool {
        self.oidc.is_some()
    }

    /// Returns this user linked to an `OIDC` identity.
    #[must_use]
    pub fn with_oidc(self, identity: Option<OidcIdentity>) -> Self {
        Self {
            oidc: identity,
            ..self
        }
    }

    /// `true` if it is a robot account (CI): it does not sign in with a password or
    /// `SSO`, only with API tokens.
    #[must_use]
    pub fn is_robot(&self) -> bool {
        self.robot
    }

    /// Marks or unmarks the account as a robot. A robot cannot be
    /// [`Role::Admin`].
    #[must_use]
    pub fn with_robot(self, robot: bool) -> Self {
        Self { robot, ..self }
    }
}

impl PartialEq for User {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for User {}

impl std::hash::Hash for User {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_empty_username() {
        assert_eq!(Username::parse(""), Err(UsernameError::Empty));
    }

    #[test]
    fn rejects_an_invalid_character() {
        assert_eq!(
            Username::parse("ada lovelace"),
            Err(UsernameError::InvalidCharacter(' '))
        );
    }

    #[test]
    fn accepts_a_valid_username() {
        assert!(Username::parse("admin").is_ok());
    }

    #[test]
    fn equality_is_based_on_identity() {
        let id = UserId::new();
        let first = User::from_parts(id, Username::parse("admin").unwrap(), Role::Admin, None);
        let second = User::from_parts(id, Username::parse("other").unwrap(), Role::Reader, None);

        assert_eq!(first, second);
    }

    #[test]
    fn with_role_keeps_identity_and_changes_authorization() {
        let user = User::new(Username::parse("ada").unwrap(), Role::Reader);
        let updated = user.clone().with_role(Role::Admin);

        assert_eq!(user, updated);
        assert_eq!(user.role(), Role::Reader);
        assert_eq!(updated.role(), Role::Admin);
    }

    #[test]
    fn admin_can_manage_users_and_write() {
        assert!(Role::Admin.can_manage_users());
        assert!(Role::Admin.can_write_artifacts());
        assert!(!Role::Developer.can_manage_users());
        assert!(Role::Developer.can_write_artifacts());
        assert!(!Role::Reader.can_write_artifacts());
    }

    #[test]
    fn role_round_trips_through_label() {
        for role in [Role::Admin, Role::Developer, Role::Reader] {
            assert_eq!(Role::parse(role.as_str()).unwrap(), role);
        }
    }

    #[test]
    fn email_is_normalized_to_lowercase() {
        let email = Email::parse("Ada@Example.COM").unwrap();
        assert_eq!(email.as_str(), "ada@example.com");
    }

    #[test]
    fn rejects_an_invalid_email() {
        assert_eq!(Email::parse(""), Err(EmailError::Empty));
        assert_eq!(Email::parse("not-an-email"), Err(EmailError::Invalid));
        assert_eq!(Email::parse("ada@localhost"), Err(EmailError::Invalid));
    }

    #[test]
    fn password_policy_requires_length_and_character_classes() {
        assert!(validate_password_policy("Secret1a").is_ok());
        assert_eq!(
            validate_password_policy("secret"),
            Err(PasswordPolicyError::TooWeak { min: 8 })
        );
        assert_eq!(
            validate_password_policy("alllowercase1"),
            Err(PasswordPolicyError::TooWeak { min: 8 })
        );
        assert_eq!(
            validate_password_policy("ALLUPPERCASE1"),
            Err(PasswordPolicyError::TooWeak { min: 8 })
        );
        assert_eq!(
            validate_password_policy("NoDigitsHere"),
            Err(PasswordPolicyError::TooWeak { min: 8 })
        );
    }
}
