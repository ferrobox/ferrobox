use async_trait::async_trait;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use serde_json::json;
use sqlx::PgPool;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

/// Adaptador de [`RepositoryStore`] contra `PostgreSQL`.
pub struct PostgresRepositoryStore {
    pool: PgPool,
}

impl PostgresRepositoryStore {
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

fn backend_error(message: impl Into<String>) -> RepositoryStoreError {
    RepositoryStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn repository_kind_to_columns(kind: &RepositoryKind) -> (&'static str, serde_json::Value) {
    match kind {
        RepositoryKind::Forge => ("forge", json!({})),
        RepositoryKind::Mirror { upstream } => ("mirror", json!({ "upstream": upstream.as_str() })),
        RepositoryKind::Alloy { members } => (
            "alloy",
            json!({
                "members": members.iter().map(ToString::to_string).collect::<Vec<_>>(),
            }),
        ),
    }
}

fn row_to_repository(
    id: Uuid,
    name: String,
    kind: &str,
    kind_data: &serde_json::Value,
    ecosystem: &str,
) -> Result<Repository, RepositoryStoreError> {
    let repository_kind = match kind {
        "forge" => RepositoryKind::Forge,
        "mirror" => {
            let upstream = kind_data
                .get("upstream")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| backend_error("mirror row missing 'upstream'"))?;
            let upstream = Url::parse(upstream)
                .map_err(|err| backend_error(format!("invalid upstream URL: {err}")))?;
            RepositoryKind::Mirror { upstream }
        }
        "alloy" => {
            let members = kind_data
                .get("members")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| backend_error("alloy row missing 'members'"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .map(RepositoryId::from)
                        .ok_or_else(|| backend_error("invalid member id in alloy row"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            RepositoryKind::Alloy { members }
        }
        other => return Err(backend_error(format!("unknown repository kind: {other}"))),
    };

    let name = RepositoryName::parse(name).map_err(|err| backend_error(err.to_string()))?;
    let ecosystem =
        crate::ecosystem_column::from_column(ecosystem).map_err(backend_error)?;

    Repository::from_parts(RepositoryId::from(id), name, repository_kind, ecosystem)
        .map_err(|err| backend_error(err.to_string()))
}

fn translate_save_error(name: &RepositoryName, err: &sqlx::Error) -> RepositoryStoreError {
    if let sqlx::Error::Database(db_err) = err
        && db_err.constraint() == Some("repositories_name_key")
    {
        return RepositoryStoreError::DuplicateName(name.clone());
    }

    backend_error(err.to_string())
}

#[async_trait]
impl RepositoryStore for PostgresRepositoryStore {
    async fn save(&self, repository: &Repository) -> Result<(), RepositoryStoreError> {
        let (kind, kind_data) = repository_kind_to_columns(repository.kind());
        let ecosystem = crate::ecosystem_column::to_column(repository.ecosystem());
        let id: Uuid = repository.id().into();

        sqlx::query!(
            r#"
            INSERT INTO repositories (id, name, kind, kind_data, ecosystem)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE
            SET name = EXCLUDED.name,
                kind = EXCLUDED.kind,
                kind_data = EXCLUDED.kind_data,
                ecosystem = EXCLUDED.ecosystem
            "#,
            id,
            repository.name().as_str(),
            kind,
            kind_data,
            ecosystem,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| translate_save_error(repository.name(), &err))?;

        Ok(())
    }

    async fn find_by_id(
        &self,
        id: RepositoryId,
    ) -> Result<Option<Repository>, RepositoryStoreError> {
        let id: Uuid = id.into();

        let row = sqlx::query!(
            r#"SELECT id, name, kind, kind_data, ecosystem FROM repositories WHERE id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|r| row_to_repository(r.id, r.name, &r.kind, &r.kind_data, &r.ecosystem))
            .transpose()
    }

    async fn find_by_name(
        &self,
        name: &RepositoryName,
    ) -> Result<Option<Repository>, RepositoryStoreError> {
        let row = sqlx::query!(
            r#"SELECT id, name, kind, kind_data, ecosystem FROM repositories WHERE name = $1"#,
            name.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|r| row_to_repository(r.id, r.name, &r.kind, &r.kind_data, &r.ecosystem))
            .transpose()
    }

    async fn find_all(&self) -> Result<Vec<Repository>, RepositoryStoreError> {
        let rows = sqlx::query!(
            r#"SELECT id, name, kind, kind_data, ecosystem FROM repositories ORDER BY created_at"#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|r| row_to_repository(r.id, r.name, &r.kind, &r.kind_data, &r.ecosystem))
            .collect()
    }

    async fn delete(&self, id: RepositoryId) -> Result<(), RepositoryStoreError> {
        let id: Uuid = id.into();

        sqlx::query!(r#"DELETE FROM repositories WHERE id = $1"#, id)
            .execute(&self.pool)
            .await
            .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::RepositoryKind;
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    async fn pool_from_env() -> PgPool {
        let database_url =
            std::env::var("DATABASE_URL").expect("set DATABASE_URL to run this integration test");
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to connect to PostgreSQL")
    }

    fn forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Generic,
        )
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires a running PostgreSQL instance; run with `cargo test -- --ignored`"]
    async fn save_find_and_delete_a_real_repository() {
        let store = PostgresRepositoryStore::new(pool_from_env().await);
        let repository = forge("integration-test-repo");

        store.save(&repository).await.unwrap();

        let found = store.find_by_id(repository.id()).await.unwrap();
        assert_eq!(found, Some(repository.clone()));

        let found_by_name = store.find_by_name(repository.name()).await.unwrap();
        assert_eq!(found_by_name, Some(repository.clone()));

        store.delete(repository.id()).await.unwrap();
        assert_eq!(store.find_by_id(repository.id()).await.unwrap(), None);
    }

    #[tokio::test]
    #[ignore = "requires a running PostgreSQL instance; run with `cargo test -- --ignored`"]
    async fn find_all_returns_every_persisted_repository() {
        let store = PostgresRepositoryStore::new(pool_from_env().await);
        let first = forge("integration-test-find-all-first");
        let second = forge("integration-test-find-all-second");
        store.save(&first).await.unwrap();
        store.save(&second).await.unwrap();

        let all = store.find_all().await.unwrap();

        store.delete(first.id()).await.unwrap();
        store.delete(second.id()).await.unwrap();

        assert!(all.contains(&first));
        assert!(all.contains(&second));
    }

    #[tokio::test]
    #[ignore = "requires a running PostgreSQL instance; run with `cargo test -- --ignored`"]
    async fn saving_a_duplicate_name_is_rejected() {
        let store = PostgresRepositoryStore::new(pool_from_env().await);
        let first = forge("integration-test-duplicate");
        store.save(&first).await.unwrap();

        let second = forge("integration-test-duplicate");
        let result = store.save(&second).await;

        store.delete(first.id()).await.unwrap();

        assert!(matches!(
            result,
            Err(RepositoryStoreError::DuplicateName(_))
        ));
    }
}
