//! Estrategia de empaquetado para el ecosistema `PyPI`: implementa el
//! subconjunto del protocolo que `twine upload` y `pip install`
//! necesitan contra un repositorio `FerroBox`.
//!
//! Referencias:
//! - índice simple: <https://peps.python.org/pep-0503/>
//! - subida *legacy*: <https://docs.pypi.org/api/upload/>
//!
//! Este primer corte cubre **Forge** (subir sdist/wheel, índice simple,
//! descarga y yank) y lecturas en **Alloy**. Un `Mirror` de `PyPI` no
//! está implementado todavía.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::ArtifactId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::artifact_store::ArtifactStore;
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::repository_store::RepositoryStore;
use ferrobox_ports::storage::StoragePort;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{PackageSearchHit, PackagingError, PackagingStrategy, PublishOutcome};
use crate::content_hash::sha256_checksum;
use crate::storage_key::storage_key_for;

/// Estrategia de empaquetado para el ecosistema `PyPI`.
pub struct PypiPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    repository_store: Arc<dyn RepositoryStore>,
    public_base_url: String,
}

impl PypiPackagingStrategy {
    /// Construye la estrategia a partir de sus puertos y de la URL
    /// pública con la que se rellenan los enlaces del índice simple.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        repository_store: Arc<dyn RepositoryStore>,
        public_base_url: String,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
            repository_store,
            public_base_url,
        }
    }

    fn ensure_pypi_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::PyPi {
            return Err(PackagingError::EcosystemMismatch {
                expected: "pypi",
                actual: repository.ecosystem().label(),
            });
        }
        Ok(())
    }

    fn is_read_only(repository: &Repository) -> bool {
        matches!(
            repository.kind(),
            RepositoryKind::Mirror { .. } | RepositoryKind::Alloy { .. }
        )
    }

    async fn resolve_read_targets(
        &self,
        repository: &Repository,
    ) -> Result<Vec<Repository>, PackagingError> {
        match repository.kind() {
            RepositoryKind::Alloy { members } => {
                let mut targets = Vec::new();
                for member_id in members {
                    let Some(member) = self.repository_store.find_by_id(*member_id).await? else {
                        continue;
                    };
                    if matches!(member.kind(), RepositoryKind::Alloy { .. }) {
                        continue;
                    }
                    if member.ecosystem() != repository.ecosystem() {
                        continue;
                    }
                    targets.push(member);
                }
                Ok(targets)
            }
            _ => Ok(vec![repository.clone()]),
        }
    }

    fn file_url(&self, repository: &Repository, filename: &str) -> String {
        format!(
            "{}/pypi/{}/packages/{}",
            self.public_base_url.trim_end_matches('/'),
            repository.id(),
            filename
        )
    }

    async fn load_version_entry(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<VersionEntry>, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::PyPi, coordinate.name())
            .await?;
        for entry_bytes in entries {
            let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if entry.version == coordinate.version().as_str() {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    async fn index_one(
        &self,
        serving: &Repository,
        source: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        let entries = self
            .package_index_store
            .entries_for_package(source.id(), PackageEcosystem::PyPi, name)
            .await?;
        if entries.is_empty() {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        }
        simple_project_page(name.as_str(), &entries, |filename| {
            self.file_url(serving, filename)
        })
    }

    async fn search_one(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let entries = self
            .package_index_store
            .entries_for_repository(repository.id(), PackageEcosystem::PyPi)
            .await?;

        let needle = query.to_ascii_lowercase();
        let mut by_name: BTreeMap<String, Vec<VersionEntry>> = BTreeMap::new();
        for entry_bytes in entries {
            let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if !entry.name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            by_name.entry(entry.name.clone()).or_default().push(entry);
        }

        let mut hits = Vec::new();
        for (name, versions) in by_name {
            let max_version = versions
                .iter()
                .rev()
                .find(|entry| !entry.yanked)
                .or_else(|| versions.last())
                .expect("grouped versions are never empty")
                .version
                .clone();
            hits.push(PackageSearchHit { name, max_version });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }

    async fn download_stored(&self, artifact_id: ArtifactId) -> Result<Bytes, PackagingError> {
        let artifact = self
            .artifact_store
            .find_by_id(artifact_id)
            .await?
            .ok_or_else(|| PackagingError::FileNotFound(artifact_id.to_string()))?;
        let content = self.storage.get(&storage_key_for(artifact_id)).await?;
        let actual = sha256_checksum(&content);
        if actual != *artifact.checksum() {
            return Err(PackagingError::ChecksumMismatch {
                expected: artifact.checksum().to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(content)
    }
}

#[async_trait]
impl PackagingStrategy for PypiPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::PyPi
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_pypi_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }

        let parsed = parse_publish_payload(&payload)?;
        let name = PackageName::parse(normalize_pypi_name(&parsed.name))
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let version = PackageVersion::parse(parsed.version.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let coordinate = PackageCoordinate::new(PackageEcosystem::PyPi, name, version);

        let checksum = sha256_checksum(&parsed.content);
        if let Some(expected) = parsed.sha256_digest.as_ref()
            && expected.to_ascii_lowercase() != checksum.as_str()
        {
            return Err(PackagingError::InvalidPayload(format!(
                "sha256_digest mismatch: expected {expected}, got {checksum}"
            )));
        }

        let mut entry = self
            .load_version_entry(repository, &coordinate)
            .await?
            .unwrap_or(VersionEntry {
                name: coordinate.name().as_str().to_string(),
                version: coordinate.version().as_str().to_string(),
                yanked: false,
                files: Vec::new(),
            });

        if entry
            .files
            .iter()
            .any(|file| file.filename == parsed.filename)
        {
            return Err(PackagingError::AlreadyPublished(coordinate));
        }

        let artifact = Artifact::new(
            repository.id(),
            checksum.clone(),
            parsed.content.len() as u64,
        );
        self.storage
            .put(&storage_key_for(artifact.id()), parsed.content)
            .await?;
        self.artifact_store.save(&artifact).await?;

        entry.files.push(FileEntry {
            filename: parsed.filename,
            sha256: checksum.to_string(),
            artifact_id: artifact.id().to_string(),
            yanked: false,
        });

        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("a VersionEntry always serializes to valid JSON"),
        );
        self.package_index_store
            .upsert_entry(
                repository.id(),
                &coordinate,
                Some(artifact.id()),
                entry_bytes,
            )
            .await?;

        Ok(coordinate)
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_pypi_repository(repository)?;
        let normalized = PackageName::parse(normalize_pypi_name(name.as_str()))
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;

        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut documents = Vec::new();
            let mut other_error = None;
            for target in &targets {
                match self.index_one(repository, target, &normalized).await {
                    Ok(document) => documents.push(document),
                    Err(PackagingError::PackageNotFound(_)) => {}
                    Err(error) => {
                        if other_error.is_none() {
                            other_error = Some(error);
                        }
                    }
                }
            }
            if documents.is_empty() {
                return Err(other_error.unwrap_or_else(|| {
                    PackagingError::PackageNotFound(normalized.to_string())
                }));
            }
            return merge_simple_pages(normalized.as_str(), &documents);
        }

        self.index_one(repository, repository, &normalized).await
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_pypi_repository(repository)?;
        let entry = self
            .load_version_entry(repository, coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
        let file = entry
            .files
            .first()
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
        let artifact_id = parse_artifact_id(&file.artifact_id)?;
        self.download_stored(artifact_id).await
    }

    async fn download_file(
        &self,
        repository: &Repository,
        filename: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_pypi_repository(repository)?;

        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut last_error = PackagingError::FileNotFound(filename.to_string());
            for target in &targets {
                match self.download_file(target, filename).await {
                    Ok(bytes) => return Ok(bytes),
                    Err(PackagingError::FileNotFound(_) | PackagingError::PackageNotFound(_)) => {}
                    Err(error) => last_error = error,
                }
            }
            return Err(last_error);
        }

        let entries = self
            .package_index_store
            .entries_for_repository(repository.id(), PackageEcosystem::PyPi)
            .await?;
        for entry_bytes in entries {
            let entry: VersionEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if let Some(file) = entry.files.iter().find(|file| file.filename == filename) {
                let artifact_id = parse_artifact_id(&file.artifact_id)?;
                return self.download_stored(artifact_id).await;
            }
        }
        Err(PackagingError::FileNotFound(filename.to_string()))
    }

    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError> {
        Self::ensure_pypi_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }

        let Some(mut entry) = self.load_version_entry(repository, coordinate).await? else {
            return Err(PackagingError::VersionNotFound(coordinate.clone()));
        };
        if entry.yanked == yanked {
            return Ok(());
        }
        entry.yanked = yanked;
        for file in &mut entry.files {
            file.yanked = yanked;
        }
        let artifact_id = self
            .package_index_store
            .artifact_for(repository.id(), coordinate)
            .await?;
        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("a VersionEntry always serializes to valid JSON"),
        );
        self.package_index_store
            .upsert_entry(repository.id(), coordinate, artifact_id, entry_bytes)
            .await?;
        Ok(())
    }

    async fn search(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        Self::ensure_pypi_repository(repository)?;
        if limit == 0 {
            return Ok(Vec::new());
        }

        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            let targets = self.resolve_read_targets(repository).await?;
            let mut merged = Vec::new();
            let mut seen = HashSet::new();
            for target in &targets {
                for hit in self.search_one(target, query, usize::MAX).await? {
                    if seen.insert(hit.name.clone()) {
                        merged.push(hit);
                    }
                }
            }
            merged.truncate(limit);
            return Ok(merged);
        }

        self.search_one(repository, query, limit).await
    }
}

