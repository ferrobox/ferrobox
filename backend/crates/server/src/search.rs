//! Búsqueda global de paquetes en el catálogo.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use ferrobox_application::search_packages::default_search_limit;
use serde::Deserialize;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::dto::{PackageSearchHitResponse, SearchResponse};
use crate::error::ApiError;

#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    default_search_limit()
}

pub(crate) async fn search_packages(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    let visibility = state.groups.visibility(&user).await?;
    let hits = state.search_packages.execute(&query.q, query.limit).await?;
    Ok(Json(SearchResponse {
        hits: hits
            .into_iter()
            .filter(|hit| visibility.contains(hit.repository_id))
            .map(PackageSearchHitResponse::from)
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use crate::AppState;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use bytes::Bytes;
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
    use ferrobox_application::retention::RetentionService;
    use ferrobox_application::search_packages::SearchPackagesUseCase;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryGroupStore,
        InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore, InMemoryRepositoryStore,
        InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore, InMemoryWebhookStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::RepositoryName;
    use ferrobox_domain::user::Role;
    use ferrobox_ports::package_index_store::PackageIndexStore;
    use serde_json::Value;
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
        let repo = state
            .create_repository
            .execute(
                RepositoryName::parse("crates-releases").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        package_index_store
            .upsert_entry(
                repo,
                &PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("serde").unwrap(),
                    PackageVersion::parse("1.0.210").unwrap(),
                ),
                None,
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        (crate::build_router(state), token, reader_token)
    }

    #[tokio::test]
    async fn search_requires_auth_and_is_readable_by_a_reader() {
        let (app, _token, reader_token) = fixture().await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/search?q=serde")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/search?q=ser")
                    .header("Authorization", format!("Bearer {reader_token}"))
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
        assert_eq!(json["hits"][0]["name"], "serde");
        assert_eq!(json["hits"][0]["version"], "1.0.210");
        assert_eq!(json["hits"][0]["repository_name"], "crates-releases");
    }

    #[tokio::test]
    async fn empty_query_returns_no_hits() {
        let (app, token, _) = fixture().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/search")
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
        assert_eq!(json["hits"], Value::Array(vec![]));
    }
}
