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

#[cfg(test)]
mod test_support;
