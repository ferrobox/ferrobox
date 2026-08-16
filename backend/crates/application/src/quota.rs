//! Cuota de almacenamiento por repositorio.

use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::quota::{StorageQuota, StorageQuotaError};
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::quota_store::{QuotaStore, QuotaStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Uso actual y tope configurado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaSnapshot {
    /// Tope en bytes, o `None` si no hay límite.
    pub limit_bytes: Option<u64>,
    /// Suma de los binarios del repositorio (también los desindexados).
    pub used_bytes: u64,
}

/// Motivos por los que consultar o aplicar una cuota puede fallar.
#[derive(Debug, Error)]
pub enum QuotaError {
    /// El repositorio no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// Un `Alloy` no almacena binarios propios.
    #[error("quota does not apply to Alloy repositories")]
    AlloyRepository,

    /// Falta la migración SQL (`sqlx migrate run` en el directorio backend).
    #[error(
        "missing SQL migration: run `sqlx migrate run` from the backend directory \
         (table repository_quota is missing)"
    )]
    MissingSchema,

    /// El tope pedido no es válido.
    #[error(transparent)]
    InvalidQuota(#[from] StorageQuotaError),

    /// Guardar este binario superaría el tope.
    #[error(
        "repository quota exceeded: using {used_bytes} of {limit_bytes} bytes, \
         need {additional_bytes} more"
    )]
    Exceeded {
        /// Bytes ya ocupados.
        used_bytes: u64,
        /// Tope configurado.
        limit_bytes: u64,
        /// Tamaño del binario que se quiere añadir.
        additional_bytes: u64,
    },

    /// Fallo al consultar el almacén de repositorios.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),

    /// Fallo al consultar artefactos.
    #[error(transparent)]
    Artifacts(#[from] ArtifactStoreError),

    /// Fallo al consultar o persistir la cuota.
    #[error(transparent)]
    Policy(#[from] QuotaStoreError),
}

/// Caso de uso: cuota de almacenamiento.
#[derive(Clone)]
pub struct QuotaService {
    repository_store: Arc<dyn RepositoryStore>,
    artifact_store: Arc<dyn ArtifactStore>,
    quota_store: Arc<dyn QuotaStore>,
}

impl QuotaService {
    /// Construye el servicio a partir de sus puertos.
    #[must_use]
    pub fn new(
        repository_store: Arc<dyn RepositoryStore>,
        artifact_store: Arc<dyn ArtifactStore>,
        quota_store: Arc<dyn QuotaStore>,
    ) -> Self {
        Self {
            repository_store,
            artifact_store,
            quota_store,
        }
    }

    /// Devuelve tope y uso actual.
    ///
    /// # Errors
    ///
    /// [`QuotaError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn get_snapshot(
        &self,
        repository_id: RepositoryId,
    ) -> Result<QuotaSnapshot, QuotaError> {
        self.require_target(repository_id).await?;
        let quota = match self.quota_store.find_by_repository(repository_id).await {
            Ok(quota) => quota,
            Err(QuotaStoreError::MissingSchema) => StorageQuota::unlimited(),
            Err(err) => return Err(err.into()),
        };
        Ok(QuotaSnapshot {
            limit_bytes: quota.limit_bytes(),
            used_bytes: self.used_bytes(repository_id).await?,
        })
    }

    /// Guarda el tope. No borra nada.
    ///
    /// # Errors
    ///
    /// [`QuotaError::RepositoryNotFound`], [`QuotaError::AlloyRepository`]
    /// o [`QuotaError::MissingSchema`].
    pub async fn save(
        &self,
        repository_id: RepositoryId,
        quota: StorageQuota,
    ) -> Result<QuotaSnapshot, QuotaError> {
        self.require_target(repository_id).await?;
        match self.quota_store.save(repository_id, quota).await {
            Ok(()) => {}
            Err(QuotaStoreError::MissingSchema) => return Err(QuotaError::MissingSchema),
            Err(err) => return Err(err.into()),
        }
        self.get_snapshot(repository_id).await
    }

    /// Rechaza un `publish` o un cacheo que no quepa en el tope.
    ///
    /// # Errors
    ///
    /// [`QuotaError::Exceeded`] si `used + additional` supera el tope.
    pub async fn ensure_can_store(
        &self,
        repository_id: RepositoryId,
        additional_bytes: u64,
    ) -> Result<(), QuotaError> {
        if additional_bytes == 0 {
            return Ok(());
        }
        let snapshot = self.get_snapshot(repository_id).await?;
        let Some(limit_bytes) = snapshot.limit_bytes else {
            return Ok(());
        };
        if snapshot.used_bytes.saturating_add(additional_bytes) <= limit_bytes {
            return Ok(());
        }
        Err(QuotaError::Exceeded {
            used_bytes: snapshot.used_bytes,
            limit_bytes,
            additional_bytes,
        })
    }

    async fn used_bytes(&self, repository_id: RepositoryId) -> Result<u64, QuotaError> {
        let artifacts = self
            .artifact_store
            .find_by_repository_id(repository_id)
            .await?;
        Ok(artifacts
            .iter()
            .map(ferrobox_domain::artifact::Artifact::size_bytes)
            .sum())
    }

    async fn require_target(&self, repository_id: RepositoryId) -> Result<(), QuotaError> {
        let Some(repository) = self.repository_store.find_by_id(repository_id).await? else {
            return Err(QuotaError::RepositoryNotFound(repository_id));
        };
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(QuotaError::AlloyRepository);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::artifact::Artifact;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::quota_store::QuotaStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::test_support::{
        InMemoryArtifactStore, InMemoryQuotaStore, InMemoryRepositoryStore,
    };

    use super::*;

    fn checksum() -> Sha256Checksum {
        Sha256Checksum::parse("a".repeat(64)).unwrap()
    }

    #[tokio::test]
    async fn unlimited_by_default_and_blocks_when_set() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let quotas = Arc::new(InMemoryQuotaStore::default());
        let repository = Repository::new(
            RepositoryName::parse("crates").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&repository).await.unwrap();
        let artifact = Artifact::new(repository.id(), checksum(), 80);
        artifacts.save(&artifact).await.unwrap();

        let service = QuotaService::new(repositories, artifacts, quotas);
        service
            .ensure_can_store(repository.id(), 1_000)
            .await
            .unwrap();

        service
            .save(repository.id(), StorageQuota::new(Some(100)).unwrap())
            .await
            .unwrap();
        service
            .ensure_can_store(repository.id(), 20)
            .await
            .unwrap();
        let err = service
            .ensure_can_store(repository.id(), 21)
            .await
            .unwrap_err();
        assert!(matches!(err, QuotaError::Exceeded { used_bytes: 80, limit_bytes: 100, additional_bytes: 21 }));
    }

    #[tokio::test]
    async fn alloy_is_rejected() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let member = Repository::new(
            RepositoryName::parse("member").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        let alloy = Repository::new(
            RepositoryName::parse("all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![member.id()],
            },
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&member).await.unwrap();
        repositories.save(&alloy).await.unwrap();
        let service = QuotaService::new(
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryQuotaStore::default()),
        );
        let err = service.get_snapshot(alloy.id()).await.unwrap_err();
        assert!(matches!(err, QuotaError::AlloyRepository));
    }
}
