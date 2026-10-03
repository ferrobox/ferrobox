//! Local OSV (*Open Source Vulnerabilities*) index.
//!
//! The live `api.osv.dev` query stays the default. When this index is
//! loaded, assay uses it instead and does not call the network. The
//! file is a JSON document (optionally gzipped) produced ahead of time;
//! `FerroBox` does not reindex the raw OSV corpus on import.
//!
//! An advisory matches a version when that version is listed in
//! `versions`, or when it falls in `[introduced, fixed)`. `fixed` is
//! exclusive, matching the OSV "fixed" event. `introduced` defaults to
//! `0` when only `fixed` or `last_affected` is set. `last_affected` is
//! inclusive and is used when the OSV range has no `fixed` event.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::Path;

use ferrobox_domain::assay::{AssayComponent, AssayFinding, AssaySeverity};
use ferrobox_domain::package_coordinate::PackageEcosystem;
use flate2::read::GzDecoder;
use serde::Deserialize;
use thiserror::Error;

use super::extract::osv_query_target;

const FORMAT: &str = "ferrobox-osv-index";
const FORMAT_VERSION: u32 = 1;

/// A parsed vulnerability feed.
#[derive(Debug, Clone)]
pub struct OsvFeed {
    dataset: String,
    advisories: HashMap<(String, String), Vec<Advisory>>,
}

/// Why a feed file could not be loaded.
#[derive(Debug, Error)]
pub enum FeedError {
    /// The path could not be read.
    #[error("could not read {path}: {source}")]
    Io {
        /// Path that failed.
        path: String,
        /// Filesystem error.
        source: std::io::Error,
    },

    /// Gzip framing was present but the payload did not decompress.
    #[error("could not decompress the feed: {0}")]
    Gzip(std::io::Error),

    /// The JSON document is not valid.
    #[error("feed JSON is invalid: {0}")]
    Json(serde_json::Error),

    /// The document is not a `ferrobox-osv-index` v1 file.
    #[error("invalid vulnerability feed: {0}")]
    Format(String),
}

#[derive(Debug, Clone)]
struct Advisory {
    ecosystem: String,
    name: String,
    id: String,
    aliases: Vec<String>,
    summary: String,
    severity: AssaySeverity,
    fixed: Option<String>,
    introduced: String,
    last_affected: Option<String>,
    use_range: bool,
    versions: Vec<String>,
    details_url: Option<String>,
}

#[derive(Deserialize)]
struct FeedDocument {
    format: String,
    format_version: u32,
    dataset: String,
    advisories: Vec<FeedAdvisory>,
}

#[derive(Deserialize)]
struct FeedAdvisory {
    ecosystem: String,
    name: String,
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    summary: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    fixed: Option<String>,
    #[serde(default)]
    introduced: Option<String>,
    #[serde(default)]
    last_affected: Option<String>,
    #[serde(default)]
    versions: Vec<String>,
    #[serde(default)]
    details_url: Option<String>,
}

