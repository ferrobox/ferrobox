//! Application layer of `FerroBox`.
//!
//! Contains the use cases (interactors) that orchestrate domain entities
//! through the ports defined in `ferrobox-ports`.

/// Derives storage keys for artifacts.
mod storage_key;

/// Computes SHA-256 checksums of binary contents.
mod content_hash;

/// Use case: create a new repository.
pub mod create_repository;

/// Shared validation of `Alloy` members.
pub mod alloy_members;

/// Use case: update the members of an `Alloy` repository.
pub mod update_alloy_members;

/// Use case: publish an artifact to an existing repository.
pub mod publish_artifact;

/// Use case: copy a published version from one Forge to another Forge.
pub mod promote_package;

/// Use case: warm a `Mirror` cache from its *upstream*.
pub mod prefetch_package;

/// Export and import a repository as a portable archive.
pub mod repository_bundle;

/// Interval and scheduled refresh of a `Mirror`.
pub mod mirror_schedule;

/// Use case: delete a repository and all of its content.
pub mod delete_repository;

/// Use case: delete an artifact (and its index entry, if any).
pub mod delete_artifact;

/// Use case: download an already published artifact.
pub mod download_artifact;

/// Use case: list the artifacts of a repository.
pub mod list_repository_artifacts;

/// Use case: list every existing repository.
pub mod list_repositories;

/// Use case: look up the details of an existing repository.
pub mod get_repository;

/// Strategy pattern for publishing, indexing, and downloading packages
/// according to their ecosystem (Cargo, npm, `PyPI`, ...).
pub mod packaging;

/// Hashing of passwords and API token secrets.
pub mod auth_crypto;

/// Use case: create the initial administrator if there are no users.
pub mod bootstrap_admin;

/// Use case: authenticate with username and password.
pub mod login;

/// Federated sign-in (`OIDC`): PKCE, JIT, and group mapping.
pub mod oidc;

/// Use case: the authenticated user changes their own password.
pub mod change_password;

/// Use cases: create, list, and revoke API tokens.
pub mod manage_api_tokens;

/// Use cases: create, list, change the role of, reset the password of,
/// and delete users.
pub mod manage_users;

/// Use cases: user groups and repository access.
pub mod manage_groups;

/// Use cases: HTTP notifications per repository.
pub mod webhooks;

/// Use case: resolve a Bearer secret to an authenticated principal.
pub mod authenticate_token;

/// Package assay: inventory and known vulnerabilities.
pub mod assay;

/// Version retention and garbage collection.
pub mod retention;

/// Admission policy when pulling or publishing.
pub mod admission;

/// Audit log of business writes.
pub mod audit;

/// Per-repository storage quota.
pub mod quota;

/// Write-once / read-many lock per repository.
pub mod worm;

/// Replica push, pull, and scheduled runs to another FerroBox instance.
pub mod replica;

/// Search packages in the instance catalog.
pub mod search_packages;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_support;
