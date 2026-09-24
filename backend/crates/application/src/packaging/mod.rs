//! El patrón Strategy que permite a `FerroBox` soportar varios
//! ecosistemas de paquetes (Cargo, npm, `PyPI`...) sin que el resto de la
//! capa de aplicación necesite conocer los detalles de ninguno de ellos.
//!
//! Cada ecosistema tiene reglas propias e incompatibles entre sí para
//! tres operaciones: cómo se interpreta la petición de publicación
//! (`cargo publish` no envía el mismo formato que `npm publish`), cómo
//! se construye la respuesta del protocolo de índice que el
//! gestor de paquetes nativo consulta para resolver dependencias, y cómo
//! se localiza el artefacto binario correspondiente para su descarga.
//! [`PackagingStrategy`] captura esas tres operaciones como un contrato
//! único; [`PackagingRegistry`] selecciona, en tiempo de ejecución, qué
//! implementación concreta usar según el [`PackageEcosystem`] del
//! repositorio sobre el que se está operando.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use ferrobox_domain::repository::Repository;
use ferrobox_ports::artifact_store::{ArtifactStore, ArtifactStoreError};
use ferrobox_ports::http_client::HttpClientError;
use ferrobox_ports::package_index_store::PackageIndexStoreError;
use ferrobox_ports::repository_store::RepositoryStoreError;
use ferrobox_ports::storage::{StorageError, StoragePort};
use thiserror::Error;

use crate::assay::AssayService;

/// La implementación de Cargo (protocolo de índice disperso) del
/// patrón Strategy.
pub mod cargo;

/// La implementación de npm (registro compatible con `npm publish` /
/// `npm install`) del patrón Strategy.
pub mod npm;

/// La implementación de `PyPI` (`twine upload` / `pip install`, índice
/// simple PEP 503) del patrón Strategy.
pub mod pypi;

/// Detección de firmas Cosign / Sigstore y otros accesorios OCI.
pub mod cosign;

/// La implementación de OCI (Distribution Spec v2: `docker push` /
/// `docker pull`) del patrón Strategy.
pub mod oci;

/// La implementación de Conan (API v2 con revisiones: `conan upload` /
/// `conan install`) del patrón Strategy.
pub mod conan;

