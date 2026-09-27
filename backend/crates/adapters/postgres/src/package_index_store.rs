use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_ports::package_index_store::{
    IndexedArtifact, PackageIndexRecord, PackageIndexStore, PackageIndexStoreError,
};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::ecosystem_column;

/// [`PackageIndexStore`] adapter against `PostgreSQL`.
pub struct PostgresPackageIndexStore {
    pool: PgPool,
}

impl PostgresPackageIndexStore {
    /// Builds the adapter from an already configured connection `pool`.
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

    async fn delete_by_artifact(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<(), PackageIndexStoreError> {
        let artifact_id: Uuid = artifact_id.into();

        sqlx::query!(
            r#"
            DELETE FROM package_index_entries
            WHERE artifact_id = $1
            "#,
            artifact_id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }

    async fn delete_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<(), PackageIndexStoreError> {
        let repository_id: Uuid = repository_id.into();

        sqlx::query!(
            r#"
            DELETE FROM package_index_entries
            WHERE repository_id = $1
            "#,
            repository_id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }

    async fn entries_for_repository(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
    ) -> Result<Vec<Bytes>, PackageIndexStoreError> {
        let repository_id: Uuid = repository_id.into();
        let ecosystem = ecosystem_column::to_column(ecosystem);

        let rows = sqlx::query!(
            r#"
            SELECT entry
            FROM package_index_entries
            WHERE repository_id = $1
              AND ecosystem = $2
            ORDER BY created_at
            "#,
            repository_id,
            ecosystem,
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

    async fn find_indexed_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<IndexedArtifact>, PackageIndexStoreError> {
        let repository_id: Uuid = repository_id.into();

        let rows = sqlx::query(
            r"
            SELECT artifact_id, ecosystem, package_name, package_version, entry
            FROM package_index_entries
            WHERE repository_id = $1
              AND artifact_id IS NOT NULL
            ORDER BY package_name, package_version
            ",
        )
        .bind(repository_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| {
                let artifact_id: Option<Uuid> = row
                    .try_get("artifact_id")
                    .map_err(|err| backend_error(err.to_string()))?;
                let artifact_id =
                    artifact_id.ok_or_else(|| backend_error("indexed row missing artifact_id"))?;
                let ecosystem: String = row
                    .try_get("ecosystem")
                    .map_err(|err| backend_error(err.to_string()))?;
                let package_name: String = row
                    .try_get("package_name")
                    .map_err(|err| backend_error(err.to_string()))?;
                let package_version: String = row
                    .try_get("package_version")
                    .map_err(|err| backend_error(err.to_string()))?;

                let ecosystem =
                    ecosystem_column::from_column(&ecosystem).map_err(backend_error)?;
                let name =
                    PackageName::parse(package_name).map_err(|err| backend_error(err.to_string()))?;
                let version = PackageVersion::parse(package_version)
                    .map_err(|err| backend_error(err.to_string()))?;
                let entry: serde_json::Value = row
                    .try_get("entry")
                    .map_err(|err| backend_error(err.to_string()))?;
                let entry = serde_json::to_vec(&entry)
                    .map(Bytes::from)
                    .map_err(|err| backend_error(err.to_string()))?;

                Ok(IndexedArtifact {
                    artifact_id: ArtifactId::from(artifact_id),
                    coordinate: PackageCoordinate::new(ecosystem, name, version),
                    entry,
                })
            })
            .collect()
    }

    async fn list_entries(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<PackageIndexRecord>, PackageIndexStoreError> {
        let repository_id: Uuid = repository_id.into();
        let rows = sqlx::query(
            r"
            SELECT artifact_id, ecosystem, package_name, package_version, entry, created_at
            FROM package_index_entries
            WHERE repository_id = $1
            ORDER BY created_at
            ",
        )
        .bind(repository_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        rows.into_iter()
            .map(|row| {
                let artifact_id: Option<Uuid> = row
                    .try_get("artifact_id")
                    .map_err(|err| backend_error(err.to_string()))?;
                let ecosystem: String = row
                    .try_get("ecosystem")
                    .map_err(|err| backend_error(err.to_string()))?;
                let package_name: String = row
                    .try_get("package_name")
                    .map_err(|err| backend_error(err.to_string()))?;
                let package_version: String = row
                    .try_get("package_version")
                    .map_err(|err| backend_error(err.to_string()))?;
                let entry: serde_json::Value = row
                    .try_get("entry")
                    .map_err(|err| backend_error(err.to_string()))?;
                let created_at: chrono::DateTime<chrono::Utc> = row
                    .try_get("created_at")
                    .map_err(|err| backend_error(err.to_string()))?;

                let ecosystem =
                    ecosystem_column::from_column(&ecosystem).map_err(backend_error)?;
                let name =
                    PackageName::parse(package_name).map_err(|err| backend_error(err.to_string()))?;
                let version = PackageVersion::parse(package_version)
                    .map_err(|err| backend_error(err.to_string()))?;
                let entry = serde_json::to_vec(&entry)
                    .map(Bytes::from)
                    .map_err(|err| backend_error(err.to_string()))?;

                Ok(PackageIndexRecord {
                    artifact_id: artifact_id.map(ArtifactId::from),
                    coordinate: PackageCoordinate::new(ecosystem, name, version),
                    entry,
                    created_at_rfc3339: created_at.to_rfc3339(),
                })
            })
            .collect()
    }

    async fn delete_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<(), PackageIndexStoreError> {
        let repository_id: Uuid = repository_id.into();
        let ecosystem = ecosystem_column::to_column(coordinate.ecosystem());
        sqlx::query(
            r"
            DELETE FROM package_index_entries
            WHERE repository_id = $1
              AND ecosystem = $2
              AND lower(package_name) = lower($3)
              AND package_version = $4
            ",
        )
        .bind(repository_id)
        .bind(ecosystem)
        .bind(coordinate.name().as_str())
        .bind(coordinate.version().as_str())
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        Ok(())
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

        // Same as `artifacts`, `package_index_entries` references
        // `repositories` and `artifacts` without `ON DELETE CASCADE`
        // on purpose -- dependent rows must be undone by hand before
        // the test repository and artifact can be deleted.
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
