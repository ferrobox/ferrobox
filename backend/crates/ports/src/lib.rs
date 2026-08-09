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
