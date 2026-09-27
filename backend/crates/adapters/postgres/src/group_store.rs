use async_trait::async_trait;
use ferrobox_domain::group::{Group, GroupName};
use ferrobox_domain::ids::{GroupId, RepositoryId, UserId};
use ferrobox_domain::user::Role;
use ferrobox_ports::group_store::{GroupStore, GroupStoreError, RepositoryGroupGrant};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// [`GroupStore`] adapter against `PostgreSQL`.
pub struct PostgresGroupStore {
    pool: PgPool,
}

impl PostgresGroupStore {
    /// Builds the adapter from an already configured connection `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> GroupStoreError {
    GroupStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn translate_save_error(group: &Group, err: &sqlx::Error) -> GroupStoreError {
    if let sqlx::Error::Database(db_err) = err
        && db_err.constraint() == Some("groups_name_key")
    {
        return GroupStoreError::DuplicateName(group.name().clone());
    }
    backend_error(err.to_string())
}

fn row_to_group(id: Uuid, name: String) -> Result<Group, GroupStoreError> {
    let name = GroupName::parse(name).map_err(|err| backend_error(err.to_string()))?;
    Ok(Group::from_parts(GroupId::from(id), name))
}

fn row_to_grant(
    repository_id: Uuid,
    group_id: Uuid,
    role: &str,
) -> Result<RepositoryGroupGrant, GroupStoreError> {
    let role = Role::parse(role).map_err(|err| backend_error(err.to_string()))?;
    Ok(RepositoryGroupGrant {
        repository_id: RepositoryId::from(repository_id),
        group_id: GroupId::from(group_id),
        role,
    })
}

#[async_trait]
impl GroupStore for PostgresGroupStore {
    async fn save(&self, group: &Group) -> Result<(), GroupStoreError> {
        let id: Uuid = group.id().into();

        sqlx::query!(
            r#"
            INSERT INTO groups (id, name)
            VALUES ($1, $2)
            ON CONFLICT (id) DO UPDATE
            SET name = EXCLUDED.name
            "#,
            id,
            group.name().as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(|err| translate_save_error(group, &err))?;

        Ok(())
    }

    async fn find_by_id(&self, id: GroupId) -> Result<Option<Group>, GroupStoreError> {
        let id: Uuid = id.into();

        let row = sqlx::query!(
            r#"
            SELECT id, name
            FROM groups
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|row| row_to_group(row.id, row.name)).transpose()
    }

    async fn find_by_name(&self, name: &GroupName) -> Result<Option<Group>, GroupStoreError> {
        let row = sqlx::query!(
            r#"
            SELECT id, name
            FROM groups
            WHERE name = $1
            "#,
            name.as_str(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|row| row_to_group(row.id, row.name)).transpose()
    }

    async fn find_all(&self) -> Result<Vec<Group>, GroupStoreError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, name
            FROM groups
            ORDER BY name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| row_to_group(row.id, row.name))
            .collect()
    }

