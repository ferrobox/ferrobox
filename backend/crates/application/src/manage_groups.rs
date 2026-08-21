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

/// Resumen de un grupo para listados.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSummary {
    /// El grupo.
    pub group: Group,
    /// Número de miembros.
    pub member_count: usize,
    /// Número de repositorios asignados.
    pub repository_count: usize,
}

/// Detalle de un grupo: miembros y repositorios.
#[derive(Debug, Clone)]
pub struct GroupDetail {
    /// El grupo.
    pub group: Group,
    /// Usuarios miembros.
    pub members: Vec<User>,
    /// Repositorios asignados, con el rol del grupo en cada uno.
    pub repositories: Vec<(Repository, Role)>,
}

/// Visibilidad de repositorios para un usuario.
#[derive(Debug, Clone)]
pub struct RepositoryVisibility {
    all: bool,
    ids: HashSet<RepositoryId>,
}

impl RepositoryVisibility {
    /// El usuario ve todos los repositorios (Admin, o un usuario sin
    /// grupos cuando todavía no hay restricciones).
    #[must_use]
    pub fn all() -> Self {
        Self {
            all: true,
            ids: HashSet::new(),
        }
    }

    /// El usuario solo ve este conjunto.
    #[must_use]
    pub fn only(ids: HashSet<RepositoryId>) -> Self {
        Self { all: false, ids }
    }

    /// `true` si el repositorio es visible.
    #[must_use]
    pub fn contains(&self, id: RepositoryId) -> bool {
        self.all || self.ids.contains(&id)
    }
}

/// Motivos por los que una operación de grupos puede fallar.
#[derive(Debug, Error)]
pub enum GroupError {
    /// El grupo no existe.
    #[error("group not found")]
    GroupNotFound,

    /// Un usuario indicado no existe.
    #[error("user not found")]
    UserNotFound,

    /// Un repositorio indicado no existe.
    #[error("repository not found")]
    RepositoryNotFound,

    /// Un grupo no puede tener rol `admin` en un repositorio.
    #[error("group repository role must be reader or developer")]
    InvalidGroupRole,

    /// Fallo al persistir grupos.
    #[error(transparent)]
    Groups(#[from] GroupStoreError),

    /// Fallo al consultar usuarios.
    #[error(transparent)]
    Users(#[from] UserStoreError),

    /// Fallo al consultar repositorios.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),
}

/// Servicio de grupos y de acceso a repositorios.
#[allow(clippy::struct_field_names)]
#[derive(Clone)]
pub struct GroupService {
    group_store: Arc<dyn GroupStore>,
    user_store: Arc<dyn UserStore>,
    repository_store: Arc<dyn RepositoryStore>,
}

impl GroupService {
    /// Construye el servicio a partir de sus puertos.
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

    /// Crea un grupo vacío.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si el nombre está duplicado o falla la persistencia.
    pub async fn create(&self, name: GroupName) -> Result<Group, GroupError> {
        let group = Group::new(name);
        self.group_store.save(&group).await?;
        Ok(group)
    }

    /// Lista grupos con recuentos.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si falla un puerto.
    pub async fn list(&self) -> Result<Vec<GroupSummary>, GroupError> {
        let groups = self.group_store.find_all().await?;
        let mut summaries = Vec::with_capacity(groups.len());
        for group in groups {
            let member_count = self.group_store.members(group.id()).await?.len();
            let repository_count = self.group_store.grants_for_group(group.id()).await?.len();
            summaries.push(GroupSummary {
                group,
                member_count,
                repository_count,
            });
        }
        Ok(summaries)
    }

    /// Detalle de un grupo.
    ///
    /// # Errors
    ///
    /// [`GroupError::GroupNotFound`] o fallo de persistencia.
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

        let grants = self.group_store.grants_for_group(id).await?;
        let mut repositories = Vec::with_capacity(grants.len());
        for grant in grants {
            if let Some(repository) = self.repository_store.find_by_id(grant.repository_id).await?
            {
                repositories.push((repository, grant.role));
            }
        }
        repositories.sort_by(|a, b| a.0.name().as_str().cmp(b.0.name().as_str()));

        Ok(GroupDetail {
            group,
            members,
            repositories,
        })
    }

    /// Elimina un grupo.
    ///
    /// # Errors
    ///
    /// [`GroupError::GroupNotFound`] o fallo de persistencia.
    pub async fn delete(&self, id: GroupId) -> Result<(), GroupError> {
        if self.group_store.delete(id).await? {
            Ok(())
        } else {
            Err(GroupError::GroupNotFound)
        }
    }

    /// Sustituye los miembros del grupo.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si el grupo o algún usuario no existen.
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

    /// Sustituye los repositorios asignados a un grupo.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si el grupo o un repositorio no existen, o el rol
    /// no es `reader`/`developer`.
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

    /// Grupos asignados a un repositorio.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si falla un puerto.
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

    /// Sustituye los grupos de un repositorio.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si el repositorio o un grupo no existen.
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

    /// Acceso efectivo de `user` al repositorio.
    ///
    /// Un Admin de instancia siempre escribe. Si el repositorio tiene
    /// grupos, solo cuentan esos grupos (y el Admin). Si no tiene
    /// grupos, vale el rol de instancia **salvo** que el usuario ya
    /// pertenezca a algún grupo: entonces solo ve los repositorios
    /// asignados a sus grupos.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si falla un puerto.
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

    /// `true` si el repositorio tiene al menos un grupo asignado.
    ///
    /// # Errors
    ///
    /// [`GroupError`] si falla un puerto.
    pub async fn is_restricted(&self, repository_id: RepositoryId) -> Result<bool, GroupError> {
        Ok(!self
            .group_store
            .grants_for_repository(repository_id)
            .await?
            .is_empty())
    }

    /// Repositorios que `user` puede ver.
    ///
    /// Un Admin ve todos. Un usuario que pertenece a algún grupo solo
    /// ve los repositorios asignados a esos grupos. Quien no está en
    /// ningún grupo ve los repositorios sin restringir (sin grupos).
    ///
    /// # Errors
    ///
    /// [`GroupError`] si falla un puerto.
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
}
