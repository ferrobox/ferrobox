//! Inventory from OCI layers, Helm charts, and Conan recipes.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use bytes::Bytes;
use ferrobox_domain::assay::{AssayComponent, AssayComponentKind};
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName, PackageVersion};
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::storage::StoragePort;
use flate2::read::GzDecoder;
use serde::Deserialize;
use tar::Archive;
use uuid::Uuid;

use super::extract::{extract_components, merge_component};
use super::licenses;
use super::lockfiles::append_from_stored_package;
use crate::storage_key::storage_key_for;

const BLOB_PACKAGE: &str = "_blob";

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    #[serde(default)]
    artifact_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OciManifest {
    #[serde(default)]
    layers: Vec<OciDescriptor>,
    #[serde(default)]
    manifests: Vec<OciDescriptor>,
    #[serde(default)]
    config: Option<OciDescriptor>,
}

#[derive(Debug, Deserialize)]
struct OciDescriptor {
    digest: String,
    #[serde(default)]
    #[serde(rename = "mediaType")]
    media_type: String,
    #[serde(default)]
    platform: Option<OciPlatform>,
}

#[derive(Debug, Deserialize)]
struct OciPlatform {
    #[serde(default)]
    architecture: String,
    #[serde(default)]
    os: String,
}

#[derive(Debug, Deserialize)]
struct RecipeEntry {
    #[serde(default)]
    files: Vec<RecipeFile>,
    #[serde(default)]
    revisions: Vec<RecipeRevision>,
}

#[derive(Debug, Deserialize)]
struct RecipeFile {
    filename: String,
    artifact_id: String,
}

#[derive(Debug, Deserialize)]
struct RecipeRevision {
    #[serde(default)]
    files: Vec<RecipeFile>,
}

#[derive(Default)]
struct LayerFiles {
    apk_installed: Option<String>,
    dpkg_status: Option<String>,
    os_release: Option<String>,
    chart_yaml: Option<String>,
    helm_config: Option<String>,
    values_yaml: Option<String>,
    rpm_manifest: Option<String>,
    pacman_descs: Vec<String>,
}

/// Package inventory, including layers or recipe when they apply.
pub async fn extract_inventory(
    storage: &dyn StoragePort,
    index: &dyn PackageIndexStore,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
    entry: &[u8],
) -> Vec<AssayComponent> {
    match coordinate.ecosystem() {
        PackageEcosystem::Npm | PackageEcosystem::PyPi | PackageEcosystem::Cargo => {
            let mut components = extract_components(
                coordinate.ecosystem(),
                coordinate.name().as_str(),
                coordinate.version().as_str(),
                entry,
            );
            append_from_stored_package(
                storage,
                index,
                repository_id,
                coordinate,
                entry,
                &mut components,
            )
            .await;
            components
        }
        PackageEcosystem::Oci | PackageEcosystem::Helm => {
            extract_oci(storage, index, repository_id, coordinate, entry).await
        }
        PackageEcosystem::Conan => extract_conan(storage, coordinate, entry).await,
        PackageEcosystem::Generic
        | PackageEcosystem::Maven
        | PackageEcosystem::Nuget
        | PackageEcosystem::Go => {
            extract_components(
                coordinate.ecosystem(),
                coordinate.name().as_str(),
                coordinate.version().as_str(),
                entry,
            )
        }
    }
}

async fn extract_oci(
    storage: &dyn StoragePort,
    index: &dyn PackageIndexStore,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
    entry: &[u8],
) -> Vec<AssayComponent> {
    let mut components = vec![root_component(coordinate)];
    let Some(manifest_bytes) = load_manifest_bytes(storage, index, repository_id, coordinate, entry).await
    else {
        return components;
    };
    let Ok(manifest) = serde_json::from_slice::<OciManifest>(&manifest_bytes) else {
        return components;
    };

    let mut files = LayerFiles::default();
    if let Some(config) = &manifest.config
        && let Some(body) = load_blob(storage, index, repository_id, coordinate.ecosystem(), &config.digest)
            .await
        && config.media_type.contains("helm")
        && let Ok(text) = std::str::from_utf8(&body)
    {
        files.helm_config = Some(text.to_string());
    }

    for layer in &manifest.layers {
        let Some(body) =
            load_blob(storage, index, repository_id, coordinate.ecosystem(), &layer.digest).await
        else {
            continue;
        };
        merge_layer(&mut files, &body);
    }

    append_os_packages(&mut components, &files);
    append_helm_chart(&mut components, &files);
    append_helm_images(&mut components, &files);
    components
}

