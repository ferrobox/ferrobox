//! Inventario a partir de lockfiles y metadatos dentro del artefacto.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use ferrobox_domain::assay::{AssayComponent, AssayComponentKind};
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem};
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::storage::StoragePort;
use flate2::read::GzDecoder;
use serde_json::Value;
use tar::Archive;
use uuid::Uuid;
use zip::ZipArchive;

use super::extract::{is_exact_version, merge_component, purl_for};
use crate::storage_key::storage_key_for;

const MAX_LOCKFILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_COMPONENTS: usize = 400;

/// Añade componentes resueltos desde lockfiles o metadatos del binario.
pub async fn append_from_stored_package(
    storage: &dyn StoragePort,
    index: &dyn PackageIndexStore,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
    entry: &[u8],
    components: &mut Vec<AssayComponent>,
) {
    let artifacts = collect_artifact_ids(index, repository_id, coordinate, entry).await;
    for (filename, artifact_id) in artifacts {
        let Ok(body) = storage.get(&storage_key_for(artifact_id)).await else {
            continue;
        };
        append_from_archive(coordinate.ecosystem(), &filename, &body, components);
        if components.len() >= MAX_COMPONENTS {
            break;
        }
    }
}

async fn collect_artifact_ids(
    index: &dyn PackageIndexStore,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
    entry: &[u8],
) -> Vec<(String, ArtifactId)> {
    let mut found = Vec::new();
    if let Ok(Some(artifact_id)) = index.artifact_for(repository_id, coordinate).await {
        found.push((String::new(), artifact_id));
    }
    if let Ok(value) = serde_json::from_slice::<Value>(entry)
        && let Some(files) = value.get("files").and_then(Value::as_array)
    {
        for file in files {
            let filename = file
                .get("filename")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let Some(id) = file.get("artifact_id").and_then(Value::as_str) else {
                continue;
            };
            let Ok(uuid) = Uuid::parse_str(id) else {
                continue;
            };
            let artifact_id = ArtifactId::from(uuid);
            if found.iter().any(|(_, existing)| *existing == artifact_id) {
                continue;
            }
            found.push((filename, artifact_id));
        }
    }
    found
}

fn append_from_archive(
    ecosystem: PackageEcosystem,
    filename: &str,
    body: &[u8],
    components: &mut Vec<AssayComponent>,
) {
    let is_wheel = std::path::Path::new(filename)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("whl"));
    let files = if is_wheel || body.starts_with(b"PK") {
        read_zip_map(body)
    } else {
        read_tar_map_lockfiles(body)
    };
    for (path, text) in files {
        let parsed = parse_lockfile(ecosystem, &path, &text);
        let kind = if is_metadata_path(&path) {
            AssayComponentKind::Direct
        } else {
            AssayComponentKind::Transitive
        };
        for (name, version) in parsed {
            if components.len() >= MAX_COMPONENTS {
                return;
            }
            let purl = if is_exact_version(&version) {
                purl_for(ecosystem, &name, &version)
            } else {
                None
            };
            merge_component(components, name, version, kind, purl);
        }
    }
}

