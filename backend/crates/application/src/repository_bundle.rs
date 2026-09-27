//! Exports and imports the content of a repository as a portable
//! `.tar.gz` (`ferrobox.repository.v1`).
//!
//! The archive holds `manifest.json` (ecosystem, packages, and index)
//! and the binaries in `blobs/<sha256>`. Used to copy a `Forge` or a
//! `Mirror` cache to another `Forge` of the same instance or another.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use chrono::Utc;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::checksum::Sha256Checksum;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use tar::{Archive, Builder, Header};
use thiserror::Error;

use crate::content_hash::sha256_checksum;
use crate::get_repository::{GetRepositoryError, GetRepositoryUseCase};
use crate::quota::{QuotaError, QuotaService};
use crate::storage_key::storage_key_for;

const BUNDLE_FORMAT: &str = "ferrobox.repository.v1";

/// Count of an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportBundle {
    /// Bytes of the `.tar.gz`.
    pub bytes: Bytes,
    /// Suggested filename.
    pub filename: String,
    /// Indexed packages included.
    pub packages: u32,
    /// Binaries included.
    pub artifacts: u32,
    /// Index entries without a binary (omitted).
    pub skipped_index_only: u32,
}

/// Count of an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    /// New coordinates written.
    pub packages_imported: u32,
    /// New binaries written.
    pub artifacts_imported: u32,
    /// Packages or binaries that were already there.
    pub skipped: u32,
    /// Bytes written to the destination.
    pub bytes_copied: u64,
}