async fn extract_conan(
    storage: &dyn StoragePort,
    coordinate: &PackageCoordinate,
    entry: &[u8],
) -> Vec<AssayComponent> {
    let mut components = vec![root_component(coordinate)];
    let Ok(recipe) = serde_json::from_slice::<RecipeEntry>(entry) else {
        return components;
    };
    let mut files: Vec<&RecipeFile> = recipe.files.iter().collect();
    if let Some(revision) = recipe.revisions.last() {
        files.extend(revision.files.iter());
    }
    for file in files {
        let Some(body) = load_artifact(storage, &file.artifact_id).await else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&body) else {
            continue;
        };
        if file.filename.ends_with("conanfile.py") {
            licenses::attach(&mut components, None, licenses::from_conanfile_py(text));
        }
        let parsed = if file.filename.ends_with("conanfile.txt") {
            parse_conanfile_txt(text)
        } else if file.filename.ends_with("conanfile.py") {
            parse_conanfile_py(text)
        } else {
            Vec::new()
        };
        for (name, version) in parsed {
            if components
                .iter()
                .any(|component| component.name() == name && component.version() == version)
            {
                continue;
            }
            components.push(AssayComponent::new(
                name.clone(),
                version.clone(),
                Some(format!("pkg:conan/{name}@{version}")),
                AssayComponentKind::Direct,
            ));
        }
    }
    components
}

fn root_component(coordinate: &PackageCoordinate) -> AssayComponent {
    AssayComponent::new(
        coordinate.name().as_str(),
        coordinate.version().as_str(),
        None,
        AssayComponentKind::Root,
    )
}

async fn load_manifest_bytes(
    storage: &dyn StoragePort,
    index: &dyn PackageIndexStore,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
    entry: &[u8],
) -> Option<Bytes> {
    let parsed: ManifestEntry = serde_json::from_slice(entry).ok()?;
    let mut body = match parsed.artifact_id.as_deref() {
        Some(id) => load_artifact(storage, id).await?,
        None => return None,
    };
    let Ok(manifest) = serde_json::from_slice::<OciManifest>(&body) else {
        return Some(body);
    };
    if manifest.layers.is_empty() && !manifest.manifests.is_empty() {
        let digest = pick_platform_digest(&manifest.manifests)?;
        if let Some(child) =
            load_digest_manifest(storage, index, repository_id, coordinate, &digest).await
        {
            body = child;
        }
    }
    Some(body)
}

fn pick_platform_digest(manifests: &[OciDescriptor]) -> Option<String> {
    manifests
        .iter()
        .find(|descriptor| {
            descriptor
                .platform
                .as_ref()
                .is_some_and(|platform| platform.os == "linux" && platform.architecture == "amd64")
        })
        .or_else(|| manifests.first())
        .map(|descriptor| descriptor.digest.clone())
}

async fn load_digest_manifest(
    storage: &dyn StoragePort,
    index: &dyn PackageIndexStore,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
    digest: &str,
) -> Option<Bytes> {
    if let Some(body) = load_blob(storage, index, repository_id, coordinate.ecosystem(), digest).await
    {
        return Some(body);
    }
    let entries = index
        .entries_for_package(repository_id, coordinate.ecosystem(), coordinate.name())
        .await
        .ok()?;
    for entry in entries {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&entry) else {
            continue;
        };
        if value.get("reference").and_then(serde_json::Value::as_str) != Some(digest) {
            continue;
        }
        if let Some(id) = value.get("artifact_id").and_then(serde_json::Value::as_str) {
            return load_artifact(storage, id).await;
        }
    }
    None
}

