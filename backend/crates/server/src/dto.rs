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
    /// Artefactos Maven.
    Maven,
    /// Paquetes `NuGet`.
    Nuget,
    /// Módulos `Go`.
    Go,
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
            PackageEcosystem::Maven => Self::Maven,
            PackageEcosystem::Nuget => Self::Nuget,
            PackageEcosystem::Go => Self::Go,
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
            PackageEcosystemDto::Maven => Self::Maven,
            PackageEcosystemDto::Nuget => Self::Nuget,
            PackageEcosystemDto::Go => Self::Go,
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

/// Acceso efectivo de un usuario a un repositorio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RepositoryAccessDto {
    /// Puede listar y descargar.
    Read,
    /// Puede publicar, borrar y configurar el repositorio.
    Write,
}

impl From<ferrobox_domain::group::RepositoryAccess> for RepositoryAccessDto {
    fn from(access: ferrobox_domain::group::RepositoryAccess) -> Self {
        match access {
            ferrobox_domain::group::RepositoryAccess::Read => Self::Read,
            ferrobox_domain::group::RepositoryAccess::Write => Self::Write,
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
    /// Acceso del usuario autenticado a este repositorio.
    pub(crate) access: RepositoryAccessDto,
    /// `true` si hay grupos asignados; entonces solo esos grupos (y
    /// los administradores) pueden verlo.
    pub(crate) restricted: bool,
    /// Horas entre refrescos programados del *upstream*. `null` = apagado.
    #[ts(type = "number | null")]
    pub(crate) prefetch_interval_hours: Option<u32>,
    /// Último refresco programado, RFC 3339, o `null`.
    pub(crate) last_prefetch_at: Option<String>,
}

impl RepositoryResponse {
    pub(crate) fn from_repository(
        repository: &Repository,
        access: ferrobox_domain::group::RepositoryAccess,
        restricted: bool,
    ) -> Self {
        Self {
            id: repository.id().to_string(),
            name: repository.name().to_string(),
            kind: repository.kind().into(),
            ecosystem: repository.ecosystem().into(),
            access: access.into(),
            restricted,
            prefetch_interval_hours: repository.prefetch_interval_hours(),
            last_prefetch_at: repository
                .last_prefetch_at()
                .map(|at| at.to_rfc3339()),
        }
    }
}

/// Cuerpo para el intervalo de refresco de un `Mirror`.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetMirrorScheduleRequest {
    /// Horas entre refrescos. `null` o `0` apaga el cron.
    #[serde(default)]
    #[ts(optional)]
    #[ts(type = "number")]
    pub(crate) prefetch_interval_hours: Option<u32>,
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
    /// `true` si hay una firma Cosign / Notation enlazada a este artefacto.
    pub(crate) signed: bool,
    /// `true` si alguna firma Cosign verifica contra las claves del
    /// repositorio.
    pub(crate) verified: bool,
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
            signed: listed.signed(),
            verified: listed.verified(),
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
    /// Correo de la cuenta, o `null` si el administrador de arranque
    /// no tiene uno.
    pub(crate) email: Option<String>,
    pub(crate) role: RoleDto,
    /// `true` si la cuenta está vinculada a un emisor `OIDC`.
    pub(crate) sso: bool,
    /// `true` si es una cuenta robot (CI).
    pub(crate) robot: bool,
}

impl From<&User> for UserResponse {
    fn from(user: &User) -> Self {
        Self {
            id: user.id().to_string(),
            username: user.username().to_string(),
            email: user.email().map(ToString::to_string),
            role: user.role().into(),
            sso: user.is_sso_linked(),
            robot: user.is_robot(),
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
    pub(crate) email: String,
    pub(crate) password: String,
    pub(crate) role: RoleDto,
}

/// Cuerpo de la petición para crear una cuenta robot.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRobotRequest {
    pub(crate) username: String,
    pub(crate) role: RoleDto,
    pub(crate) token_name: String,
    /// Caducidad RFC 3339 del token inicial. `null` = no caduca.
    #[ts(optional)]
    pub(crate) expires_at: Option<String>,
}

/// Respuesta al crear un robot: la cuenta y el secreto del token inicial.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRobotResponse {
    pub(crate) user: UserResponse,
    pub(crate) token: ApiTokenCreatedResponse,
}

/// Cuerpo de la petición para que un Admin restablezca la contraseña de
/// otra cuenta.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct ResetUserPasswordRequest {
    pub(crate) password: String,
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
    /// `true` si hay un `IdP` `OIDC` configurado.
    pub(crate) oidc_enabled: bool,
    /// Emisor `OIDC`, si el SSO está activo.
    pub(crate) oidc_issuer: Option<String>,
}

