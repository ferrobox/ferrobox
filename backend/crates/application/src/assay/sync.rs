//! Pull a Cosign-signed OCI artifact whose layer is a `ferrobox-osv-index`.
//!
//! The layer bytes are the same file an air-gapped instance imports with
//! `POST /api/security/osv-feed`. A missing or invalid signature returns
//! an error and does not touch the active index.

use bytes::Bytes;
use ferrobox_ports::http_client::{HttpClient, HttpClientError, HttpResponse};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

use crate::packaging::cosign::{self, verify_simple};

use super::FeedImportError;

/// Why sync settings were not saved or a configured pull did not start.
#[derive(Debug, Error)]
pub enum SyncSettingsError {
    /// The reference is not `host/name:tag` or `host/name@sha256:…`.
    #[error("invalid vulnerability index reference '{reference}'")]
    InvalidReference {
        /// Text that could not be parsed.
        reference: String,
    },

    /// The text is empty or is not a PEM public key.
    #[error("Cosign public key must be a PEM public key")]
    InvalidPublicKey,

    /// No reference has been saved.
    #[error("vulnerability index sync is not configured")]
    NotConfigured,

    /// The settings row could not be saved or read.
    #[error(transparent)]
    Persistence(#[from] ferrobox_ports::osv_sync_store::OsvSyncStoreError),
}

/// Checks that `reference` is an OCI tag or digest reference.
///
/// # Errors
///
/// Returns [`SyncError::InvalidReference`] when it cannot be parsed.
pub fn validate_reference(reference: &str) -> Result<(), SyncError> {
    parse_reference(reference).map(|_| ())
}

/// OCI layer media type of a `ferrobox-osv-index` v1 document (JSON or gzip).
pub const INDEX_MEDIA_TYPE: &str = "application/vnd.ferrobox.osv-index.v1";

const MANIFEST_ACCEPT: &str =
    "application/vnd.oci.image.manifest.v1+json, application/vnd.oci.artifact.manifest.v1+json";
const INDEX_ACCEPT: &str = "application/vnd.oci.image.index.v1+json";

/// Why a signed pull did not install an index.
#[derive(Debug, Error)]
pub enum SyncError {
    /// The reference is not `host/name:tag` or `host/name@sha256:…`.
    #[error("invalid vulnerability index reference '{reference}'")]
    InvalidReference {
        /// Text that could not be parsed.
        reference: String,
    },

    /// The registry answered outside 2xx after authentication.
    #[error("registry HTTP {status} for {url}")]
    Registry {
        /// HTTP status.
        status: u16,
        /// Requested URL.
        url: String,
    },

    /// The registry could not be reached.
    #[error("registry request failed for {url}: {message}")]
    Transport {
        /// Requested URL.
        url: String,
        /// Failure detail.
        message: String,
    },

    /// The bearer token endpoint did not return a token.
    #[error("registry token endpoint failed: {0}")]
    Token(String),

    /// No Cosign signature manifest was found.
    #[error("vulnerability index {reference} has no Cosign signature")]
    Unsigned {
        /// OCI reference that was pulled.
        reference: String,
    },

    /// A signature was present and did not verify.
    #[error("Cosign signature rejected for {reference}")]
    SignatureRejected {
        /// OCI reference that was pulled.
        reference: String,
    },

    /// The manifest is not a single index layer of [`INDEX_MEDIA_TYPE`].
    #[error("vulnerability index {reference} is not a ferrobox-osv-index artifact")]
    UnexpectedLayout {
        /// OCI reference that was pulled.
        reference: String,
    },

    /// The registry digest header does not match the manifest bytes.
    #[error("registry manifest digest does not match the body for {reference}")]
    ManifestDigest {
        /// OCI reference that was pulled.
        reference: String,
    },

    /// The downloaded layer does not match the digest in the manifest.
    #[error("index blob digest mismatch for {reference}")]
    BlobDigest {
        /// OCI reference that was pulled.
        reference: String,
    },