fn parse_lockfile(
    ecosystem: PackageEcosystem,
    path: &str,
    text: &str,
) -> Vec<(String, String)> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match ecosystem {
        PackageEcosystem::Npm => match name {
            "package-lock.json" | "npm-shrinkwrap.json" => parse_package_lock(text),
            "yarn.lock" => parse_yarn_lock(text),
            "pnpm-lock.yaml" => parse_pnpm_lock(text),
            _ => Vec::new(),
        },
        PackageEcosystem::Cargo if name == "Cargo.lock" => parse_toml_packages(text),
        PackageEcosystem::PyPi => match name {
            "poetry.lock" | "uv.lock" => parse_toml_packages(text),
            "Pipfile.lock" => parse_pipfile_lock(text),
            "requirements.txt" | "requirements.lock" => parse_requirements(text),
            "METADATA" | "PKG-INFO" => parse_python_metadata(text),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn is_metadata_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(name, "METADATA" | "PKG-INFO")
}

fn is_lockfile_path(path: &str) -> bool {
    if path.contains("node_modules/") {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "Cargo.lock"
            | "poetry.lock"
            | "uv.lock"
            | "Pipfile.lock"
            | "requirements.txt"
            | "requirements.lock"
            | "METADATA"
            | "PKG-INFO"
    )
}

fn read_tar_map_lockfiles(body: &[u8]) -> BTreeMap<String, String> {
    if let Some(files) = read_tar_lockfiles(GzDecoder::new(Cursor::new(body))) {
        return files;
    }
    read_tar_lockfiles(Cursor::new(body)).unwrap_or_default()
}

fn read_tar_lockfiles<R: Read>(reader: R) -> Option<BTreeMap<String, String>> {
    let mut archive = Archive::new(reader);
    let mut files = BTreeMap::new();
    let entries = archive.entries().ok()?;
    for entry in entries.flatten() {
        let mut entry = entry;
        let Ok(path) = entry.path() else {
            continue;
        };
        let normalized = path.to_string_lossy().trim_start_matches("./").to_string();
        if !is_lockfile_path(&normalized) {
            continue;
        }
        if entry.size() > MAX_LOCKFILE_BYTES {
            continue;
        }
        let mut buf = String::new();
        if entry.read_to_string(&mut buf).is_err() {
            continue;
        }
        files.insert(normalized, buf);
    }
    Some(files)
}

fn read_zip_map(body: &[u8]) -> BTreeMap<String, String> {
    let Ok(mut archive) = ZipArchive::new(Cursor::new(body)) else {
        return BTreeMap::new();
    };
    let mut files = BTreeMap::new();
    for index in 0..archive.len() {
        let Ok(mut file) = archive.by_index(index) else {
            continue;
        };
        let name = file.name().trim_start_matches("./").to_string();
        if !is_lockfile_path(&name) || file.size() > MAX_LOCKFILE_BYTES {
            continue;
        }
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            continue;
        }
        files.insert(name, buf);
    }
    files
}

fn parse_package_lock(text: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    if let Some(packages) = value.get("packages").and_then(Value::as_object) {
        let mut deps = Vec::new();
        for (key, spec) in packages {
            if key.is_empty() {
                continue;
            }
            let Some(name) = npm_name_from_lock_key(key) else {
                continue;
            };
            let Some(version) = spec.get("version").and_then(Value::as_str) else {
                continue;
            };
            deps.push((name, version.to_string()));
        }
        return deps;
    }
    let mut deps = Vec::new();
    if let Some(tree) = value.get("dependencies") {
        walk_npm_v1(tree, &mut deps);
    }
    deps
}

fn npm_name_from_lock_key(key: &str) -> Option<String> {
    let rest = key.rsplit("node_modules/").next().unwrap_or(key);
    if rest.is_empty() || rest == key && !key.contains("node_modules/") {
        return None;
    }
    if rest.starts_with('@') {
        let mut parts = rest.split('/');
        let scope = parts.next()?;
        let name = parts.next()?;
        Some(format!("{scope}/{name}"))
    } else {
        Some(rest.split('/').next()?.to_string())
    }
}

fn walk_npm_v1(deps: &Value, out: &mut Vec<(String, String)>) {
    let Some(map) = deps.as_object() else {
        return;
    };
    for (name, spec) in map {
        if let Some(version) = spec.get("version").and_then(Value::as_str) {
            out.push((name.clone(), version.to_string()));
        }
        if let Some(nested) = spec.get("dependencies") {
            walk_npm_v1(nested, out);
        }
    }
}

fn parse_yarn_lock(text: &str) -> Vec<(String, String)> {
    if text.contains("__metadata:") {
        return Vec::new();
    }
    let mut current_names = Vec::new();
    let mut deps = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t']) && line.contains(':') {
            current_names = yarn_entry_names(line);
            continue;
        }
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix("version ") else {
            continue;
        };
        let version = unquote(value);
        for name in current_names.drain(..) {
            deps.push((name, version.clone()));
        }
    }
    deps
}

fn yarn_entry_names(header: &str) -> Vec<String> {
    header
        .trim()
        .trim_end_matches(':')
        .split(',')
        .filter_map(|part| yarn_package_name(part.trim().trim_matches('"')))
        .collect()
}

fn yarn_package_name(spec: &str) -> Option<String> {
    if spec.starts_with('@') {
        let rest = spec.trim_start_matches('@');
        let (scope, name_and_range) = rest.split_once('/')?;
        let name = name_and_range.split('@').next()?;
        Some(format!("@{scope}/{name}"))
    } else {
        let name = spec.split('@').next()?;
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }
}