/// Normaliza un nombre de proyecto según PEP 503: minúsculas y rachas
/// de `-`, `_` o `.` sustituidas por un único `-`.
#[must_use]
pub fn normalize_pypi_name(name: &str) -> String {
    let mut normalized = String::new();
    let mut previous_separator = false;
    for character in name.chars() {
        if matches!(character, '-' | '_' | '.') {
            if !previous_separator && !normalized.is_empty() {
                normalized.push('-');
                previous_separator = true;
            }
        } else {
            for lower in character.to_lowercase() {
                normalized.push(lower);
            }
            previous_separator = false;
        }
    }
    normalized.trim_end_matches('-').to_string()
}

fn parse_artifact_id(value: &str) -> Result<ArtifactId, PackagingError> {
    Uuid::parse_str(value)
        .map(ArtifactId::from)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))
}

fn parse_publish_payload(payload: &[u8]) -> Result<ParsedPublish, PackagingError> {
    let body: PypiPublishBody = serde_json::from_slice(payload)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    if body.name.is_empty() || body.version.is_empty() || body.filename.is_empty() {
        return Err(PackagingError::InvalidPayload(
            "publish body is missing name, version or filename".to_string(),
        ));
    }
    if body.filename.contains('/') || body.filename.contains('\\') {
        return Err(PackagingError::InvalidPayload(
            "filename must not contain path separators".to_string(),
        ));
    }
    let compact: String = body
        .content
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let content = BASE64.decode(compact.as_bytes()).map_err(|err| {
        PackagingError::InvalidPayload(format!("invalid file encoding: {err}"))
    })?;
    Ok(ParsedPublish {
        name: body.name,
        version: body.version,
        filename: body.filename,
        sha256_digest: body.sha256_digest,
        content: Bytes::from(content),
    })
}

