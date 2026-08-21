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

/// Identificador único de un repositorio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RepositoryId(Uuid);

impl RepositoryId {
    /// Genera un nuevo identificador, usando UUID versión 7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for RepositoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for RepositoryId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<RepositoryId> for Uuid {
    fn from(value: RepositoryId) -> Self {
        value.0
    }
}

/// Identificador único de un usuario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(Uuid);

impl UserId {
    /// Genera un nuevo identificador, usando UUID versión 7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for UserId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<UserId> for Uuid {
    fn from(value: UserId) -> Self {
        value.0
    }
}

/// Identificador único de un token de API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApiTokenId(Uuid);

impl ApiTokenId {
    /// Genera un nuevo identificador, usando UUID versión 7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for ApiTokenId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ApiTokenId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for ApiTokenId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<ApiTokenId> for Uuid {
    fn from(value: ApiTokenId) -> Self {
        value.0
    }
}

/// Identificador único de un ensaye (`Assay`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssayId(Uuid);

impl AssayId {
    /// Genera un nuevo identificador, usando UUID versión 7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for AssayId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AssayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for AssayId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<AssayId> for Uuid {
    fn from(value: AssayId) -> Self {
        value.0
    }
}

/// Identificador único de un grupo de usuarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupId(Uuid);

impl GroupId {
    /// Genera un nuevo identificador, usando UUID versión 7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for GroupId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for GroupId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<GroupId> for Uuid {
    fn from(value: GroupId) -> Self {
        value.0
    }
}

/// Identificador único de un aviso HTTP (`webhook`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WebhookId(Uuid);

impl WebhookId {
    /// Genera un nuevo identificador, usando UUID versión 7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for WebhookId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WebhookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for WebhookId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<WebhookId> for Uuid {
    fn from(value: WebhookId) -> Self {
        value.0
    }
}

/// Identificador único de un envío de aviso HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WebhookDeliveryId(Uuid);

impl WebhookDeliveryId {
    /// Genera un nuevo identificador, usando UUID versión 7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for WebhookDeliveryId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WebhookDeliveryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for WebhookDeliveryId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<WebhookDeliveryId> for Uuid {
    fn from(value: WebhookDeliveryId) -> Self {
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

    #[test]
    fn user_and_token_ids_round_trip_through_uuid() {
        let user_id = UserId::new();
        let token_id = ApiTokenId::new();

        assert_eq!(UserId::from(Uuid::from(user_id)), user_id);
        assert_eq!(ApiTokenId::from(Uuid::from(token_id)), token_id);
        let assay_id = AssayId::new();
        assert_eq!(AssayId::from(Uuid::from(assay_id)), assay_id);
        let group_id = GroupId::new();
        assert_eq!(GroupId::from(Uuid::from(group_id)), group_id);
        let webhook_id = WebhookId::new();
        assert_eq!(WebhookId::from(Uuid::from(webhook_id)), webhook_id);
        let delivery_id = WebhookDeliveryId::new();
        assert_eq!(
            WebhookDeliveryId::from(Uuid::from(delivery_id)),
            delivery_id
        );
    }
}
