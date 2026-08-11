use std::fmt;

use thiserror::Error;

use crate::ids::UserId;

const MAX_USERNAME_LENGTH: usize = 64;

/// Nombre de usuario validado: no vacío, con longitud acotada, y
/// restringido a caracteres seguros para identificadores.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Username(String);

/// Motivos por los que una cadena no es un [`Username`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsernameError {
    /// El nombre de usuario no puede estar vacío.
    #[error("username cannot be empty")]
    Empty,

    /// El nombre de usuario supera la longitud máxima permitida.
    #[error("username cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Longitud máxima permitida.
        max: usize,
        /// Longitud real recibida.
        actual: usize,
    },

    /// El nombre contiene un carácter fuera del alfabeto permitido.
    #[error(
        "username contains an invalid character: '{0}' \
         (only ASCII letters, digits, '-' and '_' are allowed)"
    )]
    InvalidCharacter(char),
}

impl Username {
    /// Valida y construye un nombre de usuario.
    ///
    /// # Errors
    ///
    /// Devuelve [`UsernameError`] si `username` está vacío, supera
    /// `64` caracteres, o contiene algún carácter fuera del alfabeto
    /// permitido (letras y dígitos ASCII, `-` y `_`).
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

    /// Devuelve el nombre de usuario como cadena de texto.
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

/// Un usuario de `FerroBox`: identidad autenticable que puede poseer
/// tokens de API.
///
/// El hash de la contraseña no forma parte de esta entidad: es un
/// detalle de credenciales que vive en la capa de aplicación /
/// persistencia, no un concepto del modelo de negocio.
#[derive(Debug, Clone)]
pub struct User {
    id: UserId,
    username: Username,
}

impl User {
    /// Registra un usuario nuevo, asignándole un identificador nuevo.
    #[must_use]
    pub fn new(username: Username) -> Self {
        Self {
            id: UserId::new(),
            username,
        }
    }

    /// Reconstituye un usuario ya existente a partir de un
    /// identificador conocido (por ejemplo, al cargarlo desde
    /// persistencia).
    #[must_use]
    pub fn from_parts(id: UserId, username: Username) -> Self {
        Self { id, username }
    }

    /// Identificador único de este usuario.
    #[must_use]
    pub fn id(&self) -> UserId {
        self.id
    }

    /// Nombre de usuario.
    #[must_use]
    pub fn username(&self) -> &Username {
        &self.username
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
        let first = User::from_parts(id, Username::parse("admin").unwrap());
        let second = User::from_parts(id, Username::parse("other").unwrap());

        assert_eq!(first, second);
    }
}
