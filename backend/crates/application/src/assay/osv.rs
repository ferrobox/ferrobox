//! Consulta OSV (*Open Source Vulnerabilities*) por lote.

use std::collections::HashMap;

use bytes::Bytes;
use ferrobox_domain::assay::{AssayComponent, AssayFinding, AssaySeverity};
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use serde::Deserialize;
use serde_json::json;

use super::extract::osv_query_target;

const OSV_QUERYBATCH_URL: &str = "https://api.osv.dev/v1/querybatch";
const OSV_VULN_URL: &str = "https://api.osv.dev/v1/vulns";
const OSV_BATCH_SIZE: usize = 1000;

/// Consulta OSV para los componentes con versión concreta.
///
/// # Errors
///
/// Devuelve [`HttpClientError`] si el remoto falla.
///
/// # Panics
///
/// No entra en pánico en la práctica: el lote de consultas se construye
/// con tipos que siempre serializan.
pub async fn query_findings(
    http: &dyn HttpClient,
    ecosystem: PackageEcosystem,
    components: &[AssayComponent],
) -> Result<Vec<AssayFinding>, HttpClientError> {
    let mut queries = Vec::new();
    let mut queried_packages = Vec::new();
    for component in components {
        let Some((osv_eco, name, version)) = osv_query_target(component, ecosystem) else {
            continue;
        };
        queries.push(json!({
            "package": { "name": name, "ecosystem": osv_eco },
            "version": version,
        }));
        queried_packages.push((name, version));
    }

    if queries.is_empty() {
        return Ok(Vec::new());
    }

    let mut parsed = Vec::new();
    for start in (0..queries.len()).step_by(OSV_BATCH_SIZE) {
        let end = (start + OSV_BATCH_SIZE).min(queries.len());
        let body = Bytes::from(
            serde_json::to_vec(&json!({ "queries": queries[start..end] }))
                .expect("query batch always serializes"),
        );
        let response = http
            .post(OSV_QUERYBATCH_URL, body, "application/json")
            .await?;
        parsed.extend(parse_querybatch_vulns(
            &response.body,
            &queried_packages[start..end],
        ));
    }
    hydrate_thin_vulns(http, &mut parsed).await;
    Ok(parsed
        .iter()
        .map(|(name, version, vuln)| finding_from_vuln(vuln, name, version))
        .collect())
}

#[derive(Deserialize)]
struct QueryBatchResponse {
    #[serde(default)]
    results: Vec<QueryBatchResult>,
}

#[derive(Deserialize)]
struct QueryBatchResult {
    #[serde(default)]
    vulns: Vec<OsvVuln>,
}

#[derive(Clone, Deserialize)]
struct OsvVuln {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    database_specific: Option<OsvDatabaseSpecific>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
    #[serde(default)]
    references: Vec<OsvReference>,
}

#[derive(Clone, Deserialize)]
struct OsvSeverity {
    #[serde(default)]
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    score: String,
}

#[derive(Clone, Deserialize)]
struct OsvDatabaseSpecific {
    #[serde(default)]
    severity: Option<String>,
}

#[derive(Clone, Deserialize)]
struct OsvAffected {
    #[serde(default)]
    ranges: Vec<OsvRange>,
}

#[derive(Clone, Deserialize)]
struct OsvRange {
    #[serde(default)]
    events: Vec<OsvEvent>,
}

#[derive(Clone, Deserialize)]
struct OsvEvent {
    #[serde(default)]
    fixed: Option<String>,
}

#[derive(Clone, Deserialize)]
struct OsvReference {
    #[serde(default)]
    url: Option<String>,
}

fn parse_querybatch_vulns(
    body: &[u8],
    queried: &[(String, String)],
) -> Vec<(String, String, OsvVuln)> {
    let Ok(parsed) = serde_json::from_slice::<QueryBatchResponse>(body) else {
        return Vec::new();
    };
    let mut vulns = Vec::new();
    for (index, result) in parsed.results.into_iter().enumerate() {
        let Some((name, version)) = queried.get(index) else {
            continue;
        };
        for vuln in result.vulns {
            vulns.push((name.clone(), version.clone(), vuln));
        }
    }
    vulns
}

fn is_thin(vuln: &OsvVuln) -> bool {
    vuln.summary.is_none()
        && vuln.details.is_none()
        && vuln.severity.is_empty()
        && vuln.database_specific.is_none()
        && vuln.affected.is_empty()
        && vuln.aliases.is_empty()
}

