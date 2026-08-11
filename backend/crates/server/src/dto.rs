//! Cuerpos de petición y de respuesta de la API HTTP.
//!
//! Todos derivan [`ts_rs::TS`] con `#[ts(export)]`: `cargo test -p
//! ferrobox-server` regenera automáticamente los tipos de TypeScript
//! equivalentes en `frontend/src/api/generated/`.

use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::{Repository, RepositoryKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Cuerpo de una respuesta de error.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

/// Ecosistema de paquetes de un repositorio, tal y como viaja en la API
/// HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PackageEcosystemDto {
    /// Artefactos binarios sin ningún formato de paquete específico.
    Generic,
    /// Crates de Rust.
    Cargo,
    /// Paquetes de Node.js.
    Npm,
    /// Paquetes de Python.
    Pypi,
    /// Artefactos conformes a la especificación OCI.
    Oci,
    /// Helm Charts.
    Helm,
}

impl From<PackageEcosystem> for PackageEcosystemDto {
    fn from(ecosystem: PackageEcosystem) -> Self {
        match ecosystem {
            PackageEcosystem::Generic => Self::Generic,
            PackageEcosystem::Cargo => Self::Cargo,
            PackageEcosystem::Npm => Self::Npm,
            PackageEcosystem::PyPi => Self::Pypi,
            PackageEcosystem::Oci => Self::Oci,
            PackageEcosystem::Helm => Self::Helm,
        }
    }
}

impl From<PackageEcosystemDto> for PackageEcosystem {
    fn from(dto: PackageEcosystemDto) -> Self {
        match dto {
            PackageEcosystemDto::Generic => Self::Generic,
            PackageEcosystemDto::Cargo => Self::Cargo,
            PackageEcosystemDto::Npm => Self::Npm,
            PackageEcosystemDto::Pypi => Self::PyPi,
            PackageEcosystemDto::Oci => Self::Oci,
            PackageEcosystemDto::Helm => Self::Helm,
        }
    }
}

/// Estrategia de origen y almacenamiento de un repositorio, tal y como
/// viaja en la API HTTP. Se serializa como una unión discriminada por el
/// campo `type`, para que el cliente de TypeScript generado obtenga un
/// tipo de unión exhaustivo en vez de una cadena de texto opaca.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum RepositoryKindDto {
    /// Almacenamiento propio.
    Forge,
    /// Réplica cacheada de una fuente externa.
    Mirror {
        /// URL base del repositorio externo replicado.
        upstream: String,
    },
    /// Agregación de otros repositorios.
    Alloy {
        /// Identificadores de los repositorios agregados.
        members: Vec<String>,
    },
}

impl From<&RepositoryKind> for RepositoryKindDto {
    fn from(kind: &RepositoryKind) -> Self {
        match kind {
            RepositoryKind::Forge => Self::Forge,
            RepositoryKind::Mirror { upstream } => Self::Mirror {
                upstream: upstream.to_string(),
            },
            RepositoryKind::Alloy { members } => Self::Alloy {
                members: members.iter().map(ToString::to_string).collect(),
            },
        }
    }
}

/// Representación de un repositorio en las respuestas de la API.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct RepositoryResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: RepositoryKindDto,
    pub(crate) ecosystem: PackageEcosystemDto,
}

impl From<&Repository> for RepositoryResponse {
    fn from(repository: &Repository) -> Self {
        Self {
            id: repository.id().to_string(),
            name: repository.name().to_string(),
            kind: repository.kind().into(),
            ecosystem: repository.ecosystem().into(),
        }
    }
}

/// Cuerpo de la petición para crear un repositorio.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRepositoryRequest {
    pub(crate) name: String,
    pub(crate) ecosystem: PackageEcosystemDto,
}

/// Respuesta al crear un repositorio correctamente.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRepositoryResponse {
    pub(crate) id: String,
}

/// Representación de un artefacto en las respuestas de la API.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ArtifactResponse {
    pub(crate) id: String,
    pub(crate) checksum: String,
    #[ts(type = "number")]
    pub(crate) size_bytes: u64,
}

impl From<Artifact> for ArtifactResponse {
    fn from(artifact: Artifact) -> Self {
        Self {
            id: artifact.id().to_string(),
            checksum: artifact.checksum().to_string(),
            size_bytes: artifact.size_bytes(),
        }
    }
}

/// Respuesta al publicar un artefacto correctamente.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PublishResponse {
    pub(crate) id: String,
}

/// Cuerpo de la petición de inicio de sesión.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct LoginRequest {
    pub(crate) username: String,
    pub(crate) password: String,
}

/// Representación de un usuario en las respuestas de la API.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct UserResponse {
    pub(crate) id: String,
    pub(crate) username: String,
}

/// Respuesta al iniciar sesión correctamente.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct LoginResponse {
    /// Secreto del token de sesión (mostrado una sola vez).
    pub(crate) token: String,
    pub(crate) user: UserResponse,
}

/// Cuerpo de la petición para crear un token de API.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateApiTokenRequest {
    pub(crate) name: String,
}

/// Representación de un token de API (sin secreto) en listados.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ApiTokenResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) created_at: String,
}

/// Respuesta al crear un token de API: incluye el secreto una sola vez.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ApiTokenCreatedResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    /// Secreto en claro. Solo se expone en esta respuesta.
    pub(crate) token: String,
}
