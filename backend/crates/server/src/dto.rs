//! Cuerpos de petición y de respuesta de la API HTTP.
//!
//! Todos derivan [`ts_rs::TS`] con `#[ts(export)]`: `cargo test -p
//! ferrobox-server` regenera automáticamente los tipos de TypeScript
//! equivalentes en `frontend/src/api/generated/`.

use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_domain::user::{Role, User};
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
    /// Paquetes C/C++ de Conan.
    Conan,
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
            PackageEcosystem::Conan => Self::Conan,
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
            PackageEcosystemDto::Conan => Self::Conan,
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
    /// Tipo de repositorio. Si se omite, se crea un `Forge`.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) kind: Option<CreateRepositoryKindDto>,
}

/// Tipo de repositorio solicitado al crearlo.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum CreateRepositoryKindDto {
    /// Almacenamiento propio.
    #[default]
    Forge,
    /// Réplica cacheada de un *upstream*.
    Mirror {
        /// URL base del índice disperso remoto (p. ej. `https://index.crates.io/`).
        upstream: String,
    },
    /// Agregación de otros repositorios `Forge` o `Mirror`.
    Alloy {
        /// Identificadores de los repositorios miembro, en orden de
        /// resolución.
        members: Vec<String>,
    },
}

/// Respuesta al crear un repositorio correctamente.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRepositoryResponse {
    pub(crate) id: String,
}

/// Cuerpo de la petición para actualizar los miembros de un `Alloy`.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct UpdateAlloyMembersRequest {
    /// Identificadores de los repositorios miembro, en orden de
    /// resolución.
    pub(crate) members: Vec<String>,
}

/// Representación de un artefacto en las respuestas de la API.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ArtifactResponse {
    pub(crate) id: String,
    /// Nombre del paquete (crate, etc.) si el índice lo conoce.
    pub(crate) name: Option<String>,
    /// Versión del paquete si el índice la conoce.
    pub(crate) version: Option<String>,
    /// Nombre de fichero en el índice, si el ecosistema lo distingue
    /// (receta `Conan`, sdist/wheel de `PyPI`, etc.).
    pub(crate) filename: Option<String>,
    pub(crate) checksum: String,
    #[ts(type = "number")]
    pub(crate) size_bytes: u64,
    /// `true` si el índice marca esta versión como *yanked*.
    pub(crate) yanked: bool,
    /// Repositorio que almacena el binario. En un `Alloy` es el
    /// miembro del que proviene el paquete.
    pub(crate) repository_id: String,
}

impl From<ferrobox_application::list_repository_artifacts::ListedArtifact> for ArtifactResponse {
    fn from(listed: ferrobox_application::list_repository_artifacts::ListedArtifact) -> Self {
        Self {
            id: listed.artifact().id().to_string(),
            name: listed.package_name().map(ToOwned::to_owned),
            version: listed.package_version().map(ToOwned::to_owned),
            filename: listed.filename().map(ToOwned::to_owned),
            checksum: listed.artifact().checksum().to_string(),
            size_bytes: listed.artifact().size_bytes(),
            yanked: listed.yanked(),
            repository_id: listed.artifact().repository_id().to_string(),
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
    pub(crate) role: RoleDto,
}

impl From<&User> for UserResponse {
    fn from(user: &User) -> Self {
        Self {
            id: user.id().to_string(),
            username: user.username().to_string(),
            role: user.role().into(),
        }
    }
}

/// Rol de autorización, tal y como viaja en la API HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RoleDto {
    /// Gestión completa.
    Admin,
    /// Puede escribir repositorios y artefactos.
    Developer,
    /// Solo lectura.
    Reader,
}

impl From<Role> for RoleDto {
    fn from(role: Role) -> Self {
        match role {
            Role::Admin => Self::Admin,
            Role::Developer => Self::Developer,
            Role::Reader => Self::Reader,
        }
    }
}

impl From<RoleDto> for Role {
    fn from(role: RoleDto) -> Self {
        match role {
            RoleDto::Admin => Self::Admin,
            RoleDto::Developer => Self::Developer,
            RoleDto::Reader => Self::Reader,
        }
    }
}

/// Cuerpo de la petición para crear un usuario.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateUserRequest {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) role: RoleDto,
}

