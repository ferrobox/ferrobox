//! Conversion between [`PackageEcosystem`] and its text-column
//! representation in `PostgreSQL`. This is an internal convention
//! shared by more than one adapter (`repositories.ecosystem`,
//! `package_index_entries.ecosystem`), so it lives in a single place
//! instead of being duplicated.

use ferrobox_domain::package_coordinate::PackageEcosystem;

/// Translates a [`PackageEcosystem`] to the string persisted in
/// text columns.
#[must_use]
pub(crate) fn to_column(ecosystem: PackageEcosystem) -> &'static str {
    match ecosystem {
        PackageEcosystem::Generic => "generic",
        PackageEcosystem::Cargo => "cargo",
        PackageEcosystem::Npm => "npm",
        PackageEcosystem::PyPi => "pypi",
        PackageEcosystem::Oci => "oci",
        PackageEcosystem::Helm => "helm",
        PackageEcosystem::Conan => "conan",
        PackageEcosystem::Maven => "maven",
        PackageEcosystem::Nuget => "nuget",
        PackageEcosystem::Go => "go",
    }
}

/// Translates the string persisted in a text column back to a
/// [`PackageEcosystem`].
///
/// # Errors
///
/// Returns a readable error message if `ecosystem` does not match any
/// known value -- for example, if the schema evolved and this version
/// of the code does not yet know a new ecosystem.
pub(crate) fn from_column(ecosystem: &str) -> Result<PackageEcosystem, String> {
    match ecosystem {
        "generic" => Ok(PackageEcosystem::Generic),
        "cargo" => Ok(PackageEcosystem::Cargo),
        "npm" => Ok(PackageEcosystem::Npm),
        "pypi" => Ok(PackageEcosystem::PyPi),
        "oci" => Ok(PackageEcosystem::Oci),
        "helm" => Ok(PackageEcosystem::Helm),
        "conan" => Ok(PackageEcosystem::Conan),
        "maven" => Ok(PackageEcosystem::Maven),
        "nuget" => Ok(PackageEcosystem::Nuget),
        "go" => Ok(PackageEcosystem::Go),
        other => Err(format!("unknown package ecosystem: {other}")),
    }
}
