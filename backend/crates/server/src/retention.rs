//! Rutas HTTP de retención y recolección de basura.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::retention::RetentionPolicy;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_write_artifacts;
use crate::dto::{
    CleanupPreviewResponse, CleanupReportResponse, RetentionPolicyRequest, RetentionPolicyResponse,
};
use crate::error::ApiError;

pub(crate) async fn get_policy(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<RetentionPolicyResponse>, ApiError> {
    let policy = state
        .retention
        .get_policy(RepositoryId::from(repository_id))
        .await?;
    Ok(Json(RetentionPolicyResponse::from(policy)))
}

pub(crate) async fn save_policy(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<RetentionPolicyRequest>,
) -> Result<Json<RetentionPolicyResponse>, ApiError> {
    require_write_artifacts(&user)?;
    let policy = RetentionPolicy::new(payload.keep_last, payload.keep_days)?;
    let saved = state
        .retention
        .save_policy(RepositoryId::from(repository_id), policy)
        .await?;
    Ok(Json(RetentionPolicyResponse::from(saved)))
}

pub(crate) async fn dry_run(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<RetentionPolicyRequest>,
) -> Result<Json<CleanupPreviewResponse>, ApiError> {
    require_write_artifacts(&user)?;
    let policy = RetentionPolicy::new(payload.keep_last, payload.keep_days)?;
    let preview = state
        .retention
        .dry_run(RepositoryId::from(repository_id), policy)
        .await?;
    Ok(Json(CleanupPreviewResponse::from(preview)))
}

pub(crate) async fn apply(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<RetentionPolicyRequest>,
) -> Result<Json<CleanupPreviewResponse>, ApiError> {
    require_write_artifacts(&user)?;
    let policy = RetentionPolicy::new(payload.keep_last, payload.keep_days)?;
    let preview = state
        .retention
        .apply_policy(RepositoryId::from(repository_id), policy)
        .await?;
    Ok(Json(CleanupPreviewResponse::from(preview)))
}

pub(crate) async fn collect_garbage(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<CleanupReportResponse>, ApiError> {
    require_write_artifacts(&user)?;
    let report = state
        .retention
        .collect_garbage_only(RepositoryId::from(repository_id))
        .await?;
    Ok(Json(CleanupReportResponse::from(report)))
}

pub(crate) async fn dry_run_garbage_collection(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<CleanupPreviewResponse>, ApiError> {
    require_write_artifacts(&user)?;
    let preview = state.retention.dry_run_garbage_collection().await?;
    Ok(Json(CleanupPreviewResponse::from(preview)))
}

pub(crate) async fn collect_garbage_all(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<CleanupPreviewResponse>, ApiError> {
    require_write_artifacts(&user)?;
    let preview = state.retention.collect_garbage_all().await?;
    Ok(Json(CleanupPreviewResponse::from(preview)))
}

#[cfg(test)]
mod tests {
    use crate::AppState;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ferrobox_application::authenticate_token::AuthenticateTokenUseCase;
    use ferrobox_application::change_password::ChangePasswordUseCase;
    use ferrobox_application::create_repository::{CreateRepositoryKind, CreateRepositoryUseCase};
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
    use ferrobox_application::manage_users::{
        ChangeUserRoleUseCase, CreateUserUseCase, DeleteUserUseCase, ListUsersUseCase,
    };
    use ferrobox_application::packaging::PackagingRegistry;
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::retention::RetentionService;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryHttpClient,
        InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryRetentionStore, InMemoryStorage,
        InMemoryUserStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::RepositoryName;
    use ferrobox_domain::user::{Role, Username};
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[allow(clippy::too_many_lines)]
    async fn fixture() -> (Router, String, String, String) {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let assay_store = Arc::new(InMemoryAssayStore::default());
        let retention_store = Arc::new(InMemoryRetentionStore::default());
        let http_client = Arc::new(InMemoryHttpClient::default());

        let state = Arc::new(AppState {
            create_repository: CreateRepositoryUseCase::new(repository_store.clone()),
            list_repositories: ListRepositoriesUseCase::new(repository_store.clone()),
            get_repository: GetRepositoryUseCase::new(repository_store.clone()),
            update_alloy_members: UpdateAlloyMembersUseCase::new(repository_store.clone()),
            publish_artifact: PublishArtifactUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                storage.clone(),
            ),
            download_artifact: DownloadArtifactUseCase::new(artifact_store.clone(), storage.clone()),
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
            packaging: PackagingRegistry::new(),
            assays: ferrobox_application::assay::AssayService::new(
                assay_store.clone(),
                package_index_store.clone(),
                repository_store.clone(),
                storage.clone(),
                http_client,
            ),
            retention: RetentionService::new(
                repository_store.clone(),
                artifact_store,
                package_index_store,
                storage,
                assay_store,
                retention_store,
            ),
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
        });

        let developer = state
            .create_user
            .execute(
                Username::parse("developer").unwrap(),
                "secret",
                Role::Developer,
            )
            .await
            .unwrap();
        let token = state
            .create_api_token
            .execute(developer.id(), ApiTokenName::parse("dev").unwrap())
            .await
            .unwrap()
            .plaintext_secret;
        let reader = state
            .create_user
            .execute(Username::parse("reader").unwrap(), "secret", Role::Reader)
            .await
            .unwrap();
        let reader_token = state
            .create_api_token
            .execute(reader.id(), ApiTokenName::parse("read").unwrap())
            .await
            .unwrap()
            .plaintext_secret;
        let repo = state
            .create_repository
            .execute(
                RepositoryName::parse("crates-releases").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        (
            crate::build_router(state),
            token,
            reader_token,
            repo.to_string(),
        )
    }

    #[tokio::test]
    async fn get_policy_defaults_to_keep_all() {
        let (app, token, _, repo) = fixture().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/repositories/{repo}/retention"))
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["keep_last"], Value::Null);
        assert_eq!(json["keep_days"], Value::Null);
    }

    #[tokio::test]
    async fn save_and_apply_require_write_role() {
        let (app, token, reader_token, repo) = fixture().await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/repositories/{repo}/retention"))
                    .header("Authorization", format!("Bearer {reader_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"keep_last":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/repositories/{repo}/retention/dry-run"))
                    .header("Authorization", format!("Bearer {reader_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"keep_last":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/repositories/{repo}/retention"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"keep_last":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/repositories/{repo}/retention/dry-run"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"keep_last":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["dry_run"], true);
        assert_eq!(json["dropped_versions"], 0);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/repositories/{repo}/retention/apply"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"keep_last":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["dry_run"], false);
        assert_eq!(json["dropped_versions"], 0);
        assert_eq!(json["deleted_artifacts"], 0);
    }

    #[tokio::test]
    async fn instance_gc_dry_run_requires_write_role() {
        let (app, token, reader_token, _) = fixture().await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gc/dry-run")
                    .header("Authorization", format!("Bearer {reader_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gc/dry-run")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["dry_run"], true);
        assert_eq!(json["deleted_artifacts"], 0);
    }
}
