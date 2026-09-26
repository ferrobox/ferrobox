//! Admission policy: persistence, dry-run, and deny/warn on pull and promote.

use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use ferrobox_domain::admission::{
    AdmissionEffect, AdmissionEvent, AdmissionFacts, AdmissionHit, AdmissionPolicy,
    AdmissionPolicyError,
};
use ferrobox_domain::assay::{Assay, AssaySeverity, AssayStatus};
use ferrobox_domain::ids::{AdmissionEventId, RepositoryId};
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::admission_store::{AdmissionRecord, AdmissionStore, AdmissionStoreError};
use ferrobox_ports::assay_store::AssayStore;
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

    /// El repositorio no admite política de admisión.
    #[error("admission policies apply to Forge and Mirror repositories")]
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
    assays: Option<Arc<dyn AssayStore>>,
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
            assays: None,
        }
    }

    /// Ensayes para evaluar CVE y licencia. Sin ellos, esas cláusulas
    /// no disparan (fail-open).
    #[must_use]
    pub fn with_assays(mut self, assays: Arc<dyn AssayStore>) -> Self {
        self.assays = Some(assays);
        self
    }

    /// Devuelve la política guardada, o la inactiva por defecto.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::RepositoryNotFound`] o un fallo de puerto.
    pub async fn get_policy(
        &self,
        repository_id: RepositoryId,
    ) -> Result<AdmissionRecord, AdmissionError> {
        self.require_target(repository_id).await?;
        match self.store.find_by_repository(repository_id).await {
            Ok(record) => Ok(record),
            Err(AdmissionStoreError::MissingSchema) => Ok(AdmissionRecord {
                policy: AdmissionPolicy::inactive(),
                public_keys_pem: String::new(),
            }),
            Err(err) => Err(err.into()),
        }
    }

    /// PEM de claves Cosign del repositorio. Vacío si no hay o falla.
    pub async fn public_keys_pem(&self, repository_id: RepositoryId) -> String {
        self.store
            .find_by_repository(repository_id)
            .await
            .map(|record| record.public_keys_pem)
            .unwrap_or_default()
    }

    /// Persiste la política y las claves Cosign.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::AlloyRepository`] o un fallo de puerto.
    pub async fn save_policy(
        &self,
        repository_id: RepositoryId,
        policy: AdmissionPolicy,
        public_keys_pem: impl Into<String>,
    ) -> Result<AdmissionRecord, AdmissionError> {
        self.require_target(repository_id).await?;
        let public_keys_pem = public_keys_pem.into();
        self.store
            .save(repository_id, policy.clone(), &public_keys_pem)
            .await?;
        Ok(AdmissionRecord {
            policy,
            public_keys_pem,
        })
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
        public_keys_pem: impl Into<String>,
    ) -> Result<AdmissionPreview, AdmissionError> {
        self.require_target(repository_id).await?;
        let listed = self
            .list_artifacts
            .execute_with_keys(repository_id, &public_keys_pem.into())
            .await?;
        let assays = self.load_assays(repository_id).await;
        let consider_signature = self.considers_signature(repository_id).await;
        Ok(preview_against(
            &listed,
            &policy,
            &assays,
            consider_signature,
        ))
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
        verified: bool,
    ) -> Result<(), PackagingError> {
        let Ok(record) = self.store.find_by_repository(repository_id).await else {
            return Ok(());
        };
        let policy = record.policy;
        let facts = self
            .facts_for(repository_id, name, reference, signed, verified)
            .await;
        let Some((effect, hit)) = policy.apply_facts(&facts) else {
            return Ok(());
        };
        let reason = hit_reason(&hit, effect, true);
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
                "admission policy denies pull of {name}:{reference}: {}",
                hit_reason(&hit, effect, true)
            )));
        }
        Ok(())
    }

    /// Como [`Self::enforce_pull`], para un paquete sin firma Cosign.
    ///
    /// # Errors
    ///
    /// [`PackagingError::PolicyDenied`] si hay que bloquear.
    pub async fn enforce_package_pull(
        &self,
        repository_id: RepositoryId,
        name: &str,
        version: &str,
    ) -> Result<(), PackagingError> {
        self.enforce_pull(repository_id, name, version, true, true)
            .await
    }

    /// Evaluates the **target** policy against facts from the **source**
    /// package before a promote. Same clauses as pull; fail-open without
    /// a ready assay. OCI/Helm signatures use the target's Cosign keys.
    ///
    /// # Errors
    ///
    /// [`PackagingError::PolicyDenied`] if the target policy denies.
    pub async fn enforce_promote(
        &self,
        target_id: RepositoryId,
        source_id: RepositoryId,
        name: &str,
        reference: &str,
    ) -> Result<(), PackagingError> {
        let Ok(record) = self.store.find_by_repository(target_id).await else {
            return Ok(());
        };
        let policy = record.policy;
        let consider_signature = self.considers_signature(source_id).await;
        let (signed, verified) = if consider_signature {
            self.signature_facts(source_id, name, reference, &record.public_keys_pem)
                .await
        } else {
            (true, true)
        };
        let assay = self.find_assay(source_id, name, reference).await;
        let facts = facts_from_assay(assay.as_ref(), signed, verified, consider_signature);
        let Some((effect, hit)) = policy.apply_facts(&facts) else {
            return Ok(());
        };
        let reason = hit_reason_op(&hit, effect, true, "promote");
        let now = Utc::now();
        let recent = self
            .store
            .list_events(target_id, 10)
            .await
            .unwrap_or_default();
        if !is_duplicate_pull_event(&recent, name, reference, effect, now) {
            let event = AdmissionEvent::from_parts(
                AdmissionEventId::new(),
                target_id,
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
                "admission policy denies promote of {name}:{reference}: {}",
                hit_reason_op(&hit, effect, true, "promote")
            )));
        }
        Ok(())
    }

    async fn signature_facts(
        &self,
        source_id: RepositoryId,
        name: &str,
        reference: &str,
        public_keys_pem: &str,
    ) -> (bool, bool) {
        let Ok(listed) = self
            .list_artifacts
            .execute_with_keys(source_id, public_keys_pem)
            .await
        else {
            return (true, true);
        };
        listed
            .iter()
            .find(|item| {
                item.package_name() == Some(name) && item.package_version() == Some(reference)
            })
            .map_or((false, false), |item| (item.signed(), item.verified()))
    }

    async fn require_target(&self, repository_id: RepositoryId) -> Result<(), AdmissionError> {
        let Some(repository) = self.repositories.find_by_id(repository_id).await? else {
            return Err(AdmissionError::RepositoryNotFound(repository_id));
        };
        if matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(AdmissionError::AlloyRepository);
        }
        if !matches!(
            repository.kind(),
            RepositoryKind::Forge | RepositoryKind::Mirror { .. }
        ) {
            return Err(AdmissionError::UnsupportedRepository);
        }
        Ok(())
    }

    async fn considers_signature(&self, repository_id: RepositoryId) -> bool {
        let Ok(Some(repository)) = self.repositories.find_by_id(repository_id).await else {
            return false;
        };
        matches!(
            repository.ecosystem(),
            PackageEcosystem::Oci | PackageEcosystem::Helm
        )
    }

    async fn load_assays(&self, repository_id: RepositoryId) -> Vec<Assay> {
        let Some(assays) = &self.assays else {
            return Vec::new();
        };
        assays
            .find_by_repository(repository_id)
            .await
            .unwrap_or_default()
    }

    async fn facts_for(
        &self,
        repository_id: RepositoryId,
        name: &str,
        reference: &str,
        signed: bool,
        verified: bool,
    ) -> AdmissionFacts {
        let consider_signature = self.considers_signature(repository_id).await;
        let assay = self.find_assay(repository_id, name, reference).await;
        facts_from_assay(assay.as_ref(), signed, verified, consider_signature)
    }

    async fn find_assay(
        &self,
        repository_id: RepositoryId,
        name: &str,
        reference: &str,
    ) -> Option<Assay> {
        let assays = self.assays.as_ref()?;
        let repository = self.repositories.find_by_id(repository_id).await.ok()??;
        let coordinate = PackageCoordinate::new(
            repository.ecosystem(),
            PackageName::parse(name).ok()?,
            PackageVersion::parse(reference).ok()?,
        );
        assays
            .find_by_coordinate(repository_id, &coordinate)
            .await
            .ok()
            .flatten()
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

fn preview_against(
    listed: &[ListedArtifact],
    policy: &AdmissionPolicy,
    assays: &[Assay],
    consider_signature: bool,
) -> AdmissionPreview {
    let mut matches = Vec::new();
    let mut allowed = 0_usize;
    let mut seen = std::collections::HashSet::<(String, String)>::new();
    for item in listed {
        let Some(name) = item.package_name() else {
            continue;
        };
        let Some(version) = item.package_version() else {
            continue;
        };
        if !seen.insert((name.to_string(), version.to_string())) {
            continue;
        }
        let assay = assays.iter().find(|assay| {
            assay.coordinate().name().as_str() == name
                && assay.coordinate().version().as_str() == version
        });
        let facts = facts_from_assay(assay, item.signed(), item.verified(), consider_signature);
        match policy.preview_facts(&facts) {
            None => allowed += 1,
            Some(hit) => matches.push(AdmissionPreviewItem {
                name: name.to_string(),
                version: version.to_string(),
                effect: policy.effect(),
                reason: hit_reason(&hit, policy.effect(), false),
            }),
        }
    }
    AdmissionPreview { matches, allowed }
}

fn facts_from_assay(
    assay: Option<&Assay>,
    signed: bool,
    verified: bool,
    consider_signature: bool,
) -> AdmissionFacts {
    let (max_finding, licenses) = match assay {
        Some(assay) if assay.status() == AssayStatus::Ready => {
            let max_finding = assay
                .findings()
                .iter()
                .map(ferrobox_domain::assay::AssayFinding::severity)
                .filter(|severity| !matches!(severity, AssaySeverity::Unknown))
                .min();
            let licenses = assay
                .components()
                .iter()
                .flat_map(|component| component.licenses().iter().cloned())
                .collect();
            (max_finding, licenses)
        }
        _ => (None, Vec::new()),
    };
    AdmissionFacts {
        signed,
        verified,
        consider_signature,
        max_finding,
        licenses,
    }
}

fn hit_reason(hit: &AdmissionHit, effect: AdmissionEffect, applied: bool) -> String {
    hit_reason_op(hit, effect, applied, "pull")
}

fn hit_reason_op(
    hit: &AdmissionHit,
    effect: AdmissionEffect,
    applied: bool,
    operation: &str,
) -> String {
    let outcome = match (effect, applied) {
        (AdmissionEffect::Deny, true) => format!("se denegó el {operation}"),
        (AdmissionEffect::Deny, false) => format!("se denegaría el {operation}"),
        (AdmissionEffect::Warn, true) => format!("solo aviso, el {operation} siguió"),
        (AdmissionEffect::Warn, false) => format!("solo aviso, el {operation} seguiría"),
    };
    match hit {
        AdmissionHit::Unsigned => format!("no está firmada: {outcome}"),
        AdmissionHit::Unverified => format!("no está verificada: {outcome}"),
        AdmissionHit::Finding(severity) => {
            format!("hallazgo {}: {outcome}", severity.as_str())
        }
        AdmissionHit::ForbiddenLicense(license) => {
            format!("licencia {license}: {outcome}")
        }
    }
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
    use ferrobox_ports::assay_store::AssayStore;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use ferrobox_ports::repository_store::RepositoryStore;

    use super::*;
    use crate::test_support::{
        InMemoryAdmissionStore, InMemoryArtifactStore, InMemoryAssayStore,
        InMemoryPackageIndexStore, InMemoryRepositoryStore,
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

    fn service_with_assays(
        store: Arc<InMemoryAdmissionStore>,
        repositories: Arc<InMemoryRepositoryStore>,
        artifacts: Arc<InMemoryArtifactStore>,
        index: Arc<InMemoryPackageIndexStore>,
        assays: Arc<InMemoryAssayStore>,
    ) -> AdmissionService {
        service(store, repositories, artifacts, index).with_assays(assays)
    }

    async fn forge_cargo(repositories: &InMemoryRepositoryStore) -> Repository {
        let repository = Repository::new(
            RepositoryName::parse("crates-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&repository).await.unwrap();
        repository
    }

    async fn index_crate(
        repository_id: ferrobox_domain::ids::RepositoryId,
        artifacts: &InMemoryArtifactStore,
        index: &InMemoryPackageIndexStore,
        name: &str,
        version: &str,
    ) {
        let hex = "cd".repeat(32);
        let crate_file = Artifact::new(repository_id, checksum(&hex), 64);
        artifacts.save(&crate_file).await.unwrap();
        index
            .upsert_entry(
                repository_id,
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse(name).unwrap(),
                    PackageVersion::parse(version).unwrap(),
                ),
                Some(crate_file.id()),
                Bytes::from("{}"),
            )
            .await
            .unwrap();
    }

    fn ready_assay(
        repository_id: ferrobox_domain::ids::RepositoryId,
        name: &str,
        version: &str,
        status: AssayStatus,
        finding: Option<AssaySeverity>,
        licenses: &[&str],
    ) -> Assay {
        use ferrobox_domain::assay::{AssayComponent, AssayComponentKind, AssayFinding};
        use ferrobox_domain::ids::AssayId;

        let mut root = AssayComponent::new(name, version, None, AssayComponentKind::Root);
        root.add_licenses(licenses.iter().map(|item| (*item).to_string()));
        let findings = finding
            .map(|severity| {
                vec![AssayFinding::new(
                    "RUSTSEC-0000-0001",
                    Vec::new(),
                    "test finding",
                    severity,
                    name,
                    version,
                    None,
                    None,
                )]
            })
            .unwrap_or_default();
        Assay::from_parts(
            AssayId::new(),
            repository_id,
            PackageCoordinate::new(
                PackageEcosystem::Cargo,
                PackageName::parse(name).unwrap(),
                PackageVersion::parse(version).unwrap(),
            ),
            status,
            Some("2026-09-26T00:00:00Z".to_string()),
            None,
            vec![root],
            findings,
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
                "",
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
            .enforce_pull(repository.id(), "alpine", "latest", false, false)
            .await
            .unwrap();

        store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
                "",
            )
            .await
            .unwrap();
        let error = admission
            .enforce_pull(repository.id(), "alpine", "latest", false, false)
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
        admission
            .enforce_pull(repository.id(), "alpine", "latest", true, false)
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
                "",
            )
            .await
            .unwrap();
        let admission = service(store, repositories, artifacts, index);
        admission
            .enforce_pull(repository.id(), "alpine", "latest", false, false)
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
                "",
            )
            .await
            .unwrap();
        let admission = service(store, repositories, artifacts, index);
        let digest = "sha256:cc58b463f0e3772e56c92408102e282ed4bbcb88eeb40bf7cc9a2ff1cc713562";
        admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false, false)
            .await
            .unwrap();
        admission
            .enforce_pull(repository.id(), "busybox", digest, false, false)
            .await
            .unwrap();
        admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false, false)
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
                "",
            )
            .await
            .unwrap();
        let admission = service(store, repositories, artifacts, index);
        let first = admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false, false)
            .await
            .unwrap_err();
        let second = admission
            .enforce_pull(repository.id(), "busybox", "unsigned", false, false)
            .await
            .unwrap_err();
        assert!(matches!(first, PackagingError::PolicyDenied(_)));
        assert!(matches!(second, PackagingError::PolicyDenied(_)));
        let events = admission.list_events(repository.id()).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn enforce_not_verified_ignores_detect_only() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let repository = forge_oci(&repositories).await;
        store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_verified", "deny").unwrap(),
                "",
            )
            .await
            .unwrap();
        let admission = service(store, repositories, artifacts, index);
        let error = admission
            .enforce_pull(repository.id(), "alpine", "latest", true, false)
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
        admission
            .enforce_pull(repository.id(), "alpine", "latest", true, true)
            .await
            .unwrap();
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
    async fn accepts_cargo_forges() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let cargo = Repository::new(
            RepositoryName::parse("crates-local").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&cargo).await.unwrap();
        let record = service(
            Arc::new(InMemoryAdmissionStore::default()),
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
        )
        .get_policy(cargo.id())
        .await
        .unwrap();
        assert!(!record.policy.enabled());
    }

    #[tokio::test]
    async fn accepts_oci_mirrors() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let mirror = Repository::new(
            RepositoryName::parse("oci-proxy").unwrap(),
            RepositoryKind::Mirror {
                upstream: url::Url::parse("https://registry-1.docker.io").unwrap(),
            },
            PackageEcosystem::Oci,
        )
        .unwrap();
        repositories.save(&mirror).await.unwrap();
        let policy = service(
            Arc::new(InMemoryAdmissionStore::default()),
            repositories,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
        )
        .save_policy(
            mirror.id(),
            AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
            "",
        )
        .await
        .unwrap();
        assert!(policy.policy.enabled());
    }

    #[tokio::test]
    async fn cargo_signature_policy_does_not_block_downloads() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let repository = forge_cargo(&repositories).await;
        store
            .save(
                repository.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
                "",
            )
            .await
            .unwrap();
        service(store, repositories, artifacts, index)
            .enforce_package_pull(repository.id(), "libc", "0.2.177")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dry_run_lists_ready_assay_findings() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let assays = Arc::new(InMemoryAssayStore::default());
        let repository = forge_cargo(&repositories).await;
        index_crate(repository.id(), &artifacts, &index, "libc", "0.2.177").await;
        assays
            .upsert(&ready_assay(
                repository.id(),
                "libc",
                "0.2.177",
                AssayStatus::Ready,
                Some(AssaySeverity::High),
                &["MIT"],
            ))
            .await
            .unwrap();

        let preview = service_with_assays(
            Arc::new(InMemoryAdmissionStore::default()),
            repositories,
            artifacts,
            index,
            assays,
        )
        .dry_run(
            repository.id(),
            AdmissionPolicy::profile_openchain_security(),
            "",
        )
        .await
        .unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert_eq!(preview.matches[0].name, "libc");
        assert_eq!(preview.matches[0].effect, AdmissionEffect::Warn);
        assert!(preview.matches[0].reason.contains("hallazgo high"));
        assert_eq!(preview.allowed, 0);
    }

    #[tokio::test]
    async fn dry_run_fails_open_without_a_ready_assay() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let assays = Arc::new(InMemoryAssayStore::default());
        let repository = forge_cargo(&repositories).await;
        index_crate(repository.id(), &artifacts, &index, "libc", "0.2.177").await;
        assays
            .upsert(&ready_assay(
                repository.id(),
                "libc",
                "0.2.177",
                AssayStatus::Running,
                Some(AssaySeverity::Critical),
                &["GPL-3.0-only"],
            ))
            .await
            .unwrap();

        let preview = service_with_assays(
            Arc::new(InMemoryAdmissionStore::default()),
            repositories,
            artifacts,
            index,
            assays,
        )
        .dry_run(
            repository.id(),
            AdmissionPolicy::profile_openchain_security(),
            "",
        )
        .await
        .unwrap();
        assert!(preview.matches.is_empty());
        assert_eq!(preview.allowed, 1);
    }

    #[tokio::test]
    async fn enforce_denies_forbidden_license_on_cargo() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let assays = Arc::new(InMemoryAssayStore::default());
        let repository = forge_cargo(&repositories).await;
        assays
            .upsert(&ready_assay(
                repository.id(),
                "libc",
                "0.2.177",
                AssayStatus::Ready,
                None,
                &["AGPL-3.0-only"],
            ))
            .await
            .unwrap();
        let mut policy = AdmissionPolicy::profile_copyleft_restrict();
        policy = AdmissionPolicy::compose(
            true,
            policy.when().as_str(),
            policy.effect().as_str(),
            policy.clauses().clone(),
        )
        .unwrap();
        store.save(repository.id(), policy, "").await.unwrap();
        let error = service_with_assays(store, repositories, artifacts, index, assays)
            .enforce_package_pull(repository.id(), "libc", "0.2.177")
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
    }

    fn enabled_copyleft() -> AdmissionPolicy {
        let policy = AdmissionPolicy::profile_copyleft_restrict();
        AdmissionPolicy::compose(
            true,
            policy.when().as_str(),
            policy.effect().as_str(),
            policy.clauses().clone(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn promote_uses_target_policy_and_source_assay() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let assays = Arc::new(InMemoryAssayStore::default());
        let source = forge_cargo(&repositories).await;
        let target = Repository::new(
            RepositoryName::parse("crates-prod").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&target).await.unwrap();
        assays
            .upsert(&ready_assay(
                source.id(),
                "libc",
                "0.2.177",
                AssayStatus::Ready,
                None,
                &["AGPL-3.0-only"],
            ))
            .await
            .unwrap();
        store
            .save(target.id(), enabled_copyleft(), "")
            .await
            .unwrap();

        let error = service_with_assays(store.clone(), repositories, artifacts, index, assays)
            .enforce_promote(target.id(), source.id(), "libc", "0.2.177")
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
        assert!(error.to_string().contains("promote"));

        let events = store.list_events(target.id(), 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].reason().contains("promote"));
    }

    #[tokio::test]
    async fn promote_fails_open_without_a_ready_assay() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let assays = Arc::new(InMemoryAssayStore::default());
        let source = forge_cargo(&repositories).await;
        let target = Repository::new(
            RepositoryName::parse("crates-prod").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap();
        repositories.save(&target).await.unwrap();
        store
            .save(target.id(), enabled_copyleft(), "")
            .await
            .unwrap();

        service_with_assays(store, repositories, artifacts, index, assays)
            .enforce_promote(target.id(), source.id(), "libc", "0.2.177")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn promote_denies_unsigned_oci_against_target_keys() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let source = forge_oci(&repositories).await;
        let target = Repository::new(
            RepositoryName::parse("oci-prod").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repositories.save(&target).await.unwrap();
        let hex = "ab".repeat(32);
        let image = Artifact::new(source.id(), checksum(&hex), 80);
        artifacts.save(&image).await.unwrap();
        index
            .upsert_entry(
                source.id(),
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
        store
            .save(
                target.id(),
                AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap(),
                "",
            )
            .await
            .unwrap();

        let error = service(store, repositories, artifacts, index)
            .enforce_promote(target.id(), source.id(), "alpine", "latest")
            .await
            .unwrap_err();
        assert!(matches!(error, PackagingError::PolicyDenied(_)));
    }
}
