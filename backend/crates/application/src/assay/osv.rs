//! Consulta OSV (*Open Source Vulnerabilities*) por lote.

use bytes::Bytes;
use ferrobox_domain::assay::{AssayComponent, AssayFinding, AssaySeverity};
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use serde::Deserialize;
use serde_json::json;

use super::extract::{is_exact_version, osv_ecosystem};

const OSV_QUERYBATCH_URL: &str = "https://api.osv.dev/v1/querybatch";

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
    let Some(osv_eco) = osv_ecosystem(ecosystem) else {
        return Ok(Vec::new());
    };

    let mut queries = Vec::new();
    let mut queried_packages = Vec::new();
    for component in components {
        if component.kind() != ferrobox_domain::assay::AssayComponentKind::Root
            && !is_exact_version(component.version())
        {
            continue;
        }
        queries.push(json!({
            "package": { "name": component.name(), "ecosystem": osv_eco },
            "version": component.version(),
        }));
        queried_packages.push((component.name().to_string(), component.version().to_string()));
    }

    if queries.is_empty() {
        return Ok(Vec::new());
    }

    let body = Bytes::from(
        serde_json::to_vec(&json!({ "queries": queries })).expect("query batch always serializes"),
    );
    let response = http
        .post(OSV_QUERYBATCH_URL, body, "application/json")
        .await?;
    Ok(parse_querybatch(&response.body, &queried_packages))
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

#[derive(Deserialize)]
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

#[derive(Deserialize)]
struct OsvSeverity {
    #[serde(default)]
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    score: String,
}

#[derive(Deserialize)]
struct OsvDatabaseSpecific {
    #[serde(default)]
    severity: Option<String>,
}

#[derive(Deserialize)]
struct OsvAffected {
    #[serde(default)]
    ranges: Vec<OsvRange>,
}

#[derive(Deserialize)]
struct OsvRange {
    #[serde(default)]
    events: Vec<OsvEvent>,
}

#[derive(Deserialize)]
struct OsvEvent {
    #[serde(default)]
    fixed: Option<String>,
}

#[derive(Deserialize)]
struct OsvReference {
    #[serde(default)]
    url: Option<String>,
}

fn parse_querybatch(body: &[u8], queried: &[(String, String)]) -> Vec<AssayFinding> {
    let Ok(parsed) = serde_json::from_slice::<QueryBatchResponse>(body) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for (index, result) in parsed.results.into_iter().enumerate() {
        let Some((name, version)) = queried.get(index) else {
            continue;
        };
        for vuln in result.vulns {
            findings.push(finding_from_vuln(&vuln, name, version));
        }
    }
    findings
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
        let findings = parse_querybatch(
            &serde_json::to_vec(&body).unwrap(),
            &[("lodash".to_string(), "4.17.20".to_string())],
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].vulnerability_id(), "GHSA-35jh-r3h4-6jhm");
        assert_eq!(findings[0].aliases(), &["CVE-2021-23337".to_string()]);
        assert_eq!(findings[0].severity(), AssaySeverity::High);
        assert_eq!(findings[0].fixed_version(), Some("4.17.21"));
    }
}
