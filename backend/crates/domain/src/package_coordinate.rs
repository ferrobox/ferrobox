use std::fmt;

use thiserror::Error;

const MAX_COMPONENT_LENGTH: usize = 214;

/// The package ecosystems that `FerroBox` can index.
///
/// Unlike `RepositoryKind`, no variant needs additional data
/// -- the ecosystem itself is only a label that, from
/// Phase 7 onward, will determine which concrete Strategy-pattern
/// implementation manages publication, indexing, and download of this
/// kind of package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageEcosystem {
    /// Binary artifacts with no specific package format.
    Generic,
    /// Rust crates (registry compatible with the sparse index
    /// protocol of `cargo`).
    Cargo,
    /// Node.js packages.
    Npm,
    /// Python packages (Python Package Index).
    PyPi,
    /// Artifacts conforming to the OCI specification.
    Oci,
    /// Helm Charts, packaged as OCI artifacts.
    Helm,
    /// C/C++ packages from the Conan manager (v2 API with revisions).
    Conan,
    /// Maven artifacts (`groupId:artifactId`, classic HTTP layout).
    Maven,
    /// `NuGet` packages (V3 API: `dotnet nuget push` / `dotnet restore`).
    Nuget,
    /// Go modules (`GOPROXY` protocol: `go get` / `go mod download`).
    Go,
}

impl PackageEcosystem {
    /// Brief one-word description, useful for logs and debugging.
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

/// Reasons why a package name or version is not valid.
///
/// Only covers the rules common to *all* ecosystems. Rules
/// specific to each one (for example, scoped npm packages)
/// live in the corresponding packaging strategy, not here.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PackageComponentError {
    /// The value cannot be empty.
    #[error("value cannot be empty")]
    Empty,

    /// The value exceeds the maximum allowed length.
    #[error("value cannot exceed {max} characters, got {actual}")]
    TooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length received.
        actual: usize,
    },

    /// The value contains a whitespace or control character.
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

/// Validated package name, without rules specific to any
/// concrete ecosystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageName(String);

impl PackageName {
    /// Validates and builds a package name.
    ///
    /// # Errors
    ///
    /// Returns [`PackageComponentError`] if `name` is empty, exceeds
    /// `214` characters, or contains a whitespace or control
    /// character.
    pub fn parse(name: impl Into<String>) -> Result<Self, PackageComponentError> {
        let name = name.into();
        validate_component(&name)?;
        Ok(Self(name))
    }

    /// Returns the name as a text string.
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

/// Validated package version, without any versioning
/// rule (Semantic Versioning, PEP 440, etc.) yet -- those
/// rules will be applied in the corresponding packaging strategy.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageVersion(String);

impl PackageVersion {
    /// Validates and builds a package version.
    ///
    /// # Errors
    ///
    /// Returns [`PackageComponentError`] if `version` is empty, exceeds
    /// `214` characters, or contains a whitespace or control
    /// character.
    pub fn parse(version: impl Into<String>) -> Result<Self, PackageComponentError> {
        let version = version.into();
        validate_component(&version)?;
        Ok(Self(version))
    }

    /// Returns the version as a text string.
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

/// Uniquely identifies a concrete version of a package within
/// an ecosystem: for example, `serde` at version `1.0.210`, within
/// the `Cargo` ecosystem.
///
/// It is a pure Value Object: two coordinates with the same ecosystem,
/// name, and version are interchangeable, so `PartialEq` and `Hash` are
/// derived directly -- unlike `Artifact` and `Repository`, there
/// is no notion of identity beyond the value itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageCoordinate {
    ecosystem: PackageEcosystem,
    name: PackageName,
    version: PackageVersion,
}

impl PackageCoordinate {
    /// Combines an already validated ecosystem, name, and version into
    /// a package coordinate.
    ///
    /// Does not return `Result`: unlike `Repository::new`, no
    /// new invariant emerges when combining the parts -- each one
    /// was already validated at its own construction, and any
    /// combination of an ecosystem, a valid name, and a valid
    /// version is, in itself, a valid coordinate.
    #[must_use]
    pub fn new(ecosystem: PackageEcosystem, name: PackageName, version: PackageVersion) -> Self {
        Self {
            ecosystem,
            name,
            version,
        }
    }

    /// Ecosystem this package belongs to.
    #[must_use]
    pub fn ecosystem(&self) -> PackageEcosystem {
        self.ecosystem
    }

    /// Package name.
    #[must_use]
    pub fn name(&self) -> &PackageName {
        &self.name
    }

    /// Package version.
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