async fn load_blob(
    storage: &dyn StoragePort,
    index: &dyn PackageIndexStore,
    repository_id: RepositoryId,
    ecosystem: PackageEcosystem,
    digest: &str,
) -> Option<Bytes> {
    let name = PackageName::parse(BLOB_PACKAGE).ok()?;
    let version = PackageVersion::parse(digest).ok()?;
    let coordinate = PackageCoordinate::new(ecosystem, name, version);
    let artifact_id = index
        .artifact_for(repository_id, &coordinate)
        .await
        .ok()
        .flatten()?;
    storage.get(&storage_key_for(artifact_id)).await.ok()
}

async fn load_artifact(storage: &dyn StoragePort, artifact_id: &str) -> Option<Bytes> {
    let id = Uuid::parse_str(artifact_id).ok()?;
    storage
        .get(&storage_key_for(ArtifactId::from(id)))
        .await
        .ok()
}

fn merge_layer(files: &mut LayerFiles, body: &[u8]) {
    let entries = read_layer_files(body);
    for (path, text) in entries {
        match path.as_str() {
            "lib/apk/db/installed" => files.apk_installed = Some(text),
            "var/lib/dpkg/status" => files.dpkg_status = Some(text),
            "etc/os-release" | "usr/lib/os-release" => files.os_release = Some(text),
            "var/lib/rpmmanifest/container-manifest-2" => files.rpm_manifest = Some(text),
            other if other.ends_with("Chart.yaml") || other.ends_with("Chart.yml") => {
                files.chart_yaml = Some(text);
            }
            other if other.ends_with("values.yaml") || other.ends_with("values.yml") => {
                files.values_yaml = Some(text);
            }
            other if other.contains("var/lib/pacman/local/") && other.ends_with("/desc") => {
                files.pacman_descs.push(text);
            }
            _ => {}
        }
    }
}

fn read_layer_files(body: &[u8]) -> BTreeMap<String, String> {
    if let Some(files) = read_tar_map(GzDecoder::new(Cursor::new(body))) {
        return files;
    }
    read_tar_map(Cursor::new(body)).unwrap_or_default()
}

