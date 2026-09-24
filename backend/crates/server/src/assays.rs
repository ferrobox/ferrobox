//! Rutas HTTP del ensaye (`Assay`) de paquetes.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::Response;
use ferrobox_application::assay::{AssayError, to_cyclonedx};
use ferrobox_domain::ids::{AssayId, RepositoryId};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_repo_read, require_repo_write, require_write_artifacts};
use crate::dto::{AssayLookupRequest, AssayResponse, AssayRerunResponse};
use crate::error::ApiError;

pub(crate) async fn list_all(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<Vec<AssayResponse>>, ApiError> {
    let visibility = state.groups.visibility(&user).await?;
    let assays = state.assays.list_all().await?;
    Ok(Json(
        assays
            .iter()
            .filter(|assay| visibility.contains(assay.repository_id()))
            .map(AssayResponse::from)
            .collect(),
    ))
}

pub(crate) async fn rerun_all(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<AssayRerunResponse>, ApiError> {
    require_write_artifacts(&user)?;
    let scheduled = state.assays.rerun_all().await?;
    Ok(Json(AssayRerunResponse { scheduled }))
}

pub(crate) async fn list_for_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<Vec<AssayResponse>>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, repository_id).await?;
    let assays = state.assays.list_for_repository(repository_id).await?;
    Ok(Json(assays.iter().map(AssayResponse::from).collect()))
}

pub(crate) async fn get_or_run(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Query(lookup): Query<AssayLookupRequest>,
) -> Result<Json<AssayResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, repository_id).await?;
    let assay = state
        .assays
        .get_or_run(
            repository_id,
            lookup.ecosystem.into(),
            &lookup.name,
            &lookup.version,
        )
        .await?;
    Ok(Json(AssayResponse::from(&assay)))
}

pub(crate) async fn run(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(lookup): Json<AssayLookupRequest>,
) -> Result<Json<AssayResponse>, ApiError> {
    require_repo_write(&state.groups, &user, RepositoryId::from(repository_id)).await?;
    let assay = state
        .assays
        .run(
            RepositoryId::from(repository_id),
            lookup.ecosystem.into(),
            &lookup.name,
            &lookup.version,
        )
        .await?;
    Ok(Json(AssayResponse::from(&assay)))
}

pub(crate) async fn get_by_id(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(assay_id): Path<Uuid>,
) -> Result<Json<AssayResponse>, ApiError> {
    let assay = state.assays.get_by_id(AssayId::from(assay_id)).await?;
    require_repo_read(&state.groups, &user, assay.repository_id()).await?;
    Ok(Json(AssayResponse::from(&assay)))
}

pub(crate) async fn download_sbom(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(assay_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let assay = state.assays.get_by_id(AssayId::from(assay_id)).await?;
    require_repo_read(&state.groups, &user, assay.repository_id()).await?;
    let body = to_cyclonedx(&assay);
    let filename = format!(
        "{}-{}.cdx.json",
        assay
            .coordinate()
            .name()
            .as_str()
            .replace(['/', '\\'], "_"),
        assay.coordinate().version().as_str()
    );
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.cyclonedx+json"),
    );
    let disposition = format!("attachment; filename=\"{filename}\"");
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

impl From<AssayError> for ApiError {
    fn from(err: AssayError) -> Self {
        match err {
            AssayError::RepositoryNotFound(_)
            | AssayError::AssayNotFound(_)
            | AssayError::PackageNotFound(_) => Self::NotFound(err.to_string()),
            AssayError::InvalidCoordinate(_) => Self::BadRequest(err.to_string()),
            AssayError::Persistence(_)
            | AssayError::Index(_)
            | AssayError::Repositories(_)
            | AssayError::Storage(_) => Self::Internal(err.to_string()),
        }
    }
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
    use ferrobox_application::packaging::cargo::CargoPackagingStrategy;
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryGroupStore, InMemoryHttpClient, InMemoryWebhookStore,
        InMemoryPackageIndexStore, InMemoryQuotaStore, InMemoryRepositoryStore,
        InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore,
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
    async fn fixture() -> (Router, String, String) {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let http_client = Arc::new(InMemoryHttpClient::default());

        let packaging = PackagingRegistry::new().register(Arc::new(CargoPackagingStrategy::new(
            artifact_store.clone(),
            package_index_store.clone(),
            storage.clone(),
            http_client.clone(),
            repository_store.clone(),
        )));

        let quota = ferrobox_application::quota::QuotaService::new(
            repository_store.clone(),
            artifact_store.clone(),
            Arc::new(InMemoryQuotaStore::default()),
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
            promote_package: ferrobox_application::promote_package::PromotePackageUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                storage.clone(),
                quota.clone(),
            ),
            packaging,
            assays: ferrobox_application::assay::AssayService::new(
                Arc::new(InMemoryAssayStore::default()),
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
            retention: ferrobox_application::retention::RetentionService::new(
                repository_store.clone(),
                artifact_store,
                package_index_store,
                storage,
                Arc::new(InMemoryAssayStore::default()),
                Arc::new(InMemoryRetentionStore::default()),
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
        let _repo = state
            .create_repository
            .execute(
                RepositoryName::parse("crates-releases").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        (crate::build_router(state), token, reader_token)
    }

    #[tokio::test]
    async fn list_assays_requires_auth_and_starts_empty() {
        let (app, token, _) = fixture().await;

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/assays").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/assays")
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
        assert_eq!(json.as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn rerun_all_requires_write_role_and_schedules_nothing_when_empty() {
        let (app, token, reader_token) = fixture().await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/assays/rerun")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/assays/rerun")
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
                    .uri("/assays/rerun")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
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
        assert_eq!(json["scheduled"], 0);
    }
}