/// Cuerpo de la petición para cambiar el rol de un usuario.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct UpdateUserRoleRequest {
    pub(crate) role: RoleDto,
}

/// Cuerpo de la petición para cambiar la contraseña del usuario
/// autenticado.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct ChangePasswordRequest {
    /// Contraseña actual, para comprobar que quien pide el cambio es
    /// el titular de la cuenta.
    pub(crate) current_password: String,
    /// Nueva contraseña en claro. El servidor la hashea antes de
    /// persistirla.
    pub(crate) new_password: String,
}

/// Información de la instancia que consume la página de configuración.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct SettingsResponse {
    /// URL pública (esquema + host + puerto, sin barra final) con la
    /// que los clientes deben hablar con esta instancia.
    pub(crate) public_base_url: String,
    /// Versión del servidor (`CARGO_PKG_VERSION`).
    pub(crate) version: String,
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

/// Estado de un ensaye.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AssayStatusDto {
    /// Inventario y consulta completados.
    Ready,
    /// Falló la consulta de vulnerabilidades.
    Failed,
    /// El ecosistema todavía no admite ensaye.
    Unsupported,
}

impl From<ferrobox_domain::assay::AssayStatus> for AssayStatusDto {
    fn from(status: ferrobox_domain::assay::AssayStatus) -> Self {
        match status {
            ferrobox_domain::assay::AssayStatus::Ready => Self::Ready,
            ferrobox_domain::assay::AssayStatus::Failed => Self::Failed,
            ferrobox_domain::assay::AssayStatus::Unsupported => Self::Unsupported,
        }
    }
}

/// Severidad de un hallazgo del ensaye.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AssaySeverityDto {
    /// Crítica.
    Critical,
    /// Alta.
    High,
    /// Media.
    Medium,
    /// Baja.
    Low,
    /// Sin puntuación.
    Unknown,
}

impl From<ferrobox_domain::assay::AssaySeverity> for AssaySeverityDto {
    fn from(severity: ferrobox_domain::assay::AssaySeverity) -> Self {
        match severity {
            ferrobox_domain::assay::AssaySeverity::Critical => Self::Critical,
            ferrobox_domain::assay::AssaySeverity::High => Self::High,
            ferrobox_domain::assay::AssaySeverity::Medium => Self::Medium,
            ferrobox_domain::assay::AssaySeverity::Low => Self::Low,
            ferrobox_domain::assay::AssaySeverity::Unknown => Self::Unknown,
        }
    }
}

/// Recuento de hallazgos por severidad.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayCountsDto {
    #[ts(type = "number")]
    pub(crate) critical: u32,
    #[ts(type = "number")]
    pub(crate) high: u32,
    #[ts(type = "number")]
    pub(crate) medium: u32,
    #[ts(type = "number")]
    pub(crate) low: u32,
    #[ts(type = "number")]
    pub(crate) unknown: u32,
}

impl From<ferrobox_domain::assay::AssayCounts> for AssayCountsDto {
    fn from(counts: ferrobox_domain::assay::AssayCounts) -> Self {
        Self {
            critical: counts.critical,
            high: counts.high,
            medium: counts.medium,
            low: counts.low,
            unknown: counts.unknown,
        }
    }
}

/// Componente del inventario de un ensaye.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayComponentResponse {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) purl: Option<String>,
    /// `root` es el paquete ensayado; `direct` una dependencia declarada;
    /// `transitive` una resuelta desde lockfile.
    pub(crate) kind: String,
}

/// Hallazgo (impureza) de un ensaye.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayFindingResponse {
    pub(crate) vulnerability_id: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) title: String,
    pub(crate) severity: AssaySeverityDto,
    pub(crate) component_name: String,
    pub(crate) component_version: String,
    pub(crate) fixed_version: Option<String>,
    pub(crate) details_url: Option<String>,
}