    /// The bytes verified, but they are not a usable index.
    #[error(transparent)]
    Import(#[from] FeedImportError),
}

/// Result of a pull that verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The verified layer replaced the active index.
    Imported(ferrobox_ports::osv_feed_store::OsvFeedRecord),
    /// The verified layer is already the active index.
    Unchanged {
        /// Lowercase hex SHA-256 of that layer.
        sha256: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistryRef {
    registry: String,
    name: String,
    reference: String,
}

/// Downloads the index layer after a Cosign simple signature verifies.
///
/// # Errors
///
/// Returns [`SyncError`] when the reference, registry, signature, or
/// layout is unusable. The caller must not install an index in that case.
pub async fn pull_signed_index(
    http: &dyn HttpClient,
    reference: &str,
    public_key_pem: &str,
    token: Option<&str>,
) -> Result<Bytes, SyncError> {
    if public_key_pem.trim().is_empty() {
        return Err(SyncError::SignatureRejected {
            reference: reference.to_string(),
        });
    }
    let parsed = parse_reference(reference)?;
    let manifest_url = manifest_url(&parsed, &parsed.reference);
    let manifest = registry_get(http, &manifest_url, MANIFEST_ACCEPT, token).await?;
    let digest = sha256_digest(&manifest.body);
    if let Some(declared) = manifest.header("docker-content-digest")
        && declared.trim().to_ascii_lowercase() != digest
    {
        return Err(SyncError::ManifestDigest {
            reference: reference.to_string(),
        });
    }
    if !signature_verifies(http, &parsed, &digest, public_key_pem, token).await? {
        return Err(SyncError::SignatureRejected {
            reference: reference.to_string(),
        });
    }
    let layer = index_layer_digest(&manifest.body).ok_or_else(|| SyncError::UnexpectedLayout {
        reference: reference.to_string(),
    })?;
    let blob_url = blob_url(&parsed, &layer);
    let blob = registry_get(http, &blob_url, "application/octet-stream", token).await?;
    if sha256_digest(&blob.body) != layer {
        return Err(SyncError::BlobDigest {
            reference: reference.to_string(),
        });
    }
    Ok(blob.body)
}

fn parse_reference(input: &str) -> Result<RegistryRef, SyncError> {
    let input = input.trim();
    let (explicit, rest) = if let Some(rest) = input.strip_prefix("https://") {
        (Some("https"), rest)
    } else if let Some(rest) = input.strip_prefix("http://") {
        (Some("http"), rest)
    } else {
        (None, input)
    };
    let Some((host, path)) = rest.split_once('/') else {
        return Err(invalid(input));
    };
    if host.is_empty() || path.is_empty() || host.contains('@') {
        return Err(invalid(input));
    }
    let (name, reference) = split_name_and_reference(path).ok_or_else(|| invalid(input))?;
    let scheme = explicit.unwrap_or_else(|| {
        if host.starts_with("127.0.0.1")
            || host.starts_with("localhost")
            || host.starts_with("[::1]")
        {
            "http"
        } else {
            "https"
        }
    });
    Ok(RegistryRef {
        registry: format!("{scheme}://{host}"),
        name,
        reference,
    })
}

fn invalid(reference: &str) -> SyncError {
    SyncError::InvalidReference {
        reference: reference.to_string(),
    }
}

fn split_name_and_reference(path: &str) -> Option<(String, String)> {
    if let Some((name, digest)) = path.rsplit_once('@') {
        let digest = digest.to_ascii_lowercase();
        if name.is_empty() || !is_sha256_digest(&digest) {
            return None;
        }
        return Some((name.to_string(), digest));
    }
    let (name, tag) = path.rsplit_once(':')?;
    if name.is_empty() || tag.is_empty() || tag.contains('/') {
        return None;
    }
    Some((name.to_string(), tag.to_string()))
}

fn is_sha256_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.chars().all(|character| character.is_ascii_hexdigit())
}

fn manifest_url(parsed: &RegistryRef, reference: &str) -> String {
    format!(
        "{}/v2/{}/manifests/{reference}",
        parsed.registry, parsed.name
    )
}

fn blob_url(parsed: &RegistryRef, digest: &str) -> String {
    format!("{}/v2/{}/blobs/{digest}", parsed.registry, parsed.name)
}

fn referrers_url(parsed: &RegistryRef, digest: &str) -> String {
    format!("{}/v2/{}/referrers/{digest}", parsed.registry, parsed.name)
}

fn signature_tag(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    if !is_sha256_digest(digest) {
        return None;
    }
    Some(format!("sha256-{hex}.sig"))
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

async fn signature_verifies(
    http: &dyn HttpClient,
    parsed: &RegistryRef,
    manifest_digest: &str,
    public_key_pem: &str,
    token: Option<&str>,
) -> Result<bool, SyncError> {
    let mut saw_signature = false;
    for body in signature_manifests(http, parsed, manifest_digest, token).await? {
        let Some(signature) = cosign::extract_simple_signature(&body) else {
            continue;
        };
        saw_signature = true;
        let payload_url = blob_url(parsed, &signature.payload_digest);
        let Ok(payload) = registry_get(
            http,
            &payload_url,
            "application/vnd.dev.cosign.simplesigning.v1+json",
            token,
        )
        .await
        else {
            continue;
        };
        if sha256_digest(&payload.body) != signature.payload_digest {
            continue;
        }
        if verify_simple(
            &payload.body,
            &signature.signature_b64,
            public_key_pem,
            Some(manifest_digest),
        ) {
            return Ok(true);
        }
    }
    if saw_signature {
        Ok(false)
    } else {
        Err(SyncError::Unsigned {
            reference: format!("{}/{}:{}", parsed.registry, parsed.name, parsed.reference),
        })
    }
}

async fn signature_manifests(
    http: &dyn HttpClient,
    parsed: &RegistryRef,
    manifest_digest: &str,
    token: Option<&str>,
) -> Result<Vec<Bytes>, SyncError> {
    let mut bodies = Vec::new();
    let referrers = referrers_url(parsed, manifest_digest);
    match registry_get(http, &referrers, INDEX_ACCEPT, token).await {
        Ok(response) => {
            for digest in referrer_digests(&response.body) {
                let url = manifest_url(parsed, &digest);
                if let Ok(manifest) = registry_get(http, &url, MANIFEST_ACCEPT, token).await {
                    bodies.push(manifest.body);
                }
            }
        }
        Err(SyncError::Registry { status: 404, .. }) => {}
        Err(err) => return Err(err),
    }
    if let Some(tag) = signature_tag(manifest_digest) {
        let url = manifest_url(parsed, &tag);
        match registry_get(http, &url, MANIFEST_ACCEPT, token).await {
            Ok(response) => bodies.push(response.body),
            Err(SyncError::Registry { status: 404, .. }) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(bodies)
}

fn referrer_digests(body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let Some(manifests) = value.get("manifests").and_then(Value::as_array) else {
        return Vec::new();
    };
    manifests
        .iter()
        .filter_map(|item| {
            let artifact_type = item
                .get("artifactType")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !artifact_type.is_empty() && artifact_type != cosign::COSIGN_SIMPLE_MEDIA_TYPE {
                return None;
            }
            item.get("digest")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .filter(|digest| is_sha256_digest(digest))
        })
        .collect()
}

fn index_layer_digest(manifest: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(manifest).ok()?;
    let layers = value.get("layers")?.as_array()?;
    let mut matches = layers.iter().filter_map(|layer| {
        let media_type = layer.get("mediaType").and_then(Value::as_str)?;
        if media_type != INDEX_MEDIA_TYPE {
            return None;
        }
        let digest = layer
            .get("digest")
            .and_then(Value::as_str)?
            .to_ascii_lowercase();
        is_sha256_digest(&digest).then_some(digest)
    });
    let digest = matches.next()?;
    matches.next().is_none().then_some(digest)
}

async fn registry_get(
    http: &dyn HttpClient,
    url: &str,
    accept: &str,
    token: Option<&str>,
) -> Result<HttpResponse, SyncError> {
    let first = send(http, url, accept, token).await?;
    if first.is_success() {
        return Ok(first);
    }
    if first.status != 401 {
        return Err(SyncError::Registry {
            status: first.status,
            url: url.to_string(),
        });
    }
    let Some(challenge) = bearer_header(&first) else {
        return Err(SyncError::Registry {
            status: 401,
            url: url.to_string(),
        });
    };
    let bearer = fetch_bearer_token(http, challenge).await?;
    let second = send(http, url, accept, Some(&bearer)).await?;
    if second.is_success() {
        return Ok(second);
    }
    Err(SyncError::Registry {
        status: second.status,
        url: url.to_string(),
    })
}

async fn send(
    http: &dyn HttpClient,
    url: &str,
    accept: &str,
    bearer: Option<&str>,
) -> Result<HttpResponse, SyncError> {
    let mut headers = vec![("accept", accept)];
    let authorization;
    if let Some(token) = bearer {
        authorization = format!("Bearer {token}");
        headers.push(("authorization", authorization.as_str()));
    }
    http.get_with_headers(url, &headers)
        .await
        .map_err(|err| match err {
            HttpClientError::Transport { url, message } => SyncError::Transport { url, message },
            HttpClientError::Status { status, url } => SyncError::Registry { status, url },
        })
}

fn bearer_header(response: &HttpResponse) -> Option<&str> {
    response.headers.iter().find_map(|(name, value)| {
        (name.eq_ignore_ascii_case("www-authenticate") && value.trim().starts_with("Bearer"))
            .then_some(value.as_str())
    })
}

async fn fetch_bearer_token(http: &dyn HttpClient, challenge: &str) -> Result<String, SyncError> {
    let rest = challenge
        .trim()
        .strip_prefix("Bearer")
        .ok_or_else(|| SyncError::Token("WWW-Authenticate is not Bearer".to_string()))?
        .trim();
    let mut realm = None;
    let mut service = None;
    let mut scope = None;
    for part in rest.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "realm" => realm = Some(value.to_string()),
            "service" => service = Some(value.to_string()),
            "scope" => scope = Some(value.to_string()),
            _ => {}
        }
    }
    let realm =
        realm.ok_or_else(|| SyncError::Token("Bearer challenge is missing realm".into()))?;
    let service =
        service.ok_or_else(|| SyncError::Token("Bearer challenge is missing service".into()))?;
    let url = token_request_url(&realm, &service, scope.as_deref())?;
    let response = http
        .get_with_headers(&url, &[])
        .await
        .map_err(|err| SyncError::Token(err.to_string()))?;
    if !response.is_success() {
        return Err(SyncError::Token(format!(
            "token endpoint returned HTTP {}",
            response.status
        )));
    }
    let parsed: TokenBody =
        serde_json::from_slice(&response.body).map_err(|err| SyncError::Token(err.to_string()))?;
    parsed
        .token
        .or(parsed.access_token)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| SyncError::Token("token response is missing token".into()))
}

fn token_request_url(realm: &str, service: &str, scope: Option<&str>) -> Result<String, SyncError> {
    let mut url = Url::parse(realm).map_err(|err| SyncError::Token(err.to_string()))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("service", service);
        if let Some(scope) = scope {
            pairs.append_pair("scope", scope);
        }
    }
    Ok(url.to_string())
}

#[derive(Deserialize)]
struct TokenBody {
    token: Option<String>,
    access_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use chrono::{Duration, Utc};
    use ferrobox_ports::osv_feed_store::{OsvFeedRecord, OsvFeedStore, OsvFeedStoreError};
    use ferrobox_ports::osv_sync_store::{
        OSV_SYNC_IMPORTED, OSV_SYNC_REJECTED, OsvSyncSettings, OsvSyncStore, OsvSyncStoreError,
    };
    use ferrobox_ports::storage::{StorageKey, StoragePort};
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer;
    use p256::pkcs8::EncodePublicKey;