fn parse_pnpm_lock(text: &str) -> Vec<(String, String)> {
    let mut in_packages = false;
    let mut deps = Vec::new();
    for line in text.lines() {
        if line.starts_with("packages:") {
            in_packages = true;
            continue;
        }
        if in_packages && !line.is_empty() && !line.starts_with([' ', '\t']) {
            in_packages = false;
        }
        if !in_packages {
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.ends_with(':') {
            continue;
        }
        let id = unquote(trimmed.trim_end_matches(':'));
        if let Some(pair) = pnpm_package_id(&id) {
            deps.push(pair);
        }
    }
    deps
}

fn pnpm_package_id(raw: &str) -> Option<(String, String)> {
    let raw = raw.trim().trim_start_matches('/');
    let raw = raw.split(['(', ':']).next()?.trim();
    if raw.starts_with('@') {
        let rest = raw.trim_start_matches('@');
        let (scope, namever) = rest.split_once('/')?;
        if let Some((name, version)) = namever.split_once('@') {
            return Some((format!("@{scope}/{name}"), version.to_string()));
        }
        let (name, version) = namever.split_once('/')?;
        return Some((format!("@{scope}/{name}"), version.to_string()));
    }
    raw.split_once('@')
        .or_else(|| raw.split_once('/'))
        .map(|(name, version)| (name.to_string(), version.to_string()))
        .filter(|(name, version)| !name.is_empty() && !version.is_empty())
}

fn parse_toml_packages(text: &str) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    let mut in_package = false;
    let mut name = None;
    let mut version = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_package && let (Some(package), Some(ver)) = (name.take(), version.take()) {
                deps.push((package, ver));
            }
            in_package = trimmed == "[[package]]";
            name = None;
            version = None;
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name") {
            name = toml_quoted_value(value);
        } else if let Some(value) = trimmed.strip_prefix("version") {
            version = toml_quoted_value(value);
        }
    }
    if in_package && let (Some(package), Some(ver)) = (name, version) {
        deps.push((package, ver));
    }
    deps
}

fn toml_quoted_value(rest: &str) -> Option<String> {
    let rest = rest.trim().trim_start_matches('=').trim();
    Some(unquote(rest)).filter(|value| !value.is_empty())
}

fn parse_pipfile_lock(text: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for section in ["default", "develop"] {
        let Some(map) = value.get(section).and_then(Value::as_object) else {
            continue;
        };
        for (name, spec) in map {
            let Some(version) = spec.get("version").and_then(Value::as_str) else {
                continue;
            };
            let version = version.trim().trim_start_matches("==").trim();
            if is_exact_version(version) {
                deps.push((name.clone(), version.to_string()));
            }
        }
    }
    deps
}

fn parse_requirements(text: &str) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', '-']) {
            continue;
        }
        let Some((name, rest)) = line.split_once("==") else {
            continue;
        };
        let name = name.split('[').next().unwrap_or(name).trim();
        let version = rest
            .split([';', ' ', '#', '\\'])
            .next()
            .unwrap_or(rest)
            .trim();
        if !name.is_empty() && is_exact_version(version) {
            deps.push((name.to_string(), version.to_string()));
        }
    }
    deps
}

