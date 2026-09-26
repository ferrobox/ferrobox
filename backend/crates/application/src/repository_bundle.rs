//! Exporta e importa el contenido de un repositorio como un `.tar.gz`
//! portable (`ferrobox.repository.v1`).
//!
//! El archivo lleva `manifest.json` (ecosistema, paquetes e índice) y
//! los binarios en `blobs/<sha256>`. Sirve para copiar un `Forge` o la
//! caché de un `Mirror` a otro `Forge` de la misma instancia o de otra.

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

/// Recuento de un export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportBundle {
    /// Bytes del `.tar.gz`.
    pub bytes: Bytes,
    /// Nombre sugerido del archivo.
    pub filename: String,
    /// Paquetes indexados incluidos.
    pub packages: u32,
    /// Binarios incluidos.
    pub artifacts: u32,
    /// Entradas de índice sin binario (omitidas).
    pub skipped_index_only: u32,
}

/// Recuento de un import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    /// Coordenadas nuevas escritas.
    pub packages_imported: u32,
    /// Binarios nuevos escritos.
    pub artifacts_imported: u32,
    /// Paquetes o binarios que ya estaban.
    pub skipped: u32,
    /// Bytes escritos en el destino.
    pub bytes_copied: u64,
}

/// Motivos por los que exportar o importar puede fallar.
#[derive(Debug, Error)]
pub enum BundleError {
    /// Un `Alloy` no se exporta ni recibe un import.
    #[error("cannot {0} an Alloy repository")]
    Alloy(&'static str),

    /// El destino de un import tiene que ser un `Forge`.
    #[error("cannot import into a {0} repository")]
    TargetNotForge(&'static str),

    /// El archivo no es un bundle válido.
    #[error("invalid repository bundle: {0}")]
    InvalidBundle(String),

    /// El ecosistema del archivo no coincide con el destino.
    #[error("bundle ecosystem '{bundle}' does not match repository '{repository}'")]
    EcosystemMismatch {
        /// Ecosistema del archivo.
        bundle: &'static str,
        /// Ecosistema del repositorio destino.
        repository: &'static str,
    },

    /// El repositorio no existe.
    #[error(transparent)]
    Repository(#[from] GetRepositoryError),

    /// Fallo al listar repositorios.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),

    /// Fallo al listar o guardar artefactos.
    #[error(transparent)]
    Artifact(#[from] ArtifactStoreError),

    /// Fallo al leer o escribir el índice.
    #[error(transparent)]
    Index(#[from] PackageIndexStoreError),

    /// Fallo al leer o escribir objetos.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// El destino no admite más binarios.
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

/// Exporta e importa archivos portables de un repositorio.
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
    /// Construye el servicio a partir de sus puertos.
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

    /// Empaqueta el contenido cacheado de un `Forge` o un `Mirror`.
    ///
    /// # Errors
    ///
    /// [`BundleError`] si el repositorio es un `Alloy` o falla un puerto.
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
                if let std::collections::hash_map::Entry::Vacant(slot) = blobs.entry(checksum.clone())
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

    /// Restaura un bundle en un `Forge` del mismo ecosistema.
    ///
    /// Las coordenadas o checksums que ya existen se omiten.
    ///
    /// # Errors
    ///
    /// [`BundleError`] si el destino no es un `Forge`, el archivo no es
    /// válido, el ecosistema no coincide, o falla un puerto.
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
            .write_new_blobs(repository.id(), &manifest.artifacts, &blobs, &mut checksum_to_id)
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
            if self
                .package_index
                .artifact_for(repository_id, &coordinate)
                .await?
                .is_some()
            {
                count.skipped += 1;
                continue;
            }
            let entry = BASE64
                .decode(package.index_entry_b64.as_bytes())
                .map_err(|err| BundleError::InvalidBundle(err.to_string()))?;
            let artifact_id = match package.checksum.as_ref() {
                Some(checksum) => Some(*checksum_to_id.get(checksum).ok_or_else(|| {
                    BundleError::InvalidBundle(format!("package refers to missing blob {checksum}"))
                })?),
                None => None,
            };
            self.package_index
                .upsert_entry(repository_id, &coordinate, artifact_id, Bytes::from(entry))
                .await?;
            count.imported += 1;
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

fn write_bundle(
    manifest: &[u8],
    blobs: &HashMap<String, Bytes>,
) -> Result<Bytes, BundleError> {
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
    use crate::prefetch_package::PrefetchPackageUseCase;
    use crate::publish_artifact::PublishArtifactUseCase;
    use crate::packaging::PackagingRegistry;
    use crate::packaging::cargo::CargoPackagingStrategy;
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

        let bundles = service(
            repositories,
            artifacts,
            index.clone(),
            storage,
        );
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
        assert!(index
            .artifact_for(target.id(), &coordinate)
            .await
            .unwrap()
            .is_some());
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
