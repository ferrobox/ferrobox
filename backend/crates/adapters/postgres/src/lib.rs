//! Adaptadores de persistencia de `FerroBox` contra `PostgreSQL`, usando
//! `sqlx`.

/// Adaptador de `RepositoryStore`.
pub mod repository_store;

/// Adaptador de `ArtifactStore`.
pub mod artifact_store;

/// Adaptador de `PackageIndexStore`.
pub mod package_index_store;

mod ecosystem_column;
