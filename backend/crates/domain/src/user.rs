use std::fmt;

use thiserror::Error;

use crate::ids::UserId;

const MAX_USERNAME_LENGTH: usize = 64;

/// Rol de autorización de un usuario en `FerroBox`.
///
/// Los roles son deliberadamente pocos y ordenados por privilegio:
/// `Admin` gestiona usuarios y puede escribir; `Developer` puede
/// publicar y crear repositorios; `Reader` solo lee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Gestión completa: usuarios, repositorios y publicación.
    Admin,
    /// Puede crear repositorios y publicar artefactos; no gestiona
    /// usuarios.
    Developer,
    /// Solo lectura de repositorios y artefactos.
    Reader,
}

impl Role {
    /// Etiqueta estable usada en persistencia y en la API HTTP.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Developer => "developer",
            Self::Reader => "reader",
        }
    }

    /// Parsea la etiqueta estable de un rol.
    ///
    /// # Errors
    ///
    /// Devuelve [`RoleError::Unknown`] si la etiqueta no es conocida.
    pub fn parse(value: &str) -> Result<Self, RoleError> {
        match value {
            "admin" => Ok(Self::Admin),
            "developer" => Ok(Self::Developer),
            "reader" => Ok(Self::Reader),
            other => Err(RoleError::Unknown(other.to_string())),
        }
    }

    /// `true` si este rol puede gestionar usuarios.
    #[must_use]
    pub fn can_manage_users(self) -> bool {
        matches!(self, Self::Admin)
    }

    /// `true` si este rol puede crear repositorios y publicar
    /// artefactos.
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

/// Motivos por los que una cadena no es un [`Role`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RoleError {
    /// Etiqueta de rol desconocida.
    #[error("unknown role: '{0}'")]
    Unknown(String),
}

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

/// Un usuario de `FerroBox`: identidad autenticable con un rol de
/// autorización, que puede poseer tokens de API.
///
/// El hash de la contraseña no forma parte de esta entidad: es un
/// detalle de credenciales que vive en la capa de aplicación /
/// persistencia, no un concepto del modelo de negocio.
#[derive(Debug, Clone)]
pub struct User {
    id: UserId,
    username: Username,
    role: Role,
}

impl User {
    /// Registra un usuario nuevo, asignándole un identificador nuevo.
    #[must_use]
    pub fn new(username: Username, role: Role) -> Self {
        Self {
            id: UserId::new(),
            username,
            role,
        }
    }

    /// Reconstituye un usuario ya existente a partir de un
    /// identificador conocido (por ejemplo, al cargarlo desde
    /// persistencia).
    #[must_use]
    pub fn from_parts(id: UserId, username: Username, role: Role) -> Self {
        Self { id, username, role }
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

    /// Rol de autorización de este usuario.
    #[must_use]
    pub fn role(&self) -> Role {
        self.role
    }

    /// Devuelve este usuario con un rol distinto. La identidad no cambia.
    #[must_use]
    pub fn with_role(self, role: Role) -> Self {
        Self { role, ..self }
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
        let first = User::from_parts(id, Username::parse("admin").unwrap(), Role::Admin);
        let second = User::from_parts(id, Username::parse("other").unwrap(), Role::Reader);

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
}
