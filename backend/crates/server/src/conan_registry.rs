//! HTTP routes for the Conan v2 protocol (`conan upload` / `conan install`).
//!
//! The remote is mounted at `/conan/<UUID>/`. `GET /v1/ping` declares the
//! `revisions` and `complex_search` capabilities. Reads are public;
//! writes require an API token and a write role.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::{AuthenticatedUser, extract_bearer_token};
use crate::authz::{require_public_repo_read, require_repo_write};
use crate::error::ApiError;

const CONAN_UPLOAD_LIMIT: usize = 512 * 1024 * 1024;
static CONAN_CAPABILITIES: HeaderName = HeaderName::from_static("x-conan-server-capabilities");

/// Read-only routes (ping, auth, recipes, binaries, search).
pub(crate) fn public_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/conan/{repository_id}/v1/ping", get(ping))
        .route(
            "/conan/{repository_id}/v1/users/authenticate",
            get(authenticate),
        )
        .route(
            "/conan/{repository_id}/v2/users/authenticate",
            get(authenticate),
        )
        .route(
            "/conan/{repository_id}/v1/users/check_credentials",
            get(check_credentials),
        )
        .route(
            "/conan/{repository_id}/v2/users/check_credentials",
            get(check_credentials),
        )
        .route(
            "/conan/{repository_id}/v2/conans/{*rest}",
            get(conan_get).head(conan_head),
        )
}

/// Write routes (file `PUT` and yank).
pub(crate) fn write_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/conan/{repository_id}/v2/conans/{*rest}", put(conan_put))
        .route(
            "/conan/{repository_id}/recipes/{name}/{version}/yank",
            delete(yank),
        )
        .route(
            "/conan/{repository_id}/recipes/{name}/{version}/unyank",
            put(unyank),
        )
        .layer(DefaultBodyLimit::max(CONAN_UPLOAD_LIMIT))
}

fn conan_strategy(state: &AppState) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    state
        .packaging
        .strategy_for(PackageEcosystem::Conan)
        .ok_or_else(|| ApiError::Internal("no conan packaging strategy is registered".to_string()))
}

async fn load_conan_repository(
    state: &AppState,
    repository_id: Uuid,
) -> Result<ferrobox_domain::repository::Repository, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    if repository.ecosystem() != PackageEcosystem::Conan {
        return Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'conan'",
            repository.ecosystem().label()
        )));
    }
    Ok(repository)
}

fn json_response(body: Bytes) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (StatusCode::OK, headers, body)
}

fn bytes_response(body: Bytes) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    (StatusCode::OK, headers, body)
}

async fn ping() -> (StatusCode, HeaderMap, &'static str) {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONAN_CAPABILITIES.clone(),
        HeaderValue::from_static("revisions,complex_search"),
    );
    (StatusCode::OK, headers, "")
}

async fn authenticate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, String), ApiError> {
    let Some(secret) = extract_bearer_token(&headers) else {
        return Err(ApiError::Unauthorized(
            "missing Authorization credentials".to_string(),
        ));
    };
    state.authenticate_token.execute(&secret).await?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    Ok((StatusCode::OK, response_headers, secret))
}

async fn check_credentials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, String), ApiError> {
    authenticate(State(state), headers).await
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
}

#[derive(Serialize)]
struct SearchResults {
    results: Vec<String>,
}

#[derive(Serialize)]
struct ConanOk {
    ok: bool,
}

async fn conan_head(
    State(state): State<Arc<AppState>>,
    Path((repository_id, rest)): Path<(Uuid, String)>,
    Query(search): Query<SearchParams>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let (status, headers, _) = conan_get(
        State(state),
        Path((repository_id, rest)),
        Query(search),
        headers,
    )
    .await?;
    Ok((status, headers, Bytes::new()))
}

async fn conan_get(
    State(state): State<Arc<AppState>>,
    Path((repository_id, rest)): Path<(Uuid, String)>,
    Query(search): Query<SearchParams>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let repository = load_conan_repository(&state, repository_id).await?;
    let strategy = conan_strategy(&state)?;
    let rest = rest.trim_matches('/');

    if rest == "search" {
        let hits = strategy.search(&repository, &search.q, 100).await?;
        let body = serde_json::to_vec(&SearchResults {
            results: hits.into_iter().map(|hit| hit.name).collect(),
        })
        .map_err(|err| ApiError::Internal(err.to_string()))?;
        return Ok(json_response(Bytes::from(body)));
    }

    if rest.contains("/files/") && !rest.ends_with("/files") && !rest.ends_with("/files/") {
        crate::admission::enforce_download(
            &state,
            repository.id(),
            ferrobox_application::packaging::conan::admission_download_target(rest),
        )
        .await?;
        let body = strategy.get_protocol_file(&repository, rest).await?;
        return Ok(bytes_response(body));
    }

    let body = strategy.protocol_metadata(&repository, rest).await?;
    Ok(json_response(body))
}