fn read_tar_map<R: Read>(reader: R) -> Option<BTreeMap<String, String>> {
    let mut archive = Archive::new(reader);
    let mut files = BTreeMap::new();
    let entries = archive.entries().ok()?;
    for entry in entries.flatten() {
        let mut entry = entry;
        let Ok(path) = entry.path() else {
            continue;
        };
        let normalized = path.to_string_lossy().trim_start_matches("./").to_string();
        if !is_interesting_path(&normalized) {
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

fn is_interesting_path(path: &str) -> bool {
    matches!(
        path,
        "lib/apk/db/installed"
            | "var/lib/dpkg/status"
            | "etc/os-release"
            | "usr/lib/os-release"
            | "var/lib/rpmmanifest/container-manifest-2"
    ) || path.ends_with("Chart.yaml")
        || path.ends_with("Chart.yml")
        || path.ends_with("values.yaml")
        || path.ends_with("values.yml")
        || (path.contains("var/lib/pacman/local/") && path.ends_with("/desc"))
}

fn append_os_packages(components: &mut Vec<AssayComponent>, files: &LayerFiles) {
    let distro = os_release_id(files.os_release.as_deref());
    if let Some(text) = &files.apk_installed {
        let namespace = apk_purl_namespace(&distro);
        for (name, version, declared) in parse_apk_installed(text) {
            merge_component(
                components,
                name.clone(),
                version.clone(),
                AssayComponentKind::Direct,
                Some(format!("pkg:apk/{namespace}/{name}@{version}")),
            );
            licenses::attach(components, Some(&name), declared);
        }
    }
    if let Some(text) = &files.dpkg_status {
        let namespace = if distro == "ubuntu" { "ubuntu" } else { "debian" };
        for (name, version) in parse_dpkg_status(text) {
            merge_component(
                components,
                name.clone(),
                version.clone(),
                AssayComponentKind::Direct,
                Some(format!("pkg:deb/{namespace}/{name}@{version}")),
            );
        }
    }
    if let Some(text) = &files.rpm_manifest {
        let namespace = rpm_purl_namespace(&distro);
        for (name, version) in parse_rpm_manifest(text) {
            merge_component(
                components,
                name.clone(),
                version.clone(),
                AssayComponentKind::Direct,
                Some(format!("pkg:rpm/{namespace}/{name}@{version}")),
            );
        }
    }
    for text in &files.pacman_descs {
        if let Some((name, version, declared)) = parse_pacman_desc(text) {
            merge_component(
                components,
                name.clone(),
                version.clone(),
                AssayComponentKind::Direct,
                Some(format!("pkg:alpm/arch/{name}@{version}")),
            );
            licenses::attach(components, Some(&name), declared);
        }
    }
}

fn append_helm_chart(components: &mut Vec<AssayComponent>, files: &LayerFiles) {
    let text = files
        .chart_yaml
        .as_deref()
        .or(files.helm_config.as_deref());
    let Some(text) = text else {
        return;
    };
    licenses::attach(components, None, licenses::from_chart_yaml(text));
    for (name, version) in parse_chart_dependencies(text) {
        merge_component(
            components,
            name.clone(),
            version.clone(),
            AssayComponentKind::Direct,
            Some(format!("pkg:helm/{name}@{version}")),
        );
    }
}

fn append_helm_images(components: &mut Vec<AssayComponent>, files: &LayerFiles) {
    let mut images = Vec::new();
    if let Some(text) = files.chart_yaml.as_deref() {
        images.extend(parse_helm_images(text));
    }
    if let Some(text) = files.values_yaml.as_deref() {
        images.extend(parse_helm_images(text));
    }
    if let Some(text) = files.helm_config.as_deref() {
        images.extend(parse_helm_images(text));
    }
    for (name, version) in images {
        let purl = Some(format!("pkg:oci/{name}@{version}"));
        merge_component(
            components,
            name,
            version,
            AssayComponentKind::Direct,
            purl,
        );
    }
}

fn apk_purl_namespace(distro: &str) -> &'static str {
    match distro {
        "wolfi" => "wolfi",
        "chainguard" => "chainguard",
        "alpaquita" => "alpaquita",
        "minimos" => "minimos",
        _ => "alpine",
    }
}

fn rpm_purl_namespace(distro: &str) -> &str {
    match distro {
        "rhel" | "centos" | "ol" | "redhat" | "debian" | "ubuntu" | "alpine" | "wolfi" => "rhel",
        "rocky" => "rocky",
        "almalinux" | "alma" => "almalinux",
        "azurelinux" | "mariner" => "azurelinux",
        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" => "opensuse",
        "sles" | "suse" => "suse",
        "photon" => "photon",
        other => other,
    }
}

fn os_release_id(text: Option<&str>) -> String {
    let Some(text) = text else {
        return "debian".to_string();
    };
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            return value.trim().trim_matches('"').to_ascii_lowercase();
        }
    }
    "debian".to_string()
}

fn parse_apk_installed(text: &str) -> Vec<(String, String, Vec<String>)> {
    let mut name = None;
    let mut version = None;
    let mut declared = Vec::new();
    let mut packages = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            if let (Some(package), Some(ver)) = (name.take(), version.take()) {
                packages.push((package, ver, std::mem::take(&mut declared)));
            }
            declared.clear();
            continue;
        }
        if let Some(value) = line.strip_prefix("P:") {
            name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("V:") {
            version = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("L:") {
            declared = licenses::split_declared(value);
        }
    }
    if let (Some(package), Some(ver)) = (name, version) {
        packages.push((package, ver, declared));
    }
    packages
}

fn parse_dpkg_status(text: &str) -> Vec<(String, String)> {
    let mut name = None;
    let mut version = None;
    let mut installed = false;
    let mut packages = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            if installed && let (Some(package), Some(ver)) = (name.take(), version.take()) {
                packages.push((package, ver));
            }
            name = None;
            version = None;
            installed = false;
            continue;
        }
        if let Some(value) = line.strip_prefix("Package: ") {
            name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("Version: ") {
            version = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("Status: ") {
            installed = value.contains("install ok installed");
        }
    }
    if installed && let (Some(package), Some(ver)) = (name, version) {
        packages.push((package, ver));
    }
    packages
}

fn parse_chart_dependencies(text: &str) -> Vec<(String, String)> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        return json_chart_deps(&value);
    }
    yaml_chart_deps(text)
}

