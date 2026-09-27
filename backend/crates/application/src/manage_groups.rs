use std::collections::HashSet;
use std::sync::Arc;

use ferrobox_domain::group::{Group, GroupName, RepositoryAccess};
use ferrobox_domain::ids::{GroupId, RepositoryId, UserId};
use ferrobox_domain::repository::Repository;
use ferrobox_domain::user::{Role, User};
use ferrobox_ports::group_store::{GroupStore, GroupStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use thiserror::Error;

/// Summary of a group for listings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSummary {
    /// The group.
    pub group: Group,
    /// Number of members.
    pub member_count: usize,
    /// Number of assigned repositories.
    pub repository_count: usize,
    /// Usernames of the members, sorted.
    pub member_names: Vec<String>,
    /// Names of the assigned repositories, sorted.
    pub repository_names: Vec<String>,
}

/// Detail of a group: members and repositories.
#[derive(Debug, Clone)]
pub struct GroupDetail {
    /// The group.
    pub group: Group,
    /// Member users.
    pub members: Vec<User>,
    /// Assigned repositories, with the group's role on each.
    pub repositories: Vec<(Repository, Role)>,
}

/// Group a user belongs to, with the repositories it grants them.
/// Does not include the other members.
#[derive(Debug, Clone)]
pub struct GroupMembership {
    /// The group.
    pub group: Group,
    /// Repositories assigned to this group, with the granted role.
    pub repositories: Vec<(Repository, Role)>,
}

/// Repository visibility for a user.
#[derive(Debug, Clone)]
pub struct RepositoryVisibility {
    all: bool,
    ids: HashSet<RepositoryId>,
}

impl RepositoryVisibility {
    /// The user sees every repository (Admin, or a user with no
    /// groups when there are still no restrictions).
    #[must_use]
    pub fn all() -> Self {
        Self {
            all: true,
            ids: HashSet::new(),
        }
    }

    /// The user sees only this set.
    #[must_use]
    pub fn only(ids: HashSet<RepositoryId>) -> Self {
        Self { all: false, ids }
    }

    /// `true` if the repository is visible.
    #[must_use]
    pub fn contains(&self, id: RepositoryId) -> bool {
        self.all || self.ids.contains(&id)
    }
}

/// Reasons a group operation can fail.
#[derive(Debug, Error)]
pub enum GroupError {
    /// The group does not exist.
    #[error("group not found")]
    GroupNotFound,

    /// A given user does not exist.
    #[error("user not found")]
    UserNotFound,

    /// A given repository does not exist.
    #[error("repository not found")]
    RepositoryNotFound,

    /// A group cannot have the `admin` role on a repository.
    #[error("group repository role must be reader or developer")]
    InvalidGroupRole,

