//! HTTP routes for user administration (`Admin` role only).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::UserId;
use ferrobox_domain::user::{Email, Username};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_manage_users;
use ferrobox_domain::api_token::ApiTokenName;

use crate::dto::{
    ApiTokenCreatedResponse, CreateRobotRequest, CreateRobotResponse, CreateUserRequest,
    ResetUserPasswordRequest, UpdateUserRoleRequest, UserResponse, token_scope_labels,
};
use crate::error::ApiError;

pub(crate) async fn list_users(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    require_manage_users(&user, &token)?;
    let users = state.list_users.execute().await?;
    Ok(Json(users.iter().map(UserResponse::from).collect()))
}

pub(crate) async fn create_user(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Json(payload): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    require_manage_users(&user, &token)?;

    let username =
        Username::parse(payload.username).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let email = Email::parse(payload.email).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let created = state
        .create_user
        .execute(username, email, &payload.password, payload.role.into())
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::UserCreated,
        AuditTargetKind::User,
        created.username().to_string(),
        created.role().as_str(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(UserResponse::from(&created))))
}

pub(crate) async fn create_robot(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Json(payload): Json<CreateRobotRequest>,
) -> Result<(StatusCode, Json<CreateRobotResponse>), ApiError> {
    require_manage_users(&user, &token)?;

    let username =
        Username::parse(payload.username).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let token_name = ApiTokenName::parse(payload.token_name)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let expires_at = crate::auth::parse_optional_expiry(payload.expires_at.as_deref())?;

    let created = state
        .create_user
        .execute_robot(username, payload.role.into())
        .await?;
    let token = state
        .create_api_token
        .execute(created.id(), token_name, expires_at)
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::UserCreated,
        AuditTargetKind::User,
        created.username().to_string(),
        "robot",
    )
    .await;
    crate::audit::record(
        &state,
        &user,
        AuditAction::TokenCreated,
        AuditTargetKind::Token,
        token.token.name().to_string(),
        token.token.prefix().to_string(),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CreateRobotResponse {
            user: UserResponse::from(&created),
            token: ApiTokenCreatedResponse {
                id: token.token.id().to_string(),
                name: token.token.name().to_string(),
                prefix: token.token.prefix().to_string(),
                token: token.plaintext_secret,
                expires_at: token.token.expires_at().map(|at| at.to_rfc3339()),
                scopes: token_scope_labels(&token.token),
            },
        }),
    ))
}

pub(crate) async fn delete_user(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_manage_users(&user, &token)?;

    state
        .delete_user
        .execute(user.id(), UserId::from(user_id))
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::UserDeleted,
        AuditTargetKind::User,
        user_id.to_string(),
        "",
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn update_user_role(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<UpdateUserRoleRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    require_manage_users(&user, &token)?;

    let updated = state
        .change_user_role
        .execute(UserId::from(user_id), payload.role.into())
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::UserRoleChanged,
        AuditTargetKind::User,
        updated.username().to_string(),
        updated.role().as_str(),
    )
    .await;

    Ok(Json(UserResponse::from(&updated)))
}

