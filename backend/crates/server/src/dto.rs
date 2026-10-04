//! HTTP API request and response bodies.
//!
//! All derive [`ts_rs::TS`] with `#[ts(export)]`: `cargo test -p
//! ferrobox-server` automatically regenerates the equivalent TypeScript
//! types in `frontend/src/api/generated/`.

use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_domain::user::{Role, User};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Body of an error response.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

/// Package ecosystem of a repository, as it travels in the HTTP API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PackageEcosystemDto {
    /// Binary artifacts with no specific package format.
    Generic,
    /// Rust crates.
    Cargo,
    /// Node.js packages.
    Npm,
    /// Python packages.
    Pypi,
    /// Artifacts conforming to the OCI specification.
    Oci,
    /// Helm Charts.
    Helm,
    /// Conan C/C++ packages.
    Conan,
    /// Maven artifacts.
    Maven,
    /// `NuGet` packages.
    Nuget,
    /// `Go` modules.
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

/// Origin and storage strategy of a repository, as it travels in the
/// HTTP API. Serialized as a union discriminated by the `type` field,
/// so the generated TypeScript client gets an exhaustive union type
/// instead of an opaque string.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum RepositoryKindDto {
    /// Own storage.
    Forge,
    /// Cached replica of an external source.
    Mirror {
        /// Base URL of the replicated external repository.
        upstream: String,
    },
    /// Aggregation of other repositories.
    Alloy {
        /// Identifiers of the aggregated repositories.
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

/// Effective access of a user to a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RepositoryAccessDto {
    /// Can list and download.
    Read,
    /// Can publish, delete, and configure the repository.
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

/// Representation of a repository in API responses.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct RepositoryResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: RepositoryKindDto,
    pub(crate) ecosystem: PackageEcosystemDto,
    /// Access of the authenticated user to this repository.
    pub(crate) access: RepositoryAccessDto,
    /// `true` if groups are assigned; then only those groups (and
    /// administrators) can see it.
    pub(crate) restricted: bool,
    /// Hours between scheduled *upstream* refreshes. `null` = off.
    #[ts(type = "number | null")]
    pub(crate) prefetch_interval_hours: Option<u32>,
    /// Last scheduled refresh, RFC 3339, or `null`.
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
            last_prefetch_at: repository.last_prefetch_at().map(|at| at.to_rfc3339()),
        }
    }
}

/// Body for a `Mirror` refresh interval.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetMirrorScheduleRequest {
    /// Hours between refreshes. `null` or `0` turns the cron off.
    #[serde(default)]
    #[ts(optional)]
    #[ts(type = "number")]
    pub(crate) prefetch_interval_hours: Option<u32>,
}

/// Whether a mirror has an upstream secret. The secret itself is never returned.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct MirrorUpstreamAuthResponse {
    /// `true` when a secret is stored.
    pub(crate) configured: bool,
    /// Username, or empty when the secret is a bearer token.
    pub(crate) username: String,
}

/// Body that replaces a mirror's upstream credential.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetMirrorUpstreamAuthRequest {
    /// Empty means the secret is sent as `Authorization: Bearer`.
    #[serde(default)]
    pub(crate) username: String,
    /// Required on every save. Never returned by a later read.
    pub(crate) secret: String,
}

/// Request body to create a repository.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRepositoryRequest {
    pub(crate) name: String,
    pub(crate) ecosystem: PackageEcosystemDto,
    /// Repository type. If omitted, a `Forge` is created.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) kind: Option<CreateRepositoryKindDto>,
}

/// Repository type requested at creation.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum CreateRepositoryKindDto {
    /// Own storage.
    #[default]
    Forge,
    /// Cached replica of an *upstream*.
    Mirror {
        /// Base URL of the remote sparse index (e.g. `https://index.crates.io/`).
        upstream: String,
    },
    /// Aggregation of other `Forge` or `Mirror` repositories.
    Alloy {
        /// Identifiers of the member repositories, in resolution
        /// order.
        members: Vec<String>,
    },
}

/// Response after creating a repository successfully.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRepositoryResponse {
    pub(crate) id: String,
}

