//! Rutas HTTP del registro de auditoría (solo rol `Admin`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::user::User;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_manage_users;
use crate::dto::AuditEventResponse;
use crate::error::ApiError;

/// Deja constancia de una escritura. No falla la petición.
pub(crate) async fn record(
    state: &AppState,
    actor: &User,
    action: AuditAction,
    target_kind: AuditTargetKind,
    target: impl Into<String>,
    detail: impl Into<String>,
) {
    state
        .audit
        .record(actor, action, target_kind, target, detail)
        .await;
}

pub(crate) async fn list_events(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<Vec<AuditEventResponse>>, ApiError> {
    require_manage_users(&user)?;
    let events = state.audit.list().await?;
    Ok(Json(
        events.into_iter().map(AuditEventResponse::from).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use crate::AppState;
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
        InMemoryAdmissionStore, InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore,
        InMemoryAuditStore, InMemoryGroupStore, InMemoryHttpClient, InMemoryPackageIndexStore,
        InMemoryQuotaStore, InMemoryReplicaStore, InMemoryRepositoryStore, InMemoryRetentionStore,
        InMemoryStorage, InMemoryUserStore, InMemoryWebhookStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::user::Role;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    #[allow(clippy::too_many_lines)]
    async fn fixture() -> (Router, String, String) {
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
                Arc::new(InMemoryAdmissionStore::default()),
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
                http_client.clone(),
                repository_store.clone(),
            ),
            audit: ferrobox_application::audit::AuditService::new(Arc::new(
                InMemoryAuditStore::default(),
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

        (crate::build_router(state), admin_token, reader_token)
    }

    async fn json_body(response: axum::http::Response<Body>) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn reader_cannot_list_audit() {
        let (app, _admin, reader_token) = fixture().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/audit")
                    .header("Authorization", format!("Bearer {reader_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn creating_a_user_is_recorded() {
        let (app, admin_token, _reader) = fixture().await;

        let created = app
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

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/audit")
                    .header("Authorization", format!("Bearer {admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let events = body.as_array().expect("audit list");
        assert!(
            events.iter().any(|event| {
                event["action"] == "user.created"
                    && event["actor"] == "admin"
                    && event["target"] == "dev"
            }),
            "expected user.created for dev, got {body}"
        );
    }
}