/// Reasons exporting or importing can fail.
#[derive(Debug, Error)]
pub enum BundleError {
    /// An `Alloy` is not exported and does not receive an import.
    #[error("cannot {0} an Alloy repository")]
    Alloy(&'static str),

    /// The destination of an import must be a `Forge`.
    #[error("cannot import into a {0} repository")]
    TargetNotForge(&'static str),

    /// The archive is not a valid bundle.
    #[error("invalid repository bundle: {0}")]
    InvalidBundle(String),

    /// The archive ecosystem does not match the destination.
    #[error("bundle ecosystem '{bundle}' does not match repository '{repository}'")]
    EcosystemMismatch {
        /// Archive ecosystem.
        bundle: &'static str,
        /// Destination repository ecosystem.
        repository: &'static str,
    },

    /// The repository does not exist.
    #[error(transparent)]
    Repository(#[from] GetRepositoryError),

    /// Failed to list repositories.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),

    /// Failed to list or save artifacts.
    #[error(transparent)]
    Artifact(#[from] ArtifactStoreError),

    /// Failed to read or write the index.
    #[error(transparent)]
    Index(#[from] PackageIndexStoreError),

    /// Failed to read or write objects.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// The destination cannot accept more binaries.
    #[error(transparent)]
    Quota(#[from] QuotaError),
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleManifest {
    format: String,
    ecosystem: String,
    source_name: String,
    source_kind: String,
    exported_at: String,
    packages: Vec<BundlePackage>,
    artifacts: Vec<BundleBlob>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundlePackage {
    name: String,
    version: String,
    checksum: Option<String>,
    index_entry_b64: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleBlob {
    checksum: String,
    size_bytes: u64,
}

/// Exports and imports portable archives of a repository.
#[derive(Clone)]
#[allow(clippy::struct_field_names)]
pub struct RepositoryBundleService {
    repositories: GetRepositoryUseCase,
    artifact_store: Arc<dyn ArtifactStore>,
    package_index: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    quota: QuotaService,
}

impl RepositoryBundleService {
    /// Builds the service from its ports.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        package_index: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        quota: QuotaService,
    ) -> Self {
        Self {
            repositories: GetRepositoryUseCase::new(repository_store),
            artifact_store,
            package_index,
            storage,
            quota,
        }
    }

    /// Packs the cached content of a `Forge` or a `Mirror`.
    ///
    /// # Errors
    ///
    /// [`BundleError`] if the repository is an `Alloy` or a port fails.
    pub async fn export(&self, repository_id: RepositoryId) -> Result<ExportBundle, BundleError> {
        let repository = self.repositories.execute(repository_id).await?;
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(BundleError::Alloy("export"));
        }

        let mut blobs: HashMap<String, Bytes> = HashMap::new();
        let mut packages = Vec::new();
        let mut skipped_index_only = 0;

        if repository.ecosystem() != PackageEcosystem::Generic {
            for record in self.package_index.list_entries(repository.id()).await? {
                let Some(artifact_id) = record.artifact_id else {
                    skipped_index_only += 1;
                    continue;
                };
                let Some(artifact) = self.artifact_store.find_by_id(artifact_id).await? else {
                    skipped_index_only += 1;
                    continue;
                };
                let checksum = artifact.checksum().to_string();
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    blobs.entry(checksum.clone())
                {
                    let content = self.storage.get(&storage_key_for(artifact_id)).await?;
                    let actual = sha256_checksum(&content);
                    if actual != *artifact.checksum() {
                        return Err(BundleError::InvalidBundle(format!(
                            "stored blob checksum mismatch for {}",
                            artifact.id()
                        )));
                    }
                    slot.insert(content);
                }
                packages.push(BundlePackage {
                    name: record.coordinate.name().as_str().to_string(),
                    version: record.coordinate.version().as_str().to_string(),
                    checksum: Some(checksum),
                    index_entry_b64: BASE64.encode(&record.entry),
                });
            }
        }

        if repository.ecosystem() == PackageEcosystem::Generic {
            for artifact in self
                .artifact_store
                .find_by_repository_id(repository.id())
                .await?
            {
                let checksum = artifact.checksum().to_string();
                if blobs.contains_key(&checksum) {
                    continue;
                }
                let content = self.storage.get(&storage_key_for(artifact.id())).await?;
                blobs.insert(checksum, content);
            }
        }

        let artifacts: Vec<BundleBlob> = blobs
            .iter()
            .map(|(checksum, content)| BundleBlob {
                checksum: checksum.clone(),
                size_bytes: content.len() as u64,
            })
            .collect();

        let manifest = BundleManifest {
            format: BUNDLE_FORMAT.to_string(),
            ecosystem: repository.ecosystem().label().to_string(),
            source_name: repository.name().as_str().to_string(),
            source_kind: repository.kind().label().to_string(),
            exported_at: Utc::now().to_rfc3339(),
            packages,
            artifacts,
        };
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;

        let archive_bytes = write_bundle(&manifest_bytes, &blobs)?;
        let package_count = u32::try_from(manifest.packages.len()).unwrap_or(u32::MAX);
        let artifact_count = u32::try_from(blobs.len()).unwrap_or(u32::MAX);

        Ok(ExportBundle {
            bytes: archive_bytes,
            filename: format!("{}.ferrobox.tar.gz", repository.name().as_str()),
            packages: package_count,
            artifacts: artifact_count,
            skipped_index_only,
        })
    }

    /// Restores a bundle into a `Forge` of the same ecosystem.
    ///
    /// Coordinates or checksums that already exist are omitted.
    ///
    /// # Errors
    ///
    /// [`BundleError`] if the destination is not a `Forge`, the archive
    /// is not valid, the ecosystem does not match, or a port fails.
    pub async fn import(
        &self,
        repository_id: RepositoryId,
        archive: Bytes,
    ) -> Result<ImportOutcome, BundleError> {
        let repository = self.repositories.execute(repository_id).await?;
        match repository.kind() {
            RepositoryKind::Forge => {}
            RepositoryKind::Alloy { .. } => return Err(BundleError::Alloy("import")),
            other @ RepositoryKind::Mirror { .. } => {
                return Err(BundleError::TargetNotForge(other.label()));
            }
        }

        let (manifest, blobs) = read_bundle(&archive)?;
        if manifest.format != BUNDLE_FORMAT {
            return Err(BundleError::InvalidBundle(format!(
                "unsupported format '{}'",
                manifest.format
            )));
        }
        let bundle_ecosystem = ecosystem_from_label(&manifest.ecosystem).ok_or_else(|| {
            BundleError::InvalidBundle(format!("unknown ecosystem '{}'", manifest.ecosystem))
        })?;
        if bundle_ecosystem != repository.ecosystem() {
            return Err(BundleError::EcosystemMismatch {
                bundle: bundle_ecosystem.label(),
                repository: repository.ecosystem().label(),
            });
        }

        let existing = self
            .artifact_store
            .find_by_repository_id(repository.id())
            .await?;
        let mut checksum_to_id: HashMap<String, ferrobox_domain::ids::ArtifactId> = existing
            .iter()
            .map(|artifact| (artifact.checksum().to_string(), artifact.id()))
            .collect();

        let mut new_bytes = 0_u64;
        for blob in &manifest.artifacts {
            if checksum_to_id.contains_key(&blob.checksum) {
                continue;
            }
            let content = blobs.get(&blob.checksum).ok_or_else(|| {
                BundleError::InvalidBundle(format!("missing blob {}", blob.checksum))
            })?;
            new_bytes = new_bytes.saturating_add(content.len() as u64);
        }
        self.quota
            .ensure_can_store(repository.id(), new_bytes)
            .await?;

        let written = self
            .write_new_blobs(
                repository.id(),
                &manifest.artifacts,
                &blobs,
                &mut checksum_to_id,
            )
            .await?;
        let indexed = self
            .write_packages(
                repository.id(),
                bundle_ecosystem,
                &manifest.packages,
                &checksum_to_id,
            )
            .await?;

        Ok(ImportOutcome {
            packages_imported: indexed.imported,
            artifacts_imported: written.imported,
            skipped: written.skipped + indexed.skipped,
            bytes_copied: written.bytes_copied,
        })
    }

    async fn write_new_blobs(
        &self,
        repository_id: RepositoryId,
        declared: &[BundleBlob],
        blobs: &HashMap<String, Bytes>,
        checksum_to_id: &mut HashMap<String, ferrobox_domain::ids::ArtifactId>,
    ) -> Result<WriteCount, BundleError> {
        let mut count = WriteCount::default();
        for blob in declared {
            if checksum_to_id.contains_key(&blob.checksum) {
                count.skipped += 1;
                continue;
            }
            let content = blobs.get(&blob.checksum).cloned().ok_or_else(|| {
                BundleError::InvalidBundle(format!("missing blob {}", blob.checksum))
            })?;
            let checksum = Sha256Checksum::parse(blob.checksum.clone())
                .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
            let actual = sha256_checksum(&content);
            if actual != checksum {
                return Err(BundleError::InvalidBundle(format!(
                    "blob {} does not match its bytes",
                    blob.checksum
                )));
            }
            let artifact = Artifact::new(repository_id, checksum, content.len() as u64);
            self.storage
                .put(&storage_key_for(artifact.id()), content)
                .await?;
            self.artifact_store.save(&artifact).await?;
            checksum_to_id.insert(blob.checksum.clone(), artifact.id());
            count.imported += 1;
            count.bytes_copied += blob.size_bytes;
        }
        Ok(count)
    }

    async fn write_packages(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        packages: &[BundlePackage],
        checksum_to_id: &HashMap<String, ferrobox_domain::ids::ArtifactId>,
    ) -> Result<WriteCount, BundleError> {
        let mut count = WriteCount::default();
        for package in packages {
            let name = PackageName::parse(package.name.clone())
                .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
            let version = PackageVersion::parse(package.version.clone())
                .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
            let coordinate = PackageCoordinate::new(ecosystem, name, version);
            let already = self
                .package_index
                .artifact_for(repository_id, &coordinate)
                .await?
                .is_some();
            let entry = BASE64
                .decode(package.index_entry_b64.as_bytes())
                .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
            let artifact_id = match package.checksum.as_ref() {
                Some(checksum) => Some(*checksum_to_id.get(checksum).ok_or_else(|| {
                    BundleError::InvalidBundle(format!("package refers to missing blob {checksum}"))
                })?),
                None => None,
            };
            let entry = remap_index_entry(Bytes::from(entry), artifact_id);
            self.package_index
                .upsert_entry(repository_id, &coordinate, artifact_id, entry)
                .await?;
            if already {
                count.skipped += 1;
            } else {
                count.imported += 1;
            }
        }
        Ok(count)
    }
}

#[derive(Default)]
struct WriteCount {
    imported: u32,
    skipped: u32,
    bytes_copied: u64,
}

fn write_bundle(manifest: &[u8], blobs: &HashMap<String, Bytes>) -> Result<Bytes, BundleError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = Builder::new(&mut encoder);
        append_file(&mut archive, "manifest.json", manifest)?;
        let mut checksums: Vec<_> = blobs.keys().cloned().collect();
        checksums.sort();
        for checksum in checksums {
            let content = &blobs[&checksum];
            append_file(&mut archive, &format!("blobs/{checksum}"), content)?;
        }
        archive
            .finish()
            .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
    }
    encoder
        .finish()
        .map(Bytes::from)
        .map_err(|err| BundleError::InvalidBundle(err.to_string()))
}

fn append_file<W: std::io::Write>(
    archive: &mut Builder<W>,
    path: &str,
    content: &[u8],
) -> Result<(), BundleError> {
    let mut header = Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, path, content)
        .map_err(|err| BundleError::InvalidBundle(err.to_string()))
}

fn read_bundle(archive: &[u8]) -> Result<(BundleManifest, HashMap<String, Bytes>), BundleError> {
    let decoder = GzDecoder::new(Cursor::new(archive));
    let mut tar = Archive::new(decoder);
    let mut manifest = None;
    let mut blobs = HashMap::new();

    let entries = tar
        .entries()
        .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
        let path = entry
            .path()
            .map_err(|err| BundleError::InvalidBundle(err.to_string()))?
            .into_owned();
        let path = path.to_string_lossy();
        if path.contains("..") {
            return Err(BundleError::InvalidBundle("path traversal".to_string()));
        }
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
        if path == "manifest.json" {
            manifest = Some(
                serde_json::from_slice::<BundleManifest>(&data)
                    .map_err(|err| BundleError::InvalidBundle(err.to_string()))?,
            );
        } else if let Some(checksum) = path.strip_prefix("blobs/") {
            blobs.insert(checksum.to_string(), Bytes::from(data));
        }
    }

    let manifest =
        manifest.ok_or_else(|| BundleError::InvalidBundle("missing manifest.json".to_string()))?;
    Ok((manifest, blobs))
}

/// The source index stores local UUIDs in `files[].artifact_id`.
/// After copying the blob, those keys must point to the new artifact
/// or the destination lists empty and NuGet cannot download the `.nupkg`.
fn remap_index_entry(entry: Bytes, dest_id: Option<ferrobox_domain::ids::ArtifactId>) -> Bytes {
    let Some(dest_id) = dest_id else {
        return entry;
    };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&entry) else {
        return entry;
    };
    let dest = dest_id.to_string();
    if let Some(files) = value
        .get_mut("files")
        .and_then(serde_json::Value::as_array_mut)
    {
        for file in files {
            if let Some(id) = file.get_mut("artifact_id") {
                *id = serde_json::Value::String(dest.clone());
            }
        }
    }
    if value
        .get("artifact_id")
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        value["artifact_id"] = serde_json::Value::String(dest);
    }
    serde_json::to_vec(&value).map(Bytes::from).unwrap_or(entry)
}

fn ecosystem_from_label(label: &str) -> Option<PackageEcosystem> {
    match label {
        "generic" => Some(PackageEcosystem::Generic),
        "cargo" => Some(PackageEcosystem::Cargo),
        "npm" => Some(PackageEcosystem::Npm),
        "pypi" => Some(PackageEcosystem::PyPi),
        "oci" => Some(PackageEcosystem::Oci),
        "helm" => Some(PackageEcosystem::Helm),
        "conan" => Some(PackageEcosystem::Conan),
        "maven" => Some(PackageEcosystem::Maven),
        "nuget" => Some(PackageEcosystem::Nuget),
        "go" => Some(PackageEcosystem::Go),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::packaging::PackagingRegistry;
    use crate::packaging::cargo::CargoPackagingStrategy;
    use crate::prefetch_package::PrefetchPackageUseCase;
    use crate::publish_artifact::PublishArtifactUseCase;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn service(
        repositories: Arc<InMemoryRepositoryStore>,
        artifacts: Arc<InMemoryArtifactStore>,
        index: Arc<InMemoryPackageIndexStore>,
        storage: Arc<InMemoryStorage>,
    ) -> RepositoryBundleService {
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        RepositoryBundleService::new(repositories, artifacts, index, storage, quota)
    }

    fn forge(name: &str, ecosystem: PackageEcosystem) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            ecosystem,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn exports_and_imports_a_generic_artifact() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let source = forge("generic-src", PackageEcosystem::Generic);
        let target = forge("generic-dst", PackageEcosystem::Generic);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota,
        )
        .execute(source.id(), Bytes::from_static(b"backup-bytes"))
        .await
        .unwrap();

        let bundles = service(
            repositories.clone(),
            artifacts.clone(),
            index,
            storage.clone(),
        );
        let exported = bundles.export(source.id()).await.unwrap();
        assert_eq!(exported.artifacts, 1);
        assert!(exported.filename.ends_with(".ferrobox.tar.gz"));

        let imported = bundles.import(target.id(), exported.bytes).await.unwrap();
        assert_eq!(imported.artifacts_imported, 1);
        assert_eq!(imported.bytes_copied, 12);
        assert_eq!(
            artifacts
                .find_by_repository_id(target.id())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn imports_are_idempotent() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let source = forge("generic-src", PackageEcosystem::Generic);
        let target = forge("generic-dst", PackageEcosystem::Generic);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota,
        )
        .execute(source.id(), Bytes::from_static(b"same"))
        .await
        .unwrap();
        let bundles = service(repositories, artifacts, index, storage);
        let exported = bundles.export(source.id()).await.unwrap();
        bundles
            .import(target.id(), exported.bytes.clone())
            .await
            .unwrap();
        let second = bundles.import(target.id(), exported.bytes).await.unwrap();
        assert_eq!(second.artifacts_imported, 0);
        assert_eq!(second.skipped, 1);
    }