/// Request body to update the members of an `Alloy`.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct UpdateAlloyMembersRequest {
    /// Identifiers of the member repositories, in resolution
    /// order.
    pub(crate) members: Vec<String>,
}

/// Representation of an artifact in API responses.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ArtifactResponse {
    pub(crate) id: String,
    /// Package name (crate, etc.) if the index knows it.
    pub(crate) name: Option<String>,
    /// Package version if the index knows it.
    pub(crate) version: Option<String>,
    /// Filename in the index, if the ecosystem distinguishes it
    /// (`Conan` recipe, `PyPI` sdist/wheel, etc.).
    pub(crate) filename: Option<String>,
    pub(crate) checksum: String,
    #[ts(type = "number")]
    pub(crate) size_bytes: u64,
    /// `true` if the index marks this version as *yanked*.
    pub(crate) yanked: bool,
    /// `true` if a Cosign / Notation signature is linked to this artifact.
    pub(crate) signed: bool,
    /// `true` if some Cosign signature verifies against the repository
    /// keys.
    pub(crate) verified: bool,
    /// `false` when the version is indexed and the binary is not cached yet.
    pub(crate) cached: bool,
    /// Repository that stores the binary. In an `Alloy` this is the
    /// member the package came from.
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
            cached: listed.cached(),
            repository_id: listed.artifact().repository_id().to_string(),
        }
    }
}

/// Response after publishing an artifact successfully.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PublishResponse {
    pub(crate) id: String,
}

/// Sign-in request body.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct LoginRequest {
    pub(crate) username: String,
    pub(crate) password: String,
}

/// Representation of a user in API responses.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct UserResponse {
    pub(crate) id: String,
    pub(crate) username: String,
    /// Account email, or `null` if the bootstrap administrator has
    /// none.
    pub(crate) email: Option<String>,
    pub(crate) role: RoleDto,
    /// `true` if the account is linked to an `OIDC` issuer.
    pub(crate) sso: bool,
    /// `true` if this is a robot account (CI).
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

/// Authorization role, as it travels in the HTTP API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RoleDto {
    /// Full management.
    Admin,
    /// Can write repositories and artifacts.
    Developer,
    /// Read only.
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

/// Request body to create a user.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateUserRequest {
    pub(crate) username: String,
    pub(crate) email: String,
    pub(crate) password: String,
    pub(crate) role: RoleDto,
}

/// Request body to create a robot account.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRobotRequest {
    pub(crate) username: String,
    pub(crate) role: RoleDto,
    pub(crate) token_name: String,
    /// RFC 3339 expiry of the initial token. `null` = does not expire.
    #[ts(optional)]
    pub(crate) expires_at: Option<String>,
}

/// Response after creating a robot: the account and the initial token secret.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateRobotResponse {
    pub(crate) user: UserResponse,
    pub(crate) token: ApiTokenCreatedResponse,
}

/// Request body for an Admin to reset another account's password.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct ResetUserPasswordRequest {
    pub(crate) password: String,
}

/// Request body to change a user's role.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct UpdateUserRoleRequest {
    pub(crate) role: RoleDto,
}

/// Request body to change the authenticated user's password.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct ChangePasswordRequest {
    /// Current password, to check that the requester is the account
    /// holder.
    pub(crate) current_password: String,
    /// New password in clear text. The server hashes it before
    /// persisting it.
    pub(crate) new_password: String,
}

/// Instance information consumed by the settings page.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct SettingsResponse {
    /// Public URL (scheme + host + port, no trailing slash) that
    /// clients should use to talk to this instance.
    pub(crate) public_base_url: String,
    /// Server version (`CARGO_PKG_VERSION`).
    pub(crate) version: String,
    /// `true` if an `OIDC` `IdP` is configured.
    pub(crate) oidc_enabled: bool,
    /// `OIDC` issuer, if SSO is active.
    pub(crate) oidc_issuer: Option<String>,
}

