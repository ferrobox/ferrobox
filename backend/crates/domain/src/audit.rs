//! Registro de auditoría: quién hizo qué sobre usuarios, grupos,
//! paquetes y la configuración de la instancia.
//!
//! No es un log de aplicación ni telemetría. Cada fila es una escritura
//! de negocio que un administrador debe poder revisar.

use thiserror::Error;

use crate::ids::{AuditEventId, UserId};

/// Acción registrada. Las etiquetas son estables: viajan en la API y
/// en la tabla SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    /// Se creó una cuenta.
    UserCreated,
    /// Se eliminó una cuenta.
    UserDeleted,
    /// Se cambió el rol de una cuenta.
    UserRoleChanged,
    /// Un administrador restableció la contraseña de otra cuenta.
    UserPasswordReset,
    /// El usuario cambió su propia contraseña.
    UserPasswordChanged,
    /// Un usuario inició sesión (o se aprovisionó) vía `OIDC`.
    UserSsoSignedIn,
    /// Se emitió un token de API.
    TokenCreated,
    /// Se revocó un token de API.
    TokenRevoked,
    /// Se creó un grupo.
    GroupCreated,
    /// Se eliminó un grupo.
    GroupDeleted,
    /// Se sustituyó la lista de miembros de un grupo.
    GroupMembersChanged,
    /// Se sustituyeron los repositorios de un grupo.
    GroupRepositoriesChanged,
    /// Se creó un repositorio.
    RepositoryCreated,
    /// Se eliminó un repositorio.
    RepositoryDeleted,
    /// Se cambiaron los miembros de un `Alloy`.
    RepositoryMembersChanged,
    /// Se publicó un artefacto genérico.
    ArtifactPublished,
    /// Se eliminó un artefacto.
    ArtifactDeleted,
    /// Se publicó un paquete (Cargo, npm, `PyPI`, OCI, Conan).
    PackagePublished,
    /// Se yankeó un paquete.
    PackageYanked,
    /// Se deshizo un yank.
    PackageUnyanked,
    /// Se copió una versión de un Forge a otro.
    PackagePromoted,
    /// Se cacheó un paquete desde el *upstream* de un `Mirror`.
    PackagePrefetched,
    /// Se guardó la política de admisión.
    AdmissionPolicyChanged,
    /// Se guardó la política de retención.
    RetentionPolicyChanged,
    /// Se aplicó la retención.
    RetentionApplied,
    /// Se ejecutó la recolección de basura.
    RetentionGarbageCollected,
    /// Se cambió la cuota de un repositorio.
    QuotaChanged,
    /// Se creó un aviso HTTP.
    WebhookCreated,
    /// Se actualizó un aviso HTTP.
    WebhookUpdated,
    /// Se eliminó un aviso HTTP.
    WebhookDeleted,
}

/// Clase del objeto sobre el que actúa el evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditTargetKind {
    /// Una cuenta.
    User,
    /// Un token de API.
    Token,
    /// Un grupo.
    Group,
    /// Un repositorio.
    Repository,
    /// Un artefacto genérico.
    Artifact,
    /// Un paquete de un ecosistema.
    Package,
    /// Un aviso HTTP.
    Webhook,
    /// La política de admisión de un repositorio.
    Admission,
    /// La política de retención o un GC.
    Retention,
    /// La cuota de un repositorio.
    Quota,
}

/// Motivos por los que una etiqueta persistida no es válida.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuditParseError {
    /// La acción no es una de las soportadas.
    #[error("unsupported audit action '{0}'")]
    UnknownAction(String),

    /// El tipo de destino no es uno de los soportados.
    #[error("unsupported audit target kind '{0}'")]
    UnknownTargetKind(String),
}

/// Una fila del registro de auditoría.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    id: AuditEventId,
    actor_id: Option<UserId>,
    actor_username: String,
    action: AuditAction,
    target_kind: AuditTargetKind,
    target: String,
    detail: String,
    created_at: String,
}

