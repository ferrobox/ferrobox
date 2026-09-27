//! Search packages in the catalog of every repository.

use std::collections::HashMap;
use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::{RepositoryKind, RepositoryName};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use serde::Deserialize;
use thiserror::Error;

const BLOB_PACKAGE: &str = "_blob";
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;

/// A match: a package in a specific repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSearchHit {
    /// Repository where it is indexed.
    pub repository_id: RepositoryId,
    /// Repository name.
    pub repository_name: RepositoryName,
    /// Forge or Mirror (`Alloy`s do not appear: they keep no index of their own).
    pub repository_kind: RepositoryKind,
    /// Package ecosystem.
    pub ecosystem: PackageEcosystem,
    /// Package name.
    pub name: String,
    /// Displayed version (the latest non-yanked, or the latest yanked).
    pub version: String,
    /// `true` if that version is yanked.
    pub yanked: bool,
}

/// Reasons searching packages can fail.
#[derive(Debug, Error)]
pub enum SearchPackagesError {
    /// Failed to list repositories.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),

    /// Failed to read the package index.
    #[error(transparent)]
    Index(#[from] PackageIndexStoreError),
}

/// Use case: search packages by name across the instance.
#[allow(clippy::struct_field_names)]
pub struct SearchPackagesUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
}

#[derive(Deserialize)]
struct IndexYanked {
    #[serde(default)]
    yanked: bool,
}

impl SearchPackagesUseCase {
    /// Builds the use case from its ports.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
    ) -> Self {
        Self {
            repository_store,
            package_index_store,
        }
    }

    /// Returns up to `limit` packages whose name contains `query`.
    ///
    /// Empty or whitespace-only → no matches. Does not query *upstreams*:
    /// a Mirror only shows what it already has in the local index.
    ///
    /// # Errors
    ///
    /// [`SearchPackagesError`] if a port fails.
    pub async fn execute(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, SearchPackagesError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, MAX_LIMIT);
        let needle = query.to_lowercase();

        let mut grouped: HashMap<(RepositoryId, String), PackageSearchHit> = HashMap::new();
        for repository in self.repository_store.find_all().await? {
            if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
                continue;
            }
            let entries = self
                .package_index_store
                .list_entries(repository.id())
                .await?;
            for entry in entries {
                let name = entry.coordinate.name().as_str();
                let version = entry.coordinate.version().as_str();
                if !is_catalog_package(name, version) {
                    continue;
                }
                if !name.to_lowercase().contains(&needle) {
                    continue;
                }
                let yanked = serde_json::from_slice::<IndexYanked>(&entry.entry)
                    .is_ok_and(|meta| meta.yanked);
                let key = (repository.id(), name.to_owned());
                match grouped.get(&key) {
                    Some(current) if !current.yanked && yanked => {}
                    _ => {
                        grouped.insert(
                            key,
                            PackageSearchHit {
                                repository_id: repository.id(),
                                repository_name: repository.name().clone(),
                                repository_kind: repository.kind().clone(),
                                ecosystem: entry.coordinate.ecosystem(),
                                name: name.to_owned(),
                                version: version.to_owned(),
                                yanked,
                            },
                        );
                    }
                }
            }
        }

        let mut hits: Vec<PackageSearchHit> = grouped.into_values().collect();
        hits.sort_by(|left, right| {
            let left_prefix = left.name.to_lowercase().starts_with(&needle);
            let right_prefix = right.name.to_lowercase().starts_with(&needle);
            right_prefix
                .cmp(&left_prefix)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.repository_name.as_str().cmp(right.repository_name.as_str()))
                .then_with(|| left.ecosystem.label().cmp(right.ecosystem.label()))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

fn is_catalog_package(name: &str, version: &str) -> bool {
    name != BLOB_PACKAGE && !version.starts_with("sha256:")
}

/// Default limit if the client does not send one.
#[must_use]
pub fn default_search_limit() -> usize {
    DEFAULT_LIMIT
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{InMemoryPackageIndexStore, InMemoryRepositoryStore};

    use super::*;

    async fn index(
        store: &InMemoryPackageIndexStore,
        repository: &Repository,
        name: &str,
        version: &str,
        entry: &'static [u8],
    ) {
        store
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    repository.ecosystem(),
                    PackageName::parse(name).unwrap(),
                    PackageVersion::parse(version).unwrap(),
                ),
                None,
                Bytes::from_static(entry),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn empty_query_returns_no_hits() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let index_store = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("crates").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&repository).await.unwrap();
        index(index_store.as_ref(), &repository, "serde", "1.0.0", b"{}").await;

        let hits = SearchPackagesUseCase::new(repositories, index_store)
            .execute("  ", 50)
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn finds_packages_across_repos_and_skips_alloy_blobs_and_digests() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let index_store = Arc::new(InMemoryPackageIndexStore::default());
        let forge = Repository::new(
            RepositoryName::parse("crates-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let mirror = Repository::new(
            RepositoryName::parse("crates-io").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.crates.io/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("all-crates").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let oci = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repositories.save(&forge).await.unwrap();
        repositories.save(&mirror).await.unwrap();
        repositories.save(&alloy).await.unwrap();
        repositories.save(&oci).await.unwrap();

        index(index_store.as_ref(), &forge, "serde", "1.0.0", b"{}").await;
        index(
            index_store.as_ref(),
            &forge,
            "serde",
            "1.0.1",
            br#"{"yanked":true}"#,
        )
        .await;
        index(index_store.as_ref(), &mirror, "serde-json", "1.0.0", b"{}").await;
        index(index_store.as_ref(), &alloy, "serde", "9.9.9", b"{}").await;
        index(index_store.as_ref(), &oci, "_blob", "sha256:aaaa", b"{}").await;

        let use_case = SearchPackagesUseCase::new(repositories, index_store);
        let hits = use_case.execute("ser", 50).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "serde");
        assert_eq!(hits[0].version, "1.0.0");
        assert!(!hits[0].yanked);
        assert_eq!(hits[0].repository_name.as_str(), "crates-releases");
        assert_eq!(hits[1].name, "serde-json");
        assert!(use_case.execute("_blob", 50).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn prefers_prefix_matches_and_respects_limit() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let index_store = Arc::new(InMemoryPackageIndexStore::default());
        let forge = Repository::new(
            RepositoryName::parse("crates").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&forge).await.unwrap();
        index(index_store.as_ref(), &forge, "lazy-serde", "1.0.0", b"{}").await;
        index(index_store.as_ref(), &forge, "serde", "1.0.0", b"{}").await;
        index(index_store.as_ref(), &forge, "serde-json", "1.0.0", b"{}").await;

        let hits = SearchPackagesUseCase::new(repositories, index_store)
            .execute("serde", 2)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "serde");
        assert_eq!(hits[1].name, "serde-json");
    }

    #[tokio::test]
    async fn oci_tag_is_searchable() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let index_store = Arc::new(InMemoryPackageIndexStore::default());
        let oci = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repositories.save(&oci).await.unwrap();
        index(index_store.as_ref(), &oci, "alpine", "3.20", b"{}").await;
        index(
            index_store.as_ref(),
            &oci,
            "alpine",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            b"{}",
        )
        .await;

        let hits = SearchPackagesUseCase::new(repositories, index_store)
            .execute("alpine", 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].version, "3.20");
    }
}