impl OsvFeed {
    /// Loads a plain or gzipped index from disk.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError`] if the file is missing, truncated, or not a
    /// v1 `ferrobox-osv-index` document.
    pub fn load_path(path: impl AsRef<Path>) -> Result<Self, FeedError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| FeedError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_bytes(&bytes)
    }

    /// Parses a plain or gzipped index.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError`] if the bytes are not a v1 index.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FeedError> {
        let json = inflate_if_gzip(bytes)?;
        let document: FeedDocument = serde_json::from_slice(&json).map_err(FeedError::Json)?;
        Self::from_document(document)
    }

    /// Dataset label, for example `2026-10-02`.
    #[must_use]
    pub fn dataset(&self) -> &str {
        &self.dataset
    }

    /// Number of advisories in the index.
    #[must_use]
    pub fn advisory_count(&self) -> usize {
        self.advisories.values().map(Vec::len).sum()
    }

    /// OSV ecosystem names present in the index, in sorted order.
    #[must_use]
    pub fn ecosystems(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for (ecosystem, _) in self.advisories.keys() {
            names.insert(ecosystem.clone());
        }
        names.into_iter().collect()
    }

    /// Findings for every component the assay knows how to ask OSV about.
    #[must_use]
    pub fn query(
        &self,
        repository_ecosystem: PackageEcosystem,
        components: &[AssayComponent],
    ) -> Vec<AssayFinding> {
        let mut findings = Vec::new();
        for component in components {
            let Some((ecosystem, name, version)) =
                osv_query_target(component, repository_ecosystem)
            else {
                continue;
            };
            let key = (ecosystem.to_string(), normalize_name(ecosystem, &name));
            let Some(advisories) = self.advisories.get(&key) else {
                continue;
            };
            for advisory in advisories {
                if advisory.matches(&version) {
                    findings.push(advisory.to_finding(&name, &version));
                }
            }
        }
        findings
    }

    fn from_document(document: FeedDocument) -> Result<Self, FeedError> {
        if document.format != FORMAT || document.format_version != FORMAT_VERSION {
            return Err(FeedError::Format(format!(
                "expected {FORMAT} version {FORMAT_VERSION}, got {} version {}",
                document.format, document.format_version
            )));
        }
        if document.dataset.trim().is_empty() {
            return Err(FeedError::Format(
                "dataset version must not be empty".to_string(),
            ));
        }
        let mut advisories: HashMap<(String, String), Vec<Advisory>> = HashMap::new();
        for (index, raw) in document.advisories.into_iter().enumerate() {
            let advisory = Advisory::try_from(raw)
                .map_err(|reason| FeedError::Format(format!("advisory {index}: {reason}")))?;
            let key = (
                advisory.ecosystem.clone(),
                normalize_name(&advisory.ecosystem, &advisory.name),
            );
            advisories.entry(key).or_default().push(advisory);
        }
        Ok(Self {
            dataset: document.dataset,
            advisories,
        })
    }
}

impl Advisory {
    fn matches(&self, version: &str) -> bool {
        let listed = self
            .versions
            .iter()
            .any(|candidate| cmp_version(version, candidate) == Ordering::Equal);
        let ranged = self.use_range
            && in_range(
                version,
                &self.introduced,
                self.fixed.as_deref(),
                self.last_affected.as_deref(),
            );
        listed || ranged
    }

    fn to_finding(&self, component_name: &str, component_version: &str) -> AssayFinding {
        let title = if self.summary.is_empty() {
            self.id.clone()
        } else {
            self.summary.clone()
        };
        AssayFinding::new(
            self.id.clone(),
            self.aliases.clone(),
            title,
            self.severity,
            component_name,
            component_version,
            self.fixed.clone(),
            details_url(&self.id, self.details_url.clone()),
        )
    }
}

impl TryFrom<FeedAdvisory> for Advisory {
    type Error = String;

    fn try_from(raw: FeedAdvisory) -> Result<Self, Self::Error> {
        if raw.ecosystem.trim().is_empty() || raw.name.trim().is_empty() || raw.id.trim().is_empty()
        {
            return Err("ecosystem, name, and id are required".to_string());
        }
        let use_range =
            raw.introduced.is_some() || raw.fixed.is_some() || raw.last_affected.is_some();
        if !use_range && raw.versions.is_empty() {
            return Err(format!(
                "{} has neither a version range nor an explicit version list",
                raw.id
            ));
        }
        Ok(Self {
            ecosystem: raw.ecosystem,
            name: raw.name,
            id: raw.id,
            aliases: raw.aliases,
            summary: raw.summary,
            severity: AssaySeverity::parse(&raw.severity),
            fixed: raw.fixed,
            introduced: raw.introduced.unwrap_or_else(|| "0".to_string()),
            last_affected: raw.last_affected,
            use_range,
            versions: raw.versions,
            details_url: raw.details_url,
        })
    }
}