/// Estado público del inicio de sesión federado (no exige sesión).
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct OidcStatusResponse {
    /// `true` si el botón SSO debe mostrarse.
    pub(crate) enabled: bool,
    /// Emisor, si está habilitado.
    pub(crate) issuer: Option<String>,
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
    /// Caducidad RFC 3339. `null` o ausente = no caduca.
    #[ts(optional)]
    pub(crate) expires_at: Option<String>,
}

/// Representación de un token de API (sin secreto) en listados.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ApiTokenResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) created_at: String,
    /// Caducidad RFC 3339, o `null` si no caduca.
    pub(crate) expires_at: Option<String>,
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
    /// Caducidad RFC 3339, o `null` si no caduca.
    pub(crate) expires_at: Option<String>,
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
    /// Encolado o ejecutándose.
    Running,
}

impl From<ferrobox_domain::assay::AssayStatus> for AssayStatusDto {
    fn from(status: ferrobox_domain::assay::AssayStatus) -> Self {
        match status {
            ferrobox_domain::assay::AssayStatus::Ready => Self::Ready,
            ferrobox_domain::assay::AssayStatus::Failed => Self::Failed,
            ferrobox_domain::assay::AssayStatus::Unsupported => Self::Unsupported,
            ferrobox_domain::assay::AssayStatus::Running => Self::Running,
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
    /// Licencias declaradas en el manifiesto o en metadatos de distro.
    pub(crate) licenses: Vec<String>,
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
                    licenses: component.licenses().to_vec(),
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

/// Tope de almacenamiento enviado al guardar.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct QuotaRequest {
    /// Tope en bytes. Ausente o `null` = ilimitado.
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub(crate) limit_bytes: Option<u64>,
}

/// Uso y tope de almacenamiento de un repositorio.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct QuotaResponse {
    /// Tope en bytes, o `null` si no hay límite.
    #[ts(type = "number | null")]
    pub(crate) limit_bytes: Option<u64>,
    /// Bytes ocupados por todos los binarios del repositorio.
    #[ts(type = "number")]
    pub(crate) used_bytes: u64,
}

impl From<ferrobox_application::quota::QuotaSnapshot> for QuotaResponse {
    fn from(snapshot: ferrobox_application::quota::QuotaSnapshot) -> Self {
        Self {
            limit_bytes: snapshot.limit_bytes,
            used_bytes: snapshot.used_bytes,
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

/// Una fila del dry-run o del resultado real.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CleanupItemResponse {
    /// Repositorio al que pertenece la fila.
    pub(crate) repository: String,
    /// Nombre del paquete, o vacío si es un binario huérfano.
    pub(crate) name: String,
    /// Versión, etiqueta, digest o identificador del binario.
    pub(crate) version: String,
    /// Tamaño del binario, si se conoce.
    #[ts(type = "number")]
    pub(crate) size_bytes: u64,
    /// Por qué se incluye en la limpieza.
    pub(crate) reason: String,
}

/// Simulación o aplicación de retención/GC.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CleanupPreviewResponse {
    /// `true` si no se ha borrado nada.
    pub(crate) dry_run: bool,
    /// Versiones o etiquetas eliminadas (o que se eliminarían) del índice.
    #[ts(type = "number")]
    pub(crate) dropped_versions: u64,
    /// Binarios borrados (o que se borrarían).
    #[ts(type = "number")]
    pub(crate) deleted_artifacts: u64,
    /// Bytes liberados (o que se liberarían).
    #[ts(type = "number")]
    pub(crate) freed_bytes: u64,
    /// Detalle para revisar antes de aplicar.
    pub(crate) items: Vec<CleanupItemResponse>,
}

impl From<ferrobox_application::retention::CleanupPreview> for CleanupPreviewResponse {
    fn from(preview: ferrobox_application::retention::CleanupPreview) -> Self {
        Self {
            dry_run: preview.dry_run,
            dropped_versions: preview.report.dropped_versions,
            deleted_artifacts: preview.report.deleted_artifacts,
            freed_bytes: preview.report.freed_bytes,
            items: preview
                .items
                .into_iter()
                .map(|item| CleanupItemResponse {
                    repository: item.repository,
                    name: item.name,
                    version: item.version,
                    size_bytes: item.size_bytes,
                    reason: item.reason,
                })
                .collect(),
        }
    }
}

/// Una coincidencia de la búsqueda global de paquetes.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PackageSearchHitResponse {
    /// Repositorio donde está indexado el paquete.
    pub(crate) repository_id: String,
    /// Nombre del repositorio.
    pub(crate) repository_name: String,
    /// Forge o Mirror.
    pub(crate) kind: RepositoryKindDto,
    /// Ecosistema del paquete.
    pub(crate) ecosystem: PackageEcosystemDto,
    /// Nombre del paquete.
    pub(crate) name: String,
    /// Versión mostrada.
    pub(crate) version: String,
    /// `true` si esa versión está yankada.
    pub(crate) yanked: bool,
}

impl From<ferrobox_application::search_packages::PackageSearchHit> for PackageSearchHitResponse {
    fn from(hit: ferrobox_application::search_packages::PackageSearchHit) -> Self {
        Self {
            repository_id: hit.repository_id.to_string(),
            repository_name: hit.repository_name.to_string(),
            kind: (&hit.repository_kind).into(),
            ecosystem: hit.ecosystem.into(),
            name: hit.name,
            version: hit.version,
            yanked: hit.yanked,
        }
    }
}

/// Resultado de buscar paquetes en toda la instancia.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct SearchResponse {
    /// Coincidencias, ya recortadas al límite pedido.
    pub(crate) hits: Vec<PackageSearchHitResponse>,
}

