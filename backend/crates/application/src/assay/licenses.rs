//! Licencias declaradas en manifiestos e índices.

use ferrobox_domain::assay::{AssayComponent, AssayComponentKind};
use serde_json::Value;

/// Separa una declaración en etiquetas SPDX (u otras) sin inventar.
#[must_use]
pub fn split_declared(raw: &str) -> Vec<String> {
    let raw = raw
        .trim()
        .trim_matches(['"', '\'', '(', ')'])
        .trim();
    if raw.is_empty() || is_placeholder(raw) {
        return Vec::new();
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == ',' || ch == '/' {
            push_part(&mut parts, current.trim());
            current.clear();
            continue;
        }
        if ch.is_ascii_whitespace() {
            let rest: String = chars.clone().collect();
            let rest_lower = rest.trim_start().to_ascii_lowercase();
            if rest_lower.starts_with("or ") || rest_lower.starts_with("and ") {
                push_part(&mut parts, current.trim());
                current.clear();
                if rest_lower.starts_with("or ") {
                    chars.by_ref().take(2).for_each(drop);
                } else {
                    chars.by_ref().take(3).for_each(drop);
                }
                continue;
            }
        }
        current.push(ch);
    }
    push_part(&mut parts, current.trim());
    parts
}

fn is_placeholder(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "unknown" | "none" | "see license" | "put the package license here"
    ) || lower.starts_with("see license in")
        || lower.starts_with('<')
        || lower.contains("put the package license")
}

fn push_part(parts: &mut Vec<String>, part: &str) {
    let part = part.trim().trim_matches(['"', '\'', '(', ')']).trim();
    if part.is_empty() || is_placeholder(part) {
        return;
    }
    if parts
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(part))
    {
        return;
    }
    parts.push(part.to_string());
}

/// Licencias del manifiesto npm (`license`, `licenses` o `{ type }`).
#[must_use]
pub fn from_npm_manifest(manifest: &Value) -> Vec<String> {
    let licenses = from_npm_value(manifest.get("license"));
    if licenses.is_empty() {
        from_npm_value(manifest.get("licenses"))
    } else {
        licenses
    }
}

fn from_npm_value(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::String(raw) => split_declared(raw),
        Value::Object(map) => map
            .get("type")
            .and_then(Value::as_str)
            .map(split_declared)
            .unwrap_or_default(),
        Value::Array(items) => {
            let mut licenses = Vec::new();
            for item in items {
                for license in from_npm_value(Some(item)) {
                    push_part(&mut licenses, &license);
                }
            }
            licenses
        }
        _ => Vec::new(),
    }
}

/// Licencia del índice o del `Cargo.toml` (`license = "…"`).
#[must_use]
pub fn from_cargo_index(entry: &Value) -> Vec<String> {
    entry
        .get("license")
        .and_then(Value::as_str)
        .map(split_declared)
        .unwrap_or_default()
}

/// Licencia de la sección `[package]` de un `Cargo.toml`.
#[must_use]
pub fn from_cargo_toml(text: &str) -> (Option<String>, Vec<String>) {
    let mut in_package = false;
    let mut name = None;
    let mut licenses = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name") {
            name = toml_quoted(value);
        } else if let Some(value) = trimmed.strip_prefix("license")
            && !trimmed.starts_with("license-file")
            && let Some(raw) = toml_quoted(value)
        {
            licenses = split_declared(&raw);
        }
    }
    (name, licenses)
}

fn toml_quoted(rest: &str) -> Option<String> {
    let rest = rest.trim().trim_start_matches('=').trim();
    let value = rest.trim_matches('"').trim_matches('\'').trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Licencias de `METADATA` / `PKG-INFO` de Python.
#[must_use]
pub fn from_python_metadata(text: &str) -> (Option<String>, Vec<String>) {
    let mut name = None;
    let mut expression = Vec::new();
    let mut license_field = Vec::new();
    let mut classifiers = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("Name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("License-Expression:") {
            expression = split_declared(value);
        } else if let Some(value) = line.strip_prefix("License:") {
            license_field = split_declared(value);
        } else if let Some(value) = line.strip_prefix("Classifier:") {
            let value = value.trim();
            if let Some(label) = value.strip_prefix("License ::") {
                let label = label
                    .rsplit("::")
                    .next()
                    .unwrap_or(label)
                    .trim();
                if !label.is_empty() && !label.eq_ignore_ascii_case("OSI Approved") {
                    push_part(&mut classifiers, label);
                }
            }
        }
    }
    let licenses = if !expression.is_empty() {
        expression
    } else if !license_field.is_empty() {
        license_field
    } else {
        classifiers
    };
    (name.filter(|value| !value.is_empty()), licenses)
}