fn inflate_if_gzip(bytes: &[u8]) -> Result<Vec<u8>, FeedError> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.to_vec());
    }
    let mut decoder = GzDecoder::new(bytes);
    let mut text = Vec::new();
    decoder.read_to_end(&mut text).map_err(FeedError::Gzip)?;
    Ok(text)
}

fn normalize_name(ecosystem: &str, name: &str) -> String {
    match ecosystem {
        "PyPI" | "crates.io" | "NuGet" => name.to_ascii_lowercase(),
        _ => name.to_string(),
    }
}

fn in_range(
    version: &str,
    introduced: &str,
    fixed: Option<&str>,
    last_affected: Option<&str>,
) -> bool {
    if cmp_version(version, introduced) == Ordering::Less {
        return false;
    }
    if let Some(fixed) = fixed {
        return cmp_version(version, fixed) == Ordering::Less;
    }
    last_affected.is_none_or(|last| cmp_version(version, last) != Ordering::Greater)
}

fn details_url(id: &str, explicit: Option<String>) -> Option<String> {
    if explicit.is_some() {
        return explicit;
    }
    if id.starts_with("CVE-") {
        Some(format!("https://nvd.nist.gov/vuln/detail/{id}"))
    } else if id.starts_with("GHSA-") {
        Some(format!("https://github.com/advisories/{id}"))
    } else if id.is_empty() {
        None
    } else {
        Some(format!("https://osv.dev/vulnerability/{id}"))
    }
}

/// Compares two dotted versions. A pre-release (`1.0.0-rc.1`) is less
/// than the release (`1.0.0`). Missing numeric components count as zero.
fn cmp_version(left: &str, right: &str) -> Ordering {
    let (left_release, left_pre) = split_pre(left.trim());
    let (right_release, right_pre) = split_pre(right.trim());
    let release = cmp_release(left_release, right_release);
    if release != Ordering::Equal {
        return release;
    }
    match (left_pre, right_pre) {
        (None, None) => Ordering::Equal,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(left_pre), Some(right_pre)) => cmp_idents(left_pre, right_pre),
    }
}

fn split_pre(version: &str) -> (&str, Option<&str>) {
    match version.split_once('-') {
        Some((release, pre)) => (release, Some(pre)),
        None => (version, None),
    }
}