impl AuditEvent {
    /// Construye un evento ya persistido o recién emitido.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        id: AuditEventId,
        actor_id: Option<UserId>,
        actor_username: impl Into<String>,
        action: AuditAction,
        target_kind: AuditTargetKind,
        target: impl Into<String>,
        detail: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id,
            actor_id,
            actor_username: actor_username.into(),
            action,
            target_kind,
            target: target.into(),
            detail: detail.into(),
            created_at: created_at.into(),
        }
    }

    /// Identificador.
    #[must_use]
    pub fn id(&self) -> AuditEventId {
        self.id
    }

    /// Usuario que actuó, si la cuenta sigue existiendo.
    #[must_use]
    pub fn actor_id(&self) -> Option<UserId> {
        self.actor_id
    }

    /// Nombre del actor en el momento de la acción.
    #[must_use]
    pub fn actor_username(&self) -> &str {
        &self.actor_username
    }

    /// Acción.
    #[must_use]
    pub fn action(&self) -> AuditAction {
        self.action
    }

    /// Clase del objeto.
    #[must_use]
    pub fn target_kind(&self) -> AuditTargetKind {
        self.target_kind
    }

    /// Nombre o identificador del objeto.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Detalle opcional (rol nuevo, recuento, versión…).
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// Instante RFC 3339.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.created_at
    }
}

impl AuditAction {
    /// Etiqueta persistida.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserCreated => "user.created",
            Self::UserDeleted => "user.deleted",
            Self::UserRoleChanged => "user.role_changed",
            Self::UserPasswordReset => "user.password_reset",
            Self::UserPasswordChanged => "user.password_changed",
            Self::UserSsoSignedIn => "user.sso_signed_in",
            Self::TokenCreated => "token.created",
            Self::TokenRevoked => "token.revoked",
            Self::GroupCreated => "group.created",
            Self::GroupDeleted => "group.deleted",
            Self::GroupMembersChanged => "group.members_changed",
            Self::GroupRepositoriesChanged => "group.repositories_changed",
            Self::RepositoryCreated => "repository.created",
            Self::RepositoryDeleted => "repository.deleted",
            Self::RepositoryMembersChanged => "repository.members_changed",
            Self::ArtifactPublished => "artifact.published",
            Self::ArtifactDeleted => "artifact.deleted",
            Self::PackagePublished => "package.published",
            Self::PackageYanked => "package.yanked",
            Self::PackageUnyanked => "package.unyanked",
            Self::PackagePromoted => "package.promoted",
            Self::PackagePrefetched => "package.prefetched",
            Self::AdmissionPolicyChanged => "admission.policy_changed",
            Self::RetentionPolicyChanged => "retention.policy_changed",
            Self::RetentionApplied => "retention.applied",
            Self::RetentionGarbageCollected => "retention.gc",
            Self::QuotaChanged => "quota.changed",
            Self::WebhookCreated => "webhook.created",
            Self::WebhookUpdated => "webhook.updated",
            Self::WebhookDeleted => "webhook.deleted",
        }
    }

    /// Parsea la etiqueta persistida.
    ///
    /// # Errors
    ///
    /// [`AuditParseError::UnknownAction`] si la etiqueta no es válida.
    pub fn parse(value: &str) -> Result<Self, AuditParseError> {
        match value {
            "user.created" => Ok(Self::UserCreated),
            "user.deleted" => Ok(Self::UserDeleted),
            "user.role_changed" => Ok(Self::UserRoleChanged),
            "user.password_reset" => Ok(Self::UserPasswordReset),
            "user.password_changed" => Ok(Self::UserPasswordChanged),
            "user.sso_signed_in" => Ok(Self::UserSsoSignedIn),
            "token.created" => Ok(Self::TokenCreated),
            "token.revoked" => Ok(Self::TokenRevoked),
            "group.created" => Ok(Self::GroupCreated),
            "group.deleted" => Ok(Self::GroupDeleted),
            "group.members_changed" => Ok(Self::GroupMembersChanged),
            "group.repositories_changed" => Ok(Self::GroupRepositoriesChanged),
            "repository.created" => Ok(Self::RepositoryCreated),
            "repository.deleted" => Ok(Self::RepositoryDeleted),
            "repository.members_changed" => Ok(Self::RepositoryMembersChanged),
            "artifact.published" => Ok(Self::ArtifactPublished),
            "artifact.deleted" => Ok(Self::ArtifactDeleted),
            "package.published" => Ok(Self::PackagePublished),
            "package.yanked" => Ok(Self::PackageYanked),
            "package.unyanked" => Ok(Self::PackageUnyanked),
            "package.promoted" => Ok(Self::PackagePromoted),
            "package.prefetched" => Ok(Self::PackagePrefetched),
            "admission.policy_changed" => Ok(Self::AdmissionPolicyChanged),
            "retention.policy_changed" => Ok(Self::RetentionPolicyChanged),
            "retention.applied" => Ok(Self::RetentionApplied),
            "retention.gc" => Ok(Self::RetentionGarbageCollected),
            "quota.changed" => Ok(Self::QuotaChanged),
            "webhook.created" => Ok(Self::WebhookCreated),
            "webhook.updated" => Ok(Self::WebhookUpdated),
            "webhook.deleted" => Ok(Self::WebhookDeleted),
            other => Err(AuditParseError::UnknownAction(other.to_string())),
        }
    }
}