/// Motivos por los que una operación de empaquetado puede fallar.
#[derive(Debug, Error)]
pub enum PackagingError {
    /// Se invocó una estrategia con un repositorio de un ecosistema
    /// distinto al que la estrategia implementa.
    #[error(
        "repository is configured for ecosystem '{actual}', but this strategy handles '{expected}'"
    )]
    EcosystemMismatch {
        /// Ecosistema que la estrategia sabe manejar.
        expected: &'static str,
        /// Ecosistema real del repositorio recibido.
        actual: &'static str,
    },

    /// La petición de publicación no tiene un formato válido para este
    /// ecosistema.
    #[error("invalid publish payload: {0}")]
    InvalidPayload(String),

    /// Ya existe una versión publicada con esa misma coordenada -- los
    /// registros de paquetes son inmutables una vez publicados (yank
    /// aparte, que no sobrescribe el contenido, solo lo marca).
    #[error("{0} was already published and cannot be overwritten")]
    AlreadyPublished(PackageCoordinate),

    /// No existe ninguna versión publicada de este paquete en el
    /// repositorio.
    #[error("no published versions of package '{0}' were found in this repository")]
    PackageNotFound(String),

    /// La coordenada solicitada no corresponde a ninguna versión
    /// publicada.
    #[error("{0} was not found")]
    VersionNotFound(PackageCoordinate),

    /// No existe un fichero con ese nombre en el índice del repositorio
    /// (p. ej. un wheel o sdist concreto de `PyPI`).
    #[error("file '{0}' was not found in this repository")]
    FileNotFound(String),

    /// Fallo al subir o descargar el contenido binario del paquete.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// Fallo al persistir los metadatos del artefacto binario.
    #[error(transparent)]
    ArtifactPersistence(#[from] ArtifactStoreError),

    /// Fallo al leer o escribir el índice de paquetes.
    #[error(transparent)]
    IndexPersistence(#[from] PackageIndexStoreError),

    /// El contenido binario almacenado no coincide con el checksum SHA-256
    /// persistido al publicar.
    #[error("stored artifact checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Checksum persistido al publicar.
        expected: String,
        /// Checksum recalculado sobre el objeto almacenado.
        actual: String,
    },

    /// El repositorio es de solo lectura (un `Mirror` o un `Alloy`) y
    /// no acepta publicaciones.
    #[error("repository is read-only and does not accept publishes")]
    ReadOnlyRepository,

    /// Fallo al consultar el almacén de repositorios (p. ej. al
    /// resolver los miembros de un `Alloy`).
    #[error(transparent)]
    RepositoryPersistence(#[from] RepositoryStoreError),

    /// Fallo al consultar el *upstream* de un repositorio `Mirror`.
    #[error(transparent)]
    Upstream(#[from] HttpClientError),

    /// La respuesta del *upstream* no tiene el formato esperado.
    #[error("invalid upstream response: {0}")]
    InvalidUpstream(String),

    /// El binario no cabe en la cuota de almacenamiento del repositorio.
    #[error(transparent)]
    Quota(#[from] crate::quota::QuotaError),
}

/// El resultado de publicar un paquete: su coordenada recién asignada.
pub type PublishOutcome = PackageCoordinate;

/// Recuento de una promoción Forge→Forge: la coordenada copiada y el
/// volumen de binarios nuevos en el destino.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteOutcome {
    /// Coordenada publicada en el repositorio destino.
    pub coordinate: PackageCoordinate,
    /// Binarios nuevos creados en el destino (sin contar blobs OCI ya presentes).
    pub artifacts_copied: u32,
    /// Bytes escritos en el almacenamiento del destino.
    pub bytes_copied: u64,
}

/// Una coincidencia de `cargo search` contra el índice de un repositorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSearchHit {
    /// Nombre del paquete.
    pub name: String,
    /// Última versión no yankada (o la última publicada, si todas lo están).
    pub max_version: String,
}

/// Un manifiesto OCI leído del índice: media type, digest y cuerpo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciManifestDocument {
    /// Media type OCI o Docker del manifiesto.
    pub media_type: String,
    /// Digest `sha256:…` del cuerpo.
    pub digest: String,
    /// Cuerpo crudo del manifiesto.
    pub body: Bytes,
}

/// Estrategia de empaquetado para un ecosistema concreto.
///
/// Cada implementación encapsula las tres operaciones que el gestor de
/// paquetes nativo de su ecosistema espera de un registro: publicar un
/// paquete nuevo, responder al protocolo de índice con el que se
/// resuelven las dependencias, y servir el contenido binario de una
/// versión ya publicada.
#[async_trait]
pub trait PackagingStrategy: Send + Sync {
    /// El ecosistema que esta estrategia sabe manejar.
    fn ecosystem(&self) -> PackageEcosystem;

