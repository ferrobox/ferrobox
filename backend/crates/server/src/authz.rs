//! Comprobaciones de autorización basadas en el rol del usuario.

use ferrobox_domain::user::User;

use crate::error::ApiError;

/// Exige que el usuario pueda gestionar otros usuarios (`Admin`).
pub(crate) fn require_manage_users(user: &User) -> Result<(), ApiError> {
    if user.role().can_manage_users() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin role required to manage users".to_string(),
        ))
    }
}

/// Exige que el usuario pueda crear repositorios y publicar artefactos
/// (`Admin` o `Developer`).
pub(crate) fn require_write_artifacts(user: &User) -> Result<(), ApiError> {
    if user.role().can_write_artifacts() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin or developer role required to write artifacts".to_string(),
        ))
    }
}
