//! Audit log: who did what to users, groups,
//! packages, and instance configuration.
//!
//! This is not an application log or telemetry. Each row is a business
//! write that an administrator must be able to review.

use thiserror::Error;

use crate::ids::{AuditEventId, UserId};

/// Recorded action. Labels are stable: they travel in the API and
/// in the SQL table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    /// An account was created.
    UserCreated,
    /// An account was deleted.
    UserDeleted,
    /// An account's role was changed.
    UserRoleChanged,
    /// An administrator reset another account's password.
    UserPasswordReset,
    /// The user changed their own password.
    UserPasswordChanged,
    /// A user signed in (or was provisioned) via `OIDC`.
    UserSsoSignedIn,
    /// An API token was issued.
    TokenCreated,
    /// An API token was revoked.
    TokenRevoked,
    /// A group was created.
    GroupCreated,
    /// A group was deleted.
    GroupDeleted,
    /// A group's member list was replaced.
    GroupMembersChanged,
    /// A group's repositories were replaced.
    GroupRepositoriesChanged,
    /// A repository was created.
    RepositoryCreated,
    /// A repository was deleted.
    RepositoryDeleted,
    /// An `Alloy`'s members were changed.
    RepositoryMembersChanged,
    /// A generic artifact was published.
    ArtifactPublished,
    /// An artifact was deleted.
    ArtifactDeleted,
    /// A package was published (Cargo, npm, `PyPI`, OCI, Conan).
    PackagePublished,
    /// A package was yanked.
    PackageYanked,
    /// A yank was undone.
    PackageUnyanked,
    /// A version was copied from one Forge to another.
    PackagePromoted,
    /// A package was cached from a `Mirror`'s *upstream*.
    PackagePrefetched,
    /// A repository was exported to a portable archive.
    RepositoryExported,
    /// A portable archive was imported into a repository.
    RepositoryImported,
    /// A `Mirror`'s refresh interval was changed.
    MirrorScheduleChanged,
    /// A `Mirror`'s upstream username or secret was replaced or removed.
    MirrorUpstreamChanged,
    /// A `Mirror`'s upstream URL was changed.
    MirrorUpstreamUrlChanged,
    /// The admission policy was saved.
    AdmissionPolicyChanged,
    /// The retention policy was saved.
    RetentionPolicyChanged,
    /// Retention was applied.
    RetentionApplied,
    /// Garbage collection was run.
    RetentionGarbageCollected,
    /// A repository's quota was changed.
    QuotaChanged,
    /// A repository's WORM lock was turned on or off.
    WormPolicyChanged,
    /// An HTTP webhook was created.
    WebhookCreated,
    /// An HTTP webhook was updated.
    WebhookUpdated,
    /// An HTTP webhook was deleted.
    WebhookDeleted,
    /// The replica target was saved.
    ReplicaPolicyChanged,
    /// A repository was pushed to another instance.
    ReplicaPushed,
    /// A repository was pulled from another instance.
    ReplicaPulled,
}

/// Class of the object the event acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditTargetKind {
    /// An account.
    User,
    /// An API token.
    Token,
    /// A group.
    Group,
    /// A repository.
    Repository,
    /// A generic artifact.
    Artifact,
    /// A package from an ecosystem.
    Package,
    /// An HTTP webhook.
    Webhook,
    /// A repository's admission policy.
    Admission,
    /// The retention policy or a GC.
    Retention,
    /// A repository's quota.
    Quota,
    /// A repository's WORM lock.
    Worm,
}

/// Reasons why a persisted label is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuditParseError {
    /// The action is not one of the supported ones.
    #[error("unsupported audit action '{0}'")]
    UnknownAction(String),

    /// The target type is not one of the supported ones.
    #[error("unsupported audit target kind '{0}'")]
    UnknownTargetKind(String),
}

