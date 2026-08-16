//! Retención de versiones (índice) y recolección de basura (binarios) de un repositorio.

use std::collections::{BTreeMap, HashMap, HashSet};
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
    fn absorb(&mut self, other: Self) {
        self.dropped_versions += other.dropped_versions;
        self.deleted_artifacts += other.deleted_artifacts;
        self.freed_bytes += other.freed_bytes;
    }
}

/// Qué se borraría (o se ha borrado) al aplicar retención o GC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupItem {
    /// Repositorio al que pertenece la fila.
    pub repository: String,
    /// Nombre del paquete, o `_blob` para una capa OCI.
    pub name: String,
    /// Versión, etiqueta o digest.
    pub version: String,
    /// Tamaño del binario, si se conoce.
    pub size_bytes: u64,
    /// Por qué entra en la limpieza.
    pub reason: String,
}

/// Resultado de simular o aplicar una limpieza.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupPreview {
    /// `true` si no se ha borrado nada.
    pub dry_run: bool,
    /// Resumen numérico.
    pub report: CleanupReport,
    /// Versiones y binarios afectados, para enseñarlos en la UI.
    pub items: Vec<CleanupItem>,
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

    /// Falta la migración SQL (`sqlx migrate run` en el directorio backend).
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_retention is missing)"
    )]
    MissingSchema,

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
        match self.retention_store.find_by_repository(repository_id).await {
            Ok(policy) => Ok(policy),
            Err(RetentionStoreError::MissingSchema) => Ok(RetentionPolicy::keep_all()),
            Err(err) => Err(err.into()),
        }
    }

    /// Guarda la política. No desindexa nada hasta [`Self::apply`].
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
        self.save_store(repository_id, policy).await?;
        Ok(policy)
    }

    /// Simula la política **sin borrar nada**.
    ///
    /// # Errors
    ///
    /// [`RetentionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn dry_run(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<CleanupPreview, RetentionError> {
        self.require_cleanup_target(repository_id).await?;
        let mut preview = self.preview(repository_id, policy).await?;
        preview.dry_run = true;
        Ok(preview)
    }

    /// Guarda la política y la aplica: quita versiones del índice. El disco
    /// se libera después, con la recolección de basura.
    ///
    /// # Errors
    ///
    /// [`RetentionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn apply_policy(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<CleanupPreview, RetentionError> {
        self.require_cleanup_target(repository_id).await?;
        self.save_store(repository_id, policy).await?;
        let mut preview = self.preview(repository_id, policy).await?;
        self.execute(repository_id, &preview).await?;
        preview.dry_run = false;
        Ok(preview)
    }

    /// Aplica la política persistida (solo índice).
    ///
    /// No bloquea `install` ni `publish`: hay que invocarlo a propósito.
    ///
    /// # Errors
    ///
    /// [`RetentionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn apply(&self, repository_id: RepositoryId) -> Result<CleanupReport, RetentionError> {
        self.require_cleanup_target(repository_id).await?;
        let policy = self.retention_store.find_by_repository(repository_id).await?;
        Ok(self.apply_policy(repository_id, policy).await?.report)
    }

    /// Recolecta binarios huérfanos de un repositorio sin tocar versiones indexadas.
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

    /// Simula la recolección de basura de toda la instancia, sin borrar.
    ///
    /// # Errors
    ///
    /// Un fallo de puerto.
    pub async fn dry_run_garbage_collection(&self) -> Result<CleanupPreview, RetentionError> {
        let mut preview = self.preview_all_garbage().await?;
        preview.dry_run = true;
        Ok(preview)
    }

    /// Borra de disco los binarios que ya no están en el índice, en toda la instancia.
    ///
    /// # Errors
    ///
    /// Un fallo de puerto.
    pub async fn collect_garbage_all(&self) -> Result<CleanupPreview, RetentionError> {
        let mut preview = self.preview_all_garbage().await?;
        for repository in self.repository_store.find_all().await? {
            if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
                continue;
            }
            self.collect_garbage(repository.id()).await?;
        }
        preview.dry_run = false;
        Ok(preview)
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

    async fn save_store(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<(), RetentionError> {
        match self.retention_store.save(repository_id, policy).await {
            Ok(()) => Ok(()),
            Err(RetentionStoreError::MissingSchema) => Err(RetentionError::MissingSchema),
            Err(err) => Err(RetentionError::Policy(err)),
        }
    }

    async fn preview(
        &self,
        repository_id: RepositoryId,
        policy: RetentionPolicy,
    ) -> Result<CleanupPreview, RetentionError> {
        let repository = self
            .repository_store
            .find_by_id(repository_id)
            .await?
            .ok_or(RetentionError::RepositoryNotFound(repository_id))?;
        let repo_name = repository.name().as_str();
        let entries = self.package_index_store.list_entries(repository_id).await?;
        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;
        let sizes: HashMap<ArtifactId, u64> = artifacts
            .iter()
            .map(|artifact| (artifact.id(), artifact.size_bytes()))
            .collect();

        let mut items = Vec::new();
        let mut remaining: Vec<PackageIndexRecord> = entries.clone();
        let now = Utc::now();

        if !policy.is_keep_all() {
            let dropping = versions_to_drop(&entries, policy, now);
            let mut drop_keys: HashSet<(String, String)> = dropping
                .iter()
                .map(|(record, _)| coordinate_key(&record.coordinate))
                .collect();
            for (record, reason) in dropping {
                items.push(item_from_record(repo_name, &record, &sizes, reason));
            }
            remaining.retain(|entry| !drop_keys.contains(&coordinate_key(&entry.coordinate)));

            for record in digest_aliases_to_drop(&remaining) {
                drop_keys.insert(coordinate_key(&record.coordinate));
                items.push(item_from_record(
                    repo_name,
                    &record,
                    &sizes,
                    "digest sin etiqueta que lo apunte".to_string(),
                ));
            }
        }

        let dropped_versions = u64::try_from(items.len()).unwrap_or(0);
        Ok(CleanupPreview {
            dry_run: true,
            report: CleanupReport {
                dropped_versions,
                deleted_artifacts: 0,
                freed_bytes: 0,
            },
            items,
        })
    }

    async fn preview_all_garbage(&self) -> Result<CleanupPreview, RetentionError> {
        let mut items = Vec::new();
        let mut report = CleanupReport::default();
        for repository in self.repository_store.find_all().await? {
            if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
                continue;
            }
            let preview = self.preview_garbage(repository.id()).await?;
            items.extend(preview.items);
            report.absorb(preview.report);
        }
        items.sort_by(|left, right| {
            left.repository
                .cmp(&right.repository)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.version.cmp(&right.version))
        });
        Ok(CleanupPreview {
            dry_run: true,
            report,
            items,
        })
    }

    async fn preview_garbage(
        &self,
        repository_id: RepositoryId,
    ) -> Result<CleanupPreview, RetentionError> {
        let repository = self
            .repository_store
            .find_by_id(repository_id)
            .await?
            .ok_or(RetentionError::RepositoryNotFound(repository_id))?;
        let repo_name = repository.name().as_str();
        let mut entries = self.package_index_store.list_entries(repository_id).await?;
        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;
        let sizes: HashMap<ArtifactId, u64> = artifacts
            .iter()
            .map(|artifact| (artifact.id(), artifact.size_bytes()))
            .collect();

        let mut items = Vec::new();
        let mut listed_ids = HashSet::new();
        if matches!(
            repository.ecosystem(),
            PackageEcosystem::Oci | PackageEcosystem::Helm
        ) {
            let blobs = self.unreferenced_blobs(&entries).await?;
            let blob_keys: HashSet<(String, String)> = blobs
                .iter()
                .map(|record| coordinate_key(&record.coordinate))
                .collect();
            for record in blobs {
                if let Some(artifact_id) = record.artifact_id {
                    listed_ids.insert(artifact_id);
                }
                items.push(item_from_record(
                    repo_name,
                    &record,
                    &sizes,
                    "capa u objeto OCI sin manifiesto".to_string(),
                ));
            }
            entries.retain(|entry| !blob_keys.contains(&coordinate_key(&entry.coordinate)));
        }

        let skip_generic =
            entries.is_empty() && repository.ecosystem() == PackageEcosystem::Generic;
        let referenced = referenced_artifact_ids(&entries);
        let doomed: Vec<_> = artifacts
            .iter()
            .filter(|artifact| !skip_generic && !referenced.contains(&artifact.id()))
            .collect();
        for artifact in &doomed {
            if listed_ids.contains(&artifact.id()) {
                continue;
            }
            items.push(CleanupItem {
                repository: repo_name.to_string(),
                name: String::new(),
                version: artifact.id().to_string(),
                size_bytes: artifact.size_bytes(),
                reason: "binario huérfano".to_string(),
            });
        }

        Ok(CleanupPreview {
            dry_run: true,
            report: CleanupReport {
                dropped_versions: u64::try_from(items.len().saturating_sub(doomed.len()))
                    .unwrap_or(0),
                deleted_artifacts: u64::try_from(doomed.len()).unwrap_or(0),
                freed_bytes: doomed.iter().map(|artifact| artifact.size_bytes()).sum(),
            },
            items,
        })
    }

    async fn execute(
        &self,
        repository_id: RepositoryId,
        preview: &CleanupPreview,
    ) -> Result<(), RetentionError> {
        let Some(repository) = self.repository_store.find_by_id(repository_id).await? else {
            return Err(RetentionError::RepositoryNotFound(repository_id));
        };
        let ecosystem = repository.ecosystem();
        for item in &preview.items {
            if item.reason == "binario huérfano" || item.name.is_empty() {
                continue;
            }
            let Ok(name) =
                ferrobox_domain::package_coordinate::PackageName::parse(item.name.clone())
            else {
                continue;
            };
            let Ok(version) = PackageVersion::parse(item.version.clone()) else {
                continue;
            };
            let coordinate = PackageCoordinate::new(ecosystem, name, version);
            self.assay_store
                .delete_by_coordinate(repository_id, &coordinate)
                .await?;
            self.package_index_store
                .delete_by_coordinate(repository_id, &coordinate)
                .await?;
        }
        Ok(())
    }

    async fn unreferenced_blobs(
        &self,
        remaining: &[PackageIndexRecord],
    ) -> Result<Vec<PackageIndexRecord>, RetentionError> {
        let mut referenced_digests = HashSet::new();
        for entry in remaining {
            if is_blob_name(entry.coordinate.name().as_str()) {
                continue;
            }
            let Some(artifact_id) = entry.artifact_id else {
                continue;
            };
            let body = match self.storage.get(&storage_key_for(artifact_id)).await {
                Ok(body) => body,
                Err(StorageError::NotFound(_)) => {
                    return Ok(Vec::new());
                }
                Err(err) => return Err(err.into()),
            };
            collect_manifest_digests(&body, &mut referenced_digests);
        }

        Ok(remaining
            .iter()
            .filter(|entry| {
                is_blob_name(entry.coordinate.name().as_str())
                    && !referenced_digests.contains(entry.coordinate.version().as_str())
            })
            .cloned()
            .collect())
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

fn coordinate_key(coordinate: &PackageCoordinate) -> (String, String) {
    (
        coordinate.name().as_str().to_string(),
        coordinate.version().as_str().to_string(),
    )
}

fn item_from_record(
    repository: &str,
    record: &PackageIndexRecord,
    sizes: &HashMap<ArtifactId, u64>,
    reason: String,
) -> CleanupItem {
    let size_bytes = record
        .artifact_id
        .and_then(|id| sizes.get(&id).copied())
        .unwrap_or(0);
    CleanupItem {
        repository: repository.to_string(),
        name: record.coordinate.name().as_str().to_string(),
        version: record.coordinate.version().as_str().to_string(),
        size_bytes,
        reason,
    }
}

fn drop_reason(policy: RetentionPolicy, _rank: u32, age_days: u64) -> String {
    match (policy.keep_last(), policy.keep_days()) {
        (Some(limit), None) => {
            format!("fuera de las {limit} versiones más recientes")
        }
        (None, Some(days)) => {
            format!("indexada hace {age_days} días (límite {days})")
        }
        (Some(limit), Some(days)) => {
            format!("fuera de las {limit} más recientes y con más de {days} días")
        }
        (None, None) => "fuera de la política".to_string(),
    }
}

fn versions_to_drop(
    entries: &[PackageIndexRecord],
    policy: RetentionPolicy,
    now: DateTime<Utc>,
) -> Vec<(PackageIndexRecord, String)> {
    let mut by_package: BTreeMap<String, Vec<&PackageIndexRecord>> = BTreeMap::new();
    for entry in entries {
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

    let mut dropping = Vec::new();
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
            let age = age_days(&record.created_at_rfc3339, now);
            if policy.keeps(rank, age) {
                continue;
            }
            dropping.push(((*record).clone(), drop_reason(policy, rank, age)));
        }
    }
    dropping
}

