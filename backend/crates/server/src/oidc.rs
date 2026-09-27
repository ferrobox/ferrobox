//! Public routes for federated sign-in (`OIDC`).

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use ferrobox_application::oidc::{OidcLoginService, OidcSettings};
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::oidc::OidcRoleMapping;
use serde::Deserialize;

use crate::AppState;
use crate::config::Config;
use crate::dto::OidcStatusResponse;
use crate::error::ApiError;

#[derive(Debug, Deserialize)]
pub(crate) struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Builds the service when `OIDC_ISSUER` and `OIDC_CLIENT_ID` are set.
pub(crate) fn service_from_config(
    config: &Config,
    http_client: Arc<dyn ferrobox_ports::http_client::HttpClient>,
    user_store: Arc<dyn ferrobox_ports::user_store::UserStore>,
    group_store: Arc<dyn ferrobox_ports::group_store::GroupStore>,
    api_token_store: Arc<dyn ferrobox_ports::api_token_store::ApiTokenStore>,
) -> Option<OidcLoginService> {
    let issuer = config.oidc_issuer.as_deref()?.trim();
    let client_id = config.oidc_client_id.as_deref()?.trim();
    if issuer.is_empty() || client_id.is_empty() {
        return None;
    }
    let public = config.public_base_url.trim_end_matches('/');
    let redirect_uri = config.oidc_redirect_uri.clone().unwrap_or_else(|| {
        if config.frontend_dir.is_some() {
            format!("{public}/api/auth/oidc/callback")
        } else {
            format!("{public}/auth/oidc/callback")
        }
    });
    let success_redirect = config
        .oidc_success_redirect
        .clone()
        .unwrap_or_else(|| format!("{public}/login"));
    let mapping = role_mapping(
        config.oidc_admin_roles.as_deref(),
        config.oidc_developer_roles.as_deref(),
        config.oidc_reader_roles.as_deref(),
    );
    let settings = OidcSettings::new(
        issuer,
        client_id,
        config.oidc_client_secret.clone(),
        redirect_uri,
        success_redirect,
    )
    .with_role_mapping(mapping)
    .with_extra_role_claim(config.oidc_role_claim.clone())
    .with_group_claim(config.oidc_group_claim.clone().unwrap_or_default())
    .with_auto_create_groups(config.oidc_auto_create_groups)
    .with_scopes(config.oidc_scopes.clone().unwrap_or_default());
    Some(
        OidcLoginService::new(
            settings,
            http_client,
            user_store,
            group_store,
            api_token_store,
        )
        .with_session_ttl(config.session_ttl),
    )
}

fn role_mapping(
    admin: Option<&str>,
    developer: Option<&str>,
    reader: Option<&str>,
) -> OidcRoleMapping {
    if admin.is_none() && developer.is_none() && reader.is_none() {
        return OidcRoleMapping::ferrobox_defaults();
    }
    OidcRoleMapping::new(
        split_roles(admin.unwrap_or("ferrobox-admin")),
        split_roles(developer.unwrap_or("ferrobox-developer")),
        split_roles(reader.unwrap_or("ferrobox-reader")),
    )
}

fn split_roles(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) async fn status(State(state): State<Arc<AppState>>) -> Json<OidcStatusResponse> {
    match &state.oidc {
        Some(oidc) => {
            let status = oidc.status();
            Json(OidcStatusResponse {
                enabled: status.enabled,
                issuer: status.issuer,
            })
        }
        None => Json(OidcStatusResponse {
            enabled: false,
            issuer: None,
        }),
    }
}

pub(crate) async fn start(State(state): State<Arc<AppState>>) -> Result<Redirect, ApiError> {
    let oidc = state.oidc.as_ref().ok_or(ApiError::NotFound(
        "single sign-on is not configured".to_string(),
    ))?;
    let url = oidc.start().await?;
    Ok(Redirect::temporary(&url))
}

pub(crate) async fn logout(State(state): State<Arc<AppState>>) -> Result<Redirect, ApiError> {
    let oidc = state.oidc.as_ref().ok_or(ApiError::NotFound(
        "single sign-on is not configured".to_string(),
    ))?;
    let url = oidc.logout_url().await?;
    Ok(Redirect::temporary(&url))
}

pub(crate) async fn callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OidcCallbackQuery>,
) -> Result<Response, ApiError> {
    let oidc = state.oidc.as_ref().ok_or(ApiError::NotFound(
        "single sign-on is not configured".to_string(),
    ))?;
    let success = oidc.settings().success_redirect().trim_end_matches('/');

    if let Some(error) = query.error {
        let detail = query.error_description.unwrap_or(error);
        return Ok(redirect_error(success, &detail));
    }

    let (Some(code), Some(state_param)) = (query.code, query.state) else {
        return Ok(redirect_error(success, "missing authorization code"));
    };

    match oidc.finish(&code, &state_param).await {
        Ok(result) => {
            crate::audit::record(
                &state,
                &result.user,
                AuditAction::UserSsoSignedIn,
                AuditTargetKind::User,
                result.user.username().to_string(),
                result.user.role().as_str(),
            )
            .await;
            let location = format!("{success}#sso_token={}", result.plaintext_secret);
            Ok(Redirect::temporary(&location).into_response())
        }
        Err(err) => Ok(redirect_error(success, &err.to_string())),
    }
}

fn redirect_error(success: &str, message: &str) -> Response {
    let encoded = urlencoding_query(message);
    let location = format!("{success}?sso_error={encoded}");
    Redirect::temporary(&location).into_response()
}

fn urlencoding_query(value: &str) -> String {
    let mut encoded = String::new();
    for ch in value.bytes() {
        match ch {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(ch));
            }
            other => {
                let _ = write!(encoded, "%{other:02X}");
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_roles_replace_the_defaults() {
        let mapping = role_mapping(Some("realm-admin"), None, None);
        assert_eq!(mapping.admin(), &["realm-admin".to_string()]);
        assert_eq!(mapping.developer(), &["ferrobox-developer".to_string()]);
    }
}
