//! Ports of `FerroBox`.
//!
//! Defines the trait-based contracts ("ports") that the application
//! layer requires from the outside world. Concrete adapters
//! implement these traits; this crate never depends on any
//! adapter.

/// Binary content storage port.
pub mod storage;

/// Persistence port for the `Repository` entity.
pub mod repository_store;

/// Persistence port for the `Artifact` entity.
pub mod artifact_store;

/// Persistence port for the per-ecosystem package index.
pub mod package_index_store;

/// Persistence port for the `User` entity and its credentials.
pub mod user_store;

/// Persistence port for groups and their repository access.
pub mod group_store;

/// Persistence port for HTTP webhooks and their deliveries.
pub mod webhook_store;

/// Persistence port for the `ApiToken` entity.
pub mod api_token_store;

/// Persistence port for the `Assay` entity.
pub mod assay_store;

/// Persistence port for the active vulnerability index.
pub mod osv_feed_store;

/// Persistence port for the retention policy.
pub mod retention_store;

/// Persistence port for the replica policy.
pub mod replica_store;

/// Persistence port for the admission policy.
pub mod admission_store;

/// Persistence port for the audit log.
pub mod audit_store;

/// Persistence port for the storage quota.
pub mod quota_store;

/// Persistence port for a repository WORM lock.
pub mod worm_store;

/// Outbound HTTP client port.
pub mod http_client;
