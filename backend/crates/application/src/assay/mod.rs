//! Ensaye (`Assay`) de una versión de paquete: inventario y vulnerabilidades.

mod cyclonedx;
mod extract;
mod layers;
mod licenses;
mod lockfiles;
mod osv;

use std::collections::HashSet;
use std::sync::Arc;

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use ferrobox_domain::assay::{Assay, AssayStatus};
use ferrobox_domain::ids::{AssayId, RepositoryId};
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::assay_store::{AssayStore, AssayStoreError};
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use ferrobox_ports::package_index_store::{PackageIndexStore, PackageIndexStoreError};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::webhooks::WebhookService;

use self::extract::osv_query_target;
use self::layers::extract_inventory;
use self::osv::query_findings;

pub use self::cyclonedx::to_cyclonedx;
pub use self::extract::{is_exact_version, purl_for};

/// Motivos por los que un ensaye puede fallar.
#[derive(Debug, Error)]
pub enum AssayError {
    /// El repositorio no existe.
    #[error("repository {0} does not exist")]
    RepositoryNotFound(RepositoryId),

    /// El ensaye no existe.
    #[error("assay {0} was not found")]
    AssayNotFound(AssayId),

    /// No hay ninguna versión publicada con esa coordenada.
    #[error("{0} was not found")]
    PackageNotFound(PackageCoordinate),

    /// El nombre o la versión no son válidos.
    #[error("invalid package coordinate: {0}")]
    InvalidCoordinate(String),

