//! HTTP import of the local OSV (*Open Source Vulnerabilities*) index.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use chrono::SecondsFormat;
use ferrobox_application::assay::FeedImportError;
use ferrobox_ports::osv_feed_store::OsvFeedRecord;
use serde::Serialize;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_manage_users, require_token_read};
use crate::error::ApiError;

/// Metadata returned after a successful import and by the status route.
#[derive(Debug, Serialize)]
pub(crate) struct OsvFeedResponse {
    dataset: String,
    sha256: String,
    advisory_count: u64,
    ecosystems: Vec<String>,
    imported_at: String,
    /// How this index arrived: `file` after an upload, `sync` after a signed pull.
    mode: String,
}

impl From<OsvFeedRecord> for OsvFeedResponse {
    fn from(record: OsvFeedRecord) -> Self {
        Self {
            dataset: record.dataset,
            sha256: record.sha256,
            advisory_count: record.advisory_count,
            ecosystems: record.ecosystems,
            imported_at: record
                .imported_at
                .to_rfc3339_opts(SecondsFormat::Secs, true),
            mode: record.source,
        }
    }
}

pub(crate) async fn get_feed(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { token, .. }: AuthenticatedUser,
) -> Result<Json<OsvFeedResponse>, ApiError> {
    require_token_read(&token)?;
    match state.assays.imported_feed().await? {
        Some(record) => Ok(Json(record.into())),
        None => Err(ApiError::NotFound(
            "no vulnerability feed has been imported".to_string(),
        )),
    }
}

pub(crate) async fn import_feed(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<OsvFeedResponse>, ApiError> {
    require_manage_users(&user, &token)?;
    let Some(expected) = sha256_header(&headers) else {
        return Err(ApiError::BadRequest(
            "missing X-FerroBox-Sha256 header".to_string(),
        ));
    };
    let record = state.assays.import_feed(body, &expected).await?;
    Ok(Json(record.into()))
}

pub(crate) async fn delete_feed(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
) -> Result<StatusCode, ApiError> {
    require_manage_users(&user, &token)?;
    state.assays.remove_feed().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn sha256_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-ferrobox-sha256")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

impl From<FeedImportError> for ApiError {
    fn from(err: FeedImportError) -> Self {
        match &err {
            FeedImportError::ChecksumMismatch { .. } | FeedImportError::Invalid(_) => {
                Self::BadRequest(err.to_string())
            }
            FeedImportError::NotConfigured
            | FeedImportError::Storage(_)
            | FeedImportError::Persistence(_)
            | FeedImportError::SyncPersistence(_) => Self::Internal(err.to_string()),
        }
    }
}
