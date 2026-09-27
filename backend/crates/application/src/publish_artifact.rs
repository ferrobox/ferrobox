use std::sync::Arc;

use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::content_hash::sha256_checksum;
use crate::quota::{QuotaError, QuotaService};
use crate::storage_key::storage_key_for;

/// Motivos por los que publicar un artefacto puede fallar.
#[derive(Debug, Error)]
pub enum PublishArtifactError {
    /// El repositorio indicado no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// El repositorio es de solo lectura (un `Mirror` o un `Alloy`).
    #[error("repository is read-only and does not accept publishes")]
    ReadOnlyRepository,

    /// Fallo al consultar el repositorio.
    #[error(transparent)]
    RepositoryLookup(#[from] RepositoryStoreError),

    /// Fallo al subir el contenido binario.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// Fallo al persistir los metadatos del artefacto.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),

    /// El binario no cabe en la cuota de almacenamiento.
    #[error(transparent)]
    Quota(#[from] QuotaError),
}

/// Caso de uso: publicar un artefacto binario en un repositorio
/// existente.
pub struct PublishArtifactUseCase {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    storage: Arc<dyn StoragePort>,
    quota: QuotaService,
}

impl PublishArtifactUseCase {
    /// Construye el caso de uso a partir de sus puertos.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        storage: Arc<dyn StoragePort>,
        quota: QuotaService,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            storage,
            quota,
        }
    }

    /// Ejecuta la publicación: verifica que el repositorio existe,
    /// calcula el checksum del contenido, lo sube al almacenamiento
    /// binario, y persiste los metadatos del artefacto.
    ///
    /// # Errors
    ///
    /// Devuelve [`PublishArtifactError::RepositoryNotFound`] si
    /// `repository_id` no corresponde a ningún repositorio existente, o
    /// cualquiera de las demás variantes si falla el puerto
    /// correspondiente.
    ///
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
        content: Bytes,
    ) -> Result<ArtifactId, PublishArtifactError> {
        self.execute_named(repository_id, content, None).await
    }

    /// Like [`Self::execute`], storing `filename` when the client sent one.
    ///
    /// # Errors
    ///
    /// Same as [`Self::execute`].
    pub async fn execute_named(
        &self,
        repository_id: RepositoryId,
        content: Bytes,
        filename: Option<&str>,
    ) -> Result<ArtifactId, PublishArtifactError> {
        let Some(repository) = self.repository_store.find_by_id(repository_id).await? else {
            return Err(PublishArtifactError::RepositoryNotFound(repository_id));
        };
        if matches!(
            repository.kind(),
            RepositoryKind::Mirror { .. } | RepositoryKind::Alloy { .. }
        ) {
            return Err(PublishArtifactError::ReadOnlyRepository);
        }

        let checksum = sha256_checksum(&content);
        let artifact = Artifact::new(repository_id, checksum, content.len() as u64)
            .with_filename(filename.and_then(sanitize_filename));

        self.quota
            .ensure_can_store(repository_id, content.len() as u64)
            .await?;

        self.storage
            .put(&storage_key_for(artifact.id()), content)
            .await?;
        self.artifact_store.save(&artifact).await?;

        Ok(artifact.id())
    }
}

fn sanitize_filename(raw: &str) -> Option<String> {
    let name = raw.replace('\\', "/");
    let name = name.rsplit('/').next().unwrap_or("").trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if name.chars().any(char::is_control) || name.len() > 214 {
        return None;
    }
    Some(name.to_string())
}

use ferrobox_domain::ids::ArtifactId;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;

    use crate::quota::QuotaService;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryQuotaStore, InMemoryRepositoryStore, InMemoryStorage, forge,
    };
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::quota::StorageQuota;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::quota_store::QuotaStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;

    fn use_case(
        repository_store: Arc<InMemoryRepositoryStore>,
        artifact_store: Arc<InMemoryArtifactStore>,
        storage: Arc<InMemoryStorage>,
    ) -> PublishArtifactUseCase {
        let quota = QuotaService::new(
            repository_store.clone(),
            artifact_store.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        PublishArtifactUseCase::new(repository_store, artifact_store, storage, quota)
    }

    #[tokio::test]
    async fn publishes_an_artifact_to_an_existing_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());

        let repository = forge("cargo-releases");
        repository_store.save(&repository).await.unwrap();

        let use_case = use_case(repository_store, artifact_store.clone(), storage);

        let artifact_id = use_case
            .execute(repository.id(), Bytes::from_static(b"hello, ferrobox"))
            .await
            .unwrap();

        let stored = artifact_store
            .find_by_id(artifact_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.checksum().as_str(),
            crate::content_hash::sha256_checksum(b"hello, ferrobox").as_str()
        );
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_nonexistent_repository() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let use_case = use_case(repository_store, artifact_store, storage);

        let result = use_case
            .execute(RepositoryId::new(), Bytes::from_static(b"data"))
            .await;

        assert!(matches!(
            result,
            Err(PublishArtifactError::RepositoryNotFound(_))
        ));
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_mirror_or_alloy() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());

        let mirror = Repository::new(
            RepositoryName::parse("crates-io").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://index.crates.io/").unwrap(),
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let forge_member = Repository::new(
            RepositoryName::parse("crates-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("crates-alloy").unwrap(),
            RepositoryKind::Alloy {
                members: vec![forge_member.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repository_store.save(&mirror).await.unwrap();
        repository_store.save(&forge_member).await.unwrap();
        repository_store.save(&alloy).await.unwrap();

        let use_case = use_case(repository_store, artifact_store, storage);

        let mirror_result = use_case
            .execute(mirror.id(), Bytes::from_static(b"data"))
            .await;
        assert!(matches!(
            mirror_result,
            Err(PublishArtifactError::ReadOnlyRepository)
        ));

        let alloy_result = use_case
            .execute(alloy.id(), Bytes::from_static(b"data"))
            .await;
        assert!(matches!(
            alloy_result,
            Err(PublishArtifactError::ReadOnlyRepository)
        ));
    }

    #[tokio::test]
    async fn rejects_publish_when_quota_is_exceeded() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let quotas = Arc::new(InMemoryQuotaStore::default());
        let repository = forge("cargo-releases");
        repository_store.save(&repository).await.unwrap();
        quotas
            .save(repository.id(), StorageQuota::new(Some(4)).unwrap())
            .await
            .unwrap();
        let quota = QuotaService::new(repository_store.clone(), artifact_store.clone(), quotas);
        let use_case =
            PublishArtifactUseCase::new(repository_store, artifact_store, storage, quota);

        let err = use_case
            .execute(repository.id(), Bytes::from_static(b"too-big"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PublishArtifactError::Quota(QuotaError::Exceeded { .. })
        ));
    }

    #[tokio::test]
    async fn stores_a_sanitized_upload_filename() {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let repository = forge("binaries");
        repository_store.save(&repository).await.unwrap();
        let use_case = use_case(repository_store, artifact_store.clone(), storage);

        let artifact_id = use_case
            .execute_named(
                repository.id(),
                Bytes::from_static(b"hello"),
                Some(r"C:\Downloads\firefox-142.0.1.tar.xz"),
            )
            .await
            .unwrap();

        let stored = artifact_store
            .find_by_id(artifact_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.filename(), Some("firefox-142.0.1.tar.xz"));
    }
}