/// Resumen de un grupo en listados.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupSummaryResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    #[ts(type = "number")]
    pub(crate) member_count: usize,
    #[ts(type = "number")]
    pub(crate) repository_count: usize,
    /// Nombres de usuario de los miembros, ordenados.
    pub(crate) member_names: Vec<String>,
    /// Nombres de los repositorios asignados, ordenados.
    pub(crate) repository_names: Vec<String>,
}

/// Grupo al que pertenece el usuario autenticado.
///
/// No incluye el resto de miembros: eso solo lo ve un administrador
/// en la ficha del grupo.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct MyGroupMembershipResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) repositories: Vec<GroupRepositoryGrantResponse>,
}

/// Repositorio asignado a un grupo, con el rol en ese repositorio.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupRepositoryGrantResponse {
    pub(crate) repository_id: String,
    pub(crate) repository_name: String,
    pub(crate) role: RoleDto,
}

/// Detalle de un grupo: miembros y repositorios.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupDetailResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) members: Vec<UserResponse>,
    pub(crate) repositories: Vec<GroupRepositoryGrantResponse>,
}

/// Cuerpo de la petición para crear un grupo.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateGroupRequest {
    pub(crate) name: String,
}

/// Cuerpo de la petición para sustituir los miembros de un grupo.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetGroupMembersRequest {
    pub(crate) user_ids: Vec<String>,
}

/// Asignación de un repositorio a un grupo.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupRepositoryGrantRequest {
    pub(crate) repository_id: String,
    pub(crate) role: RoleDto,
}

/// Cuerpo de la petición para sustituir los repositorios de un grupo.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetGroupRepositoriesRequest {
    pub(crate) grants: Vec<GroupRepositoryGrantRequest>,
}

