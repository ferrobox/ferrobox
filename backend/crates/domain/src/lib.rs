//! Capa de dominio de `FerroBox`.
//!
//! Contiene entidades, objetos de valor e invariantes de negocio. Este
//! crate no tiene ninguna dependencia hacia otro crate de `FerroBox` y nunca
//! debe depender de detalles de infraestructura (bases de datos, HTTP,
//! almacenamiento de objetos). Es el anillo más interno de la Arquitectura
//! Limpia (Robert C. Martin, *Clean Architecture*, 2017): el resto de
//! crates pueden depender de este, pero este crate nunca puede depender
//! de ellos.

/// Identificadores de dominio (objetos de valor).
/// Checksums validados de artefactos.
pub mod checksum;

/// Identificadores de dominio (objetos de valor).
pub mod ids;
