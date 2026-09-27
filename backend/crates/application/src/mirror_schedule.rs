//! Refresh interval of a `Mirror` and the sweep of those due to be
//! reindexed.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{PackageEcosystem, PackageName, PackageVersion};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::RepositoryStoreError;
use thiserror::Error;

use crate::get_repository::{GetRepositoryError, GetRepositoryUseCase};
use crate::packaging::PackagingRegistry;
use crate::prefetch_package::{PrefetchError, PrefetchPackageUseCase};

/// Minimum hours between scheduled refreshes.
pub const MIN_PREFETCH_INTERVAL_HOURS: u32 = 1;
/// Maximum hours (one week).
pub const MAX_PREFETCH_INTERVAL_HOURS: u32 = 168;

/// Reasons the interval cannot be saved.
#[derive(Debug, Error)]
pub enum SetMirrorScheduleError {
    /// Only a `Mirror` has a prefetch cron.
    #[error("cannot schedule prefetch on a {0} repository")]
    NotAMirror(&'static str),

    /// The interval is outside 1..=168 (except 0 / absent = turn off).
    #[error(
        "prefetch interval must be between {MIN_PREFETCH_INTERVAL_HOURS} and {MAX_PREFETCH_INTERVAL_HOURS} hours, or 0 to disable"
    )]
    InvalidInterval,

    /// The repository does not exist or persistence failed.
    #[error(transparent)]
    Repository(#[from] GetRepositoryError),
}

/// Reasons the scheduled sweep can fail.
#[derive(Debug, Error)]
pub enum MirrorScheduleRunError {
    /// Failed to list or save repositories.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),