fn digest_aliases_to_drop(remaining: &[PackageIndexRecord]) -> Vec<PackageIndexRecord> {
    let mut kept_digests = HashSet::new();
    for entry in remaining {
        if is_blob_name(entry.coordinate.name().as_str())
            || is_digest_reference(entry.coordinate.version().as_str())
        {
            continue;
        }
        if let Some(digest) = manifest_digest(&entry.entry) {
            kept_digests.insert((entry.coordinate.name().as_str().to_string(), digest));
        }
    }
    remaining
        .iter()
        .filter(|entry| {
            is_digest_reference(entry.coordinate.version().as_str())
                && !is_blob_name(entry.coordinate.name().as_str())
                && !kept_digests.contains(&(
                    entry.coordinate.name().as_str().to_string(),
                    entry.coordinate.version().as_str().to_string(),
                ))
        })
        .cloned()
        .collect()
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
    async fn keep_last_drops_old_versions_from_the_index() {
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
        assert_eq!(report.deleted_artifacts, 0);
        assert_eq!(report.freed_bytes, 0);
        assert!(fixture.artifacts.find_by_id(old).await.unwrap().is_some());
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

        let gc = fixture
            .service
            .collect_garbage_only(repository.id())
            .await
            .unwrap();
        assert_eq!(gc.deleted_artifacts, 1);
        assert_eq!(gc.freed_bytes, 10);
        assert!(fixture.artifacts.find_by_id(old).await.unwrap().is_none());
        assert!(fixture.artifacts.find_by_id(mid).await.unwrap().is_some());
        assert!(fixture.artifacts.find_by_id(new).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn dry_run_lists_the_same_version_without_deleting() {
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
        publish(
            &fixture,
            repository.id(),
            "demo",
            "2.0.0",
            "2026-01-01T00:00:00Z",
        )
        .await;

        let preview = fixture
            .service
            .dry_run(
                repository.id(),
                RetentionPolicy::new(Some(1), None).unwrap(),
            )
            .await
            .unwrap();

        assert!(preview.dry_run);
        assert_eq!(preview.report.dropped_versions, 1);
        assert_eq!(preview.report.deleted_artifacts, 0);
        assert_eq!(preview.report.freed_bytes, 0);
        assert_eq!(preview.items[0].name, "demo");
        assert_eq!(preview.items[0].version, "1.0.0");
        assert!(
            fixture
                .artifacts
                .find_by_id(old)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            fixture
                .index
                .artifact_for(repository.id(), &coordinate("demo", "1.0.0"))
                .await
                .unwrap(),
            Some(old)
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

        assert_eq!(
            fixture
                .index
                .artifact_for(repository.id(), &coordinate("demo", "1.0.0"))
                .await
                .unwrap(),
            None
        );
        assert!(fixture.artifacts.find_by_id(stale).await.unwrap().is_some());
        assert!(fixture.artifacts.find_by_id(fresh).await.unwrap().is_some());

        fixture
            .service
            .collect_garbage_only(repository.id())
            .await
            .unwrap();
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
    async fn instance_gc_dry_run_lists_orphans_then_collect_deletes() {
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
        publish(
            &fixture,
            repository.id(),
            "demo",
            "2.0.0",
            "2026-01-01T00:00:00Z",
        )
        .await;
        fixture
            .service
            .apply_policy(
                repository.id(),
                RetentionPolicy::new(Some(1), None).unwrap(),
            )
            .await
            .unwrap();

        let preview = fixture
            .service
            .dry_run_garbage_collection()
            .await
            .unwrap();
        assert!(preview.dry_run);
        assert_eq!(preview.report.deleted_artifacts, 1);
        assert_eq!(preview.items[0].repository, "crates");
        assert!(fixture.artifacts.find_by_id(old).await.unwrap().is_some());

        let collected = fixture.service.collect_garbage_all().await.unwrap();
        assert!(!collected.dry_run);
        assert_eq!(collected.report.deleted_artifacts, 1);
        assert!(fixture.artifacts.find_by_id(old).await.unwrap().is_none());
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