    /// Publica un paquete nuevo en `repository` a partir de la petición
    /// cruda enviada por el cliente nativo del ecosistema (por ejemplo,
    /// el cuerpo de una petición `cargo publish`).
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::EcosystemMismatch`] si `repository` no
    /// pertenece a este ecosistema, [`PackagingError::InvalidPayload`]
    /// si `payload` no tiene el formato esperado,
    /// [`PackagingError::AlreadyPublished`] si la coordenada resultante
    /// ya existía, o cualquier otro error si falla el puerto
    /// correspondiente.
    async fn publish(
        &self,
        repository: &Repository,
        payload: Bytes,
    ) -> Result<PublishOutcome, PackagingError>;

    /// Construye la respuesta del protocolo de índice de este ecosistema
    /// para el paquete `name` dentro de `repository`.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::EcosystemMismatch`] si `repository` no
    /// pertenece a este ecosistema, o cualquier otro error si falla el
    /// puerto correspondiente.
    async fn index(
        &self,
        repository: &Repository,
        name: &PackageName,
    ) -> Result<Bytes, PackagingError>;

    /// Descarga el contenido binario de una versión ya publicada.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::EcosystemMismatch`] si `repository` no
    /// pertenece a este ecosistema, [`PackagingError::PackageNotFound`]
    /// si `coordinate` no corresponde a ninguna versión publicada, o
    /// cualquier otro error si falla el puerto correspondiente.
    async fn download(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
    ) -> Result<Bytes, PackagingError>;

    /// Descarga un fichero del repositorio por su nombre de archivo
    /// (p. ej. un wheel o sdist de `PyPI`). La implementación por defecto
    /// indica que el ecosistema no resuelve artefactos por nombre.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::FileNotFound`] si el ecosistema no
    /// soporta esta operación o el fichero no existe.
    async fn download_file(
        &self,
        _repository: &Repository,
        filename: &str,
    ) -> Result<Bytes, PackagingError> {
        Err(PackagingError::FileNotFound(filename.to_string()))
    }

    /// Sube un fichero identificado por la ruta relativa del protocolo
    /// nativo (p. ej. receta o paquete Conan).
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::InvalidPayload`] si el ecosistema no
    /// acepta este tipo de subida.
    async fn put_protocol_file(
        &self,
        _repository: &Repository,
        _path: &str,
        _body: Bytes,
    ) -> Result<(), PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not accept protocol file uploads".to_string(),
        ))
    }

    /// Descarga un fichero identificado por la ruta relativa del
    /// protocolo nativo.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::FileNotFound`] si no existe.
    async fn get_protocol_file(
        &self,
        _repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Err(PackagingError::FileNotFound(path.to_string()))
    }

    /// Metadatos JSON del protocolo nativo para `path` (latest,
    /// revisiones, listado de ficheros, búsqueda de binarios).
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::PackageNotFound`] si no hay datos.
    async fn protocol_metadata(
        &self,
        _repository: &Repository,
        path: &str,
    ) -> Result<Bytes, PackagingError> {
        Err(PackagingError::PackageNotFound(path.to_string()))
    }

    /// Almacena un blob OCI identificado por su digest `sha256:…`.
    /// La implementación por defecto indica que el ecosistema no usa blobs.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::InvalidPayload`] si el ecosistema no
    /// soporta blobs, u otro error si falla el puerto correspondiente.
    async fn put_blob(
        &self,
        _repository: &Repository,
        _digest: &str,
        _body: Bytes,
    ) -> Result<u64, PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not store OCI blobs".to_string(),
        ))
    }

    /// Publica un manifiesto OCI (por etiqueta o por digest).
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::InvalidPayload`] si el ecosistema no
    /// soporta manifiestos, u otro error si falla el puerto correspondiente.
    async fn put_manifest(
        &self,
        _repository: &Repository,
        _name: &str,
        _reference: &str,
        _media_type: &str,
        _body: Bytes,
    ) -> Result<String, PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not store OCI manifests".to_string(),
        ))
    }

    /// Lee un manifiesto OCI por etiqueta o digest.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::PackageNotFound`] o
    /// [`PackagingError::VersionNotFound`] si no existe.
    async fn get_manifest(
        &self,
        _repository: &Repository,
        _name: &str,
        _reference: &str,
    ) -> Result<OciManifestDocument, PackagingError> {
        Err(PackagingError::PackageNotFound(_name.to_string()))
    }

    /// Descarga un blob OCI por digest. `name` es el nombre de imagen
    /// (un `Mirror` lo necesita para construir la URL *upstream*).
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::FileNotFound`] si el blob no existe.
    async fn get_blob(
        &self,
        repository: &Repository,
        _name: &str,
        digest: &str,
    ) -> Result<Bytes, PackagingError> {
        self.download_file(repository, digest).await
    }

    /// Lista las etiquetas de una imagen OCI.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::PackageNotFound`] si la imagen no existe.
    async fn list_tags(
        &self,
        _repository: &Repository,
        _name: &str,
    ) -> Result<Vec<String>, PackagingError> {
        Err(PackagingError::PackageNotFound(_name.to_string()))
    }

    /// Lista los manifiestos que apuntan a `digest` como `subject`
    /// (Referrers API del Distribution Spec).
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::PackageNotFound`] si el ecosistema no
    /// implementa referrers o la imagen no existe.
    async fn list_referrers(
        &self,
        _repository: &Repository,
        _name: &str,
        _digest: &str,
        _artifact_type: Option<&str>,
    ) -> Result<OciManifestDocument, PackagingError> {
        Err(PackagingError::PackageNotFound(_name.to_string()))
    }

    /// Marca (o desmarca) una versión ya publicada como *yanked*. No
    /// borra el binario: `cargo` sigue pudiendo descargarlo si está
    /// fijado en un `Cargo.lock`, pero deja de considerarlo para
    /// resoluciones nuevas.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::EcosystemMismatch`] si `repository` no
    /// pertenece a este ecosistema, [`PackagingError::ReadOnlyRepository`]
    /// si es un `Mirror` o un `Alloy`, [`PackagingError::VersionNotFound`]
    /// si esa coordenada no existe, o cualquier otro error si falla el
    /// puerto correspondiente.
    async fn set_yanked(
        &self,
        repository: &Repository,
        coordinate: &PackageCoordinate,
        yanked: bool,
    ) -> Result<(), PackagingError>;

    /// Copia una versión ya publicada de `source` a `target` (ambos
    /// `Forge` del mismo ecosistema). Los binarios se reescriben con
    /// identificadores nuevos; por defecto la copia no hereda el yank.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::InvalidPayload`] si el ecosistema no
    /// soporta esta operación, [`PackagingError::VersionNotFound`] si
    /// la coordenada no existe en el origen,
    /// [`PackagingError::AlreadyPublished`] si ya está en el destino, u
    /// otro error si falla un puerto.
    async fn promote_version(
        &self,
        _source: &Repository,
        _target: &Repository,
        _coordinate: &PackageCoordinate,
        _preserve_yanked: bool,
    ) -> Result<PromoteOutcome, PackagingError> {
        Err(PackagingError::InvalidPayload(
            "this ecosystem does not support promoting versions".to_string(),
        ))
    }

    /// Busca paquetes cuyo nombre contiene `query` (sin distinguir
    /// mayúsculas), hasta `limit` coincidencias.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::EcosystemMismatch`] si `repository` no
    /// pertenece a este ecosistema, o cualquier otro error si falla el
    /// puerto correspondiente.
    async fn search(
        &self,
        repository: &Repository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<PackageSearchHit>, PackagingError>;
}

