//! Conversión entre [`PackageEcosystem`] y su representación como
//! columna de texto en `PostgreSQL`. Es una convención interna
//! compartida por más de un adaptador (`repositories.ecosystem`,
//! `package_index_entries.ecosystem`), así que vive en un único lugar en
//! vez de duplicarse.

use ferrobox_domain::package_coordinate::PackageEcosystem;

/// Traduce un [`PackageEcosystem`] a la cadena que se persiste en
/// columnas de texto.
#[must_use]
pub(crate) fn to_column(ecosystem: PackageEcosystem) -> &'static str {
    match ecosystem {
        PackageEcosystem::Generic => "generic",
        PackageEcosystem::Cargo => "cargo",
        PackageEcosystem::Npm => "npm",
        PackageEcosystem::PyPi => "pypi",
        PackageEcosystem::Oci => "oci",
        PackageEcosystem::Helm => "helm",
    }
}

/// Traduce la cadena persistida en una columna de texto de vuelta a un
/// [`PackageEcosystem`].
///
/// # Errors
///
/// Devuelve un mensaje de error legible si `ecosystem` no corresponde a
/// ningún valor conocido -- por ejemplo, si el esquema evolucionó y esta
/// versión del código todavía no conoce un ecosistema nuevo.
pub(crate) fn from_column(ecosystem: &str) -> Result<PackageEcosystem, String> {
    match ecosystem {
        "generic" => Ok(PackageEcosystem::Generic),
        "cargo" => Ok(PackageEcosystem::Cargo),
        "npm" => Ok(PackageEcosystem::Npm),
        "pypi" => Ok(PackageEcosystem::PyPi),
        "oci" => Ok(PackageEcosystem::Oci),
        "helm" => Ok(PackageEcosystem::Helm),
        other => Err(format!("unknown package ecosystem: {other}")),
    }
}
