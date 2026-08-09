use std::fmt;

use thiserror::Error;

const SHA256_HEX_LENGTH: usize = 64;

/// Un checksum SHA-256 validado: siempre exactamente 64 caracteres
/// hexadecimales en minúsculas.
///
/// La única forma de obtener una instancia es a través de [`Self::parse`],
/// que garantiza el invariante en el momento de la construcción -- una vez
/// que existe un `Sha256Checksum`, el resto del sistema puede confiar en
/// que es válido sin volver a comprobarlo.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Checksum(String);

/// Motivos por los que una cadena no es un [`Sha256Checksum`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChecksumError {
    /// El texto no tiene la longitud exacta esperada.
    #[error("checksum must be exactly {expected} hexadecimal characters, got {actual}")]
    InvalidLength {
        /// Longitud esperada, en caracteres.
        expected: usize,
        /// Longitud real recibida, en caracteres.
        actual: usize,
    },

    /// El texto contiene un carácter fuera del alfabeto hexadecimal.
    #[error("checksum contains a non-hexadecimal character: '{0}'")]
    InvalidCharacter(char),
}

impl Sha256Checksum {
    /// Valida y construye un checksum a partir de su representación
    /// hexadecimal, normalizando cualquier letra en mayúscula a minúscula.
    ///
    /// # Errors
    ///
    /// Devuelve [`ChecksumError`] si `hex` no tiene exactamente 64
    /// caracteres, o si contiene algún carácter fuera del alfabeto
    /// hexadecimal.
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

    /// Devuelve la representación hexadecimal en minúsculas.
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
