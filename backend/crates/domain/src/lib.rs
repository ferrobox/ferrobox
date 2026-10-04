//! Domain layer of `FerroBox`.
//!
//! Contains entities, value objects, and business invariants. This
//! crate has no dependency on any other `FerroBox` crate and must never
//! depend on infrastructure details (databases, HTTP, object storage).
//! It is the innermost ring of Clean Architecture (Robert C. Martin,
//! *Clean Architecture*, 2017): the rest of the crates may depend on
//! this one, but this crate must never depend on them.

/// `RepositoryKind`, `PackageEcosystem`, and the Repository entity.
pub mod repository;

/// The Artifact entity and its lifecycle.
pub mod artifact;

/// Validated artifact checksums.
pub mod checksum;

/// Domain identifiers (value objects).
pub mod ids;

/// Package coordinates (`PackageEcosystem`, `PackageName`,
/// `PackageVersion`): uniquely identify a concrete version of a
/// package within an ecosystem, regardless of which repository it is
/// published in.
pub mod package_coordinate;

/// The `User` entity and the `Username` and `Email` value objects.
pub mod user;

/// Federated identity (`OIDC`) and IdP role and group mapping.
pub mod oidc;

/// The `Group` entity and a group's access to a repository.
pub mod group;

/// HTTP webhook for a repository.
pub mod webhook;

/// Username and secret a `Mirror` sends to its upstream.
pub mod mirror_credential;

/// The `ApiToken` entity and the `ApiTokenName` value object.
pub mod api_token;

/// The `Assay` entity: assay of an artifact (composition and impurities).
pub mod assay;

/// Version retention policy for a repository.
pub mod retention;

/// Admission policy (signature, and later other conditions).
pub mod admission;

/// Push replica toward another FerroBox instance.
pub mod replica;

/// Audit log of business writes.
pub mod audit;

/// Storage quota for a repository.
pub mod quota;

/// Write-once / read-many lock for a repository.
pub mod worm;