impl AuditTargetKind {
    /// Etiqueta persistida.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Token => "token",
            Self::Group => "group",
            Self::Repository => "repository",
            Self::Artifact => "artifact",
            Self::Package => "package",
            Self::Webhook => "webhook",
            Self::Admission => "admission",
            Self::Retention => "retention",
            Self::Quota => "quota",
        }
    }

    /// Parsea la etiqueta persistida.
    ///
    /// # Errors
    ///
    /// [`AuditParseError::UnknownTargetKind`] si la etiqueta no es válida.
    pub fn parse(value: &str) -> Result<Self, AuditParseError> {
        match value {
            "user" => Ok(Self::User),
            "token" => Ok(Self::Token),
            "group" => Ok(Self::Group),
            "repository" => Ok(Self::Repository),
            "artifact" => Ok(Self::Artifact),
            "package" => Ok(Self::Package),
            "webhook" => Ok(Self::Webhook),
            "admission" => Ok(Self::Admission),
            "retention" => Ok(Self::Retention),
            "quota" => Ok(Self::Quota),
            other => Err(AuditParseError::UnknownTargetKind(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_labels_round_trip() {
        for action in [
            AuditAction::UserCreated,
            AuditAction::UserDeleted,
            AuditAction::UserRoleChanged,
            AuditAction::UserPasswordReset,
            AuditAction::UserPasswordChanged,
            AuditAction::UserSsoSignedIn,
            AuditAction::TokenCreated,
            AuditAction::TokenRevoked,
            AuditAction::GroupCreated,
            AuditAction::GroupDeleted,
            AuditAction::GroupMembersChanged,
            AuditAction::GroupRepositoriesChanged,
            AuditAction::RepositoryCreated,
            AuditAction::RepositoryDeleted,
            AuditAction::RepositoryMembersChanged,
            AuditAction::ArtifactPublished,
            AuditAction::ArtifactDeleted,
            AuditAction::PackagePublished,
            AuditAction::PackageYanked,
            AuditAction::PackageUnyanked,
            AuditAction::PackagePromoted,
            AuditAction::PackagePrefetched,
            AuditAction::AdmissionPolicyChanged,
            AuditAction::RetentionPolicyChanged,
            AuditAction::RetentionApplied,
            AuditAction::RetentionGarbageCollected,
            AuditAction::QuotaChanged,
            AuditAction::WebhookCreated,
            AuditAction::WebhookUpdated,
            AuditAction::WebhookDeleted,
        ] {
            assert_eq!(AuditAction::parse(action.as_str()).unwrap(), action);
        }
    }

    #[test]
    fn target_kind_labels_round_trip() {
        for kind in [
            AuditTargetKind::User,
            AuditTargetKind::Token,
            AuditTargetKind::Group,
            AuditTargetKind::Repository,
            AuditTargetKind::Artifact,
            AuditTargetKind::Package,
            AuditTargetKind::Webhook,
            AuditTargetKind::Admission,
            AuditTargetKind::Retention,
            AuditTargetKind::Quota,
        ] {
            assert_eq!(AuditTargetKind::parse(kind.as_str()).unwrap(), kind);
        }
    }

    #[test]
    fn rejects_unknown_labels() {
        assert!(matches!(
            AuditAction::parse("login"),
            Err(AuditParseError::UnknownAction(_))
        ));
        assert!(matches!(
            AuditTargetKind::parse("session"),
            Err(AuditParseError::UnknownTargetKind(_))
        ));
    }
}
