use std::fmt;

use thiserror::Error;

const MAX_COMPONENT_LENGTH: usize = 214;

/// Los ecosistemas de paquetes que `FerroBox` puede indexar.
///
/// A diferencia de `RepositoryKind`, ninguna variante necesita datos
/// adicionales -- el ecosistema en sí es solo una etiqueta que, a partir
/// de la Fase 7, determinará qué implementación concreta del patrón
/// Strategy gestiona la publicación, indexación y descarga de este tipo
/// de paquete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageEcosystem {
    /// Artefactos binarios sin ningún formato de paquete específico.
    Generic,
    /// Crates de Rust (registro compatible con el protocolo de índice
    /// disperso de `cargo`).
    Cargo,
    /// Paquetes de Node.js.
    Npm,
    /// Paquetes de Python (Python Package Index).
    PyPi,
    /// Artefactos conformes a la especificación OCI.
    Oci,
    /// Helm Charts, empaquetados como artefactos OCI.
    Helm,
    /// Paquetes C/C++ del gestor Conan (API v2 con revisiones).
    Conan,
    /// Artefactos Maven (`groupId:artifactId`, layout HTTP clásico).
    Maven,
    /// Paquetes `NuGet` (API V3: `dotnet nuget push` / `dotnet restore`).
    Nuget,
    /// Módulos Go (protocolo `GOPROXY`: `go get` / `go mod download`).
    Go,
}

impl PackageEcosystem {
    /// Descripción breve en una palabra, útil para registros y depuración.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::PyPi => "pypi",
            Self::Oci => "oci",
            Self::Helm => "helm",
            Self::Conan => "conan",
            Self::Maven => "maven",
            Self::Nuget => "nuget",
            Self::Go => "go",
        }
    }
}

/// Motivos por los que un nombre o una versión de paquete no son válidos.
///
/// Solo cubre las reglas comunes a *todos* los ecosistemas. Las reglas
/// propias de cada uno (por ejemplo, los paquetes con ámbito de npm)
/// viven en la estrategia de empaquetado correspondiente, no aquí.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PackageComponentError {
    /// El valor no puede estar vacío.
    #[error("value cannot be empty")]
    Empty,

    /// El valor supera la longitud máxima permitida.
    #[error("value cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Longitud máxima permitida.
        max: usize,
        /// Longitud real recibida.
        actual: usize,
    },

    /// El valor contiene un espacio en blanco o un carácter de control.
    #[error("value contains a whitespace or control character: {0:?}")]
    InvalidCharacter(char),
}

fn validate_component(value: &str) -> Result<(), PackageComponentError> {
    if value.is_empty() {
        return Err(PackageComponentError::Empty);
    }

    if value.len() > MAX_COMPONENT_LENGTH {
        return Err(PackageComponentError::TooLong {
            max: MAX_COMPONENT_LENGTH,
            actual: value.len(),
        });
    }

    if let Some(invalid) = value.chars().find(|c| c.is_whitespace() || c.is_control()) {
        return Err(PackageComponentError::InvalidCharacter(invalid));
    }

    Ok(())
}

/// Nombre validado de un paquete, sin reglas específicas de ningún
/// ecosistema concreto.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageName(String);

impl PackageName {
    /// Valida y construye un nombre de paquete.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackageComponentError`] si `name` está vacío, supera
    /// `214` caracteres, o contiene un espacio en blanco o un carácter
    /// de control.
    pub fn parse(name: impl Into<String>) -> Result<Self, PackageComponentError> {
        let name = name.into();
        validate_component(&name)?;
        Ok(Self(name))
    }

    /// Devuelve el nombre como cadena de texto.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Versión validada de un paquete, sin ninguna regla de versionado
/// específica (Semantic Versioning, PEP 440, etc.) todavía -- esas
/// reglas se aplicarán en la estrategia de empaquetado correspondiente.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageVersion(String);

impl PackageVersion {
    /// Valida y construye una versión de paquete.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackageComponentError`] si `version` está vacía, supera
    /// `214` caracteres, o contiene un espacio en blanco o un carácter
    /// de control.
    pub fn parse(version: impl Into<String>) -> Result<Self, PackageComponentError> {
        let version = version.into();
        validate_component(&version)?;
        Ok(Self(version))
    }

    /// Devuelve la versión como cadena de texto.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifica de forma unívoca una versión concreta de un paquete dentro
/// de un ecosistema: por ejemplo, `serde` en su versión `1.0.210`, dentro
/// del ecosistema `Cargo`.
///
/// Es un Objeto de Valor puro: dos coordenadas con el mismo ecosistema,
/// nombre y versión son intercambiables, así que `PartialEq` y `Hash` se
/// derivan directamente -- a diferencia de `Artifact` y `Repository`, no
/// hay ninguna noción de identidad propia más allá del valor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageCoordinate {
    ecosystem: PackageEcosystem,
    name: PackageName,
    version: PackageVersion,
}

impl PackageCoordinate {
    /// Combina un ecosistema, un nombre y una versión ya validados en
    /// una coordenada de paquete.
    ///
    /// No devuelve `Result`: a diferencia de `Repository::new`, aquí no
    /// emerge ningún invariante nuevo al combinar las partes -- cada una
    /// ya fue validada en su propia construcción, y cualquier
    /// combinación de un ecosistema, un nombre válido y una versión
    /// válida es, en sí misma, una coordenada válida.
    #[must_use]
    pub fn new(ecosystem: PackageEcosystem, name: PackageName, version: PackageVersion) -> Self {
        Self {
            ecosystem,
            name,
            version,
        }
    }

    /// Ecosistema al que pertenece este paquete.
    #[must_use]
    pub fn ecosystem(&self) -> PackageEcosystem {
        self.ecosystem
    }

    /// Nombre del paquete.
    #[must_use]
    pub fn name(&self) -> &PackageName {
        &self.name
    }

    /// Versión del paquete.
    #[must_use]
    pub fn version(&self) -> &PackageVersion {
        &self.version
    }
}

impl fmt::Display for PackageCoordinate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}@{}", self.ecosystem.label(), self.name, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_empty_name() {
        assert_eq!(PackageName::parse(""), Err(PackageComponentError::Empty));
    }

    #[test]
    fn rejects_a_version_with_whitespace() {
        assert_eq!(
            PackageVersion::parse("1.0 .0"),
            Err(PackageComponentError::InvalidCharacter(' '))
        );
    }

    fn cargo_coordinate(version: &str) -> PackageCoordinate {
        PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("serde").unwrap(),
            PackageVersion::parse(version).unwrap(),
        )
    }

    #[test]
    fn two_coordinates_with_the_same_parts_are_equal() {
        assert_eq!(cargo_coordinate("1.0.210"), cargo_coordinate("1.0.210"));
    }

    #[test]
    fn a_different_version_makes_a_different_coordinate() {
        assert_ne!(cargo_coordinate("1.0.210"), cargo_coordinate("1.0.211"));
    }

    #[test]
    fn displays_as_ecosystem_colon_name_at_version() {
        assert_eq!(cargo_coordinate("1.0.210").to_string(), "cargo:serde@1.0.210");
    }
}
