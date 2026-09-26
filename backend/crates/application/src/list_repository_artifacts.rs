use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::admission_store::AdmissionStore;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::StoragePort;
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::packaging::cosign;
use crate::storage_key::storage_key_for;

/// Artefacto listado junto con el nombre y la versión de paquete, si el
/// índice de su ecosistema los conoce (por ejemplo, un crate publicado
/// con `cargo publish`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedArtifact {
    artifact: Artifact,
    package_name: Option<String>,
    package_version: Option<String>,
    yanked: bool,
    filename: Option<String>,
    signed: bool,
    verified: bool,
}

impl ListedArtifact {
    /// Metadatos binarios del artefacto.
    #[must_use]
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }

    /// Nombre del paquete indexado, si existe.
    #[must_use]
    pub fn package_name(&self) -> Option<&str> {
        self.package_name.as_deref()
    }

    /// Versión del paquete indexado, si existe.
    #[must_use]
    pub fn package_version(&self) -> Option<&str> {
        self.package_version.as_deref()
    }

    /// `true` si el índice marca esta versión como *yanked*.
    #[must_use]
    pub fn yanked(&self) -> bool {
        self.yanked
    }

    /// Nombre de fichero en el índice (receta `Conan`, sdist de `PyPI`, etc.).
    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// `true` si hay una firma Cosign / Notation enlazada a este artefacto.
    #[must_use]
    pub fn signed(&self) -> bool {
        self.signed
    }

    /// `true` si alguna firma Cosign simple verifica contra las claves
    /// del repositorio.
    #[must_use]
    pub fn verified(&self) -> bool {
        self.verified
    }
}

/// Motivos por los que listar los artefactos de un repositorio puede
/// fallar.
#[derive(Debug, Error)]
pub enum ListRepositoryArtifactsError {
    /// Fallo al consultar el almacén de artefactos.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),

