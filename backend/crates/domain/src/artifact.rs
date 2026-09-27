use crate::checksum::Sha256Checksum;
use crate::ids::{ArtifactId, RepositoryId};

/// A binary artifact stored in a repository.
///
/// Unlike value objects (`ArtifactId`, `Sha256Checksum`),
/// `Artifact` is an **Entity**: two instances with the same
/// identifier are the same entity, even if the rest of their fields
/// differ. That is why `PartialEq` and `Hash` are implemented by hand,
/// comparing and hashing solely from the identifier.
#[derive(Debug, Clone)]
pub struct Artifact {
    id: ArtifactId,
    repository_id: RepositoryId,
    checksum: Sha256Checksum,
    size_bytes: u64,
    filename: Option<String>,
}

impl Artifact {
    /// Registers a new artifact, assigning it a new identifier.
    #[must_use]
    pub fn new(repository_id: RepositoryId, checksum: Sha256Checksum, size_bytes: u64) -> Self {
        Self {
            id: ArtifactId::new(),
            repository_id,
            checksum,
            size_bytes,
            filename: None,
        }
    }

    /// Reconstitutes an already existing artifact from a
    /// known identifier (for example, when loading it from
    /// persistence).
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
            filename: None,
        }
    }

    /// Original upload name, when the client sent one.
    #[must_use]
    pub fn with_filename(mut self, filename: Option<String>) -> Self {
        self.filename = filename.filter(|name| !name.is_empty());
        self
    }

    /// Original upload name, if one was stored.
    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// Unique identifier of this artifact.
    #[must_use]
    pub fn id(&self) -> ArtifactId {
        self.id
    }

    /// Identifier of the repository this artifact belongs to.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Validated SHA-256 checksum of the binary content.
    #[must_use]
    pub fn checksum(&self) -> &Sha256Checksum {
        &self.checksum
    }

    /// Size in bytes of the binary content.
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

    #[test]
    fn with_filename_keeps_a_non_empty_name() {
        let artifact = Artifact::new(RepositoryId::new(), dummy_checksum(), 8)
            .with_filename(Some("firefox-142.0.1.tar.xz".to_string()));
        assert_eq!(artifact.filename(), Some("firefox-142.0.1.tar.xz"));
    }
}