    /// Failed to read the index of a due `Mirror`.
    #[error(transparent)]
    Index(#[from] PackageIndexStoreError),
}

/// Normalizes the interval: `None` or `0` turns the cron off; 1..=168
/// leaves it on.
///
/// # Errors
///
/// [`SetMirrorScheduleError::InvalidInterval`] if the value is positive
/// and falls outside the range.
pub fn normalize_prefetch_interval(
    hours: Option<u32>,
) -> Result<Option<u32>, SetMirrorScheduleError> {
    match hours {
        None | Some(0) => Ok(None),
        Some(hours)
            if (MIN_PREFETCH_INTERVAL_HOURS..=MAX_PREFETCH_INTERVAL_HOURS).contains(&hours) =>
        {
            Ok(Some(hours))
        }
        Some(_) => Err(SetMirrorScheduleError::InvalidInterval),
    }
}

/// Use case: save the refresh interval of a `Mirror`.
pub struct SetMirrorScheduleUseCase;

impl SetMirrorScheduleUseCase {
    /// Updates `prefetch_interval_hours` and keeps `last_prefetch_at`.
    ///
    /// # Errors
    ///
    /// [`SetMirrorScheduleError`] if it is not a `Mirror`, the interval
    /// is not valid, or persistence fails.
    pub async fn execute(
        repositories: &GetRepositoryUseCase,
        id: RepositoryId,
        interval_hours: Option<u32>,
    ) -> Result<Repository, SetMirrorScheduleError> {
        let repository = repositories.execute(id).await?;
        match repository.kind() {
            RepositoryKind::Mirror { .. } => {}
            other => return Err(SetMirrorScheduleError::NotAMirror(other.label())),
        }

        let interval = normalize_prefetch_interval(interval_hours)?;
        let last = repository.last_prefetch_at();
        let updated = repository.with_prefetch_schedule(interval, last);
        repositories.save(&updated).await?;
        Ok(updated)
    }
}

/// Walks `Mirror`s whose interval is due, reindexes what is already
/// known, and stamps `last_prefetch_at`.
///
/// Cargo, npm, and the other indexes refresh by name (without
/// downloading the binary). OCI and Helm request each name+version
/// pair. A failure of one package does not abort the rest or prevent
/// stamping the run: the next tick waits for the interval.
///
/// # Errors
///
/// [`MirrorScheduleRunError`] if listing repositories, reading the
/// index, or persisting the last-run stamp fails.
pub async fn run_due(
    repositories: &dyn ferrobox_ports::repository_store::RepositoryStore,
    package_index: &dyn PackageIndexStore,
    packaging: &PackagingRegistry,
    now: DateTime<Utc>,
) -> Result<u32, MirrorScheduleRunError> {
    let mut refreshed = 0;
    for repository in repositories.find_all().await? {
        if !repository.prefetch_is_due(now) {
            continue;
        }

        let entries = package_index.list_entries(repository.id()).await?;
        for (name, version) in prefetch_targets(repository.ecosystem(), &entries) {
            if let Err(err) =
                PrefetchPackageUseCase::execute(packaging, &repository, name, version).await
            {
                match err {
                    PrefetchError::UnsupportedEcosystem(_) | PrefetchError::NotAMirror(_) => {
                        break;
                    }
                    PrefetchError::MissingVersion
                    | PrefetchError::MissingStrategy(_)
                    | PrefetchError::Packaging(_) => {}
                }
            }
        }

        let interval = repository.prefetch_interval_hours();
        let updated = repository.with_prefetch_schedule(interval, Some(now));
        repositories.save(&updated).await?;
        refreshed += 1;
    }
    Ok(refreshed)
}

fn prefetch_targets(
    ecosystem: PackageEcosystem,
    entries: &[ferrobox_ports::package_index_store::PackageIndexRecord],
) -> Vec<(PackageName, Option<PackageVersion>)> {
    match ecosystem {
        PackageEcosystem::Generic => Vec::new(),
        PackageEcosystem::Oci | PackageEcosystem::Helm | PackageEcosystem::Conan => entries
            .iter()
            .map(|entry| {
                (
                    entry.coordinate.name().clone(),
                    Some(entry.coordinate.version().clone()),
                )
            })
            .collect(),
        _ => {
            let mut seen = HashSet::new();
            let mut targets = Vec::new();
            for entry in entries {
                let name = entry.coordinate.name();
                if seen.insert(name.clone()) {
                    targets.push((name.clone(), None));
                }
            }
            targets
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};

    use super::*;
    use crate::content_hash::sha256_checksum;
    use crate::get_repository::GetRepositoryUseCase;
    use crate::packaging::cargo::CargoPackagingStrategy;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn cargo_mirror() -> Repository {
        Repository::new(
            RepositoryName::parse("crates-io").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.example/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap()
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-26T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn stub_demo_crate(http: &InMemoryHttpClient) {
        let crate_bytes = Bytes::from_static(b"cached-crate-bytes");
        let cksum = sha256_checksum(&crate_bytes).to_string();
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
    }

    fn packaging_with(
        http: Arc<InMemoryHttpClient>,
        index: Arc<InMemoryPackageIndexStore>,
        repositories: Arc<InMemoryRepositoryStore>,
    ) -> PackagingRegistry {
        let strategy = CargoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            index,
            Arc::new(InMemoryStorage::default()),
            http,
            repositories,
        );
        PackagingRegistry::new().register(Arc::new(strategy))
    }

    #[tokio::test]
    async fn sets_the_interval_on_a_mirror() {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let repository = cargo_mirror();
        store.save(&repository).await.unwrap();
        let get = GetRepositoryUseCase::new(store);

        let updated = SetMirrorScheduleUseCase::execute(&get, repository.id(), Some(6))
            .await
            .unwrap();

        assert_eq!(updated.prefetch_interval_hours(), Some(6));
        assert_eq!(updated.last_prefetch_at(), None);
    }

    #[tokio::test]
    async fn zero_disables_the_interval() {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let repository = cargo_mirror().with_prefetch_schedule(Some(6), None);
        store.save(&repository).await.unwrap();
        let get = GetRepositoryUseCase::new(store);

        let updated = SetMirrorScheduleUseCase::execute(&get, repository.id(), Some(0))
            .await
            .unwrap();

        assert_eq!(updated.prefetch_interval_hours(), None);
    }

    #[tokio::test]
    async fn rejects_a_forge() {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let forge = Repository::new(
            RepositoryName::parse("local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        store.save(&forge).await.unwrap();
        let get = GetRepositoryUseCase::new(store);

        let err = SetMirrorScheduleUseCase::execute(&get, forge.id(), Some(1))
            .await
            .unwrap_err();

        assert!(matches!(err, SetMirrorScheduleError::NotAMirror("forge")));
    }

    #[tokio::test]
    async fn rejects_an_interval_over_a_week() {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let repository = cargo_mirror();
        store.save(&repository).await.unwrap();
        let get = GetRepositoryUseCase::new(store);

        let err = SetMirrorScheduleUseCase::execute(&get, repository.id(), Some(169))
            .await
            .unwrap_err();

        assert!(matches!(err, SetMirrorScheduleError::InvalidInterval));
    }

    #[tokio::test]
    async fn run_due_refreshes_unique_names_and_marks_last_run() {
        let http = Arc::new(InMemoryHttpClient::default());
        stub_demo_crate(&http);
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryRepositoryStore::default());
        let packaging = packaging_with(http, index.clone(), store.clone());
        let repository = cargo_mirror().with_prefetch_schedule(Some(1), None);
        store.save(&repository).await.unwrap();
        let demo = PackageName::parse("demo").unwrap();
        index
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    demo.clone(),
                    PackageVersion::parse("1.0.0").unwrap(),
                ),
                None,
                Bytes::from_static(b"old-1"),
            )
            .await
            .unwrap();
        index
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    demo,
                    PackageVersion::parse("1.2.3").unwrap(),
                ),
                None,
                Bytes::from_static(b"old-2"),
            )
            .await
            .unwrap();

        let refreshed = run_due(store.as_ref(), index.as_ref(), &packaging, now())
            .await
            .unwrap();

        assert_eq!(refreshed, 1);
        let saved = store.find_by_id(repository.id()).await.unwrap().unwrap();
        assert_eq!(saved.last_prefetch_at(), Some(now()));
        let lines = index
            .entries_for_package(
                repository.id(),
                PackageEcosystem::Cargo,
                &PackageName::parse("demo").unwrap(),
            )
            .await
            .unwrap();
        assert!(
            lines
                .iter()
                .any(|line| String::from_utf8_lossy(line).contains("1.2.3"))
        );
    }

    #[tokio::test]
    async fn run_due_skips_a_mirror_that_is_not_due() {
        let http = Arc::new(InMemoryHttpClient::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryRepositoryStore::default());
        let packaging = packaging_with(http, index.clone(), store.clone());
        let repository = cargo_mirror().with_prefetch_schedule(Some(1), Some(now()));
        store.save(&repository).await.unwrap();

        let refreshed = run_due(store.as_ref(), index.as_ref(), &packaging, now())
            .await
            .unwrap();

        assert_eq!(refreshed, 0);
        let saved = store.find_by_id(repository.id()).await.unwrap().unwrap();
        assert_eq!(saved.last_prefetch_at(), Some(now()));
    }
}