/// Grupo con acceso a un repositorio.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct RepositoryAccessGrantResponse {
    pub(crate) group_id: String,
    pub(crate) group_name: String,
    pub(crate) role: RoleDto,
}

/// Asignación de un grupo a un repositorio.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct RepositoryAccessGrantRequest {
    pub(crate) group_id: String,
    pub(crate) role: RoleDto,
}

/// Cuerpo de la petición para sustituir los grupos de un repositorio.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetRepositoryAccessRequest {
    pub(crate) grants: Vec<RepositoryAccessGrantRequest>,
}

/// Cuerpo de la petición para copiar una versión de un Forge a otro.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct PromotePackageRequest {
    /// Identificador del Forge destino, del mismo ecosistema.
    pub(crate) target_repository_id: String,
    /// Nombre del paquete. Obligatorio salvo en repositorios genéricos.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) name: Option<String>,
    /// Versión, etiqueta OCI o referencia Conan (`0.1@_/_`).
    #[serde(default)]
    #[ts(optional)]
    pub(crate) version: Option<String>,
    /// Identificador del binario, para repositorios genéricos.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) artifact_id: Option<String>,
    /// Si es `true`, la copia conserva el yank del origen.
    #[serde(default)]
    pub(crate) preserve_yanked: bool,
}

/// Cuerpo para calentar la caché de un `Mirror`.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct PrefetchPackageRequest {
    /// Nombre del paquete, módulo o imagen.
    pub(crate) name: String,
    /// Versión, etiqueta OCI o digest. Vacío = solo índice.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) version: Option<String>,
}

/// Resultado de importar un archivo portable.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ImportRepositoryResponse {
    #[ts(type = "number")]
    pub(crate) packages_imported: u32,
    #[ts(type = "number")]
    pub(crate) artifacts_imported: u32,
    #[ts(type = "number")]
    pub(crate) skipped: u32,
    #[ts(type = "number")]
    pub(crate) bytes_copied: u64,
}

/// Resultado del prefetch.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PrefetchPackageResponse {
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    pub(crate) indexed: bool,
    pub(crate) downloaded: bool,
}

/// Resultado de copiar una versión a otro Forge.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PromotePackageResponse {
    /// Nombre copiado, si el ecosistema indexa por coordenada.
    pub(crate) name: Option<String>,
    /// Versión copiada.
    pub(crate) version: Option<String>,
    /// Binarios nuevos creados en el destino.
    #[ts(type = "number")]
    pub(crate) artifacts_copied: u32,
    /// Bytes escritos en el destino.
    #[ts(type = "number")]
    pub(crate) bytes_copied: u64,
}

/// Evento de un aviso HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) enum WebhookEventDto {
    /// Ensaye terminado.
    #[serde(rename = "assay.completed")]
    AssayCompleted,
    /// Versión publicada o cacheada.
    #[serde(rename = "package.published")]
    PackagePublished,
}

impl From<ferrobox_domain::webhook::WebhookEvent> for WebhookEventDto {
    fn from(event: ferrobox_domain::webhook::WebhookEvent) -> Self {
        match event {
            ferrobox_domain::webhook::WebhookEvent::AssayCompleted => Self::AssayCompleted,
            ferrobox_domain::webhook::WebhookEvent::PackagePublished => Self::PackagePublished,
        }
    }
}

impl From<WebhookEventDto> for ferrobox_domain::webhook::WebhookEvent {
    fn from(event: WebhookEventDto) -> Self {
        match event {
            WebhookEventDto::AssayCompleted => Self::AssayCompleted,
            WebhookEventDto::PackagePublished => Self::PackagePublished,
        }
    }
}

/// Aviso HTTP de un repositorio.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct WebhookResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) url: String,
    /// `true` si hay un secreto HMAC configurado. El valor nunca se
    /// devuelve.
    pub(crate) has_secret: bool,
    pub(crate) events: Vec<WebhookEventDto>,
    pub(crate) enabled: bool,
}

