use async_trait::async_trait;
use ferrobox_domain::api_token::{ApiToken, ApiTokenName};
use ferrobox_domain::ids::{ApiTokenId, UserId};
use ferrobox_ports::api_token_store::{ApiTokenRecord, ApiTokenStore, ApiTokenStoreError};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Adaptador de [`ApiTokenStore`] contra `PostgreSQL`.
pub struct PostgresApiTokenStore {
    pool: PgPool,
}

impl PostgresApiTokenStore {
    /// Construye el adaptador a partir de un `pool` de conexiones ya
    /// configurado.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> ApiTokenStoreError {
    ApiTokenStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn row_to_token(
    id: Uuid,
    user_id: Uuid,
    name: String,
    prefix: String,
    expires_at: Option<DateTime<Utc>>,
) -> Result<ApiToken, ApiTokenStoreError> {
    let name = ApiTokenName::parse(name).map_err(|err| backend_error(err.to_string()))?;
    Ok(ApiToken::from_parts(
        ApiTokenId::from(id),
        UserId::from(user_id),
        name,
        prefix,
    )
    .with_expires_at(expires_at))
}

#[async_trait]
impl ApiTokenStore for PostgresApiTokenStore {
    async fn save(&self, token: &ApiToken, token_hash: &str) -> Result<(), ApiTokenStoreError> {
        let id: Uuid = token.id().into();
        let user_id: Uuid = token.user_id().into();

        sqlx::query!(
            r#"
            INSERT INTO api_tokens (id, user_id, name, prefix, token_hash, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (id) DO UPDATE
            SET name = EXCLUDED.name,
                prefix = EXCLUDED.prefix,
                token_hash = EXCLUDED.token_hash,
                expires_at = EXCLUDED.expires_at
            "#,
            id,
            user_id,
            token.name().as_str(),
            token.prefix(),
            token_hash,
            token.expires_at(),
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiToken>, ApiTokenStoreError> {
        let row = sqlx::query!(
            r#"
            SELECT id, user_id, name, prefix, expires_at
            FROM api_tokens
            WHERE token_hash = $1
            "#,
            token_hash,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|row| row_to_token(row.id, row.user_id, row.name, row.prefix, row.expires_at))
            .transpose()
    }

    async fn list_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApiTokenRecord>, ApiTokenStoreError> {
        let user_id: Uuid = user_id.into();

        let rows = sqlx::query!(
            r#"
            SELECT id, user_id, name, prefix, created_at, expires_at
            FROM api_tokens
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| {
                let token =
                    row_to_token(row.id, row.user_id, row.name, row.prefix, row.expires_at)?;
                Ok(ApiTokenRecord {
                    token,
                    created_at_rfc3339: row.created_at.to_rfc3339(),
                    expires_at_rfc3339: row.expires_at.map(|at| at.to_rfc3339()),
                })
            })
            .collect()
    }

    async fn delete_for_user(
        &self,
        token_id: ApiTokenId,
        user_id: UserId,
    ) -> Result<bool, ApiTokenStoreError> {
        let token_id: Uuid = token_id.into();
        let user_id: Uuid = user_id.into();

        let result = sqlx::query!(
            r#"
            DELETE FROM api_tokens
            WHERE id = $1 AND user_id = $2
            "#,
            token_id,
            user_id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(result.rows_affected() > 0)
    }
}