/// Public status of federated sign-in (does not require a session).
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct OidcStatusResponse {
    /// `true` if the SSO button should be shown.
    pub(crate) enabled: bool,
    /// Issuer, if enabled.
    pub(crate) issuer: Option<String>,
}

/// Response after a successful sign-in.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct LoginResponse {
    /// Session token secret (shown only once).
    pub(crate) token: String,
    pub(crate) user: UserResponse,
}

/// Request body to create an API token.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateApiTokenRequest {
    pub(crate) name: String,
    /// RFC 3339 expiry. `null` or absent = does not expire.
    #[ts(optional)]
    pub(crate) expires_at: Option<String>,
    /// `read` and/or `write`. Empty or omitted = unrestricted.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) scopes: Option<Vec<String>>,
    /// Repository ids. Empty or omitted = every repository the user can access.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) repository_ids: Option<Vec<String>>,
}

/// Representation of an API token (without the secret) in listings.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ApiTokenResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) created_at: String,
    /// RFC 3339 expiry, or `null` if it does not expire.
    pub(crate) expires_at: Option<String>,
    /// Stored scopes. Empty = unrestricted.
    pub(crate) scopes: Vec<String>,
    /// Repository ids. Empty = every repository the user can access.
    pub(crate) repository_ids: Vec<String>,
}

/// Response after creating an API token: includes the secret only once.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ApiTokenCreatedResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    /// Secret in clear text. Exposed only in this response.
    pub(crate) token: String,
    /// RFC 3339 expiry, or `null` if it does not expire.
    pub(crate) expires_at: Option<String>,
    /// Stored scopes. Empty = unrestricted.
    pub(crate) scopes: Vec<String>,
    /// Repository ids. Empty = every repository the user can access.
    pub(crate) repository_ids: Vec<String>,
}

/// Labels stored on a token for the API (`read`, `write`).
pub(crate) fn token_scope_labels(token: &ferrobox_domain::api_token::ApiToken) -> Vec<String> {
    token
        .scopes()
        .as_labels()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Repository ids stored on a token. Empty = unrestricted.
pub(crate) fn token_repository_ids(token: &ferrobox_domain::api_token::ApiToken) -> Vec<String> {
    token
        .repositories()
        .as_ids()
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Status of an assay.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AssayStatusDto {
    /// Inventory and lookup completed.
    Ready,
    /// Vulnerability lookup failed.
    Failed,
    /// The ecosystem does not yet support assay.
    Unsupported,
    /// Queued or running.
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

/// Severity of an assay finding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AssaySeverityDto {
    /// Critical.
    Critical,
    /// High.
    High,
    /// Medium.
    Medium,
    /// Low.
    Low,
    /// No score.
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

/// Finding counts by severity.
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

/// Component of an assay inventory.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayComponentResponse {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) purl: Option<String>,
    /// `root` is the assayed package; `direct` a declared dependency;
    /// `transitive` one resolved from a lockfile.
    pub(crate) kind: String,
    /// Licenses declared in the manifest or in distro metadata.
    pub(crate) licenses: Vec<String>,
}

/// Finding (impurity) of an assay.
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

/// Full assay of a package version.
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

/// Coordinate to assay, in the body or in the query.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayLookupRequest {
    pub(crate) ecosystem: PackageEcosystemDto,
    pub(crate) name: String,
    pub(crate) version: String,
}

/// Result of launching a batch re-assay.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AssayRerunResponse {
    /// Number of distinct coordinates queued to re-assay.
    pub(crate) scheduled: u32,
}

/// Retention policy sent when saving.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct RetentionPolicyRequest {
    /// Keep the N most recent versions of each package.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) keep_last: Option<u32>,
    /// Keep versions indexed in the last N days.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) keep_days: Option<u32>,
}

/// Retention policy of a repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct RetentionPolicyResponse {
    /// Keep the N most recent versions of each package.
    pub(crate) keep_last: Option<u32>,
    /// Keep versions indexed in the last N days.
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

/// Storage cap sent when saving.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct QuotaRequest {
    /// Cap in bytes. Absent or `null` = unlimited.
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub(crate) limit_bytes: Option<u64>,
}

