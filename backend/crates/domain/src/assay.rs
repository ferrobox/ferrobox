//! Assay of a package: composition (inventory) and impurities
//! (known vulnerabilities).

use crate::ids::{AssayId, RepositoryId};
use crate::package_coordinate::PackageCoordinate;

/// Result of an assay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssayStatus {
    /// Inventory and vulnerability lookup completed.
    Ready,
    /// The inventory was obtained, but the vulnerability lookup failed.
    Failed,
    /// This ecosystem does not yet support assay.
    Unsupported,
    /// Queued or running; the previous inventory is kept.
    Running,
}

impl AssayStatus {
    /// Stable label used in persistence and in the HTTP API.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
            Self::Running => "running",
        }
    }

    /// Parses the stable label.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            "unsupported" => Some(Self::Unsupported),
            "running" => Some(Self::Running),
            _ => None,
        }
    }
}

/// Severity of a finding, aligned with the usual
/// CVSS (*Common Vulnerability Scoring System*) scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssaySeverity {
    /// 9.0 or more, or the `CRITICAL` label.
    Critical,
    /// 7.0–8.9, or the `HIGH` label.
    High,
    /// 4.0–6.9, or the `MEDIUM` / `MODERATE` label.
    Medium,
    /// 0.1–3.9, or the `LOW` label.
    Low,
    /// No known score.
    Unknown,
}

impl AssaySeverity {
    /// Stable label used in persistence and in the HTTP API.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unknown => "unknown",
        }
    }

    /// Parses the stable label or an advisory label (`CRITICAL`…).
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" | "moderate" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Unknown,
        }
    }

    /// `true` if this severity meets the threshold (Critical is the most severe).
    ///
    /// `Unknown` does not trip any threshold: without a score nothing is blocked.
    #[must_use]
    pub fn meets_threshold(self, threshold: Self) -> bool {
        !matches!(self, Self::Unknown) && self <= threshold
    }

    /// Translates a CVSS 3.x score (0–10) to a severity.
    #[must_use]
    pub fn from_cvss_score(score: f64) -> Self {
        if score >= 9.0 {
            Self::Critical
        } else if score >= 7.0 {
            Self::High
        } else if score >= 4.0 {
            Self::Medium
        } else if score > 0.0 {
            Self::Low
        } else {
            Self::Unknown
        }
    }
}

/// Role of a component in the inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssayComponentKind {
    /// The assayed package.
    Root,
    /// First-level declared dependency.
    Direct,
    /// Dependency resolved from a lockfile (not declared in the manifest).
    Transitive,
}

impl AssayComponentKind {
    /// Stable label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Direct => "direct",
            Self::Transitive => "transitive",
        }
    }

    /// Parses the stable label.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "root" => Self::Root,
            "transitive" => Self::Transitive,
            _ => Self::Direct,
        }
    }
}

/// A component of the assay inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssayComponent {
    name: String,
    version: String,
    purl: Option<String>,
    kind: AssayComponentKind,
    licenses: Vec<String>,
}

impl AssayComponent {
    /// Builds an inventory component.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        purl: Option<String>,
        kind: AssayComponentKind,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            purl,
            kind,
            licenses: Vec::new(),
        }
    }

    /// Adds declared licenses, without duplicates.
    pub fn add_licenses(&mut self, licenses: impl IntoIterator<Item = String>) {
        for license in licenses {
            let license = license.trim();
            if license.is_empty() {
                continue;
            }
            if !self
                .licenses
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(license))
            {
                self.licenses.push(license.to_string());
            }
        }
    }

    /// Replaces the declared licenses.
    #[must_use]
    pub fn with_licenses(mut self, licenses: Vec<String>) -> Self {
        self.licenses.clear();
        self.add_licenses(licenses);
        self
    }

    /// Component name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Declared version or range.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// `purl` (*package URL*) identifier, if one could be built.
    #[must_use]
    pub fn purl(&self) -> Option<&str> {
        self.purl.as_deref()
    }

    /// Role in the inventory.
    #[must_use]
    pub fn kind(&self) -> AssayComponentKind {
        self.kind
    }

    /// Declared licenses (SPDX or other manifest labels).
    #[must_use]
    pub fn licenses(&self) -> &[String] {
        &self.licenses
    }
}

