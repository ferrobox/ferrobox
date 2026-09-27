use ferrobox_domain::ids::ArtifactId;
use ferrobox_ports::storage::StorageKey;

/// Derives an artifact storage key from its identifier. This is an
/// internal application-layer convention -- neither the domain nor the
/// ports need to know how it is built.
pub(crate) fn storage_key_for(id: ArtifactId) -> StorageKey {
    StorageKey::new(format!("artifacts/{id}"))
}
