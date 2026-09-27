//! FerroBox persistence adapters against `PostgreSQL`, using `sqlx`.

/// Adapter for `RepositoryStore`.
pub mod repository_store;

/// Adapter for `ArtifactStore`.
pub mod artifact_store;

/// Adapter for `PackageIndexStore`.
pub mod package_index_store;

/// Adapter for `UserStore`.
pub mod user_store;

/// Adapter for `GroupStore`.
pub mod group_store;

/// Adapter for `ApiTokenStore`.
pub mod api_token_store;

/// Adapter for `AssayStore`.
pub mod assay_store;

/// Adapter for `RetentionStore`.
pub mod retention_store;

/// Adapter for `ReplicaStore`.
pub mod replica_store;

/// Adapter for `AdmissionStore`.
pub mod admission_store;

/// Adapter for `AuditStore`.
pub mod audit_store;

/// Adapter for `QuotaStore`.
pub mod quota_store;

/// Adapter for `WormStore`.
pub mod worm_store;

/// Adapter for `WebhookStore`.
pub mod webhook_store;

mod ecosystem_column;
