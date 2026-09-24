//! Política de admisión: persistencia, dry-run y denegación en el pull.

use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use ferrobox_domain::admission::{
    AdmissionEffect, AdmissionEvent, AdmissionPolicy, AdmissionPolicyError,
};
use ferrobox_domain::ids::{AdmissionEventId, RepositoryId};
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::admission_store::{AdmissionStore, AdmissionStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

use crate::list_repository_artifacts::{
    ListRepositoryArtifactsError, ListRepositoryArtifactsUseCase, ListedArtifact,
};
use crate::packaging::PackagingError;

/// Motivos por los que consultar o guardar la política de admisión
/// puede fallar.
#[derive(Debug, Error)]
pub enum AdmissionError {
    /// El repositorio no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// Un `Alloy` no tiene política propia: rigen las de sus miembros.
    #[error("alloy repositories do not have their own admission policy")]
    AlloyRepository,

    /// Solo un Forge OCI o Helm admite política de firma en este corte.
    #[error("admission policies apply to Forge OCI and Helm repositories")]
    UnsupportedRepository,

    /// La política enviada no es válida.
    #[error(transparent)]
    InvalidPolicy(#[from] AdmissionPolicyError),

    /// Fallo al leer o escribir la política.
    #[error(transparent)]
    Store(#[from] AdmissionStoreError),

    /// Fallo al listar artefactos para el dry-run.
    #[error(transparent)]
    Artifacts(#[from] ListRepositoryArtifactsError),

    /// Fallo al consultar repositorios.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),
}

/// Un artefacto que la política tocaría en un pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPreviewItem {
    /// Nombre del paquete o de la imagen.
    pub name: String,
    /// Versión o etiqueta.
    pub version: String,
    /// `deny` o `warn`.
    pub effect: AdmissionEffect,
    /// Motivo legible.
    pub reason: String,
}

/// Resultado de simular la política contra el inventario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPreview {
    /// Artefactos que disparan la condición.
    pub matches: Vec<AdmissionPreviewItem>,
    /// Versiones listadas que no disparan la condición.
    pub allowed: usize,
}

/// Caso de uso: política de admisión de un repositorio.
#[derive(Clone)]
pub struct AdmissionService {
    store: Arc<dyn AdmissionStore>,
    repositories: Arc<dyn RepositoryStore>,
    list_artifacts: ListRepositoryArtifactsUseCase,
}

impl AdmissionService {
    /// Construye el servicio a partir de sus puertos.
    #[must_use]
    pub fn new(
        store: Arc<dyn AdmissionStore>,
        repositories: Arc<dyn RepositoryStore>,
        list_artifacts: ListRepositoryArtifactsUseCase,
    ) -> Self {
        Self {
            store,
            repositories,
            list_artifacts,
        }
    }

    /// Devuelve la política guardada, o la inactiva por defecto.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn get_policy(
        &self,
        repository_id: RepositoryId,
    ) -> Result<AdmissionPolicy, AdmissionError> {
        self.require_target(repository_id).await?;
        match self.store.find_by_repository(repository_id).await {
            Ok(policy) => Ok(policy),
            Err(AdmissionStoreError::MissingSchema) => Ok(AdmissionPolicy::inactive()),
            Err(err) => Err(err.into()),
        }
    }

    /// Persiste la política.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::AlloyRepository`] o un fallo de puerto.
    pub async fn save_policy(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
    ) -> Result<AdmissionPolicy, AdmissionError> {
        self.require_target(repository_id).await?;
        self.store.save(repository_id, policy).await?;
        Ok(policy)
    }

    /// Simula la política contra el inventario (ignora `enabled`).
    ///
    /// # Errors
    ///
    /// [`AdmissionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn dry_run(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
    ) -> Result<AdmissionPreview, AdmissionError> {
        self.require_target(repository_id).await?;
        let listed = self.list_artifacts.execute(repository_id).await?;
        Ok(preview_against(&listed, policy))
    }

    /// Últimos avisos y denegaciones del repositorio.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn list_events(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<AdmissionEvent>, AdmissionError> {
        self.require_target(repository_id).await?;
        match self.store.list_events(repository_id, 50).await {
            Ok(events) => Ok(events),
            Err(AdmissionStoreError::MissingSchema) => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    /// Deniega un pull si la política activa lo exige y deja constancia
    /// de avisos o denegaciones.
    ///
    /// # Errors
    ///
    /// [`PackagingError::PolicyDenied`] si hay que bloquear.
    pub async fn enforce_pull(
        &self,
        repository_id: RepositoryId,
        name: &str,
        reference: &str,
        signed: bool,
    ) -> Result<(), PackagingError> {
        let Ok(policy) = self.store.find_by_repository(repository_id).await else {
            return Ok(());
        };
        let Some(effect) = policy.apply_pull(signed) else {
            return Ok(());
        };
        let reason = match effect {
            AdmissionEffect::Deny => "no está firmada: se denegó el pull".to_string(),
            AdmissionEffect::Warn => "no está firmada: solo aviso, el pull siguió".to_string(),
        };
        let now = Utc::now();
        let recent = self
            .store
            .list_events(repository_id, 10)
            .await
            .unwrap_or_default();
        if !is_duplicate_pull_event(&recent, name, reference, effect, now) {
            let event = AdmissionEvent::from_parts(
                AdmissionEventId::new(),
                repository_id,
                name,
                reference,
                effect,
                reason,
                now.to_rfc3339_opts(SecondsFormat::Secs, true),
            );
            let _ = self.store.record_event(&event).await;
        }
        if matches!(effect, AdmissionEffect::Deny) {
            return Err(PackagingError::PolicyDenied(format!(
                "admission policy denies pull of {name}:{reference}: artifact is not signed"
            )));
        }
        Ok(())
    }

    async fn require_target(&self, repository_id: RepositoryId) -> Result<(), AdmissionError> {
        let Some(repository) = self.repositories.find_by_id(repository_id).await? else {
            return Err(AdmissionError::RepositoryNotFound(repository_id));
        };
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(AdmissionError::AlloyRepository);
        }
        if !matches!(repository.kind(), RepositoryKind::Forge)
            || !matches!(
                repository.ecosystem(),
                PackageEcosystem::Oci | PackageEcosystem::Helm
            )
        {
            return Err(AdmissionError::UnsupportedRepository);
        }
        Ok(())
    }
}