    use super::*;
    use crate::assay::AssayService;
    use crate::test_support::{
        InMemoryAssayStore, InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryRepositoryStore,
        InMemoryStorage,
    };

    #[tokio::test]
    async fn remove_feed_drops_the_index_the_blob_and_the_sync_settings() {
        let storage = Arc::new(InMemoryStorage::default());
        let sync_store = Arc::new(MemorySyncStore::new());
        let storage_port: Arc<dyn StoragePort> = storage.clone();
        let feed_port: Arc<dyn OsvFeedStore> = Arc::new(MemoryStore::new());
        let service = AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            storage_port,
            Arc::new(InMemoryHttpClient::default()),
        )
        .with_feed_store(feed_port)
        .with_sync_store(sync_store);
        let bytes = index_bytes("2026-10-03");
        let digest = format!("{:x}", Sha256::digest(&bytes));
        service.import_feed(bytes, &digest).await.unwrap();
        service
            .save_sync_settings("ghcr.io/ferrobox/osv-db:2026-10-03", &public_pem())
            .await
            .unwrap();

        service.remove_feed().await.unwrap();

        assert!(service.imported_feed().await.unwrap().is_none());
        assert!(service.sync_settings().await.unwrap().is_none());
        assert!(
            !storage
                .exists(&StorageKey::new(format!("system/osv-feed/{digest}")))
                .await
                .unwrap()
        );
        service.remove_feed().await.unwrap();
    }