async fn hydrate_thin_vulns(http: &dyn HttpClient, parsed: &mut [(String, String, OsvVuln)]) {
    let mut cache: HashMap<String, OsvVuln> = HashMap::new();
    for (_, _, vuln) in parsed.iter_mut() {
        if !is_thin(vuln) {
            continue;
        }
        if let Some(full) = cache.get(&vuln.id) {
            *vuln = full.clone();
            continue;
        }
        let url = format!("{OSV_VULN_URL}/{}", vuln.id);
        let Ok(response) = http.get(&url).await else {
            continue;
        };
        let Ok(full) = serde_json::from_slice::<OsvVuln>(&response.body) else {
            continue;
        };
        cache.insert(vuln.id.clone(), full.clone());
        *vuln = full;
    }
}

fn finding_from_vuln(vuln: &OsvVuln, component_name: &str, component_version: &str) -> AssayFinding {
    let title = vuln
        .summary
        .clone()
        .or_else(|| vuln.details.clone())
        .unwrap_or_else(|| vuln.id.clone());
    let details_url = vuln
        .references
        .iter()
        .find_map(|reference| reference.url.clone())
        .or_else(|| {
            if vuln.id.starts_with("CVE-") {
                Some(format!(
                    "https://nvd.nist.gov/vuln/detail/{}",
                    vuln.id
                ))
            } else if vuln.id.starts_with("GHSA-") {
                Some(format!("https://github.com/advisories/{}", vuln.id))
            } else {
                Some(format!("https://osv.dev/vulnerability/{}", vuln.id))
            }
        });
    AssayFinding::new(
        vuln.id.clone(),
        vuln.aliases.clone(),
        title,
        severity_of(vuln),
        component_name,
        component_version,
        first_fixed_version(vuln),
        details_url,
    )
}

fn severity_of(vuln: &OsvVuln) -> AssaySeverity {
    if let Some(label) = vuln
        .database_specific
        .as_ref()
        .and_then(|specific| specific.severity.as_deref())
    {
        let parsed = AssaySeverity::parse(label);
        if parsed != AssaySeverity::Unknown {
            return parsed;
        }
    }
    for entry in &vuln.severity {
        if let Ok(score) = entry.score.parse::<f64>() {
            return AssaySeverity::from_cvss_score(score);
        }
        if entry.kind.to_ascii_uppercase().contains("CVSS")
            && let Some(numeric) = cvss_numeric_from_vector(&entry.score)
        {
            return AssaySeverity::from_cvss_score(numeric);
        }
    }
    AssaySeverity::Unknown
}

fn cvss_numeric_from_vector(vector: &str) -> Option<f64> {
    // Algunos avisos mandan "7.5" y otros el vector. Si hay un número suelto, úsalo.
    vector.parse().ok()
}

fn first_fixed_version(vuln: &OsvVuln) -> Option<String> {
    vuln.affected.iter().find_map(|affected| {
        affected.ranges.iter().find_map(|range| {
            range
                .events
                .iter()
                .find_map(|event| event.fixed.clone())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_querybatch_maps_cve_and_severity() {
        let body = serde_json::json!({
            "results": [{
                "vulns": [{
                    "id": "GHSA-35jh-r3h4-6jhm",
                    "aliases": ["CVE-2021-23337"],
                    "summary": "Command Injection in lodash",
                    "database_specific": { "severity": "HIGH" },
                    "affected": [{
                        "ranges": [{ "events": [{ "fixed": "4.17.21" }] }]
                    }],
                    "references": [{ "url": "https://github.com/advisories/GHSA-35jh-r3h4-6jhm" }]
                }]
            }]
        });
        let parsed = parse_querybatch_vulns(
            &serde_json::to_vec(&body).unwrap(),
            &[("lodash".to_string(), "4.17.20".to_string())],
        );
        assert_eq!(parsed.len(), 1);
        let finding = finding_from_vuln(&parsed[0].2, &parsed[0].0, &parsed[0].1);
        assert_eq!(finding.vulnerability_id(), "GHSA-35jh-r3h4-6jhm");
        assert_eq!(finding.aliases(), &["CVE-2021-23337".to_string()]);
        assert_eq!(finding.severity(), AssaySeverity::High);
        assert_eq!(finding.fixed_version(), Some("4.17.21"));
    }
}
