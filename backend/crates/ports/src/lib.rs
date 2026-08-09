//! Puertos de `FerroBox`.
//!
//! Define los contratos basados en traits ("puertos") que la capa de
//! aplicación requiere del mundo exterior. Los adaptadores concretos
//! (a partir de este mismo paso) implementan estos traits; este crate
//! nunca depende de ningún adaptador.

/// El puerto de almacenamiento de contenido binario.
pub mod storage;
