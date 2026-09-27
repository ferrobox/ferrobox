use async_trait::async_trait;
use ferrobox_domain::group::{Group, GroupName};
use ferrobox_domain::ids::{GroupId, RepositoryId, UserId};
use ferrobox_domain::user::Role;
use thiserror::Error;

/// Reasons a group persistence operation can fail.
#[derive(Debug, Error)]
pub enum GroupStoreError {
    /// A group with that name already exists.
    #[error("a group named '{0}' already exists")]
    DuplicateName(GroupName),

    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A group assigned to a repository, with the role in that repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryGroupGrant {
    /// Repository the access applies to.
    pub repository_id: RepositoryId,
    /// Group that receives the access.
    pub group_id: GroupId,
    /// Role of the group in that repository (`reader` or `developer`).
    pub role: Role,
}

/// Persistence port for groups, members, and repository access.
#[async_trait]
pub trait GroupStore: Send + Sync {
    /// Inserts or updates a group (name).
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::DuplicateName`] or [`GroupStoreError::Backend`].
    async fn save(&self, group: &Group) -> Result<(), GroupStoreError>;

    /// Looks up a group by identifier.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn find_by_id(&self, id: GroupId) -> Result<Option<Group>, GroupStoreError>;

    /// Looks up a group by name.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn find_by_name(&self, name: &GroupName) -> Result<Option<Group>, GroupStoreError>;

    /// Lists every group, ordered by name.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn find_all(&self) -> Result<Vec<Group>, GroupStoreError>;

    /// Deletes a group and its associations. Returns `false` if it did
    /// not exist.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn delete(&self, id: GroupId) -> Result<bool, GroupStoreError>;

    /// Replaces the group members with `user_ids`.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn set_members(
        &self,
        group_id: GroupId,
        user_ids: &[UserId],
    ) -> Result<(), GroupStoreError>;

    /// Lists the members of a group.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn members(&self, group_id: GroupId) -> Result<Vec<UserId>, GroupStoreError>;

    /// Groups a user belongs to.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn groups_for_user(&self, user_id: UserId) -> Result<Vec<GroupId>, GroupStoreError>;

    /// Adds a member if they are not one already.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn add_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<(), GroupStoreError>;

    /// Removes a member. It is not an error if they were not one.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn remove_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<(), GroupStoreError>;

    /// Groups whose membership for this user is managed by the `IdP`.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn sso_memberships(&self, user_id: UserId) -> Result<Vec<GroupId>, GroupStoreError>;

    /// Replaces the set of groups whose membership is managed by the `IdP`.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn set_sso_memberships(
        &self,
        user_id: UserId,
        group_ids: &[GroupId],
    ) -> Result<(), GroupStoreError>;

    /// Replaces the repositories assigned to a group.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn set_group_repositories(
        &self,
        group_id: GroupId,
        grants: &[(RepositoryId, Role)],
    ) -> Result<(), GroupStoreError>;

    /// Replaces the groups assigned to a repository.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn set_repository_groups(
        &self,
        repository_id: RepositoryId,
        grants: &[(GroupId, Role)],
    ) -> Result<(), GroupStoreError>;

    /// Group access to a specific repository.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn grants_for_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError>;

    /// Repositories assigned to a group.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn grants_for_group(
        &self,
        group_id: GroupId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError>;

    /// Every group–repository assignment.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] if the backend fails.
    async fn all_grants(&self) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError>;
}
