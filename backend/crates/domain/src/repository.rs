use std::fmt;

use thiserror::Error;
use url::Url;

use crate::ids::RepositoryId;
use crate::package_coordinate::PackageEcosystem;

const MAX_NAME_LENGTH: usize = 100;

/// Nombre validado de un repositorio: no vacío, con longitud acotada, y
/// restringido a caracteres seguros para aparecer en una URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepositoryName(String);

/// Motivos por los que una cadena no es un [`RepositoryName`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RepositoryNameError {
    /// El nombre no puede estar vacío.
    #[error("repository name cannot be empty")]
    Empty,

    /// El nombre supera la longitud máxima permitida.
    #[error("repository name cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Longitud máxima permitida.
        max: usize,
        /// Longitud real recibida.
        actual: usize,
    },

    /// El nombre contiene un carácter fuera del alfabeto permitido.
    #[error(
        "repository name contains an invalid character: '{0}' \
         (only ASCII letters, digits, '-' and '_' are allowed)"
    )]
    InvalidCharacter(char),
}

impl RepositoryName {
    /// Valida y construye un nombre de repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryNameError`] si `name` está vacío, supera
    /// `100` caracteres, o contiene algún carácter fuera del alfabeto
    /// permitido (letras y dígitos ASCII, `-` y `_`).
    pub fn parse(name: impl Into<String>) -> Result<Self, RepositoryNameError> {
        let name = name.into();

        if name.is_empty() {
            return Err(RepositoryNameError::Empty);
        }

        if name.len() > MAX_NAME_LENGTH {
            return Err(RepositoryNameError::TooLong {
                max: MAX_NAME_LENGTH,
                actual: name.len(),
            });
        }

        if let Some(invalid) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(RepositoryNameError::InvalidCharacter(invalid));
        }

        Ok(Self(name))
    }

    /// Devuelve el nombre como cadena de texto.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for RepositoryName {
    type Error = RepositoryNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

/// La estrategia de origen y almacenamiento de un repositorio.
///
/// A diferencia de una enumeración al estilo C (una simple lista de
/// etiquetas), cada variante de este tipo lleva datos propios y
/// distintos -- es un *tipo algebraico de datos*: el compilador conoce,
/// para cada variante, exactamente qué campos existen, y exige manejar
/// todas las variantes explícitamente en cualquier `match` sobre este
/// tipo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryKind {
    /// Almacenamiento propio: `FerroBox` es la fuente de verdad del
    /// contenido publicado directamente aquí.
    Forge,

    /// Réplica cacheada de una fuente externa. La primera petición de un
    /// artefacto ausente se resuelve contra `upstream`; las peticiones
    /// posteriores se sirven desde la copia ya almacenada localmente.
    Mirror {
        /// URL base del repositorio externo que se está replicando.
        upstream: Url,
    },

    /// Punto de acceso único que agrega varios repositorios (`Forge`
    /// y/o `Mirror`) bajo una sola URL, resolviendo internamente contra
    /// cuál de ellos responder.
    Alloy {
        /// Repositorios agregados, en el orden en que se consultan.
        members: Vec<RepositoryId>,
    },
}

impl RepositoryKind {
    /// Descripción breve en una palabra, útil para registros y depuración.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Forge => "forge",
            Self::Mirror { .. } => "mirror",
            Self::Alloy { .. } => "alloy",
        }
    }
}

/// Un repositorio de artefactos: agrupa una estrategia de origen
/// (`RepositoryKind`) y un ecosistema de paquetes (`PackageEcosystem`)
/// bajo un nombre único.
///
/// `RepositoryKind` y `PackageEcosystem` son deliberadamente dos campos
/// independientes, no una sola jerarquía: el primero responde a "¿de
/// dónde viene el contenido y cómo se almacena?" (almacenamiento propio,
/// réplica cacheada, o agregación de otros repositorios), mientras que
/// el segundo responde a "¿qué formato de paquete contiene?" (Cargo,
/// npm, genérico...). Ambas preguntas son ortogonales -- un repositorio
/// `Mirror` puede replicar tanto un registro de Cargo como uno de npm, y
/// un repositorio `Forge` de Cargo se comporta, en cuanto a
/// almacenamiento, igual que uno `Forge` genérico. Nexus y Artifactory
/// modelan esta misma distinción con dos ejes independientes
/// (tipo de repositorio y formato de paquete); `FerroBox` sigue el mismo
/// principio de diseño con su propio vocabulario.
#[derive(Debug, Clone)]
pub struct Repository {
    id: RepositoryId,
    name: RepositoryName,
    kind: RepositoryKind,
    ecosystem: PackageEcosystem,
}

/// Motivos por los que una combinación de nombre y tipo no forma un
/// [`Repository`] válido.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RepositoryError {
    /// Un repositorio `Alloy` sin miembros no agrega nada -- es una
    /// promesa vacía, así que se rechaza en la propia construcción.
    #[error("an Alloy repository must aggregate at least one member repository")]
    EmptyAlloy,
}

impl Repository {
    /// Registra un repositorio nuevo, asignándole un identificador nuevo.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryError`] si `kind` es un `Alloy` sin miembros.
    pub fn new(
        name: RepositoryName,
        kind: RepositoryKind,
        ecosystem: PackageEcosystem,
    ) -> Result<Self, RepositoryError> {
        Self::validate(&kind)?;
        Ok(Self {
            id: RepositoryId::new(),
            name,
            kind,
            ecosystem,
        })
    }

