//! Rutas HTTP de configuración de la instancia.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::dto::SettingsResponse;

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { .. }: AuthenticatedUser,
) -> Json<SettingsResponse> {
    Json(SettingsResponse {
        public_base_url: state.public_base_url.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}