fn parse_python_metadata(text: &str) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("Requires-Dist:") else {
            continue;
        };
        let rest = rest.trim();
        let name = rest
            .split([' ', ';', '[', '(', '<', '>', '=', '~', '!'])
            .next()
            .unwrap_or("")
            .trim();
        let Some((_, after_eq)) = rest.split_once("==") else {
            continue;
        };
        let version = after_eq
            .trim()
            .trim_start_matches('(')
            .split([')', ';', ' ', ','])
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() && is_exact_version(version) {
            deps.push((name.to_string(), version.to_string()));
        }
    }
    deps
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn package_lock_v3_lists_node_modules() {
        let text = r#"{
            "packages": {
                "": {"name": "demo"},
                "node_modules/lodash": {"version": "4.17.21"},
                "node_modules/@scope/pkg": {"version": "1.2.3"}
            }
        }"#;
        let deps = parse_package_lock(text);
        assert!(deps.contains(&("lodash".to_string(), "4.17.21".to_string())));
        assert!(deps.contains(&("@scope/pkg".to_string(), "1.2.3".to_string())));
    }

    #[test]
    fn cargo_lock_lists_packages() {
        let text = "[[package]]\nname = \"serde\"\nversion = \"1.0.210\"\n\n[[package]]\nname = \"demo\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            parse_toml_packages(text),
            vec![
                ("serde".to_string(), "1.0.210".to_string()),
                ("demo".to_string(), "0.1.0".to_string()),
            ]
        );
    }

    #[test]
    fn yarn_classic_reads_version_block() {
        let text = "lodash@^4.17.0:\n  version \"4.17.21\"\n";
        assert_eq!(
            parse_yarn_lock(text),
            vec![("lodash".to_string(), "4.17.21".to_string())]
        );
    }

    #[test]
    fn pnpm_lock_reads_package_ids() {
        let text = "packages:\n  /lodash@4.17.21:\n    resolution: {}\n  '@babel/core@7.24.0':\n    resolution: {}\n";
        let deps = parse_pnpm_lock(text);
        assert!(deps.contains(&("lodash".to_string(), "4.17.21".to_string())));
        assert!(deps.contains(&("@babel/core".to_string(), "7.24.0".to_string())));
    }

    #[test]
    fn requirements_and_metadata_keep_exact_pins() {
        assert_eq!(
            parse_requirements("requests==2.31.0\nflask>=2.0\n"),
            vec![("requests".to_string(), "2.31.0".to_string())]
        );
        assert_eq!(
            parse_python_metadata("Requires-Dist: requests (==2.31.0)\nRequires-Dist: flask (>=2.0)\n"),
            vec![("requests".to_string(), "2.31.0".to_string())]
        );
    }

    #[test]
    fn pipfile_lock_reads_default_section() {
        let text = r#"{"default": {"httpx": {"version": "==0.27.0"}}, "develop": {}}"#;
        assert_eq!(
            parse_pipfile_lock(text),
            vec![("httpx".to_string(), "0.27.0".to_string())]
        );
    }

    fn gzip_tar_file(path: &str, content: &str) -> Bytes {
        use std::io::Write;
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let data = content.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, data).unwrap();
            builder.finish().unwrap();
        }
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_buf).unwrap();
        Bytes::from(encoder.finish().unwrap())
    }

    #[tokio::test]
    async fn npm_tarball_lockfile_fills_exact_versions() {
        use ferrobox_domain::ids::ArtifactId;
        use ferrobox_domain::package_coordinate::{PackageName, PackageVersion};
        use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
        use ferrobox_ports::package_index_store::PackageIndexStore;
        use ferrobox_ports::storage::StoragePort;

        use crate::storage_key::storage_key_for;
        use crate::test_support::{InMemoryPackageIndexStore, InMemoryStorage};

        use super::super::layers::extract_inventory;

        let index = InMemoryPackageIndexStore::default();
        let storage = InMemoryStorage::default();
        let repository = Repository::new(
            RepositoryName::parse("npm-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Npm,
        )
        .unwrap();
        let tarball = gzip_tar_file(
            "package/package-lock.json",
            r#"{"packages":{"":{},"node_modules/lodash":{"version":"4.17.21"}}}"#,
        );
        let artifact_id = ArtifactId::new();
        storage
            .put(&storage_key_for(artifact_id), tarball)
            .await
            .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("demo").unwrap(),
            PackageVersion::parse("1.0.0").unwrap(),
        );
        let entry = Bytes::from(
            serde_json::json!({
                "name": "demo",
                "version": "1.0.0",
                "manifest": { "dependencies": { "lodash": "^4.17.0" } }
            })
            .to_string(),
        );
        index
            .upsert_entry(repository.id(), &coordinate, Some(artifact_id), entry.clone())
            .await
            .unwrap();

        let components = extract_inventory(
            &storage,
            &index,
            repository.id(),
            &coordinate,
            &entry,
        )
        .await;
        let lodash = components
            .iter()
            .find(|component| component.name() == "lodash")
            .unwrap();
        assert_eq!(lodash.version(), "4.17.21");
        assert_eq!(lodash.kind(), AssayComponentKind::Direct);
        assert_eq!(lodash.purl(), Some("pkg:npm/lodash@4.17.21"));
    }
}
