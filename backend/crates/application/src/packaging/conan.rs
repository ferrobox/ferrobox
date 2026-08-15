//! Estrategia de empaquetado para Conan: el subconjunto de la API v2
//! (revisiones) que `conan upload` y `conan install` necesitan contra un
//! repositorio `FerroBox`.
//!
//! Referencia: rutas de `conan/internal/rest/rest_routes.py` (Conan 2).
//!
//! Cubre **Forge** (subida de receta y binarios, latest, listado de
//! ficheros, búsqueda y yank) y lecturas en **Alloy**. Un `Mirror`
//! Conan queda para un corte posterior.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
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

/// Estrategia de empaquetado para el ecosistema Conan.
pub struct ConanPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
    repository_store: Arc<dyn RepositoryStore>,
}

impl ConanPackagingStrategy {
    /// Construye la estrategia a partir de sus puertos.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
        repository_store: Arc<dyn RepositoryStore>,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
            repository_store,
        }
    }

    fn ensure_conan_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Conan {
            return Err(PackagingError::EcosystemMismatch {
                expected: "conan",
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

    async fn load_recipe(
        &self,
        repository: &Repository,
        name: &str,
        version: &str,
        user: &str,
        channel: &str,
    ) -> Result<Option<RecipeEntry>, PackagingError> {
        let coordinate = recipe_coordinate(name, version, user, channel)?;
        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Conan, coordinate.name())
            .await?;
        for entry_bytes in entries {
            let entry: RecipeEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if entry.version == version && entry.user == user && entry.channel == channel {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    async fn save_recipe(
        &self,
        repository: &Repository,
        entry: &RecipeEntry,
        artifact_id: ArtifactId,
    ) -> Result<(), PackagingError> {
        let coordinate = recipe_coordinate(
            &entry.name,
            &entry.version,
            &entry.user,
            &entry.channel,
        )?;
        let entry_bytes = Bytes::from(
            serde_json::to_vec(entry).expect("a RecipeEntry always serializes to valid JSON"),
        );
        self.package_index_store
            .upsert_entry(repository.id(), &coordinate, Some(artifact_id), entry_bytes)
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn metadata_one(
        &self,
        repository: &Repository,
        resource: &ConanResource,
    ) -> Result<Bytes, PackagingError> {
        match resource {
            ConanResource::RecipeLatest {
                name,
                version,
                user,
                channel,
            } => {
                let entry = self
                    .load_recipe(repository, name, version, user, channel)
                    .await?
                    .ok_or_else(|| PackagingError::PackageNotFound(name.clone()))?;
                let revision = entry
                    .latest_revision()
                    .ok_or_else(|| PackagingError::PackageNotFound(name.clone()))?;
                Ok(json_bytes(&RevisionStamp {
                    revision: revision.revision.clone(),
                    time: revision.time.clone(),
                }))
            }
            ConanResource::RecipeRevisions {
                name,
                version,
                user,
                channel,
            } => {
                let entry = self
                    .load_recipe(repository, name, version, user, channel)
                    .await?
                    .ok_or_else(|| PackagingError::PackageNotFound(name.clone()))?;
                let revisions = entry
                    .revisions
                    .iter()
                    .rev()
                    .map(|revision| RevisionStamp {
                        revision: revision.revision.clone(),
                        time: revision.time.clone(),
                    })
                    .collect();
                Ok(json_bytes(&RevisionsDocument { revisions }))
            }
            ConanResource::RecipeFiles {
                name,
                version,
                user,
                channel,
                rrev,
            } => {
                let entry = self
                    .load_recipe(repository, name, version, user, channel)
                    .await?
                    .ok_or_else(|| PackagingError::PackageNotFound(name.clone()))?;
                let revision = entry.revision(rrev).ok_or_else(|| {
                    PackagingError::PackageNotFound(rrev.clone())
                })?;
                Ok(json_bytes(&FilesDocument::from_stored(&revision.files)))
            }
            ConanResource::PackageLatest {
                name,
                version,
                user,
                channel,
                rrev,
                pkgid,
            } => {
                let package = self
                    .load_package(repository, name, version, user, channel, rrev, pkgid)
                    .await?;
                let revision = package
                    .revisions
                    .last()
                    .ok_or_else(|| PackagingError::PackageNotFound(pkgid.clone()))?;
                Ok(json_bytes(&RevisionStamp {
                    revision: revision.revision.clone(),
                    time: revision.time.clone(),
                }))
            }
            ConanResource::PackageRevisions {
                name,
                version,
                user,
                channel,
                rrev,
                pkgid,
            } => {
                let package = self
                    .load_package(repository, name, version, user, channel, rrev, pkgid)
                    .await?;
                let revisions = package
                    .revisions
                    .iter()
                    .rev()
                    .map(|revision| RevisionStamp {
                        revision: revision.revision.clone(),
                        time: revision.time.clone(),
                    })
                    .collect();
                Ok(json_bytes(&RevisionsDocument { revisions }))
            }
            ConanResource::PackageFiles {
                name,
                version,
                user,
                channel,
                rrev,
                pkgid,
                prev,
            } => {
                let package = self
                    .load_package(repository, name, version, user, channel, rrev, pkgid)
                    .await?;
                let revision = package
                    .revisions
                    .iter()
                    .find(|candidate| candidate.revision == *prev)
                    .ok_or_else(|| PackagingError::PackageNotFound(prev.clone()))?;
                Ok(json_bytes(&FilesDocument::from_stored(&revision.files)))
            }
            ConanResource::RecipeSearch {
                name,
                version,
                user,
                channel,
            }
            | ConanResource::RecipeRevisionSearch {
                name,
                version,
                user,
                channel,
                ..
            } => {
                let rrev = match resource {
                    ConanResource::RecipeRevisionSearch { rrev, .. } => Some(rrev.as_str()),
                    _ => None,
                };
                let entry = self
                    .load_recipe(repository, name, version, user, channel)
                    .await?
                    .ok_or_else(|| PackagingError::PackageNotFound(name.clone()))?;
                Ok(json_bytes(&package_search_document(&entry, rrev)))
            }
            ConanResource::RecipeFile { filename, .. }
            | ConanResource::PackageFile { filename, .. } => {
                Err(PackagingError::FileNotFound(filename.clone()))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn load_package(
        &self,
        repository: &Repository,
        name: &str,
        version: &str,
        user: &str,
        channel: &str,
        rrev: &str,
        pkgid: &str,
    ) -> Result<PackageRecord, PackagingError> {
        let entry = self
            .load_recipe(repository, name, version, user, channel)
            .await?
            .ok_or_else(|| PackagingError::PackageNotFound(name.to_string()))?;
        let revision = entry
            .revision(rrev)
            .ok_or_else(|| PackagingError::PackageNotFound(rrev.to_string()))?;
        revision
            .packages
            .iter()
            .find(|package| package.package_id == pkgid)
            .cloned()
            .ok_or_else(|| PackagingError::PackageNotFound(pkgid.to_string()))
    }

    async fn get_file_one(
        &self,
        repository: &Repository,
        resource: &ConanResource,
    ) -> Result<Bytes, PackagingError> {
        let (artifact_id, filename) = match resource {
            ConanResource::RecipeFile {
                name,
                version,
                user,
                channel,
                rrev,
                filename,
            } => {
                let entry = self
                    .load_recipe(repository, name, version, user, channel)
                    .await?
                    .ok_or_else(|| PackagingError::FileNotFound(filename.clone()))?;
                let revision = entry
                    .revision(rrev)
                    .ok_or_else(|| PackagingError::FileNotFound(filename.clone()))?;
                let stored = revision
                    .files
                    .iter()
                    .find(|file| file.filename == *filename)
                    .ok_or_else(|| PackagingError::FileNotFound(filename.clone()))?;
                let artifact_id = stored.artifact_id.clone();
                (artifact_id, filename.clone())
            }
            ConanResource::PackageFile {
                name,
                version,
                user,
                channel,
                rrev,
                pkgid,
                prev,
                filename,
            } => {
                let entry = self
                    .load_recipe(repository, name, version, user, channel)
                    .await?
                    .ok_or_else(|| PackagingError::FileNotFound(filename.clone()))?;
                let package = entry
                    .revision(rrev)
                    .and_then(|revision| {
                        revision
                            .packages
                            .iter()
                            .find(|candidate| candidate.package_id == *pkgid)
                    })
                    .ok_or_else(|| PackagingError::FileNotFound(filename.clone()))?;
                let stored = package
                    .revisions
                    .iter()
                    .find(|candidate| candidate.revision == *prev)
                    .and_then(|revision| {
                        revision
                            .files
                            .iter()
                            .find(|file| file.filename == *filename)
                    })
                    .ok_or_else(|| PackagingError::FileNotFound(filename.clone()))?;
                let artifact_id = stored.artifact_id.clone();
                (artifact_id, filename.clone())
            }
            _ => {
                return Err(PackagingError::FileNotFound(resource.path_hint()));
            }
        };
        let artifact_id = ArtifactId::from(
            Uuid::parse_str(&artifact_id)
                .map_err(|_| PackagingError::FileNotFound(filename.clone()))?,
        );
        Ok(self.storage.get(&storage_key_for(artifact_id)).await?)
    }

    #[allow(clippy::too_many_arguments)]
    async fn put_recipe_file(
        &self,
        repository: &Repository,
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
        filename: String,
        artifact_id: String,
        time: String,
        saved_artifact: ArtifactId,
    ) -> Result<(), PackagingError> {
        let mut entry = self
            .load_recipe(repository, &name, &version, &user, &channel)
            .await?
            .unwrap_or(RecipeEntry {
                name: name.clone(),
                version,
                user,
                channel,
                yanked: false,
                files: Vec::new(),
                revisions: Vec::new(),
            });
        if entry.revision(&rrev).is_none() {
            entry.revisions.push(RecipeRevision {
                revision: rrev.clone(),
                time,
                files: Vec::new(),
                packages: Vec::new(),
            });
        }
        let revision = entry.revision_mut(&rrev).ok_or_else(|| {
            PackagingError::InvalidPayload("failed to locate recipe revision".to_string())
        })?;
        if let Some(existing) = revision
            .files
            .iter_mut()
            .find(|file| file.filename == filename)
        {
            existing.artifact_id.clone_from(&artifact_id);
        } else {
            revision.files.push(StoredFile {
                filename,
                artifact_id,
            });
        }
        entry.rebuild_files();
        self.save_recipe(repository, &entry, saved_artifact).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn put_package_file(
        &self,
        repository: &Repository,
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
        pkgid: String,
        prev: String,
        filename: String,
        artifact_id: String,
        time: String,
        saved_artifact: ArtifactId,
    ) -> Result<(), PackagingError> {
        let mut entry = self
            .load_recipe(repository, &name, &version, &user, &channel)
            .await?
            .ok_or_else(|| PackagingError::PackageNotFound(name))?;
        let revision = entry
            .revision_mut(&rrev)
            .ok_or_else(|| PackagingError::PackageNotFound(rrev))?;
        if !revision
            .packages
            .iter()
            .any(|package| package.package_id == pkgid)
        {
            revision.packages.push(PackageRecord {
                package_id: pkgid.clone(),
                revisions: Vec::new(),
            });
        }
        let package = revision
            .packages
            .iter_mut()
            .find(|package| package.package_id == pkgid)
            .ok_or_else(|| PackagingError::PackageNotFound(pkgid.clone()))?;
        if !package
            .revisions
            .iter()
            .any(|candidate| candidate.revision == prev)
        {
            package.revisions.push(PackageRevision {
                revision: prev.clone(),
                time,
                files: Vec::new(),
            });
        }
        let package_revision = package
            .revisions
            .iter_mut()
            .find(|candidate| candidate.revision == prev)
            .ok_or_else(|| PackagingError::PackageNotFound(prev.clone()))?;
        if let Some(existing) = package_revision
            .files
            .iter_mut()
            .find(|file| file.filename == filename)
        {
            existing.artifact_id.clone_from(&artifact_id);
        } else {
            package_revision.files.push(StoredFile {
                filename,
                artifact_id,
            });
        }
        entry.rebuild_files();
        self.save_recipe(repository, &entry, saved_artifact).await
    }
}

fn recipe_coordinate(
    name: &str,
    version: &str,
    user: &str,
    channel: &str,
) -> Result<PackageCoordinate, PackagingError> {
    let package_name = PackageName::parse(name)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    let package_version = PackageVersion::parse(format!("{version}@{user}:{channel}"))
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    Ok(PackageCoordinate::new(
        PackageEcosystem::Conan,
        package_name,
        package_version,
    ))
}

fn json_bytes<T: Serialize>(value: &T) -> Bytes {
    Bytes::from(serde_json::to_vec(value).expect("conan protocol JSON always serializes"))
}

fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn package_search_document(entry: &RecipeEntry, rrev: Option<&str>) -> BTreeMap<String, serde_json::Value> {
    let revisions = match rrev {
        Some(revision) => entry
            .revisions
            .iter()
            .filter(|candidate| candidate.revision == revision)
            .collect::<Vec<_>>(),
        None => entry.latest_revision().into_iter().collect(),
    };
    let mut packages = BTreeMap::new();
    for revision in revisions {
        for package in &revision.packages {
            packages.insert(package.package_id.clone(), serde_json::json!({}));
        }
    }
    packages
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConanResource {
    RecipeLatest {
        name: String,
        version: String,
        user: String,
        channel: String,
    },
    RecipeRevisions {
        name: String,
        version: String,
        user: String,
        channel: String,
    },
    RecipeFiles {
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
    },
    RecipeFile {
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
        filename: String,
    },
    PackageLatest {
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
        pkgid: String,
    },
    PackageRevisions {
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
        pkgid: String,
    },
    PackageFiles {
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
        pkgid: String,
        prev: String,
    },
    PackageFile {
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
        pkgid: String,
        prev: String,
        filename: String,
    },
    RecipeSearch {
        name: String,
        version: String,
        user: String,
        channel: String,
    },
    RecipeRevisionSearch {
        name: String,
        version: String,
        user: String,
        channel: String,
        rrev: String,
    },
}

impl ConanResource {
    fn path_hint(&self) -> String {
        match self {
            Self::RecipeFile { filename, .. } | Self::PackageFile { filename, .. } => {
                filename.clone()
            }
            Self::RecipeLatest { name, .. }
            | Self::RecipeRevisions { name, .. }
            | Self::RecipeFiles { name, .. }
            | Self::PackageLatest { name, .. }
            | Self::PackageRevisions { name, .. }
            | Self::PackageFiles { name, .. }
            | Self::RecipeSearch { name, .. }
            | Self::RecipeRevisionSearch { name, .. } => name.clone(),
        }
    }

    fn is_file(&self) -> bool {
        matches!(self, Self::RecipeFile { .. } | Self::PackageFile { .. })
    }
}

fn parse_conan_path(path: &str) -> Result<ConanResource, PackagingError> {
    let trimmed = path.trim_matches('/');
    let parts: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() < 5 {
        return Err(PackagingError::InvalidPayload(format!(
            "invalid Conan path '{path}'"
        )));
    }
    let name = parts[0].to_string();
    let version = parts[1].to_string();
    let user = parts[2].to_string();
    let channel = parts[3].to_string();
    let rest = &parts[4..];
    match rest {
        ["latest"] => Ok(ConanResource::RecipeLatest {
            name,
            version,
            user,
            channel,
        }),
        ["revisions"] => Ok(ConanResource::RecipeRevisions {
            name,
            version,
            user,
            channel,
        }),
        ["search"] => Ok(ConanResource::RecipeSearch {
            name,
            version,
            user,
            channel,
        }),
        ["revisions", rrev, "files"] => Ok(ConanResource::RecipeFiles {
            name,
            version,
            user,
            channel,
            rrev: (*rrev).to_string(),
        }),
        ["revisions", rrev, "search"] => Ok(ConanResource::RecipeRevisionSearch {
            name,
            version,
            user,
            channel,
            rrev: (*rrev).to_string(),
        }),
        ["revisions", rrev, "files", filename @ ..] if !filename.is_empty() => {
            Ok(ConanResource::RecipeFile {
                name,
                version,
                user,
                channel,
                rrev: (*rrev).to_string(),
                filename: filename.join("/"),
            })
        }
        ["revisions", rrev, "packages", pkgid, "latest"] => Ok(ConanResource::PackageLatest {
            name,
            version,
            user,
            channel,
            rrev: (*rrev).to_string(),
            pkgid: (*pkgid).to_string(),
        }),
        ["revisions", rrev, "packages", pkgid, "revisions"] => Ok(ConanResource::PackageRevisions {
            name,
            version,
            user,
            channel,
            rrev: (*rrev).to_string(),
            pkgid: (*pkgid).to_string(),
        }),
        ["revisions", rrev, "packages", pkgid, "revisions", prev, "files"] => {
            Ok(ConanResource::PackageFiles {
                name,
                version,
                user,
                channel,
                rrev: (*rrev).to_string(),
                pkgid: (*pkgid).to_string(),
                prev: (*prev).to_string(),
            })
        }
        ["revisions", rrev, "packages", pkgid, "revisions", prev, "files", filename @ ..]
            if !filename.is_empty() =>
        {
            Ok(ConanResource::PackageFile {
                name,
                version,
                user,
                channel,
                rrev: (*rrev).to_string(),
                pkgid: (*pkgid).to_string(),
                prev: (*prev).to_string(),
                filename: filename.join("/"),
            })
        }
        _ => Err(PackagingError::InvalidPayload(format!(
            "invalid Conan path '{path}'"
        ))),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecipeEntry {
    name: String,
    version: String,
    user: String,
    channel: String,
    yanked: bool,
    files: Vec<IndexFile>,
    revisions: Vec<RecipeRevision>,
}

impl RecipeEntry {
    fn latest_revision(&self) -> Option<&RecipeRevision> {
        self.revisions.last()
    }

    fn revision(&self, rrev: &str) -> Option<&RecipeRevision> {
        self.revisions
            .iter()
            .find(|revision| revision.revision == rrev)
    }

    fn revision_mut(&mut self, rrev: &str) -> Option<&mut RecipeRevision> {
        self.revisions
            .iter_mut()
            .find(|revision| revision.revision == rrev)
    }

    fn rebuild_files(&mut self) {
        let mut files = Vec::new();
        for revision in &self.revisions {
            for file in &revision.files {
                files.push(IndexFile {
                    filename: file.filename.clone(),
                    artifact_id: file.artifact_id.clone(),
                    yanked: self.yanked,
                });
            }
            for package in &revision.packages {
                for package_revision in &package.revisions {
                    for file in &package_revision.files {
                        files.push(IndexFile {
                            filename: file.filename.clone(),
                            artifact_id: file.artifact_id.clone(),
                            yanked: self.yanked,
                        });
                    }
                }
            }
        }
        self.files = files;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexFile {
    filename: String,
    artifact_id: String,
    #[serde(default)]
    yanked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecipeRevision {
    revision: String,
    time: String,
    files: Vec<StoredFile>,
    packages: Vec<PackageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackageRecord {
    package_id: String,
    revisions: Vec<PackageRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackageRevision {
    revision: String,
    time: String,
    files: Vec<StoredFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredFile {
    filename: String,
    artifact_id: String,
}

#[derive(Serialize)]
struct RevisionStamp {
    revision: String,
    time: String,
}

#[derive(Serialize)]
struct RevisionsDocument {
    revisions: Vec<RevisionStamp>,
}

#[derive(Serialize)]
struct FilesDocument {
    files: BTreeMap<String, serde_json::Value>,
}

impl FilesDocument {
    fn from_stored(files: &[StoredFile]) -> Self {
        let mut map = BTreeMap::new();
        for file in files {
            map.insert(file.filename.clone(), serde_json::json!({}));
        }
        Self { files: map }
    }
}

#[async_trait]
impl PackagingStrategy for ConanPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Conan
    }

    async fn publish(
        &self,
        repository: &Repository,
        _payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_conan_repository(repository)?;
        Err(PackagingError::InvalidPayload(
            "publish Conan packages with conan upload against /conan/<uuid>/v2/".to_string(),
        ))
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_conan_repository(repository)?;
        let mut documents = Vec::new();
        for target in self.resolve_read_targets(repository).await? {
            let entries = self
                .package_index_store
                .entries_for_package(target.id(), PackageEcosystem::Conan, name)
                .await?;
            documents.extend(entries);
        }
        if documents.is_empty() {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        }
        let mut joined = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            if index > 0 {
                joined.push(b'\n');
            }
            joined.extend_from_slice(document);
        }
        Ok(Bytes::from(joined))
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_conan_repository(repository)?;
        for target in self.resolve_read_targets(repository).await? {
            let entries = self
                .package_index_store
                .entries_for_package(target.id(), PackageEcosystem::Conan, coordinate.name())
                .await?;
            for entry_bytes in entries {
                let entry: RecipeEntry = serde_json::from_slice(&entry_bytes)
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
                if format!("{}@{}:{}", entry.version, entry.user, entry.channel)
                    != coordinate.version().as_str()
                {
                    continue;
                }
                let Some(file) = entry.files.first() else {
                    continue;
                };
                let artifact_id = ArtifactId::from(
                    Uuid::parse_str(&file.artifact_id)
                        .map_err(|_| PackagingError::VersionNotFound(coordinate.clone()))?,
                );
                return Ok(self.storage.get(&storage_key_for(artifact_id)).await?);
            }
        }
        Err(PackagingError::VersionNotFound(coordinate.clone()))
    }

    async fn put_protocol_file(
        &self,
        repository: &Repository,
        path: &str,
        body: Bytes,
    ) -> Result<(), PackagingError> {
        Self::ensure_conan_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let resource = parse_conan_path(path)?;
        let checksum = sha256_checksum(&body);
        let artifact = Artifact::new(repository.id(), checksum, body.len() as u64);
        self.storage
            .put(&storage_key_for(artifact.id()), body)
            .await?;
        self.artifact_store.save(&artifact).await?;
        let artifact_id = artifact.id().to_string();
        let time = now_stamp();

        match resource {
            ConanResource::RecipeFile {
                name,
                version,
                user,
                channel,
                rrev,
                filename,
            } => {
                self.put_recipe_file(
                    repository,
                    name,
                    version,
                    user,
                    channel,
                    rrev,
                    filename,
                    artifact_id,
                    time,
                    artifact.id(),
                )
                .await
            }
            ConanResource::PackageFile {
                name,
                version,
                user,
                channel,
                rrev,
                pkgid,
                prev,
                filename,
            } => {
                self.put_package_file(
                    repository,
                    name,
                    version,
                    user,
                    channel,
                    rrev,
                    pkgid,
                    prev,
                    filename,
                    artifact_id,
                    time,
                    artifact.id(),
                )
                .await
            }
            _ => Err(PackagingError::InvalidPayload(
                "Conan uploads must target a files/... path".to_string(),
            )),
        }
    }

    async fn get_protocol_file(
        &self,
        repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_conan_repository(repository)?;
        let resource = parse_conan_path(path)?;
        if !resource.is_file() {
            return Err(PackagingError::FileNotFound(path.to_string()));
        }
        let mut other_error = None;
        for target in self.resolve_read_targets(repository).await? {
            match self.get_file_one(&target, &resource).await {
                Ok(body) => return Ok(body),
                Err(PackagingError::FileNotFound(_) | PackagingError::PackageNotFound(_)) => {}
                Err(error) => {
                    if other_error.is_none() {
                        other_error = Some(error);
                    }
                }
            }
        }
        Err(other_error.unwrap_or_else(|| PackagingError::FileNotFound(path.to_string())))
    }

    async fn protocol_metadata(
        &self,
        repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_conan_repository(repository)?;
        let resource = parse_conan_path(path)?;
        if resource.is_file() {
            return Err(PackagingError::PackageNotFound(path.to_string()));
        }
        let mut other_error = None;
        for target in self.resolve_read_targets(repository).await? {
            match self.metadata_one(&target, &resource).await {
                Ok(body) => return Ok(body),
                Err(PackagingError::PackageNotFound(_) | PackagingError::VersionNotFound(_)) => {}
                Err(error) => {
                    if other_error.is_none() {
                        other_error = Some(error);
                    }
                }
            }
        }
        Err(other_error.unwrap_or_else(|| PackagingError::PackageNotFound(path.to_string())))
    }

    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError> {
        Self::ensure_conan_repository(repository)?;
        if Self::is_read_only(repository) {
            return Err(PackagingError::ReadOnlyRepository);
        }
        let entries = self
            .package_index_store
            .entries_for_package(
                repository.id(),
                PackageEcosystem::Conan,
                coordinate.name(),
            )
            .await?;
        for entry_bytes in entries {
            let mut entry: RecipeEntry = serde_json::from_slice(&entry_bytes)
                .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
            if format!("{}@{}:{}", entry.version, entry.user, entry.channel)
                != coordinate.version().as_str()
            {
                continue;
            }
            if entry.yanked == yanked {
                return Ok(());
            }
            entry.yanked = yanked;
            entry.rebuild_files();
            let artifact_id = entry
                .files
                .first()
                .and_then(|file| Uuid::parse_str(&file.artifact_id).ok())
                .map(ArtifactId::from)
                .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;
            return self.save_recipe(repository, &entry, artifact_id).await;
        }
        Err(PackagingError::VersionNotFound(coordinate.clone()))
    }

    async fn search(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError> {
        Self::ensure_conan_repository(repository)?;
        let needle = query.trim().trim_matches('*').to_ascii_lowercase();
        let mut hits = Vec::new();
        for target in self.resolve_read_targets(repository).await? {
            let entries = self
                .package_index_store
                .entries_for_repository(target.id(), PackageEcosystem::Conan)
                .await?;
            for entry_bytes in entries {
                let entry: RecipeEntry = serde_json::from_slice(&entry_bytes)
                    .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
                if entry.yanked {
                    continue;
                }
                let reference = format!(
                    "{}/{}@{}/{}",
                    entry.name, entry.version, entry.user, entry.channel
                );
                if !needle.is_empty() && !reference.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                if hits
                    .iter()
                    .any(|hit: &PackageSearchHit| hit.name == reference)
                {
                    continue;
                }
                hits.push(PackageSearchHit {
                    name: reference,
                    max_version: entry.version,
                });
                if hits.len() >= limit {
                    return Ok(hits);
                }
            }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryStorage,
    };

    fn strategy() -> ConanPackagingStrategy {
        ConanPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        )
    }

    fn conan_forge(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Conan,
        )
        .unwrap()
    }

    #[test]
    fn parse_recipe_and_package_file_paths() {
        assert_eq!(
            parse_conan_path("hello/0.1/_/_/revisions/abc/files/conanfile.py").unwrap(),
            ConanResource::RecipeFile {
                name: "hello".into(),
                version: "0.1".into(),
                user: "_".into(),
                channel: "_".into(),
                rrev: "abc".into(),
                filename: "conanfile.py".into(),
            }
        );
        assert_eq!(
            parse_conan_path(
                "hello/0.1/_/_/revisions/abc/packages/pkg/revisions/def/files/metadata/sign"
            )
            .unwrap(),
            ConanResource::PackageFile {
                name: "hello".into(),
                version: "0.1".into(),
                user: "_".into(),
                channel: "_".into(),
                rrev: "abc".into(),
                pkgid: "pkg".into(),
                prev: "def".into(),
                filename: "metadata/sign".into(),
            }
        );
        assert!(matches!(
            parse_conan_path("hello/0.1/_/_/latest").unwrap(),
            ConanResource::RecipeLatest { .. }
        ));
    }

    #[tokio::test]
    async fn upload_recipe_then_read_latest_and_file() {
        let strategy = strategy();
        let repository = conan_forge("conan-local");
        strategy
            .put_protocol_file(
                &repository,
                "hello/0.1/_/_/revisions/rrev1/files/conanfile.py",
                Bytes::from_static(b"from conan import ConanFile\n"),
            )
            .await
            .unwrap();
        strategy
            .put_protocol_file(
                &repository,
                "hello/0.1/_/_/revisions/rrev1/files/conanmanifest.txt",
                Bytes::from_static(b"0\nconanfile.py: abc\n"),
            )
            .await
            .unwrap();

        let latest: serde_json::Value = serde_json::from_slice(
            &strategy
                .protocol_metadata(&repository, "hello/0.1/_/_/latest")
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(latest["revision"], "rrev1");

        let files: serde_json::Value = serde_json::from_slice(
            &strategy
                .protocol_metadata(&repository, "hello/0.1/_/_/revisions/rrev1/files")
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(files["files"].get("conanfile.py").is_some());
        assert!(files["files"].get("conanmanifest.txt").is_some());

        let body = strategy
            .get_protocol_file(
                &repository,
                "hello/0.1/_/_/revisions/rrev1/files/conanfile.py",
            )
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"from conan import ConanFile\n");
    }

    #[tokio::test]
    async fn upload_package_binary_and_search() {
        let strategy = strategy();
        let repository = conan_forge("conan-pkgs");
        strategy
            .put_protocol_file(
                &repository,
                "hello/0.1/_/_/revisions/rrev1/files/conanfile.py",
                Bytes::from_static(b"recipe"),
            )
            .await
            .unwrap();
        strategy
            .put_protocol_file(
                &repository,
                "hello/0.1/_/_/revisions/rrev1/packages/pkgid/revisions/prev1/files/conan_package.tgz",
                Bytes::from_static(b"binary"),
            )
            .await
            .unwrap();

        let latest: serde_json::Value = serde_json::from_slice(
            &strategy
                .protocol_metadata(
                    &repository,
                    "hello/0.1/_/_/revisions/rrev1/packages/pkgid/latest",
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(latest["revision"], "prev1");

        let hits = strategy.search(&repository, "hello", 20).await.unwrap();
        assert_eq!(hits[0].name, "hello/0.1@_/_");
    }

    #[tokio::test]
    async fn yank_hides_recipe_from_search() {
        let strategy = strategy();
        let repository = conan_forge("conan-yank");
        strategy
            .put_protocol_file(
                &repository,
                "hello/0.1/_/_/revisions/rrev1/files/conanfile.py",
                Bytes::from_static(b"recipe"),
            )
            .await
            .unwrap();
        let coordinate = recipe_coordinate("hello", "0.1", "_", "_").unwrap();
        strategy
            .set_yanked(&repository, &coordinate, true)
            .await
            .unwrap();
        let hits = strategy.search(&repository, "*", 20).await.unwrap();
        assert!(hits.is_empty());
        strategy
            .set_yanked(&repository, &coordinate, false)
            .await
            .unwrap();
        let hits = strategy.search(&repository, "*", 20).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn alloy_reads_member_recipe() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let strategy = ConanPackagingStrategy::new(
            artifact_store,
            package_index_store,
            storage,
            repository_store.clone(),
        );
        let forge = conan_forge("conan-member");
        repository_store.save(&forge).await.unwrap();
        strategy
            .put_protocol_file(
                &forge,
                "hello/0.1/_/_/revisions/rrev1/files/conanfile.py",
                Bytes::from_static(b"from-member"),
            )
            .await
            .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("conan-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge.id()],
            },
            PackageEcosystem::Conan,
        )
        .unwrap();
        let body = strategy
            .get_protocol_file(&alloy, "hello/0.1/_/_/revisions/rrev1/files/conanfile.py")
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"from-member");
        let hits = strategy.search(&alloy, "hello", 10).await.unwrap();
        assert_eq!(hits[0].name, "hello/0.1@_/_");
    }

    #[tokio::test]
    async fn rejects_upload_to_a_mirror() {
        let strategy = strategy();
        let mirror = Repository::new(
            RepositoryName::parse("conan-mirror").unwrap(),
            RepositoryKind::Mirror {
                upstream: "https://center.conan.io".parse().unwrap(),
            },
            PackageEcosystem::Conan,
        )
        .unwrap();
        let error = strategy
            .put_protocol_file(
                &mirror,
                "hello/0.1/_/_/revisions/rrev1/files/conanfile.py",
                Bytes::from_static(b"nope"),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::ReadOnlyRepository));
    }
}
