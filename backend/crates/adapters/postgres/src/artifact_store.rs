use async_trait::async_trait;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::checksum::Sha256Checksum;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Adaptador de [`ArtifactStore`] contra `PostgreSQL`.
pub struct PostgresArtifactStore {
    pool: PgPool,
}

impl PostgresArtifactStore {
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

fn backend_error(message: impl Into<String>) -> ArtifactStoreError {
    ArtifactStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn row_to_artifact(
    id: Uuid,
    repository_id: Uuid,
    checksum: String,
    size_bytes: i64,
) -> Result<Artifact, ArtifactStoreError> {
    let checksum = Sha256Checksum::parse(checksum).map_err(|err| backend_error(err.to_string()))?;
    let size_bytes = u64::try_from(size_bytes)
        .map_err(|err| backend_error(format!("negative size_bytes in row: {err}")))?;

    Ok(Artifact::from_parts(
        ArtifactId::from(id),
        RepositoryId::from(repository_id),
        checksum,
        size_bytes,
    ))
}

#[async_trait]
impl ArtifactStore for PostgresArtifactStore {
    async fn save(&self, artifact: &Artifact) -> Result<(), ArtifactStoreError> {
        let id: Uuid = artifact.id().into();
        let repository_id: Uuid = artifact.repository_id().into();
        let size_bytes = i64::try_from(artifact.size_bytes())
            .map_err(|err| backend_error(format!("size_bytes too large: {err}")))?;

        sqlx::query!(
            r#"
            INSERT INTO artifacts (id, repository_id, checksum, size_bytes)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO UPDATE
            SET repository_id = EXCLUDED.repository_id,
                checksum = EXCLUDED.checksum,
                size_bytes = EXCLUDED.size_bytes
            "#,
            id,
            repository_id,
            artifact.checksum().as_str(),
            size_bytes,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }

    async fn find_by_id(&self, id: ArtifactId) -> Result<Option<Artifact>, ArtifactStoreError> {
        let id: Uuid = id.into();

        let row = sqlx::query!(
            r#"SELECT id, repository_id, checksum, size_bytes FROM artifacts WHERE id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        row.map(|r| row_to_artifact(r.id, r.repository_id, r.checksum, r.size_bytes))
            .transpose()
    }

    async fn find_by_repository_id(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Artifact>, ArtifactStoreError> {
        let repository_id: Uuid = repository_id.into();

        let rows = sqlx::query!(
            r#"SELECT id, repository_id, checksum, size_bytes FROM artifacts WHERE repository_id = $1 ORDER BY created_at"#,
            repository_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|r| row_to_artifact(r.id, r.repository_id, r.checksum, r.size_bytes))
            .collect()
    }

    async fn delete(&self, id: ArtifactId) -> Result<(), ArtifactStoreError> {
        let id: Uuid = id.into();

        sqlx::query!(r#"DELETE FROM artifacts WHERE id = $1"#, id)
            .execute(&self.pool)
            .await
            .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;
    use sqlx::postgres::PgPoolOptions;

    use crate::repository_store::PostgresRepositoryStore;

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

    #[tokio::test]
    #[ignore = "requires a running PostgreSQL instance; run with `cargo test -- --ignored`"]
    async fn save_find_and_delete_a_real_artifact() {
        let pool = pool_from_env().await;
        let repository_store = PostgresRepositoryStore::new(pool.clone());
        let artifact_store = PostgresArtifactStore::new(pool);

        let repository = Repository::new(
            RepositoryName::parse("integration-test-artifacts-repo").unwrap(),
            RepositoryKind::Forge,
        )
        .unwrap();
        repository_store.save(&repository).await.unwrap();

        let artifact = Artifact::new(
            repository.id(),
            Sha256Checksum::parse("b".repeat(64)).unwrap(),
            2048,
        );

        artifact_store.save(&artifact).await.unwrap();
        let found = artifact_store.find_by_id(artifact.id()).await.unwrap();
        assert_eq!(found, Some(artifact.clone()));

        artifact_store.delete(artifact.id()).await.unwrap();
        assert_eq!(
            artifact_store.find_by_id(artifact.id()).await.unwrap(),
            None
        );

        repository_store.delete(repository.id()).await.unwrap();
    }
}
