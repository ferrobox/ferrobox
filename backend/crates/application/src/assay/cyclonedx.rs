//! Documento [`CycloneDX`](https://cyclonedx.org/) 1.5 generado a partir de un ensaye.

use bytes::Bytes;
use ferrobox_domain::assay::Assay;
use serde_json::{Value, json};

/// Serializa el ensaye como JSON `CycloneDX` 1.5.
///
/// # Panics
///
/// No entra en pánico en la práctica: el documento se construye con
/// tipos que siempre serializan.
#[must_use]
pub fn to_cyclonedx(assay: &Assay) -> Bytes {
    let coordinate = assay.coordinate();
    let metadata_component = json!({
        "type": "library",
        "name": coordinate.name().as_str(),
        "version": coordinate.version().as_str(),
        "purl": assay.components().first().and_then(|component| component.purl()),
    });
    let components: Vec<Value> = assay
        .components()
        .iter()
        .skip(1)
        .map(|component| {
            json!({
                "type": "library",
                "name": component.name(),
                "version": component.version(),
                "purl": component.purl(),
                "scope": "required",
            })
        })
        .collect();
    let vulnerabilities: Vec<Value> = assay
        .findings()
        .iter()
        .map(|finding| {
            json!({
                "id": finding.vulnerability_id(),
                "description": finding.title(),
                "source": { "name": "osv.dev", "url": finding.details_url() },
                "ratings": [{
                    "severity": finding.severity().as_str().to_ascii_uppercase(),
                    "method": "other",
                }],
                "affects": [{
                    "ref": format!("{}@{}", finding.component_name(), finding.component_version()),
                }],
                "recommendation": finding.fixed_version().map(|fixed| {
                    format!("Actualiza {name} a {fixed} o posterior", name = finding.component_name())
                }),
            })
        })
        .collect();

    Bytes::from(
        serde_json::to_vec_pretty(&json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.5",
            "version": 1,
            "metadata": {
                "component": metadata_component,
            },
            "components": components,
            "vulnerabilities": vulnerabilities,
        }))
        .expect("cyclonedx always serializes"),
    )
}