/// Storage use and cap of a repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct QuotaResponse {
    /// Cap in bytes, or `null` if there is no limit.
    #[ts(type = "number | null")]
    pub(crate) limit_bytes: Option<u64>,
    /// Bytes occupied by every binary in the repository.
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

/// Instance-wide storage occupied by published binaries.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct StorageResponse {
    /// Sum of every artifact on the instance.
    #[ts(type = "number")]
    pub(crate) used_bytes: u64,
}

/// WORM lock sent when saving.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct WormRequest {
    /// `true` rejects delete, yank, retention apply, and per-repository GC.
    pub(crate) enabled: bool,
}

/// WORM lock of a repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct WormResponse {
    /// `true` when the repository must stay immutable.
    pub(crate) enabled: bool,
}

impl From<ferrobox_domain::worm::WormPolicy> for WormResponse {
    fn from(policy: ferrobox_domain::worm::WormPolicy) -> Self {
        Self {
            enabled: policy.enabled(),
        }
    }
}

/// Result of applying retention or collecting garbage.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CleanupReportResponse {
    /// Versions or tags removed from the index.
    #[ts(type = "number")]
    pub(crate) dropped_versions: u64,
    /// Binaries deleted.
    #[ts(type = "number")]
    pub(crate) deleted_artifacts: u64,
    /// Bytes freed in storage.
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

/// A row from the dry-run or from the real result.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CleanupItemResponse {
    /// Repository the row belongs to.
    pub(crate) repository: String,
    /// Package name, or empty if this is an orphan binary.
    pub(crate) name: String,
    /// Version, tag, digest, or binary identifier.
    pub(crate) version: String,
    /// Binary size, if known.
    #[ts(type = "number")]
    pub(crate) size_bytes: u64,
    /// Why it is included in the cleanup.
    pub(crate) reason: String,
}

/// Simulation or application of retention/GC.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct CleanupPreviewResponse {
    /// `true` if nothing has been deleted.
    pub(crate) dry_run: bool,
    /// Versions or tags removed (or that would be removed) from the index.
    #[ts(type = "number")]
    pub(crate) dropped_versions: u64,
    /// Binaries deleted (or that would be deleted).
    #[ts(type = "number")]
    pub(crate) deleted_artifacts: u64,
    /// Bytes freed (or that would be freed).
    #[ts(type = "number")]
    pub(crate) freed_bytes: u64,
    /// Detail to review before applying.
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

/// A match from the global package search.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PackageSearchHitResponse {
    /// Repository where the package is indexed.
    pub(crate) repository_id: String,
    /// Repository name.
    pub(crate) repository_name: String,
    /// Forge or Mirror.
    pub(crate) kind: RepositoryKindDto,
    /// Package ecosystem.
    pub(crate) ecosystem: PackageEcosystemDto,
    /// Package name.
    pub(crate) name: String,
    /// Displayed version.
    pub(crate) version: String,
    /// `true` if that version is yanked.
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

/// Result of searching packages across the instance.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct SearchResponse {
    /// Matches, already trimmed to the requested limit.
    pub(crate) hits: Vec<PackageSearchHitResponse>,
}

/// Group summary in listings.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupSummaryResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    #[ts(type = "number")]
    pub(crate) member_count: usize,
    #[ts(type = "number")]
    pub(crate) repository_count: usize,
    /// Member usernames, sorted.
    pub(crate) member_names: Vec<String>,
    /// Assigned repository names, sorted.
    pub(crate) repository_names: Vec<String>,
}

/// Group the authenticated user belongs to.
///
/// Does not include the other members: only an administrator sees
/// those on the group detail page.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct MyGroupMembershipResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) repositories: Vec<GroupRepositoryGrantResponse>,
}

/// Repository assigned to a group, with the role on that repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupRepositoryGrantResponse {
    pub(crate) repository_id: String,
    pub(crate) repository_name: String,
    pub(crate) role: RoleDto,
}

