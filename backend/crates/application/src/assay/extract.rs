//! Extrae el inventario de componentes a partir de una entrada de índice.

use ferrobox_domain::assay::{AssayComponent, AssayComponentKind};
use ferrobox_domain::package_coordinate::PackageEcosystem;
use serde::Deserialize;
use serde_json::Value;

use super::licenses;

/// `true` si `spec` parece una versión concreta (`1.2.3`), no un rango.
#[must_use]
pub fn is_exact_version(spec: &str) -> bool {
    let spec = spec.trim();
    if spec.is_empty() {
        return false;
    }
    let Some(first) = spec.chars().next() else {
        return false;
    };
    if matches!(first, '^' | '~' | '*' | '<' | '>' | '=' | 'x' | 'X') {
        return false;
    }
    if spec.contains([' ', '|', ',', '*']) {
        return false;
    }
    spec.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+' | '_'))
        && spec.chars().any(|ch| ch.is_ascii_digit())
}

/// Construye un `purl` (*package URL*) para los ecosistemas que OSV
/// (*Open Source Vulnerabilities*) entiende.
#[must_use]
pub fn purl_for(ecosystem: PackageEcosystem, name: &str, version: &str) -> Option<String> {
    let encoded = match ecosystem {
        PackageEcosystem::Npm if name.starts_with('@') => {
            format!("%40{}", name.trim_start_matches('@'))
        }
        _ => name.to_string(),
    };
    if ecosystem == PackageEcosystem::Maven {
        let (group, artifact) = name.split_once(':')?;
        return Some(format!("pkg:maven/{group}/{artifact}@{version}"));
    }
    let prefix = match ecosystem {
        PackageEcosystem::Npm => "pkg:npm/",
        PackageEcosystem::PyPi => "pkg:pypi/",
        PackageEcosystem::Cargo => "pkg:cargo/",
        PackageEcosystem::Nuget => "pkg:nuget/",
        PackageEcosystem::Generic
        | PackageEcosystem::Oci
        | PackageEcosystem::Helm
        | PackageEcosystem::Conan
        | PackageEcosystem::Maven => return None,
    };
    Some(format!("{prefix}{encoded}@{version}"))
}

/// Ecosistema de OSV correspondiente, si el ensaye aplica.
#[must_use]
pub fn osv_ecosystem(ecosystem: PackageEcosystem) -> Option<&'static str> {
    match ecosystem {
        PackageEcosystem::Npm => Some("npm"),
        PackageEcosystem::PyPi => Some("PyPI"),
        PackageEcosystem::Cargo => Some("crates.io"),
        PackageEcosystem::Maven => Some("Maven"),
        PackageEcosystem::Nuget => Some("NuGet"),
        PackageEcosystem::Generic
        | PackageEcosystem::Oci
        | PackageEcosystem::Helm
        | PackageEcosystem::Conan => None,
    }
}

/// Ecosistema de OSV a consultar para un componente concreto.
///
/// Los paquetes de distro (Alpine, Debian, Ubuntu) van en el `purl`;
/// npm / `PyPI` / Cargo usan el ecosistema del repositorio.
#[must_use]
pub fn osv_query_target(
    component: &AssayComponent,
    repository_ecosystem: PackageEcosystem,
) -> Option<(&'static str, String, String)> {
    if let Some(purl) = component.purl()
        && let Some(ecosystem) = ecosystem_from_purl(purl)
    {
        return Some((
            ecosystem,
            component.name().to_string(),
            component.version().to_string(),
        ));
    }
    let ecosystem = osv_ecosystem(repository_ecosystem)?;
    if component.kind() != AssayComponentKind::Root && !is_exact_version(component.version()) {
        return None;
    }
    Some((
        ecosystem,
        component.name().to_string(),
        component.version().to_string(),
    ))
}

fn ecosystem_from_purl(purl: &str) -> Option<&'static str> {
    let rest = purl.strip_prefix("pkg:")?;
    if rest.starts_with("apk/wolfi/") {
        Some("Wolfi")
    } else if rest.starts_with("apk/chainguard/") {
        Some("Chainguard")
    } else if rest.starts_with("apk/alpaquita/") {
        Some("Alpaquita")
    } else if rest.starts_with("apk/minimos/") {
        Some("MinimOS")
    } else if rest.starts_with("apk/") {
        Some("Alpine")
    } else if rest.starts_with("deb/ubuntu/") {
        Some("Ubuntu")
    } else if rest.starts_with("deb/") {
        Some("Debian")
    } else if rest.starts_with("rpm/rocky/") {
        Some("Rocky Linux")
    } else if rest.starts_with("rpm/almalinux/") {
        Some("AlmaLinux")
    } else if rest.starts_with("rpm/azurelinux/") || rest.starts_with("rpm/mariner/") {
        Some("Azure Linux")
    } else if rest.starts_with("rpm/opensuse/") {
        Some("openSUSE")
    } else if rest.starts_with("rpm/suse/") {
        Some("SUSE")
    } else if rest.starts_with("rpm/photon/") {
        Some("Photon OS")
    } else if rest.starts_with("rpm/rhel/")
        || rest.starts_with("rpm/centos/")
        || rest.starts_with("rpm/redhat/")
        || rest.starts_with("rpm/ol/")
    {
        Some("Red Hat")
    } else {
        None
    }
}

