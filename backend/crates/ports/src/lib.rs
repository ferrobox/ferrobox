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

/// El puerto de persistencia de la entidad `ApiToken`.
pub mod api_token_store;

/// El puerto de cliente HTTP saliente.
pub mod http_client;