/// Group detail: members and repositories.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupDetailResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) members: Vec<UserResponse>,
    pub(crate) repositories: Vec<GroupRepositoryGrantResponse>,
}

/// Request body to create a group.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct CreateGroupRequest {
    pub(crate) name: String,
}

/// Request body to replace the members of a group.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetGroupMembersRequest {
    pub(crate) user_ids: Vec<String>,
}

/// Assignment of a repository to a group.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct GroupRepositoryGrantRequest {
    pub(crate) repository_id: String,
    pub(crate) role: RoleDto,
}

/// Request body to replace the repositories of a group.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetGroupRepositoriesRequest {
    pub(crate) grants: Vec<GroupRepositoryGrantRequest>,
}

/// Group with access to a repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct RepositoryAccessGrantResponse {
    pub(crate) group_id: String,
    pub(crate) group_name: String,
    pub(crate) role: RoleDto,
}

/// Assignment of a group to a repository.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct RepositoryAccessGrantRequest {
    pub(crate) group_id: String,
    pub(crate) role: RoleDto,
}

/// Request body to replace the groups of a repository.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct SetRepositoryAccessRequest {
    pub(crate) grants: Vec<RepositoryAccessGrantRequest>,
}

/// Request body to copy a version from one Forge to another.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct PromotePackageRequest {
    /// Identifier of the destination Forge, of the same ecosystem.
    pub(crate) target_repository_id: String,
    /// Package name. Required except in generic repositories.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) name: Option<String>,
    /// Version, OCI tag, or Conan reference (`0.1@_/_`).
    #[serde(default)]
    #[ts(optional)]
    pub(crate) version: Option<String>,
    /// Binary identifier, for generic repositories.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) artifact_id: Option<String>,
    /// If `true`, the copy keeps the yank of the source.
    #[serde(default)]
    pub(crate) preserve_yanked: bool,
}

/// Body to warm the cache of a `Mirror`.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct PrefetchPackageRequest {
    /// Package, module, or image name.
    pub(crate) name: String,
    /// Version, OCI tag, or digest. Empty = index only.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) version: Option<String>,
}

/// Replica destination sent when saving.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct ReplicaPolicyRequest {
    /// URL of the remote instance (`http://host:3000`). Empty = delete.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) remote_url: Option<String>,
    /// UUID of the destination Forge on that instance.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) destination_id: Option<String>,
    /// API token with write access on the destination. Absent: keeps the previous one.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) token: Option<String>,
    /// `push` (default) or `pull`.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) direction: Option<String>,
    /// Minutes between scheduled runs. `0` disables. Omitted keeps the current value.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) interval_minutes: Option<u32>,
}

/// Last replica run.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ReplicaRunResponse {
    pub(crate) occurred_at: String,
    #[ts(type = "number")]
    pub(crate) packages_imported: u32,
    #[ts(type = "number")]
    pub(crate) artifacts_imported: u32,
    #[ts(type = "number")]
    pub(crate) skipped: u32,
    pub(crate) error: Option<String>,
}

/// Replica policy of a repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ReplicaPolicyResponse {
    pub(crate) configured: bool,
    pub(crate) remote_url: Option<String>,
    pub(crate) destination_id: Option<String>,
    pub(crate) direction: String,
    pub(crate) has_token: bool,
    #[ts(type = "number | null")]
    pub(crate) interval_minutes: Option<u32>,
    pub(crate) last_run: Option<ReplicaRunResponse>,
}

impl From<ferrobox_domain::replica::ReplicaPolicy> for ReplicaPolicyResponse {
    fn from(policy: ferrobox_domain::replica::ReplicaPolicy) -> Self {
        let last_run = policy.last_run().map(|run| ReplicaRunResponse {
            occurred_at: run.occurred_at().to_string(),
            packages_imported: run.packages_imported(),
            artifacts_imported: run.artifacts_imported(),
            skipped: run.skipped(),
            error: run.error().map(str::to_string),
        });
        match policy.target() {
            Some(target) => Self {
                configured: true,
                remote_url: Some(target.remote_url().as_str().to_string()),
                destination_id: Some(target.destination_id().to_string()),
                direction: target.direction().as_str().to_string(),
                has_token: target.token().is_some(),
                interval_minutes: policy.interval_minutes(),
                last_run,
            },
            None => Self {
                configured: false,
                remote_url: None,
                destination_id: None,
                direction: "push".to_string(),
                has_token: false,
                interval_minutes: policy.interval_minutes(),
                last_run,
            },
        }
    }
}