    #[tokio::test]
    async fn exports_a_cached_crate_and_imports_it_into_a_forge() {
        let http = Arc::new(InMemoryHttpClient::default());
        let crate_bytes = Bytes::from_static(b"cached-crate-bytes");
        let cksum = crate::content_hash::sha256_checksum(&crate_bytes).to_string();
        let index_line = format!(
            r#"{{"name":"demo","vers":"1.2.3","deps":[],"cksum":"{cksum}","features":{{}},"yanked":false}}"#
        );
        http.stub(
            "https://index.example/de/mo/demo",
            200,
            Bytes::from(format!("{index_line}\n")),
        );
        http.stub(
            "https://index.example/config.json",
            200,
            Bytes::from_static(
                br#"{"dl":"https://static.example/crates/{crate}/{crate}-{version}.crate","api":"https://index.example"}"#,
            ),
        );
        http.stub(
            "https://static.example/crates/demo/demo-1.2.3.crate",
            200,
            crate_bytes,
        );

        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = CargoPackagingStrategy::new(
            artifacts.clone(),
            index.clone(),
            storage.clone(),
            http,
            repositories.clone(),
        );
        let packaging = PackagingRegistry::new().register(Arc::new(strategy));
        let source = Repository::new(
            RepositoryName::parse("crates-io").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.example/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let target = forge("cargo-backup", PackageEcosystem::Cargo);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        PrefetchPackageUseCase::execute(
            &packaging,
            &source,
            PackageName::parse("demo").unwrap(),
            Some(PackageVersion::parse("1.2.3").unwrap()),
        )
        .await
        .unwrap();

        let bundles = service(repositories, artifacts, index.clone(), storage);
        let exported = bundles.export(source.id()).await.unwrap();
        assert_eq!(exported.packages, 1);
        assert_eq!(exported.artifacts, 1);

        let imported = bundles.import(target.id(), exported.bytes).await.unwrap();
        assert_eq!(imported.packages_imported, 1);
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.2.3").unwrap(),
        );
        assert!(
            index
                .artifact_for(target.id(), &coordinate)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn import_rewrites_nuget_file_artifact_ids_and_repairs_a_second_push() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let source = forge("nuget-src", PackageEcosystem::Nuget);
        let target = forge("nuget-dst", PackageEcosystem::Nuget);
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();

        let body = Bytes::from_static(b"nupkg-bytes");
        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(source.id(), checksum, body.len() as u64);
        storage
            .put(&storage_key_for(artifact.id()), body)
            .await
            .unwrap();
        artifacts.save(&artifact).await.unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Nuget,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let entry = format!(
            r#"{{"id":"Demo","version":"1.0.0","files":[{{"filename":"Demo.1.0.0.nupkg","artifact_id":"{}","yanked":false}}]}}"#,
            artifact.id()
        );
        index
            .upsert_entry(
                source.id(),
                &coordinate,
                Some(artifact.id()),
                Bytes::from(entry),
            )
            .await
            .unwrap();

        let bundles = service(repositories, artifacts.clone(), index.clone(), storage);
        let exported = bundles.export(source.id()).await.unwrap();
        let first = bundles
            .import(target.id(), exported.bytes.clone())
            .await
            .unwrap();
        assert_eq!(first.packages_imported, 1);
        assert_eq!(first.artifacts_imported, 1);

        let dest_artifact = artifacts
            .find_by_repository_id(target.id())
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("dest blob");
        let dest_entry = index
            .list_entries(target.id())
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("dest index");
        let parsed: serde_json::Value = serde_json::from_slice(&dest_entry.entry).unwrap();
        assert_eq!(
            parsed["files"][0]["artifact_id"].as_str(),
            Some(dest_artifact.id().to_string().as_str())
        );
        assert_ne!(dest_artifact.id(), artifact.id());

        let second = bundles.import(target.id(), exported.bytes).await.unwrap();
        assert_eq!(second.packages_imported, 0);
        assert_eq!(second.skipped, 2);
    }

    #[tokio::test]
    async fn rejects_import_into_a_mirror() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let source = forge("generic-src", PackageEcosystem::Generic);
        let target = Repository::new(
            RepositoryName::parse("mirror").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://example/").unwrap(),
            },
            PackageEcosystem::Generic,
        )
        .unwrap();
        repositories.save(&source).await.unwrap();
        repositories.save(&target).await.unwrap();
        let quota = QuotaService::new(
            repositories.clone(),
            artifacts.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        PublishArtifactUseCase::new(
            repositories.clone(),
            artifacts.clone(),
            storage.clone(),
            quota,
        )
        .execute(source.id(), Bytes::from_static(b"x"))
        .await
        .unwrap();
        let bundles = service(repositories, artifacts, index, storage);
        let exported = bundles.export(source.id()).await.unwrap();
        let err = bundles
            .import(target.id(), exported.bytes)
            .await
            .unwrap_err();
        assert!(matches!(err, BundleError::TargetNotForge("mirror")));
    }

    #[tokio::test]
    async fn rejects_an_alloy() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let member = forge("member", PackageEcosystem::Generic);
        let alloy = Repository::new(
            RepositoryName::parse("all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![member.id()],
            },
            PackageEcosystem::Generic,
        )
        .unwrap();
        repositories.save(&member).await.unwrap();
        repositories.save(&alloy).await.unwrap();
        let bundles = service(
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
        );
        let err = bundles.export(alloy.id()).await.unwrap_err();
        assert!(matches!(err, BundleError::Alloy("export")));
    }
}
