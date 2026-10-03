//! Build a `ferrobox-osv-index` v1 document from OSV (*Open Source Vulnerabilities*) exports.
//!
//! The input is the per-ecosystem `all.zip` published by osv.dev, or the
//! JSON files inside it. The output is the one file an install imports
//! with curl or pulls as `ghcr.io/<owner>/osv-db:YYYY-MM-DD`.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

const FORMAT: &str = "ferrobox-osv-index";
const FORMAT_VERSION: u32 = 1;

/// Ecosystems an assay can ask this index about.
const PACKAGE_ECOSYSTEMS: &[&str] = &["npm", "PyPI", "crates.io", "Maven", "NuGet", "Go"];

/// How many OSV documents were read and how many index rows were written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertStats {
    /// JSON documents read from the inputs.
    pub files: u64,
    /// Documents skipped because OSV marked them withdrawn.
    pub withdrawn: u64,
    /// Rows written into the index.
    pub advisories: u64,
}

/// Why the index could not be written.
#[derive(Debug, Error)]
pub enum ConvertError {
    /// The command line was incomplete.
    #[error("{0}")]
    Usage(String),

    /// The dataset label was empty.
    #[error("dataset version must not be empty")]
    EmptyDataset,

    /// A path could not be read or written.
    #[error("could not read {path}: {source}")]
    Io {
        /// Path that failed.
        path: String,
        /// Filesystem error.
        #[source]
        source: std::io::Error,
    },

    /// One OSV document was not JSON.
    #[error("invalid OSV JSON in {path}: {source}")]
    Json {
        /// Entry or file name.
        path: String,
        /// Parse error.
        #[source]
        source: serde_json::Error,
    },

    /// A zip archive could not be opened.
    #[error("could not read OSV zip {path}: {source}")]
    Zip {
        /// Archive path.
        path: String,
        /// Zip error.
        #[source]
        source: zip::result::ZipError,
    },

