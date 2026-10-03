//! Saved reference and `Cosign` public key for the signed index pull.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::SecondsFormat;
use ferrobox_application::assay::SyncSettingsError;
use ferrobox_ports::osv_sync_store::OsvSyncSettings;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_manage_users, require_token_read};
use crate::error::ApiError;

/// Settings shown on the vulnerability-feed card.
#[derive(Debug, Serialize)]
pub(crate) struct OsvSyncResponse {
    reference: String,
    public_key_pem: String,
    last_outcome: Option<String>,
    last_detail: Option<String>,
    last_attempt_at: Option<String>,
}

impl From<OsvSyncSettings> for OsvSyncResponse {
    fn from(settings: OsvSyncSettings) -> Self {
        Self {
            reference: settings.reference,
            public_key_pem: settings.public_key_pem,
            last_outcome: settings.last_outcome,
            last_detail: settings.last_detail,
            last_attempt_at: settings
                .last_attempt_at
                .map(|at| at.to_rfc3339_opts(SecondsFormat::Secs, true)),
        }
    }
}

/// Body of `PUT /security/osv-sync`.
#[derive(Debug, Deserialize)]
pub(crate) struct SaveOsvSyncRequest {
    reference: String,
    public_key_pem: String,
}

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
) -> Result<Json<OsvSyncResponse>, ApiError> {
    require_token_read(&token)?;
    if !user.role().can_manage_users() {
        return Err(ApiError::Forbidden(
            "admin role required to read vulnerability index sync".to_string(),
        ));
    }
    match state.assays.sync_settings().await? {
        Some(settings) => Ok(Json(settings.into())),
        None => Err(ApiError::NotFound(
            "vulnerability index sync is not configured".to_string(),
        )),
    }
}

pub(crate) async fn put_settings(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Json(body): Json<SaveOsvSyncRequest>,
) -> Result<Json<OsvSyncResponse>, ApiError> {
    require_manage_users(&user, &token)?;
    let settings = state
        .assays
        .save_sync_settings(&body.reference, &body.public_key_pem)
        .await?;
    Ok(Json(settings.into()))
}

pub(crate) async fn run_sync(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
) -> Result<Json<OsvSyncResponse>, ApiError> {
    require_manage_users(&user, &token)?;
    let settings = state.assays.run_configured_sync().await?;
    Ok(Json(settings.into()))
}

impl From<SyncSettingsError> for ApiError {
    fn from(err: SyncSettingsError) -> Self {
        match &err {
            SyncSettingsError::InvalidReference { .. }
            | SyncSettingsError::InvalidPublicKey
            | SyncSettingsError::NotConfigured => Self::BadRequest(err.to_string()),
            SyncSettingsError::Persistence(_) => Self::Internal(err.to_string()),
        }
    }
}