    /// Fallo al consultar el índice de paquetes.
    #[error(transparent)]
    IndexPersistence(#[from] PackageIndexStoreError),

    /// Fallo al consultar el almacén de repositorios (p. ej. al
    /// resolver los miembros de un `Alloy`).
    #[error(transparent)]
    RepositoryPersistence(#[from] RepositoryStoreError),
}

/// Caso de uso: listar los artefactos de un repositorio.
#[derive(Clone)]
#[allow(clippy::struct_field_names)]
pub struct ListRepositoryArtifactsUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    cosign_verify: Option<CosignVerifyPorts>,
}

#[derive(Clone)]
struct CosignVerifyPorts {
    storage: Arc<dyn StoragePort>,
    admission: Arc<dyn AdmissionStore>,
}

impl ListRepositoryArtifactsUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            package_index_store,
            cosign_verify: None,
        }
    }

    /// Carga manifiestos Cosign y verifica contra las claves del
    /// repositorio para el badge «Verificada» y el dry-run.
    #[must_use]
    pub fn with_cosign_verify(
        mut self,
        storage: Arc<dyn StoragePort>,
        admission: Arc<dyn AdmissionStore>,
    ) -> Self {
        self.cosign_verify = Some(CosignVerifyPorts { storage, admission });
        self
    }

    /// Lista los artefactos del repositorio indicado, enriquecidos con
    /// nombre y versión cuando el índice de paquetes los conoce. Si el
    /// repositorio es un `Alloy`, une los artefactos de sus miembros.
    ///
    /// # Errors
    ///
    /// Devuelve [`ListRepositoryArtifactsError`] si falla un puerto.
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<ListedArtifact>, ListRepositoryArtifactsError> {
        self.list_all(repository_id, None).await
    }

    /// Como [`Self::execute`], pero usa `public_keys_pem` para verificar
    /// (dry-run de «no está verificada» sin guardar todavía).
    ///
    /// # Errors
    ///
    /// Devuelve [`ListRepositoryArtifactsError`] si falla un puerto.
    pub async fn execute_with_keys(
        &self,
        repository_id: RepositoryId,
        public_keys_pem: &str,
    ) -> Result<Vec<ListedArtifact>, ListRepositoryArtifactsError> {
        self.list_all(repository_id, Some(public_keys_pem)).await
    }

    async fn list_all(
        &self,
        repository_id: RepositoryId,
        keys_override: Option<&str>,
    ) -> Result<Vec<ListedArtifact>, ListRepositoryArtifactsError> {
        let ids = match self.repository_store.find_by_id(repository_id).await? {
            Some(repository) => match repository.kind() {
                RepositoryKind::Alloy { members } => members.clone(),
                _ => vec![repository_id],
            },
            None => vec![repository_id],
        };

        let mut listed = Vec::new();
        for id in ids {
            let hide_unindexed = self
                .repository_store
                .find_by_id(id)
                .await?
                .is_some_and(|repository| repository.ecosystem() != PackageEcosystem::Generic);
            listed.extend(self.list_one(id, hide_unindexed, keys_override).await?);
        }
        sort_listed(&mut listed);
        Ok(listed)
    }

    async fn list_one(
        &self,
        repository_id: RepositoryId,
        hide_unindexed: bool,
        keys_override: Option<&str>,
    ) -> Result<Vec<ListedArtifact>, ListRepositoryArtifactsError> {
        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;
        let indexed = self
            .package_index_store
            .find_indexed_by_repository(repository_id)
            .await?;

        let mut names_by_artifact = HashMap::new();
        let mut signed_subjects = HashSet::new();
        let mut signatures = Vec::new();
        let mut blobs = HashMap::new();
        for item in &indexed {
            collect_signature(&mut signed_subjects, &mut signatures, item);
            collect_blob_id(&mut blobs, item);
            apply_index_item(&mut names_by_artifact, item);
        }

        let verified_subjects = self
            .verified_subjects(
                repository_id,
                keys_override,
                &signed_subjects,
                &signatures,
                &blobs,
            )
            .await;

        Ok(artifacts
            .into_iter()
            .filter_map(|artifact| {
                let meta = names_by_artifact.remove(&artifact.id());
                if hide_unindexed && meta.is_none() {
                    return None;
                }
                let checksum = artifact.checksum().to_string();
                let signed = signed_subjects
                    .iter()
                    .any(|digest| cosign::digest_matches_checksum(digest, &checksum));
                let verified = verified_subjects
                    .iter()
                    .any(|digest| cosign::digest_matches_checksum(digest, &checksum));
                Some(ListedArtifact {
                    artifact,
                    package_name: meta.as_ref().map(|item| item.name.clone()),
                    package_version: meta.as_ref().map(|item| item.version.clone()),
                    yanked: meta.as_ref().is_some_and(|item| item.yanked),
                    filename: meta.and_then(|item| item.filename),
                    signed,
                    verified,
                })
            })
            .collect())
    }

    async fn verified_subjects(
        &self,
        repository_id: RepositoryId,
        keys_override: Option<&str>,
        signed_subjects: &HashSet<String>,
        signatures: &[SignatureCandidate],
        blobs: &HashMap<String, ArtifactId>,
    ) -> HashSet<String> {
        let Some(ports) = &self.cosign_verify else {
            return HashSet::new();
        };
        if signed_subjects.is_empty() {
            return HashSet::new();
        }
        let keys = match keys_override {
            Some(keys) => keys.to_string(),
            None => ports
                .admission
                .find_by_repository(repository_id)
                .await
                .map(|record| record.public_keys_pem)
                .unwrap_or_default(),
        };
        if keys.trim().is_empty() {
            return HashSet::new();
        }
        let mut verified = HashSet::new();
        for digest in signed_subjects {
            if verify_subject(&*ports.storage, &keys, signatures, blobs, digest).await {
                verified.insert(digest.clone());
            }
        }
        verified
    }
}

struct ArtifactIndexMeta {
    name: String,
    version: String,
    yanked: bool,
    filename: Option<String>,
}

