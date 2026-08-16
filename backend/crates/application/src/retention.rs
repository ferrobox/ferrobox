//! Retención de versiones y recolección de basura (GC) de un repositorio.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageVersion};
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_domain::retention::{RetentionPolicy, RetentionPolicyError};
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::assay_store::{AssayStore, AssayStoreError};
use ferrobox_ports::package_index_store::{
    PackageIndexRecord, PackageIndexStore, PackageIndexStoreError,
};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::retention_store::{RetentionStore, RetentionStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::storage_key::storage_key_for;

const BLOB_PACKAGE: &str = "_blob";

/// Resultado de aplicar retención y/o recolección de basura.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CleanupReport {
    /// Coordenadas de paquete (versiones o etiquetas) eliminadas del índice.
    pub dropped_versions: u64,
    /// Binarios borrados del almacén de artefactos.
    pub deleted_artifacts: u64,
    /// Bytes liberados en almacenamiento.
    pub freed_bytes: u64,
}

impl CleanupReport {
    fn merge(&mut self, other: Self) {
        self.dropped_versions += other.dropped_versions;
        self.deleted_artifacts += other.deleted_artifacts;
        self.freed_bytes += other.freed_bytes;
    }
}

/// Motivos por los que retención o GC pueden fallar.
#[derive(Debug, Error)]
pub enum RetentionError {
    /// El repositorio no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// Un `Alloy` no almacena binarios propios: no hay nada que retener.
    #[error("retention does not apply to Alloy repositories")]
    AlloyRepository,