/// Inserta o concreta un componente. Si ya había un rango declarado,
/// una versión exacta del lockfile la sustituye y conserva el papel
/// (`direct`). Las nuevas entradas de lockfile van como `transitive`.
pub(crate) fn merge_component(
    components: &mut Vec<AssayComponent>,
    name: String,
    version: String,
    kind: AssayComponentKind,
    purl: Option<String>,
) {
    if name.is_empty() || version.is_empty() {
        return;
    }
    if let Some(index) = components
        .iter()
        .position(|component| component.name().eq_ignore_ascii_case(&name))
    {
        let existing = &components[index];
        if existing.kind() == AssayComponentKind::Root || is_exact_version(existing.version()) {
            return;
        }
        if !is_exact_version(&version) {
            return;
        }
        let keep_kind = existing.kind();
        let licenses = existing.licenses().to_vec();
        let purl = purl.or_else(|| existing.purl().map(ToOwned::to_owned));
        components[index] = AssayComponent::new(name, version, purl, keep_kind).with_licenses(licenses);
        return;
    }
    components.push(AssayComponent::new(name, version, purl, kind));
}

/// Inventario a partir de la entrada de índice del ecosistema.
#[must_use]
pub fn extract_components(
    ecosystem: PackageEcosystem,
    name: &str,
    version: &str,
    entry: &[u8],
) -> Vec<AssayComponent> {
    let mut components = vec![AssayComponent::new(
        name,
        version,
        purl_for(ecosystem, name, version),
        AssayComponentKind::Root,
    )];

    match ecosystem {
        PackageEcosystem::Npm => {
            append_npm_deps(&mut components, entry);
            if let Ok(value) = serde_json::from_slice::<Value>(entry)
                && let Some(manifest) = value.get("manifest")
            {
                licenses::attach(&mut components, Some(name), licenses::from_npm_manifest(manifest));
            }
        }
        PackageEcosystem::Cargo => {
            append_cargo_deps(&mut components, entry);
            if let Ok(value) = serde_json::from_slice::<Value>(entry) {
                licenses::attach(&mut components, Some(name), licenses::from_cargo_index(&value));
            }
        }
        PackageEcosystem::PyPi
        | PackageEcosystem::Generic
        | PackageEcosystem::Oci
        | PackageEcosystem::Helm
        | PackageEcosystem::Conan
        | PackageEcosystem::Maven
        | PackageEcosystem::Nuget => {}
    }

    components
}

fn append_npm_deps(components: &mut Vec<AssayComponent>, entry: &[u8]) {
    let Ok(value) = serde_json::from_slice::<Value>(entry) else {
        return;
    };
    let Some(manifest) = value.get("manifest") else {
        return;
    };
    for key in ["dependencies", "optionalDependencies"] {
        let Some(Value::Object(map)) = manifest.get(key) else {
            continue;
        };
        for (dep_name, dep_spec) in map {
            let Some(spec) = dep_spec.as_str() else {
                continue;
            };
            let purl = if is_exact_version(spec) {
                purl_for(PackageEcosystem::Npm, dep_name, spec)
            } else {
                None
            };
            components.push(AssayComponent::new(
                dep_name.clone(),
                spec,
                purl,
                AssayComponentKind::Direct,
            ));
        }
    }
}

#[derive(Deserialize)]
struct CargoIndexEntry {
    #[serde(default)]
    deps: Vec<CargoIndexDep>,
}

#[derive(Deserialize)]
struct CargoIndexDep {
    name: String,
    req: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    package: Option<String>,
}