/// Dispara un ensaye en segundo plano si la estrategia tiene
/// [`AssayService`]. No bloquea publish ni install.
pub(crate) fn notify_assay(
    assays: Option<&AssayService>,
    repository_id: RepositoryId,
    coordinate: &PackageCoordinate,
) {
    if let Some(assays) = assays {
        assays.schedule_after_publish(repository_id, coordinate.clone());
    }
}

pub(crate) async fn ensure_quota(
    quota: Option<&crate::quota::QuotaService>,
    repository_id: RepositoryId,
    additional_bytes: u64,
) -> Result<(), PackagingError> {
    if let Some(quota) = quota {
        quota
            .ensure_can_store(repository_id, additional_bytes)
            .await?;
    }
    Ok(())
}

/// Copia el objeto almacenado de `source_artifact_id` a un artefacto
/// nuevo en `target_repository_id`. Aplica la cuota del destino.
pub(crate) async fn copy_stored_artifact(
    artifact_store: &dyn ArtifactStore,
    storage: &dyn StoragePort,
    quota: Option<&crate::quota::QuotaService>,
    source_artifact_id: ArtifactId,
    target_repository_id: RepositoryId,
) -> Result<(ArtifactId, u64), PackagingError> {
    let source = artifact_store
        .find_by_id(source_artifact_id)
        .await?
        .ok_or_else(|| PackagingError::FileNotFound(source_artifact_id.to_string()))?;
    let storage_key = crate::storage_key::storage_key_for(source_artifact_id);
    let content = storage.get(&storage_key).await?;
    let actual = crate::content_hash::sha256_checksum(&content);
    if actual != *source.checksum() {
        return Err(PackagingError::ChecksumMismatch {
            expected: source.checksum().to_string(),
            actual: actual.to_string(),
        });
    }

    let size_bytes = source.size_bytes();
    ensure_quota(quota, target_repository_id, size_bytes).await?;
    let copied = Artifact::new(target_repository_id, source.checksum().clone(), size_bytes);
    storage
        .put(&crate::storage_key::storage_key_for(copied.id()), content)
        .await?;
    artifact_store.save(&copied).await?;
    Ok((copied.id(), size_bytes))
}

