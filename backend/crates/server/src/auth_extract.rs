//! Extractor y middleware de autenticación Bearer / Token / Basic.

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::user::User;

use crate::AppState;
use crate::dto::ErrorResponse;

/// Token Bearer que `GET /v2/token` emite sin credenciales cuando el
/// `scope` es solo `pull` (o no hay scope). Las escrituras lo rechazan:
/// no es un secreto de API real, y Helm/ORAS no reintenta tras un 401.
pub(crate) const OCI_ANONYMOUS_TOKEN: &str = "anonymous";

/// URL base del `realm` Bearer que Docker usa para pedir el token.
///
/// Docker solo reenvía las credenciales de `docker login` al `realm` si
/// el host coincide con el del registro. `PUBLIC_BASE_URL` puede ser
/// `localhost` mientras el cliente usa `127.0.0.1` (o al revés), y el
/// push falla entonces con `missing Authorization credentials`.
/// Preferimos `Host` / `X-Forwarded-Host`. El esquema sale de
/// `X-Forwarded-Proto` o, si el host coincide, de `PUBLIC_BASE_URL`.
pub(crate) fn oci_realm_base(public_base_url: &str, headers: &HeaderMap) -> String {
    let public = public_base_url.trim_end_matches('/');
    let (public_scheme, public_host) = split_url_scheme_host(public);

    let forwarded_proto = header_csv_first(headers, "x-forwarded-proto")
        .filter(|value| value.eq_ignore_ascii_case("http") || value.eq_ignore_ascii_case("https"));

    let host = header_csv_first(headers, "x-forwarded-host")
        .or_else(|| header_text(headers, header::HOST))
        .filter(|host| is_safe_authority(host));

    match host {
        Some(host) => {
            let host_matches_public =
                public_host.is_some_and(|public_host| public_host.eq_ignore_ascii_case(&host));
            let scheme = forwarded_proto
                .as_deref()
                .or(if host_matches_public {
                    public_scheme
                } else {
                    None
                })
                .unwrap_or("http");
            format!("{scheme}://{host}")
        }
        None => public.to_string(),
    }
}

/// Construye el desafío Bearer que Docker espera: un `realm` con URL
/// absoluta hacia el emisor de tokens.
pub(crate) fn oci_bearer_challenge(public_base_url: &str, path: &str) -> HeaderValue {
    let realm = format!("{}/v2/token", public_base_url.trim_end_matches('/'));
    let value = match oci_repository_scope(path) {
        Some(scope) => {
            format!(r#"Bearer realm="{realm}",service="ferrobox",scope="{scope}""#)
        }
        None => format!(r#"Bearer realm="{realm}",service="ferrobox""#),
    };
    HeaderValue::from_str(&value).unwrap_or_else(|_| {
        HeaderValue::from_static(r#"Bearer realm="ferrobox",service="ferrobox""#)
    })
}

fn split_url_scheme_host(url: &str) -> (Option<&str>, Option<&str>) {
    let Some((scheme, rest)) = url.split_once("://") else {
        return (None, Some(url));
    };
    let scheme = if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
        Some(scheme)
    } else {
        None
    };
    let host = rest
        .split('/')
        .next()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    (scheme, host)
}

fn header_csv_first(headers: &HeaderMap, name: &'static str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    let first = value.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    }
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn is_safe_authority(host: &str) -> bool {
    !host.is_empty()
        && host.len() < 256
        && host.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'-' | b'[' | b']')
        })
}

fn oci_repository_scope(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/v2/")?.trim_matches('/');
    if rest.is_empty() || rest == "token" {
        return None;
    }
    let (repository_id, remainder) = rest.split_once('/')?;
    let name = strip_distribution_suffix(remainder)?;
    Some(format!("repository:{repository_id}/{name}:pull,push"))
}

fn strip_distribution_suffix(remainder: &str) -> Option<&str> {
    let remainder = remainder.trim_matches('/');
    remainder
        .strip_suffix("/tags/list")
        .or_else(|| remainder.rsplit_once("/manifests/").map(|(name, _)| name))
        .or_else(|| remainder.split_once("/blobs/").map(|(name, _)| name))
        .map(|name| name.trim_matches('/'))
        .filter(|name| !name.is_empty())
}