#[derive(Deserialize, Default)]
struct IndexEntryMeta {
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    files: Vec<IndexFileMeta>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    accessory: Option<String>,
}

#[derive(Deserialize)]
struct IndexFileMeta {
    artifact_id: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    filename: Option<String>,
}

struct SignatureCandidate {
    subject: String,
    manifest_id: ArtifactId,
}

fn collect_signature(
    signed_subjects: &mut HashSet<String>,
    signatures: &mut Vec<SignatureCandidate>,
    item: &ferrobox_ports::package_index_store::IndexedArtifact,
) {
    let meta = serde_json::from_slice::<IndexEntryMeta>(&item.entry).unwrap_or_default();
    let version = item.coordinate.version().as_str();
    if !cosign::is_signature_accessory(meta.accessory.as_deref(), version) {
        return;
    }
    if let Some(subject) = cosign::signature_subject(meta.subject.as_deref(), version) {
        signed_subjects.insert(subject.clone());
        signatures.push(SignatureCandidate {
            subject,
            manifest_id: item.artifact_id,
        });
    }
}

fn collect_blob_id(
    blobs: &mut HashMap<String, ArtifactId>,
    item: &ferrobox_ports::package_index_store::IndexedArtifact,
) {
    let name = item.coordinate.name().as_str();
    let version = item
        .coordinate
        .version()
        .as_str()
        .trim()
        .to_ascii_lowercase();
    if name == "_blob" && cosign::is_digest_reference(&version) {
        blobs.insert(version, item.artifact_id);
    }
}

async fn verify_subject(
    storage: &dyn StoragePort,
    keys: &str,
    signatures: &[SignatureCandidate],
    blobs: &HashMap<String, ArtifactId>,
    subject: &str,
) -> bool {
    for signature in signatures.iter().filter(|item| item.subject == subject) {
        let Ok(manifest) = storage.get(&storage_key_for(signature.manifest_id)).await else {
            continue;
        };
        let Some(simple) = cosign::extract_simple_signature(&manifest) else {
            continue;
        };
        let Some(blob_id) = blobs.get(&simple.payload_digest) else {
            continue;
        };
        let Ok(payload) = storage.get(&storage_key_for(*blob_id)).await else {
            continue;
        };
        if cosign::verify_simple(&payload, &simple.signature_b64, keys, Some(subject)) {
            return true;
        }
    }
    false
}

fn apply_index_item(
    names_by_artifact: &mut HashMap<ArtifactId, ArtifactIndexMeta>,
    item: &ferrobox_ports::package_index_store::IndexedArtifact,
) {
    let name = item.coordinate.name().as_str().to_owned();
    let version = item.coordinate.version().as_str().to_owned();
    let meta = serde_json::from_slice::<IndexEntryMeta>(&item.entry).unwrap_or_default();
    if cosign::should_hide_from_listing(&name, &version, meta.accessory.as_deref()) {
        return;
    }

    let mut mapped_file = false;
    let mut filename = None;
    for file in &meta.files {
        if let Ok(uuid) = Uuid::parse_str(&file.artifact_id) {
            let file_name = file
                .filename
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            if filename.is_none() {
                filename.clone_from(&file_name);
            }
            names_by_artifact.insert(
                ArtifactId::from(uuid),
                ArtifactIndexMeta {
                    name: name.clone(),
                    version: version.clone(),
                    yanked: meta.yanked || file.yanked,
                    filename: file_name,
                },
            );
            mapped_file = true;
        }
    }

    // Tras un import, `files[].artifact_id` puede ser el UUID del origen.
    // La columna del índice es el artefacto del destino.
    if mapped_file {
        names_by_artifact
            .entry(item.artifact_id)
            .or_insert(ArtifactIndexMeta {
                name,
                version,
                yanked: meta.yanked,
                filename,
            });
    } else {
        let incoming = ArtifactIndexMeta {
            name,
            version,
            yanked: meta.yanked,
            filename: None,
        };
        match names_by_artifact.entry(item.artifact_id) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(incoming);
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                if cosign::is_digest_reference(&slot.get().version)
                    && !cosign::is_digest_reference(&incoming.version)
                {
                    slot.insert(incoming);
                }
            }
        }
    }
}

