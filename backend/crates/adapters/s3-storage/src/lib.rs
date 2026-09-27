//! `StoragePort` adapter against any backend compatible with the S3
//! programming interface -- Garage in local development, and potentially
//! real AWS S3 or other compatible backends in production, without
//! changing a single line of this file, only the configuration used to
//! build the client.

use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use ferrobox_ports::storage::{StorageError, StorageKey, StoragePort};

/// [`StoragePort`] adapter against an S3-compatible backend.
pub struct S3StorageAdapter {
    client: Client,
    bucket: String,
}

impl S3StorageAdapter {
    /// Builds the adapter from an already configured S3 client
    /// (credentials, *endpoint*, and addressing style) and the
    /// bucket name to use.
    #[must_use]
    pub fn new(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }

    /// Checks that the configured bucket exists and is reachable.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Backend`] if the *endpoint* does not
    /// respond, the credentials are unusable, or the bucket does not exist.
    pub async fn ensure_reachable(&self) -> Result<(), StorageError> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(backend_error)?;
        Ok(())
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
            .map_err(backend_error)?;

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
            .map_err(|err| {
                if err
                    .as_service_error()
                    .is_some_and(|inner| matches!(inner, GetObjectError::NoSuchKey(_)))
                {
                    StorageError::NotFound(key.clone())
                } else {
                    backend_error(err)
                }
            })?;

        let data = output.body.collect().await.map_err(backend_error)?;

        Ok(data.into_bytes())
    }

    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(backend_error)?;

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
            Err(err) => {
                if err
                    .as_service_error()
                    .is_some_and(|inner| matches!(inner, HeadObjectError::NotFound(_)))
                {
                    Ok(false)
                } else {
                    Err(backend_error(err))
                }
            }
        }
    }
}

fn backend_error(err: impl std::error::Error + Send + Sync + 'static) -> StorageError {
    StorageError::Backend(Box::new(BackendFailure(format_error_chain(&err))))
}

fn format_error_chain(err: &dyn std::error::Error) -> String {
    let mut parts = Vec::new();
    let mut current = Some(err);
    while let Some(item) = current {
        let text = item.to_string();
        if parts
            .last()
            .is_none_or(|previous: &String| previous != &text)
        {
            parts.push(text);
        }
        current = item.source();
    }
    parts.join(": ")
}

#[derive(Debug)]
struct BackendFailure(String);

impl std::fmt::Display for BackendFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BackendFailure {}

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
            .request_checksum_calculation(
                aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired,
            )
            .response_checksum_validation(
                aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired,
            )
            .build();

        Client::from_conf(config)
    }

    #[tokio::test]
    #[ignore = "requires a running Garage instance; run with `cargo test -- --ignored`"]
    async fn put_get_and_delete_a_real_object_in_garage() {
        let bucket =
            std::env::var("FERROBOX_TEST_S3_BUCKET").unwrap_or_else(|_| "ferrobox".to_string());
        let adapter = S3StorageAdapter::new(client_from_env(), bucket);
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

    #[test]
    fn backend_error_includes_the_source_chain() {
        let inner = std::io::Error::other("NoSuchBucket");
        let outer = std::io::Error::new(std::io::ErrorKind::Other, inner);
        let error = backend_error(outer);
        let message = error.to_string();
        assert!(
            message.contains("NoSuchBucket"),
            "expected the inner S3 code in {message}"
        );
    }
}