impl From<&ferrobox_domain::webhook::Webhook> for WebhookResponse {
    fn from(webhook: &ferrobox_domain::webhook::Webhook) -> Self {
        Self {
            id: webhook.id().to_string(),
            name: webhook.name().to_string(),
            url: webhook.url().to_string(),
            has_secret: webhook.secret().is_some(),
            events: webhook
                .events()
                .iter()
                .copied()
                .map(WebhookEventDto::from)
                .collect(),
            enabled: webhook.enabled(),
        }
    }
}

/// Envío de un aviso HTTP.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct WebhookDeliveryResponse {
    pub(crate) id: String,
    pub(crate) event: String,
    pub(crate) status: String,
    #[ts(type = "number | null")]
    pub(crate) http_status: Option<u16>,
    pub(crate) error: Option<String>,
    pub(crate) created_at: String,
}

impl From<&ferrobox_domain::webhook::WebhookDelivery> for WebhookDeliveryResponse {
    fn from(delivery: &ferrobox_domain::webhook::WebhookDelivery) -> Self {
        Self {
            id: delivery.id().to_string(),
            event: delivery.event().to_string(),
            status: delivery.status().as_str().to_string(),
            http_status: delivery.http_status(),
            error: delivery.error().map(str::to_string),
            created_at: delivery.created_at().to_string(),
        }
    }
}

/// Cuerpo para crear un aviso HTTP.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateWebhookRequest {
    pub(crate) name: String,
    pub(crate) url: String,
    #[serde(default)]
    #[ts(optional)]
    pub(crate) secret: Option<String>,
    pub(crate) events: Vec<WebhookEventDto>,
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Momento de evaluación de una política de admisión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionWhenDto {
    /// Al resolver un manifiesto o un paquete para instalarlo.
    Pull,
}

impl AdmissionWhenDto {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pull => "pull",
        }
    }
}

impl From<ferrobox_domain::admission::AdmissionWhen> for AdmissionWhenDto {
    fn from(value: ferrobox_domain::admission::AdmissionWhen) -> Self {
        match value {
            ferrobox_domain::admission::AdmissionWhen::Pull => Self::Pull,
        }
    }
}

/// Condición que dispara el efecto de una política de admisión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionPredicateDto {
    /// El artefacto no tiene una firma Cosign / Notation enlazada.
    NotSigned,
    /// El artefacto no tiene una firma Cosign válida contra las claves.
    NotVerified,
}

impl AdmissionPredicateDto {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotSigned => "not_signed",
            Self::NotVerified => "not_verified",
        }
    }
}

impl From<ferrobox_domain::admission::AdmissionPredicate> for AdmissionPredicateDto {
    fn from(value: ferrobox_domain::admission::AdmissionPredicate) -> Self {
        match value {
            ferrobox_domain::admission::AdmissionPredicate::NotSigned => Self::NotSigned,
            ferrobox_domain::admission::AdmissionPredicate::NotVerified => Self::NotVerified,
        }
    }
}

/// Qué hacer si la condición de admisión se cumple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionEffectDto {
    /// Bloquea la operación.
    Deny,
    /// Deja pasar y solo deja constancia.
    Warn,
}

impl AdmissionEffectDto {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Warn => "warn",
        }
    }
}

impl From<ferrobox_domain::admission::AdmissionEffect> for AdmissionEffectDto {
    fn from(value: ferrobox_domain::admission::AdmissionEffect) -> Self {
        match value {
            ferrobox_domain::admission::AdmissionEffect::Deny => Self::Deny,
            ferrobox_domain::admission::AdmissionEffect::Warn => Self::Warn,
        }
    }
}

/// Política de admisión enviada al guardar o simular.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPolicyRequest {
    /// `true` si la regla se aplica en el pull.
    pub(crate) enabled: bool,
    /// Momento de evaluación.
    pub(crate) when: AdmissionWhenDto,
    /// Condición que dispara el efecto.
    pub(crate) predicate: AdmissionPredicateDto,
    /// Efecto si la condición se cumple.
    pub(crate) effect: AdmissionEffectDto,
    /// PEM de claves públicas Cosign (`cosign generate-key-pair`).
    #[serde(default)]
    pub(crate) public_keys_pem: String,
}

