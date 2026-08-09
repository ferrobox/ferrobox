//! Capa de aplicación de `FerroBox`.
//!
//! Contiene los casos de uso (interactores) que orquestan las entidades
//! de dominio a través de los puertos definidos en `ferrobox-ports`.

/// Deriva claves de almacenamiento para artefactos.
mod storage_key;

/// Caso de uso: publicar un artefacto en un repositorio existente.
pub mod publish_artifact;

/// Caso de uso: descargar un artefacto ya publicado.
pub mod download_artifact;

#[cfg(test)]
mod test_support;
