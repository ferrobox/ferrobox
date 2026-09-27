use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use thiserror::Error;

/// Reasons a package-index operation can fail.
#[derive(Debug, Error)]
pub enum PackageIndexStoreError {
    /// The concrete persistence backend returned its own error.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Relationship between a stored artifact and the package coordinate
/// (ecosystem, name, and version) that published it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedArtifact {
    /// Identifier of the persisted binary.
    pub artifact_id: ArtifactId,
    /// Associated package coordinate.
    pub coordinate: PackageCoordinate,
    /// Index entry already serialized by the packaging strategy (one
    /// JSON line for Cargo). Lets callers read flags such as `yanked`
    /// without this port knowing the ecosystem.
    pub entry: Bytes,
}

/// Package-index row, with or without a cached binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageIndexRecord {
    /// Identifier of the persisted binary, if it has already been cached.
    pub artifact_id: Option<ArtifactId>,
    /// Associated package coordinate.
    pub coordinate: PackageCoordinate,
    /// Index entry already serialized by the packaging strategy.
    pub entry: Bytes,
    /// Time the row was inserted, in RFC 3339.
    pub created_at_rfc3339: String,
}

/// Persistence port for the package index: the list, by repository and
/// coordinate, of the entries each packaging strategy
/// (`PackagingStrategy`) needs to answer its ecosystem index protocol
/// (for example, the Cargo sparse index).
///
/// This port is deliberately agnostic of each ecosystem's format:
/// `entry` is a byte block already serialized by the corresponding
/// strategy (one JSON line for Cargo, and potentially another format
/// for future ecosystems). The port only stores it and returns it in
/// publication order, just as `ArtifactStore` does not understand the
/// binary content it stores.
#[async_trait]
pub trait PackageIndexStore: Send + Sync {
    /// Inserts or replaces the index entry of a specific package
    /// coordinate. `artifact_id` may be `None` when the entry comes
    /// from the *upstream* of a `Mirror` and the binary has not been
    /// cached locally yet.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn upsert_entry(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
        artifact_id: Option<ArtifactId>,
        entry: Bytes,
    ) -> Result<(), PackageIndexStoreError>;

    /// Lists the index entries of every published version of a package,
    /// in the order they were published.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn entries_for_package(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &PackageName,
    ) -> Result<Vec<Bytes>, PackageIndexStoreError>;

    /// Looks up the identifier of the binary artifact associated with
    /// an already published package coordinate. Returns `None` if that
    /// coordinate was never published in that repository.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn artifact_for(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<ArtifactId>, PackageIndexStoreError>;

    /// Deletes the index entries associated with an artifact. It is not
    /// an error if there are none.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn delete_by_artifact(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<(), PackageIndexStoreError>;

    /// Deletes every index entry of a repository. It is not an error if
    /// the repository has none.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn delete_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<(), PackageIndexStoreError>;

    /// Lists the index entries of every package in a repository, in the
    /// order they were published.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn entries_for_repository(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
    ) -> Result<Vec<Bytes>, PackageIndexStoreError>;

    /// Lists the package coordinates of a repository that already have
    /// an associated binary artifact (name, version, and identifier).
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn find_indexed_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<IndexedArtifact>, PackageIndexStoreError>;

    /// Lists every index entry of a repository, including those that
    /// do not yet have a cached binary.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn list_entries(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<PackageIndexRecord>, PackageIndexStoreError>;

    /// Deletes the index entry of a coordinate. It is not an error if
    /// it does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`PackageIndexStoreError::Backend`] if the underlying
    /// backend fails.
    async fn delete_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<(), PackageIndexStoreError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ferrobox_domain::package_coordinate::PackageVersion;

    use super::*;

    type EntryMap = HashMap<(RepositoryId, PackageCoordinate), (Option<ArtifactId>, Bytes)>;

    #[derive(Default)]
    struct InMemoryPackageIndexStore {
        entries: Mutex<EntryMap>,
    }

    #[async_trait]
    impl PackageIndexStore for InMemoryPackageIndexStore {
        async fn upsert_entry(
            &self,
            repository_id: RepositoryId,
            coordinate: &PackageCoordinate,
            artifact_id: Option<ArtifactId>,
            entry: Bytes,
        ) -> Result<(), PackageIndexStoreError> {
            let mut entries = self.entries.lock().unwrap();
            let key = (repository_id, coordinate.clone());
            let merged_artifact = match entries.get(&key) {
                Some((existing, _)) => existing.or(artifact_id),
                None => artifact_id,
            };
            entries.insert(key, (merged_artifact, entry));
            Ok(())
        }

        async fn entries_for_package(
            &self,
            repository_id: RepositoryId,
            ecosystem: PackageEcosystem,
            name: &PackageName,
        ) -> Result<Vec<Bytes>, PackageIndexStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|((repo_id, coordinate), _)| {
                    *repo_id == repository_id
                        && coordinate.ecosystem() == ecosystem
                        && coordinate.name() == name
                })
                .map(|(_, (_, entry))| entry.clone())
                .collect())
        }

        async fn artifact_for(
            &self,
            repository_id: RepositoryId,
            coordinate: &PackageCoordinate,
        ) -> Result<Option<ArtifactId>, PackageIndexStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(&(repository_id, coordinate.clone()))
                .and_then(|(artifact_id, _)| *artifact_id))
        }

        async fn delete_by_artifact(
            &self,
            artifact_id: ArtifactId,
        ) -> Result<(), PackageIndexStoreError> {
            self.entries
                .lock()
                .unwrap()
                .retain(|_, (existing, _)| *existing != Some(artifact_id));
            Ok(())
        }

        async fn delete_by_repository(
            &self,
            repository_id: RepositoryId,
        ) -> Result<(), PackageIndexStoreError> {
            self.entries
                .lock()
                .unwrap()
                .retain(|(existing, _), _| *existing != repository_id);
            Ok(())
        }

        async fn entries_for_repository(
            &self,
            repository_id: RepositoryId,
            ecosystem: PackageEcosystem,
        ) -> Result<Vec<Bytes>, PackageIndexStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|((repo_id, coordinate), _)| {
                    *repo_id == repository_id && coordinate.ecosystem() == ecosystem
                })
                .map(|(_, (_, entry))| entry.clone())
                .collect())
        }

        async fn find_indexed_by_repository(
            &self,
            repository_id: RepositoryId,
        ) -> Result<Vec<IndexedArtifact>, PackageIndexStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter_map(|((repo_id, coordinate), (artifact_id, entry))| {
                    if *repo_id != repository_id {
                        return None;
                    }
                    artifact_id.map(|artifact_id| IndexedArtifact {
                        artifact_id,
                        coordinate: coordinate.clone(),
                        entry: entry.clone(),
                    })
                })
                .collect())
        }

        async fn list_entries(
            &self,
            repository_id: RepositoryId,
        ) -> Result<Vec<PackageIndexRecord>, PackageIndexStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter_map(|((repo_id, coordinate), (artifact_id, entry))| {
                    if *repo_id != repository_id {
                        return None;
                    }
                    Some(PackageIndexRecord {
                        artifact_id: *artifact_id,
                        coordinate: coordinate.clone(),
                        entry: entry.clone(),
                        created_at_rfc3339: "2026-01-01T00:00:00Z".to_string(),
                    })
                })
                .collect())
        }

        async fn delete_by_coordinate(
            &self,
            repository_id: RepositoryId,
            coordinate: &PackageCoordinate,
        ) -> Result<(), PackageIndexStoreError> {
            self.entries
                .lock()
                .unwrap()
                .remove(&(repository_id, coordinate.clone()));
            Ok(())
        }
    }

    fn coordinate(version: &str) -> PackageCoordinate {
        PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("ferrobox-cli").unwrap(),
            PackageVersion::parse(version).unwrap(),
        )
    }

    #[tokio::test]
    async fn upsert_then_artifact_for_returns_the_associated_artifact() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();
        let artifact_id = ArtifactId::new();

        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(artifact_id),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let found = store
            .artifact_for(repository_id, &coordinate("1.0.0"))
            .await
            .unwrap();

        assert_eq!(found, Some(artifact_id));
    }

    #[tokio::test]
    async fn artifact_for_an_unpublished_coordinate_returns_none() {
        let store = InMemoryPackageIndexStore::default();

        let found = store
            .artifact_for(RepositoryId::new(), &coordinate("1.0.0"))
            .await
            .unwrap();

        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn entries_for_package_lists_every_published_version() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();

        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{\"vers\":\"1.0.0\"}"),
            )
            .await
            .unwrap();
        store
            .upsert_entry(
                repository_id,
                &coordinate("1.1.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{\"vers\":\"1.1.0\"}"),
            )
            .await
            .unwrap();

        let entries = store
            .entries_for_package(
                repository_id,
                PackageEcosystem::Cargo,
                &PackageName::parse("ferrobox-cli").unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn delete_by_artifact_removes_only_that_artifact_entries() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();
        let first = ArtifactId::new();
        let second = ArtifactId::new();

        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(first),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();
        store
            .upsert_entry(
                repository_id,
                &coordinate("1.1.0"),
                Some(second),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        store.delete_by_artifact(first).await.unwrap();

        assert_eq!(
            store
                .artifact_for(repository_id, &coordinate("1.0.0"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .artifact_for(repository_id, &coordinate("1.1.0"))
                .await
                .unwrap(),
            Some(second)
        );
    }

    #[tokio::test]
    async fn delete_by_repository_clears_every_entry() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();
        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        store.delete_by_repository(repository_id).await.unwrap();

        let entries = store
            .entries_for_package(
                repository_id,
                PackageEcosystem::Cargo,
                &PackageName::parse("ferrobox-cli").unwrap(),
            )
            .await
            .unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn entries_for_repository_lists_every_package() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();
        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{\"vers\":\"1.0.0\"}"),
            )
            .await
            .unwrap();
        store
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("other").unwrap(),
                    PackageVersion::parse("2.0.0").unwrap(),
                ),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{\"vers\":\"2.0.0\"}"),
            )
            .await
            .unwrap();

        let entries = store
            .entries_for_repository(repository_id, PackageEcosystem::Cargo)
            .await
            .unwrap();

        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn delete_by_coordinate_removes_only_that_row() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();
        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();
        store
            .upsert_entry(
                repository_id,
                &coordinate("1.1.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        store
            .delete_by_coordinate(repository_id, &coordinate("1.0.0"))
            .await
            .unwrap();

        let listed = store.list_entries(repository_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].coordinate.version().as_str(), "1.1.0");
    }

    #[tokio::test]
    async fn find_indexed_by_repository_skips_entries_without_artifact() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();
        let artifact_id = ArtifactId::new();

        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(artifact_id),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();
        store
            .upsert_entry(
                repository_id,
                &coordinate("2.0.0"),
                None,
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let indexed = store
            .find_indexed_by_repository(repository_id)
            .await
            .unwrap();

        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].artifact_id, artifact_id);
        assert_eq!(indexed[0].coordinate.version().as_str(), "1.0.0");
    }
}
