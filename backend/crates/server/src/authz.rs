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

/// Instance administration is only for a token that is not limited to
/// a repository list. A repository-scoped token keeps its read or write
/// scope on those repositories and cannot mint a wider credential.
pub(crate) fn require_unrestricted_repositories(token: &ApiToken) -> Result<(), ApiError> {
    if token.repositories().is_unrestricted() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "token is limited to specific repositories".to_string(),
        ))
    }
}

/// Requires the user to be able to manage other users (`Admin`).
pub(crate) fn require_manage_users(user: &User, token: &ApiToken) -> Result<(), ApiError> {
    require_unrestricted_repositories(token)?;
    require_token_write(token)?;
    if user.role().can_manage_users() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin role required to manage users".to_string(),
        ))
    }
}

/// Requires the user to be able to manage groups (`Admin`).
pub(crate) fn require_manage_groups(user: &User, token: &ApiToken) -> Result<(), ApiError> {
    require_unrestricted_repositories(token)?;
    require_token_write(token)?;
    if user.role().can_manage_users() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin role required to manage groups".to_string(),
        ))
    }
}

/// Requires the user to be able to create repositories and publish
/// artifacts (`Admin` or `Developer`) at instance level.
pub(crate) fn require_write_artifacts(user: &User, token: &ApiToken) -> Result<(), ApiError> {
    require_unrestricted_repositories(token)?;
    require_token_write(token)?;
    if user.role().can_write_artifacts() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin or developer role required to write artifacts".to_string(),
        ))
    }
}

/// Requires repository read access (instance role or group membership).
pub(crate) async fn require_repo_read(
    groups: &GroupService,
    user: &User,
    token: &ApiToken,
    repository_id: RepositoryId,
) -> Result<RepositoryAccess, ApiError> {
    require_token_read(token)?;
    if !token.repositories().allows(repository_id) {
        return Err(ApiError::Forbidden(
            "token is not scoped to this repository".to_string(),
        ));
    }
    groups
        .access_on(user, repository_id)
        .await?
        .ok_or_else(|| ApiError::Forbidden("you do not have access to this repository".to_string()))
}

/// Requires write access to the repository.
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

/// Public registry read: if the repository is restricted, requires a
/// token with read access.
pub(crate) async fn require_public_repo_read(
    groups: &GroupService,
    authenticate: &AuthenticateTokenUseCase,
    headers: &HeaderMap,
    repository_id: RepositoryId,
) -> Result<(), ApiError> {
    let restricted = groups.is_restricted(repository_id).await?;
    let Some(secret) = extract_bearer_token(headers) else {
        return if restricted {
            Err(ApiError::Unauthorized(
                "authentication required to read this repository".to_string(),
            ))
        } else {
            Ok(())
        };
    };
    if secret == OCI_ANONYMOUS_TOKEN {
        return if restricted {
            Err(ApiError::Unauthorized(
                "authentication required to read this repository".to_string(),
            ))
        } else {
            Ok(())
        };
    }

    match authenticate.execute(&secret).await {
        Ok(principal) => {
            if !principal.token.repositories().allows(repository_id) {
                return Err(ApiError::Forbidden(
                    "token is not scoped to this repository".to_string(),
                ));
            }
            if restricted {
                require_repo_read(groups, &principal.user, &principal.token, repository_id).await?;
            } else {
                require_token_read(&principal.token)?;
            }
            Ok(())
        }
        // A public repository stays readable when the presented token
        // cannot be authenticated. A valid token is still limited to
        // its repository list.
        Err(_) if !restricted => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_application::manage_groups::GroupService;
    use ferrobox_application::test_support::{
        InMemoryGroupStore, InMemoryRepositoryStore, InMemoryUserStore,
    };
    use ferrobox_domain::api_token::{ApiToken, ApiTokenName, TokenRepositories, TokenScopes};
    use ferrobox_domain::ids::RepositoryId;
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

    fn groups() -> GroupService {
        GroupService::new(
            Arc::new(InMemoryGroupStore::default()),
            Arc::new(InMemoryUserStore::default()),
            Arc::new(InMemoryRepositoryStore::default()),
        )
    }

    #[test]
    fn unrestricted_token_keeps_role_checks() {
        let (admin, token) = principal(Role::Admin, &[]);
        assert!(require_manage_users(&admin, &token).is_ok());

        let (reader, token) = principal(Role::Reader, &[]);
        assert!(require_manage_users(&reader, &token).is_err());
        assert!(require_write_artifacts(&reader, &token).is_err());
    }

    #[test]
    fn repository_scoped_token_cannot_administer_the_instance() {
        let (user, token) = principal(Role::Admin, &["write"]);
        let token = token.with_repositories(TokenRepositories::from_ids([RepositoryId::new()]));
        assert!(require_manage_users(&user, &token).is_err());
        assert!(require_write_artifacts(&user, &token).is_err());
        assert!(require_unrestricted_repositories(&token).is_err());
    }

    #[tokio::test]
    async fn repository_scoped_token_cannot_read_another_repository() {
        let (user, token) = principal(Role::Admin, &["read"]);
        let allowed = RepositoryId::new();
        let token = token.with_repositories(TokenRepositories::from_ids([allowed]));
        let groups = groups();

        assert!(
            require_repo_read(&groups, &user, &token, allowed)
                .await
                .is_ok()
        );
        assert!(
            require_repo_read(&groups, &user, &token, RepositoryId::new())
                .await
                .is_err()
        );
    }
}