fn json_chart_deps(value: &serde_json::Value) -> Vec<(String, String)> {
    let Some(deps) = value.get("dependencies").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    deps.iter()
        .filter_map(|dep| {
            Some((
                dep.get("name")?.as_str()?.to_string(),
                dep.get("version")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn yaml_chart_deps(text: &str) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    let mut current_name: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_deps = trimmed.starts_with("dependencies:");
            current_name = None;
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some(value) = trimmed
            .strip_prefix("- name:")
            .or_else(|| trimmed.strip_prefix("name:"))
        {
            current_name = Some(unquote(value));
        } else if let Some(value) = trimmed.strip_prefix("version:")
            && let Some(name) = current_name.take()
        {
            deps.push((name, unquote(value)));
        }
    }
    deps
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn parse_rpm_manifest(text: &str) -> Vec<(String, String)> {
    let mut packages = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[0].trim();
        if name.is_empty() || name == "gpg-pubkey" {
            continue;
        }
        let epoch = parts[1].trim();
        let version = parts[2].trim();
        let release = parts[3].trim();
        if version.is_empty() {
            continue;
        }
        let full = if epoch.is_empty() || epoch == "0" {
            format!("{version}-{release}")
        } else {
            format!("{epoch}:{version}-{release}")
        };
        packages.push((name.to_string(), full));
    }
    packages
}

fn parse_pacman_desc(text: &str) -> Option<(String, String, Vec<String>)> {
    let mut name = None;
    let mut version = None;
    let mut declared = Vec::new();
    let mut section = "";
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('%') && trimmed.ends_with('%') {
            section = trimmed;
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        match section {
            "%NAME%" => name = Some(trimmed.to_string()),
            "%VERSION%" => version = Some(trimmed.to_string()),
            "%LICENSE%" => declared.extend(licenses::split_declared(trimmed)),
            _ => {}
        }
    }
    match (name, version) {
        (Some(name), Some(version)) if !name.is_empty() && !version.is_empty() => {
            Some((name, version, declared))
        }
        _ => None,
    }
}

fn parse_helm_images(text: &str) -> Vec<(String, String)> {
    let mut images = Vec::new();
    let mut pending_repository: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches('-').trim();
        if trimmed.contains("{{") || trimmed.contains("${") {
            pending_repository = None;
            continue;
        }
        if let Some(value) = trimmed
            .strip_prefix("image:")
            .or_else(|| trimmed.strip_prefix("image :"))
        {
            if let Some(pair) = split_image_ref(&unquote(value)) {
                images.push(pair);
            }
            pending_repository = None;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("repository:") {
            pending_repository = Some(unquote(value));
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("tag:")
            && let Some(repository) = pending_repository.take()
        {
            let tag = unquote(value);
            if let Some(pair) = image_from_repository_tag(&repository, &tag) {
                images.push(pair);
            }
        }
    }
    images
}

fn split_image_ref(raw: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains(' ') || raw.starts_with('{') {
        return None;
    }
    let without_digest = raw.split('@').next().unwrap_or(raw);
    let (repo, tag) = without_digest.rsplit_once(':')?;
    image_from_repository_tag(repo, tag)
}

fn image_from_repository_tag(repository: &str, tag: &str) -> Option<(String, String)> {
    let repository = repository
        .trim()
        .trim_start_matches("docker.io/")
        .trim_start_matches("library/");
    let tag = tag.trim();
    if repository.is_empty() || tag.is_empty() || tag.contains(['{', '/', ' ']) {
        return None;
    }
    if repository.contains('{') {
        return None;
    }
    let name = repository.rsplit('/').next().unwrap_or(repository);
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), tag.to_string()))
}

fn parse_conanfile_txt(text: &str) -> Vec<(String, String)> {
    let mut in_requires = false;
    let mut deps = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_requires = trimmed.eq_ignore_ascii_case("[requires]");
            continue;
        }
        if !in_requires || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(pair) = split_conan_ref(trimmed) {
            deps.push(pair);
        }
    }
    deps
}

