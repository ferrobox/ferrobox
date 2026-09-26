//! Adaptadores de persistencia de `FerroBox` contra `PostgreSQL`, usando
//! `sqlx`.

/// Adaptador de `RepositoryStore`.
pub mod repository_store;

/// Adaptador de `ArtifactStore`.
pub mod artifact_store;

/// Adaptador de `PackageIndexStore`.
pub mod package_index_store;

/// Adaptador de `UserStore`.
pub mod user_store;

/// Adaptador de `GroupStore`.
pub mod group_store;

/// Adaptador de `ApiTokenStore`.
pub mod api_token_store;

/// Adaptador de `AssayStore`.
pub mod assay_store;

/// Adaptador de `RetentionStore`.
pub mod retention_store;

/// Adaptador de `ReplicaStore`.
pub mod replica_store;

/// Adaptador de `AdmissionStore`.
pub mod admission_store;

/// Adaptador de `AuditStore`.
pub mod audit_store;

/// Adaptador de `QuotaStore`.
pub mod quota_store;

/// Adaptador de `WebhookStore`.
pub mod webhook_store;

mod ecosystem_column;
