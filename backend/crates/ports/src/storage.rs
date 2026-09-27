use std::fmt;

use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;

/// Uniquely identifies a binary object inside the underlying storage
/// -- for example, a path or "key" inside an S3 bucket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageKey(String);

impl StorageKey {
    /// Builds a storage key from any text string.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Returns the key as a text string.
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

/// Reasons a storage operation can fail.
#[derive(Debug, Error)]
pub enum StorageError {
    /// No object exists under the given key.
    #[error("object not found: {0}")]
    NotFound(StorageKey),

    /// The concrete storage backend (Garage, S3, local filesystem,
    /// etc.) returned its own error. It is wrapped as an opaque error
    /// because this ports crate does not depend on any concrete
    /// backend -- each adapter translates its own error type to this
    /// variant at the layer boundary.
    #[error("storage backend failure: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Binary content storage port.
///
/// Any backend (Garage, AWS S3, Azure Blob Storage, local
/// filesystem...) that implements this *trait* can replace any other
/// without the domain or the application layer needing to change a
/// single line -- it is the direct materialization of the first
/// architectural requirement of `FerroBox`: storage-agnostic design.
#[async_trait]
pub trait StoragePort: Send + Sync {
    /// Uploads the binary content under the given key, overwriting any
    /// previous object with the same key.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Backend`] if the underlying backend fails.
    async fn put(&self, key: &StorageKey, content: Bytes) -> Result<(), StorageError>;

    /// Downloads the binary content stored under the given key.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] if the key does not exist, or
    /// [`StorageError::Backend`] if the underlying backend fails.
    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError>;

    /// Deletes the object stored under the given key. Deleting a
    /// missing key is not an error.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Backend`] if the underlying backend fails.
    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError>;

    /// Checks whether an object exists under the given key.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Backend`] if the underlying backend fails.
    async fn exists(&self, key: &StorageKey) -> Result<bool, StorageError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// Fake adapter, tests only: stores objects in memory instead of a
    /// real backend. Lets us verify the port design end to end before
    /// writing the first real adapter (Garage) in the next step.
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
