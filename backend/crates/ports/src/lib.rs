//! Puertos de FerroBox.
//!
//! Define los contratos basados en traits ("puertos", en la terminología
//! de la Arquitectura Hexagonal de Alistair Cockburn, 2005) que la capa de
//! aplicación necesita del mundo exterior -- por ejemplo, persistir los
//! metadatos de un artefacto o almacenar su contenido binario. Los
//! adaptadores concretos (a partir de la Fase 3) implementarán estos
//! traits; este crate nunca depende de ningún adaptador.