pub(crate) async fn reset_user_password(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<ResetUserPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    require_manage_users(&user, &token)?;

    state
        .reset_user_password
        .execute(user.id(), UserId::from(user_id), &payload.password)
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::UserPasswordReset,
        AuditTargetKind::User,
        user_id.to_string(),
        "",
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ferrobox_application::assay::AssayService;
    use ferrobox_application::authenticate_token::AuthenticateTokenUseCase;
    use ferrobox_application::change_password::ChangePasswordUseCase;
    use ferrobox_application::create_repository::CreateRepositoryUseCase;
    use ferrobox_application::delete_artifact::DeleteArtifactUseCase;
    use ferrobox_application::delete_repository::DeleteRepositoryUseCase;
    use ferrobox_application::download_artifact::DownloadArtifactUseCase;
    use ferrobox_application::get_repository::GetRepositoryUseCase;
    use ferrobox_application::list_repositories::ListRepositoriesUseCase;
    use ferrobox_application::list_repository_artifacts::ListRepositoryArtifactsUseCase;
    use ferrobox_application::login::LoginUseCase;
    use ferrobox_application::manage_api_tokens::{
        CreateApiTokenUseCase, ListApiTokensUseCase, RevokeApiTokenUseCase,
    };
    use ferrobox_application::manage_groups::GroupService;
    use ferrobox_application::manage_users::{
        ChangeUserRoleUseCase, CreateUserUseCase, DeleteUserUseCase, ListUsersUseCase,
        ResetUserPasswordUseCase,
    };
    use ferrobox_application::packaging::PackagingRegistry;
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::quota::QuotaService;
    use ferrobox_application::retention::RetentionService;
    use ferrobox_application::search_packages::SearchPackagesUseCase;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryGroupStore,
        InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore, InMemoryReplicaStore,
        InMemoryRepositoryStore, InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore,
        InMemoryWebhookStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::user::Role;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::AppState;

    #[allow(clippy::too_many_lines)]
    async fn fixture() -> (Router, String, String, String, String) {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let assay_store = Arc::new(InMemoryAssayStore::default());
        let retention_store = Arc::new(InMemoryRetentionStore::default());
        let http_client = Arc::new(InMemoryHttpClient::default());
        let quota = QuotaService::new(
            repository_store.clone(),
            artifact_store.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        let search_packages =
            SearchPackagesUseCase::new(repository_store.clone(), package_index_store.clone());

        let state = Arc::new(AppState {
            create_repository: CreateRepositoryUseCase::new(repository_store.clone()),
            list_repositories: ListRepositoriesUseCase::new(repository_store.clone()),
            get_repository: GetRepositoryUseCase::new(repository_store.clone()),
            update_alloy_members: UpdateAlloyMembersUseCase::new(repository_store.clone()),
            publish_artifact: PublishArtifactUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                storage.clone(),
                quota.clone(),
            ),
            download_artifact: DownloadArtifactUseCase::new(
                artifact_store.clone(),
                storage.clone(),
            ),
            list_repository_artifacts: ListRepositoryArtifactsUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                package_index_store.clone(),
            ),
            delete_repository: DeleteRepositoryUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
            ),
            delete_artifact: DeleteArtifactUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
            ),
            repository_bundle:
                ferrobox_application::repository_bundle::RepositoryBundleService::new(
                    repository_store.clone(),
                    artifact_store.clone(),
                    package_index_store.clone(),
                    storage.clone(),
                    quota.clone(),
                ),
            replica: ferrobox_application::replica::ReplicaService::new(
                Arc::new(InMemoryReplicaStore::default()),
                repository_store.clone(),
                ferrobox_application::repository_bundle::RepositoryBundleService::new(
                    repository_store.clone(),
                    artifact_store.clone(),
                    package_index_store.clone(),
                    storage.clone(),
                    quota.clone(),
                ),
                http_client.clone(),
            ),
            promote_package: ferrobox_application::promote_package::PromotePackageUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                storage.clone(),
                quota.clone(),
            ),
            packaging: PackagingRegistry::new(),
            assays: AssayService::new(
                assay_store.clone(),
                package_index_store.clone(),
                repository_store.clone(),
                storage.clone(),
                http_client.clone(),
            ),
            admission: ferrobox_application::admission::AdmissionService::new(
                Arc::new(ferrobox_application::test_support::InMemoryAdmissionStore::default()),
                repository_store.clone(),
                ListRepositoryArtifactsUseCase::new(
                    repository_store.clone(),
                    artifact_store.clone(),
                    package_index_store.clone(),
                ),
            ),
            retention: RetentionService::new(
                repository_store.clone(),
                artifact_store,
                package_index_store,
                storage,
                assay_store,
                retention_store,
            ),
            quota,
            worm: ferrobox_application::worm::WormService::new(
                Arc::new(ferrobox_application::test_support::InMemoryWormStore::default()),
                repository_store.clone(),
            ),
            search_packages,
            public_base_url: "http://127.0.0.1:3000".to_string(),
            login: LoginUseCase::new(user_store.clone(), api_token_store.clone()),
            change_password: ChangePasswordUseCase::new(user_store.clone()),
            authenticate_token: AuthenticateTokenUseCase::new(
                user_store.clone(),
                api_token_store.clone(),
            ),
            create_api_token: CreateApiTokenUseCase::new(api_token_store.clone()),
            list_api_tokens: ListApiTokensUseCase::new(api_token_store.clone()),
            revoke_api_token: RevokeApiTokenUseCase::new(api_token_store),
            create_user: CreateUserUseCase::new(user_store.clone()),
            list_users: ListUsersUseCase::new(user_store.clone()),
            delete_user: DeleteUserUseCase::new(user_store.clone()),
            change_user_role: ChangeUserRoleUseCase::new(user_store.clone()),
            reset_user_password: ResetUserPasswordUseCase::new(user_store.clone()),
            groups: GroupService::new(
                Arc::new(InMemoryGroupStore::default()),
                user_store.clone(),
                repository_store.clone(),
            ),
            webhooks: ferrobox_application::webhooks::WebhookService::new(
                Arc::new(InMemoryWebhookStore::default()),
                http_client.clone(),
                repository_store.clone(),
            ),
            audit: ferrobox_application::audit::AuditService::new(Arc::new(
                ferrobox_application::test_support::InMemoryAuditStore::default(),
            )),
            oidc: None,
        });

        let admin = state.create_user.seed("admin", Role::Admin).await.unwrap();
        let admin_token = state
            .create_api_token
            .execute(admin.id(), ApiTokenName::parse("admin").unwrap(), None)
            .await
            .unwrap()
            .plaintext_secret;
        let reader = state
            .create_user
            .seed("reader", Role::Reader)
            .await
            .unwrap();
        let reader_token = state
            .create_api_token
            .execute(reader.id(), ApiTokenName::parse("read").unwrap(), None)
            .await
            .unwrap()
            .plaintext_secret;

        (
            crate::build_router(state),
            admin_token,
            reader_token,
            admin.id().to_string(),
            reader.id().to_string(),
        )
    }

    async fn json_body(response: axum::http::Response<Body>) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn create_user_requires_email_and_a_strong_password() {
        let (app, admin_token, _reader, _admin_id, _reader_id) = fixture().await;

        let weak = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/users")
                    .header("Authorization", format!("Bearer {admin_token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "username": "dev",
                            "email": "dev@example.com",
                            "password": "secret",
                            "role": "developer"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(weak.status(), StatusCode::BAD_REQUEST);

        let created = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/users")
                    .header("Authorization", format!("Bearer {admin_token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "username": "dev",
                            "email": "dev@example.com",
                            "password": "Secret1a",
                            "role": "developer"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = json_body(created).await;
        assert_eq!(body["username"], "dev");
        assert_eq!(body["email"], "dev@example.com");
        assert_eq!(body["role"], "developer");
    }

    #[tokio::test]
    async fn reader_cannot_manage_users() {
        let (app, _admin, reader_token, _admin_id, _reader_id) = fixture().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/users")
                    .header("Authorization", format!("Bearer {reader_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_can_reset_another_user_password_but_not_own() {
        let (app, admin_token, _reader_token, admin_id, reader_id) = fixture().await;

        let self_reset = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/users/{admin_id}/password"))
                    .header("Authorization", format!("Bearer {admin_token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(json!({ "password": "NewPass1a" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(self_reset.status(), StatusCode::CONFLICT);

        let reset = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/users/{reader_id}/password"))
                    .header("Authorization", format!("Bearer {admin_token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(json!({ "password": "NewPass1a" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reset.status(), StatusCode::NO_CONTENT);
    }
}