    async fn delete(&self, id: GroupId) -> Result<bool, GroupStoreError> {
        let id: Uuid = id.into();

        let result = sqlx::query!(
            r#"
            DELETE FROM groups
            WHERE id = $1
            "#,
            id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_members(
        &self,
        group_id: GroupId,
        user_ids: &[UserId],
    ) -> Result<(), GroupStoreError> {
        let group_id: Uuid = group_id.into();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| backend_error(err.to_string()))?;

        sqlx::query!(
            r#"
            DELETE FROM group_members
            WHERE group_id = $1
            "#,
            group_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        for user_id in user_ids {
            let user_id: Uuid = (*user_id).into();
            sqlx::query!(
                r#"
                INSERT INTO group_members (group_id, user_id)
                VALUES ($1, $2)
                "#,
                group_id,
                user_id,
            )
            .execute(&mut *tx)
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }

    async fn members(&self, group_id: GroupId) -> Result<Vec<UserId>, GroupStoreError> {
        let group_id: Uuid = group_id.into();

        let rows = sqlx::query!(
            r#"
            SELECT user_id
            FROM group_members
            WHERE group_id = $1
            "#,
            group_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| UserId::from(row.user_id))
            .collect())
    }

    async fn groups_for_user(&self, user_id: UserId) -> Result<Vec<GroupId>, GroupStoreError> {
        let user_id: Uuid = user_id.into();

        let rows = sqlx::query!(
            r#"
            SELECT group_id
            FROM group_members
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| GroupId::from(row.group_id))
            .collect())
    }

    async fn add_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<(), GroupStoreError> {
        let group_id: Uuid = group_id.into();
        let user_id: Uuid = user_id.into();
        sqlx::query!(
            r#"
            INSERT INTO group_members (group_id, user_id)
            VALUES ($1, $2)
            ON CONFLICT (group_id, user_id) DO NOTHING
            "#,
            group_id,
            user_id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }

    async fn remove_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<(), GroupStoreError> {
        let group_id: Uuid = group_id.into();
        let user_id: Uuid = user_id.into();
        sqlx::query!(
            r#"
            DELETE FROM group_members
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }

    async fn sso_memberships(&self, user_id: UserId) -> Result<Vec<GroupId>, GroupStoreError> {
        let user_id: Uuid = user_id.into();
        let rows = sqlx::query!(
            r#"
            SELECT group_id
            FROM sso_group_memberships
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| GroupId::from(row.group_id))
            .collect())
    }

    async fn set_sso_memberships(
        &self,
        user_id: UserId,
        group_ids: &[GroupId],
    ) -> Result<(), GroupStoreError> {
        let user_id: Uuid = user_id.into();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        sqlx::query!(
            r#"
            DELETE FROM sso_group_memberships
            WHERE user_id = $1
            "#,
            user_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        for group_id in group_ids {
            let group_id: Uuid = (*group_id).into();
            sqlx::query!(
                r#"
                INSERT INTO sso_group_memberships (user_id, group_id)
                VALUES ($1, $2)
                "#,
                user_id,
                group_id,
            )
            .execute(&mut *tx)
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }

    async fn set_group_repositories(
        &self,
        group_id: GroupId,
        grants: &[(RepositoryId, Role)],
    ) -> Result<(), GroupStoreError> {
        let group_id: Uuid = group_id.into();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| backend_error(err.to_string()))?;

        sqlx::query!(
            r#"
            DELETE FROM repository_group_access
            WHERE group_id = $1
            "#,
            group_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        for (repository_id, role) in grants {
            let repository_id: Uuid = (*repository_id).into();
            sqlx::query!(
                r#"
                INSERT INTO repository_group_access (repository_id, group_id, role)
                VALUES ($1, $2, $3)
                "#,
                repository_id,
                group_id,
                role.as_str(),
            )
            .execute(&mut *tx)
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }

    async fn set_repository_groups(
        &self,
        repository_id: RepositoryId,
        grants: &[(GroupId, Role)],
    ) -> Result<(), GroupStoreError> {
        let repository_id: Uuid = repository_id.into();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| backend_error(err.to_string()))?;

        sqlx::query!(
            r#"
            DELETE FROM repository_group_access
            WHERE repository_id = $1
            "#,
            repository_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        for (group_id, role) in grants {
            let group_id: Uuid = (*group_id).into();
            sqlx::query!(
                r#"
                INSERT INTO repository_group_access (repository_id, group_id, role)
                VALUES ($1, $2, $3)
                "#,
                repository_id,
                group_id,
                role.as_str(),
            )
            .execute(&mut *tx)
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }

    async fn grants_for_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError> {
        let repository_id: Uuid = repository_id.into();

        let rows = sqlx::query!(
            r#"
            SELECT repository_id, group_id, role
            FROM repository_group_access
            WHERE repository_id = $1
            "#,
            repository_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| row_to_grant(row.repository_id, row.group_id, &row.role))
            .collect()
    }

    async fn grants_for_group(
        &self,
        group_id: GroupId,
    ) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError> {
        let group_id: Uuid = group_id.into();

        let rows = sqlx::query!(
            r#"
            SELECT repository_id, group_id, role
            FROM repository_group_access
            WHERE group_id = $1
            "#,
            group_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| row_to_grant(row.repository_id, row.group_id, &row.role))
            .collect()
    }

    async fn all_grants(&self) -> Result<Vec<RepositoryGroupGrant>, GroupStoreError> {
        let rows = sqlx::query!(
            r#"
            SELECT repository_id, group_id, role
            FROM repository_group_access
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| row_to_grant(row.repository_id, row.group_id, &row.role))
            .collect()
    }
}