/// Docker pide el manifiesto por etiqueta y otra vez por digest (y a
/// menudo HEAD + GET). Sin esto, un pull deja dos filas idénticas.
const PULL_EVENT_DEDUPE_SECS: i64 = 15;

fn is_digest_reference(reference: &str) -> bool {
    reference.starts_with("sha256:")
}

fn is_duplicate_pull_event(
    existing: &[AdmissionEvent],
    name: &str,
    reference: &str,
    effect: AdmissionEffect,
    now: DateTime<Utc>,
) -> bool {
    existing.iter().any(|event| {
        if event.name() != name || event.effect() != effect {
            return false;
        }
        let Ok(created) = DateTime::parse_from_rfc3339(event.created_at()) else {
            return false;
        };
        let age = now.signed_duration_since(created.with_timezone(&Utc));
        if age.num_seconds() < 0 || age.num_seconds() > PULL_EVENT_DEDUPE_SECS {
            return false;
        }
        event.reference() == reference
            || is_digest_reference(reference)
            || is_digest_reference(event.reference())
    })
}

fn preview_against(listed: &[ListedArtifact], policy: AdmissionPolicy) -> AdmissionPreview {
    let mut matches = Vec::new();
    let mut allowed = 0_usize;
    for item in listed {
        let Some(name) = item.package_name() else {
            continue;
        };
        let Some(version) = item.package_version() else {
            continue;
        };
        match policy.preview_pull(item.signed()) {
            None => allowed += 1,
            Some(effect) => matches.push(AdmissionPreviewItem {
                name: name.to_string(),
                version: version.to_string(),
                effect,
                reason: match effect {
                    AdmissionEffect::Deny => "no está firmada: se denegaría el pull".to_string(),
                    AdmissionEffect::Warn => {
                        "no está firmada: solo aviso, el pull seguiría".to_string()
                    }
                },
            }),
        }
    }
    AdmissionPreview { matches, allowed }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ferrobox_domain::artifact::Artifact;
    use ferrobox_domain::checksum::Sha256Checksum;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
    use ferrobox_ports::admission_store::AdmissionStore;
    use ferrobox_ports::artifact_store::ArtifactStore;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::test_support::{
        InMemoryAdmissionStore, InMemoryArtifactStore, InMemoryPackageIndexStore,
        InMemoryRepositoryStore,
    };

    fn checksum(hex: &str) -> Sha256Checksum {
        Sha256Checksum::parse(hex).unwrap()
    }

    async fn forge_oci(repositories: &InMemoryRepositoryStore) -> Repository {
        let repository = Repository::new(
            RepositoryName::parse("oci-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repositories.save(&repository).await.unwrap();
        repository
    }

    fn service(
        store: Arc<InMemoryAdmissionStore>,
        repositories: Arc<InMemoryRepositoryStore>,
        artifacts: Arc<InMemoryArtifactStore>,
        index: Arc<InMemoryPackageIndexStore>,
    ) -> AdmissionService {
        AdmissionService::new(
            store,
            repositories.clone(),
            ListRepositoryArtifactsUseCase::new(repositories, artifacts, index),
        )
    }

    #[tokio::test]
    async fn dry_run_lists_unsigned_images() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let repository = forge_oci(&repositories).await;
        let hex = "ab".repeat(32);
        let image = Artifact::new(repository.id(), checksum(&hex), 80);
        artifacts.save(&image).await.unwrap();
        index
            .upsert_entry(
                repository.id(),
                &PackageCoordinate::new(
                    PackageEcosystem::Oci,
                    PackageName::parse("alpine").unwrap(),
                    PackageVersion::parse("latest").unwrap(),
                ),
                Some(image.id()),
                Bytes::from(
                    serde_json::json!({
                        "name": "alpine",
                        "reference": "latest",
                        "digest": format!("sha256:{hex}"),
                        "media_type": "application/vnd.oci.image.manifest.v1+json",
                        "size": 80,
                        "artifact_id": image.id().to_string()
                    })
                    .to_string(),
                ),
            )
            .await
            .unwrap();

        let preview = service(store, repositories, artifacts, index)
            .dry_run(
                repository.id(),
                AdmissionPolicy::parse(false, "pull", "not_signed", "deny").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert_eq!(preview.matches[0].name, "alpine");
        assert_eq!(preview.allowed, 0);
    }

    #[tokio::test]
    async fn enforce_blocks_only_when_enabled() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let repository = forge_oci(&repositories).await;
        let admission = service(store.clone(), repositories, artifacts, index);

        admission
            .enforce_pull(repository.id(), "alpine", "latest", false)
            .await
            .unwrap();

        store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
            )
            .await
            .unwrap();
        let error = admission
            .enforce_pull(repository.id(), "alpine", "latest", false)
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
        admission
            .enforce_pull(repository.id(), "alpine", "latest", true)
            .await
            .unwrap();
        let events = admission.list_events(repository.id()).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].effect(), AdmissionEffect::Deny);
        assert_eq!(events[0].name(), "alpine");
    }

    #[tokio::test]
    async fn warn_records_and_allows() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let repository = forge_oci(&repositories).await;
        store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "warn").unwrap(),
            )
            .await
            .unwrap();
        let admission = service(store, repositories, artifacts, index);
        admission
            .enforce_pull(repository.id(), "alpine", "latest", false)
            .await
            .unwrap();
        let events = admission.list_events(repository.id()).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].effect(), AdmissionEffect::Warn);
    }

    #[tokio::test]
    async fn enforce_records_one_event_for_tag_and_digest_of_the_same_pull() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let repository = forge_oci(&repositories).await;
        store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "warn").unwrap(),
            )
            .await
            .unwrap();
        let admission = service(store, repositories, artifacts, index);
        let digest = "sha256:cc58b463f0e3772e56c92408102e282ed4bbcb88eeb40bf7cc9a2ff1cc713562";
        admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false)
            .await
            .unwrap();
        admission
            .enforce_pull(repository.id(), "busybox", digest, false)
            .await
            .unwrap();
        admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false)
            .await
            .unwrap();
        let events = admission.list_events(repository.id()).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reference(), "unsigned");
        assert_eq!(events[0].effect(), AdmissionEffect::Warn);
    }

    #[tokio::test]
    async fn enforce_still_denies_on_a_duplicate_lookup() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let repository = forge_oci(&repositories).await;
        store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
            )
            .await
            .unwrap();
        let admission = service(store, repositories, artifacts, index);
        let first = admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false)
            .await
            .unwrap_err();
        let second = admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false)
            .await
            .unwrap_err();
        assert!(matches!(first, PackagingError::PolicyDenied(_)));
        assert!(matches!(second, PackagingError::PolicyDenied(_)));
        let events = admission.list_events(repository.id()).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn rejects_alloy_targets() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let member = forge_oci(&repositories).await;
        let alloy = Repository::new(
            RepositoryName::parse("oci-all").unwrap(),
            RepositoryKind::Alloy {
                members: vec![member.id()],
            },
            PackageEcosystem::Oci,
        )
        .unwrap();
        repositories.save(&alloy).await.unwrap();
        let error = service(
            Arc::new(InMemoryAdmissionStore::default()),
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
        )
        .get_policy(alloy.id())
        .await
        .unwrap_err();
        assert!(matches!(error, AdmissionError::AlloyRepository));
    }

    #[tokio::test]
    async fn rejects_non_oci_forges() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let cargo = Repository::new(
            RepositoryName::parse("crates-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&cargo).await.unwrap();
        let error = service(
            Arc::new(InMemoryAdmissionStore::default()),
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
        )
        .get_policy(cargo.id())
        .await
        .unwrap_err();
        assert!(matches!(error, AdmissionError::UnsupportedRepository));
    }
}