    /// The gzip wrapper could not be finished.
    #[error("could not gzip the index: {0}")]
    Gzip(#[source] std::io::Error),
}

/// Writes a plain or gzipped index. A path ending in `.gz` is gzipped.
///
/// # Errors
///
/// Returns [`ConvertError`] when an input cannot be read or is not OSV JSON.
pub fn write_index(
    dataset: &str,
    inputs: &[PathBuf],
    output: impl AsRef<Path>,
) -> Result<ConvertStats, ConvertError> {
    let output = output.as_ref();
    let result = write_index_file(dataset, inputs, output);
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

fn write_index_file(
    dataset: &str,
    inputs: &[PathBuf],
    output: &Path,
) -> Result<ConvertStats, ConvertError> {
    if dataset.trim().is_empty() {
        return Err(ConvertError::EmptyDataset);
    }
    let file = File::create(output).map_err(|source| ConvertError::Io {
        path: output.display().to_string(),
        source,
    })?;
    if output.extension().is_some_and(|ext| ext == "gz") {
        let mut encoder = GzEncoder::new(file, Compression::default());
        let stats = write_document(&mut encoder, dataset, inputs)?;
        encoder.finish().map_err(ConvertError::Gzip)?;
        Ok(stats)
    } else {
        let mut file = file;
        write_document(&mut file, dataset, inputs)
    }
}

fn write_document<W: Write>(
    writer: &mut W,
    dataset: &str,
    inputs: &[PathBuf],
) -> Result<ConvertStats, ConvertError> {
    writer.write_all(br#"{"format":""#).map_err(io_output)?;
    writer.write_all(FORMAT.as_bytes()).map_err(io_output)?;
    writer
        .write_all(format!(r#"","format_version":{FORMAT_VERSION},"dataset":"#).as_bytes())
        .map_err(io_output)?;
    serde_json::to_writer(&mut *writer, dataset).map_err(json_output)?;
    writer.write_all(br#","advisories":["#).map_err(io_output)?;
    let mut stats = ConvertStats {
        files: 0,
        withdrawn: 0,
        advisories: 0,
    };
    let mut first = true;
    for input in inputs {
        ingest_input(input, writer, &mut first, &mut stats)?;
    }
    writer.write_all(b"]}").map_err(io_output)?;
    Ok(stats)
}

fn io_output(source: std::io::Error) -> ConvertError {
    ConvertError::Io {
        path: "index output".to_string(),
        source,
    }
}

fn json_output(source: serde_json::Error) -> ConvertError {
    ConvertError::Io {
        path: "index output".to_string(),
        source: std::io::Error::other(source),
    }
}

fn ingest_input<W: Write>(
    input: &Path,
    writer: &mut W,
    first: &mut bool,
    stats: &mut ConvertStats,
) -> Result<(), ConvertError> {
    if input.is_dir() {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(input).map_err(|source| ConvertError::Io {
            path: input.display().to_string(),
            source,
        })? {
            let entry = entry.map_err(|source| ConvertError::Io {
                path: input.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                paths.push(path);
            }
        }
        paths.sort();
        for path in paths {
            ingest_input(&path, writer, first, stats)?;
        }
        return Ok(());
    }
    let name = input.display().to_string();
    if input.extension().is_some_and(|ext| ext == "zip") {
        return ingest_zip(input, writer, first, stats);
    }
    let bytes = std::fs::read(input).map_err(|source| ConvertError::Io {
        path: name.clone(),
        source,
    })?;
    ingest_bytes(&name, &bytes, writer, first, stats)
}

fn ingest_zip<W: Write>(
    input: &Path,
    writer: &mut W,
    first: &mut bool,
    stats: &mut ConvertStats,
) -> Result<(), ConvertError> {
    let file = File::open(input).map_err(|source| ConvertError::Io {
        path: input.display().to_string(),
        source,
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|source| ConvertError::Zip {
        path: input.display().to_string(),
        source,
    })?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|source| ConvertError::Zip {
                path: input.display().to_string(),
                source,
            })?;
        let name = entry.name().to_string();
        let is_json = Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
        if entry.is_dir() || !is_json {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|source| ConvertError::Io {
                path: format!("{}:{name}", input.display()),
                source,
            })?;
        ingest_bytes(
            &format!("{}:{name}", input.display()),
            &bytes,
            writer,
            first,
            stats,
        )?;
    }
    Ok(())
}

fn ingest_bytes<W: Write>(
    name: &str,
    bytes: &[u8],
    writer: &mut W,
    first: &mut bool,
    stats: &mut ConvertStats,
) -> Result<(), ConvertError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let doc: OsvDocument = serde_json::from_slice(bytes).map_err(|source| ConvertError::Json {
        path: name.to_string(),
        source,
    })?;
    stats.files += 1;
    match rows_from_document(&doc) {
        Rows::Withdrawn => stats.withdrawn += 1,
        Rows::Advisories(rows) => {
            for row in rows {
                if !*first {
                    writer.write_all(b",").map_err(io_output)?;
                }
                *first = false;
                serde_json::to_writer(&mut *writer, &row).map_err(json_output)?;
                stats.advisories += 1;
            }
        }
    }
    Ok(())
}

enum Rows {
    Withdrawn,
    Advisories(Vec<IndexAdvisory>),
}

fn rows_from_document(doc: &OsvDocument) -> Rows {
    if doc
        .withdrawn
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Rows::Withdrawn;
    }
    let mut rows = Vec::new();
    for affected in &doc.affected {
        let Some(package) = &affected.package else {
            continue;
        };
        if !PACKAGE_ECOSYSTEMS.contains(&package.ecosystem.as_str())
            || package.name.trim().is_empty()
            || doc.id.trim().is_empty()
        {
            continue;
        }
        let windows = version_windows(affected);
        if windows.is_empty() {
            let versions = clean_versions(&affected.versions);
            if versions.is_empty() {
                continue;
            }
            rows.push(row(doc, package, None, versions));
            continue;
        }
        for window in windows {
            rows.push(row(doc, package, Some(&window), Vec::new()));
        }
    }
    Rows::Advisories(rows)
}

fn version_windows(affected: &OsvAffected) -> Vec<Window> {
    let mut windows = Vec::new();
    for range in &affected.ranges {
        if range.kind == "SEMVER" || range.kind == "ECOSYSTEM" {
            windows.extend(windows_from_events(&range.events));
        }
    }
    windows
}

fn windows_from_events(events: &[OsvEvent]) -> Vec<Window> {
    let mut windows = Vec::new();
    let mut open: Option<String> = None;
    for event in events {
        if let Some(introduced) = event
            .introduced
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            if let Some(previous) = open.take() {
                windows.push(Window {
                    introduced: previous,
                    fixed: None,
                    last_affected: None,
                });
            }
            open = Some(introduced.to_string());
        } else if let Some(fixed) = event
            .fixed
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            windows.push(Window {
                introduced: open.take().unwrap_or_else(|| "0".to_string()),
                fixed: Some(fixed.to_string()),
                last_affected: None,
            });
        } else if let Some(last) = event
            .last_affected
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            windows.push(Window {
                introduced: open.take().unwrap_or_else(|| "0".to_string()),
                fixed: None,
                last_affected: Some(last.to_string()),
            });
        }
    }
    if let Some(introduced) = open {
        windows.push(Window {
            introduced,
            fixed: None,
            last_affected: None,
        });
    }
    windows
}

fn clean_versions(versions: &[String]) -> Vec<String> {
    versions
        .iter()
        .map(|version| version.trim())
        .filter(|version| !version.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn row(
    doc: &OsvDocument,
    package: &OsvPackage,
    window: Option<&Window>,
    versions: Vec<String>,
) -> IndexAdvisory {
    IndexAdvisory {
        ecosystem: package.ecosystem.clone(),
        name: package.name.clone(),
        id: doc.id.clone(),
        aliases: doc
            .aliases
            .iter()
            .map(|alias| alias.trim())
            .filter(|alias| !alias.is_empty())
            .map(ToString::to_string)
            .collect(),
        summary: summary_of(doc),
        severity: severity_label(doc),
        introduced: window.as_ref().map(|window| window.introduced.clone()),
        fixed: window.as_ref().and_then(|window| window.fixed.clone()),
        last_affected: window
            .as_ref()
            .and_then(|window| window.last_affected.clone()),
        versions,
        details_url: format!("https://osv.dev/vulnerability/{}", doc.id),
    }
}

fn summary_of(doc: &OsvDocument) -> String {
    let summary = doc.summary.trim();
    if !summary.is_empty() {
        return truncate_chars(summary, 240);
    }
    let line = doc.details.lines().next().unwrap_or("").trim();
    truncate_chars(line, 240)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn severity_label(doc: &OsvDocument) -> String {
    if let Some(specific) = &doc.database_specific
        && let Some(label) = specific.severity.as_deref()
        && let Some(mapped) = map_severity_word(label)
    {
        return mapped.to_string();
    }
    for item in &doc.severity {
        let score = item.score.trim();
        if let Some(score) = cvss_v3_base_score(score) {
            return score_label(score).to_string();
        }
        if let Ok(score) = score.parse::<f64>() {
            return score_label(score).to_string();
        }
    }
    String::new()
}

fn map_severity_word(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "critical" => Some("critical"),
        "high" => Some("high"),
        "medium" | "moderate" => Some("medium"),
        "low" => Some("low"),
        _ => None,
    }
}

fn score_label(score: f64) -> &'static str {
    if score >= 9.0 {
        "critical"
    } else if score >= 7.0 {
        "high"
    } else if score >= 4.0 {
        "medium"
    } else if score > 0.0 {
        "low"
    } else {
        "unknown"
    }
}

struct Window {
    introduced: String,
    fixed: Option<String>,
    last_affected: Option<String>,
}

#[derive(Serialize)]
struct IndexAdvisory {
    ecosystem: String,
    name: String,
    id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    summary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    introduced: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fixed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_affected: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    versions: Vec<String>,
    details_url: String,
}

#[derive(Deserialize)]
struct OsvDocument {
    id: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    details: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
    withdrawn: Option<String>,
    #[serde(default)]
    database_specific: Option<DatabaseSpecific>,
}

#[derive(Deserialize)]
struct DatabaseSpecific {
    #[serde(default)]
    severity: Option<String>,
}

#[derive(Deserialize)]
struct OsvSeverity {
    score: String,
}

#[derive(Deserialize)]
struct OsvAffected {
    package: Option<OsvPackage>,
    #[serde(default)]
    ranges: Vec<OsvRange>,
    #[serde(default)]
    versions: Vec<String>,
}

#[derive(Deserialize)]
struct OsvPackage {
    ecosystem: String,
    name: String,
}

#[derive(Deserialize)]
struct OsvRange {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    events: Vec<OsvEvent>,
}

#[derive(Deserialize)]
struct OsvEvent {
    #[serde(default)]
    introduced: Option<String>,
    #[serde(default)]
    fixed: Option<String>,
    #[serde(default)]
    last_affected: Option<String>,
}

/// CVSS 3.0 and 3.1 base score. The vector's numeric score is not included.
fn cvss_v3_base_score(vector: &str) -> Option<f64> {
    let rest = vector
        .trim()
        .strip_prefix("CVSS:3.1/")
        .or_else(|| vector.trim().strip_prefix("CVSS:3.0/"))?;
    let mut metrics = std::collections::HashMap::new();
    for part in rest.split('/') {
        let (key, value) = part.split_once(':')?;
        metrics.insert(key, value);
    }
    let attack_vector = match *metrics.get("AV")? {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        "P" => 0.20,
        _ => return None,
    };
    let attack_complexity = match *metrics.get("AC")? {
        "L" => 0.77,
        "H" => 0.44,
        _ => return None,
    };
    let scope_changed = match *metrics.get("S")? {
        "U" => false,
        "C" => true,
        _ => return None,
    };
    let privileges = match (*metrics.get("PR")?, scope_changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        ("H", false) => 0.27,
        ("H", true) => 0.50,
        _ => return None,
    };
    let user_interaction = match *metrics.get("UI")? {
        "N" => 0.85,
        "R" => 0.62,
        _ => return None,
    };
    let confidentiality = impact_value(metrics.get("C")?)?;
    let integrity = impact_value(metrics.get("I")?)?;
    let availability = impact_value(metrics.get("A")?)?;
    let impact_subscore =
        1.0 - ((1.0 - confidentiality) * (1.0 - integrity) * (1.0 - availability));
    let impact = if scope_changed {
        7.52 * (impact_subscore - 0.029) - 3.25 * (impact_subscore - 0.02).powi(15)
    } else {
        6.42 * impact_subscore
    };
    if impact <= 0.0 {
        return Some(0.0);
    }
    let exploitability = 8.22 * attack_vector * attack_complexity * privileges * user_interaction;
    let raw = if scope_changed {
        (1.08 * (impact + exploitability)).min(10.0)
    } else {
        (impact + exploitability).min(10.0)
    };
    Some(round_up_tenth(raw))
}

fn impact_value(value: &str) -> Option<f64> {
    match value {
        "H" => Some(0.56),
        "L" => Some(0.22),
        "N" => Some(0.0),
        _ => None,
    }
}

/// Round up to one decimal, as the CVSS 3.1 specification defines it.
///
/// The scaled integer stays under one million for a score of at most 10,
/// so the integer casts do not change the result.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn round_up_tenth(value: f64) -> f64 {
    let scaled = (value * 100_000.0).round() as i64;
    if scaled.rem_euclid(10_000) == 0 {
        scaled as f64 / 100_000.0
    } else {
        ((scaled / 10_000) + 1) as f64 / 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LODASH: &str = r#"{
        "id": "GHSA-35jh-r3h4-6jhm",
        "summary": "Command Injection in lodash",
        "aliases": ["CVE-2021-23337"],
        "severity": [{"type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:H/UI:N/S:U/C:H/I:H/A:H"}],
        "database_specific": {"severity": "HIGH"},
        "affected": [{
            "package": {"ecosystem": "npm", "name": "lodash"},
            "ranges": [{"type": "SEMVER", "events": [{"introduced": "0"}, {"fixed": "4.17.21"}]}]
        }]
    }"#;

    #[test]
    fn lodash_advisory_keeps_the_exclusive_fix_and_the_osv_severity() {
        let doc: OsvDocument = serde_json::from_str(LODASH).unwrap();
        let Rows::Advisories(rows) = rows_from_document(&doc) else {
            panic!("withdrawn");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ecosystem, "npm");
        assert_eq!(rows[0].name, "lodash");
        assert_eq!(rows[0].fixed.as_deref(), Some("4.17.21"));
        assert_eq!(rows[0].introduced.as_deref(), Some("0"));
        assert_eq!(rows[0].severity, "high");
        assert_eq!(rows[0].aliases, ["CVE-2021-23337"]);
        assert_eq!(
            rows[0].details_url,
            "https://osv.dev/vulnerability/GHSA-35jh-r3h4-6jhm"
        );
    }

    #[test]
    fn last_affected_and_a_second_range_become_separate_rows() {
        let doc: OsvDocument = serde_json::from_str(
            r#"{
                "id": "GHSA-2226-4v3c-cff8",
                "summary": "two windows",
                "affected": [{
                    "package": {"ecosystem": "crates.io", "name": "demo"},
                    "ranges": [
                        {"type": "SEMVER", "events": [{"introduced": "0"}, {"last_affected": "0.3.24"}]},
                        {"type": "SEMVER", "events": [{"introduced": "1.0.0"}, {"fixed": "1.2.0"}, {"introduced": "2.0.0"}, {"fixed": "2.1.0"}]}
                    ]
                }, {
                    "package": {"ecosystem": "npm", "name": "other"},
                    "ranges": [{"type": "GIT", "events": [{"introduced": "abc"}]}]
                }]
            }"#,
        )
        .unwrap();
        let Rows::Advisories(rows) = rows_from_document(&doc) else {
            panic!("withdrawn");
        };
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].last_affected.as_deref(), Some("0.3.24"));
        assert!(rows[0].fixed.is_none());
        assert_eq!(rows[1].fixed.as_deref(), Some("1.2.0"));
        assert_eq!(rows[2].introduced.as_deref(), Some("2.0.0"));
        assert_eq!(rows[2].fixed.as_deref(), Some("2.1.0"));
    }

    #[test]
    fn a_git_range_falls_back_to_the_explicit_versions() {
        let doc: OsvDocument = serde_json::from_str(
            r#"{
                "id": "OSV-1",
                "summary": "commits only",
                "affected": [{
                    "package": {"ecosystem": "Go", "name": "example.com/mod"},
                    "ranges": [{"type": "GIT", "events": [{"introduced": "abc"}]}],
                    "versions": ["v1.2.3"]
                }]
            }"#,
        )
        .unwrap();
        let Rows::Advisories(rows) = rows_from_document(&doc) else {
            panic!("withdrawn");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].versions, ["v1.2.3"]);
        assert!(rows[0].introduced.is_none());
    }

    #[test]
    fn withdrawn_and_unrelated_ecosystems_add_no_rows() {
        let withdrawn: OsvDocument =
            serde_json::from_str(r#"{"id":"X","withdrawn":"2024-01-01T00:00:00Z","affected":[]}"#)
                .unwrap();
        assert!(matches!(rows_from_document(&withdrawn), Rows::Withdrawn));
        let other: OsvDocument = serde_json::from_str(
            r#"{
                "id": "X",
                "summary": "debian",
                "affected": [{"package": {"ecosystem": "Debian", "name": "openssl"}, "versions": ["1"]}]
            }"#,
        )
        .unwrap();
        let Rows::Advisories(rows) = rows_from_document(&other) else {
            panic!("withdrawn");
        };
        assert!(rows.is_empty());
    }

    #[test]
    fn cvss_v3_base_scores_match_the_specification() {
        assert_eq!(
            cvss_v3_base_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            Some(9.8)
        );
        assert_eq!(
            cvss_v3_base_score("CVSS:3.1/AV:N/AC:L/PR:H/UI:N/S:U/C:H/I:H/A:H"),
            Some(7.2)
        );
        assert_eq!(score_label(9.8), "critical");
        assert_eq!(score_label(7.2), "high");
    }

    #[test]
    fn gzipped_output_round_trips_the_lodash_advisory() {
        let dir = std::env::temp_dir().join(format!("ferrobox-osv-index-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("lodash.json");
        let output = dir.join("index.json.gz");
        std::fs::write(&input, LODASH).unwrap();
        let stats = write_index("2026-10-03", &[input], &output).unwrap();
        assert_eq!(stats.files, 1);
        assert_eq!(stats.advisories, 1);
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut text = String::new();
        decoder.read_to_string(&mut text).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["format"], "ferrobox-osv-index");
        assert_eq!(parsed["format_version"], 1);
        assert_eq!(parsed["dataset"], "2026-10-03");
        assert_eq!(parsed["advisories"][0]["id"], "GHSA-35jh-r3h4-6jhm");
        assert_eq!(parsed["advisories"][0]["fixed"], "4.17.21");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
