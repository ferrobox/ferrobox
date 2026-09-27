use std::fmt;

use thiserror::Error;

const SHA256_HEX_LENGTH: usize = 64;

/// A validated SHA-256 checksum: always exactly 64
/// lowercase hexadecimal characters.
///
/// The only way to obtain an instance is through [`Self::parse`],
/// which guarantees the invariant at construction time -- once
/// a `Sha256Checksum` exists, the rest of the system can trust that
/// it is valid without checking it again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Checksum(String);

/// Reasons why a string is not a valid [`Sha256Checksum`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChecksumError {
    /// The text does not have the exact expected length.
    #[error("checksum must be exactly {expected} hexadecimal characters, got {actual}")]
    InvalidLength {
        /// Expected length, in characters.
        expected: usize,
        /// Actual length received, in characters.
        actual: usize,
    },

    /// The text contains a character outside the hexadecimal alphabet.
    #[error("checksum contains a non-hexadecimal character: '{0}'")]
    InvalidCharacter(char),
}

impl Sha256Checksum {
    /// Validates and builds a checksum from its hexadecimal
    /// representation, normalizing any uppercase letter to lowercase.
    ///
    /// # Errors
    ///
    /// Returns [`ChecksumError`] if `hex` does not have exactly 64
    /// characters, or if it contains any character outside the
    /// hexadecimal alphabet.
    pub fn parse(hex: impl Into<String>) -> Result<Self, ChecksumError> {
        let hex = hex.into();

        if hex.len() != SHA256_HEX_LENGTH {
            return Err(ChecksumError::InvalidLength {
                expected: SHA256_HEX_LENGTH,
                actual: hex.len(),
            });
        }

        if let Some(invalid) = hex.chars().find(|c| !c.is_ascii_hexdigit()) {
            return Err(ChecksumError::InvalidCharacter(invalid));
        }

        Ok(Self(hex.to_ascii_lowercase()))
    }

    /// Returns the hexadecimal representation in lowercase.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for Sha256Checksum {
    type Error = ChecksumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_hex() -> String {
        "a".repeat(SHA256_HEX_LENGTH)
    }

    #[test]
    fn accepts_a_valid_lowercase_hex_string() {
        let checksum = Sha256Checksum::parse(valid_hex()).unwrap();
        assert_eq!(checksum.as_str(), valid_hex());
    }

    #[test]
    fn normalizes_uppercase_input_to_lowercase() {
        let uppercase = "A".repeat(SHA256_HEX_LENGTH);
        let checksum = Sha256Checksum::parse(uppercase).unwrap();
        assert_eq!(checksum.as_str(), valid_hex());
    }

    #[test]
    fn rejects_the_wrong_length() {
        let error = Sha256Checksum::parse("abc").unwrap_err();
        assert_eq!(
            error,
            ChecksumError::InvalidLength {
                expected: SHA256_HEX_LENGTH,
                actual: 3,
            }
        );
    }

    #[test]
    fn rejects_a_non_hexadecimal_character() {
        let mut input = valid_hex();
        input.replace_range(0..1, "g");

        let error = Sha256Checksum::parse(input).unwrap_err();
        assert_eq!(error, ChecksumError::InvalidCharacter('g'));
    }
}