/// 401 del Distribution Spec con `WWW-Authenticate` usable por Docker.
pub(crate) fn oci_unauthorized_response(
    public_base_url: &str,
    request_headers: &HeaderMap,
    path: &str,
    message: &str,
) -> Response {
    let payload = serde_json::json!({
        "errors": [{ "code": "UNAUTHORIZED", "message": message }]
    });
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("docker-distribution-api-version"),
        HeaderValue::from_static("registry/2.0"),
    );
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.append(
        header::WWW_AUTHENTICATE,
        oci_bearer_challenge(&oci_realm_base(public_base_url, request_headers), path),
    );
    headers.append(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(r#"Basic realm="ferrobox""#),
    );
    (
        StatusCode::UNAUTHORIZED,
        headers,
        serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec()),
    )
        .into_response()
}

/// Usuario (y token) autenticados extraídos de la cabecera
/// `Authorization`.
#[derive(Clone)]
pub(crate) struct AuthenticatedUser {
    pub(crate) user: User,
    pub(crate) token: ApiToken,
}

/// Error de autenticación HTTP.
pub(crate) enum AuthError {
    Missing,
    Invalid,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let message = match self {
            Self::Missing => "missing Authorization credentials",
            Self::Invalid => "invalid or revoked API token",
        };

        let mut response = (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: message.to_string(),
            }),
        )
            .into_response();
        response.headers_mut().append(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Bearer realm="ferrobox""#),
        );
        response.headers_mut().append(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Basic realm="ferrobox""#),
        );
        response
    }
}

impl FromRequestParts<Arc<AppState>> for AuthenticatedUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or(AuthError::Missing)
    }
}

/// Middleware que exige un token Bearer / Token válido y lo deja en
/// las extensiones de la petición para los extractores posteriores.
pub(crate) async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, AuthError> {
    let path = request.uri().path().to_owned();
    let is_oci = path.starts_with("/v2/");

    let Some(secret) = extract_bearer_token(request.headers()) else {
        if is_oci {
            return Ok(oci_unauthorized_response(
                &state.public_base_url,
                request.headers(),
                &path,
                "missing Authorization credentials",
            ));
        }
        return Err(AuthError::Missing);
    };

    if is_oci && secret == OCI_ANONYMOUS_TOKEN {
        return Ok(oci_unauthorized_response(
            &state.public_base_url,
            request.headers(),
            &path,
            "anonymous token cannot write",
        ));
    }

    let principal = match state.authenticate_token.execute(&secret).await {
        Ok(principal) => principal,
        Err(_) if is_oci => {
            return Ok(oci_unauthorized_response(
                &state.public_base_url,
                request.headers(),
                &path,
                "invalid or revoked API token",
            ));
        }
        Err(_) => return Err(AuthError::Invalid),
    };

    request.extensions_mut().insert(AuthenticatedUser {
        user: principal.user,
        token: principal.token,
    });

    Ok(next.run(request).await)
}

/// Extrae el secreto de `Authorization`.
///
/// `cargo publish` envía el token **tal cual** (`Authorization: fb_…`),
/// sin esquema. La UI y curl suelen usar `Bearer` o `Token`. `twine`
/// envía HTTP Basic (`__token__` / `fb_…`): se usa la contraseña.
pub(crate) fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    if let Some(key) = headers
        .get("x-nuget-apikey")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(key.to_string());
    }

    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    if value.is_empty() {
        return None;
    }

    let secret = strip_auth_scheme(value, "Bearer")
        .or_else(|| strip_auth_scheme(value, "Token"))
        .map(str::trim)
        .map(ToOwned::to_owned)
        .or_else(|| extract_basic_password(value))
        .unwrap_or_else(|| value.trim().to_string());

    if secret.is_empty() {
        None
    } else {
        Some(secret)
    }
}

