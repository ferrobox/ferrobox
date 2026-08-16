//! Capa de dominio de `FerroBox`.
//!
//! Contiene entidades, objetos de valor e invariantes de negocio. Este
//! crate no tiene ninguna dependencia hacia otro crate de `FerroBox` y nunca
//! debe depender de detalles de infraestructura (bases de datos, HTTP,
//! almacenamiento de objetos). Es el anillo más interno de la Arquitectura
//! Limpia (Robert C. Martin, *Clean Architecture*, 2017): el resto de
//! crates pueden depender de este, pero este crate nunca puede depender
//! de ellos.

/// `RepositoryKind`, `PackageEcosystem` y la entidad Repository.
pub mod repository;

/// La entidad Artifact y su ciclo de vida.
pub mod artifact;

/// Checksums validados de artefactos.
pub mod checksum;

/// Identificadores de dominio (objetos de valor).
pub mod ids;

/// Coordenadas de paquete (`PackageEcosystem`, `PackageName`,
/// `PackageVersion`): identifican unívocamente una versión concreta de
/// un paquete dentro de un ecosistema, independientemente de en qué
/// repositorio esté publicada.
pub mod package_coordinate;

/// La entidad `User` y el objeto de valor `Username`.
pub mod user;

/// La entidad `ApiToken` y el objeto de valor `ApiTokenName`.
pub mod api_token;

/// La entidad `Assay`: ensaye de un artefacto (composición e impurezas).
pub mod assay;

/// Política de retención de versiones de un repositorio.
pub mod retention;

/// Cuota de almacenamiento de un repositorio.
pub mod quota;
