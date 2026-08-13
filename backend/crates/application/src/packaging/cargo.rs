//! Estrategia de empaquetado para el ecosistema Cargo: implementa el
//! subconjunto del protocolo de índice disperso (*sparse index*) que
//! `cargo` necesita para publicar paquetes (`cargo publish`) y resolver
//! sus dependencias (`cargo build`, `cargo add`) contra un repositorio
//! `FerroBox`.
//!
//! Referencia del protocolo:
//! <https://doc.rust-lang.org/cargo/reference/registry-index.html>
//! y <https://doc.rust-lang.org/cargo/reference/registries.html>.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::checksum::Sha256Checksum;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::Repository;
use ferrobox_ports::artifact_store::ArtifactStore;
use ferrobox_ports::package_index_store::PackageIndexStore;
use ferrobox_ports::storage::StoragePort;
use serde::{Deserialize, Serialize};

use super::{PackagingError, PackagingStrategy, PublishOutcome};
use crate::content_hash::sha256_checksum;
use crate::storage_key::storage_key_for;

/// Estrategia de empaquetado para el ecosistema Cargo.
pub struct CargoPackagingStrategy {
    artifact_store: Arc<dyn ArtifactStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    storage: Arc<dyn StoragePort>,
}

impl CargoPackagingStrategy {
    /// Construye la estrategia a partir de sus puertos.
    #[must_use]
    pub fn new(
        artifact_store: Arc<dyn ArtifactStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        storage: Arc<dyn StoragePort>,
    ) -> Self {
        Self {
            artifact_store,
            package_index_store,
            storage,
        }
    }