fn extract_basic_password(value: &str) -> Option<String> {
    let encoded = strip_auth_scheme(value, "Basic")?.trim();
    let decoded = BASE64.decode(encoded.as_bytes()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (_username, password) = decoded.split_once(':')?;
    let password = password.trim();
    if password.is_empty() {
        None
    } else {
        Some(password.to_string())
    }
}

fn strip_auth_scheme<'a>(value: &'a str, scheme: &str) -> Option<&'a str> {
    let rest = value.get(scheme.len()..)?;
    let (separator, secret) = rest.split_at(1.min(rest.len()));
    if separator != " " || !value[..scheme.len()].eq_ignore_ascii_case(scheme) {
        return None;
    }
    Some(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(value).expect("valid header"),
        );
        map
    }

    #[test]
    fn extracts_nuget_api_key_header() {
        let mut map = HeaderMap::new();
        map.insert(
            HeaderName::from_static("x-nuget-apikey"),
            HeaderValue::from_static("fb_nuget"),
        );
        assert_eq!(extract_bearer_token(&map).as_deref(), Some("fb_nuget"));
    }

    #[test]
    fn extracts_cargo_raw_token() {
        assert_eq!(
            extract_bearer_token(&headers("fb_deadbeef")).as_deref(),
            Some("fb_deadbeef")
        );
    }

    #[test]
    fn extracts_bearer_and_token_schemes() {
        assert_eq!(
            extract_bearer_token(&headers("Bearer fb_secret")).as_deref(),
            Some("fb_secret")
        );
        assert_eq!(
            extract_bearer_token(&headers("token fb_secret")).as_deref(),
            Some("fb_secret")
        );
    }

    #[test]
    fn missing_or_blank_authorization_is_none() {
        assert_eq!(extract_bearer_token(&HeaderMap::new()), None);
        assert_eq!(extract_bearer_token(&headers("   ")), None);
    }

    #[test]
    fn extracts_basic_auth_password_as_token() {
        let encoded = BASE64.encode("__token__:fb_secret");
        assert_eq!(
            extract_bearer_token(&headers(&format!("Basic {encoded}"))).as_deref(),
            Some("fb_secret")
        );
    }

    #[test]
    fn oci_bearer_challenge_uses_token_realm_url() {
        let header = oci_bearer_challenge(
            "http://127.0.0.1:3000",
            "/v2/01a00518-8774-7f01-aa8c-3e6eb7d947d9/alpine/blobs/uploads/",
        );
        let value = header.to_str().unwrap();
        assert!(value.contains(r#"realm="http://127.0.0.1:3000/v2/token""#));
        assert!(value.contains(r#"service="ferrobox""#));
        assert!(value.contains(
            r#"scope="repository:01a00518-8774-7f01-aa8c-3e6eb7d947d9/alpine:pull,push""#
        ));

        let nested = oci_bearer_challenge(
            "http://127.0.0.1:3000",
            "/v2/01a00518-8774-7f01-aa8c-3e6eb7d947d9/bitnami/nginx/manifests/latest",
        );
        assert!(nested.to_str().unwrap().contains(
            r#"scope="repository:01a00518-8774-7f01-aa8c-3e6eb7d947d9/bitnami/nginx:pull,push""#
        ));
    }

    fn request_headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                HeaderValue::from_str(value).expect("header value"),
            );
        }
        map
    }

    #[test]
    fn oci_realm_base_prefers_request_host_over_public_url() {
        let headers = request_headers(&[("host", "localhost:3000")]);
        assert_eq!(
            oci_realm_base("http://127.0.0.1:3000", &headers),
            "http://localhost:3000"
        );
    }

    #[test]
    fn oci_realm_base_keeps_public_scheme_when_host_matches() {
        let headers = request_headers(&[("host", "ferrobox.example")]);
        assert_eq!(
            oci_realm_base("https://ferrobox.example", &headers),
            "https://ferrobox.example"
        );
    }

    #[test]
    fn oci_realm_base_uses_forwarded_host_and_proto() {
        let headers = request_headers(&[
            ("host", "127.0.0.1:3000"),
            ("x-forwarded-host", "registry.example:8443"),
            ("x-forwarded-proto", "https, http"),
        ]);
        assert_eq!(
            oci_realm_base("http://127.0.0.1:3000", &headers),
            "https://registry.example:8443"
        );
    }

    #[test]
    fn oci_realm_base_falls_back_to_public_url_without_host() {
        assert_eq!(
            oci_realm_base("http://127.0.0.1:3000", &HeaderMap::new()),
            "http://127.0.0.1:3000"
        );
    }

    #[test]
    fn oci_realm_base_ignores_unsafe_host() {
        let headers = request_headers(&[("host", r#"evil.com",service="x"#)]);
        assert_eq!(
            oci_realm_base("http://127.0.0.1:3000", &headers),
            "http://127.0.0.1:3000"
        );
    }
}
