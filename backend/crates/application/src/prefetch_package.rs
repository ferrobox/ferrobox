//! Pull a package from a Mirror's upstream without waiting for a client
//! (on-demand pull-through).

use std::collections::BTreeMap;

use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use serde::Deserialize;
use thiserror::Error;

use crate::packaging::{PackagingError, PackagingRegistry, PackagingStrategy};

/// Resultado de un prefetch: se indexó el metadato y, si había
/// versión, se cacheó el binario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchOutcome {
    /// Nombre pedido.
    pub name: String,
    /// Versión, etiqueta o referencia, si se pidió.
    pub version: Option<String>,
    /// `true` si se consultó el índice *upstream*.
    pub indexed: bool,
    /// `true` si se cacheó un binario o un manifiesto.
    pub downloaded: bool,
}

/// Motivos por los que el prefetch puede fallar.
#[derive(Debug, Error)]
pub enum PrefetchError {
    /// Solo un `Mirror` puede prefetch.
    #[error("cannot prefetch into a {0} repository")]
    NotAMirror(&'static str),

    /// The ecosystem does not support prefetch (generic only).
    #[error("prefetch is not supported for the '{0}' ecosystem")]
    UnsupportedEcosystem(&'static str),

    /// OCI/Helm need a tag or digest; Conan needs a recipe version.
    #[error("version is required to prefetch an OCI image or a Conan recipe")]
    MissingVersion,

    /// No hay estrategia registrada.
    #[error("no packaging strategy for ecosystem '{0}'")]
    MissingStrategy(&'static str),

    /// Fallo de la estrategia (upstream, índice, almacén).
    #[error(transparent)]
    Packaging(#[from] PackagingError),
}

/// Caso de uso: calentar la caché de un `Mirror` reutilizando el
/// pull-through de cada ecosistema.
pub struct PrefetchPackageUseCase;

impl PrefetchPackageUseCase {
    /// Indexa `name` en el *upstream* y, si `version` está, descarga
    /// esa versión (o el manifiesto OCI).
    ///
    /// # Errors
    ///
    /// [`PrefetchError`] si el repositorio no es un `Mirror`, el
    /// ecosistema no soporta prefetch, falta la etiqueta OCI, o falla
    /// el *upstream*.
    pub async fn execute(
        packaging: &PackagingRegistry,
        repository: &Repository,
        name: PackageName,
        version: Option<PackageVersion>,
    ) -> Result<PrefetchOutcome, PrefetchError> {
        match repository.kind() {
            RepositoryKind::Mirror { .. } => {}
            other => return Err(PrefetchError::NotAMirror(other.label())),
        }

        let ecosystem = repository.ecosystem();
        if matches!(ecosystem, PackageEcosystem::Generic) {
            return Err(PrefetchError::UnsupportedEcosystem(ecosystem.label()));
        }

        let strategy = packaging
            .strategy_for(ecosystem)
            .ok_or(PrefetchError::MissingStrategy(ecosystem.label()))?;

        if matches!(ecosystem, PackageEcosystem::Oci | PackageEcosystem::Helm) {
            let reference = version.ok_or(PrefetchError::MissingVersion)?;
            strategy
                .get_manifest(repository, name.as_str(), reference.as_str())
                .await?;
            return Ok(PrefetchOutcome {
                name: name.to_string(),
                version: Some(reference.to_string()),
                indexed: false,
                downloaded: true,
            });
        }

        if matches!(ecosystem, PackageEcosystem::Conan) {
            let version = version.ok_or(PrefetchError::MissingVersion)?;
            prefetch_conan_recipe(
                strategy.as_ref(),
                repository,
                name.as_str(),
                version.as_str(),
            )
            .await?;
            return Ok(PrefetchOutcome {
                name: name.to_string(),
                version: Some(version.to_string()),
                indexed: true,
                downloaded: true,
            });
        }

        strategy.index(repository, &name).await?;
        let mut downloaded = false;
        if let Some(version) = version.clone() {
            let coordinate = PackageCoordinate::new(ecosystem, name.clone(), version);
            strategy.download(repository, &coordinate).await?;
            downloaded = true;
        }

        Ok(PrefetchOutcome {
            name: name.to_string(),
            version: version.map(|value| value.to_string()),
            indexed: true,
            downloaded,
        })
    }
}

#[derive(Deserialize)]
struct ConanRevisionStamp {
    revision: String,
}

#[derive(Deserialize)]
struct ConanFilesDocument {
    files: BTreeMap<String, serde_json::Value>,
}

/// Cache the latest Conan recipe revision and its files from upstream.
///
/// Does not pull package binaries (those are profile-specific).
async fn prefetch_conan_recipe(
    strategy: &dyn PackagingStrategy,
    repository: &Repository,
    name: &str,
    version_field: &str,
) -> Result<(), PrefetchError> {
    let (version, user, channel) = split_conan_prefetch_version(version_field);
    let latest_path = format!("{name}/{version}/{user}/{channel}/latest");
    let latest = strategy.protocol_metadata(repository, &latest_path).await?;
    let stamp: ConanRevisionStamp = serde_json::from_slice(&latest)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    let files_path = format!(
        "{name}/{version}/{user}/{channel}/revisions/{}/files",
        stamp.revision
    );
    let files_body = strategy.protocol_metadata(repository, &files_path).await?;
    let files: ConanFilesDocument = serde_json::from_slice(&files_body)
        .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
    for filename in files.files.keys() {
        let file_path = format!("{files_path}/{filename}");
        strategy.get_protocol_file(repository, &file_path).await?;
    }
    Ok(())
}

fn split_conan_prefetch_version(value: &str) -> (String, String, String) {
    match value.split_once('@') {
        Some((version, rest)) => match rest.split_once(':').or_else(|| rest.split_once('/')) {
            Some((user, channel)) => (version.to_string(), user.to_string(), channel.to_string()),
            None => (version.to_string(), rest.to_string(), "_".to_string()),
        },
        None => (value.to_string(), "_".to_string(), "_".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};

    use super::*;
    use crate::content_hash::sha256_checksum;
    use crate::packaging::cargo::CargoPackagingStrategy;
    use crate::packaging::conan::ConanPackagingStrategy;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn cargo_mirror(http: Arc<InMemoryHttpClient>) -> (PackagingRegistry, Repository) {
        let strategy = CargoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http,
            Arc::new(InMemoryRepositoryStore::default()),
        );
        let repository = Repository::new(
            RepositoryName::parse("crates-io").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.example/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        (
            PackagingRegistry::new().register(Arc::new(strategy)),
            repository,
        )
    }

    fn stub_demo_crate(http: &InMemoryHttpClient) -> Bytes {
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
            crate_bytes.clone(),
        );
        crate_bytes
    }

    #[tokio::test]
    async fn prefetches_a_crate_version_from_upstream() {
        let http = Arc::new(InMemoryHttpClient::default());
        stub_demo_crate(&http);
        let (packaging, repository) = cargo_mirror(http);

        let outcome = PrefetchPackageUseCase::execute(
            &packaging,
            &repository,
            PackageName::parse("demo").unwrap(),
            Some(PackageVersion::parse("1.2.3").unwrap()),
        )
        .await
        .unwrap();

        assert!(outcome.indexed);
        assert!(outcome.downloaded);
        assert_eq!(outcome.name, "demo");
        assert_eq!(outcome.version.as_deref(), Some("1.2.3"));
    }

    #[tokio::test]
    async fn name_only_indexes_without_downloading_the_crate() {
        let http = Arc::new(InMemoryHttpClient::default());
        stub_demo_crate(&http);
        let (packaging, repository) = cargo_mirror(http);

        let outcome = PrefetchPackageUseCase::execute(
            &packaging,
            &repository,
            PackageName::parse("demo").unwrap(),
            None,
        )
        .await
        .unwrap();

        assert!(outcome.indexed);
        assert!(!outcome.downloaded);
    }

    #[tokio::test]
    async fn rejects_a_forge() {
        let http = Arc::new(InMemoryHttpClient::default());
        let (packaging, _) = cargo_mirror(http);
        let forge = Repository::new(
            RepositoryName::parse("local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();

        let err = PrefetchPackageUseCase::execute(
            &packaging,
            &forge,
            PackageName::parse("demo").unwrap(),
            None,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PrefetchError::NotAMirror("forge")));
    }

    fn conan_mirror(http: Arc<InMemoryHttpClient>) -> (PackagingRegistry, Repository) {
        let strategy = ConanPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            http,
            Arc::new(InMemoryRepositoryStore::default()),
        );
        let repository = Repository::new(
            RepositoryName::parse("conan-center").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://center.conan.io").unwrap(),
            },
            PackageEcosystem::Conan,
        )
        .unwrap();
        (
            PackagingRegistry::new().register(Arc::new(strategy)),
            repository,
        )
    }

    #[tokio::test]
    async fn prefetches_a_conan_recipe_from_upstream() {
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            "https://center.conan.io/v2/conans/zlib/1.3.1/_/_/latest",
            200,
            Bytes::from_static(br#"{"revision":"rrev1","time":"2026-01-01T00:00:00Z"}"#),
        );
        http.stub(
            "https://center.conan.io/v2/conans/zlib/1.3.1/_/_/revisions/rrev1/files",
            200,
            Bytes::from_static(br#"{"files":{"conanfile.py":{}}}"#),
        );
        http.stub(
            "https://center.conan.io/v2/conans/zlib/1.3.1/_/_/revisions/rrev1/files/conanfile.py",
            200,
            Bytes::from_static(b"from conan import ConanFile\n"),
        );
        let (packaging, repository) = conan_mirror(http);

        let outcome = PrefetchPackageUseCase::execute(
            &packaging,
            &repository,
            PackageName::parse("zlib").unwrap(),
            Some(PackageVersion::parse("1.3.1").unwrap()),
        )
        .await
        .unwrap();

        assert!(outcome.indexed);
        assert!(outcome.downloaded);
        assert_eq!(outcome.name, "zlib");
        assert_eq!(outcome.version.as_deref(), Some("1.3.1"));
    }

    #[tokio::test]
    async fn conan_prefetch_requires_a_version() {
        let http = Arc::new(InMemoryHttpClient::default());
        let (packaging, repository) = conan_mirror(http);
        let err = PrefetchPackageUseCase::execute(
            &packaging,
            &repository,
            PackageName::parse("zlib").unwrap(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PrefetchError::MissingVersion));
    }
}