/// A row of the audit log.
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
    /// Builds an event that is already persisted or just emitted.
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

    /// Identifier.
    #[must_use]
    pub fn id(&self) -> AuditEventId {
        self.id
    }

    /// User who acted, if the account still exists.
    #[must_use]
    pub fn actor_id(&self) -> Option<UserId> {
        self.actor_id
    }

    /// Actor name at the time of the action.
    #[must_use]
    pub fn actor_username(&self) -> &str {
        &self.actor_username
    }

    /// Action.
    #[must_use]
    pub fn action(&self) -> AuditAction {
        self.action
    }

    /// Class of the object.
    #[must_use]
    pub fn target_kind(&self) -> AuditTargetKind {
        self.target_kind
    }

    /// Name or identifier of the object.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Optional detail (new role, count, version…).
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// RFC 3339 timestamp.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.created_at
    }
}

impl AuditAction {
    /// Persisted label.
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
            Self::MirrorScheduleChanged => "mirror.schedule_changed",
            Self::MirrorUpstreamChanged => "mirror.upstream_changed",
            Self::MirrorUpstreamUrlChanged => "mirror.upstream_url_changed",
            Self::RepositoryExported => "repository.exported",
            Self::RepositoryImported => "repository.imported",
            Self::AdmissionPolicyChanged => "admission.policy_changed",
            Self::RetentionPolicyChanged => "retention.policy_changed",
            Self::RetentionApplied => "retention.applied",
            Self::RetentionGarbageCollected => "retention.gc",
            Self::QuotaChanged => "quota.changed",
            Self::WormPolicyChanged => "worm.policy_changed",
            Self::WebhookCreated => "webhook.created",
            Self::WebhookUpdated => "webhook.updated",
            Self::WebhookDeleted => "webhook.deleted",
            Self::ReplicaPolicyChanged => "replica.policy_changed",
            Self::ReplicaPushed => "replica.pushed",
            Self::ReplicaPulled => "replica.pulled",
        }
    }

    /// Parses the persisted label.
    ///
    /// # Errors
    ///
    /// [`AuditParseError::UnknownAction`] if the label is not valid.
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
            "mirror.schedule_changed" => Ok(Self::MirrorScheduleChanged),
            "mirror.upstream_changed" => Ok(Self::MirrorUpstreamChanged),
            "mirror.upstream_url_changed" => Ok(Self::MirrorUpstreamUrlChanged),
            "repository.exported" => Ok(Self::RepositoryExported),
            "repository.imported" => Ok(Self::RepositoryImported),
            "admission.policy_changed" => Ok(Self::AdmissionPolicyChanged),
            "retention.policy_changed" => Ok(Self::RetentionPolicyChanged),
            "retention.applied" => Ok(Self::RetentionApplied),
            "retention.gc" => Ok(Self::RetentionGarbageCollected),
            "quota.changed" => Ok(Self::QuotaChanged),
            "worm.policy_changed" => Ok(Self::WormPolicyChanged),
            "webhook.created" => Ok(Self::WebhookCreated),
            "webhook.updated" => Ok(Self::WebhookUpdated),
            "webhook.deleted" => Ok(Self::WebhookDeleted),
            "replica.policy_changed" => Ok(Self::ReplicaPolicyChanged),
            "replica.pushed" => Ok(Self::ReplicaPushed),
            "replica.pulled" => Ok(Self::ReplicaPulled),
            other => Err(AuditParseError::UnknownAction(other.to_string())),
        }
    }
}

impl AuditTargetKind {
    /// Persisted label.
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
            Self::Worm => "worm",
        }
    }

    /// Parses the persisted label.
    ///
    /// # Errors
    ///
    /// [`AuditParseError::UnknownTargetKind`] if the label is not valid.
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
            "worm" => Ok(Self::Worm),
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
            AuditAction::MirrorScheduleChanged,
            AuditAction::MirrorUpstreamChanged,
            AuditAction::MirrorUpstreamUrlChanged,
            AuditAction::RepositoryExported,
            AuditAction::RepositoryImported,
            AuditAction::AdmissionPolicyChanged,
            AuditAction::RetentionPolicyChanged,
            AuditAction::RetentionApplied,
            AuditAction::RetentionGarbageCollected,
            AuditAction::QuotaChanged,
            AuditAction::WormPolicyChanged,
            AuditAction::WebhookCreated,
            AuditAction::WebhookUpdated,
            AuditAction::WebhookDeleted,
            AuditAction::ReplicaPolicyChanged,
            AuditAction::ReplicaPushed,
            AuditAction::ReplicaPulled,
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
            AuditTargetKind::Worm,
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
