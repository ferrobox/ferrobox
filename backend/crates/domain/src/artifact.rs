use crate::checksum::Sha256Checksum;
use crate::ids::{ArtifactId, RepositoryId};

/// Un artefacto binario almacenado en un repositorio.
///
/// A diferencia de los objetos de valor (`ArtifactId`, `Sha256Checksum`),
/// `Artifact` es una **Entidad**: dos instancias con el mismo
/// identificador son la misma entidad, incluso si el resto de sus campos
/// difiere. Por eso `PartialEq` y `Hash` se implementan a mano,
/// comparando y derivando únicamente a partir del identificador.
#[derive(Debug, Clone)]
pub struct Artifact {
    id: ArtifactId,
    repository_id: RepositoryId,
    checksum: Sha256Checksum,
    size_bytes: u64,
}

impl Artifact {
    /// Registra un artefacto nuevo, asignándole un identificador nuevo.
    #[must_use]
    pub fn new(repository_id: RepositoryId, checksum: Sha256Checksum, size_bytes: u64) -> Self {
        Self {
            id: ArtifactId::new(),
            repository_id,
            checksum,
            size_bytes,
        }
    }

    /// Reconstituye un artefacto ya existente a partir de un
    /// identificador conocido (por ejemplo, al cargarlo desde
    /// persistencia).
    #[must_use]
    pub fn from_parts(
        id: ArtifactId,
        repository_id: RepositoryId,
        checksum: Sha256Checksum,
        size_bytes: u64,
    ) -> Self {
        Self {
            id,
            repository_id,
            checksum,
            size_bytes,
        }
    }

    /// Identificador único de este artefacto.
    #[must_use]
    pub fn id(&self) -> ArtifactId {
        self.id
    }

    /// Identificador del repositorio al que pertenece este artefacto.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
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
        let repository_id = RepositoryId::new();
        let first = Artifact::new(repository_id, dummy_checksum(), 1024);
        let second = Artifact::new(repository_id, dummy_checksum(), 1024);

        assert_ne!(first, second);
    }

    #[test]
    fn equality_is_based_on_identity_not_on_other_fields() {
        let id = ArtifactId::new();
        let repository_id = RepositoryId::new();
        let original = Artifact::from_parts(id, repository_id, dummy_checksum(), 1024);
        let corrected_size = Artifact::from_parts(id, repository_id, dummy_checksum(), 2048);

        assert_eq!(original, corrected_size);
    }
}
