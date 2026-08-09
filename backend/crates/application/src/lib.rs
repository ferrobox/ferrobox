//! Capa de aplicación de `FerroBox`.
//!
//! Contiene los casos de uso (interactores) que orquestan las entidades
//! de dominio a través de los puertos definidos en `ferrobox-ports`. Esta
//! es la capa que codifica *qué hace el sistema*, independientemente de
//! *cómo* se expone (HTTP, línea de comandos) o *cómo* se persisten
//! realmente los datos.
