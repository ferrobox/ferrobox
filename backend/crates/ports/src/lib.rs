//! Puertos de `FerroBox`.
//!
//! Define los contratos basados en traits ("puertos") que la capa de
//! aplicación requiere del mundo exterior. Los adaptadores concretos
//! implementan estos traits; este crate nunca depende de ningún
//! adaptador.

/// El puerto de almacenamiento de contenido binario.
pub mod storage;

/// El puerto de persistencia de la entidad `Repository`.
pub mod repository_store;

/// El puerto de persistencia de la entidad `Artifact`.
pub mod artifact_store;

/// El puerto de persistencia del índice de paquetes por ecosistema.
pub mod package_index_store;

/// El puerto de persistencia de la entidad `User` y sus credenciales.
pub mod user_store;

/// El puerto de persistencia de grupos y su acceso a repositorios.
pub mod group_store;

/// El puerto de persistencia de avisos HTTP y sus envíos.
pub mod webhook_store;

/// El puerto de persistencia de la entidad `ApiToken`.
pub mod api_token_store;

/// El puerto de persistencia de la entidad `Assay`.
pub mod assay_store;

/// El puerto de persistencia de la política de retención.
pub mod retention_store;

/// El puerto de persistencia de la política de réplica.
pub mod replica_store;

/// El puerto de persistencia de la política de admisión.
pub mod admission_store;

/// El puerto de persistencia del registro de auditoría.
pub mod audit_store;

/// El puerto de persistencia de la cuota de almacenamiento.
pub mod quota_store;

/// Persistence port for a repository WORM lock.
pub mod worm_store;

/// El puerto de cliente HTTP saliente.
pub mod http_client;
