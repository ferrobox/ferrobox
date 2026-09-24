//! Documento [`CycloneDX`](https://cyclonedx.org/) 1.5 generado a partir de un ensaye.

use bytes::Bytes;
use ferrobox_domain::assay::{Assay, AssayComponent};
use serde_json::{Value, json};

fn cyclonedx_licenses(component: &AssayComponent) -> Vec<Value> {
    component
        .licenses()
        .iter()
        .map(|license| {
            if looks_like_spdx_id(license) {
                json!({ "license": { "id": license } })
            } else {
                json!({ "license": { "name": license } })
            }
        })
        .collect()
}

fn looks_like_spdx_id(value: &str) -> bool {
    !value.contains(' ')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+'))
}

/// Serializa el ensaye como JSON `CycloneDX` 1.5.
///
/// # Panics
///
/// No entra en pánico en la práctica: el documento se construye con
/// tipos que siempre serializan.
#[must_use]
pub fn to_cyclonedx(assay: &Assay) -> Bytes {
    let coordinate = assay.coordinate();
    let metadata_component = {
        let root = assay.components().first();
        let mut component = json!({
            "type": "library",
            "name": coordinate.name().as_str(),
            "version": coordinate.version().as_str(),
            "purl": root.and_then(|component| component.purl()),
        });
        if let Some(licenses) = root.map(cyclonedx_licenses).filter(|licenses| !licenses.is_empty())
        {
            component["licenses"] = Value::Array(licenses);
        }
        component
    };
    let components: Vec<Value> = assay
        .components()
        .iter()
        .skip(1)
        .map(|component| {
            let mut item = json!({
                "type": "library",
                "name": component.name(),
                "version": component.version(),
                "purl": component.purl(),
                "scope": "required",
            });
            let licenses = cyclonedx_licenses(component);
            if !licenses.is_empty() {
                item["licenses"] = Value::Array(licenses);
            }
            item
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

#[cfg(test)]
mod tests {
    use ferrobox_domain::assay::{
        Assay, AssayComponent, AssayComponentKind, AssayStatus,
    };
    use ferrobox_domain::ids::{AssayId, RepositoryId};
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };

    use super::*;

    #[test]
    fn writes_spdx_ids_on_the_root_component() {
        let assay = Assay::from_parts(
            AssayId::new(),
            RepositoryId::new(),
            PackageCoordinate::new(
                PackageEcosystem::Cargo,
                PackageName::parse("demo").unwrap(),
                PackageVersion::parse("1.0.0").unwrap(),
            ),
            AssayStatus::Ready,
            None,
            None,
            vec![
                AssayComponent::new("demo", "1.0.0", None, AssayComponentKind::Root)
                    .with_licenses(vec!["MIT".to_string()]),
                AssayComponent::new("serde", "1.0.0", None, AssayComponentKind::Direct)
                    .with_licenses(vec!["MIT License".to_string()]),
            ],
            Vec::new(),
        );
        let document: Value = serde_json::from_slice(&to_cyclonedx(&assay)).unwrap();
        assert_eq!(document["metadata"]["component"]["licenses"][0]["license"]["id"], "MIT");
        assert_eq!(
            document["components"][0]["licenses"][0]["license"]["name"],
            "MIT License"
        );
    }
}