    fn ensure_cargo_repository(repository: &Repository) -> Result<(), PackagingError> {
        if repository.ecosystem() != PackageEcosystem::Cargo {
            return Err(PackagingError::EcosystemMismatch {
                expected: "cargo",
                actual: repository.ecosystem().label(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl PackagingStrategy for CargoPackagingStrategy {
    fn ecosystem(&self) -> PackageEcosystem {
        PackageEcosystem::Cargo
    }

    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError> {
        Self::ensure_cargo_repository(repository)?;

        let parsed = parse_publish_payload(payload)?;

        let name = PackageName::parse(parsed.metadata.name.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let version = PackageVersion::parse(parsed.metadata.vers.clone())
            .map_err(|err| PackagingError::InvalidPayload(err.to_string()))?;
        let coordinate = PackageCoordinate::new(PackageEcosystem::Cargo, name, version);

        if self
            .package_index_store
            .artifact_for(repository.id(), &coordinate)
            .await?
            .is_some()
        {
            return Err(PackagingError::AlreadyPublished(coordinate));
        }

        let artifact = Artifact::new(
            repository.id(),
            parsed.checksum.clone(),
            parsed.crate_bytes.len() as u64,
        );

        self.storage
            .put(&storage_key_for(artifact.id()), parsed.crate_bytes)
            .await?;
        self.artifact_store.save(&artifact).await?;

        let entry = IndexEntry::from_publish_metadata(&parsed.metadata, parsed.checksum.as_str());
        let entry_bytes = Bytes::from(
            serde_json::to_vec(&entry).expect("an IndexEntry always serializes to valid JSON"),
        );

        self.package_index_store
            .upsert_entry(repository.id(), &coordinate, artifact.id(), entry_bytes)
            .await?;

        Ok(coordinate)
    }

    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_cargo_repository(repository)?;

        let entries = self
            .package_index_store
            .entries_for_package(repository.id(), PackageEcosystem::Cargo, name)
            .await?;

        if entries.is_empty() {
            return Err(PackagingError::PackageNotFound(name.to_string()));
        }

        let mut body = BytesMut::new();
        for entry in entries {
            body.extend_from_slice(&entry);
            body.extend_from_slice(b"\n");
        }

        Ok(body.freeze())
    }

    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError> {
        Self::ensure_cargo_repository(repository)?;

        let artifact_id = self
            .package_index_store
            .artifact_for(repository.id(), coordinate)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;

        let artifact = self
            .artifact_store
            .find_by_id(artifact_id)
            .await?
            .ok_or_else(|| PackagingError::VersionNotFound(coordinate.clone()))?;

        let content = self.storage.get(&storage_key_for(artifact_id)).await?;
        let actual = sha256_checksum(&content);
        if actual != *artifact.checksum() {
            return Err(PackagingError::ChecksumMismatch {
                expected: artifact.checksum().to_string(),
                actual: actual.to_string(),
            });
        }

        Ok(content)
    }
}

/// Calcula la ruta relativa -- bajo la raíz del índice disperso -- en la
/// que `cargo` espera encontrar las entradas de índice de un paquete,
/// siguiendo las reglas oficiales de fragmentación (*sharding*) del
/// protocolo de índice disperso: nombres de 1 y 2 caracteres viven en
/// `1/` y `2/` respectivamente; los de 3 caracteres, en `3/{primera
/// letra}/`; y el resto, en `{dos primeras letras}/{siguientes dos
/// letras}/`. `cargo` siempre normaliza el nombre a minúsculas al
/// calcular esta ruta.
#[must_use]
pub fn cargo_index_shard_path(name: &PackageName) -> String {
    let lower = name.as_str().to_ascii_lowercase();
    match lower.len() {
        1 => format!("1/{lower}"),
        2 => format!("2/{lower}"),
        3 => format!("3/{}/{lower}", &lower[0..1]),
        _ => format!("{}/{}/{lower}", &lower[0..2], &lower[2..4]),
    }
}

/// Cuerpo JSON que `cargo publish` envía como primer bloque de la
/// petición de publicación. Solo se modelan los campos que
/// `FerroBox` necesita para construir la entrada de índice; el resto
/// (autores, descripción, licencia...) se ignoran silenciosamente
/// gracias al comportamiento por defecto de `serde` ante campos
/// desconocidos.
#[derive(Debug, Deserialize)]
struct PublishMetadata {
    name: String,
    vers: String,
    #[serde(default)]
    deps: Vec<PublishDependency>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    links: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublishDependency {
    name: String,
    version_req: String,
    #[serde(default)]
    features: Vec<String>,
    optional: bool,
    default_features: bool,
    #[serde(default)]
    target: Option<String>,
    kind: String,
    #[serde(default)]
    registry: Option<String>,
    #[serde(default)]
    explicit_name_in_toml: Option<String>,
}

/// Una entrada del índice disperso de Cargo: una línea JSON que describe
/// una versión publicada de un paquete.
///
/// Formato oficial:
/// <https://doc.rust-lang.org/cargo/reference/registry-index.html#json-schema>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    name: String,
    vers: String,
    deps: Vec<IndexDependency>,
    cksum: String,
    features: BTreeMap<String, Vec<String>>,
    yanked: bool,
    links: Option<String>,
    v: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IndexDependency {
    name: String,
    req: String,
    features: Vec<String>,
    optional: bool,
    default_features: bool,
    target: Option<String>,
    kind: String,
    registry: Option<String>,
    package: Option<String>,
}

impl IndexEntry {
    fn from_publish_metadata(metadata: &PublishMetadata, cksum: &str) -> Self {
        Self {
            name: metadata.name.clone(),
            vers: metadata.vers.clone(),
            deps: metadata.deps.iter().map(IndexDependency::from).collect(),
            cksum: cksum.to_string(),
            features: metadata.features.clone(),
            yanked: false,
            links: metadata.links.clone(),
            // Se fija deliberadamente en 1: esta versión de esquema le
            // indica a `cargo` que no busque un campo `features2`, que
            // esta estrategia nunca genera (solo es necesario para
            // "weak dependency features", fuera del alcance de esta
            // primera implementación).
            v: 1,
        }
    }
}

impl From<&PublishDependency> for IndexDependency {
    fn from(dep: &PublishDependency) -> Self {
        // Si la dependencia se renombró en el `Cargo.toml` del
        // publicador (`foo = { package = "bar", version = "1" }`), el
        // índice debe reflejar el nombre local bajo `name` y el nombre
        // real del paquete bajo `package` -- así es como `cargo`
        // distingue un alias de un nombre de paquete real al resolver.
        let (name, package) = match &dep.explicit_name_in_toml {
            Some(local_alias) => (local_alias.clone(), Some(dep.name.clone())),
            None => (dep.name.clone(), None),
        };

        Self {
            name,
            req: dep.version_req.clone(),
            features: dep.features.clone(),
            optional: dep.optional,
            default_features: dep.default_features,
            target: dep.target.clone(),
            kind: dep.kind.clone(),
            registry: dep.registry.clone(),
            package,
        }
    }
}

/// Resultado de descomponer una petición `cargo publish`: metadatos,
/// contenido del `.crate` y el SHA-256 de ese contenido, ya listo para
/// persistirse tanto en el artefacto como en la entrada de índice.
struct ParsedPublishPayload {
    metadata: PublishMetadata,
    crate_bytes: Bytes,
    checksum: Sha256Checksum,
}

/// Descompone el cuerpo binario de una petición `cargo publish` en sus
/// dos partes: los metadatos JSON y el contenido del archivo `.crate`.
/// El checksum SHA-256 se calcula aquí, sobre los bytes del `.crate`,
/// para que la entrada de índice (`cksum`) y los metadatos del artefacto
/// compartan exactamente el mismo valor.
///
/// Formato (todos los enteros en *little-endian*):
/// `[u32 longitud de metadatos][metadatos JSON][u32 longitud del
/// `.crate`][contenido del `.crate`]`. Cualquier dato adicional a
/// continuación (extensiones de versiones recientes de `cargo`, como el
/// archivo `.crate` firmado) se ignora.
fn parse_publish_payload(mut payload: Bytes) -> Result<ParsedPublishPayload, PackagingError> {
    let metadata_len = read_u32_le(&mut payload, "metadata length prefix")?;
    let metadata_bytes = split_prefix(&mut payload, metadata_len, "metadata")?;
    let metadata: PublishMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|err| PackagingError::InvalidPayload(format!("invalid metadata JSON: {err}")))?;

    let crate_len = read_u32_le(&mut payload, "crate file length prefix")?;
    let crate_bytes = split_prefix(&mut payload, crate_len, "crate file")?;
    let checksum = sha256_checksum(&crate_bytes);

    Ok(ParsedPublishPayload {
        metadata,
        crate_bytes,
        checksum,
    })
}

fn read_u32_le(payload: &mut Bytes, what: &str) -> Result<usize, PackagingError> {
    if payload.remaining() < 4 {
        return Err(PackagingError::InvalidPayload(format!(
            "payload is too short to contain the {what}"
        )));
    }
    Ok(payload.get_u32_le() as usize)
}

fn split_prefix(payload: &mut Bytes, len: usize, what: &str) -> Result<Bytes, PackagingError> {
    if payload.remaining() < len {
        return Err(PackagingError::InvalidPayload(format!(
            "payload is shorter than the declared {what} length"
        )));
    }
    Ok(payload.split_to(len))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::repository::{RepositoryKind, RepositoryName};

    use crate::test_support::{InMemoryArtifactStore, InMemoryPackageIndexStore, InMemoryStorage};

    use super::*;

    fn cargo_repository(name: &str) -> Repository {
        Repository::new(
            RepositoryName::parse(name).unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Cargo,
        )
        .unwrap()
    }

    fn strategy() -> CargoPackagingStrategy {
        CargoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
        )
    }

    fn encode_publish_payload(metadata_json: &str, crate_bytes: &[u8]) -> Bytes {
        let metadata_bytes = metadata_json.as_bytes();
        let mut payload = BytesMut::new();
        payload.extend_from_slice(&u32::try_from(metadata_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(metadata_bytes);
        payload.extend_from_slice(&u32::try_from(crate_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(crate_bytes);
        payload.freeze()
    }

    fn minimal_metadata(name: &str, vers: &str) -> String {
        format!(r#"{{"name":"{name}","vers":"{vers}","deps":[],"features":{{}}}}"#)
    }

    #[tokio::test]
    async fn publishes_a_crate_with_no_dependencies() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let payload =
            encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"fake tarball");

        let coordinate = strategy.publish(&repository, payload).await.unwrap();

        assert_eq!(coordinate.name().as_str(), "ferrobox-cli");
        assert_eq!(coordinate.version().as_str(), "0.1.0");
    }

    #[tokio::test]
    async fn rejects_publishing_to_a_non_cargo_repository() {
        let strategy = strategy();
        let repository = Repository::new(
            RepositoryName::parse("generic-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Generic,
        )
        .unwrap();
        let payload = encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"data");

        let result = strategy.publish(&repository, payload).await;

        assert!(matches!(
            result,
            Err(PackagingError::EcosystemMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_publishing_the_same_version_twice() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let payload =
            || encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"tarball");

        strategy.publish(&repository, payload()).await.unwrap();
        let result = strategy.publish(&repository, payload()).await;

        assert!(matches!(result, Err(PackagingError::AlreadyPublished(_))));
    }

    #[tokio::test]
    async fn rejects_a_truncated_payload() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");

        let result = strategy
            .publish(&repository, Bytes::from_static(b"\x05\x00"))
            .await;

        assert!(matches!(result, Err(PackagingError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn index_lists_every_published_version_as_a_json_line_per_entry() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), b"v1"),
            )
            .await
            .unwrap();
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.2.0"), b"v2"),
            )
            .await
            .unwrap();

        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();

        let lines: Vec<&str> = std::str::from_utf8(&index).unwrap().lines().collect();
        assert_eq!(lines.len(), 2);
        let first: IndexEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first.vers, "0.1.0");
        assert!(!first.yanked);
        assert_eq!(first.cksum, sha256_checksum(b"v1").as_str());
    }

    #[tokio::test]
    async fn index_for_an_unpublished_package_is_not_found() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");

        let result = strategy
            .index(&repository, &PackageName::parse("nonexistent").unwrap())
            .await;

        assert!(matches!(result, Err(PackagingError::PackageNotFound(_))));
    }

    #[tokio::test]
    async fn publish_stores_sha256_of_the_crate_file_in_the_index_entry() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let crate_bytes = b"tarball-for-checksum";
        strategy
            .publish(
                &repository,
                encode_publish_payload(&minimal_metadata("ferrobox-cli", "0.1.0"), crate_bytes),
            )
            .await
            .unwrap();

        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();
        let entry: IndexEntry =
            serde_json::from_str(std::str::from_utf8(&index).unwrap().trim()).unwrap();

        assert_eq!(entry.cksum, sha256_checksum(crate_bytes).as_str());
        assert_eq!(entry.cksum.len(), 64);
    }

    #[tokio::test]
    async fn downloads_a_previously_published_crate() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let coordinate = strategy
            .publish(
                &repository,
                encode_publish_payload(
                    &minimal_metadata("ferrobox-cli", "0.1.0"),
                    b"tarball bytes",
                ),
            )
            .await
            .unwrap();

        let content = strategy.download(&repository, &coordinate).await.unwrap();

        assert_eq!(content, Bytes::from_static(b"tarball bytes"));
    }

    #[tokio::test]
    async fn downloading_an_unpublished_version_is_not_found() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("ferrobox-cli").unwrap(),
            PackageVersion::parse("9.9.9").unwrap(),
        );

        let result = strategy.download(&repository, &coordinate).await;

        assert!(matches!(result, Err(PackagingError::VersionNotFound(_))));
    }

    #[tokio::test]
    async fn maps_a_renamed_dependency_to_name_and_package() {
        let strategy = strategy();
        let repository = cargo_repository("crates-releases");
        let metadata = r#"{
            "name": "ferrobox-cli",
            "vers": "0.1.0",
            "deps": [{
                "name": "serde_json",
                "version_req": "^1",
                "features": [],
                "optional": false,
                "default_features": true,
                "target": null,
                "kind": "normal",
                "registry": null,
                "explicit_name_in_toml": "json"
            }],
            "features": {}
        }"#;

        strategy
            .publish(&repository, encode_publish_payload(metadata, b"tarball"))
            .await
            .unwrap();

        let index = strategy
            .index(&repository, &PackageName::parse("ferrobox-cli").unwrap())
            .await
            .unwrap();
        let entry: IndexEntry =
            serde_json::from_str(std::str::from_utf8(&index).unwrap().trim()).unwrap();

        assert_eq!(entry.deps[0].name, "json");
        assert_eq!(entry.deps[0].package.as_deref(), Some("serde_json"));
    }

    #[test]
    fn shard_path_for_a_one_character_name() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("a").unwrap()),
            "1/a"
        );
    }

    #[test]
    fn shard_path_for_a_two_character_name() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("ab").unwrap()),
            "2/ab"
        );
    }

    #[test]
    fn shard_path_for_a_three_character_name() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("abc").unwrap()),
            "3/a/abc"
        );
    }

    #[test]
    fn shard_path_for_a_longer_name_uses_the_first_four_characters() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("serde").unwrap()),
            "se/rd/serde"
        );
    }

    #[test]
    fn shard_path_normalizes_case() {
        assert_eq!(
            cargo_index_shard_path(&PackageName::parse("Ferrobox").unwrap()),
            "fe/rr/ferrobox"
        );
    }
}
