use std::fmt;

use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;

/// Identifica de forma única un objeto binario dentro del almacenamiento
/// subyacente -- por ejemplo, una ruta o "key" dentro de un bucket S3.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageKey(String);

impl StorageKey {
    /// Construye una clave de almacenamiento a partir de cualquier
    /// cadena de texto.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Devuelve la clave como cadena de texto.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Motivos por los que una operación de almacenamiento puede fallar.
#[derive(Debug, Error)]
pub enum StorageError {
    /// No existe ningún objeto bajo la clave indicada.
    #[error("object not found: {0}")]
    NotFound(StorageKey),

    /// El backend de almacenamiento concreto (Garage, S3, sistema de
    /// archivos local, etc.) devolvió un error propio. Se envuelve como
    /// un error opaco porque este crate de puertos no depende de ningún
    /// backend concreto -- cada adaptador traduce su propio tipo de
    /// error a esta variante en el límite de la capa.
    #[error("storage backend failure: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Puerto de almacenamiento de contenido binario.
///
/// Cualquier backend (Garage, AWS S3, Azure Blob Storage, sistema de
/// archivos local...) que implemente este *trait* puede sustituir a
/// cualquier otro sin que el dominio ni la capa de aplicación necesiten
/// cambiar una sola línea -- es la materialización directa del primer
/// requisito arquitectónico de `FerroBox`: almacenamiento agnóstico.
#[async_trait]
pub trait StoragePort: Send + Sync {
    /// Sube el contenido binario bajo la clave indicada, sobrescribiendo
    /// cualquier objeto previo con la misma clave.
    ///
    /// # Errors
    ///
    /// Devuelve [`StorageError::Backend`] si el backend subyacente falla.
    async fn put(&self, key: &StorageKey, content: Bytes) -> Result<(), StorageError>;

    /// Descarga el contenido binario almacenado bajo la clave indicada.
    ///
    /// # Errors
    ///
    /// Devuelve [`StorageError::NotFound`] si la clave no existe, o
    /// [`StorageError::Backend`] si el backend subyacente falla.
    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError>;

    /// Elimina el objeto almacenado bajo la clave indicada. No es un
    /// error eliminar una clave que no existe.
    ///
    /// # Errors
    ///
    /// Devuelve [`StorageError::Backend`] si el backend subyacente falla.
    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError>;

    /// Comprueba si existe un objeto bajo la clave indicada.
    ///
    /// # Errors
    ///
    /// Devuelve [`StorageError::Backend`] si el backend subyacente falla.
    async fn exists(&self, key: &StorageKey) -> Result<bool, StorageError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// Adaptador falso, solo para pruebas: guarda los objetos en memoria
    /// en lugar de en un backend real. Nos permite comprobar que el
    /// diseño del puerto funciona de extremo a extremo antes de escribir
    /// el primer adaptador real (Garage) en el siguiente paso.
    #[derive(Default)]
    struct InMemoryStorage {
        objects: Mutex<HashMap<StorageKey, Bytes>>,
    }

    #[async_trait]
    impl StoragePort for InMemoryStorage {
        async fn put(&self, key: &StorageKey, content: Bytes) -> Result<(), StorageError> {
            self.objects.lock().unwrap().insert(key.clone(), content);
            Ok(())
        }

        async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError> {
            self.objects
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| StorageError::NotFound(key.clone()))
        }

        async fn delete(&self, key: &StorageKey) -> Result<(), StorageError> {
            self.objects.lock().unwrap().remove(key);
            Ok(())
        }

        async fn exists(&self, key: &StorageKey) -> Result<bool, StorageError> {
            Ok(self.objects.lock().unwrap().contains_key(key))
        }
    }

    #[tokio::test]
    async fn put_then_get_returns_the_same_content() {
        let storage = InMemoryStorage::default();
        let key = StorageKey::new("artifacts/example.bin");

        storage
            .put(&key, Bytes::from_static(b"hello"))
            .await
            .unwrap();
        let content = storage.get(&key).await.unwrap();

        assert_eq!(content, Bytes::from_static(b"hello"));
    }

    #[tokio::test]
    async fn get_on_a_missing_key_returns_not_found() {
        let storage = InMemoryStorage::default();
        let key = StorageKey::new("missing");

        let error = storage.get(&key).await.unwrap_err();

        assert!(matches!(error, StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_removes_the_object() {
        let storage = InMemoryStorage::default();
        let key = StorageKey::new("to-delete");
        storage
            .put(&key, Bytes::from_static(b"data"))
            .await
            .unwrap();

        storage.delete(&key).await.unwrap();

        assert!(!storage.exists(&key).await.unwrap());
    }

    #[test]
    fn backend_error_includes_the_inner_message() {
        let error = StorageError::Backend(Box::new(std::io::Error::other("NoSuchBucket")));

        assert_eq!(error.to_string(), "storage backend failure: NoSuchBucket");
    }
}
