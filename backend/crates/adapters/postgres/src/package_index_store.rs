use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::ecosystem_column;

/// Adaptador de [`PackageIndexStore`] contra `PostgreSQL`.
pub struct PostgresPackageIndexStore {
    pool: PgPool,
}

impl PostgresPackageIndexStore {
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

fn backend_error(message: impl Into<String>) -> PackageIndexStoreError {
    PackageIndexStoreError::Backend(Box::new(RowConversionError(message.into())))
}

#[async_trait]
impl PackageIndexStore for PostgresPackageIndexStore {
    async fn upsert_entry(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
        artifact_id: Option<ArtifactId>,
        entry: Bytes,
    ) -> Result<(), PackageIndexStoreError> {
        let repository_id: Uuid = repository_id.into();
        let artifact_id: Option<Uuid> = artifact_id.map(Into::into);
        let ecosystem = ecosystem_column::to_column(coordinate.ecosystem());
        let entry: serde_json::Value =
            serde_json::from_slice(&entry).map_err(|err| backend_error(err.to_string()))?;

        sqlx::query!(
            r#"
            INSERT INTO package_index_entries
                (repository_id, ecosystem, package_name, package_version, artifact_id, entry)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (repository_id, ecosystem, package_name, package_version) DO UPDATE
            SET artifact_id = COALESCE(package_index_entries.artifact_id, EXCLUDED.artifact_id),
                entry = EXCLUDED.entry
            "#,
            repository_id,
            ecosystem,
            coordinate.name().as_str(),
            coordinate.version().as_str(),
            artifact_id,
            entry,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }

    async fn entries_for_package(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &PackageName,
    ) -> Result<Vec<Bytes>, PackageIndexStoreError> {
        let repository_id: Uuid = repository_id.into();
        let ecosystem = ecosystem_column::to_column(ecosystem);

        let rows = sqlx::query!(
            r#"
            SELECT entry
            FROM package_index_entries
            WHERE repository_id = $1
              AND ecosystem = $2
              AND lower(package_name) = lower($3)
            ORDER BY created_at
            "#,
            repository_id,
            ecosystem,
            name.as_str(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| {
                serde_json::to_vec(&row.entry)
                    .map(Bytes::from)
                    .map_err(|err| backend_error(err.to_string()))
            })
            .collect()
    }

    async fn artifact_for(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<ArtifactId>, PackageIndexStoreError> {
        let repository_id_uuid: Uuid = repository_id.into();
        let ecosystem = ecosystem_column::to_column(coordinate.ecosystem());

        let row = sqlx::query!(
            r#"
            SELECT artifact_id
            FROM package_index_entries
            WHERE repository_id = $1
              AND ecosystem = $2
              AND lower(package_name) = lower($3)
              AND package_version = $4
            "#,
            repository_id_uuid,
            ecosystem,
            coordinate.name().as_str(),
            coordinate.version().as_str(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(row.and_then(|r| r.artifact_id.map(ArtifactId::from)))
    }
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::package_coordinate::PackageVersion;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::repository_store::RepositoryStore;
    use sqlx::postgres::PgPoolOptions;

    use crate::artifact_store::PostgresArtifactStore;
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

    fn coordinate(name: &str, version: &str) -> PackageCoordinate {
        PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse(name).unwrap(),
            PackageVersion::parse(version).unwrap(),
        )
    }

    #[tokio::test]
    #[ignore = "requires a running PostgreSQL instance; run with `cargo test -- --ignored`"]
    async fn upsert_then_lookup_a_real_package_index_entry() {
        let pool = pool_from_env().await;
        let repository_store = PostgresRepositoryStore::new(pool.clone());
        let artifact_store = PostgresArtifactStore::new(pool.clone());
        let index_store = PostgresPackageIndexStore::new(pool.clone());

        let repository = Repository::new(
            RepositoryName::parse("integration-test-cargo-repo").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repository_store.save(&repository).await.unwrap();

        let artifact = ferrobox_domain::artifact::Artifact::new(
            repository.id(),
            ferrobox_domain::checksum::Sha256Checksum::parse("c".repeat(64)).unwrap(),
            42,
        );
        artifact_store.save(&artifact).await.unwrap();

        let coordinate = coordinate("integration-test-crate", "1.0.0");
        index_store
            .upsert_entry(
                repository.id(),
                &coordinate,
                Some(artifact.id()),
                Bytes::from_static(br#"{"name":"integration-test-crate","vers":"1.0.0"}"#),
            )
            .await
            .unwrap();

        let found_artifact = index_store
            .artifact_for(repository.id(), &coordinate)
            .await
            .unwrap();
        assert_eq!(found_artifact, Some(artifact.id()));

        let entries = index_store
            .entries_for_package(
                repository.id(),
                PackageEcosystem::Cargo,
                &PackageName::parse("integration-test-crate").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);

        // Igual que `artifacts`, `package_index_entries` referencia
        // `repositories` y `artifacts` sin `ON DELETE CASCADE`
        // deliberadamente -- hay que deshacer las filas dependientes a
        // mano antes de poder borrar el repositorio y el artefacto de
        // prueba.
        sqlx::query!(
            "DELETE FROM package_index_entries WHERE repository_id = $1",
            Uuid::from(repository.id()),
        )
        .execute(&pool)
        .await
        .unwrap();
        artifact_store.delete(artifact.id()).await.unwrap();
        repository_store.delete(repository.id()).await.unwrap();
    }
}
