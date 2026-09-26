//! Rutas HTTP de réplica push o pull hacia otra instancia.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::RepositoryId;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_repo_read, require_repo_write};
use crate::dto::{ReplicaPolicyRequest, ReplicaPolicyResponse, ReplicaPushResponse};
use crate::error::ApiError;

pub(crate) async fn get_policy(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<ReplicaPolicyResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, repository_id).await?;
    let policy = state.replica.get_policy(repository_id).await?;
    Ok(Json(ReplicaPolicyResponse::from(policy)))
}

pub(crate) async fn save_policy(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<ReplicaPolicyRequest>,
) -> Result<Json<ReplicaPolicyResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let destination_id = payload
        .destination_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            Uuid::parse_str(value.trim())
                .map_err(|_| ApiError::BadRequest("destination_id must be a UUID".to_string()))
        })
        .transpose()?;
    let saved = state
        .replica
        .save_policy(
            repository_id,
            payload.remote_url,
            destination_id,
            payload.token,
            payload.direction,
        )
        .await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::ReplicaPolicyChanged,
        AuditTargetKind::Repository,
        repository_id.to_string(),
        saved
            .target()
            .map(|target| target.remote_url().as_str().to_string())
            .unwrap_or_default(),
    )
    .await;
    Ok(Json(ReplicaPolicyResponse::from(saved)))
}

pub(crate) async fn push_now(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<ReplicaPushResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let outcome = state.replica.push_now(repository_id).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::ReplicaPushed,
        AuditTargetKind::Repository,
        repository_id.to_string(),
        format!(
            "{} packages, {} artifacts, {} skipped",
            outcome.packages_imported, outcome.artifacts_imported, outcome.skipped
        ),
    )
    .await;
    Ok(Json(ReplicaPushResponse {
        packages_imported: outcome.packages_imported,
        artifacts_imported: outcome.artifacts_imported,
        skipped: outcome.skipped,
    }))
}

pub(crate) async fn pull_now(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<ReplicaPushResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let outcome = state.replica.pull_now(repository_id).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::ReplicaPulled,
        AuditTargetKind::Repository,
        repository_id.to_string(),
        format!(
            "{} packages, {} artifacts, {} skipped",
            outcome.packages_imported, outcome.artifacts_imported, outcome.skipped
        ),
    )
    .await;
    Ok(Json(ReplicaPushResponse {
        packages_imported: outcome.packages_imported,
        artifacts_imported: outcome.artifacts_imported,
        skipped: outcome.skipped,
    }))
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
    use ferrobox_application::manage_groups::GroupService;
    use ferrobox_application::manage_users::{
        ChangeUserRoleUseCase, CreateUserUseCase, DeleteUserUseCase, ListUsersUseCase,
        ResetUserPasswordUseCase,
    };
    use ferrobox_application::packaging::PackagingRegistry;
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::quota::QuotaService;
    use ferrobox_application::replica::ReplicaService;
    use ferrobox_application::retention::RetentionService;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryGroupStore,
        InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore, InMemoryReplicaStore,
        InMemoryRepositoryStore, InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore,
        InMemoryWebhookStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::RepositoryName;
    use ferrobox_domain::user::Role;
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
        let quota = QuotaService::new(
            repository_store.clone(),
            artifact_store.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        let bundles = ferrobox_application::repository_bundle::RepositoryBundleService::new(
            repository_store.clone(),
            artifact_store.clone(),
            package_index_store.clone(),
            storage.clone(),
            quota.clone(),
        );
        let search_packages = ferrobox_application::search_packages::SearchPackagesUseCase::new(
            repository_store.clone(),
            package_index_store.clone(),
        );

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
            promote_package: ferrobox_application::promote_package::PromotePackageUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                storage.clone(),
                quota.clone(),
            ),
            repository_bundle: bundles.clone(),
            replica: ReplicaService::new(
                Arc::new(InMemoryReplicaStore::default()),
                repository_store.clone(),
                bundles,
                http_client.clone(),
            ),
            packaging: PackagingRegistry::new(),
            assays: ferrobox_application::assay::AssayService::new(
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
                http_client,
                repository_store,
            ),
            audit: ferrobox_application::audit::AuditService::new(Arc::new(
                ferrobox_application::test_support::InMemoryAuditStore::default(),
            )),
            oidc: None,
        });

        let developer = state
            .create_user
            .seed("developer", Role::Developer)
            .await
            .unwrap();
        let token = state
            .create_api_token
            .execute(developer.id(), ApiTokenName::parse("dev").unwrap(), None)
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
    async fn get_policy_defaults_to_unconfigured() {
        let (app, token, _, repo) = fixture().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/repositories/{repo}/replica"))
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
        assert_eq!(json["configured"], false);
        assert_eq!(json["has_token"], false);
        assert_eq!(json["direction"], "push");
    }

    #[tokio::test]
    async fn save_requires_write_role() {
        let (app, token, reader_token, repo) = fixture().await;
        let dest = uuid::Uuid::now_v7();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/repositories/{repo}/replica"))
                    .header("Authorization", format!("Bearer {reader_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"remote_url":"http://peer.example","destination_id":"{dest}","token":"t"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/repositories/{repo}/replica"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"remote_url":"http://peer.example","destination_id":"{dest}","token":"t"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["configured"], true);
        assert_eq!(json["has_token"], true);
        assert_eq!(json["remote_url"], "http://peer.example/");
        assert_eq!(json["direction"], "push");
    }

    #[tokio::test]
    async fn save_stores_pull_direction() {
        let (app, token, _, repo) = fixture().await;
        let dest = uuid::Uuid::now_v7();
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/repositories/{repo}/replica"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"remote_url":"http://peer.example","destination_id":"{dest}","token":"t","direction":"pull"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["direction"], "pull");
    }
}