/// Política de admisión de un repositorio.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPolicyResponse {
    /// `true` si la regla se aplica en el pull.
    pub(crate) enabled: bool,
    /// Momento de evaluación.
    pub(crate) when: AdmissionWhenDto,
    /// Condición que dispara el efecto.
    pub(crate) predicate: AdmissionPredicateDto,
    /// Efecto si la condición se cumple.
    pub(crate) effect: AdmissionEffectDto,
    /// PEM de claves públicas Cosign del repositorio.
    pub(crate) public_keys_pem: String,
}

impl From<ferrobox_ports::admission_store::AdmissionRecord> for AdmissionPolicyResponse {
    fn from(record: ferrobox_ports::admission_store::AdmissionRecord) -> Self {
        Self {
            enabled: record.policy.enabled(),
            when: record.policy.when().into(),
            predicate: record.policy.predicate().into(),
            effect: record.policy.effect().into(),
            public_keys_pem: record.public_keys_pem,
        }
    }
}

/// Un artefacto que la política tocaría en un pull.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPreviewItemResponse {
    /// Nombre del paquete o de la imagen.
    pub(crate) name: String,
    /// Versión o etiqueta.
    pub(crate) version: String,
    /// `deny` o `warn`.
    pub(crate) effect: AdmissionEffectDto,
    /// Motivo legible.
    pub(crate) reason: String,
}

/// Resultado de simular la política contra el inventario.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPreviewResponse {
    /// Artefactos que disparan la condición.
    pub(crate) matches: Vec<AdmissionPreviewItemResponse>,
    /// Versiones listadas que no disparan la condición.
    #[ts(type = "number")]
    pub(crate) allowed: usize,
}

impl From<ferrobox_application::admission::AdmissionPreview> for AdmissionPreviewResponse {
    fn from(preview: ferrobox_application::admission::AdmissionPreview) -> Self {
        Self {
            matches: preview
                .matches
                .into_iter()
                .map(|item| AdmissionPreviewItemResponse {
                    name: item.name,
                    version: item.version,
                    effect: item.effect.into(),
                    reason: item.reason,
                })
                .collect(),
            allowed: preview.allowed,
        }
    }
}

/// Un aviso o una denegación registrados en un pull.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionEventResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) reference: String,
    pub(crate) effect: AdmissionEffectDto,
    pub(crate) reason: String,
    pub(crate) created_at: String,
}

impl From<ferrobox_domain::admission::AdmissionEvent> for AdmissionEventResponse {
    fn from(event: ferrobox_domain::admission::AdmissionEvent) -> Self {
        Self {
            id: event.id().to_string(),
            name: event.name().to_string(),
            reference: event.reference().to_string(),
            effect: event.effect().into(),
            reason: event.reason().to_string(),
            created_at: event.created_at().to_string(),
        }
    }
}

/// Una fila del registro de auditoría.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AuditEventResponse {
    pub(crate) id: String,
    pub(crate) actor: String,
    pub(crate) action: String,
    pub(crate) target_kind: String,
    pub(crate) target: String,
    pub(crate) detail: String,
    pub(crate) created_at: String,
}

impl From<ferrobox_domain::audit::AuditEvent> for AuditEventResponse {
    fn from(event: ferrobox_domain::audit::AuditEvent) -> Self {
        Self {
            id: event.id().to_string(),
            actor: event.actor_username().to_string(),
            action: event.action().as_str().to_string(),
            target_kind: event.target_kind().as_str().to_string(),
            target: event.target().to_string(),
            detail: event.detail().to_string(),
            created_at: event.created_at().to_string(),
        }
    }
}

/// Cuerpo para actualizar un aviso HTTP.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct UpdateWebhookRequest {
    pub(crate) name: String,
    pub(crate) url: String,
    /// Ausente: conserva el secreto. Cadena vacía: lo borra.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) secret: Option<String>,
    pub(crate) events: Vec<WebhookEventDto>,
    pub(crate) enabled: bool,
}
