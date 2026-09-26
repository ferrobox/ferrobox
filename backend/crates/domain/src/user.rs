use std::fmt;

use thiserror::Error;

use crate::ids::UserId;
use crate::oidc::OidcIdentity;

const MAX_USERNAME_LENGTH: usize = 64;
const MAX_EMAIL_LENGTH: usize = 254;
const MIN_PASSWORD_LENGTH: usize = 8;

/// Rol de autorización de un usuario en `FerroBox`.
///
/// Los roles son deliberadamente pocos y ordenados por privilegio:
/// `Admin` gestiona cuentas (crear, borrar, cambiar rol, restablecer
/// contraseñas ajenas) y puede escribir; `Developer` puede publicar y
/// crear repositorios; `Reader` solo lee.
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

/// Correo electrónico validado, normalizado a minúsculas ASCII.
///
/// El administrador inicial creado al arrancar puede no tener correo
/// (`None` en [`User`]). Las cuentas que crea un Admin sí lo requieren.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

/// Motivos por los que una cadena no es un [`Email`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EmailError {
    /// El correo no puede estar vacío.
    #[error("email cannot be empty")]
    Empty,

    /// El correo supera la longitud máxima permitida.
    #[error("email cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Longitud máxima permitida.
        max: usize,
        /// Longitud real recibida.
        actual: usize,
    },

    /// El correo no tiene forma `local@dominio.tld`.
    #[error("email is not a valid address")]
    Invalid,
}

impl Email {
    /// Valida y normaliza un correo electrónico.
    ///
    /// # Errors
    ///
    /// Devuelve [`EmailError`] si está vacío, es demasiado largo o no
    /// tiene forma de dirección.
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

    /// Devuelve el correo como cadena de texto.
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

/// Motivos por los que una contraseña no cumple la política de la
/// instancia.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PasswordPolicyError {
    /// Menos de ocho caracteres, o le falta minúscula, mayúscula o
    /// dígito.
    #[error(
        "password must be at least {min} characters and include a lowercase letter, \
         an uppercase letter, and a digit"
    )]
    TooWeak {
        /// Longitud mínima exigida.
        min: usize,
    },
}

/// Comprueba que `password` tenga al menos 8 caracteres, una minúscula,
/// una mayúscula y un dígito. Aplica a crear cuentas, restablecer y
/// cambiar la propia contraseña; no al administrador inicial de
/// arranque.
///
/// # Errors
///
/// Devuelve [`PasswordPolicyError::TooWeak`] si no cumple la política.
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
    email: Option<Email>,
    role: Role,
    oidc: Option<OidcIdentity>,
    robot: bool,
}

impl User {
    /// Registra un usuario nuevo, asignándole un identificador nuevo.
    /// El correo queda vacío: úsalo para el administrador de arranque
    /// o llama a [`User::with_email`].
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

    /// Reconstituye un usuario ya existente a partir de un
    /// identificador conocido (por ejemplo, al cargarlo desde
    /// persistencia).
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

    /// Correo electrónico, si la cuenta lo tiene.
    #[must_use]
    pub fn email(&self) -> Option<&Email> {
        self.email.as_ref()
    }

    /// Rol de autorización de este usuario.
    #[must_use]
    pub fn role(&self) -> Role {
        self.role
    }

    /// Devuelve este usuario con un correo distinto. La identidad no
    /// cambia.
    #[must_use]
    pub fn with_email(self, email: Option<Email>) -> Self {
        Self { email, ..self }
    }

    /// Devuelve este usuario con un rol distinto. La identidad no cambia.
    #[must_use]
    pub fn with_role(self, role: Role) -> Self {
        Self { role, ..self }
    }

    /// Identidad federada, si la cuenta se ha vinculado a un `IdP`.
    #[must_use]
    pub fn oidc(&self) -> Option<&OidcIdentity> {
        self.oidc.as_ref()
    }

    /// `true` si la cuenta está vinculada a un emisor `OIDC`.
    #[must_use]
    pub fn is_sso_linked(&self) -> bool {
        self.oidc.is_some()
    }

    /// Devuelve este usuario vinculado a una identidad `OIDC`.
    #[must_use]
    pub fn with_oidc(self, identity: Option<OidcIdentity>) -> Self {
        Self {
            oidc: identity,
            ..self
        }
    }

    /// `true` si es una cuenta robot (CI): no entra con contraseña ni
    /// `SSO`, solo con tokens de API.
    #[must_use]
    pub fn is_robot(&self) -> bool {
        self.robot
    }

    /// Marca o desmarca la cuenta como robot. Un robot no puede ser
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
