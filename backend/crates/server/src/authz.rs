//! Authorization checks from the user's role, group ACLs, and token scopes.

use axum::http::HeaderMap;
use ferrobox_application::authenticate_token::AuthenticateTokenUseCase;
use ferrobox_application::manage_groups::GroupService;
use ferrobox_domain::api_token::ApiToken;
use ferrobox_domain::group::RepositoryAccess;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::user::User;

use crate::auth_extract::{OCI_ANONYMOUS_TOKEN, extract_bearer_token};
use crate::error::ApiError;

/// Requires the token to allow reads.
pub(crate) fn require_token_read(token: &ApiToken) -> Result<(), ApiError> {
    if token.scopes().allows_read() {
        Ok(())
    } else {
        Err(ApiError::Forbidden("token does not allow read".to_string()))
    }
}

/// Requires the token to allow writes.
pub(crate) fn require_token_write(token: &ApiToken) -> Result<(), ApiError> {
    if token.scopes().allows_write() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "token does not allow write".to_string(),
        ))
    }
}

/// Exige que el usuario pueda gestionar otros usuarios (`Admin`).
pub(crate) fn require_manage_users(user: &User, token: &ApiToken) -> Result<(), ApiError> {
    require_token_write(token)?;
    if user.role().can_manage_users() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin role required to manage users".to_string(),
        ))
    }
}

/// Exige que el usuario pueda gestionar grupos (`Admin`).
pub(crate) fn require_manage_groups(user: &User, token: &ApiToken) -> Result<(), ApiError> {
    require_token_write(token)?;
    if user.role().can_manage_users() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin role required to manage groups".to_string(),
        ))
    }
}

/// Exige que el usuario pueda crear repositorios y publicar artefactos
/// (`Admin` o `Developer`) a nivel de instancia.
pub(crate) fn require_write_artifacts(user: &User, token: &ApiToken) -> Result<(), ApiError> {
    require_token_write(token)?;
    if user.role().can_write_artifacts() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin or developer role required to write artifacts".to_string(),
        ))
    }
}

/// Exige lectura del repositorio (rol de instancia o membresía de grupo).
pub(crate) async fn require_repo_read(
    groups: &GroupService,
    user: &User,
    token: &ApiToken,
    repository_id: RepositoryId,
) -> Result<RepositoryAccess, ApiError> {
    require_token_read(token)?;
    groups
        .access_on(user, repository_id)
        .await?
        .ok_or_else(|| ApiError::Forbidden("you do not have access to this repository".to_string()))
}

/// Exige escritura en el repositorio.
pub(crate) async fn require_repo_write(
    groups: &GroupService,
    user: &User,
    token: &ApiToken,
    repository_id: RepositoryId,
) -> Result<(), ApiError> {
    require_token_write(token)?;
    let access = require_repo_read(groups, user, token, repository_id).await?;
    if access.can_write() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "write access to this repository is required".to_string(),
        ))
    }
}

/// Lectura pública de un registro: si el repositorio está restringido,
/// exige un token con acceso de lectura.
pub(crate) async fn require_public_repo_read(
    groups: &GroupService,
    authenticate: &AuthenticateTokenUseCase,
    headers: &HeaderMap,
    repository_id: RepositoryId,
) -> Result<(), ApiError> {
    if !groups.is_restricted(repository_id).await? {
        return Ok(());
    }

    let Some((user, token)) = authenticate_from_headers(authenticate, headers).await? else {
        return Err(ApiError::Unauthorized(
            "authentication required to read this repository".to_string(),
        ));
    };
    require_repo_read(groups, &user, &token, repository_id).await?;
    Ok(())
}

async fn authenticate_from_headers(
    authenticate: &AuthenticateTokenUseCase,
    headers: &HeaderMap,
) -> Result<Option<(User, ApiToken)>, ApiError> {
    let Some(secret) = extract_bearer_token(headers) else {
        return Ok(None);
    };
    if secret == OCI_ANONYMOUS_TOKEN {
        return Ok(None);
    }
    let principal = authenticate.execute(&secret).await?;
    Ok(Some((principal.user, principal.token)))
}

#[cfg(test)]
mod tests {
    use ferrobox_domain::api_token::{ApiToken, ApiTokenName, TokenScopes};
    use ferrobox_domain::user::{Role, User, Username};

    use super::*;

    fn principal(role: Role, scopes: &[&str]) -> (User, ApiToken) {
        let user = User::new(Username::parse("admin").unwrap(), role);
        let token = ApiToken::new(
            user.id(),
            ApiTokenName::parse("ci").unwrap(),
            "fb_ab".into(),
        )
        .with_scopes(TokenScopes::parse(scopes.iter().copied()).unwrap());
        (user, token)
    }

    #[test]
    fn read_token_cannot_manage_users() {
        let (user, token) = principal(Role::Admin, &["read"]);
        assert!(require_manage_users(&user, &token).is_err());
        assert!(require_write_artifacts(&user, &token).is_err());
        assert!(require_token_read(&token).is_ok());
    }

    #[test]
    fn write_token_can_manage_when_role_allows() {
        let (user, token) = principal(Role::Admin, &["write"]);
        assert!(require_manage_users(&user, &token).is_ok());
        assert!(require_write_artifacts(&user, &token).is_ok());
    }

    #[test]
    fn unrestricted_token_keeps_role_checks() {
        let (admin, token) = principal(Role::Admin, &[]);
        assert!(require_manage_users(&admin, &token).is_ok());

        let (reader, token) = principal(Role::Reader, &[]);
        assert!(require_manage_users(&reader, &token).is_err());
        assert!(require_write_artifacts(&reader, &token).is_err());
    }
}
