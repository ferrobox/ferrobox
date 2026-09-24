//! Rutas HTTP de la política de admisión.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use ferrobox_domain::admission::AdmissionPolicy;
use ferrobox_domain::ids::RepositoryId;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_repo_read, require_repo_write};
use crate::dto::{
    AdmissionEventResponse, AdmissionPolicyRequest, AdmissionPolicyResponse,
    AdmissionPreviewResponse,
};
use crate::error::ApiError;

pub(crate) async fn get_policy(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<AdmissionPolicyResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, repository_id).await?;
    let policy = state.admission.get_policy(repository_id).await?;
    Ok(Json(AdmissionPolicyResponse::from(policy)))
}

pub(crate) async fn save_policy(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<AdmissionPolicyRequest>,
) -> Result<Json<AdmissionPolicyResponse>, ApiError> {
    require_repo_write(&state.groups, &user, RepositoryId::from(repository_id)).await?;
    let policy = AdmissionPolicy::parse(
        payload.enabled,
        payload.when.as_str(),
        payload.predicate.as_str(),
        payload.effect.as_str(),
    )?;
    let saved = state
        .admission
        .save_policy(RepositoryId::from(repository_id), policy)
        .await?;
    Ok(Json(AdmissionPolicyResponse::from(saved)))
}

pub(crate) async fn dry_run(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<AdmissionPolicyRequest>,
) -> Result<Json<AdmissionPreviewResponse>, ApiError> {
    require_repo_write(&state.groups, &user, RepositoryId::from(repository_id)).await?;
    let policy = AdmissionPolicy::parse(
        payload.enabled,
        payload.when.as_str(),
        payload.predicate.as_str(),
        payload.effect.as_str(),
    )?;
    let preview = state
        .admission
        .dry_run(RepositoryId::from(repository_id), policy)
        .await?;
    Ok(Json(AdmissionPreviewResponse::from(preview)))
}

pub(crate) async fn list_events(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<Vec<AdmissionEventResponse>>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, repository_id).await?;
    let events = state.admission.list_events(repository_id).await?;
    Ok(Json(
        events
            .into_iter()
            .map(AdmissionEventResponse::from)
            .collect(),
    ))
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
    use ferrobox_application::retention::RetentionService;
    use ferrobox_application::test_support::{
        InMemoryAdmissionStore, InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore,
        InMemoryGroupStore, InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore,
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

    struct Fixture {
        app: Router,
        token: String,
        reader_token: String,
        oci: String,
        cargo: String,
        alloy: String,
    }

    #[allow(clippy::too_many_lines)]
    async fn fixture() -> Fixture {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let assay_store = Arc::new(InMemoryAssayStore::default());
        let retention_store = Arc::new(InMemoryRetentionStore::default());
        let http_client = Arc::new(InMemoryHttpClient::default());

        let quota = ferrobox_application::quota::QuotaService::new(
            repository_store.clone(),
            artifact_store.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );

        let search_packages = ferrobox_application::search_packages::SearchPackagesUseCase::new(
            repository_store.clone(),
            package_index_store.clone(),
        );

        let list_repository_artifacts = ListRepositoryArtifactsUseCase::new(
            repository_store.clone(),
            artifact_store.clone(),
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
            list_repository_artifacts: list_repository_artifacts.clone(),
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
            packaging: PackagingRegistry::new(),
            assays: ferrobox_application::assay::AssayService::new(
                assay_store.clone(),
                package_index_store.clone(),
                repository_store.clone(),
                storage.clone(),
                http_client.clone(),
            ),
            admission: ferrobox_application::admission::AdmissionService::new(
                Arc::new(InMemoryAdmissionStore::default()),
                repository_store.clone(),
                list_repository_artifacts,
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
                http_client.clone(),
                repository_store.clone(),
            ),
        });

        let developer = state
            .create_user
            .seed("developer", Role::Developer)
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
            .seed("reader", Role::Reader)
            .await
            .unwrap();
        let reader_token = state
            .create_api_token
            .execute(reader.id(), ApiTokenName::parse("read").unwrap())
            .await
            .unwrap()
            .plaintext_secret;
        let oci = state
            .create_repository
            .execute(
                RepositoryName::parse("oci-local").unwrap(),
                PackageEcosystem::Oci,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let cargo = state
            .create_repository
            .execute(
                RepositoryName::parse("crates-local").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let alloy = state
            .create_repository
            .execute(
                RepositoryName::parse("oci-all").unwrap(),
                PackageEcosystem::Oci,
                CreateRepositoryKind::Alloy { members: vec![oci] },
            )
            .await
            .unwrap();

        Fixture {
            app: crate::build_router(state),
            token,
            reader_token,
            oci: oci.to_string(),
            cargo: cargo.to_string(),
            alloy: alloy.to_string(),
        }
    }

    #[tokio::test]
    async fn get_policy_defaults_to_inactive() {
        let fixture = fixture().await;
        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .uri(format!("/repositories/{}/admission", fixture.oci))
                    .header("Authorization", format!("Bearer {}", fixture.token))
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
        assert_eq!(json["enabled"], false);
        assert_eq!(json["when"], "pull");
        assert_eq!(json["predicate"], "not_signed");
        assert_eq!(json["effect"], "deny");
    }

    #[tokio::test]
    async fn save_and_dry_run_require_write_role() {
        let fixture = fixture().await;
        let body = r#"{"enabled":true,"when":"pull","predicate":"not_signed","effect":"warn"}"#;

        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/repositories/{}/admission", fixture.oci))
                    .header("Authorization", format!("Bearer {}", fixture.reader_token))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/repositories/{}/admission/dry-run", fixture.oci))
                    .header("Authorization", format!("Bearer {}", fixture.reader_token))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/repositories/{}/admission", fixture.oci))
                    .header("Authorization", format!("Bearer {}", fixture.token))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&saved).unwrap();
        assert_eq!(json["enabled"], true);
        assert_eq!(json["effect"], "warn");

        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/repositories/{}/admission/dry-run", fixture.oci))
                    .header("Authorization", format!("Bearer {}", fixture.token))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let preview = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&preview).unwrap();
        assert_eq!(json["matches"], Value::Array(vec![]));
        assert_eq!(json["allowed"], 0);
    }

    #[tokio::test]
    async fn list_events_starts_empty() {
        let fixture = fixture().await;
        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .uri(format!("/repositories/{}/admission/events", fixture.oci))
                    .header("Authorization", format!("Bearer {}", fixture.token))
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
        assert_eq!(json, Value::Array(vec![]));
    }

    #[tokio::test]
    async fn rejects_alloy_and_non_oci_targets() {
        let fixture = fixture().await;

        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/repositories/{}/admission", fixture.alloy))
                    .header("Authorization", format!("Bearer {}", fixture.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .uri(format!("/repositories/{}/admission", fixture.cargo))
                    .header("Authorization", format!("Bearer {}", fixture.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
