use async_trait::async_trait;
use bytes::Bytes;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName};
use thiserror::Error;

/// Motivos por los que una operación sobre el índice de paquetes puede
/// fallar.
#[derive(Debug, Error)]
pub enum PackageIndexStoreError {
    /// El backend de persistencia concreto devolvió un error propio.
    #[error("persistence backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de persistencia del índice de paquetes: la lista, por
/// repositorio y coordenada, de las entradas que cada estrategia de
/// empaquetado (`PackagingStrategy`) necesita para responder al
/// protocolo de índice de su ecosistema (por ejemplo, el índice disperso
/// de `cargo`).
///
/// Este puerto es deliberadamente agnóstico del formato de cada
/// ecosistema: `entry` es un bloque de bytes ya serializado por la
/// estrategia correspondiente (una línea JSON para `cargo`, y
/// potencialmente otro formato para futuros ecosistemas). El puerto solo
/// se encarga de guardarlo y devolverlo en el orden de publicación,
/// igual que `ArtifactStore` no entiende el contenido binario que
/// almacena.
#[async_trait]
pub trait PackageIndexStore: Send + Sync {
    /// Inserta o reemplaza la entrada de índice de una coordenada de
    /// paquete concreta. `artifact_id` puede ser `None` cuando la
    /// entrada proviene del *upstream* de un `Mirror` y el binario
    /// todavía no se ha cacheado localmente.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackageIndexStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn upsert_entry(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
        artifact_id: Option<ArtifactId>,
        entry: Bytes,
    ) -> Result<(), PackageIndexStoreError>;

    /// Lista las entradas de índice de todas las versiones publicadas de
    /// un paquete, en el orden en que se publicaron.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackageIndexStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn entries_for_package(
        &self,
        repository_id: RepositoryId,
        ecosystem: PackageEcosystem,
        name: &PackageName,
    ) -> Result<Vec<Bytes>, PackageIndexStoreError>;

    /// Busca el identificador del artefacto binario asociado a una
    /// coordenada de paquete ya publicada. Devuelve `None` si esa
    /// coordenada nunca se publicó en ese repositorio.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackageIndexStoreError::Backend`] si el backend
    /// subyacente falla.
    async fn artifact_for(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<ArtifactId>, PackageIndexStoreError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ferrobox_domain::package_coordinate::PackageVersion;

    use super::*;

    type EntryMap = HashMap<(RepositoryId, PackageCoordinate), (Option<ArtifactId>, Bytes)>;

    #[derive(Default)]
    struct InMemoryPackageIndexStore {
        entries: Mutex<EntryMap>,
    }

    #[async_trait]
    impl PackageIndexStore for InMemoryPackageIndexStore {
        async fn upsert_entry(
            &self,
            repository_id: RepositoryId,
            coordinate: &PackageCoordinate,
            artifact_id: Option<ArtifactId>,
            entry: Bytes,
        ) -> Result<(), PackageIndexStoreError> {
            let mut entries = self.entries.lock().unwrap();
            let key = (repository_id, coordinate.clone());
            let merged_artifact = match entries.get(&key) {
                Some((existing, _)) => existing.or(artifact_id),
                None => artifact_id,
            };
            entries.insert(key, (merged_artifact, entry));
            Ok(())
        }

        async fn entries_for_package(
            &self,
            repository_id: RepositoryId,
            ecosystem: PackageEcosystem,
            name: &PackageName,
        ) -> Result<Vec<Bytes>, PackageIndexStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|((repo_id, coordinate), _)| {
                    *repo_id == repository_id
                        && coordinate.ecosystem() == ecosystem
                        && coordinate.name() == name
                })
                .map(|(_, (_, entry))| entry.clone())
                .collect())
        }

        async fn artifact_for(
            &self,
            repository_id: RepositoryId,
            coordinate: &PackageCoordinate,
        ) -> Result<Option<ArtifactId>, PackageIndexStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(&(repository_id, coordinate.clone()))
                .and_then(|(artifact_id, _)| *artifact_id))
        }
    }

    fn coordinate(version: &str) -> PackageCoordinate {
        PackageCoordinate::new(
            PackageEcosystem::Cargo,
            PackageName::parse("ferrobox-cli").unwrap(),
            PackageVersion::parse(version).unwrap(),
        )
    }

    #[tokio::test]
    async fn upsert_then_artifact_for_returns_the_associated_artifact() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();
        let artifact_id = ArtifactId::new();

        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(artifact_id),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let found = store
            .artifact_for(repository_id, &coordinate("1.0.0"))
            .await
            .unwrap();

        assert_eq!(found, Some(artifact_id));
    }

    #[tokio::test]
    async fn artifact_for_an_unpublished_coordinate_returns_none() {
        let store = InMemoryPackageIndexStore::default();

        let found = store
            .artifact_for(RepositoryId::new(), &coordinate("1.0.0"))
            .await
            .unwrap();

        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn entries_for_package_lists_every_published_version() {
        let store = InMemoryPackageIndexStore::default();
        let repository_id = RepositoryId::new();

        store
            .upsert_entry(
                repository_id,
                &coordinate("1.0.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{\"vers\":\"1.0.0\"}"),
            )
            .await
            .unwrap();
        store
            .upsert_entry(
                repository_id,
                &coordinate("1.1.0"),
                Some(ArtifactId::new()),
                Bytes::from_static(b"{\"vers\":\"1.1.0\"}"),
            )
            .await
            .unwrap();

        let entries = store
            .entries_for_package(
                repository_id,
                PackageEcosystem::Cargo,
                &PackageName::parse("ferrobox-cli").unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 2);
    }
}