/// Result of a replica push.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct ReplicaPushResponse {
    #[ts(type = "number")]
    pub(crate) packages_imported: u32,
    #[ts(type = "number")]
    pub(crate) artifacts_imported: u32,
    #[ts(type = "number")]
    pub(crate) skipped: u32,
}

/// Result of importing a portable archive.
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

/// Result of the prefetch.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PrefetchPackageResponse {
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    pub(crate) indexed: bool,
    pub(crate) downloaded: bool,
}

/// Result of copying a version to another Forge.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct PromotePackageResponse {
    /// Copied name, if the ecosystem indexes by coordinate.
    pub(crate) name: Option<String>,
    /// Copied version.
    pub(crate) version: Option<String>,
    /// New binaries created on the destination.
    #[ts(type = "number")]
    pub(crate) artifacts_copied: u32,
    /// Bytes written on the destination.
    #[ts(type = "number")]
    pub(crate) bytes_copied: u64,
}

/// Event of an HTTP notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) enum WebhookEventDto {
    /// Assay finished.
    #[serde(rename = "assay.completed")]
    AssayCompleted,
    /// Version published or cached.
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

/// HTTP notification of a repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct WebhookResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) url: String,
    /// `true` if an HMAC secret is configured. The value is never
    /// returned.
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

/// Delivery of an HTTP notification.
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

/// Body to create an HTTP notification.
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

/// Evaluation moment of an admission policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionWhenDto {
    /// When resolving a manifest or a package to install it.
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

/// Condition that triggers the effect of an admission policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionPredicateDto {
    /// The artifact has no linked Cosign / Notation signature.
    NotSigned,
    /// The artifact has no Cosign signature valid against the keys.
    NotVerified,
}

impl AdmissionPredicateDto {
    #[allow(dead_code)]
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

/// What to do if the admission condition is met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionEffectDto {
    /// Blocks the operation.
    Deny,
    /// Lets it through and only records it.
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

/// Admission policy sent when saving or simulating.
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPolicyRequest {
    /// `true` if the rule applies on pull.
    pub(crate) enabled: bool,
    /// Evaluation moment.
    pub(crate) when: AdmissionWhenDto,
    /// Signature condition (compatibility).
    pub(crate) predicate: AdmissionPredicateDto,
    /// Effect if the condition is met.
    pub(crate) effect: AdmissionEffectDto,
    /// PEM of Cosign public keys (`cosign generate-key-pair`).
    #[serde(default)]
    pub(crate) public_keys_pem: String,
    /// Require a Cosign / Notation signature.
    #[serde(default)]
    pub(crate) require_signed: Option<bool>,
    /// Require verification against the PEM keys.
    #[serde(default)]
    pub(crate) require_verified: Option<bool>,
    /// Finding threshold (`medium`, `high`, `critical`). Empty = off.
    #[serde(default)]
    pub(crate) min_finding: Option<String>,
    /// Denied SPDX licenses.
    #[serde(default)]
    pub(crate) forbidden_licenses: Vec<String>,
    /// Profile that filled the rule, if one was chosen.
    #[serde(default)]
    pub(crate) profile: Option<String>,
}

/// Admission policy of a repository.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPolicyResponse {
    /// `true` if the rule applies on pull.
    pub(crate) enabled: bool,
    /// Evaluation moment.
    pub(crate) when: AdmissionWhenDto,
    /// Signature condition (compatibility).
    pub(crate) predicate: AdmissionPredicateDto,
    /// Effect if the condition is met.
    pub(crate) effect: AdmissionEffectDto,
    /// PEM of the repository Cosign public keys.
    pub(crate) public_keys_pem: String,
    /// Require a Cosign / Notation signature.
    pub(crate) require_signed: bool,
    /// Require verification against the PEM keys.
    pub(crate) require_verified: bool,
    /// Finding threshold, if the clause is armed.
    pub(crate) min_finding: Option<String>,
    /// Denied SPDX licenses.
    pub(crate) forbidden_licenses: Vec<String>,
    /// Profile that filled the rule, if one was chosen.
    pub(crate) profile: Option<String>,
}