/// Licencia de `Chart.yaml` (campo `license:`).
#[must_use]
pub fn from_chart_yaml(text: &str) -> Vec<String> {
    if let Ok(value) = serde_json::from_str::<Value>(text)
        && let Some(raw) = value.get("license").and_then(Value::as_str)
    {
        return split_declared(raw);
    }
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("license:") {
            return split_declared(value.trim().trim_matches(['"', '\'']));
        }
    }
    Vec::new()
}

/// Licencia de `conanfile.py` (`license = "…"`).
#[must_use]
pub fn from_conanfile_py(text: &str) -> Vec<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("license")
            .or_else(|| trimmed.strip_prefix("self.license"))
        else {
            continue;
        };
        let rest = rest.trim().trim_start_matches('=').trim();
        if rest.starts_with(['"', '\'']) {
            return split_declared(rest);
        }
    }
    Vec::new()
}

/// Añade licencias al componente del mismo nombre, o al raíz si no hay nombre.
pub fn attach(
    components: &mut [AssayComponent],
    name: Option<&str>,
    licenses: Vec<String>,
) {
    if licenses.is_empty() {
        return;
    }
    if let Some(name) = name
        && let Some(component) = components
            .iter_mut()
            .find(|component| component.name().eq_ignore_ascii_case(name))
    {
        component.add_licenses(licenses);
        return;
    }
    if let Some(root) = components
        .iter_mut()
        .find(|component| component.kind() == AssayComponentKind::Root)
    {
        root.add_licenses(licenses);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_spdx_expressions() {
        assert_eq!(
            split_declared("MIT OR Apache-2.0"),
            vec!["MIT".to_string(), "Apache-2.0".to_string()]
        );
        assert_eq!(
            split_declared("MIT/Apache-2.0"),
            vec!["MIT".to_string(), "Apache-2.0".to_string()]
        );
        assert!(split_declared("SEE LICENSE IN LICENSE").is_empty());
        assert!(split_declared("<Put the package license here>").is_empty());
    }

    #[test]
    fn npm_reads_string_object_and_array() {
        assert_eq!(
            from_npm_manifest(&serde_json::json!({ "license": "MIT" })),
            vec!["MIT".to_string()]
        );
        assert_eq!(
            from_npm_manifest(&serde_json::json!({ "license": { "type": "BSD-3-Clause" } })),
            vec!["BSD-3-Clause".to_string()]
        );
        assert_eq!(
            from_npm_manifest(&serde_json::json!({
                "licenses": [{ "type": "MIT" }, { "type": "Apache-2.0" }]
            })),
            vec!["MIT".to_string(), "Apache-2.0".to_string()]
        );
    }

    #[test]
    fn cargo_toml_reads_package_license() {
        let text = "[package]\nname = \"demo\"\nlicense = \"MIT OR Apache-2.0\"\n";
        let (name, licenses) = from_cargo_toml(text);
        assert_eq!(name.as_deref(), Some("demo"));
        assert_eq!(licenses, vec!["MIT".to_string(), "Apache-2.0".to_string()]);
    }

    #[test]
    fn python_prefers_license_expression() {
        let text = "Name: demo\nLicense: MIT\nLicense-Expression: Apache-2.0\nClassifier: License :: OSI Approved :: GNU General Public License v3 (GPLv3)\n";
        let (name, licenses) = from_python_metadata(text);
        assert_eq!(name.as_deref(), Some("demo"));
        assert_eq!(licenses, vec!["Apache-2.0".to_string()]);
    }
}
