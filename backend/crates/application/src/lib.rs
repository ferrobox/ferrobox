//! Capa de aplicación de `FerroBox`.
//!
//! Contiene los casos de uso (interactores) que orquestan las entidades
//! de dominio a través de los puertos definidos en `ferrobox-ports`.

/// Deriva claves de almacenamiento para artefactos.
mod storage_key;

/// Calcula checksums SHA-256 de contenidos binarios.
mod content_hash;

/// Caso de uso: crear un nuevo repositorio.
pub mod create_repository;

/// Validación compartida de los miembros de un `Alloy`.
pub mod alloy_members;

/// Caso de uso: actualizar los miembros de un repositorio `Alloy`.
pub mod update_alloy_members;

/// Caso de uso: publicar un artefacto en un repositorio existente.
pub mod publish_artifact;

/// Caso de uso: copiar una versión publicada de un Forge a otro Forge.
pub mod promote_package;

/// Caso de uso: eliminar un repositorio y todo su contenido.
pub mod delete_repository;

/// Caso de uso: eliminar un artefacto (y su entrada de índice, si la hay).
pub mod delete_artifact;

/// Caso de uso: descargar un artefacto ya publicado.
pub mod download_artifact;

/// Caso de uso: listar los artefactos de un repositorio.
pub mod list_repository_artifacts;

/// Caso de uso: listar todos los repositorios existentes.
pub mod list_repositories;

/// Caso de uso: consultar el detalle de un repositorio existente.
pub mod get_repository;

/// El patrón Strategy para publicar, indexar y descargar paquetes según
/// su ecosistema (Cargo, npm, `PyPI`, ...).
pub mod packaging;

/// Hashing de contraseñas y secretos de tokens de API.
pub mod auth_crypto;

/// Caso de uso: crear el administrador inicial si no hay usuarios.
pub mod bootstrap_admin;

/// Caso de uso: autenticar con usuario y contraseña.
pub mod login;

/// Caso de uso: el usuario autenticado cambia su propia contraseña.
pub mod change_password;

/// Casos de uso: crear, listar y revocar tokens de API.
pub mod manage_api_tokens;

/// Casos de uso: crear, listar, cambiar el rol, restablecer la
/// contraseña y eliminar usuarios.
pub mod manage_users;

/// Casos de uso: grupos de usuarios y acceso a repositorios.
pub mod manage_groups;

/// Casos de uso: avisos HTTP por repositorio.
pub mod webhooks;

/// Caso de uso: resolver un secreto Bearer a un principal autenticado.
pub mod authenticate_token;

/// Ensaye de paquetes: inventario y vulnerabilidades conocidas.
pub mod assay;

/// Retención de versiones y recolección de basura.
pub mod retention;

/// Política de admisión al bajar o publicar.
pub mod admission;

/// Cuota de almacenamiento por repositorio.
pub mod quota;

/// Búsqueda de paquetes en el catálogo de la instancia.
pub mod search_packages;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_support;