    #[test]
    fn parses_a_ghcr_tag_and_a_local_http_reference() {
        let ghcr = parse_reference("ghcr.io/ferrobox/osv-db:2026-10-03").unwrap();
        assert_eq!(ghcr.registry, "https://ghcr.io");
        assert_eq!(ghcr.name, "ferrobox/osv-db");
        assert_eq!(ghcr.reference, "2026-10-03");

        let local = parse_reference(
            "http://127.0.0.1:3000/11111111-1111-1111-1111-111111111111/osv-db:2026-10-03",
        )
        .unwrap();
        assert_eq!(local.registry, "http://127.0.0.1:3000");
        assert_eq!(local.name, "11111111-1111-1111-1111-111111111111/osv-db");
        assert_eq!(local.reference, "2026-10-03");

        let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let pinned = parse_reference(&format!("ghcr.io/ferrobox/osv-db@{digest}")).unwrap();
        assert_eq!(pinned.reference, digest);
        assert!(parse_reference("ghcr.io/ferrobox/osv-db").is_err());
    }

    #[tokio::test]
    async fn signed_pull_returns_the_index_bytes() {
        let index = index_bytes("2026-10-03");
        let published = publish(&index, true);
        let pulled = pull_signed_index(
            published.http.as_ref(),
            &published.reference,
            &published.pem,
            None,
        )
        .await
        .unwrap();
        assert_eq!(pulled, index);
    }

