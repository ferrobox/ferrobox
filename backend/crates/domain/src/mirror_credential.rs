//! Username and secret a `Mirror` uses when it calls its upstream.
//!
//! The secret is recoverable: FerroBox has to send it. It is never
//! returned by the HTTP API after it is saved.

use thiserror::Error;

const MAX_USERNAME_LENGTH: usize = 200;
const MAX_SECRET_LENGTH: usize = 4096;

/// Credentials for one mirror's upstream.
#[derive(Clone, PartialEq, Eq)]
pub struct MirrorCredential {
    username: String,
    secret: String,
}

/// Reasons why a username and secret are not a valid credential.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MirrorCredentialError {
    /// The secret is empty.
    #[error("upstream secret cannot be empty")]
    EmptySecret,

    /// The username is too long.
    #[error("upstream username cannot exceed {max} characters, got {actual}")]
    UsernameTooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },

    /// The secret is too long.
    #[error("upstream secret cannot exceed {max} characters, got {actual}")]
    SecretTooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },

    /// The username contains `:`, which would split HTTP Basic auth.
    #[error("upstream username cannot contain ':'")]
    UsernameColon,

    /// A header value cannot contain a line break.
    #[error("upstream credentials cannot contain a line break")]
    LineBreak,
}

impl MirrorCredential {
    /// Validates a username and secret.
    ///
    /// An empty username means the secret is sent as a bearer token.
    /// A username means HTTP Basic (`username:secret`).
    ///
    /// # Errors
    ///
    /// [`MirrorCredentialError`] if the secret is empty, a field is too
    /// long, the username contains `:`, or either field contains a line break.
    pub fn parse(
        username: impl Into<String>,
        secret: impl Into<String>,
    ) -> Result<Self, MirrorCredentialError> {
        let username = username.into().trim().to_string();
        let secret = secret.into().trim().to_string();
        if username.len() > MAX_USERNAME_LENGTH {
            return Err(MirrorCredentialError::UsernameTooLong {
                max: MAX_USERNAME_LENGTH,
                actual: username.len(),
            });
        }
        if secret.is_empty() {
            return Err(MirrorCredentialError::EmptySecret);
        }
        if secret.len() > MAX_SECRET_LENGTH {
            return Err(MirrorCredentialError::SecretTooLong {
                max: MAX_SECRET_LENGTH,
                actual: secret.len(),
            });
        }
        if username.contains(':') {
            return Err(MirrorCredentialError::UsernameColon);
        }
        if username.contains(['\n', '\r']) || secret.contains(['\n', '\r']) {
            return Err(MirrorCredentialError::LineBreak);
        }
        Ok(Self { username, secret })
    }

    /// Username. Empty means bearer-token authentication.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Secret sent to the upstream. Not for API responses.
    #[must_use]
    pub fn secret(&self) -> &str {
        &self.secret
    }
}

impl std::fmt::Debug for MirrorCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MirrorCredential")
            .field("username", &self.username)
            .field("secret", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_credential_keeps_an_empty_username() {
        let credential = MirrorCredential::parse("", "token-1").unwrap();
        assert_eq!(credential.username(), "");
        assert_eq!(credential.secret(), "token-1");
    }

    #[test]
    fn rejects_an_empty_secret_a_colon_and_a_line_break() {
        assert_eq!(
            MirrorCredential::parse("ci", "  "),
            Err(MirrorCredentialError::EmptySecret)
        );
        assert_eq!(
            MirrorCredential::parse("ci:bot", "secret"),
            Err(MirrorCredentialError::UsernameColon)
        );
        assert_eq!(
            MirrorCredential::parse("ci", "one\ntwo"),
            Err(MirrorCredentialError::LineBreak)
        );
    }
}