    /// Failed to persist groups.
    #[error(transparent)]
    Groups(#[from] GroupStoreError),

    /// Failed to query users.
    #[error(transparent)]
    Users(#[from] UserStoreError),

    /// Failed to query repositories.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),
}

/// Service for groups and repository access.
#[allow(clippy::struct_field_names)]
#[derive(Clone)]
pub struct GroupService {
    group_store: Arc<dyn GroupStore>,
    user_store: Arc<dyn UserStore>,
    repository_store: Arc<dyn RepositoryStore>,
}

impl GroupService {
    /// Builds the service from its ports.
    #[must_use]
    pub fn new(
        group_store: Arc<dyn GroupStore>,
        user_store: Arc<dyn UserStore>,
        repository_store: Arc<dyn RepositoryStore>,
    ) -> Self {
        Self {
            group_store,
            user_store,
            repository_store,
        }
    }

    /// Creates an empty group.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if the name is duplicated or persistence fails.
    pub async fn create(&self, name: GroupName) -> Result<Group, GroupError> {
        let group = Group::new(name);
        self.group_store.save(&group).await?;
        Ok(group)
    }

    /// Lists groups with counts and names of members and repositories.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if a port fails.
    pub async fn list(&self) -> Result<Vec<GroupSummary>, GroupError> {
        let groups = self.group_store.find_all().await?;
        let mut summaries = Vec::with_capacity(groups.len());
        for group in groups {
            let member_names = self.member_names(group.id()).await?;
            let repository_names = self.repository_names(group.id()).await?;
            summaries.push(GroupSummary {
                member_count: member_names.len(),
                repository_count: repository_names.len(),
                member_names,
                repository_names,
                group,
            });
        }
        Ok(summaries)
    }

    /// Groups `user_id` belongs to, with the repositories those
    /// groups grant. Does not include the other members.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if a port fails.
    pub async fn memberships_for(
        &self,
        user_id: UserId,
    ) -> Result<Vec<GroupMembership>, GroupError> {
        let group_ids = self.group_store.groups_for_user(user_id).await?;
        let mut memberships = Vec::with_capacity(group_ids.len());
        for group_id in group_ids {
            let Some(group) = self.group_store.find_by_id(group_id).await? else {
                continue;
            };
            let repositories = self.granted_repositories(group_id).await?;
            memberships.push(GroupMembership {
                group,
                repositories,
            });
        }
        memberships.sort_by(|left, right| {
            left.group.name().as_str().cmp(right.group.name().as_str())
        });
        Ok(memberships)
    }

    /// Detail of a group.
    ///
    /// # Errors
    ///
    /// [`GroupError::GroupNotFound`] or a persistence failure.
    pub async fn get(&self, id: GroupId) -> Result<GroupDetail, GroupError> {
        let group = self
            .group_store
            .find_by_id(id)
            .await?
            .ok_or(GroupError::GroupNotFound)?;
        let member_ids = self.group_store.members(id).await?;
        let mut members = Vec::with_capacity(member_ids.len());
        for user_id in member_ids {
            if let Some(user) = self.user_store.find_by_id(user_id).await? {
                members.push(user);
            }
        }
        members.sort_by(|a, b| a.username().as_str().cmp(b.username().as_str()));

        let repositories = self.granted_repositories(id).await?;

        Ok(GroupDetail {
            group,
            members,
            repositories,
        })
    }

    /// Deletes a group.
    ///
    /// # Errors
    ///
    /// [`GroupError::GroupNotFound`] or a persistence failure.
    pub async fn delete(&self, id: GroupId) -> Result<(), GroupError> {
        if self.group_store.delete(id).await? {
            Ok(())
        } else {
            Err(GroupError::GroupNotFound)
        }
    }

    /// Replaces the group members.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if the group or a user does not exist.
    pub async fn set_members(&self, id: GroupId, user_ids: Vec<UserId>) -> Result<(), GroupError> {
        self.ensure_group(id).await?;
        for user_id in &user_ids {
            if self.user_store.find_by_id(*user_id).await?.is_none() {
                return Err(GroupError::UserNotFound);
            }
        }
        self.group_store.set_members(id, &user_ids).await?;
        Ok(())
    }

    /// Replaces the repositories assigned to a group.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if the group or a repository does not exist, or
    /// the role is not `reader`/`developer`.
    pub async fn set_group_repositories(
        &self,
        id: GroupId,
        grants: Vec<(RepositoryId, Role)>,
    ) -> Result<(), GroupError> {
        self.ensure_group(id).await?;
        self.validate_grants(&grants).await?;
        self.group_store.set_group_repositories(id, &grants).await?;
        Ok(())
    }

    /// Groups assigned to a repository.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if a port fails.
    pub async fn repository_grants(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<(Group, Role)>, GroupError> {
        let grants = self.group_store.grants_for_repository(repository_id).await?;
        let mut result = Vec::with_capacity(grants.len());
        for grant in grants {
            if let Some(group) = self.group_store.find_by_id(grant.group_id).await? {
                result.push((group, grant.role));
            }
        }
        result.sort_by(|a, b| a.0.name().as_str().cmp(b.0.name().as_str()));
        Ok(result)
    }

    /// Replaces the groups of a repository.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if the repository or a group does not exist.
    pub async fn set_repository_groups(
        &self,
        repository_id: RepositoryId,
        grants: Vec<(GroupId, Role)>,
    ) -> Result<(), GroupError> {
        if self.repository_store.find_by_id(repository_id).await?.is_none() {
            return Err(GroupError::RepositoryNotFound);
        }
        for (group_id, role) in &grants {
            self.ensure_group(*group_id).await?;
            if RepositoryAccess::from_group_role(*role).is_none() {
                return Err(GroupError::InvalidGroupRole);
            }
        }
        self.group_store
            .set_repository_groups(repository_id, &grants)
            .await?;
        Ok(())
    }

    /// Effective access of `user` to the repository.
    ///
    /// An instance Admin always writes. If the repository has groups,
    /// only those groups (and the Admin) count. If it has no groups,
    /// the instance role applies **except** when the user already
    /// belongs to some group: then they only see the repositories
    /// assigned to their groups.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if a port fails.
    pub async fn access_on(
        &self,
        user: &User,
        repository_id: RepositoryId,
    ) -> Result<Option<RepositoryAccess>, GroupError> {
        if user.role() == Role::Admin {
            return Ok(Some(RepositoryAccess::Write));
        }

        let grants = self.group_store.grants_for_repository(repository_id).await?;
        if grants.is_empty() {
            if self.belongs_to_any_group(user.id()).await? {
                return Ok(None);
            }
            return Ok(Some(instance_access(user.role())));
        }

        let memberships: HashSet<GroupId> = self
            .group_store
            .groups_for_user(user.id())
            .await?
            .into_iter()
            .collect();

        let mut best: Option<RepositoryAccess> = None;
        for grant in grants {
            if memberships.contains(&grant.group_id)
                && let Some(access) = RepositoryAccess::from_group_role(grant.role)
            {
                best = Some(best.map_or(access, |current| current.max(access)));
            }
        }
        Ok(best)
    }

    /// `true` if the repository has at least one assigned group.
    ///
    /// # Errors
    ///
    /// [`GroupError`] if a port fails.
    pub async fn is_restricted(&self, repository_id: RepositoryId) -> Result<bool, GroupError> {
        Ok(!self
            .group_store
            .grants_for_repository(repository_id)
            .await?
            .is_empty())
    }

    /// Repositories that `user` can see.
    ///
    /// An Admin sees all. A user who belongs to some group only sees
    /// the repositories assigned to those groups. Someone in no group
    /// sees unrestricted repositories (no groups).
    ///
    /// # Errors
    ///
    /// [`GroupError`] if a port fails.
    pub async fn visibility(&self, user: &User) -> Result<RepositoryVisibility, GroupError> {
        if user.role() == Role::Admin {
            return Ok(RepositoryVisibility::all());
        }

        let memberships: HashSet<GroupId> = self
            .group_store
            .groups_for_user(user.id())
            .await?
            .into_iter()
            .collect();
        let grants = self.group_store.all_grants().await?;

        if !memberships.is_empty() {
            let visible = grants
                .into_iter()
                .filter(|grant| memberships.contains(&grant.group_id))
                .map(|grant| grant.repository_id)
                .collect();
            return Ok(RepositoryVisibility::only(visible));
        }

        if grants.is_empty() {
            return Ok(RepositoryVisibility::all());
        }

        let restricted: HashSet<RepositoryId> =
            grants.iter().map(|grant| grant.repository_id).collect();
        let visible = self
            .repository_store
            .find_all()
            .await?
            .into_iter()
            .map(|repository| repository.id())
            .filter(|id| !restricted.contains(id))
            .collect();
        Ok(RepositoryVisibility::only(visible))
    }

    async fn belongs_to_any_group(&self, user_id: UserId) -> Result<bool, GroupError> {
        Ok(!self.group_store.groups_for_user(user_id).await?.is_empty())
    }

    async fn member_names(&self, group_id: GroupId) -> Result<Vec<String>, GroupError> {
        let member_ids = self.group_store.members(group_id).await?;
        let mut names = Vec::with_capacity(member_ids.len());
        for user_id in member_ids {
            if let Some(user) = self.user_store.find_by_id(user_id).await? {
                names.push(user.username().to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    async fn repository_names(&self, group_id: GroupId) -> Result<Vec<String>, GroupError> {
        Ok(self
            .granted_repositories(group_id)
            .await?
            .into_iter()
            .map(|(repository, _)| repository.name().to_string())
            .collect())
    }

    async fn granted_repositories(
        &self,
        group_id: GroupId,
    ) -> Result<Vec<(Repository, Role)>, GroupError> {
        let grants = self.group_store.grants_for_group(group_id).await?;
        let mut repositories = Vec::with_capacity(grants.len());
        for grant in grants {
            if let Some(repository) = self.repository_store.find_by_id(grant.repository_id).await?
            {
                repositories.push((repository, grant.role));
            }
        }
        repositories.sort_by(|left, right| left.0.name().as_str().cmp(right.0.name().as_str()));
        Ok(repositories)
    }

    async fn ensure_group(&self, id: GroupId) -> Result<Group, GroupError> {
        self.group_store
            .find_by_id(id)
            .await?
            .ok_or(GroupError::GroupNotFound)
    }

    async fn validate_grants(&self, grants: &[(RepositoryId, Role)]) -> Result<(), GroupError> {
        for (repository_id, role) in grants {
            if RepositoryAccess::from_group_role(*role).is_none() {
                return Err(GroupError::InvalidGroupRole);
            }
            if self
                .repository_store
                .find_by_id(*repository_id)
                .await?
                .is_none()
            {
                return Err(GroupError::RepositoryNotFound);
            }
        }
        Ok(())
    }
}

fn instance_access(role: Role) -> RepositoryAccess {
    if role.can_write_artifacts() {
        RepositoryAccess::Write
    } else {
        RepositoryAccess::Read
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::user::{Role, Username};
    use ferrobox_ports::repository_store::RepositoryStore;
    use ferrobox_ports::user_store::UserStore;

    use crate::auth_crypto::hash_password;
    use crate::test_support::{InMemoryGroupStore, InMemoryRepositoryStore, InMemoryUserStore, forge};

    use super::*;

    async fn seeded() -> (
        GroupService,
        User,
        User,
        Repository,
        Arc<InMemoryRepositoryStore>,
    ) {
        let users = Arc::new(InMemoryUserStore::default());
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let service = GroupService::new(groups, users.clone(), repos.clone());

        let admin = User::new(Username::parse("admin").unwrap(), Role::Admin);
        let reader = User::new(Username::parse("reader").unwrap(), Role::Reader);
        let hash = hash_password("Secret1a").unwrap();
        users.save_with_password_hash(&admin, &hash).await.unwrap();
        users.save_with_password_hash(&reader, &hash).await.unwrap();

        let repo = forge("crates");
        repos.save(&repo).await.unwrap();

        (service, admin, reader, repo, repos)
    }

    #[tokio::test]
    async fn open_repository_follows_instance_role() {
        let (service, _admin, reader, repo, _) = seeded().await;
        assert_eq!(
            service.access_on(&reader, repo.id()).await.unwrap(),
            Some(RepositoryAccess::Read)
        );
    }

    #[tokio::test]
    async fn restricted_repository_hides_from_outsiders() {
        let (service, admin, reader, repo, _) = seeded().await;
        let group = service
            .create(GroupName::parse("team-a").unwrap())
            .await
            .unwrap();
        service
            .set_members(group.id(), vec![admin.id()])
            .await
            .unwrap();
        service
            .set_group_repositories(group.id(), vec![(repo.id(), Role::Developer)])
            .await
            .unwrap();

        assert_eq!(
            service.access_on(&admin, repo.id()).await.unwrap(),
            Some(RepositoryAccess::Write)
        );
        assert_eq!(service.access_on(&reader, repo.id()).await.unwrap(), None);

        let visibility = service.visibility(&reader).await.unwrap();
        assert!(!visibility.contains(repo.id()));
    }

    #[tokio::test]
    async fn group_developer_role_grants_write_to_a_reader() {
        let (service, _admin, reader, repo, _) = seeded().await;
        let group = service
            .create(GroupName::parse("writers").unwrap())
            .await
            .unwrap();
        service
            .set_members(group.id(), vec![reader.id()])
            .await
            .unwrap();
        service
            .set_group_repositories(group.id(), vec![(repo.id(), Role::Developer)])
            .await
            .unwrap();

        assert_eq!(
            service.access_on(&reader, repo.id()).await.unwrap(),
            Some(RepositoryAccess::Write)
        );
    }

    #[tokio::test]
    async fn rejects_admin_as_group_repository_role() {
        let (service, _, _, repo, _) = seeded().await;
        let group = service
            .create(GroupName::parse("oops").unwrap())
            .await
            .unwrap();
        let err = service
            .set_group_repositories(group.id(), vec![(repo.id(), Role::Admin)])
            .await
            .unwrap_err();
        assert!(matches!(err, GroupError::InvalidGroupRole));
    }

    #[tokio::test]
    async fn open_repository_stays_visible_when_another_is_restricted() {
        let (service, admin, reader, repo, repos) = seeded().await;
        let open = forge("public-crates");
        repos.save(&open).await.unwrap();

        let group = service
            .create(GroupName::parse("team-a").unwrap())
            .await
            .unwrap();
        service
            .set_members(group.id(), vec![admin.id()])
            .await
            .unwrap();
        service
            .set_group_repositories(group.id(), vec![(repo.id(), Role::Developer)])
            .await
            .unwrap();

        let visibility = service.visibility(&reader).await.unwrap();
        assert!(!visibility.contains(repo.id()));
        assert!(visibility.contains(open.id()));
    }

    #[tokio::test]
    async fn group_member_only_sees_granted_repositories() {
        let (service, _admin, reader, repo, repos) = seeded().await;
        let open = forge("public-crates");
        repos.save(&open).await.unwrap();

        let group = service
            .create(GroupName::parse("team-a").unwrap())
            .await
            .unwrap();
        service
            .set_members(group.id(), vec![reader.id()])
            .await
            .unwrap();
        service
            .set_group_repositories(group.id(), vec![(repo.id(), Role::Developer)])
            .await
            .unwrap();

        let visibility = service.visibility(&reader).await.unwrap();
        assert!(visibility.contains(repo.id()));
        assert!(!visibility.contains(open.id()));
        assert_eq!(
            service.access_on(&reader, repo.id()).await.unwrap(),
            Some(RepositoryAccess::Write)
        );
        assert_eq!(service.access_on(&reader, open.id()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn instance_developer_in_a_group_is_sandboxed_to_grants() {
        let users = Arc::new(InMemoryUserStore::default());
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let service = GroupService::new(
            Arc::new(InMemoryGroupStore::default()),
            users.clone(),
            repos.clone(),
        );
        let hash = hash_password("Secret1a").unwrap();
        let developer = User::new(Username::parse("developer").unwrap(), Role::Developer);
        users
            .save_with_password_hash(&developer, &hash)
            .await
            .unwrap();

        let granted = forge("demo-repo");
        let other = forge("other-repo");
        repos.save(&granted).await.unwrap();
        repos.save(&other).await.unwrap();

        let group = service
            .create(GroupName::parse("test").unwrap())
            .await
            .unwrap();
        service
            .set_members(group.id(), vec![developer.id()])
            .await
            .unwrap();
        service
            .set_group_repositories(group.id(), vec![(granted.id(), Role::Developer)])
            .await
            .unwrap();

        let visibility = service.visibility(&developer).await.unwrap();
        assert!(visibility.contains(granted.id()));
        assert!(!visibility.contains(other.id()));
        assert_eq!(
            service.access_on(&developer, granted.id()).await.unwrap(),
            Some(RepositoryAccess::Write)
        );
        assert_eq!(
            service.access_on(&developer, other.id()).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn memberships_for_lists_only_the_users_groups_and_grants() {
        let (service, admin, reader, repo, repos) = seeded().await;
        let other = forge("other-repo");
        repos.save(&other).await.unwrap();

        let team = service
            .create(GroupName::parse("team-a").unwrap())
            .await
            .unwrap();
        let outsiders = service
            .create(GroupName::parse("outsiders").unwrap())
            .await
            .unwrap();
        service
            .set_members(team.id(), vec![reader.id()])
            .await
            .unwrap();
        service
            .set_members(outsiders.id(), vec![admin.id()])
            .await
            .unwrap();
        service
            .set_group_repositories(team.id(), vec![(repo.id(), Role::Developer)])
            .await
            .unwrap();
        service
            .set_group_repositories(outsiders.id(), vec![(other.id(), Role::Reader)])
            .await
            .unwrap();

        let memberships = service.memberships_for(reader.id()).await.unwrap();
        assert_eq!(memberships.len(), 1);
        assert_eq!(memberships[0].group.id(), team.id());
        assert_eq!(memberships[0].repositories.len(), 1);
        assert_eq!(memberships[0].repositories[0].0.id(), repo.id());
        assert_eq!(memberships[0].repositories[0].1, Role::Developer);

        let listed = service.list().await.unwrap();
        let team_summary = listed
            .iter()
            .find(|summary| summary.group.id() == team.id())
            .expect("team-a");
        assert_eq!(team_summary.member_names, vec!["reader".to_string()]);
        assert_eq!(team_summary.repository_names, vec!["crates".to_string()]);
    }
}