async fn conan_put(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, rest)): Path<(Uuid, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    require_repo_write(&state.groups, &user, &token, RepositoryId::from(repository_id)).await?;
    let repository = load_conan_repository(&state, repository_id).await?;
    let strategy = conan_strategy(&state)?;
    strategy
        .put_protocol_file(&repository, rest.trim_matches('/'), body)
        .await?;
    Ok(StatusCode::OK)
}

async fn yank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<ConanOk>), ApiError> {
    require_repo_write(&state.groups, &user, &token, RepositoryId::from(repository_id)).await?;
    set_yanked(&state, repository_id, &name, &version, true).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::PackageYanked,
        AuditTargetKind::Package,
        name,
        version,
    )
    .await;
    Ok((StatusCode::OK, Json(ConanOk { ok: true })))
}

async fn unyank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<ConanOk>), ApiError> {
    require_repo_write(&state.groups, &user, &token, RepositoryId::from(repository_id)).await?;
    set_yanked(&state, repository_id, &name, &version, false).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::PackageUnyanked,
        AuditTargetKind::Package,
        name,
        version,
    )
    .await;
    Ok((StatusCode::OK, Json(ConanOk { ok: true })))
}

async fn set_yanked(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    version: &str,
    yanked: bool,
) -> Result<(), ApiError> {
    state
        .worm
        .ensure_mutable(RepositoryId::from(repository_id))
        .await?;
    let repository = load_conan_repository(state, repository_id).await?;
    let strategy = conan_strategy(state)?;
    let name = PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version =
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(PackageEcosystem::Conan, name, version);
    strategy
        .set_yanked(&repository, &coordinate, yanked)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{HeaderMap, Request, StatusCode};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
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
    use ferrobox_application::packaging::cargo::CargoPackagingStrategy;
    use ferrobox_application::packaging::conan::ConanPackagingStrategy;
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
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
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::AppState;

    struct Fixture {
        app: Router,
        repo_id: Uuid,
        developer_token: String,
    }

    #[allow(clippy::too_many_lines)]
    async fn fixture() -> Fixture {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let http_client = Arc::new(InMemoryHttpClient::default());

        let packaging = PackagingRegistry::new()
            .register(Arc::new(CargoPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
            )))
            .register(Arc::new(ConanPackagingStrategy::new(
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

        let developer = state
            .create_user
            .seed("developer", Role::Developer)
            .await
            .unwrap();
        let developer_token = state
            .create_api_token
            .execute(developer.id(), ApiTokenName::parse("dev").unwrap(), None)
            .await
            .unwrap()
            .plaintext_secret;

        let repo_id = state
            .create_repository
            .execute(
                RepositoryName::parse("conan-releases").unwrap(),
                PackageEcosystem::Conan,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        Fixture {
            app: crate::build_router(state),
            repo_id: repo_id.into(),
            developer_token,
        }
    }

    async fn send(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, body)
    }

    #[tokio::test]
    async fn ping_advertises_revision_capabilities() {
        let fx = fixture().await;
        let (status, headers, _) = send(
            fx.app,
            Request::builder()
                .uri(format!("/conan/{}/v1/ping", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get("x-conan-server-capabilities").unwrap(),
            "revisions,complex_search"
        );
    }

    #[tokio::test]
    async fn authenticate_exchanges_basic_for_the_api_token() {
        let fx = fixture().await;
        let basic = BASE64.encode(format!("__token__:{}", fx.developer_token));
        let (status, _, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/conan/{}/v2/users/authenticate", fx.repo_id))
                .header("Authorization", format!("Basic {basic}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), fx.developer_token.as_bytes());
    }

    #[tokio::test]
    async fn upload_without_a_token_is_unauthorized() {
        let fx = fixture().await;
        let (status, _, _) = send(
            fx.app,
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/conan/{}/v2/conans/hello/0.1/_/_/revisions/rrev/files/conanfile.py",
                    fx.repo_id
                ))
                .body(Body::from("from conan import ConanFile\n"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn upload_then_latest_search_and_download() {
        let fx = fixture().await;
        let token = format!("Bearer {}", fx.developer_token);
        let recipe = "/v2/conans/hello/0.1/_/_/revisions/rrev1/files/conanfile.py";
        let (status, _, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/conan/{}{recipe}", fx.repo_id))
                .header("Authorization", token.clone())
                .body(Body::from("from conan import ConanFile\n"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, headers, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!(
                    "/conan/{}/v2/conans/hello/0.1/_/_/latest",
                    fx.repo_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["revision"], "rrev1");

        let (status, _, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/conan/{}/v2/conans/search?q=hello", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["results"][0], "hello/0.1@_/_");

        let (status, _, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/conan/{}{recipe}", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"from conan import ConanFile\n");

        let (status, _, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("DELETE")
                .uri(format!("/conan/{}/recipes/hello/0.1@_:_/yank", fx.repo_id))
                .header("Authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _, _) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!(
                    "/conan/{}/v2/conans/hello/0.1/_/_/latest",
                    fx.repo_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/conan/{}{recipe}", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"from conan import ConanFile\n");
    }
}
