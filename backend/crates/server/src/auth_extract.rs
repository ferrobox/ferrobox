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

/// Token Bearer que `GET /v2/token` emite sin credenciales para que
/// `docker pull` anónimo pueda superar el ping de `/v2/`. Las escrituras
/// lo rechazan porque no es un secreto de API real.
pub(crate) const OCI_ANONYMOUS_TOKEN: &str = "anonymous";

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
        oci_bearer_challenge(public_base_url, path),
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
    #[allow(dead_code)]
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
                &path,
                "missing Authorization credentials",
            ));
        }
        return Err(AuthError::Missing);
    };

    let principal = match state.authenticate_token.execute(&secret).await {
        Ok(principal) => principal,
        Err(_) if is_oci => {
            return Ok(oci_unauthorized_response(
                &state.public_base_url,
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
}
