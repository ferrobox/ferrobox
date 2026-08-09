//! Adaptadores de persistencia de `FerroBox` contra `PostgreSQL`, usando
//! `sqlx`.

/// Adaptador de `RepositoryStore`.
pub mod repository_store;

/// Adaptador de `ArtifactStore`.
pub mod artifact_store;
