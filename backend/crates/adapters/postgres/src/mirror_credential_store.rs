use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::mirror_credential::MirrorCredential;
use ferrobox_ports::mirror_credential_store::{MirrorCredentialStore, MirrorCredentialStoreError};
use sqlx::PgPool;
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

/// [`MirrorCredentialStore`] adapter against `PostgreSQL`.
pub struct PostgresMirrorCredentialStore {
    pool: PgPool,
}

impl PostgresMirrorCredentialStore {
    /// Builds the adapter from an already configured connection `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> MirrorCredentialStoreError {
    MirrorCredentialStoreError::Backend(Box::new(RowConversionError(message.into())))
}

#[async_trait]
impl MirrorCredentialStore for PostgresMirrorCredentialStore {
    async fn save(
        &self,
        repository_id: RepositoryId,
        credential: &MirrorCredential,
    ) -> Result<(), MirrorCredentialStoreError> {
        let repository_id: Uuid = repository_id.into();
        sqlx::query(
            r#"
            INSERT INTO mirror_credentials (repository_id, username, secret)
            VALUES ($1, $2, $3)
            ON CONFLICT (repository_id) DO UPDATE
            SET username = EXCLUDED.username,
                secret = EXCLUDED.secret,
                updated_at = now()
            "#,
        )
        .bind(repository_id)
        .bind(credential.username())
        .bind(credential.secret())
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
    }

    async fn find(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Option<MirrorCredential>, MirrorCredentialStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = sqlx::query(
            r#"
            SELECT username, secret
            FROM mirror_credentials
            WHERE repository_id = $1
            "#,
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let username: String = row
            .try_get("username")
            .map_err(|err| backend_error(err.to_string()))?;
        let secret: String = row
            .try_get("secret")
            .map_err(|err| backend_error(err.to_string()))?;
        MirrorCredential::parse(username, secret)
            .map(Some)
            .map_err(|err| backend_error(err.to_string()))
    }

    async fn delete(
        &self,
        repository_id: RepositoryId,
    ) -> Result<bool, MirrorCredentialStoreError> {
        let repository_id: Uuid = repository_id.into();
        let result = sqlx::query(
            r#"
            DELETE FROM mirror_credentials
            WHERE repository_id = $1
            "#,
        )
        .bind(repository_id)
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        Ok(result.rows_affected() > 0)
    }
}
