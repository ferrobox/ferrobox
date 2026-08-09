use ferrobox_domain::ids::ArtifactId;
use ferrobox_ports::storage::StorageKey;

/// Deriva la clave de almacenamiento de un artefacto a partir de su
/// identificador. Es una convención interna de la capa de aplicación --
/// ni el dominio ni los puertos necesitan saber cómo se construye.
pub(crate) fn storage_key_for(id: ArtifactId) -> StorageKey {
    StorageKey::new(format!("artifacts/{id}"))
}
