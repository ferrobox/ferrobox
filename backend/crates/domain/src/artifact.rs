use crate::checksum::Sha256Checksum;
use crate::ids::ArtifactId;

/// Un artefacto binario almacenado en un repositorio.
///
/// A diferencia de los objetos de valor (`ArtifactId`, `Sha256Checksum`),
/// `Artifact` es una **Entidad**: dos instancias con el mismo
/// identificador son la misma entidad, incluso si el resto de sus campos
/// difiere (por ejemplo, tras corregir un tamaño registrado
/// incorrectamente). Por eso `PartialEq` y `Hash` se implementan a mano
/// más abajo, comparando y derivando únicamente a partir del
/// identificador -- deliberadamente no los derivamos automáticamente, ya
/// que eso compararía también por `checksum` y `size_bytes`, mezclando la
/// semántica de "objeto de valor" con la de "entidad".
#[derive(Debug, Clone)]
pub struct Artifact {
    id: ArtifactId,
    checksum: Sha256Checksum,
    size_bytes: u64,
}

impl Artifact {
    /// Registra un artefacto nuevo, asignándole un identificador nuevo.
    #[must_use]
    pub fn new(checksum: Sha256Checksum, size_bytes: u64) -> Self {
        Self {
            id: ArtifactId::new(),
            checksum,
            size_bytes,
        }
    }

    /// Reconstituye un artefacto ya existente (por ejemplo, al cargarlo
    /// desde persistencia) a partir de un identificador conocido.
    #[must_use]
    pub fn from_parts(id: ArtifactId, checksum: Sha256Checksum, size_bytes: u64) -> Self {
        Self {
            id,
            checksum,
            size_bytes,
        }
    }

    /// Identificador único de este artefacto.
    #[must_use]
    pub fn id(&self) -> ArtifactId {
        self.id
    }

    /// Checksum SHA-256 validado del contenido binario.
    #[must_use]
    pub fn checksum(&self) -> &Sha256Checksum {
        &self.checksum
    }

    /// Tamaño en bytes del contenido binario.
    #[must_use]
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

impl PartialEq for Artifact {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Artifact {}

impl std::hash::Hash for Artifact {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_checksum() -> Sha256Checksum {
        Sha256Checksum::parse("a".repeat(64)).unwrap()
    }

    #[test]
    fn two_freshly_created_artifacts_have_different_identity() {
        let first = Artifact::new(dummy_checksum(), 1024);
        let second = Artifact::new(dummy_checksum(), 1024);

        assert_ne!(first, second);
    }

    #[test]
    fn equality_is_based_on_identity_not_on_other_fields() {
        let id = ArtifactId::new();
        let original = Artifact::from_parts(id, dummy_checksum(), 1024);
        let corrected_size = Artifact::from_parts(id, dummy_checksum(), 2048);

        assert_eq!(original, corrected_size);
    }
}
