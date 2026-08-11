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

/// Adaptador de `ApiTokenStore`.
pub mod api_token_store;

mod ecosystem_column;
