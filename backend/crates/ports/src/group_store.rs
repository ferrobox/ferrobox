use async_trait::async_trait;
use ferrobox_domain::group::{Group, GroupName};
use ferrobox_domain::ids::{GroupId, RepositoryId, UserId};
use ferrobox_domain::user::Role;
use thiserror::Error;

/// Motivos por los que una operación de persistencia de grupos puede
/// fallar.
#[derive(Debug, Error)]
pub enum GroupStoreError {
    /// Ya existe un grupo con ese nombre.
    #[error("a group named '{0}' already exists")]
    DuplicateName(GroupName),

    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Un grupo asignado a un repositorio, con el rol en ese repositorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryGroupGrant {
    /// Repositorio al que aplica el acceso.
    pub repository_id: RepositoryId,
    /// Grupo que recibe el acceso.
    pub group_id: GroupId,
    /// Rol del grupo en ese repositorio (`reader` o `developer`).
    pub role: Role,
}

/// Puerto de persistencia de grupos, miembros y acceso a repositorios.
#[async_trait]
pub trait GroupStore: Send + Sync {
    /// Inserta o actualiza un grupo (nombre).
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::DuplicateName`] o [`GroupStoreError::Backend`].
    async fn save(&self, group: &Group) -> Result<(), GroupStoreError>;

    /// Busca un grupo por identificador.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn find_by_id(&self, id: GroupId) -> Result<Option<Group>, GroupStoreError>;

    /// Lista todos los grupos, ordenados por nombre.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn find_all(&self) -> Result<Vec<Group>, GroupStoreError>;

    /// Elimina un grupo y sus asociaciones. Devuelve `false` si no
    /// existía.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn delete(&self, id: GroupId) -> Result<bool, GroupStoreError>;

    /// Sustituye los miembros del grupo por `user_ids`.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn set_members(
        &self,
        group_id: GroupId,
        user_ids: &[UserId],
    ) -> Result<(), GroupStoreError>;

    /// Lista los miembros de un grupo.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn members(&self, group_id: GroupId) -> Result<Vec<UserId>, GroupStoreError>;

    /// Grupos a los que pertenece un usuario.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn groups_for_user(&self, user_id: UserId) -> Result<Vec<GroupId>, GroupStoreError>;

    /// Sustituye los repositorios asignados a un grupo.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn set_group_repositories(
        &self,
        group_id: GroupId,
        grants: &[(RepositoryId, Role)],
    ) -> Result<(), GroupStoreError>;

    /// Sustituye los grupos asignados a un repositorio.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn set_repository_groups(
        &self,
        repository_id: RepositoryId,
        grants: &[(GroupId, Role)],
    ) -> Result<(), GroupStoreError>;

    /// Acceso de grupos a un repositorio concreto.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn grants_for_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError>;

    /// Repositorios asignados a un grupo.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn grants_for_group(
        &self,
        group_id: GroupId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError>;

    /// Todas las asignaciones grupo–repositorio.
    ///
    /// # Errors
    ///
    /// [`GroupStoreError::Backend`] si el backend falla.
    async fn all_grants(&self) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError>;
}
