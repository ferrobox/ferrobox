//! Capa de aplicación de `FerroBox`.
//!
//! Contiene los casos de uso (interactores) que orquestan las entidades
//! de dominio a través de los puertos definidos en `ferrobox-ports`.

/// Deriva claves de almacenamiento para artefactos.
mod storage_key;

/// Caso de uso: crear un nuevo repositorio.
pub mod create_repository;

/// Caso de uso: publicar un artefacto en un repositorio existente.
pub mod publish_artifact;

/// Caso de uso: descargar un artefacto ya publicado.
pub mod download_artifact;

/// Caso de uso: listar los artefactos de un repositorio.
pub mod list_repository_artifacts;

/// Caso de uso: listar todos los repositorios existentes.
pub mod list_repositories;

/// Caso de uso: consultar el detalle de un repositorio existente.
pub mod get_repository;

/// El patrón Strategy para publicar, indexar y descargar paquetes según
/// su ecosistema (Cargo, npm, ...).
pub mod packaging;

/// Hashing de contraseñas y secretos de tokens de API.
pub mod auth_crypto;

/// Caso de uso: crear el administrador inicial si no hay usuarios.
pub mod bootstrap_admin;

/// Caso de uso: autenticar con usuario y contraseña.
pub mod login;

/// Casos de uso: crear, listar y revocar tokens de API.
pub mod manage_api_tokens;

/// Casos de uso: crear, listar y eliminar usuarios.
pub mod manage_users;

/// Caso de uso: resolver un secreto Bearer a un principal autenticado.
pub mod authenticate_token;

#[cfg(test)]
mod test_support;