/// An impurity: a known vulnerability that affects a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssayFinding {
    vulnerability_id: String,
    aliases: Vec<String>,
    title: String,
    severity: AssaySeverity,
    component_name: String,
    component_version: String,
    fixed_version: Option<String>,
    details_url: Option<String>,
}

impl AssayFinding {
    /// Builds a finding.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        vulnerability_id: impl Into<String>,
        aliases: Vec<String>,
        title: impl Into<String>,
        severity: AssaySeverity,
        component_name: impl Into<String>,
        component_version: impl Into<String>,
        fixed_version: Option<String>,
        details_url: Option<String>,
    ) -> Self {
        Self {
            vulnerability_id: vulnerability_id.into(),
            aliases,
            title: title.into(),
            severity,
            component_name: component_name.into(),
            component_version: component_version.into(),
            fixed_version,
            details_url,
        }
    }

    /// Primary identifier (`CVE-…`, `GHSA-…`, `RUSTSEC-…`).
    #[must_use]
    pub fn vulnerability_id(&self) -> &str {
        &self.vulnerability_id
    }

    /// Other equivalent identifiers.
    #[must_use]
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    /// Short title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Severity.
    #[must_use]
    pub fn severity(&self) -> AssaySeverity {
        self.severity
    }

    /// Affected component.
    #[must_use]
    pub fn component_name(&self) -> &str {
        &self.component_name
    }

    /// Version of the affected component.
    #[must_use]
    pub fn component_version(&self) -> &str {
        &self.component_version
    }

    /// First version that fixes the finding, if known.
    #[must_use]
    pub fn fixed_version(&self) -> Option<&str> {
        self.fixed_version.as_deref()
    }

    /// Link to the public advisory (NVD, GitHub Advisory, …).
    #[must_use]
    pub fn details_url(&self) -> Option<&str> {
        self.details_url.as_deref()
    }
}

/// Finding count by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AssayCounts {
    /// Critical.
    pub critical: u32,
    /// High.
    pub high: u32,
    /// Medium.
    pub medium: u32,
    /// Low.
    pub low: u32,
    /// Unscored.
    pub unknown: u32,
}

impl AssayCounts {
    /// Sum of all findings.
    #[must_use]
    pub fn total(self) -> u32 {
        self.critical
            .saturating_add(self.high)
            .saturating_add(self.medium)
            .saturating_add(self.low)
            .saturating_add(self.unknown)
    }
}

/// Assay of a package version in a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assay {
    id: AssayId,
    repository_id: RepositoryId,
    coordinate: PackageCoordinate,
    status: AssayStatus,
    scanned_at: Option<String>,
    error_message: Option<String>,
    components: Vec<AssayComponent>,
    findings: Vec<AssayFinding>,
}

