//! Adaptador de `StoragePort` contra cualquier backend compatible con la
//! interfaz de programación de S3 -- Garage en desarrollo local, y
//! potencialmente AWS S3 real u otros backends compatibles en producción,
//! sin cambiar una sola línea de este archivo, solo la configuración con
//! la que se construye el cliente.

use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};

/// Adaptador de [`StoragePort`] contra un backend compatible con S3.
pub struct S3StorageAdapter {
    client: Client,
    bucket: String,
}

impl S3StorageAdapter {
    /// Construye el adaptador a partir de un cliente S3 ya configurado
    /// (credenciales, *endpoint* y estilo de direccionamiento) y el
    /// nombre del bucket a usar.
    #[must_use]
    pub fn new(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }
}

#[async_trait]
impl StoragePort for S3StorageAdapter {
    async fn put(&self, key: &StorageKey, content: Bytes) -> Result<(), StorageError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .body(ByteStream::from(content))
            .send()
            .await
            .map_err(|err| StorageError::Backend(Box::new(err.into_service_error())))?;

        Ok(())
    }

    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|err| match err.into_service_error() {
                GetObjectError::NoSuchKey(_) => StorageError::NotFound(key.clone()),
                other => StorageError::Backend(Box::new(other)),
            })?;

        let data = output
            .body
            .collect()
            .await
            .map_err(|err| StorageError::Backend(Box::new(err)))?;

        Ok(data.into_bytes())
    }

    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|err| StorageError::Backend(Box::new(err.into_service_error())))?;

        Ok(())
    }

    async fn exists(&self, key: &StorageKey) -> Result<bool, StorageError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => match err.into_service_error() {
                HeadObjectError::NotFound(_) => Ok(false),
                other => Err(StorageError::Backend(Box::new(other))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};

    use super::*;

    fn client_from_env() -> Client {
        let endpoint = std::env::var("FERROBOX_TEST_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:3900".to_string());
        let access_key = std::env::var("FERROBOX_TEST_S3_ACCESS_KEY")
            .expect("set FERROBOX_TEST_S3_ACCESS_KEY to run this integration test");
        let secret_key = std::env::var("FERROBOX_TEST_S3_SECRET_KEY")
            .expect("set FERROBOX_TEST_S3_SECRET_KEY to run this integration test");

        let credentials = Credentials::new(access_key, secret_key, None, None, "ferrobox-test");
        let config = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("garage"))
            .endpoint_url(endpoint)
            .credentials_provider(credentials)
            .force_path_style(true)
            .build();

        Client::from_conf(config)
    }

    #[tokio::test]
    #[ignore = "requires a running Garage instance; run with `cargo test -- --ignored`"]
    async fn put_get_and_delete_a_real_object_in_garage() {
        let adapter = S3StorageAdapter::new(client_from_env(), "ferrobox-artifacts");
        let key = StorageKey::new("integration-test/hello.txt");

        adapter
            .put(&key, Bytes::from_static(b"hola, FerroBox"))
            .await
            .unwrap();

        let content = adapter.get(&key).await.unwrap();
        assert_eq!(content, Bytes::from_static(b"hola, FerroBox"));

        adapter.delete(&key).await.unwrap();
        assert!(!adapter.exists(&key).await.unwrap());
    }
}
