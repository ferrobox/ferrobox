//! Rutas HTTP de avisos (`webhooks`) por repositorio.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::{RepositoryId, WebhookId};
use ferrobox_domain::webhook::WebhookEvent;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_repo_write;
use crate::dto::{
    CreateWebhookRequest, UpdateWebhookRequest, WebhookDeliveryResponse, WebhookResponse,
};
use crate::error::ApiError;

pub(crate) async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<Vec<WebhookResponse>>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let webhooks = state.webhooks.list(repository_id).await?;
    Ok(Json(webhooks.iter().map(WebhookResponse::from).collect()))
}

pub(crate) async fn create_webhook(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<WebhookResponse>), ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let events: Vec<WebhookEvent> = payload.events.into_iter().map(WebhookEvent::from).collect();
    let webhook = state
        .webhooks
        .create(
            repository_id,
            payload.name,
            payload.url,
            payload.secret,
            events,
            payload.enabled,
        )
        .await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::WebhookCreated,
        AuditTargetKind::Webhook,
        webhook.name().to_string(),
        repository_id.to_string(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(WebhookResponse::from(&webhook))))
}

pub(crate) async fn update_webhook(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, webhook_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdateWebhookRequest>,
) -> Result<Json<WebhookResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let events: Vec<WebhookEvent> = payload.events.into_iter().map(WebhookEvent::from).collect();
    let webhook = state
        .webhooks
        .update(
            repository_id,
            WebhookId::from(webhook_id),
            payload.name,
            payload.url,
            payload.secret,
            events,
            payload.enabled,
        )
        .await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::WebhookUpdated,
        AuditTargetKind::Webhook,
        webhook.name().to_string(),
        repository_id.to_string(),
    )
    .await;
    Ok(Json(WebhookResponse::from(&webhook)))
}

pub(crate) async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    state
        .webhooks
        .delete(repository_id, WebhookId::from(webhook_id))
        .await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::WebhookDeleted,
        AuditTargetKind::Webhook,
        webhook_id.to_string(),
        repository_id.to_string(),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_deliveries(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<WebhookDeliveryResponse>>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let deliveries = state
        .webhooks
        .deliveries(repository_id, WebhookId::from(webhook_id))
        .await?;
    Ok(Json(
        deliveries
            .iter()
            .map(WebhookDeliveryResponse::from)
            .collect(),
    ))
}

pub(crate) async fn ping_webhook(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<WebhookDeliveryResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;
    let delivery = state
        .webhooks
        .ping(repository_id, WebhookId::from(webhook_id))
        .await?;
    Ok(Json(WebhookDeliveryResponse::from(&delivery)))
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
        InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore, InMemoryRepositoryStore,
        InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore, InMemoryWebhookStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_application::webhooks::WebhookService;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::user::Role;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::AppState;

    struct Fixture {
        app: Router,
        writer_token: String,
        reader_token: String,
        repo_id: String,
        http: Arc<InMemoryHttpClient>,
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
        let quota = QuotaService::new(
            repository_store.clone(),
            artifact_store.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        let webhooks = WebhookService::new(
            Arc::new(InMemoryWebhookStore::default()),
            http_client.clone(),
            repository_store.clone(),
        );
        let assays = AssayService::new(
            assay_store.clone(),
            package_index_store.clone(),
            repository_store.clone(),
            storage.clone(),
            http_client.clone(),
        )
        .with_webhooks(webhooks.clone());

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
            packaging: PackagingRegistry::new(),
            assays,
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
                package_index_store.clone(),
                storage,
                assay_store,
                retention_store,
            ),
            quota,
            search_packages: SearchPackagesUseCase::new(
                repository_store.clone(),
                package_index_store,
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
            reset_user_password: ResetUserPasswordUseCase::new(user_store.clone()),
            groups: GroupService::new(
                Arc::new(InMemoryGroupStore::default()),
                user_store,
                repository_store,
            ),
            webhooks,
            audit: ferrobox_application::audit::AuditService::new(Arc::new(
                ferrobox_application::test_support::InMemoryAuditStore::default(),
            )),
            oidc: None,
        });

        let writer = state
            .create_user
            .seed("developer", Role::Developer)
            .await
            .unwrap();
        let writer_token = state
            .create_api_token
            .execute(writer.id(), ApiTokenName::parse("dev").unwrap())
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
        let repo_id = state
            .create_repository
            .execute(
                ferrobox_domain::repository::RepositoryName::parse("npm-all").unwrap(),
                ferrobox_domain::package_coordinate::PackageEcosystem::Npm,
                ferrobox_application::create_repository::CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        Fixture {
            app: crate::build_router(state),
            writer_token,
            reader_token,
            repo_id: repo_id.to_string(),
            http: http_client,
        }
    }

    async fn send_json(
        app: Router,
        token: &str,
        method: &str,
        uri: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    #[tokio::test]
    async fn reader_cannot_manage_webhooks() {
        let fx = fixture().await;
        let (status, _) = send_json(
            fx.app,
            &fx.reader_token,
            "POST",
            &format!("/repositories/{}/webhooks", fx.repo_id),
            json!({
                "name": "ci",
                "url": "https://example.test/hook",
                "events": ["assay.completed"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn developer_creates_lists_pings_and_deletes_a_webhook() {
        let fx = fixture().await;
        fx.http.stub("https://example.test/hook", 204, "");

        let (status, created) = send_json(
            fx.app.clone(),
            &fx.writer_token,
            "POST",
            &format!("/repositories/{}/webhooks", fx.repo_id),
            json!({
                "name": "ci",
                "url": "https://example.test/hook",
                "secret": "s3cret",
                "events": ["assay.completed", "package.published"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created["has_secret"], true);
        assert!(created.get("secret").is_none());
        let webhook_id = created["id"].as_str().unwrap();

        let (status, ping) = send_json(
            fx.app.clone(),
            &fx.writer_token,
            "POST",
            &format!("/repositories/{}/webhooks/{webhook_id}/ping", fx.repo_id),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ping["event"], "ping");
        assert_eq!(ping["status"], "success");
        assert_eq!(ping["http_status"], 204);

        let posts = fx.http.take_posts();
        assert_eq!(posts.len(), 1);
        assert!(
            posts[0]
                .headers
                .iter()
                .any(|(name, value)| name == "x-ferrobox-signature" && value.starts_with("sha256="))
        );

        let response = fx
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/repositories/{}/webhooks/{webhook_id}",
                        fx.repo_id
                    ))
                    .header("Authorization", format!("Bearer {}", fx.writer_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