/// Selecciona, en tiempo de ejecución, la [`PackagingStrategy`] adecuada
/// para el [`PackageEcosystem`] de un repositorio.
///
/// Es el "contexto" del patrón Strategy: el resto de la aplicación
/// (rutas HTTP, casos de uso futuros) depende únicamente de este
/// registro, no de ninguna estrategia concreta -- añadir un ecosistema
/// nuevo consiste en implementar `PackagingStrategy` e invocar
/// [`PackagingRegistry::register`], sin tocar ningún otro punto del
/// sistema.
#[derive(Default)]
pub struct PackagingRegistry {
    strategies: HashMap<PackageEcosystem, Arc<dyn PackagingStrategy>>,
}

impl PackagingRegistry {
    /// Crea un registro vacío.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra una estrategia para el ecosistema que ella misma declara
    /// manejar, sustituyendo cualquier estrategia registrada
    /// previamente para ese mismo ecosistema.
    #[must_use]
    pub fn register(mut self, strategy: Arc<dyn PackagingStrategy>) -> Self {
        self.strategies.insert(strategy.ecosystem(), strategy);
        self
    }

    /// Busca la estrategia registrada para el ecosistema indicado.
    /// Devuelve `None` si ningún ecosistema fue registrado para ese
    /// ecosistema -- por ejemplo, porque su implementación todavía no
    /// existe.
    #[must_use]
    pub fn strategy_for(&self, ecosystem: PackageEcosystem) -> Option<Arc<dyn PackagingStrategy>> {
        self.strategies.get(&ecosystem).cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::PackageEcosystem;

    use super::cargo::CargoPackagingStrategy;
    use super::conan::ConanPackagingStrategy;
    use super::npm::NpmPackagingStrategy;
    use super::oci::OciPackagingStrategy;
    use super::pypi::PypiPackagingStrategy;
    use super::*;
    use crate::test_support::{
        InMemoryArtifactStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryRepositoryStore, InMemoryStorage,
    };

    fn cargo_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(CargoPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        ))
    }

    fn npm_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(NpmPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        ))
    }

    fn pypi_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(PypiPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryHttpClient::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            "http://127.0.0.1:3000".to_string(),
        ))
    }

    fn oci_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(OciPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryHttpClient::default()),
        ))
    }

    fn helm_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(OciPackagingStrategy::for_ecosystem(
            PackageEcosystem::Helm,
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryHttpClient::default()),
        ))
    }

    fn conan_strategy() -> Arc<dyn PackagingStrategy> {
        Arc::new(ConanPackagingStrategy::new(
            Arc::new(InMemoryArtifactStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryStorage::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        ))
    }

    #[test]
    fn registers_and_finds_a_strategy_by_ecosystem() {
        let registry = PackagingRegistry::new()
            .register(cargo_strategy())
            .register(npm_strategy())
            .register(pypi_strategy())
            .register(oci_strategy())
            .register(helm_strategy())
            .register(conan_strategy());

        assert!(registry.strategy_for(PackageEcosystem::Cargo).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Npm).is_some());
        assert!(registry.strategy_for(PackageEcosystem::PyPi).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Oci).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Helm).is_some());
        assert!(registry.strategy_for(PackageEcosystem::Conan).is_some());
    }

    #[test]
    fn returns_none_for_an_unregistered_ecosystem() {
        let registry = PackagingRegistry::new().register(cargo_strategy());

        assert!(registry.strategy_for(PackageEcosystem::Npm).is_none());
    }
}