impl Assay {
    /// Builds a complete assay.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn from_parts(
        id: AssayId,
        repository_id: RepositoryId,
        coordinate: PackageCoordinate,
        status: AssayStatus,
        scanned_at: Option<String>,
        error_message: Option<String>,
        components: Vec<AssayComponent>,
        findings: Vec<AssayFinding>,
    ) -> Self {
        Self {
            id,
            repository_id,
            coordinate,
            status,
            scanned_at,
            error_message,
            components,
            findings,
        }
    }

    /// Assay identifier.
    #[must_use]
    pub fn id(&self) -> AssayId {
        self.id
    }

    /// Repository that stores the assayed package.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Assayed coordinate.
    #[must_use]
    pub fn coordinate(&self) -> &PackageCoordinate {
        &self.coordinate
    }

    /// Assay status.
    #[must_use]
    pub fn status(&self) -> AssayStatus {
        self.status
    }

    /// RFC 3339 timestamp of the last assay, if any.
    #[must_use]
    pub fn scanned_at(&self) -> Option<&str> {
        self.scanned_at.as_deref()
    }

    /// Detail if the assay failed or does not apply.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Component inventory.
    #[must_use]
    pub fn components(&self) -> &[AssayComponent] {
        &self.components
    }

    /// Findings (impurities).
    #[must_use]
    pub fn findings(&self) -> &[AssayFinding] {
        &self.findings
    }

    /// Same row, marked as in progress (the previous inventory stays).
    #[must_use]
    pub fn mark_running(&self) -> Self {
        Self {
            status: AssayStatus::Running,
            error_message: None,
            ..self.clone()
        }
    }

    /// Count by severity.
    #[must_use]
    pub fn counts(&self) -> AssayCounts {
        let mut counts = AssayCounts::default();
        for finding in &self.findings {
            match finding.severity {
                AssaySeverity::Critical => counts.critical += 1,
                AssaySeverity::High => counts.high += 1,
                AssaySeverity::Medium => counts.medium += 1,
                AssaySeverity::Low => counts.low += 1,
                AssaySeverity::Unknown => counts.unknown += 1,
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_coordinate::{PackageEcosystem, PackageName, PackageVersion};

    #[test]
    fn counts_group_findings_by_severity() {
        let assay = Assay::from_parts(
            AssayId::new(),
            RepositoryId::new(),
            PackageCoordinate::new(
                PackageEcosystem::Npm,
                PackageName::parse("lodash").unwrap(),
                PackageVersion::parse("4.17.20").unwrap(),
            ),
            AssayStatus::Ready,
            None,
            None,
            Vec::new(),
            vec![
                AssayFinding::new(
                    "CVE-1",
                    Vec::new(),
                    "a",
                    AssaySeverity::Critical,
                    "lodash",
                    "4.17.20",
                    None,
                    None,
                ),
                AssayFinding::new(
                    "CVE-2",
                    Vec::new(),
                    "b",
                    AssaySeverity::High,
                    "lodash",
                    "4.17.20",
                    None,
                    None,
                ),
            ],
        );
        let counts = assay.counts();
        assert_eq!(counts.critical, 1);
        assert_eq!(counts.high, 1);
        assert_eq!(counts.total(), 2);
    }

    #[test]
    fn component_licenses_deduplicate_case_insensitively() {
        let mut component = AssayComponent::new("demo", "1.0.0", None, AssayComponentKind::Root);
        component.add_licenses([
            "MIT".to_string(),
            "mit".to_string(),
            "Apache-2.0".to_string(),
        ]);
        assert_eq!(
            component.licenses(),
            &["MIT".to_string(), "Apache-2.0".to_string()]
        );
    }

    #[test]
    fn running_roundtrips_the_stable_label() {
        assert_eq!(AssayStatus::parse("running"), Some(AssayStatus::Running));
        assert_eq!(AssayStatus::Running.as_str(), "running");
    }

    #[test]
    fn mark_running_keeps_the_previous_inventory() {
        let ready = Assay::from_parts(
            AssayId::new(),
            RepositoryId::new(),
            PackageCoordinate::new(
                PackageEcosystem::Npm,
                PackageName::parse("lodash").unwrap(),
                PackageVersion::parse("4.17.20").unwrap(),
            ),
            AssayStatus::Ready,
            Some("2026-09-24T00:00:00Z".to_string()),
            None,
            vec![AssayComponent::new(
                "lodash",
                "4.17.20",
                None,
                AssayComponentKind::Root,
            )],
            Vec::new(),
        );
        let running = ready.mark_running();
        assert_eq!(running.status(), AssayStatus::Running);
        assert_eq!(running.components().len(), 1);
        assert_eq!(running.scanned_at(), ready.scanned_at());
        assert_eq!(running.error_message(), None);
    }

    #[test]
    fn cvss_score_maps_to_severity_bands() {
        assert_eq!(AssaySeverity::from_cvss_score(9.8), AssaySeverity::Critical);
        assert_eq!(AssaySeverity::from_cvss_score(7.5), AssaySeverity::High);
        assert_eq!(AssaySeverity::from_cvss_score(5.0), AssaySeverity::Medium);
        assert_eq!(AssaySeverity::from_cvss_score(2.0), AssaySeverity::Low);
    }

    #[test]
    fn unknown_never_meets_a_threshold() {
        assert!(!AssaySeverity::Unknown.meets_threshold(AssaySeverity::Low));
        assert!(AssaySeverity::Critical.meets_threshold(AssaySeverity::High));
        assert!(!AssaySeverity::Low.meets_threshold(AssaySeverity::High));
    }
}