    #[tokio::test]
    async fn bearer_challenge_is_followed_before_the_manifest_is_read() {
        let index = index_bytes("2026-10-03");
        let published = publish(&index, true);
        let manifest_url = manifest_url(&published.parsed, &published.parsed.reference);
        published.http.stub_sequence(
            &manifest_url,
            vec![
                HttpResponse {
                    status: 401,
                    body: Bytes::new(),
                    headers: vec![(
                        "www-authenticate".to_string(),
                        r#"Bearer realm="http://127.0.0.1:5000/token",service="registry",scope="repository:ferrobox/osv-db:pull""#.to_string(),
                    )],
                },
                HttpResponse::new(200, published.manifest.clone()),
            ],
        );
        let token_url = token_request_url(
            "http://127.0.0.1:5000/token",
            "registry",
            Some("repository:ferrobox/osv-db:pull"),
        )
        .unwrap();
        published
            .http
            .stub(&token_url, 200, r#"{"token":"registry-token"}"#);

        let pulled = pull_signed_index(
            published.http.as_ref(),
            &published.reference,
            &published.pem,
            None,
        )
        .await
        .unwrap();
        assert_eq!(pulled, index);
    }

    #[tokio::test]
    async fn broken_signature_keeps_the_index_installed_by_curl() {
        let installed = index_bytes("2026-10-02");
        let next = index_bytes("2026-10-03");
        let published = publish(&next, true);
        let other = SigningKey::random(&mut rand::rngs::OsRng);
        let other_pem = other
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .unwrap();
        let store = Arc::new(MemoryStore::new());
        let service = service(published.http.clone(), store.clone());
        let digest = format!("{:x}", Sha256::digest(&installed));
        service.import_feed(installed, &digest).await.unwrap();
        let before = store.current().await.unwrap().unwrap();

        let err = service
            .sync_signed_feed(&published.reference, &other_pem, None)
            .await
            .unwrap_err();
        assert!(matches!(err, SyncError::SignatureRejected { .. }), "{err}");
        let after = store.current().await.unwrap().unwrap();
        assert_eq!(after, before);
        assert_eq!(after.dataset, "2026-10-02");
        assert_eq!(after.source, "file");
    }

    #[tokio::test]
    async fn missing_signature_keeps_the_index_installed_by_curl() {
        let installed = index_bytes("2026-10-02");
        let next = index_bytes("2026-10-03");
        let published = publish(&next, false);
        let store = Arc::new(MemoryStore::new());
        let service = service(published.http.clone(), store.clone());
        let digest = format!("{:x}", Sha256::digest(&installed));
        service.import_feed(installed, &digest).await.unwrap();

        let err = service
            .sync_signed_feed(&published.reference, &published.pem, None)
            .await
            .unwrap_err();
        assert!(matches!(err, SyncError::Unsigned { .. }), "{err}");
        let after = store.current().await.unwrap().unwrap();
        assert_eq!(after.dataset, "2026-10-02");
        assert_eq!(after.source, "file");
    }

    #[tokio::test]
    async fn a_new_signed_index_imports_as_sync_and_the_same_bytes_do_not() {
        let first = index_bytes("2026-10-02");
        let second = index_bytes("2026-10-03");
        let published = publish(&second, true);
        let store = Arc::new(MemoryStore::new());
        let service = service(published.http.clone(), store.clone());
        let digest = format!("{:x}", Sha256::digest(&first));
        service.import_feed(first, &digest).await.unwrap();

        let imported = service
            .sync_signed_feed(&published.reference, &published.pem, None)
            .await
            .unwrap();
        let SyncOutcome::Imported(record) = imported else {
            panic!("expected a new index");
        };
        assert_eq!(record.dataset, "2026-10-03");
        assert_eq!(record.source, "sync");
        let imported_at = record.imported_at;

        let again = service
            .sync_signed_feed(&published.reference, &published.pem, None)
            .await
            .unwrap();
        assert_eq!(
            again,
            SyncOutcome::Unchanged {
                sha256: record.sha256.clone(),
            }
        );
        let stored = store.current().await.unwrap().unwrap();
        assert_eq!(stored.imported_at, imported_at);
        assert_eq!(stored.source, "sync");
    }

    #[test]
    fn openssl_der_signature_verifies_as_cosign_simple_signing() {
        let dir = std::env::temp_dir().join(format!("ferrobox-osv-sync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("key.pem");
        let public = dir.join("pub.pem");
        let payload_path = dir.join("payload");
        let signature_path = dir.join("sig.bin");
        openssl(&[
            "genpkey",
            "-algorithm",
            "EC",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-out",
            &key.display().to_string(),
        ]);
        openssl(&[
            "pkey",
            "-in",
            &key.display().to_string(),
            "-pubout",
            "-out",
            &public.display().to_string(),
        ]);
        let subject = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let payload = format!(
            r#"{{"critical":{{"identity":{{"docker-reference":"osv-db"}},"image":{{"docker-manifest-digest":"{subject}"}},"type":"cosign container image signature"}}}}"#
        );
        std::fs::write(&payload_path, &payload).unwrap();
        openssl(&[
            "dgst",
            "-sha256",
            "-sign",
            &key.display().to_string(),
            "-out",
            &signature_path.display().to_string(),
            &payload_path.display().to_string(),
        ]);
        let signature = std::fs::read(&signature_path).unwrap();
        let encoded = STANDARD.encode(&signature);
        let openssl_b64 = openssl(&["base64", "-A", "-in", &signature_path.display().to_string()]);
        assert_eq!(
            String::from_utf8(openssl_b64.stdout).unwrap().trim(),
            encoded
        );
        let pem = std::fs::read_to_string(&public).unwrap();
        assert!(verify_simple(
            payload.as_bytes(),
            &encoded,
            &pem,
            Some(subject)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn invalid_settings_do_not_replace_a_saved_row() {
        let http = Arc::new(InMemoryHttpClient::default());
        let sync_store = Arc::new(MemorySyncStore::new());
        let service =
            service(http, Arc::new(MemoryStore::new())).with_sync_store(sync_store.clone());
        let pem = public_pem();
        let saved = service
            .save_sync_settings("ghcr.io/ferrobox/osv-db:2026-10-03", &pem)
            .await
            .unwrap();

        let reference = service
            .save_sync_settings("not a reference", &pem)
            .await
            .unwrap_err();
        assert!(matches!(
            reference,
            SyncSettingsError::InvalidReference { .. }
        ));
        let key = service
            .save_sync_settings(
                "ghcr.io/ferrobox/osv-db:2026-10-04",
                "-----BEGIN PRIVATE KEY-----\nMII\n-----END PRIVATE KEY-----",
            )
            .await
            .unwrap_err();
        assert!(matches!(key, SyncSettingsError::InvalidPublicKey));

        let current = sync_store.current().await.unwrap().unwrap();
        assert_eq!(current, saved);
    }

    #[tokio::test]
    async fn configured_sync_imports_with_the_saved_key() {
        let index = index_bytes("2026-10-03");
        let published = publish(&index, true);
        let feed = Arc::new(MemoryStore::new());
        let sync_store = Arc::new(MemorySyncStore::new());
        let service = service(published.http.clone(), feed.clone()).with_sync_store(sync_store);
        service
            .save_sync_settings(&published.reference, &published.pem)
            .await
            .unwrap();

        let settings = service.run_configured_sync().await.unwrap();
        assert_eq!(settings.last_outcome.as_deref(), Some(OSV_SYNC_IMPORTED));
        assert_eq!(settings.last_detail.as_deref(), Some("2026-10-03"));
        let record = feed.current().await.unwrap().unwrap();
        assert_eq!(record.dataset, "2026-10-03");
        assert_eq!(record.source, "sync");
    }

    #[tokio::test]
    async fn wrong_key_records_rejected_and_keeps_the_file_index() {
        let installed = index_bytes("2026-10-02");
        let next = index_bytes("2026-10-03");
        let published = publish(&next, true);
        let other = SigningKey::random(&mut rand::rngs::OsRng);
        let other_pem = other
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .unwrap();
        let feed = Arc::new(MemoryStore::new());
        let sync_store = Arc::new(MemorySyncStore::new());
        let service = service(published.http.clone(), feed.clone()).with_sync_store(sync_store);
        let digest = format!("{:x}", Sha256::digest(&installed));
        service.import_feed(installed, &digest).await.unwrap();
        service
            .save_sync_settings(&published.reference, &other_pem)
            .await
            .unwrap();

        let settings = service.run_configured_sync().await.unwrap();
        assert_eq!(settings.last_outcome.as_deref(), Some(OSV_SYNC_REJECTED));
        let after = feed.current().await.unwrap().unwrap();
        assert_eq!(after.dataset, "2026-10-02");
        assert_eq!(after.source, "file");
        let attempted = settings.last_attempt_at.unwrap();
        assert!(
            !service
                .configured_sync_is_due(attempted + Duration::seconds(31))
                .await
                .unwrap()
        );
        assert!(
            service
                .configured_sync_is_due(attempted + Duration::hours(24))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn registry_500_is_due_again_in_about_thirty_seconds() {
        let index = index_bytes("2026-10-03");
        let published = publish(&index, true);
        let manifest = manifest_url(&published.parsed, &published.parsed.reference);
        published.http.stub(&manifest, 500, "unavailable");
        let sync_store = Arc::new(MemorySyncStore::new());
        let service = service(published.http.clone(), Arc::new(MemoryStore::new()))
            .with_sync_store(sync_store);
        service
            .save_sync_settings(&published.reference, &published.pem)
            .await
            .unwrap();

        let settings = service.run_configured_sync().await.unwrap();
        assert_eq!(settings.last_outcome.as_deref(), Some(OSV_SYNC_REJECTED));
        assert!(
            settings
                .last_detail
                .as_deref()
                .unwrap()
                .contains("HTTP 500")
        );
        let attempted = settings.last_attempt_at.unwrap();
        assert!(!service.configured_sync_is_due(attempted).await.unwrap());
        assert!(
            service
                .configured_sync_is_due(attempted + Duration::seconds(31))
                .await
                .unwrap()
        );
    }

    #[test]
    fn checksum_sidecar_does_not_change_the_index_layer() {
        let index = index_bytes("2026-10-03");
        let digest = sha256_digest(&index);
        let hex = digest.strip_prefix("sha256:").unwrap();
        let checksum = format!("{hex}  ferrobox-osv-index.json.gz\n");
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "artifactType": INDEX_MEDIA_TYPE,
            "layers": [
                {
                    "mediaType": INDEX_MEDIA_TYPE,
                    "digest": digest,
                    "size": index.len()
                },
                {
                    "mediaType": "text/plain",
                    "digest": sha256_digest(checksum.as_bytes()),
                    "size": checksum.len(),
                    "annotations": {
                        "org.opencontainers.image.title": "ferrobox-osv-index.json.gz.sha256"
                    }
                }
            ]
        }))
        .unwrap();
        assert_eq!(
            index_layer_digest(&manifest).as_deref(),
            Some(digest.as_str())
        );
    }

    struct Published {
        http: Arc<InMemoryHttpClient>,
        reference: String,
        pem: String,
        parsed: RegistryRef,
        manifest: Bytes,
    }

    fn publish(index: &Bytes, sign: bool) -> Published {
        let signing = SigningKey::random(&mut rand::rngs::OsRng);
        let pem = signing
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .unwrap();
        let reference = "http://127.0.0.1:5000/ferrobox/osv-db:2026-10-03".to_string();
        let parsed = parse_reference(&reference).unwrap();
        let layer_digest = sha256_digest(index);
        let config = Bytes::from_static(b"{}");
        let config_digest = sha256_digest(&config);
        let manifest = Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "artifactType": INDEX_MEDIA_TYPE,
                "config": {
                    "mediaType": "application/vnd.oci.empty.v1+json",
                    "digest": config_digest,
                    "size": config.len()
                },
                "layers": [{
                    "mediaType": INDEX_MEDIA_TYPE,
                    "digest": layer_digest,
                    "size": index.len()
                }]
            }))
            .unwrap(),
        );
        let manifest_digest = sha256_digest(&manifest);
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            &manifest_url(&parsed, &parsed.reference),
            200,
            manifest.clone(),
        );
        http.stub(&blob_url(&parsed, &layer_digest), 200, index.clone());
        if sign {
            let payload = Bytes::from(
                serde_json::to_vec(&serde_json::json!({
                    "critical": {
                        "identity": { "docker-reference": "ferrobox/osv-db" },
                        "image": { "docker-manifest-digest": manifest_digest },
                        "type": "cosign container image signature"
                    }
                }))
                .unwrap(),
            );
            let payload_digest = sha256_digest(&payload);
            let signature: p256::ecdsa::Signature = signing.sign(&payload);
            let encoded = STANDARD.encode(signature.to_der());
            let signature_manifest = Bytes::from(
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion": 2,
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "artifactType": cosign::COSIGN_SIMPLE_MEDIA_TYPE,
                    "config": {
                        "mediaType": "application/vnd.oci.empty.v1+json",
                        "digest": payload_digest,
                        "size": payload.len()
                    },
                    "layers": [{
                        "mediaType": cosign::COSIGN_SIMPLE_MEDIA_TYPE,
                        "digest": payload_digest,
                        "size": payload.len(),
                        "annotations": {
                            cosign::COSIGN_SIGNATURE_ANNOTATION: encoded
                        }
                    }],
                    "subject": {
                        "mediaType": "application/vnd.oci.image.manifest.v1+json",
                        "digest": manifest_digest,
                        "size": manifest.len()
                    }
                }))
                .unwrap(),
            );
            let tag = signature_tag(&manifest_digest).unwrap();
            http.stub(&manifest_url(&parsed, &tag), 200, signature_manifest);
            http.stub(&blob_url(&parsed, &payload_digest), 200, payload);
        }
        Published {
            http,
            reference,
            pem,
            parsed,
            manifest,
        }
    }

    fn index_bytes(dataset: &str) -> Bytes {
        Bytes::from(format!(
            r#"{{"format":"ferrobox-osv-index","format_version":1,"dataset":"{dataset}","advisories":[{{"ecosystem":"npm","name":"lodash","id":"GHSA-35jh-r3h4-6jhm","aliases":["CVE-2021-23337"],"summary":"Command Injection in lodash","severity":"HIGH","fixed":"4.17.21"}}]}}"#
        ))
    }

    fn service(http: Arc<InMemoryHttpClient>, store: Arc<MemoryStore>) -> AssayService {
        AssayService::new(
            Arc::new(InMemoryAssayStore::default()),
            Arc::new(InMemoryPackageIndexStore::default()),
            Arc::new(InMemoryRepositoryStore::default()),
            Arc::new(InMemoryStorage::default()),
            http,
        )
        .with_feed_store(store)
    }

    struct MemoryStore {
        record: std::sync::Mutex<Option<OsvFeedRecord>>,
    }

    impl MemoryStore {
        fn new() -> Self {
            Self {
                record: std::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl OsvFeedStore for MemoryStore {
        async fn current(&self) -> Result<Option<OsvFeedRecord>, OsvFeedStoreError> {
            Ok(self.record.lock().unwrap().clone())
        }

        async fn save(&self, record: &OsvFeedRecord) -> Result<(), OsvFeedStoreError> {
            *self.record.lock().unwrap() = Some(record.clone());
            Ok(())
        }

        async fn delete(&self) -> Result<Option<OsvFeedRecord>, OsvFeedStoreError> {
            Ok(self.record.lock().unwrap().take())
        }
    }

    fn public_pem() -> String {
        SigningKey::random(&mut rand::rngs::OsRng)
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .unwrap()
    }

    struct MemorySyncStore {
        settings: std::sync::Mutex<Option<OsvSyncSettings>>,
    }

    impl MemorySyncStore {
        fn new() -> Self {
            Self {
                settings: std::sync::Mutex::new(None),
            }
        }
    }

    #[derive(Debug)]
    struct PlainError(&'static str);

    impl std::fmt::Display for PlainError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl std::error::Error for PlainError {}

    #[async_trait::async_trait]
    impl OsvSyncStore for MemorySyncStore {
        async fn current(&self) -> Result<Option<OsvSyncSettings>, OsvSyncStoreError> {
            Ok(self.settings.lock().unwrap().clone())
        }

        async fn save(
            &self,
            reference: &str,
            public_key_pem: &str,
        ) -> Result<OsvSyncSettings, OsvSyncStoreError> {
            let settings = OsvSyncSettings {
                reference: reference.to_string(),
                public_key_pem: public_key_pem.to_string(),
                last_outcome: None,
                last_detail: None,
                last_attempt_at: None,
                retry_after: None,
            };
            *self.settings.lock().unwrap() = Some(settings.clone());
            Ok(settings)
        }

        async fn record_attempt(
            &self,
            outcome: &str,
            detail: &str,
            attempted_at: chrono::DateTime<Utc>,
            retry_after: chrono::DateTime<Utc>,
        ) -> Result<OsvSyncSettings, OsvSyncStoreError> {
            let mut guard = self.settings.lock().unwrap();
            let Some(settings) = guard.as_mut() else {
                return Err(OsvSyncStoreError::Backend(Box::new(PlainError(
                    "osv sync settings are not configured",
                ))));
            };
            settings.last_outcome = Some(outcome.to_string());
            settings.last_detail = Some(detail.to_string());
            settings.last_attempt_at = Some(attempted_at);
            settings.retry_after = Some(retry_after);
            Ok(settings.clone())
        }

        async fn delete(&self) -> Result<(), OsvSyncStoreError> {
            *self.settings.lock().unwrap() = None;
            Ok(())
        }
    }

    fn openssl(args: &[&str]) -> std::process::Output {
        let output = std::process::Command::new("openssl")
            .args(args)
            .output()
            .expect("openssl");
        assert!(
            output.status.success(),
            "openssl {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}