/// Ensaye completo de una versión de paquete.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayResponse {
    pub(crate) id: String,
    pub(crate) repository_id: String,
    pub(crate) ecosystem: PackageEcosystemDto,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) status: AssayStatusDto,
    pub(crate) scanned_at: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) counts: AssayCountsDto,
    pub(crate) components: Vec<AssayComponentResponse>,
    pub(crate) findings: Vec<AssayFindingResponse>,
}

impl From<&ferrobox_domain::assay::Assay> for AssayResponse {
    fn from(assay: &ferrobox_domain::assay::Assay) -> Self {
        Self {
            id: assay.id().to_string(),
            repository_id: assay.repository_id().to_string(),
            ecosystem: assay.coordinate().ecosystem().into(),
            name: assay.coordinate().name().as_str().to_string(),
            version: assay.coordinate().version().as_str().to_string(),
            status: assay.status().into(),
            scanned_at: assay.scanned_at().map(ToOwned::to_owned),
            error_message: assay.error_message().map(ToOwned::to_owned),
            counts: assay.counts().into(),
            components: assay
                .components()
                .iter()
                .map(|component| AssayComponentResponse {
                    name: component.name().to_string(),
                    version: component.version().to_string(),
                    purl: component.purl().map(ToOwned::to_owned),
                    kind: component.kind().as_str().to_string(),
                })
                .collect(),
            findings: assay
                .findings()
                .iter()
                .map(|finding| AssayFindingResponse {
                    vulnerability_id: finding.vulnerability_id().to_string(),
                    aliases: finding.aliases().to_vec(),
                    title: finding.title().to_string(),
                    severity: finding.severity().into(),
                    component_name: finding.component_name().to_string(),
                    component_version: finding.component_version().to_string(),
                    fixed_version: finding.fixed_version().map(ToOwned::to_owned),
                    details_url: finding.details_url().map(ToOwned::to_owned),
                })
                .collect(),
        }
    }
}

/// Coordenada a ensayar, en el cuerpo o en la query.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayLookupRequest {
    pub(crate) ecosystem: PackageEcosystemDto,
    pub(crate) name: String,
    pub(crate) version: String,
}

/// Resultado de lanzar un reensaye en lote.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayRerunResponse {
    /// Número de coordenadas distintas encoladas para reensayar.
    pub(crate) scheduled: u32,
}

/// Política de retención enviada al guardar.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct RetentionPolicyRequest {
    /// Conservar las N versiones más recientes de cada paquete.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) keep_last: Option<u32>,
    /// Conservar versiones indexadas en los últimos N días.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) keep_days: Option<u32>,
}

/// Política de retención de un repositorio.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct RetentionPolicyResponse {
    /// Conservar las N versiones más recientes de cada paquete.
    pub(crate) keep_last: Option<u32>,
    /// Conservar versiones indexadas en los últimos N días.
    pub(crate) keep_days: Option<u32>,
}

impl From<ferrobox_domain::retention::RetentionPolicy> for RetentionPolicyResponse {
    fn from(policy: ferrobox_domain::retention::RetentionPolicy) -> Self {
        Self {
            keep_last: policy.keep_last(),
            keep_days: policy.keep_days(),
        }
    }
}

/// Resultado de aplicar retención o recolectar basura.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CleanupReportResponse {
    /// Versiones o etiquetas eliminadas del índice.
    #[ts(type = "number")]
    pub(crate) dropped_versions: u64,
    /// Binarios borrados.
    #[ts(type = "number")]
    pub(crate) deleted_artifacts: u64,
    /// Bytes liberados en almacenamiento.
    #[ts(type = "number")]
    pub(crate) freed_bytes: u64,
}

impl From<ferrobox_application::retention::CleanupReport> for CleanupReportResponse {
    fn from(report: ferrobox_application::retention::CleanupReport) -> Self {
        Self {
            dropped_versions: report.dropped_versions,
            deleted_artifacts: report.deleted_artifacts,
            freed_bytes: report.freed_bytes,
        }
    }
}