fn simple_project_page(
    name: &str,
    entries: &[Bytes],
    file_url: impl Fn(&str) -> String,
) -> Result<Bytes, PackagingError> {
    let mut links = Vec::new();
    for entry_bytes in entries {
        let entry: VersionEntry = serde_json::from_slice(entry_bytes)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        for file in entry.files {
            let yanked = entry.yanked || file.yanked;
            links.push(simple_file_link(&file_url(&file.filename), &file, yanked));
        }
    }
    if links.is_empty() {
        return Err(PackagingError::PackageNotFound(name.to_string()));
    }
    Ok(Bytes::from(simple_html(&format!("Links for {name}"), &links)))
}

fn merge_simple_pages(name: &str, documents: &[Bytes]) -> Result<Bytes, PackagingError> {
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for document in documents {
        let html = std::str::from_utf8(document)
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        for line in html.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("<a ") && seen.insert(trimmed.to_string()) {
                links.push(trimmed.to_string());
            }
        }
    }
    if links.is_empty() {
        return Err(PackagingError::PackageNotFound(name.to_string()));
    }
    Ok(Bytes::from(simple_html(&format!("Links for {name}"), &links)))
}

fn simple_file_link(url: &str, file: &FileEntry, yanked: bool) -> String {
    let yanked_attr = if yanked {
        " data-yanked=\"yanked\""
    } else {
        ""
    };
    format!(
        "<a href=\"{}#sha256={}\"{}>{}</a>",
        escape_html(url),
        escape_html(&file.sha256),
        yanked_attr,
        escape_html(&file.filename)
    )
}