    /// Fallo al persistir el ensaye.
    #[error(transparent)]
    Persistence(#[from] AssayStoreError),

    /// Fallo al leer el índice de paquetes.
    #[error(transparent)]
    Index(#[from] PackageIndexStoreError),

    /// Fallo al leer repositorios.
    #[error(transparent)]
    Repositories(#[from] RepositoryStoreError),

    /// Fallo al leer blobs o manifiestos del almacenamiento.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Caso de uso: listar, obtener y ejecutar ensayes.
#[derive(Clone)]
pub struct AssayService {
    assays: Arc<dyn AssayStore>,
    package_index_store: Arc<dyn PackageIndexStore>,
    repository_store: Arc<dyn RepositoryStore>,
    storage: Arc<dyn StoragePort>,
    http_client: Arc<dyn HttpClient>,
    webhooks: Option<WebhookService>,
}

impl AssayService {
    /// Construye el servicio a partir de sus puertos.
    #[must_use]
    pub fn new(
        assays: Arc<dyn AssayStore>,
        package_index_store: Arc<dyn PackageIndexStore>,
        repository_store: Arc<dyn RepositoryStore>,
        storage: Arc<dyn StoragePort>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            assays,
            package_index_store,
            repository_store,
            storage,
            http_client,
            webhooks: None,
        }
    }

    /// Conecta el envío de avisos HTTP. Un fallo del destino no afecta
    /// al ensaye ni al publish.
    #[must_use]
    pub fn with_webhooks(mut self, webhooks: WebhookService) -> Self {
        self.webhooks = Some(webhooks);
        self
    }

    /// Lista todos los ensayes de la instancia.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayError::Persistence`] si falla el almacén.
    pub async fn list_all(&self) -> Result<Vec<Assay>, AssayError> {
        Ok(self.assays.find_all().await?)
    }

    /// Lista los ensayes de un repositorio. En un `Alloy`, une los de
    /// sus miembros.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayError::RepositoryNotFound`] si no existe.
    pub async fn list_for_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Assay>, AssayError> {
        let repository = self.require_repository(repository_id).await?;
        let mut assays = self.assays.find_by_repository(repository_id).await?;
        if let RepositoryKind::Alloy { members } = repository.kind() {
            for member_id in members {
                assays.extend(self.assays.find_by_repository(*member_id).await?);
            }
        }
        Ok(assays)
    }

    /// Devuelve el ensaye de una coordenada. Si no existe, lo ejecuta.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayError::PackageNotFound`] si el paquete no está
    /// indexado, o [`AssayError::RepositoryNotFound`].
    pub async fn get_or_run(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &str,
        version: &str,
    ) -> Result<Assay, AssayError> {
        let (source, coordinate, _entry) = self
            .resolve_indexed(repository_id, ecosystem, name, version)
            .await?;
        if let Some(existing) = self
            .assays
            .find_by_coordinate(source.id(), &coordinate)
            .await?
        {
            return Ok(existing);
        }
        self.run_on(source.id(), &coordinate).await
    }

    /// Vuelve a ensayar una coordenada, sustituyendo el resultado previo.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayError::PackageNotFound`] si el paquete no está
    /// indexado.
    pub async fn run(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &str,
        version: &str,
    ) -> Result<Assay, AssayError> {
        let (source, coordinate, _entry) = self
            .resolve_indexed(repository_id, ecosystem, name, version)
            .await?;
        self.run_on(source.id(), &coordinate).await
    }

    /// Busca un ensaye por identificador.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayError::PackageNotFound`] si no existe (se reutiliza
    /// el mismo código HTTP 404).
    pub async fn get_by_id(&self, id: AssayId) -> Result<Assay, AssayError> {
        self.assays
            .find_by_id(id)
            .await?
            .ok_or(AssayError::AssayNotFound(id))
    }

    /// Lanza un ensaye en segundo plano. No bloquea `publish` ni
    /// `install`: un fallo queda persistido como estado `Failed`.
    pub fn schedule(&self, repository_id: RepositoryId, coordinate: PackageCoordinate) {
        if !should_auto_assay(&coordinate) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            let _ = service
                .run(
                    repository_id,
                    coordinate.ecosystem(),
                    coordinate.name().as_str(),
                    coordinate.version().as_str(),
                )
                .await;
        });
    }

    /// Como [`schedule`], y además avisa `package.published` sin esperar
    /// al destino.
    pub fn schedule_after_publish(
        &self,
        repository_id: RepositoryId,
        coordinate: PackageCoordinate,
    ) {
        if let Some(webhooks) = &self.webhooks {
            webhooks.notify_package_published(repository_id, coordinate.clone());
        }
        self.schedule(repository_id, coordinate);
    }

    /// Vuelve a ensayar las coordenadas de una imagen OCI/Helm tras
    /// cachear una capa. Sirve para pasar de `unsupported` (manifiesto
    /// sin blobs) a un inventario real sin bloquear el `pull`.
    pub fn reschedule_for_name(&self, repository_id: RepositoryId, name: &str) {
        let name = name.to_string();
        let service = self.clone();
        tokio::spawn(async move {
            let Ok(assays) = service.assays.find_by_repository(repository_id).await else {
                return;
            };
            for assay in assays {
                if assay.coordinate().name().as_str() == name {
                    let _ = service.assays.upsert(&assay.mark_running()).await;
                    let _ = service.run_on(repository_id, assay.coordinate()).await;
                }
            }
        });
    }

    /// Reensaya en segundo plano todas las coordenadas ya ensayadas.
    ///
    /// # Errors
    ///
    /// Devuelve [`AssayError::Persistence`] si falla el almacén.
    pub async fn rerun_all(&self) -> Result<u32, AssayError> {
        let assays = self.assays.find_all().await?;
        let mut seen = HashSet::new();
        let mut scheduled = 0_u32;
        for assay in assays {
            let key = (assay.repository_id(), assay.coordinate().clone());
            if !seen.insert(key) {
                continue;
            }
            self.assays.upsert(&assay.mark_running()).await?;
            self.schedule(assay.repository_id(), assay.coordinate().clone());
            scheduled += 1;
        }
        Ok(scheduled)
    }

    async fn run_on(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Assay, AssayError> {
        let entry = self
            .load_entry(repository_id, coordinate)
            .await?
            .ok_or_else(|| AssayError::PackageNotFound(coordinate.clone()))?;

        let existing_id = self
            .assays
            .find_by_coordinate(repository_id, coordinate)
            .await?
            .map_or_else(AssayId::new, |assay| assay.id());

        let components = extract_inventory(
            self.storage.as_ref(),
            self.package_index_store.as_ref(),
            repository_id,
            coordinate,
            &entry,
        )
        .await;
        let scanned_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let can_query = components
            .iter()
            .any(|component| osv_query_target(component, coordinate.ecosystem()).is_some());

        let assay = if can_query {
            match query_findings(
                self.http_client.as_ref(),
                coordinate.ecosystem(),
                &components,
            )
            .await
            {
                Ok(findings) => Assay::from_parts(
                    existing_id,
                    repository_id,
                    coordinate.clone(),
                    AssayStatus::Ready,
                    Some(scanned_at.clone()),
                    None,
                    components,
                    findings,
                ),
                Err(HttpClientError::Status { status, url }) => Assay::from_parts(
                    existing_id,
                    repository_id,
                    coordinate.clone(),
                    AssayStatus::Failed,
                    Some(scanned_at),
                    Some(format!(
                        "OSV (Open Source Vulnerabilities) respondió HTTP {status} para {url}"
                    )),
                    components,
                    Vec::new(),
                ),
                Err(HttpClientError::Transport { url, message }) => Assay::from_parts(
                    existing_id,
                    repository_id,
                    coordinate.clone(),
                    AssayStatus::Failed,
                    Some(scanned_at),
                    Some(format!("no se pudo consultar OSV en {url}: {message}")),
                    components,
                    Vec::new(),
                ),
            }
        } else if components.len() > 1 {
            Assay::from_parts(
                existing_id,
                repository_id,
                coordinate.clone(),
                AssayStatus::Ready,
                Some(scanned_at),
                composition_only_message(coordinate.ecosystem()),
                components,
                Vec::new(),
            )
        } else {
            Assay::from_parts(
                existing_id,
                repository_id,
                coordinate.clone(),
                AssayStatus::Unsupported,
                Some(scanned_at),
                Some(unsupported_message(coordinate.ecosystem())),
                components,
                Vec::new(),
            )
        };

        self.assays.upsert(&assay).await?;
        if let Some(webhooks) = &self.webhooks {
            webhooks.notify_assay_completed(assay.clone());
        }
        Ok(assay)
    }

    async fn require_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Repository, AssayError> {
        self.repository_store
            .find_by_id(repository_id)
            .await?
            .ok_or(AssayError::RepositoryNotFound(repository_id))
    }

    async fn resolve_indexed(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &str,
        version: &str,
    ) -> Result<(Repository, PackageCoordinate, Bytes), AssayError> {
        let name = PackageName::parse(name.to_string())
            .map_err(|err| AssayError::InvalidCoordinate(err.to_string()))?;
        let version = PackageVersion::parse(version.to_string())
            .map_err(|err| AssayError::InvalidCoordinate(err.to_string()))?;
        let coordinate = PackageCoordinate::new(ecosystem, name, version);
        let repository = self.require_repository(repository_id).await?;

        let mut targets = vec![repository.clone()];
        if let RepositoryKind::Alloy { members } = repository.kind() {
            for member_id in members {
                if let Some(member) = self.repository_store.find_by_id(*member_id).await? {
                    targets.push(member);
                }
            }
        }

        for target in targets {
            if let Some(entry) = self.load_entry(target.id(), &coordinate).await? {
                return Ok((target, coordinate, entry));
            }
        }
        Err(AssayError::PackageNotFound(coordinate))
    }

    async fn load_entry(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<Bytes>, AssayError> {
        let entries = self
            .package_index_store
            .entries_for_package(repository_id, coordinate.ecosystem(), coordinate.name())
            .await?;
        for entry in entries {
            if entry_version_matches(&entry, coordinate.version().as_str()) {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }
}

/// Omite blobs OCI y manifiestos indexados por digest: no son una
/// versión que la UI ensaye.
pub(crate) fn should_auto_assay(coordinate: &PackageCoordinate) -> bool {
    let name = coordinate.name().as_str();
    let version = coordinate.version().as_str();
    name != "_blob" && !name.is_empty() && !version.is_empty() && !version.starts_with("sha256:")
}

fn composition_only_message(ecosystem: PackageEcosystem) -> Option<String> {
    match ecosystem {
        PackageEcosystem::Helm => Some(
            "OSV (Open Source Vulnerabilities) no indexa charts Helm. Se muestra la composición de Chart.yaml.".to_string(),
        ),
        PackageEcosystem::Conan => Some(
            "OSV (Open Source Vulnerabilities) no indexa Conan. Se muestra la composición de la receta.".to_string(),
        ),
        _ => None,
    }
}

fn unsupported_message(ecosystem: PackageEcosystem) -> String {
    match ecosystem {
        PackageEcosystem::Oci | PackageEcosystem::Helm => {
            "No hay capas locales para inventariar (apk/dpkg o Chart.yaml). Haz pull o push de la imagen/chart para cachear los blobs y vuelve a ensayar.".to_string()
        }
        PackageEcosystem::Conan => {
            "No se encontró conanfile.py ni conanfile.txt con requires en esta receta.".to_string()
        }
        _ => {
            "Assay cubre npm, PyPI, Cargo y el sistema de ficheros de imágenes OCI (Alpine/Debian/Ubuntu).".to_string()
        }
    }
}

fn entry_version_matches(entry: &Bytes, version: &str) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(entry) else {
        return false;
    };
    if ["version", "vers", "reference"]
        .iter()
        .any(|key| value.get(*key).and_then(serde_json::Value::as_str) == Some(version))
    {
        return true;
    }
    // Conan indexa `version`/`user`/`channel` por separado; la UI manda
    // la coordenada `0.1@_:_`.
    match (
        value.get("version").and_then(serde_json::Value::as_str),
        value.get("user").and_then(serde_json::Value::as_str),
        value.get("channel").and_then(serde_json::Value::as_str),
    ) {
        (Some(recipe_version), Some(user), Some(channel)) => {
            format!("{recipe_version}@{user}:{channel}") == version
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_key::storage_key_for;
    use crate::test_support::{
        InMemoryAssayStore, InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryRepositoryStore,
        InMemoryStorage,
    };
    use ferrobox_domain::ids::ArtifactId;
    use ferrobox_domain::package_coordinate::PackageName;
    use ferrobox_domain::repository::{RepositoryKind, RepositoryName};
    use ferrobox_ports::storage::StoragePort;
    use sha2::{Digest, Sha256};

    fn npm_forge() -> Repository {
        Repository::new(
            RepositoryName::parse("npm-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Npm,
        )
        .unwrap()
    }

    async fn seed_lodash(index: &InMemoryPackageIndexStore, repository: &Repository) {
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("lodash").unwrap(),
            PackageVersion::parse("4.17.20").unwrap(),
        );
        let entry = Bytes::from(
            serde_json::json!({
                "name": "lodash",
                "version": "4.17.20",
                "manifest": { "license": "MIT", "dependencies": { "foo": "^1.0.0" } }
            })
            .to_string(),
        );
        index
            .upsert_entry(repository.id(), &coordinate, None, entry)
            .await
            .unwrap();
    }

    fn osv_high_lodash_vuln() -> serde_json::Value {
        serde_json::json!({
            "id": "GHSA-35jh-r3h4-6jhm",
            "aliases": ["CVE-2021-23337"],
            "summary": "Command Injection in lodash",
            "database_specific": { "severity": "HIGH" },
            "affected": [{ "ranges": [{ "events": [{ "fixed": "4.17.21" }] }] }],
            "references": [{ "url": "https://github.com/advisories/GHSA-35jh-r3h4-6jhm" }]
        })
    }

    fn osv_high_lodash() -> Bytes {
        Bytes::from(
            serde_json::json!({
                "results": [{
                    "vulns": [osv_high_lodash_vuln()]
                }]
            })
            .to_string(),
        )
    }

    #[test]
    fn entry_version_matches_oci_reference_and_conan_recipe() {
        assert!(entry_version_matches(
            &Bytes::from(r#"{"reference":"latest"}"#),
            "latest",
        ));
        assert!(entry_version_matches(
            &Bytes::from(r#"{"version":"0.1","user":"_","channel":"_"}"#),
            "0.1@_:_",
        ));
        assert!(entry_version_matches(
            &Bytes::from(r#"{"vers":"1.0.0"}"#),
            "1.0.0",
        ));
        assert!(!entry_version_matches(
            &Bytes::from(r#"{"reference":"latest"}"#),
            "1.0.0",
        ));
    }

    #[tokio::test]
    async fn run_records_inventory_and_osv_findings() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            osv_high_lodash(),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();

        assert_eq!(assay.status(), AssayStatus::Ready);
        assert!(assay.components().iter().any(|c| c.name() == "lodash"));
        assert!(assay.components().iter().any(|c| c.name() == "foo"));
        assert_eq!(
            assay
                .components()
                .iter()
                .find(|component| component.name() == "lodash")
                .map(ferrobox_domain::assay::AssayComponent::licenses),
            Some(["MIT".to_string()].as_slice())
        );
        assert_eq!(assay.counts().high, 1);
        assert_eq!(assay.findings()[0].fixed_version(), Some("4.17.21"));
        let document: serde_json::Value = serde_json::from_slice(&to_cyclonedx(&assay)).unwrap();
        assert_eq!(document["bomFormat"], "CycloneDX");
        assert_eq!(document["vulnerabilities"][0]["id"], "GHSA-35jh-r3h4-6jhm");
    }

    #[tokio::test]
    async fn get_or_run_reuses_existing_assay() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            osv_high_lodash(),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        let first = service
            .get_or_run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        let second = service
            .get_or_run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(first.id(), second.id());
    }

    #[tokio::test]
    async fn thin_osv_batch_is_hydrated_from_vuln_endpoint() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            Bytes::from(
                serde_json::json!({
                    "results": [{ "vulns": [{ "id": "GHSA-35jh-r3h4-6jhm" }] }]
                })
                .to_string(),
            ),
        );
        http.stub(
            "https://api.osv.dev/v1/vulns/GHSA-35jh-r3h4-6jhm",
            200,
            Bytes::from(osv_high_lodash_vuln().to_string()),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        assert_eq!(assay.counts().high, 1);
        assert_eq!(assay.findings()[0].title(), "Command Injection in lodash");
        assert_eq!(assay.findings()[0].fixed_version(), Some("4.17.21"));
    }

    #[tokio::test]
    async fn oci_tag_entry_is_unsupported_without_layer_inventory() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse("latest").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(
                    r#"{"name":"alpine","reference":"latest","digest":"sha256:abc","media_type":"application/vnd.oci.image.manifest.v1+json","size":527,"artifact_id":"00000000-0000-0000-0000-000000000001"}"#,
                ),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Oci, "alpine", "latest")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Unsupported);
        assert!(assay.findings().is_empty());
        assert!(assay.error_message().is_some());
    }

    #[tokio::test]
    async fn conan_recipe_is_unsupported_without_lock_inventory() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("conan-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Conan,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Conan,
            PackageName::parse("hello").unwrap(),
            PackageVersion::parse("0.1@_:_").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(
                    r#"{"name":"hello","version":"0.1","user":"_","channel":"_","yanked":false,"files":[],"revisions":[]}"#,
                ),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(
                repository.id(),
                PackageEcosystem::Conan,
                "hello",
                "0.1@_:_",
            )
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Unsupported);
    }

    #[tokio::test]
    async fn oci_is_unsupported_without_layer_inventory() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let repository = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("library/alpine").unwrap(),
            PackageVersion::parse("3.19").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(r#"{"name":"library/alpine","version":"3.19"}"#),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(
                repository.id(),
                PackageEcosystem::Oci,
                "library/alpine",
                "3.19",
            )
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Unsupported);
        assert!(assay.findings().is_empty());
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

    fn content_digest(body: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(body))
    }

    #[tokio::test]
    async fn oci_alpine_layer_queries_osv_for_apk_packages() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = Repository::new(
            RepositoryName::parse("images").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Oci,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();

        let layer = gzip_tar_file(
            "lib/apk/db/installed",
            "P:busybox\nV:1.36.1-r19\n\n",
        );
        let layer_digest = content_digest(&layer);
        let layer_id = ArtifactId::new();
        storage
            .put(&storage_key_for(layer_id), layer.clone())
            .await
            .unwrap();
        let blob_coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("_blob").unwrap(),
            PackageVersion::parse(layer_digest.clone()).unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &blob_coordinate,
                Some(layer_id),
                Bytes::from(format!(
                    r#"{{"digest":"{layer_digest}","artifact_id":"{layer_id}","size":{}}}"#,
                    layer.len()
                )),
            )
            .await
            .unwrap();

        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "layers": [{
                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                "digest": layer_digest,
                "size": layer.len()
            }]
        });
        let manifest_bytes = Bytes::from(manifest.to_string());
        let manifest_id = ArtifactId::new();
        storage
            .put(&storage_key_for(manifest_id), manifest_bytes)
            .await
            .unwrap();
        let tag_coordinate = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse("latest").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &tag_coordinate,
                Some(manifest_id),
                Bytes::from(format!(
                    r#"{{"name":"alpine","reference":"latest","digest":"sha256:dead","media_type":"application/vnd.oci.image.manifest.v1+json","size":1,"artifact_id":"{manifest_id}"}}"#
                )),
            )
            .await
            .unwrap();

        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            Bytes::from(
                serde_json::json!({
                    "results": [{
                        "vulns": [{
                            "id": "CVE-2022-28391",
                            "summary": "busybox issue",
                            "database_specific": { "severity": "MEDIUM" },
                            "affected": [{ "ranges": [{ "events": [{ "fixed": "1.36.2-r0" }] }] }]
                        }]
                    }]
                })
                .to_string(),
            ),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            storage,
            http,
        );
        let assay = service
            .run(repository.id(), PackageEcosystem::Oci, "alpine", "latest")
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Ready);
        assert!(assay.components().iter().any(|c| c.name() == "busybox"));
        assert_eq!(assay.counts().medium, 1);
        assert_eq!(assay.findings()[0].component_name(), "busybox");
    }

    #[tokio::test]
    async fn conan_recipe_lists_requires_without_osv() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let repository = Repository::new(
            RepositoryName::parse("conan-releases").unwrap(),
            RepositoryKind::Forge,
            PackageEcosystem::Conan,
        )
        .unwrap();
        repos.save(&repository).await.unwrap();
        let file_id = ArtifactId::new();
        storage
            .put(
                &storage_key_for(file_id),
                Bytes::from("[requires]\nzlib/1.2.13\n"),
            )
            .await
            .unwrap();
        let coordinate = PackageCoordinate::new(
            PackageEcosystem::Conan,
            PackageName::parse("hello").unwrap(),
            PackageVersion::parse("0.1@_:_").unwrap(),
        );
        index
            .upsert_entry(
                repository.id(),
                &coordinate,
                None,
                Bytes::from(format!(
                    r#"{{"name":"hello","version":"0.1","user":"_","channel":"_","yanked":false,"files":[{{"filename":"conanfile.txt","artifact_id":"{file_id}"}}],"revisions":[]}}"#
                )),
            )
            .await
            .unwrap();

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            storage,
            Arc::new(InMemoryHttpClient::default()),
        );
        let assay = service
            .run(
                repository.id(),
                PackageEcosystem::Conan,
                "hello",
                "0.1@_:_",
            )
            .await
            .unwrap();
        assert_eq!(assay.status(), AssayStatus::Ready);
        assert!(assay.components().iter().any(|c| c.name() == "zlib"));
        assert!(assay.findings().is_empty());
        assert!(assay.error_message().unwrap().contains("Conan"));
    }

    #[tokio::test]
    async fn missing_package_is_not_found() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            repos,
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let err = service
            .run(repository.id(), PackageEcosystem::Npm, "missing", "1.0.0")
            .await
            .unwrap_err();
        assert!(matches!(err, AssayError::PackageNotFound(_)));
    }

    #[test]
    fn auto_assay_skips_oci_blobs_and_digest_references() {
        let blob = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("_blob").unwrap(),
            PackageVersion::parse(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
        );
        assert!(!should_auto_assay(&blob));
        let digest_tag = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
        );
        assert!(!should_auto_assay(&digest_tag));
        let tag = PackageCoordinate::new(
            PackageEcosystem::Oci,
            PackageName::parse("alpine").unwrap(),
            PackageVersion::parse("latest").unwrap(),
        );
        assert!(should_auto_assay(&tag));
        let npm = PackageCoordinate::new(
            PackageEcosystem::Npm,
            PackageName::parse("lodash").unwrap(),
            PackageVersion::parse("4.17.20").unwrap(),
        );
        assert!(should_auto_assay(&npm));
    }

    async fn wait_for_assays(service: &AssayService, expected: usize) -> Vec<Assay> {
        for _ in 0..50 {
            let assays = service.list_all().await.unwrap();
            if assays.len() >= expected {
                return assays;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        service.list_all().await.unwrap()
    }

    async fn wait_until_not_running(service: &AssayService) -> Vec<Assay> {
        for _ in 0..50 {
            let assays = service.list_all().await.unwrap();
            if assays
                .iter()
                .all(|assay| assay.status() != AssayStatus::Running)
            {
                return assays;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        service.list_all().await.unwrap()
    }

    #[tokio::test]
    async fn schedule_runs_in_the_background_without_failing_the_caller() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            osv_high_lodash(),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        service.schedule(
            repository.id(),
            PackageCoordinate::new(
                PackageEcosystem::Npm,
                PackageName::parse("lodash").unwrap(),
                PackageVersion::parse("4.17.20").unwrap(),
            ),
        );
        let assays = wait_for_assays(&service, 1).await;
        assert_eq!(assays.len(), 1);
        assert_eq!(assays[0].counts().high, 1);
    }

    #[tokio::test]
    async fn rerun_all_schedules_each_distinct_coordinate() {
        let repos = Arc::new(InMemoryRepositoryStore::default());
        let index = Arc::new(InMemoryPackageIndexStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let repository = npm_forge();
        repos.save(&repository).await.unwrap();
        seed_lodash(&index, &repository).await;
        http.stub(
            "https://api.osv.dev/v1/querybatch",
            200,
            osv_high_lodash(),
        );

        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            index,
            repos,
            Arc::new(InMemoryStorage::default()),
            http,
        );
        service
            .run(repository.id(), PackageEcosystem::Npm, "lodash", "4.17.20")
            .await
            .unwrap();
        let scheduled = service.rerun_all().await.unwrap();
        assert_eq!(scheduled, 1);
        let assays = wait_until_not_running(&service).await;
        assert_eq!(assays.len(), 1);
        assert_ne!(assays[0].status(), AssayStatus::Running);
    }
}