fn cmp_release(left: &str, right: &str) -> Ordering {
    let left: Vec<&str> = left.split('.').filter(|part| !part.is_empty()).collect();
    let right: Vec<&str> = right.split('.').filter(|part| !part.is_empty()).collect();
    let len = left.len().max(right.len());
    for index in 0..len {
        let ordering = cmp_part(
            left.get(index).copied().unwrap_or("0"),
            right.get(index).copied().unwrap_or("0"),
        );
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn cmp_idents(left: &str, right: &str) -> Ordering {
    let left: Vec<&str> = left.split('.').collect();
    let right: Vec<&str> = right.split('.').collect();
    let len = left.len().max(right.len());
    for index in 0..len {
        match (left.get(index), right.get(index)) {
            (Some(left), Some(right)) => {
                let ordering = cmp_part(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => return Ordering::Equal,
        }
    }
    Ordering::Equal
}

fn cmp_part(left: &str, right: &str) -> Ordering {
    match (left.parse::<u64>(), right.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        (Ok(_), Err(_)) => Ordering::Greater,
        (Err(_), Ok(_)) => Ordering::Less,
        (Err(_), Err(_)) => left.cmp(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrobox_domain::assay::{AssayComponent, AssayComponentKind};
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    fn lodash_feed() -> &'static str {
        r#"{
            "format": "ferrobox-osv-index",
            "format_version": 1,
            "dataset": "2026-10-02",
            "advisories": [{
                "ecosystem": "npm",
                "name": "lodash",
                "id": "GHSA-35jh-r3h4-6jhm",
                "aliases": ["CVE-2021-23337"],
                "summary": "Command Injection in lodash",
                "severity": "HIGH",
                "fixed": "4.17.21",
                "details_url": "https://github.com/advisories/GHSA-35jh-r3h4-6jhm"
            }, {
                "ecosystem": "PyPI",
                "name": "Requests",
                "id": "PYSEC-1",
                "summary": "exact pin",
                "severity": "LOW",
                "versions": ["2.25.0"]
            }]
        }"#
    }

    #[test]
    fn range_is_half_open_and_names_fold_case_for_pypi() {
        let feed = OsvFeed::from_bytes(lodash_feed().as_bytes()).unwrap();
        assert_eq!(feed.dataset(), "2026-10-02");
        assert_eq!(feed.advisory_count(), 2);
        assert_eq!(feed.ecosystems(), ["PyPI".to_string(), "npm".to_string()]);

        let vulnerable = AssayComponent::new("lodash", "4.17.20", None, AssayComponentKind::Root);
        let fixed = AssayComponent::new("lodash", "4.17.21", None, AssayComponentKind::Root);
        let requests = AssayComponent::new("requests", "2.25.0", None, AssayComponentKind::Root);
        let requests_other =
            AssayComponent::new("requests", "2.25.1", None, AssayComponentKind::Root);

        let hit = feed.query(PackageEcosystem::Npm, &[vulnerable]);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].vulnerability_id(), "GHSA-35jh-r3h4-6jhm");
        assert_eq!(hit[0].fixed_version(), Some("4.17.21"));
        assert!(feed.query(PackageEcosystem::Npm, &[fixed]).is_empty());

        assert_eq!(feed.query(PackageEcosystem::PyPi, &[requests]).len(), 1);
        assert!(
            feed.query(PackageEcosystem::PyPi, &[requests_other])
                .is_empty()
        );
    }

    #[test]
    fn gzip_payload_loads() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(lodash_feed().as_bytes()).unwrap();
        let bytes = encoder.finish().unwrap();
        let feed = OsvFeed::from_bytes(&bytes).unwrap();
        assert_eq!(feed.advisory_count(), 2);
    }

    #[test]
    fn rejects_an_advisory_with_no_match_rule() {
        let document = r#"{
            "format": "ferrobox-osv-index",
            "format_version": 1,
            "dataset": "2026-10-02",
            "advisories": [{
                "ecosystem": "npm",
                "name": "left-pad",
                "id": "GHSA-none",
                "summary": "incomplete"
            }]
        }"#;
        let error = OsvFeed::from_bytes(document.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("neither a version range"));
    }

    #[test]
    fn last_affected_includes_that_version_and_stops_after_it() {
        let document = r#"{
            "format": "ferrobox-osv-index",
            "format_version": 1,
            "dataset": "2026-10-03",
            "advisories": [{
                "ecosystem": "crates.io",
                "name": "demo",
                "id": "GHSA-2226-4v3c-cff8",
                "summary": "inclusive upper bound",
                "severity": "high",
                "introduced": "0",
                "last_affected": "0.3.24"
            }]
        }"#;
        let feed = OsvFeed::from_bytes(document.as_bytes()).unwrap();
        let affected = AssayComponent::new("demo", "0.3.24", None, AssayComponentKind::Root);
        let later = AssayComponent::new("demo", "0.3.25", None, AssayComponentKind::Root);
        assert_eq!(feed.query(PackageEcosystem::Cargo, &[affected]).len(), 1);
        assert!(feed.query(PackageEcosystem::Cargo, &[later]).is_empty());
    }

    #[test]
    fn pre_release_sorts_before_the_release() {
        assert_eq!(cmp_version("1.0.0-rc.1", "1.0.0"), Ordering::Less);
        assert_eq!(cmp_version("1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(cmp_version("4.17.20", "4.17.21"), Ordering::Less);
    }
}