fn sort_listed(listed: &mut [ListedArtifact]) {
    listed.sort_by(
        |left, right| match (left.package_name(), right.package_name()) {
            (Some(left_name), Some(right_name)) => left_name
                .cmp(right_name)
                .then_with(|| left.package_version().cmp(&right.package_version()))
                .then_with(|| left.filename().cmp(&right.filename())),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left
                .artifact()
                .id()
                .to_string()
                .cmp(&right.artifact().id().to_string()),
        },
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{
        InMemoryArtifactStore, InMemoryPackageIndexStore, InMemoryRepositoryStore,
    };

    use super::*;

    fn checksum() -> Sha256Checksum {
        Sha256Checksum::parse("a".repeat(64)).unwrap()
    }

    fn use_case(
        repository_store: Arc<InMemoryRepositoryStore>,
        artifact_store: Arc<InMemoryArtifactStore>,
        package_index_store: Arc<InMemoryPackageIndexStore>,
    ) -> ListRepositoryArtifactsUseCase {
        ListRepositoryArtifactsUseCase::new(repository_store, artifact_store, package_index_store)
    }

    async fn upsert_oci(
        store: &InMemoryPackageIndexStore,
        repository_id: RepositoryId,
        name: &str,
        version: &str,
        artifact_id: ArtifactId,
        entry: serde_json::Value,
    ) {
        store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Oci,
                    PackageName::parse(name).unwrap(),
                    PackageVersion::parse(version).unwrap(),
                ),
                Some(artifact_id),
                Bytes::from(entry.to_string()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn lists_only_artifacts_belonging_to_the_repository() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let other_repository_id = RepositoryId::new();

        let matching = Artifact::new(repository_id, checksum(), 10);
        let other = Artifact::new(other_repository_id, checksum(), 20);
        artifact_store.save(&matching).await.unwrap();
        artifact_store.save(&other).await.unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].artifact(), &matching);
        assert_eq!(result[0].package_name(), None);
    }

    #[tokio::test]
    async fn hides_unindexed_artifacts_in_package_ecosystems() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("crates").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repository_store.save(&repository).await.unwrap();
        let leftover = Artifact::new(repository.id(), checksum(), 4);
        artifact_store.save(&leftover).await.unwrap();

        let result = use_case(repository_store, artifact_store, package_index_store)
            .execute(repository.id())
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn attaches_package_name_and_version_from_the_index() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let artifact = Artifact::new(repository_id, checksum(), 810);
        artifact_store.save(&artifact).await.unwrap();

        package_index_store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("demo-ferrobox").unwrap(),
                    PackageVersion::parse("0.1.0").unwrap(),
                ),
                Some(artifact.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result[0].package_name(), Some("demo-ferrobox"));
        assert_eq!(result[0].package_version(), Some("0.1.0"));
        assert!(!result[0].yanked());
    }

    #[tokio::test]
    async fn attaches_yanked_from_the_index_entry() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let artifact = Artifact::new(repository_id, checksum(), 810);
        artifact_store.save(&artifact).await.unwrap();

        package_index_store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("demo-ferrobox").unwrap(),
                    PackageVersion::parse("0.1.0").unwrap(),
                ),
                Some(artifact.id()),
                Bytes::from_static(br#"{"yanked":true}"#),
            )
            .await
            .unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert!(result[0].yanked());
    }

    #[tokio::test]
    async fn lists_nuget_when_files_still_point_at_the_source_id() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("nuget-dest").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Nuget,
        )
        .unwrap();
        repository_store.save(&repository).await.unwrap();
        let artifact = Artifact::new(repository.id(), checksum(), 810);
        artifact_store.save(&artifact).await.unwrap();
        let source_only = Uuid::now_v7();
        let entry = format!(
            r#"{{"id":"Demo","version":"1.0.0","files":[{{"filename":"Demo.1.0.0.nupkg","artifact_id":"{source_only}","yanked":false}}]}}"#
        );
        package_index_store
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Nuget,
                    PackageName::parse("demo").unwrap(),
                    PackageVersion::parse("1.0.0").unwrap(),
                ),
                Some(artifact.id()),
                Bytes::from(entry),
            )
            .await
            .unwrap();

        let result = use_case(repository_store, artifact_store, package_index_store)
            .execute(repository.id())
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].package_name(), Some("demo"));
        assert_eq!(result[0].filename(), Some("Demo.1.0.0.nupkg"));
    }

    #[tokio::test]
    async fn unions_artifacts_from_alloy_members() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());

        let first = Repository::new(
            RepositoryName::parse("crates-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let second = Repository::new(
            RepositoryName::parse("crates-other").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first.id(), second.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repository_store.save(&first).await.unwrap();
        repository_store.save(&second).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        let from_first = Artifact::new(first.id(), checksum(), 10);
        let from_second = Artifact::new(second.id(), checksum(), 20);
        artifact_store.save(&from_first).await.unwrap();
        artifact_store.save(&from_second).await.unwrap();

        package_index_store
            .upsert_entry(
                first.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("alpha").unwrap(),
                    PackageVersion::parse("1.0.0").unwrap(),
                ),
                Some(from_first.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();
        package_index_store
            .upsert_entry(
                second.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("beta").unwrap(),
                    PackageVersion::parse("2.0.0").unwrap(),
                ),
                Some(from_second.id()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let result = use_case(repository_store, artifact_store, package_index_store)
            .execute(alloy.id())
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].package_name(), Some("alpha"));
        assert_eq!(result[0].artifact().repository_id(), first.id());
        assert_eq!(result[1].package_name(), Some("beta"));
        assert_eq!(result[1].artifact().repository_id(), second.id());
    }

    #[tokio::test]
    async fn attaches_names_from_every_pypi_file_in_the_index_entry() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let sdist = Artifact::new(repository_id, checksum(), 10);
        let wheel = Artifact::new(repository_id, checksum(), 20);
        artifact_store.save(&sdist).await.unwrap();
        artifact_store.save(&wheel).await.unwrap();

        let entry = serde_json::json!({
            "name": "demo-pypi",
            "version": "1.0.0",
            "yanked": true,
            "files": [
                {
                    "filename": "demo_pypi-1.0.0.tar.gz",
                    "sha256": "aa",
                    "artifact_id": sdist.id().to_string(),
                    "yanked": true
                },
                {
                    "filename": "demo_pypi-1.0.0-py3-none-any.whl",
                    "sha256": "bb",
                    "artifact_id": wheel.id().to_string(),
                    "yanked": true
                }
            ]
        });

        package_index_store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::PyPi,
                    PackageName::parse("demo-pypi").unwrap(),
                    PackageVersion::parse("1.0.0").unwrap(),
                ),
                Some(wheel.id()),
                Bytes::from(entry.to_string()),
            )
            .await
            .unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|item| item.package_name() == Some("demo-pypi")
                    && item.package_version() == Some("1.0.0")
                    && item.yanked())
        );
        let filenames: Vec<_> = result.iter().filter_map(ListedArtifact::filename).collect();
        assert!(filenames.contains(&"demo_pypi-1.0.0.tar.gz"));
        assert!(filenames.contains(&"demo_pypi-1.0.0-py3-none-any.whl"));
    }

    #[tokio::test]
    async fn attaches_conan_recipe_filenames_from_the_index_entry() {
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository_id = RepositoryId::new();
        let recipe = Artifact::new(repository_id, checksum(), 80);
        let manifest = Artifact::new(repository_id, checksum(), 20);
        artifact_store.save(&recipe).await.unwrap();
        artifact_store.save(&manifest).await.unwrap();

        let entry = serde_json::json!({
            "name": "hello",
            "version": "0.1",
            "user": "_",
            "channel": "_",
            "yanked": false,
            "files": [
                {
                    "filename": "conanfile.py",
                    "artifact_id": recipe.id().to_string()
                },
                {
                    "filename": "conanmanifest.txt",
                    "artifact_id": manifest.id().to_string()
                }
            ]
        });

        package_index_store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Conan,
                    PackageName::parse("hello").unwrap(),
                    PackageVersion::parse("0.1@_:_").unwrap(),
                ),
                Some(manifest.id()),
                Bytes::from(entry.to_string()),
            )
            .await
            .unwrap();

        let result = use_case(
            Arc::new(InMemoryRepositoryStore::default()),
            artifact_store,
            package_index_store,
        )
        .execute(repository_id)
        .await
        .unwrap();

        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|item| item.package_name() == Some("hello")
                    && item.package_version() == Some("0.1@_:_")
                    && !item.yanked())
        );
        let filenames: Vec<_> = result.iter().filter_map(ListedArtifact::filename).collect();
        assert!(filenames.contains(&"conanfile.py"));
        assert!(filenames.contains(&"conanmanifest.txt"));
    }

    #[tokio::test]
    async fn hides_oci_blobs_digests_and_signatures_and_marks_signed() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("oci-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repository_store.save(&repository).await.unwrap();
        let repository_id = repository.id();
        let hex = "ab".repeat(32);
        let image = Artifact::new(
            repository_id,
            Sha256Checksum::parse(hex.clone()).unwrap(),
            80,
        );
        let signature = Artifact::new(repository_id, checksum(), 20);
        let blob = Artifact::new(repository_id, checksum(), 4);
        artifact_store.save(&image).await.unwrap();
        artifact_store.save(&signature).await.unwrap();
        artifact_store.save(&blob).await.unwrap();

        let digest = format!("sha256:{hex}");
        let sig_tag = format!("sha256-{hex}.sig");
        upsert_oci(
            &package_index_store,
            repository_id,
            "demo",
            "latest",
            image.id(),
            serde_json::json!({
                "name": "demo",
                "reference": "latest",
                "digest": digest,
                "media_type": "application/vnd.oci.image.manifest.v1+json",
                "size": 80,
                "artifact_id": image.id().to_string()
            }),
        )
        .await;
        upsert_oci(
            &package_index_store,
            repository_id,
            "demo",
            &digest,
            image.id(),
            serde_json::json!({
                "name": "demo",
                "reference": digest,
                "digest": digest,
                "media_type": "application/vnd.oci.image.manifest.v1+json",
                "size": 80,
                "artifact_id": image.id().to_string()
            }),
        )
        .await;
        upsert_oci(
            &package_index_store,
            repository_id,
            "demo",
            &sig_tag,
            signature.id(),
            serde_json::json!({
                "name": "demo",
                "reference": sig_tag,
                "digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "media_type": "application/vnd.oci.image.manifest.v1+json",
                "size": 20,
                "artifact_id": signature.id().to_string(),
                "subject": digest,
                "accessory": "signature",
                "artifact_type": "application/vnd.dev.cosign.simplesigning.v1+json"
            }),
        )
        .await;
        upsert_oci(
            &package_index_store,
            repository_id,
            "_blob",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            blob.id(),
            serde_json::json!({"digest":"sha256:bb"}),
        )
        .await;

        let result = use_case(repository_store, artifact_store, package_index_store)
            .execute(repository_id)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].package_name(), Some("demo"));
        assert_eq!(result[0].package_version(), Some("latest"));
        assert!(result[0].signed());
        assert!(!result[0].verified());
    }

    fn signed_simple(subject: &str) -> (Bytes, String, Bytes, String) {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;
        use p256::ecdsa::signature::Signer;
        use p256::ecdsa::{Signature, SigningKey};
        use p256::pkcs8::EncodePublicKey;
        use sha2::{Digest, Sha256};

        let signing = SigningKey::random(&mut rand::rngs::OsRng);
        let pem = signing
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .unwrap();
        let payload = Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "critical": { "image": { "docker-manifest-digest": subject } }
            }))
            .unwrap(),
        );
        let payload_digest = format!("sha256:{}", hex::encode(Sha256::digest(&payload)));
        let signature: Signature = signing.sign(&payload);
        let manifest = Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "layers": [{
                    "digest": payload_digest,
                    "annotations": {
                        "dev.cosignproject.cosign/signature": STANDARD.encode(signature.to_der())
                    }
                }]
            }))
            .unwrap(),
        );
        (payload, payload_digest, manifest, pem)
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn marks_verified_when_cosign_simple_signature_matches_keys() {
        use ferrobox_ports::admission_store::AdmissionStore;
        use ferrobox_ports::storage::StoragePort;

        use crate::storage_key::storage_key_for;
        use crate::test_support::{InMemoryAdmissionStore, InMemoryStorage};

        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let admission = Arc::new(InMemoryAdmissionStore::default());
        let repository = Repository::new(
            RepositoryName::parse("oci-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repository_store.save(&repository).await.unwrap();
        let repository_id = repository.id();
        let image_hex = "ab".repeat(32);
        let digest = format!("sha256:{image_hex}");
        let image = Artifact::new(
            repository_id,
            Sha256Checksum::parse(image_hex.clone()).unwrap(),
            80,
        );
        artifact_store.save(&image).await.unwrap();
        let (payload, payload_digest, manifest, pem) = signed_simple(&digest);
        let sig_artifact = Artifact::new(repository_id, checksum(), manifest.len() as u64);
        let blob_artifact = Artifact::new(
            repository_id,
            Sha256Checksum::parse(payload_digest.trim_start_matches("sha256:")).unwrap(),
            payload.len() as u64,
        );
        artifact_store.save(&sig_artifact).await.unwrap();
        artifact_store.save(&blob_artifact).await.unwrap();
        storage
            .put(&storage_key_for(sig_artifact.id()), manifest)
            .await
            .unwrap();
        storage
            .put(&storage_key_for(blob_artifact.id()), payload)
            .await
            .unwrap();
        admission
            .save(
                repository_id,
                ferrobox_domain::admission::AdmissionPolicy::parse(
                    false,
                    "pull",
                    "not_verified",
                    "deny",
                )
                .unwrap(),
                &pem,
            )
            .await
            .unwrap();

        upsert_oci(
            &package_index_store,
            repository_id,
            "demo",
            "latest",
            image.id(),
            serde_json::json!({
                "name": "demo",
                "reference": "latest",
                "digest": digest,
                "media_type": "application/vnd.oci.image.manifest.v1+json",
                "size": 80,
                "artifact_id": image.id().to_string()
            }),
        )
        .await;
        let sig_tag = format!("sha256-{image_hex}.sig");
        upsert_oci(
            &package_index_store,
            repository_id,
            "demo",
            &sig_tag,
            sig_artifact.id(),
            serde_json::json!({
                "name": "demo",
                "reference": sig_tag,
                "digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "media_type": "application/vnd.oci.image.manifest.v1+json",
                "size": 20,
                "artifact_id": sig_artifact.id().to_string(),
                "subject": digest,
                "accessory": "signature"
            }),
        )
        .await;
        upsert_oci(
            &package_index_store,
            repository_id,
            "_blob",
            &payload_digest,
            blob_artifact.id(),
            serde_json::json!({"digest": payload_digest}),
        )
        .await;

        let result = use_case(repository_store, artifact_store, package_index_store)
            .with_cosign_verify(storage, admission)
            .execute(repository_id)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].signed());
        assert!(result[0].verified());
    }
}
