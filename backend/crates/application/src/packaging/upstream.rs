//! Attaches a mirror's stored upstream credential to an outbound request.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::mirror_credential::MirrorCredential;
use ferrobox_ports::http_client::{HttpClient, HttpClientError, HttpResponse};
use ferrobox_ports::mirror_credential_store::MirrorCredentialStore;

use super::PackagingError;

/// `Authorization` value for a stored credential.
///
/// An empty username is `Bearer <secret>`. A username is HTTP Basic.
#[must_use]
pub fn authorization_value(credential: &MirrorCredential) -> String {
    if credential.username().is_empty() {
        format!("Bearer {}", credential.secret())
    } else {
        let raw = format!("{}:{}", credential.username(), credential.secret());
        format!("Basic {}", BASE64.encode(raw.as_bytes()))
    }
}

pub(crate) async fn upstream_authorization(
    credentials: Option<&dyn MirrorCredentialStore>,
    repository_id: RepositoryId,
) -> Result<Option<String>, PackagingError> {
    let Some(store) = credentials else {
        return Ok(None);
    };
    let Some(credential) = store.find(repository_id).await.map_err(|err| {
        PackagingError::InvalidUpstream(format!("could not load upstream credentials: {err}"))
    })?
    else {
        return Ok(None);
    };
    Ok(Some(authorization_value(&credential)))
}

/// `GET` that sends the mirror credential when one is stored.
///
/// A non-2xx status is an error, matching [`HttpClient::get`].
pub(crate) async fn upstream_get(
    http: &dyn HttpClient,
    credentials: Option<&dyn MirrorCredentialStore>,
    repository_id: RepositoryId,
    url: &str,
) -> Result<HttpResponse, PackagingError> {
    let response = upstream_get_with_headers(http, credentials, repository_id, url, &[]).await?;
    if response.is_success() {
        Ok(response)
    } else {
        Err(HttpClientError::Status {
            status: response.status,
            url: url.to_string(),
        }
        .into())
    }
}

/// `GET` with extra headers plus the mirror credential.
///
/// Like [`HttpClient::get_with_headers`], a non-2xx status is returned
/// in the response. Callers that mirror [`HttpClient::get`] should use
/// [`upstream_get`].
pub(crate) async fn upstream_get_with_headers(
    http: &dyn HttpClient,
    credentials: Option<&dyn MirrorCredentialStore>,
    repository_id: RepositoryId,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<HttpResponse, PackagingError> {
    let authorization = upstream_authorization(credentials, repository_id).await?;
    let Some(authorization) = authorization else {
        return Ok(http.get_with_headers(url, headers).await?);
    };
    let mut owned: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect();
    if !owned
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
    {
        owned.push(("authorization".to_string(), authorization));
    }
    let refs: Vec<(&str, &str)> = owned
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    Ok(http.get_with_headers(url, &refs).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_and_basic_headers() {
        let bearer = MirrorCredential::parse("", "token-1").unwrap();
        assert_eq!(authorization_value(&bearer), "Bearer token-1");

        let basic = MirrorCredential::parse("ci-bot", "secret").unwrap();
        assert_eq!(
            authorization_value(&basic),
            format!("Basic {}", BASE64.encode(b"ci-bot:secret"))
        );
    }
}