fn simple_html(title: &str, links: &[String]) -> String {
    let mut body = String::from("<!DOCTYPE html>\n<html><head><title>");
    body.push_str(&escape_html(title));
    body.push_str("</title></head><body>\n<h1>");
    body.push_str(&escape_html(title));
    body.push_str("</h1>\n");
    for link in links {
        body.push_str(link);
        body.push('\n');
    }
    body.push_str("</body></html>\n");
    body
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Página raíz PEP 503 (`/simple/`) a partir de coincidencias de búsqueda.
#[must_use]
pub fn simple_root_page(hits: &[PackageSearchHit]) -> Bytes {
    let mut links = Vec::new();
    for hit in hits {
        let normalized = normalize_pypi_name(&hit.name);
        links.push(format!(
            "<a href=\"{normalized}/\">{name}</a>",
            name = escape_html(&hit.name)
        ));
    }
    Bytes::from(simple_html("Simple Index", &links))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VersionEntry {
    name: String,
    version: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FileEntry {
    filename: String,
    sha256: String,
    artifact_id: String,
    #[serde(default)]
    yanked: bool,
}

#[derive(Debug, Deserialize)]
struct PypiPublishBody {
    name: String,
    version: String,
    filename: String,
    #[serde(default)]
    sha256_digest: Option<String>,
    content: String,
}

struct ParsedPublish {
    name: String,
    version: String,
    filename: String,
    sha256_digest: Option<String>,
    content: Bytes,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::{PackageEcosystem, PackageName};
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryStorage,
    };

    fn strategy() -> PypiPackagingStrategy {
        PypiPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        )
    }

    fn pypi_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::PyPi,
        )
        .unwrap()
    }

    fn publish_body(name: &str, version: &str, filename: &str, content: &[u8]) -> Bytes {
        Bytes::from(
            serde_json::json!({
                "name": name,
                "version": version,
                "filename": filename,
                "content": BASE64.encode(content)
            })
            .to_string(),
        )
    }

    #[test]
    fn normalize_pypi_name_collapses_separators() {
        assert_eq!(normalize_pypi_name("Demo.Ferrobox_Pypi"), "demo-ferrobox-pypi");
        assert_eq!(normalize_pypi_name("friendly-bard"), "friendly-bard");
    }

    #[tokio::test]
    async fn publish_then_index_and_download_file() {
        let strategy = strategy();
        let repository = pypi_forge("pypi-local");
        let content = b"sdist-bytes";
        let filename = "demo_ferrobox_pypi-1.0.0.tar.gz";

        let coordinate = strategy
            .publish(
                &repository,
                publish_body("Demo_Ferrobox.Pypi", "1.0.0", filename, content),
            )
            .await
            .unwrap();
        assert_eq!(coordinate.name().as_str(), "demo-ferrobox-pypi");

        let page = strategy
            .index(&repository, coordinate.name())
            .await
            .unwrap();
        let html = std::str::from_utf8(&page).unwrap();
        assert!(html.contains(filename));
        assert!(html.contains(&format!(
            "http://127.0.0.1:3000/pypi/{}/packages/{filename}",
            repository.id()
        )));
        assert!(html.contains("#sha256="));

        let downloaded = strategy
            .download_file(&repository, filename)
            .await
            .unwrap();
        assert_eq!(downloaded.as_ref(), content);
    }

    #[tokio::test]
    async fn accepts_a_second_file_for_the_same_version() {
        let strategy = strategy();
        let repository = pypi_forge("pypi-local");
        strategy
            .publish(
                &repository,
                publish_body(
                    "demo-pypi",
                    "1.0.0",
                    "demo_pypi-1.0.0.tar.gz",
                    b"sdist",
                ),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &repository,
                publish_body(
                    "demo-pypi",
                    "1.0.0",
                    "demo_pypi-1.0.0-py3-none-any.whl",
                    b"wheel",
                ),
            )
            .await
            .unwrap();

        let page = strategy
            .index(&repository, &PackageName::parse("demo-pypi").unwrap())
            .await
            .unwrap();
        let html = std::str::from_utf8(&page).unwrap();
        assert!(html.contains("demo_pypi-1.0.0.tar.gz"));
        assert!(html.contains("demo_pypi-1.0.0-py3-none-any.whl"));
    }

    #[tokio::test]
    async fn rejects_reuploading_the_same_filename() {
        let strategy = strategy();
        let repository = pypi_forge("pypi-local");
        let body = publish_body("demo-pypi", "1.0.0", "demo_pypi-1.0.0.tar.gz", b"sdist");
        strategy.publish(&repository, body.clone()).await.unwrap();
        let err = strategy.publish(&repository, body).await.unwrap_err();
        assert!(matches!(err, PackagingError::AlreadyPublished(_)));
    }

    #[tokio::test]
    async fn yank_adds_data_yanked_to_simple_links() {
        let strategy = strategy();
        let repository = pypi_forge("pypi-local");
        let coordinate = strategy
            .publish(
                &repository,
                publish_body("demo-pypi", "1.0.0", "demo_pypi-1.0.0.tar.gz", b"sdist"),
            )
            .await
            .unwrap();
        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();
        let yanked_page = strategy
            .index(&repository, coordinate.name())
            .await
            .unwrap();
        let html = std::str::from_utf8(&yanked_page).unwrap();
        assert!(html.contains("data-yanked"));
        strategy
            .set_yanked(&repository, &coordinate, false)
            .await
            .unwrap();
        let restored_page = strategy
            .index(&repository, coordinate.name())
            .await
            .unwrap();
        let html = std::str::from_utf8(&restored_page).unwrap();
        assert!(!html.contains("data-yanked"));
    }

    #[tokio::test]
    async fn rejects_publishing_to_an_alloy() {
        let first = pypi_forge("pypi-a");
        let alloy = Repository::new(
            RepositoryName::parse("pypi-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![first.id()],
            },
            PackageEcosystem::PyPi,
        )
        .unwrap();
        let err = strategy()
            .publish(
                &alloy,
                publish_body("demo-pypi", "1.0.0", "demo_pypi-1.0.0.tar.gz", b"sdist"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PackagingError::ReadOnlyRepository));
    }

    #[tokio::test]
    async fn alloy_index_and_download_see_forge_member_files() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let strategy = PypiPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            repository_store.clone(),
            "http://127.0.0.1:3000".to_string(),
        );
        let forge = pypi_forge("pypi-local");
        let alloy = Repository::new(
            RepositoryName::parse("pypi-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge.id()],
            },
            PackageEcosystem::PyPi,
        )
        .unwrap();
        repository_store.save(&forge).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        strategy
            .publish(
                &forge,
                publish_body("demo-pypi", "1.0.0", "demo_pypi-1.0.0.tar.gz", b"from-forge"),
            )
            .await
            .unwrap();

        let alloy_page = strategy
            .index(&alloy, &PackageName::parse("demo-pypi").unwrap())
            .await
            .unwrap();
        let html = std::str::from_utf8(&alloy_page).unwrap();
        assert!(html.contains(&alloy.id().to_string()));

        let downloaded = strategy
            .download_file(&alloy, "demo_pypi-1.0.0.tar.gz")
            .await
            .unwrap();
        assert_eq!(downloaded.as_ref(), b"from-forge");
    }

    #[tokio::test]
    async fn search_matches_normalized_names() {
        let strategy = strategy();
        let repository = pypi_forge("pypi-local");
        strategy
            .publish(
                &repository,
                publish_body("demo-pypi", "1.0.0", "demo_pypi-1.0.0.tar.gz", b"sdist"),
            )
            .await
            .unwrap();
        let hits = strategy.search(&repository, "demo", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "demo-pypi");
    }

    #[tokio::test]
    async fn simple_root_lists_normalized_project_hrefs() {
        let strategy = strategy();
        let repository = pypi_forge("pypi-local");
        strategy
            .publish(
                &repository,
                publish_body(
                    "Demo_Pkg",
                    "1.0.0",
                    "demo_pkg-1.0.0.tar.gz",
                    b"sdist",
                ),
            )
            .await
            .unwrap();
        let hits = strategy.search(&repository, "", 100).await.unwrap();
        let root = simple_root_page(&hits);
        let html = std::str::from_utf8(&root).unwrap();
        assert!(html.contains("href=\"demo-pkg/\""));
        assert!(html.contains(">demo-pkg</a>"));
    }
}