    /// La política pedida no es válida.
    #[error(transparent)]
    InvalidPolicy(#[from] RetentionPolicyError),

    /// Fallo al consultar el almacén de repositorios.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),

    /// Fallo al consultar o actualizar el almacén de artefactos.
    #[error(transparent)]
    Artifacts(#[from] ArtifactStoreError),

    /// Fallo al consultar o actualizar el índice de paquetes.
    #[error(transparent)]
    Index(#[from] PackageIndexStoreError),

    /// Fallo al consultar o actualizar ensayes.
    #[error(transparent)]
    Assays(#[from] AssayStoreError),

    /// Fallo al consultar o actualizar la política persistida.
    #[error(transparent)]
    Policy(#[from] RetentionStoreError),

    /// Fallo al eliminar un objeto binario.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Caso de uso: política de retención y recolección de basura.
pub struct RetentionService {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    assay_store: Arc<dyn AssayStore>,
    retention_store: Arc<dyn RetentionStore>,
}

impl RetentionService {
    /// Construye el servicio a partir de sus puertos.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        assay_store: Arc<dyn AssayStore>,
        retention_store: Arc<dyn RetentionStore>,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            package_index_store,
            storage,
            assay_store,
            retention_store,
        }
    }

    /// Devuelve la política persistida, o «conservar todo» si no hay fila.
    ///
    /// # Errors
    ///
    /// [`RetentionError::RepositoryNotFound`] si el repositorio no existe.
    pub async fn get_policy(
        &self,
        repository_id: RepositoryId,
    ) -> Result<RetentionPolicy, RetentionError> {
        self.require_cleanup_target(repository_id).await?;
        Ok(self.retention_store.find_by_repository(repository_id).await?)
    }

    /// Guarda la política. No borra nada hasta [`Self::apply`].
    ///
    /// # Errors
    ///
    /// [`RetentionError::RepositoryNotFound`] o [`RetentionError::AlloyRepository`].
    pub async fn save_policy(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<RetentionPolicy, RetentionError> {
        self.require_cleanup_target(repository_id).await?;
        self.retention_store.save(repository_id, policy).await?;
        Ok(policy)
    }

    /// Aplica la política persistida y después recolecta binarios huérfanos.
    ///
    /// No bloquea `install` ni `publish`: hay que invocarlo a propósito.
    ///
    /// # Errors
    ///
    /// [`RetentionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn apply(&self, repository_id: RepositoryId) -> Result<CleanupReport, RetentionError> {
        self.require_cleanup_target(repository_id).await?;
        let policy = self.retention_store.find_by_repository(repository_id).await?;
        let mut report = self.drop_versions(repository_id, policy).await?;
        report.merge(self.collect_garbage(repository_id).await?);
        Ok(report)
    }

    /// Recolecta binarios huérfanos sin tocar versiones indexadas.
    ///
    /// # Errors
    ///
    /// [`RetentionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn collect_garbage_only(
        &self,
        repository_id: RepositoryId,
    ) -> Result<CleanupReport, RetentionError> {
        self.require_cleanup_target(repository_id).await?;
        self.collect_garbage(repository_id).await
    }

    async fn require_cleanup_target(
        &self,
        repository_id: RepositoryId,
    ) -> Result<(), RetentionError> {
        let Some(repository) = self.repository_store.find_by_id(repository_id).await? else {
            return Err(RetentionError::RepositoryNotFound(repository_id));
        };
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(RetentionError::AlloyRepository);
        }
        Ok(())
    }

    async fn drop_versions(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<CleanupReport, RetentionError> {
        if policy.is_keep_all() {
            return Ok(CleanupReport::default());
        }

        let entries = self.package_index_store.list_entries(repository_id).await?;
        let now = Utc::now();
        let mut by_package: BTreeMap<String, Vec<&PackageIndexRecord>> = BTreeMap::new();
        for entry in &entries {
            if is_blob_name(entry.coordinate.name().as_str())
                || is_digest_reference(entry.coordinate.version().as_str())
            {
                continue;
            }
            by_package
                .entry(entry.coordinate.name().as_str().to_string())
                .or_default()
                .push(entry);
        }

        let mut dropped_versions = 0;
        for versions in by_package.values_mut() {
            versions.sort_by(|left, right| {
                right
                    .created_at_rfc3339
                    .cmp(&left.created_at_rfc3339)
                    .then_with(|| {
                        right
                            .coordinate
                            .version()
                            .as_str()
                            .cmp(left.coordinate.version().as_str())
                    })
            });
            for (rank, record) in versions.iter().enumerate() {
                let rank = u32::try_from(rank).unwrap_or(u32::MAX);
                let age_days = age_days(&record.created_at_rfc3339, now);
                if policy.keeps(rank, age_days) {
                    continue;
                }
                self.drop_coordinate(repository_id, record, &entries)
                    .await?;
                dropped_versions += 1;
            }
        }
        dropped_versions += self.drop_untagged_digests(repository_id).await?;

        Ok(CleanupReport {
            dropped_versions,
            deleted_artifacts: 0,
            freed_bytes: 0,
        })
    }

    async fn drop_untagged_digests(
        &self,
        repository_id: RepositoryId,
    ) -> Result<u64, RetentionError> {
        let entries = self.package_index_store.list_entries(repository_id).await?;
        let mut kept_digests = HashSet::new();
        for entry in &entries {
            if is_blob_name(entry.coordinate.name().as_str())
                || is_digest_reference(entry.coordinate.version().as_str())
            {
                continue;
            }
            if let Some(digest) = manifest_digest(&entry.entry) {
                kept_digests.insert((
                    entry.coordinate.name().as_str().to_string(),
                    digest,
                ));
            }
        }

        let mut dropped = 0;
        for entry in &entries {
            if is_blob_name(entry.coordinate.name().as_str())
                || !is_digest_reference(entry.coordinate.version().as_str())
            {
                continue;
            }
            let key = (
                entry.coordinate.name().as_str().to_string(),
                entry.coordinate.version().as_str().to_string(),
            );
            if kept_digests.contains(&key) {
                continue;
            }
            self.assay_store
                .delete_by_coordinate(repository_id, &entry.coordinate)
                .await?;
            self.package_index_store
                .delete_by_coordinate(repository_id, &entry.coordinate)
                .await?;
            dropped += 1;
        }
        Ok(dropped)
    }

    async fn drop_coordinate(
        &self,
        repository_id: RepositoryId,
        record: &PackageIndexRecord,
        all_entries: &[PackageIndexRecord],
    ) -> Result<(), RetentionError> {
        self.assay_store
            .delete_by_coordinate(repository_id, &record.coordinate)
            .await?;
        self.package_index_store
            .delete_by_coordinate(repository_id, &record.coordinate)
            .await?;

        let Some(digest) = manifest_digest(&record.entry) else {
            return Ok(());
        };
        if digest_still_referenced(all_entries, record, &digest) {
            return Ok(());
        }
        let Ok(digest_version) = PackageVersion::parse(digest) else {
            return Ok(());
        };
        let digest_coordinate = PackageCoordinate::new(
            record.coordinate.ecosystem(),
            record.coordinate.name().clone(),
            digest_version,
        );
        if digest_coordinate == record.coordinate {
            return Ok(());
        }
        self.assay_store
            .delete_by_coordinate(repository_id, &digest_coordinate)
            .await?;
        self.package_index_store
            .delete_by_coordinate(repository_id, &digest_coordinate)
            .await?;
        Ok(())
    }

    async fn collect_garbage(
        &self,
        repository_id: RepositoryId,
    ) -> Result<CleanupReport, RetentionError> {
        let repository = self
            .repository_store
            .find_by_id(repository_id)
            .await?
            .ok_or(RetentionError::RepositoryNotFound(repository_id))?;

        if matches!(
            repository.ecosystem(),
            PackageEcosystem::Oci | PackageEcosystem::Helm
        ) {
            self.drop_unreferenced_blobs(repository_id).await?;
        }

        let entries = self.package_index_store.list_entries(repository_id).await?;
        if entries.is_empty() && repository.ecosystem() == PackageEcosystem::Generic {
            return Ok(CleanupReport::default());
        }

        let referenced = referenced_artifact_ids(&entries);
        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;

        let mut deleted_artifacts = 0;
        let mut freed_bytes = 0;
        for artifact in artifacts {
            if referenced.contains(&artifact.id()) {
                continue;
            }
            self.storage
                .delete(&storage_key_for(artifact.id()))
                .await?;
            self.artifact_store.delete(artifact.id()).await?;
            deleted_artifacts += 1;
            freed_bytes += artifact.size_bytes();
        }

        Ok(CleanupReport {
            dropped_versions: 0,
            deleted_artifacts,
            freed_bytes,
        })
    }

    async fn drop_unreferenced_blobs(
        &self,
        repository_id: RepositoryId,
    ) -> Result<(), RetentionError> {
        let entries = self.package_index_store.list_entries(repository_id).await?;
        let mut referenced_digests = HashSet::new();
        for entry in &entries {
            if is_blob_name(entry.coordinate.name().as_str()) {
                continue;
            }
            let Some(artifact_id) = entry.artifact_id else {
                continue;
            };
            let body = match self.storage.get(&storage_key_for(artifact_id)).await {
                Ok(body) => body,
                Err(StorageError::NotFound(_)) => {
                    return Ok(());
                }
                Err(err) => return Err(err.into()),
            };
            collect_manifest_digests(&body, &mut referenced_digests);
        }

        for entry in &entries {
            if !is_blob_name(entry.coordinate.name().as_str()) {
                continue;
            }
            let digest = entry.coordinate.version().as_str();
            if referenced_digests.contains(digest) {
                continue;
            }
            self.package_index_store
                .delete_by_coordinate(repository_id, &entry.coordinate)
                .await?;
        }
        Ok(())
    }
}

fn is_blob_name(name: &str) -> bool {
    name == BLOB_PACKAGE
}

fn is_digest_reference(version: &str) -> bool {
    version.starts_with("sha256:")
}

fn age_days(created_at_rfc3339: &str, now: DateTime<Utc>) -> u64 {
    DateTime::parse_from_rfc3339(created_at_rfc3339).map_or(0, |stamp| {
        let days = now
            .signed_duration_since(stamp.with_timezone(&Utc))
            .num_days()
            .max(0);
        u64::try_from(days).unwrap_or(0)
    })
}

fn manifest_digest(entry: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(entry).ok()?;
    value
        .get("digest")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn digest_still_referenced(
    entries: &[PackageIndexRecord],
    dropping: &PackageIndexRecord,
    digest: &str,
) -> bool {
    entries.iter().any(|entry| {
        if entry.coordinate == dropping.coordinate {
            return false;
        }
        if entry.coordinate.name() != dropping.coordinate.name() {
            return false;
        }
        if is_digest_reference(entry.coordinate.version().as_str()) {
            return false;
        }
        manifest_digest(&entry.entry).as_deref() == Some(digest)
    })
}

fn referenced_artifact_ids(entries: &[PackageIndexRecord]) -> HashSet<ArtifactId> {
    let mut ids = HashSet::new();
    for entry in entries {
        if let Some(artifact_id) = entry.artifact_id {
            ids.insert(artifact_id);
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&entry.entry) {
            collect_artifact_ids(&value, &mut ids);
        }
    }
    ids
}

fn collect_artifact_ids(value: &Value, ids: &mut HashSet<ArtifactId>) {
    match value {
        Value::Object(map) => {
            if let Some(raw) = map.get("artifact_id").and_then(Value::as_str)
                && let Ok(uuid) = Uuid::parse_str(raw)
            {
                ids.insert(ArtifactId::from(uuid));
            }
            for nested in map.values() {
                collect_artifact_ids(nested, ids);
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_artifact_ids(nested, ids);
            }
        }
        _ => {}
    }
}

fn collect_manifest_digests(body: &[u8], digests: &mut HashSet<String>) {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return;
    };
    if let Some(digest) = value
        .get("config")
        .and_then(|config| config.get("digest"))
        .and_then(Value::as_str)
    {
        digests.insert(digest.to_string());
    }
    for key in ["layers", "manifests"] {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(digest) = item.get("digest").and_then(Value::as_str) {
                    digests.insert(digest.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::{PackageEcosystem, PackageName, PackageVersion};
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;
    use ferrobox_ports::storage::StoragePort;

    use crate::storage_key::storage_key_for;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryAssayStore, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryRetentionStore, InMemoryStorage,
    };

    use super::*;

    fn checksum() -> Sha256Checksum {
        Sha256Checksum::parse("a".repeat(64)).unwrap()
    }

    fn cargo(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap()
    }

    fn coordinate(name: &str, version: &str) -> PackageCoordinate {
        PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse(name).unwrap(),
            PackageVersion::parse(version).unwrap(),
        )
    }

    struct Fixture {
        repositories: Arc<InMemoryRepositoryStore>,
        artifacts: Arc<InMemoryArtifactStore>,
        index: Arc<InMemoryPackageIndexStore>,
        storage: Arc<InMemoryStorage>,
        service: RetentionService,
    }

    fn fixture() -> Fixture {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let service = RetentionService::new(
            repositories.clone(),
            artifacts.clone(),
            index.clone(),
            storage.clone(),
            Arc::new(InMemoryAssayStore::default()),
            Arc::new(InMemoryRetentionStore::default()),
        );
        Fixture {
            repositories,
            artifacts,
            index,
            storage,
            service,
        }
    }

    async fn publish(
        fixture: &Fixture,
        repository_id: RepositoryId,
        name: &str,
        version: &str,
        created_at: &str,
    ) -> ArtifactId {
        let artifact =
            ferrobox_domain::artifact::Artifact::new(repository_id, checksum(), 10);
        fixture.artifacts.save(&artifact).await.unwrap();
        fixture
            .storage
            .put(
                &storage_key_for(artifact.id()),
                Bytes::from_static(b"0123456789"),
            )
            .await
            .unwrap();
        fixture
            .index
            .upsert_entry_at(
                repository_id,
                &coordinate(name, version),
                Some(artifact.id()),
                Bytes::from_static(b"{}"),
                created_at,
            );
        artifact.id()
    }

    #[tokio::test]
    async fn keep_last_drops_old_versions_and_their_binaries() {
        let fixture = fixture();
        let repository = cargo("crates");
        fixture.repositories.save(&repository).await.unwrap();

        let old = publish(
            &fixture,
            repository.id(),
            "demo",
            "1.0.0",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let mid = publish(
            &fixture,
            repository.id(),
            "demo",
            "1.1.0",
            "2025-01-01T00:00:00Z",
        )
        .await;
        let new = publish(
            &fixture,
            repository.id(),
            "demo",
            "2.0.0",
            "2026-01-01T00:00:00Z",
        )
        .await;

        fixture
            .service
            .save_policy(
                repository.id(),
                RetentionPolicy::new(Some(2), None).unwrap(),
            )
            .await
            .unwrap();
        let report = fixture.service.apply(repository.id()).await.unwrap();

        assert_eq!(report.dropped_versions, 1);
        assert_eq!(report.deleted_artifacts, 1);
        assert_eq!(report.freed_bytes, 10);
        assert!(fixture.artifacts.find_by_id(old).await.unwrap().is_none());
        assert!(fixture.artifacts.find_by_id(mid).await.unwrap().is_some());
        assert!(fixture.artifacts.find_by_id(new).await.unwrap().is_some());
        assert_eq!(
            fixture
                .index
                .artifact_for(repository.id(), &coordinate("demo", "1.0.0"))
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn keep_days_drops_stale_versions() {
        let fixture = fixture();
        let repository = cargo("crates");
        fixture.repositories.save(&repository).await.unwrap();
        let stale = publish(
            &fixture,
            repository.id(),
            "demo",
            "1.0.0",
            "2020-01-01T00:00:00Z",
        )
        .await;
        let fresh = publish(
            &fixture,
            repository.id(),
            "demo",
            "2.0.0",
            &Utc::now().to_rfc3339(),
        )
        .await;

        fixture
            .service
            .save_policy(
                repository.id(),
                RetentionPolicy::new(None, Some(30)).unwrap(),
            )
            .await
            .unwrap();
        fixture.service.apply(repository.id()).await.unwrap();

        assert!(fixture.artifacts.find_by_id(stale).await.unwrap().is_none());
        assert!(fixture.artifacts.find_by_id(fresh).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn gc_removes_orphans_but_keeps_generic_unindexed_artifacts() {
        let fixture = fixture();
        let generic = Repository::new(
            RepositoryName::parse("binaries").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Generic,
        )
        .unwrap();
        fixture.repositories.save(&generic).await.unwrap();
        let artifact = ferrobox_domain::artifact::Artifact::new(generic.id(), checksum(), 4);
        fixture.artifacts.save(&artifact).await.unwrap();
        fixture
            .storage
            .put(&storage_key_for(artifact.id()), Bytes::from_static(b"data"))
            .await
            .unwrap();

        let report = fixture
            .service
            .collect_garbage_only(generic.id())
            .await
            .unwrap();
        assert_eq!(report.deleted_artifacts, 0);
        assert!(
            fixture
                .artifacts
                .find_by_id(artifact.id())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn gc_drops_unreferenced_oci_blobs() {
        let fixture = fixture();
        let repository = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        fixture.repositories.save(&repository).await.unwrap();

        let layer = ferrobox_domain::artifact::Artifact::new(repository.id(), checksum(), 3);
        let orphan = ferrobox_domain::artifact::Artifact::new(repository.id(), checksum(), 5);
        let manifest = ferrobox_domain::artifact::Artifact::new(repository.id(), checksum(), 8);
        fixture.artifacts.save(&layer).await.unwrap();
        fixture.artifacts.save(&orphan).await.unwrap();
        fixture.artifacts.save(&manifest).await.unwrap();
        fixture
            .storage
            .put(&storage_key_for(layer.id()), Bytes::from_static(b"lay"))
            .await
            .unwrap();
        fixture
            .storage
            .put(&storage_key_for(orphan.id()), Bytes::from_static(b"orphn"))
            .await
            .unwrap();
        let manifest_body = serde_json::json!({
            "config": { "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
            "layers": [{ "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" }]
        });
        fixture
            .storage
            .put(
                &storage_key_for(manifest.id()),
                Bytes::from(manifest_body.to_string()),
            )
            .await
            .unwrap();

        fixture
            .index
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Oci,
                    PackageName::parse("alpine").unwrap(),
                    PackageVersion::parse("latest").unwrap(),
                ),
                Some(manifest.id()),
                Bytes::from(
                    serde_json::json!({
                        "name": "alpine",
                        "reference": "latest",
                        "digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                        "artifact_id": manifest.id().to_string()
                    })
                    .to_string(),
                ),
            )
            .await
            .unwrap();
        fixture
            .index
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Oci,
                    PackageName::parse("_blob").unwrap(),
                    PackageVersion::parse(
                        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    )
                    .unwrap(),
                ),
                Some(layer.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();
        fixture
            .index
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Oci,
                    PackageName::parse("_blob").unwrap(),
                    PackageVersion::parse(
                        "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    )
                    .unwrap(),
                ),
                Some(orphan.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let report = fixture
            .service
            .collect_garbage_only(repository.id())
            .await
            .unwrap();
        assert_eq!(report.deleted_artifacts, 1);
        assert!(fixture.artifacts.find_by_id(orphan.id()).await.unwrap().is_none());
        assert!(fixture.artifacts.find_by_id(layer.id()).await.unwrap().is_some());
        assert!(
            fixture
                .artifacts
                .find_by_id(manifest.id())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn alloy_is_rejected() {
        let fixture = fixture();
        let member = cargo("member");
        let alloy = Repository::new(
            RepositoryName::parse("all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![member.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        fixture.repositories.save(&member).await.unwrap();
        fixture.repositories.save(&alloy).await.unwrap();

        let err = fixture.service.apply(alloy.id()).await.unwrap_err();
        assert!(matches!(err, RetentionError::AlloyRepository));
    }

    #[tokio::test]
    async fn keep_all_policy_is_a_noop_until_gc() {
        let fixture = fixture();
        let repository = cargo("crates");
        fixture.repositories.save(&repository).await.unwrap();
        let id = publish(
            &fixture,
            repository.id(),
            "demo",
            "1.0.0",
            "2020-01-01T00:00:00Z",
        )
        .await;

        let policy = fixture.service.get_policy(repository.id()).await.unwrap();
        assert!(policy.is_keep_all());
        let report = fixture.service.apply(repository.id()).await.unwrap();
        assert_eq!(report.dropped_versions, 0);
        assert!(fixture.artifacts.find_by_id(id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn pypi_files_in_json_are_not_orphans() {
        let fixture = fixture();
        let repository = Repository::new(
            RepositoryName::parse("pypi").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::PyPi,
        )
        .unwrap();
        fixture.repositories.save(&repository).await.unwrap();
        let sdist = ferrobox_domain::artifact::Artifact::new(repository.id(), checksum(), 2);
        let wheel = ferrobox_domain::artifact::Artifact::new(repository.id(), checksum(), 3);
        fixture.artifacts.save(&sdist).await.unwrap();
        fixture.artifacts.save(&wheel).await.unwrap();
        fixture
            .storage
            .put(&storage_key_for(sdist.id()), Bytes::from_static(b"sd"))
            .await
            .unwrap();
        fixture
            .storage
            .put(&storage_key_for(wheel.id()), Bytes::from_static(b"wh"))
            .await
            .unwrap();
        let entry = serde_json::json!({
            "files": [
                { "filename": "a.tar.gz", "artifact_id": sdist.id().to_string() },
                { "filename": "a.whl", "artifact_id": wheel.id().to_string() }
            ]
        });
        fixture
            .index
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::PyPi,
                    PackageName::parse("demo").unwrap(),
                    PackageVersion::parse("1.0.0").unwrap(),
                ),
                Some(wheel.id()),
                Bytes::from(entry.to_string()),
            )
            .await
            .unwrap();

        let report = fixture
            .service
            .collect_garbage_only(repository.id())
            .await
            .unwrap();
        assert_eq!(report.deleted_artifacts, 0);
    }
}
