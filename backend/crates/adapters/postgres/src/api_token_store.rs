use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrobox_domain::api_token::{ApiToken, ApiTokenName, TokenRepositories, TokenScopes};
use ferrobox_domain::ids::{ApiTokenId, UserId};
use ferrobox_ports::api_token_store::{ApiTokenRecord, ApiTokenStore, ApiTokenStoreError};
use sqlx::PgPool;
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

/// [`ApiTokenStore`] adapter against `PostgreSQL`.
pub struct PostgresApiTokenStore {
    pool: PgPool,
}

impl PostgresApiTokenStore {
    /// Builds the adapter from an already configured connection `pool`.
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

fn parse_scopes(raw: &str) -> Result<TokenScopes, ApiTokenStoreError> {
    if raw.is_empty() {
        return Ok(TokenScopes::unrestricted());
    }
    TokenScopes::parse(raw.split(',').filter(|part| !part.is_empty()))
        .map_err(|err| backend_error(err.to_string()))
}

fn parse_repositories(raw: &str) -> Result<TokenRepositories, ApiTokenStoreError> {
    TokenRepositories::parse_stored(raw).map_err(|err| backend_error(err.to_string()))
}

fn row_to_token(
    id: Uuid,
    user_id: Uuid,
    name: String,
    prefix: String,
    expires_at: Option<DateTime<Utc>>,
    scopes: &str,
    repository_ids: &str,
) -> Result<ApiToken, ApiTokenStoreError> {
    let name = ApiTokenName::parse(name).map_err(|err| backend_error(err.to_string()))?;
    Ok(
        ApiToken::from_parts(ApiTokenId::from(id), UserId::from(user_id), name, prefix)
            .with_expires_at(expires_at)
            .with_scopes(parse_scopes(scopes)?)
            .with_repositories(parse_repositories(repository_ids)?),
    )
}

fn token_from_row(row: &sqlx::postgres::PgRow) -> Result<ApiToken, ApiTokenStoreError> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|err| backend_error(err.to_string()))?;
    let user_id: Uuid = row
        .try_get("user_id")
        .map_err(|err| backend_error(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| backend_error(err.to_string()))?;
    let prefix: String = row
        .try_get("prefix")
        .map_err(|err| backend_error(err.to_string()))?;
    let expires_at: Option<DateTime<Utc>> = row
        .try_get("expires_at")
        .map_err(|err| backend_error(err.to_string()))?;
    let scopes: String = row
        .try_get("scopes")
        .map_err(|err| backend_error(err.to_string()))?;
    let repository_ids: String = row
        .try_get("repository_ids")
        .map_err(|err| backend_error(err.to_string()))?;
    row_to_token(
        id,
        user_id,
        name,
        prefix,
        expires_at,
        &scopes,
        &repository_ids,
    )
}

#[async_trait]
impl ApiTokenStore for PostgresApiTokenStore {
    async fn save(&self, token: &ApiToken, token_hash: &str) -> Result<(), ApiTokenStoreError> {
        let id: Uuid = token.id().into();
        let user_id: Uuid = token.user_id().into();
        let scopes = token.scopes().as_stored();
        let repository_ids = token.repositories().as_stored();

        sqlx::query(
            r#"
            INSERT INTO api_tokens (id, user_id, name, prefix, token_hash, expires_at, scopes, repository_ids)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE
            SET name = EXCLUDED.name,
                prefix = EXCLUDED.prefix,
                token_hash = EXCLUDED.token_hash,
                expires_at = EXCLUDED.expires_at,
                scopes = EXCLUDED.scopes,
                repository_ids = EXCLUDED.repository_ids
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(token.name().as_str())
        .bind(token.prefix())
        .bind(token_hash)
        .bind(token.expires_at())
        .bind(scopes)
        .bind(repository_ids)
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiToken>, ApiTokenStoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, name, prefix, expires_at, scopes, repository_ids
            FROM api_tokens
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.as_ref().map(token_from_row).transpose()
    }

    async fn list_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApiTokenRecord>, ApiTokenStoreError> {
        let user_id: Uuid = user_id.into();

        let rows = sqlx::query(
            r#"
            SELECT id, user_id, name, prefix, created_at, expires_at, scopes, repository_ids
            FROM api_tokens
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.iter()
            .map(|row| {
                let created_at: DateTime<Utc> = row
                    .try_get("created_at")
                    .map_err(|err| backend_error(err.to_string()))?;
                let token = token_from_row(row)?;
                Ok(ApiTokenRecord {
                    token,
                    created_at_rfc3339: created_at.to_rfc3339(),
                    expires_at_rfc3339: row
                        .try_get::<Option<DateTime<Utc>>, _>("expires_at")
                        .map_err(|err| backend_error(err.to_string()))?
                        .map(|at| at.to_rfc3339()),
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

        let result = sqlx::query(
            r#"
            DELETE FROM api_tokens
            WHERE id = $1 AND user_id = $2
            "#,
        )
        .bind(token_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(result.rows_affected() > 0)
    }
}
