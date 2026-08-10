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

#[cfg(test)]
mod test_support;
