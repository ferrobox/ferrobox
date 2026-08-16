use std::fmt;

use thiserror::Error;

use crate::ids::GroupId;
use crate::user::Role;

const MAX_GROUP_NAME_LENGTH: usize = 64;

/// Acceso efectivo de un usuario a un repositorio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepositoryAccess {
    /// Puede listar y descargar.
    Read,
    /// Puede publicar, borrar y configurar el repositorio.
    Write,
}

impl RepositoryAccess {
    /// Convierte el rol asignado a un grupo en acceso al repositorio.
    /// `Admin` no se usa a nivel de grupo: es un rol de instancia.
    #[must_use]
    pub fn from_group_role(role: Role) -> Option<Self> {
        match role {
            Role::Reader => Some(Self::Read),
            Role::Developer => Some(Self::Write),
            Role::Admin => None,
        }
    }

    /// Etiqueta estable usada en la API HTTP.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    /// `true` si incluye escritura.
    #[must_use]
    pub fn can_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// Nombre validado de un grupo: no vacío, con longitud acotada, y
/// restringido a caracteres seguros para identificadores.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupName(String);

/// Motivos por los que una cadena no es un [`GroupName`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GroupNameError {
    /// El nombre no puede estar vacío.
    #[error("group name cannot be empty")]
    Empty,

    /// El nombre supera la longitud máxima permitida.
    #[error("group name cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Longitud máxima permitida.
        max: usize,
        /// Longitud real recibida.
        actual: usize,
    },

    /// El nombre contiene un carácter fuera del alfabeto permitido.
    #[error(
        "group name contains an invalid character: '{0}' \
         (only ASCII letters, digits, '-' and '_' are allowed)"
    )]
    InvalidCharacter(char),
}

impl GroupName {
    /// Valida y construye un nombre de grupo.
    ///
    /// # Errors
    ///
    /// Devuelve [`GroupNameError`] si `name` está vacío, supera `64`
    /// caracteres, o contiene un carácter fuera del alfabeto permitido.
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

    /// Devuelve el nombre como cadena de texto.
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

/// Un grupo de usuarios al que un Admin asigna repositorios.
///
/// Los miembros y el acceso a repositorios no viven en esta entidad:
/// son asociaciones que persiste el almacén de grupos.
#[derive(Debug, Clone)]
pub struct Group {
    id: GroupId,
    name: GroupName,
}

impl Group {
    /// Crea un grupo nuevo.
    #[must_use]
    pub fn new(name: GroupName) -> Self {
        Self {
            id: GroupId::new(),
            name,
        }
    }

    /// Reconstituye un grupo ya persistido.
    #[must_use]
    pub fn from_parts(id: GroupId, name: GroupName) -> Self {
        Self { id, name }
    }

    /// Identificador único.
    #[must_use]
    pub fn id(&self) -> GroupId {
        self.id
    }

    /// Nombre del grupo.
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
