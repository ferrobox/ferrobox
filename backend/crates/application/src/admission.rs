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

use crate::assay::VulnerabilityFeedSource;
use crate::list_repository_artifacts::{
    ListRepositoryArtifactsError, ListRepositoryArtifactsUseCase, ListedArtifact,
};
use crate::packaging::PackagingError;

/// Reasons querying or saving the admission policy can fail.
#[derive(Debug, Error)]
pub enum AdmissionError {
    /// The repository does not exist.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// An `Alloy` has no policy of its own: its members' policies apply.
    #[error("alloy repositories do not have their own admission policy")]
    AlloyRepository,

    /// The repository does not support an admission policy.
    #[error("admission policies apply to Forge and Mirror repositories")]
    UnsupportedRepository,

    /// The submitted policy is not valid.
    #[error(transparent)]
    InvalidPolicy(#[from] AdmissionPolicyError),

    /// Failed to read or write the policy.
    #[error(transparent)]
    Store(#[from] AdmissionStoreError),

    /// Failed to list artifacts for the dry-run.
    #[error(transparent)]
    Artifacts(#[from] ListRepositoryArtifactsError),

    /// Failed to query repositories.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),
}

/// An artifact the policy would touch on a pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPreviewItem {
    /// Package or image name.
    pub name: String,
    /// Version or tag.
    pub version: String,
    /// `deny` or `warn`.
    pub effect: AdmissionEffect,
    /// Human-readable reason.
    pub reason: String,
}

/// Result of simulating the policy against the inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPreview {
    /// Artifacts that trigger the condition.
    pub matches: Vec<AdmissionPreviewItem>,
    /// Listed versions that do not trigger the condition.
    pub allowed: usize,
}

/// Use case: admission policy of a repository.
#[derive(Clone)]
pub struct AdmissionService {
    store: Arc<dyn AdmissionStore>,
    repositories: Arc<dyn RepositoryStore>,
    list_artifacts: ListRepositoryArtifactsUseCase,
    assays: Option<Arc<dyn AssayStore>>,
    /// Local vulnerability index. Absent in tests that do not arm a CVE clause.
    vulnerability_feed: Option<Arc<dyn VulnerabilityFeedSource>>,
}

impl AdmissionService {
    /// Builds the service from its ports.
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
            vulnerability_feed: None,
        }
    }

    /// Assays for evaluating CVE and license. A license clause without
    /// a ready assay does not fire. A CVE clause fails closed: see
    /// [`Self::with_vulnerability_feed`].
    #[must_use]
    pub fn with_assays(mut self, assays: Arc<dyn AssayStore>) -> Self {
        self.assays = Some(assays);
        self
    }

    /// Local index used to decide whether a CVE clause can be evaluated.
    ///
    /// When the clause is armed and this reports nothing loaded, pull
    /// and promote are denied. An assay that is not `Ready` is denied
    /// the same way. With the clause off, a missing assay still allows
    /// the operation.
    #[must_use]
    pub fn with_vulnerability_feed(mut self, feed: Arc<dyn VulnerabilityFeedSource>) -> Self {
        self.vulnerability_feed = Some(feed);
        self
    }

    /// Returns the saved policy, or the inactive default.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::RepositoryNotFound`] or a port failure.
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

    /// PEM of the repository Cosign keys. Empty if none or on failure.
    pub async fn public_keys_pem(&self, repository_id: RepositoryId) -> String {
        self.store
            .find_by_repository(repository_id)
            .await
            .map(|record| record.public_keys_pem)
            .unwrap_or_default()
    }

    /// Persists the policy and the Cosign keys.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::AlloyRepository`] or a port failure.
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

    /// Simulates the policy against the inventory (ignores `enabled`).
    ///
    /// # Errors
    ///
    /// [`AdmissionError::RepositoryNotFound`] or a port failure.
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
            self.vulnerability_feed_loaded(),
        ))
    }

    /// Latest warnings and denials of the repository.
    ///
    /// # Errors
    ///
    /// [`AdmissionError::RepositoryNotFound`] or a port failure.
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

    /// Denies a pull if the active policy requires it and records
    /// warnings or denials.
    ///
    /// # Errors
    ///
    /// [`PackagingError::PolicyDenied`] if it must be blocked.
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
        if let Some(reason) = self
            .cve_unavailable(repository_id, &policy, name, reference, "pull")
            .await
        {
            return self
                .finish_decision(
                    repository_id,
                    name,
                    reference,
                    AdmissionEffect::Deny,
                    reason,
                    "pull",
                )
                .await;
        }
        let facts = self
            .facts_for(repository_id, name, reference, signed, verified)
            .await;
        let Some((effect, hit)) = policy.apply_facts(&facts) else {
            return Ok(());
        };
        let reason = hit_reason(&hit, effect, true);
        self.finish_decision(repository_id, name, reference, effect, reason, "pull")
            .await
    }

    /// Like [`Self::enforce_pull`], for a package without a Cosign signature.
    ///
    /// # Errors
    ///
    /// [`PackagingError::PolicyDenied`] if it must be blocked.
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
    /// package before a promote. Same clauses as pull. OCI/Helm
    /// signatures use the target's Cosign keys. A CVE clause denies
    /// when the feed is missing or the source assay is not ready.
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
        if let Some(reason) = self
            .cve_unavailable(source_id, &policy, name, reference, "promote")
            .await
        {
            return self
                .finish_decision(
                    target_id,
                    name,
                    reference,
                    AdmissionEffect::Deny,
                    reason,
                    "promote",
                )
                .await;
        }
        let assay = self.find_assay(source_id, name, reference).await;
        let facts = facts_from_assay(assay.as_ref(), signed, verified, consider_signature);
        let Some((effect, hit)) = policy.apply_facts(&facts) else {
            return Ok(());
        };
        let reason = hit_reason_op(&hit, effect, true, "promote");
        self.finish_decision(target_id, name, reference, effect, reason, "promote")
            .await
    }

    fn vulnerability_feed_loaded(&self) -> bool {
        self.vulnerability_feed
            .as_ref()
            .is_some_and(|feed| feed.vulnerability_feed_loaded())
    }

    /// CVE clause armed, but the index or the assay cannot support it.
    ///
    /// `None` means the caller should evaluate the clauses as usual.
    async fn cve_unavailable(
        &self,
        assay_repository_id: RepositoryId,
        policy: &AdmissionPolicy,
        name: &str,
        reference: &str,
        operation: &str,
    ) -> Option<String> {
        if !policy.enabled() || policy.clauses().min_finding().is_none() {
            return None;
        }
        if !self.vulnerability_feed_loaded() {
            return Some(format!("feed missing: se denegó el {operation}"));
        }
        let assay = self.find_assay(assay_repository_id, name, reference).await;
        if assay
            .as_ref()
            .is_some_and(|assay| assay.status() == AssayStatus::Ready)
        {
            return None;
        }
        Some(format!("el assay no está listo: se denegó el {operation}"))
    }

    async fn finish_decision(
        &self,
        repository_id: RepositoryId,
        name: &str,
        reference: &str,
        effect: AdmissionEffect,
        reason: String,
        operation: &str,
    ) -> Result<(), PackagingError> {
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
                reason.clone(),
                now.to_rfc3339_opts(SecondsFormat::Secs, true),
            );
            let _ = self.store.record_event(&event).await;
        }
        if matches!(effect, AdmissionEffect::Deny) {
            return Err(PackagingError::PolicyDenied(format!(
                "admission policy denies {operation} of {name}:{reference}: {reason}"
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

/// Docker requests the manifest by tag and again by digest (and often
/// HEAD + GET). Without this, a pull leaves two identical rows.
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
    feed_loaded: bool,
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
        if policy.clauses().min_finding().is_some() && !feed_loaded {
            matches.push(AdmissionPreviewItem {
                name: name.to_string(),
                version: version.to_string(),
                effect: AdmissionEffect::Deny,
                reason: "feed missing: se denegaría el pull".to_string(),
            });
            continue;
        }
        if policy.clauses().min_finding().is_some()
            && assay.is_none_or(|found| found.status() != AssayStatus::Ready)
        {
            matches.push(AdmissionPreviewItem {
                name: name.to_string(),
                version: version.to_string(),
                effect: AdmissionEffect::Deny,
                reason: "el assay no está listo: se denegaría el pull".to_string(),
            });
            continue;
        }
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

    use ferrobox_domain::admission::AdmissionClauses;

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

    struct StaticFeed(bool);

    impl VulnerabilityFeedSource for StaticFeed {
        fn vulnerability_feed_loaded(&self) -> bool {
            self.0
        }
    }

    fn cve_policy(enabled: bool) -> AdmissionPolicy {
        AdmissionPolicy::compose(
            enabled,
            "pull",
            "deny",
            AdmissionClauses::new(false, false, Some(AssaySeverity::High), Vec::new(), None),
        )
        .unwrap()
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
        .with_vulnerability_feed(Arc::new(StaticFeed(true)))
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
    async fn dry_run_denies_a_cve_policy_when_the_feed_is_missing() {
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
        assert_eq!(preview.matches.len(), 1);
        assert_eq!(preview.matches[0].effect, AdmissionEffect::Deny);
        assert!(preview.matches[0].reason.contains("feed missing"));
        assert!(!preview.matches[0].reason.contains("hallazgo"));
        assert_eq!(preview.allowed, 0);
    }

    #[tokio::test]
    async fn cve_policy_off_allows_pull_without_a_feed() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let assays = Arc::new(InMemoryAssayStore::default());
        let repository = forge_cargo(&repositories).await;
        store
            .save(repository.id(), cve_policy(false), "")
            .await
            .unwrap();
        service_with_assays(store, repositories, artifacts, index, assays)
            .enforce_package_pull(repository.id(), "libc", "0.2.177")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cve_policy_denies_pull_when_the_feed_is_missing() {
        let repositories = Arc::new(InMemoryRepositoryStore::default());
        let artifacts = Arc::new(InMemoryArtifactStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let store = Arc::new(InMemoryAdmissionStore::default());
        let assays = Arc::new(InMemoryAssayStore::default());
        let repository = forge_cargo(&repositories).await;
        store
            .save(repository.id(), cve_policy(true), "")
            .await
            .unwrap();
        let error = service_with_assays(store, repositories, artifacts, index, assays)
            .enforce_package_pull(repository.id(), "libc", "0.2.177")
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("feed missing"), "{message}");
        assert!(!message.contains("hallazgo"), "{message}");
        assert!(!message.to_ascii_lowercase().contains("cve"), "{message}");
    }

    #[tokio::test]
    async fn cve_policy_denies_a_ready_finding_when_the_feed_is_loaded() {
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
                Some(AssaySeverity::High),
                &["MIT"],
            ))
            .await
            .unwrap();
        store
            .save(repository.id(), cve_policy(true), "")
            .await
            .unwrap();
        let error = service_with_assays(store, repositories, artifacts, index, assays)
            .with_vulnerability_feed(Arc::new(StaticFeed(true)))
            .enforce_package_pull(repository.id(), "libc", "0.2.177")
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("hallazgo high"), "{message}");
    }

    #[tokio::test]
    async fn cve_policy_allows_a_clean_assay_when_the_feed_is_loaded() {
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
                &["MIT"],
            ))
            .await
            .unwrap();
        store
            .save(repository.id(), cve_policy(true), "")
            .await
            .unwrap();
        service_with_assays(store, repositories, artifacts, index, assays)
            .with_vulnerability_feed(Arc::new(StaticFeed(true)))
            .enforce_package_pull(repository.id(), "libc", "0.2.177")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cve_policy_denies_when_the_assay_is_not_ready() {
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
                AssayStatus::Failed,
                None,
                &["MIT"],
            ))
            .await
            .unwrap();
        store
            .save(repository.id(), cve_policy(true), "")
            .await
            .unwrap();
        let error = service_with_assays(store, repositories, artifacts, index, assays)
            .with_vulnerability_feed(Arc::new(StaticFeed(true)))
            .enforce_package_pull(repository.id(), "libc", "0.2.177")
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("el assay no está listo"), "{message}");
        assert!(!message.contains("feed missing"), "{message}");
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
