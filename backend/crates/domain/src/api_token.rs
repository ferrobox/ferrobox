use std::fmt;

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::ids::{ApiTokenId, UserId};

const MAX_NAME_LENGTH: usize = 100;

/// Nombre descriptivo de un token de API, validado.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApiTokenName(String);

/// Motivos por los que una cadena no es un [`ApiTokenName`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApiTokenNameError {
    /// El nombre no puede estar vacío.
    #[error("API token name cannot be empty")]
    Empty,

    /// El nombre supera la longitud máxima permitida.
    #[error("API token name cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Longitud máxima permitida.
        max: usize,
        /// Longitud real recibida.
        actual: usize,
    },
}

impl ApiTokenName {
    /// Valida y construye un nombre de token de API.
    ///
    /// # Errors
    ///
    /// Devuelve [`ApiTokenNameError`] si `name` está vacío o supera
    /// `100` caracteres.
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

    /// Devuelve el nombre como cadena de texto.
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

/// Un token de API emitido a un usuario.
///
/// El secreto en claro solo se muestra una vez al crearlo; en
/// persistencia se guarda únicamente su hash. Esta entidad conserva un
/// `prefix` no sensible para que el usuario pueda reconocer el token
/// en listados posteriores.
#[derive(Debug, Clone)]
pub struct ApiToken {
    id: ApiTokenId,
    user_id: UserId,
    name: ApiTokenName,
    prefix: String,
    expires_at: Option<DateTime<Utc>>,
}

impl ApiToken {
    /// Emite un token nuevo, asignándole un identificador nuevo.
    #[must_use]
    pub fn new(user_id: UserId, name: ApiTokenName, prefix: String) -> Self {
        Self {
            id: ApiTokenId::new(),
            user_id,
            name,
            prefix,
            expires_at: None,
        }
    }

    /// Reconstituye un token ya existente a partir de un identificador
    /// conocido (por ejemplo, al cargarlo desde persistencia).
    #[must_use]
    pub fn from_parts(
        id: ApiTokenId,
        user_id: UserId,
        name: ApiTokenName,
        prefix: String,
    ) -> Self {
        Self {
            id,
            user_id,
            name,
            prefix,
            expires_at: None,
        }
    }

    /// Identificador único de este token.
    #[must_use]
    pub fn id(&self) -> ApiTokenId {
        self.id
    }

    /// Identificador del usuario propietario.
    #[must_use]
    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    /// Nombre descriptivo del token.
    #[must_use]
    pub fn name(&self) -> &ApiTokenName {
        &self.name
    }

    /// Prefijo no sensible del secreto, útil para reconocerlo en
    /// listados.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Momento en el que el token deja de ser válido, si tiene
    /// caducidad.
    #[must_use]
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }

    /// `true` si `now` es posterior o igual a [`Self::expires_at`].
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|at| now >= at)
    }

    /// Devuelve este token con una caducidad distinta.
    #[must_use]
    pub fn with_expires_at(self, expires_at: Option<DateTime<Utc>>) -> Self {
        Self {
            expires_at,
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
}
