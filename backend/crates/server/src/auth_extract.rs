//! Extractor y middleware de autenticación Bearer / Token.

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::user::User;

use crate::AppState;
use crate::dto::ErrorResponse;

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
            Self::Missing => "missing Authorization bearer token",
            Self::Invalid => "invalid or revoked API token",
        };

        let mut response = (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: message.to_string(),
            }),
        )
            .into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Bearer realm="ferrobox""#),
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
    let secret = extract_bearer_token(request.headers()).ok_or(AuthError::Missing)?;

    let principal = state
        .authenticate_token
        .execute(&secret)
        .await
        .map_err(|_| AuthError::Invalid)?;

    request.extensions_mut().insert(AuthenticatedUser {
        user: principal.user,
        token: principal.token,
    });

    Ok(next.run(request).await)
}

/// Extrae el secreto de `Authorization: Bearer …` o
/// `Authorization: Token …` (este último lo usa `cargo publish`).
pub(crate) fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let secret = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("Token "))?;
    let secret = secret.trim();
    if secret.is_empty() {
        None
    } else {
        Some(secret.to_string())
    }
}