fn append_cargo_deps(components: &mut Vec<AssayComponent>, entry: &[u8]) {
    let Ok(parsed) = serde_json::from_slice::<CargoIndexEntry>(entry) else {
        return;
    };
    for dep in parsed.deps {
        if dep.kind == "dev" {
            continue;
        }
        let name = dep.package.unwrap_or(dep.name);
        components.push(AssayComponent::new(
            name,
            dep.req,
            None,
            AssayComponentKind::Direct,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_versions_are_queryable() {
        assert!(is_exact_version("4.17.20"));
        assert!(is_exact_version("1.0.0-beta.1"));
        assert!(!is_exact_version("^4.17.0"));
        assert!(!is_exact_version("~1.2.3"));
        assert!(!is_exact_version("*"));
        assert!(!is_exact_version(">=1.0"));
    }

    #[test]
    fn npm_extracts_declared_license_on_the_root() {
        let entry = serde_json::json!({
            "name": "demo",
            "version": "1.0.0",
            "manifest": { "license": "MIT", "dependencies": { "lodash": "4.17.20" } }
        });
        let components = extract_components(
            PackageEcosystem::Npm,
            "demo",
            "1.0.0",
            &serde_json::to_vec(&entry).unwrap(),
        );
        assert_eq!(components[0].licenses(), &["MIT".to_string()]);
        assert!(components[1].licenses().is_empty());
    }

    #[test]
    fn cargo_extracts_license_from_the_index_entry() {
        let entry = serde_json::json!({
            "name": "demo",
            "vers": "1.0.0",
            "license": "MIT OR Apache-2.0",
            "deps": []
        });
        let components = extract_components(
            PackageEcosystem::Cargo,
            "demo",
            "1.0.0",
            &serde_json::to_vec(&entry).unwrap(),
        );
        assert_eq!(
            components[0].licenses(),
            &["MIT".to_string(), "Apache-2.0".to_string()]
        );
    }

    #[test]
    fn npm_extracts_root_and_exact_direct_deps() {
        let entry = serde_json::json!({
            "name": "demo",
            "version": "1.0.0",
            "manifest": {
                "dependencies": {
                    "lodash": "4.17.20",
                    "left-pad": "^1.3.0"
                }
            }
        });
        let components = extract_components(
            PackageEcosystem::Npm,
            "demo",
            "1.0.0",
            &serde_json::to_vec(&entry).unwrap(),
        );
        assert_eq!(components.len(), 3);
        assert_eq!(components[0].name(), "demo");
        let lodash = components
            .iter()
            .find(|component| component.name() == "lodash")
            .unwrap();
        assert_eq!(lodash.purl(), Some("pkg:npm/lodash@4.17.20"));
        let ranged = components
            .iter()
            .find(|component| component.name() == "left-pad")
            .unwrap();
        assert!(ranged.purl().is_none());
    }

    #[test]
    fn cargo_extracts_normal_deps_and_skips_dev() {
        let entry = serde_json::json!({
            "name": "demo",
            "vers": "1.0.0",
            "deps": [
                {"name": "serde", "req": "^1.0", "kind": "normal"},
                {"name": "tokio", "req": "^1.0", "kind": "dev"}
            ]
        });
        let components = extract_components(
            PackageEcosystem::Cargo,
            "demo",
            "1.0.0",
            &serde_json::to_vec(&entry).unwrap(),
        );
        assert_eq!(components.len(), 2);
        assert_eq!(components[1].name(), "serde");
        assert_eq!(components[1].version(), "^1.0");
    }

    #[test]
    fn merge_component_upgrades_range_to_lockfile_pin() {
        let mut components = extract_components(
            PackageEcosystem::Npm,
            "demo",
            "1.0.0",
            &serde_json::to_vec(&serde_json::json!({
                "manifest": { "dependencies": { "lodash": "^4.17.0" } }
            }))
            .unwrap(),
        );
        merge_component(
            &mut components,
            "lodash".to_string(),
            "4.17.21".to_string(),
            AssayComponentKind::Transitive,
            purl_for(PackageEcosystem::Npm, "lodash", "4.17.21"),
        );
        let lodash = components
            .iter()
            .find(|component| component.name() == "lodash")
            .unwrap();
        assert_eq!(lodash.version(), "4.17.21");
        assert_eq!(lodash.kind(), AssayComponentKind::Direct);
        assert_eq!(lodash.purl(), Some("pkg:npm/lodash@4.17.21"));
    }

    #[test]
    fn maven_purl_uses_group_and_artifact() {
        assert_eq!(
            purl_for(
                PackageEcosystem::Maven,
                "org.apache.commons:commons-lang3",
                "3.14.0"
            ),
            Some("pkg:maven/org.apache.commons/commons-lang3@3.14.0".to_string())
        );
        assert_eq!(osv_ecosystem(PackageEcosystem::Maven), Some("Maven"));
    }

    #[test]
    fn nuget_purl_uses_package_id() {
        assert_eq!(
            purl_for(PackageEcosystem::Nuget, "Newtonsoft.Json", "13.0.3"),
            Some("pkg:nuget/Newtonsoft.Json@13.0.3".to_string())
        );
        assert_eq!(osv_ecosystem(PackageEcosystem::Nuget), Some("NuGet"));
    }

    #[test]
    fn osv_query_target_maps_distro_purls() {
        let wolfi = AssayComponent::new(
            "busybox",
            "1.36.1-r0",
            Some("pkg:apk/wolfi/busybox@1.36.1-r0".to_string()),
            AssayComponentKind::Direct,
        );
        let target = osv_query_target(&wolfi, PackageEcosystem::Oci).unwrap();
        assert_eq!(target.0, "Wolfi");
        let rocky = AssayComponent::new(
            "openssl",
            "1.1.1k-1.el8",
            Some("pkg:rpm/rocky/openssl@1.1.1k-1.el8".to_string()),
            AssayComponentKind::Direct,
        );
        assert_eq!(
            osv_query_target(&rocky, PackageEcosystem::Oci).unwrap().0,
            "Rocky Linux"
        );
        let arch = AssayComponent::new(
            "linux",
            "6.6.1-1",
            Some("pkg:alpm/arch/linux@6.6.1-1".to_string()),
            AssayComponentKind::Direct,
        );
        assert!(osv_query_target(&arch, PackageEcosystem::Oci).is_none());
    }
}