impl AdmissionPolicyRequest {
    pub(crate) fn into_policy(
        self,
    ) -> Result<
        (ferrobox_domain::admission::AdmissionPolicy, String),
        ferrobox_domain::admission::AdmissionPolicyError,
    > {
        let require_signed = self
            .require_signed
            .unwrap_or(matches!(self.predicate, AdmissionPredicateDto::NotSigned));
        let require_verified = self
            .require_verified
            .unwrap_or(matches!(self.predicate, AdmissionPredicateDto::NotVerified));
        let min_finding = match self
            .min_finding
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some("critical") => Some(ferrobox_domain::assay::AssaySeverity::Critical),
            Some("high") => Some(ferrobox_domain::assay::AssaySeverity::High),
            Some("medium") => Some(ferrobox_domain::assay::AssaySeverity::Medium),
            Some("low") => Some(ferrobox_domain::assay::AssaySeverity::Low),
            Some(other) => {
                return Err(
                    ferrobox_domain::admission::AdmissionPolicyError::UnknownFinding(
                        other.to_string(),
                    ),
                );
            }
        };
        let profile = self
            .profile
            .as_deref()
            .and_then(ferrobox_domain::admission::AdmissionProfile::parse);
        let clauses = ferrobox_domain::admission::AdmissionClauses::new(
            require_signed,
            require_verified,
            min_finding,
            self.forbidden_licenses,
            profile,
        );
        let policy = ferrobox_domain::admission::AdmissionPolicy::compose(
            self.enabled,
            self.when.as_str(),
            self.effect.as_str(),
            clauses,
        )?;
        Ok((policy, self.public_keys_pem))
    }
}

impl From<ferrobox_ports::admission_store::AdmissionRecord> for AdmissionPolicyResponse {
    fn from(record: ferrobox_ports::admission_store::AdmissionRecord) -> Self {
        let clauses = record.policy.clauses();
        Self {
            enabled: record.policy.enabled(),
            when: record.policy.when().into(),
            predicate: record.policy.predicate().into(),
            effect: record.policy.effect().into(),
            public_keys_pem: record.public_keys_pem,
            require_signed: clauses.require_signed(),
            require_verified: clauses.require_verified(),
            min_finding: clauses
                .min_finding()
                .map(|severity| severity.as_str().to_string()),
            forbidden_licenses: clauses.forbidden_licenses().to_vec(),
            profile: clauses
                .profile()
                .map(|profile| profile.as_str().to_string()),
        }
    }
}

/// An artifact the policy would touch on a pull.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPreviewItemResponse {
    /// Package or image name.
    pub(crate) name: String,
    /// Version or tag.
    pub(crate) version: String,
    /// `deny` or `warn`.
    pub(crate) effect: AdmissionEffectDto,
    /// Readable reason.
    pub(crate) reason: String,
}

/// Result of simulating the policy against the inventory.
#[derive(Serialize, TS)]
#[ts(export)]
pub(crate) struct AdmissionPreviewResponse {
    /// Artifacts that trigger the condition.
    pub(crate) matches: Vec<AdmissionPreviewItemResponse>,
    /// Listed versions that do not trigger the condition.
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

/// A warning or a denial recorded on a pull.
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

/// A row of the audit log.
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

/// Body to update an HTTP notification.
#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct UpdateWebhookRequest {
    pub(crate) name: String,
    pub(crate) url: String,
    /// Absent: keeps the secret. Empty string: deletes it.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) secret: Option<String>,
    pub(crate) events: Vec<WebhookEventDto>,
    pub(crate) enabled: bool,
}