    /// Reconstituye un repositorio ya existente a partir de un
    /// identificador conocido (por ejemplo, al cargarlo desde
    /// persistencia).
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryError`] si `kind` es un `Alloy` sin
    /// miembros -- incluso al reconstituir, no confiamos ciegamente en
    /// que los datos persistidos cumplan el invariante.
    pub fn from_parts(
        id: RepositoryId,
        name: RepositoryName,
        kind: RepositoryKind,
        ecosystem: PackageEcosystem,
    ) -> Result<Self, RepositoryError> {
        Self::validate(&kind)?;
        Ok(Self {
            id,
            name,
            kind,
            ecosystem,
        })
    }

    fn validate(kind: &RepositoryKind) -> Result<(), RepositoryError> {
        if let RepositoryKind::Alloy { members } = kind
            && members.is_empty()
        {
            return Err(RepositoryError::EmptyAlloy);
        }
        Ok(())
    }

    /// Identificador único de este repositorio.
    #[must_use]
    pub fn id(&self) -> RepositoryId {
        self.id
    }

    /// Nombre del repositorio.
    #[must_use]
    pub fn name(&self) -> &RepositoryName {
        &self.name
    }

    /// Estrategia de origen y almacenamiento de este repositorio.
    #[must_use]
    pub fn kind(&self) -> &RepositoryKind {
        &self.kind
    }

    /// Ecosistema de paquetes que este repositorio indexa.
    #[must_use]
    pub fn ecosystem(&self) -> PackageEcosystem {
        self.ecosystem
    }

    /// Sustituye la estrategia de origen, conservando identidad, nombre
    /// y ecosistema. Sirve para actualizar los miembros de un `Alloy`.
    ///
    /// # Errors
    ///
    /// Devuelve [`RepositoryError`] si `kind` es un `Alloy` sin miembros.
    pub fn with_kind(self, kind: RepositoryKind) -> Result<Self, RepositoryError> {
        Self::validate(&kind)?;
        Ok(Self { kind, ..self })
    }
}

impl PartialEq for Repository {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Repository {}

impl std::hash::Hash for Repository {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_empty_name() {
        assert_eq!(RepositoryName::parse(""), Err(RepositoryNameError::Empty));
    }

    #[test]
    fn rejects_an_invalid_character() {
        assert_eq!(
            RepositoryName::parse("my repo"),
            Err(RepositoryNameError::InvalidCharacter(' '))
        );
    }

    #[test]
    fn accepts_a_valid_name() {
        assert!(RepositoryName::parse("cargo-releases").is_ok());
    }

    #[test]
    fn a_forge_repository_can_be_created() {
        let name = RepositoryName::parse("cargo-releases").unwrap();
        assert!(Repository::new(name, RepositoryKind::Forge, PackageEcosystem::Cargo).is_ok());
    }

    #[test]
    fn an_alloy_without_members_is_rejected() {
        let name = RepositoryName::parse("public-cargo").unwrap();
        let result = Repository::new(
            name,
            RepositoryKind::Alloy { members: vec![] },
            PackageEcosystem::Cargo,
        );

        assert_eq!(result, Err(RepositoryError::EmptyAlloy));
    }

    #[test]
    fn an_alloy_with_at_least_one_member_is_accepted() {
        let name = RepositoryName::parse("public-cargo").unwrap();
        let kind = RepositoryKind::Alloy {
            members: vec![RepositoryId::new()],
        };

        assert!(Repository::new(name, kind, PackageEcosystem::Cargo).is_ok());
    }

    #[test]
    fn equality_is_based_on_identity_not_on_name_or_kind() {
        let id = RepositoryId::new();
        let original = Repository::from_parts(
            id,
            RepositoryName::parse("cargo-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let renamed = Repository::from_parts(
            id,
            RepositoryName::parse("cargo-releases-v2").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();

        assert_eq!(original, renamed);
    }

    #[test]
    fn exposes_the_package_ecosystem_it_indexes() {
        let name = RepositoryName::parse("npm-releases").unwrap();
        let repository =
            Repository::new(name, RepositoryKind::Forge, PackageEcosystem::Npm).unwrap();

        assert_eq!(repository.ecosystem(), PackageEcosystem::Npm);
    }

    #[test]
    fn with_kind_replaces_alloy_members_and_keeps_identity() {
        let first = RepositoryId::new();
        let second = RepositoryId::new();
        let original = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let id = original.id();

        let updated = original
            .with_kind(RepositoryKind::Alloy {
                members: vec![second],
            })
            .unwrap();

        assert_eq!(updated.id(), id);
        assert_eq!(updated.name().as_str(), "crates-alloy");
        match updated.kind() {
            RepositoryKind::Alloy { members } => assert_eq!(members, &vec![second]),
            other => panic!("expected alloy, got {other:?}"),
        }
    }

    #[test]
    fn with_kind_rejects_an_empty_alloy() {
        let original = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![RepositoryId::new()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();

        let result = original.with_kind(RepositoryKind::Alloy { members: vec![] });
        assert_eq!(result.err(), Some(RepositoryError::EmptyAlloy));
    }
}
