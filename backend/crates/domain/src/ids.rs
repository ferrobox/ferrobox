use std::fmt;

use uuid::Uuid;

/// Identificador único de un artefacto.
///
/// Es un *newtype* sobre [`Uuid`]: el propio sistema de tipos, no una
/// convención documentada, impide confundir el identificador de un
/// artefacto con el de cualquier otra entidad futura (por ejemplo, un
/// repositorio), aunque ambos sean, en representación binaria, el mismo
/// valor de 128 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArtifactId(Uuid);

impl ArtifactId {
    /// Genera un nuevo identificador, usando UUID versión 7 (RFC 9562),
    /// que incorpora una marca de tiempo para conservar buena localidad
    /// de escritura en el índice de la base de datos.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for ArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for ArtifactId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<ArtifactId> for Uuid {
    fn from(value: ArtifactId) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_generated_ids_are_different() {
        let first = ArtifactId::new();
        let second = ArtifactId::new();

        assert_ne!(first, second);
    }

    #[test]
    fn round_trips_through_uuid() {
        let id = ArtifactId::new();
        let uuid: Uuid = id.into();
        let restored: ArtifactId = uuid.into();

        assert_eq!(id, restored);
    }
}