fn parse_conanfile_py(text: &str) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    for line in text.lines() {
        let Some(offset) = line.find("requires(") else {
            continue;
        };
        let rest = &line[offset + 9..];
        let quote = rest.find('"').or_else(|| rest.find('\''));
        let Some(quote_at) = quote else {
            continue;
        };
        let quote_char = rest.as_bytes()[quote_at];
        let inner = &rest[quote_at + 1..];
        let Some(end) = inner.bytes().position(|byte| byte == quote_char) else {
            continue;
        };
        if let Some(pair) = split_conan_ref(&inner[..end]) {
            deps.push(pair);
        }
    }
    deps
}

fn split_conan_ref(spec: &str) -> Option<(String, String)> {
    let spec = spec.split(['#', '@']).next()?.trim();
    let (name, version) = spec.split_once('/')?;
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apk_installed_parses_name_version_and_license() {
        let text = "P:busybox\nV:1.36.1-r19\nL:GPL-2.0-only\n\nP:musl\nV:1.2.4-r2\nL:MIT\n";
        assert_eq!(
            parse_apk_installed(text),
            vec![
                (
                    "busybox".to_string(),
                    "1.36.1-r19".to_string(),
                    vec!["GPL-2.0-only".to_string()]
                ),
                (
                    "musl".to_string(),
                    "1.2.4-r2".to_string(),
                    vec!["MIT".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn dpkg_status_skips_not_installed() {
        let text = "Package: libc6\nStatus: install ok installed\nVersion: 2.35-0ubuntu3\n\nPackage: old\nStatus: deinstall ok config-files\nVersion: 1.0\n";
        assert_eq!(
            parse_dpkg_status(text),
            vec![("libc6".to_string(), "2.35-0ubuntu3".to_string())]
        );
    }

    #[test]
    fn chart_yaml_lists_dependencies() {
        let text = "name: demo\nversion: 1.0.0\ndependencies:\n  - name: postgresql\n    version: \"12.5.6\"\n";
        assert_eq!(
            parse_chart_dependencies(text),
            vec![("postgresql".to_string(), "12.5.6".to_string())]
        );
    }

    #[test]
    fn conanfile_txt_reads_requires() {
        let text = "[requires]\nzlib/1.2.13\nopenssl/3.1.0@conan/stable\n";
        assert_eq!(
            parse_conanfile_txt(text),
            vec![
                ("zlib".to_string(), "1.2.13".to_string()),
                ("openssl".to_string(), "3.1.0".to_string()),
            ]
        );
    }

    #[test]
    fn conanfile_py_reads_self_requires() {
        let text = "def requirements(self):\n    self.requires(\"zlib/1.2.13\")\n";
        assert_eq!(
            parse_conanfile_py(text),
            vec![("zlib".to_string(), "1.2.13".to_string())]
        );
    }

    #[test]
    fn rpm_manifest_skips_gpg_and_joins_epoch() {
        let text = "gpg-pubkey|0|abc|1|x86_64\nopenssl|1|1.1.1k|1.el8|x86_64\ncurl|0|7.61.1|14.el8|x86_64\n";
        assert_eq!(
            parse_rpm_manifest(text),
            vec![
                ("openssl".to_string(), "1:1.1.1k-1.el8".to_string()),
                ("curl".to_string(), "7.61.1-14.el8".to_string()),
            ]
        );
    }

    #[test]
    fn pacman_desc_reads_name_version_and_license() {
        let text = "%NAME%\nlinux\n\n%VERSION%\n6.6.1-1\n\n%LICENSE%\nGPL2\n";
        assert_eq!(
            parse_pacman_desc(text),
            Some((
                "linux".to_string(),
                "6.6.1-1".to_string(),
                vec!["GPL2".to_string()]
            ))
        );
    }

    #[test]
    fn helm_values_collect_image_and_repository_tag() {
        let text = "image: nginx:1.25.3\napp:\n  repository: bitnami/postgresql\n  tag: \"16.1.0\"\nignored: \"{{ .Values.image }}\"\n";
        let images = parse_helm_images(text);
        assert!(images.contains(&("nginx".to_string(), "1.25.3".to_string())));
        assert!(images.contains(&("postgresql".to_string(), "16.1.0".to_string())));
        assert!(!images.iter().any(|(name, _)| name.contains('{')));
    }
}
