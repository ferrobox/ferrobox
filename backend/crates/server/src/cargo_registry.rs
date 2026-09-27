//! HTTP routes that implement the subset of the Cargo registry protocol
//! that `cargo publish`, `cargo yank`, `cargo search`, `cargo add`,
//! and `cargo build` need to publish packages and resolve dependencies
//! against `FerroBox` using the sparse-index protocol.
//!
//! Reference: <https://doc.rust-lang.org/cargo/reference/registries.html>.
//!
//! Reads (`config.json`, index, download, and search) are public:
//! `cargo build` / `cargo add` do not send credentials unless
//! `auth-required` is `true`. Writes (`publish`, `yank`, `unyank`)
//! require `Authorization` (raw token, `Bearer`, or `Token`) and a
//! write role.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_application::packaging::cargo::cargo_index_shard_path;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_public_repo_read, require_repo_write};
use crate::error::ApiError;

/// Read-only Cargo protocol routes. They sit on the public router so
/// `cargo` can resolve dependencies without a token.
pub(crate) fn public_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/cargo/{repository_id}/config.json", get(config_json))
        .route("/cargo/{repository_id}/api/v1/crates", get(search))
        .route(
            "/cargo/{repository_id}/api/v1/crates/{name}/{version}/download",
            get(download),
        )
        .route("/cargo/{repository_id}/1/{name}", get(index_len1))
        .route("/cargo/{repository_id}/2/{name}", get(index_len2))
        .route("/cargo/{repository_id}/3/{prefix}/{name}", get(index_len3))
        .route(
            "/cargo/{repository_id}/{prefix}/{suffix}/{name}",
            get(index_len4),
        )
        .route("/cargo/{repository_id}/index/1/{name}", get(index_len1))
        .route("/cargo/{repository_id}/index/2/{name}", get(index_len2))
        .route(
            "/cargo/{repository_id}/index/3/{prefix}/{name}",
            get(index_len3),
        )
        .route(
            "/cargo/{repository_id}/index/{prefix}/{suffix}/{name}",
            get(index_len4),
        )
}

/// Write routes for the Cargo protocol. They sit behind
/// authentication and [`require_write_artifacts`].
pub(crate) fn write_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/cargo/{repository_id}/api/v1/crates/new", put(publish))
        .route(
            "/cargo/{repository_id}/api/v1/crates/{name}/{version}/yank",
            delete(yank),
        )
        .route(
            "/cargo/{repository_id}/api/v1/crates/{name}/{version}/unyank",
            put(unyank),
        )
}

fn cargo_strategy(state: &AppState) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    state
        .packaging
        .strategy_for(PackageEcosystem::Cargo)
        .ok_or_else(|| ApiError::Internal("no Cargo packaging strategy is registered".to_string()))
}

/// Response of `GET /cargo/{repository_id}/config.json`: tells `cargo`
/// where to download `.crate` content (`dl`) and where to send publish
/// requests (`api`). `auth-required` is `false` so the index and
/// downloads do not require a token.
#[derive(Serialize)]
struct RegistryConfig {
    dl: String,
    api: String,
    #[serde(rename = "auth-required")]
    auth_required: bool,
}

async fn config_json(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<RegistryConfig>, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    if repository.ecosystem() != PackageEcosystem::Cargo {
        return Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'cargo'",
            repository.ecosystem().label()
        )));
    }

    let auth_required = state
        .groups
        .is_restricted(RepositoryId::from(repository_id))
        .await?;
    let base = format!("{}/cargo/{repository_id}", state.public_base_url);
    Ok(Json(RegistryConfig {
        dl: format!("{base}/api/v1/crates"),
        api: base,
        auth_required,
    }))
}

async fn index_len1(
    State(state): State<Arc<AppState>>,
    Path((repository_id, name)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("1/{name}");
    serve_index(&state, &headers, repository_id, name, &expected).await
}

async fn index_len2(
    State(state): State<Arc<AppState>>,
    Path((repository_id, name)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("2/{name}");
    serve_index(&state, &headers, repository_id, name, &expected).await
}

async fn index_len3(
    State(state): State<Arc<AppState>>,
    Path((repository_id, prefix, name)): Path<(Uuid, String, String)>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("3/{prefix}/{name}");
    serve_index(&state, &headers, repository_id, name, &expected).await
}

async fn index_len4(
    State(state): State<Arc<AppState>>,
    Path((repository_id, prefix, suffix, name)): Path<(Uuid, String, String, String)>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("{prefix}/{suffix}/{name}");
    serve_index(&state, &headers, repository_id, name, &expected).await
}

/// Serves the sparse index of a crate. `expected_shard` is the
/// sharding path `cargo` should have requested for `name`; if it does
/// not match, a 404 is returned so packages are not leaked through
/// incorrect paths.
async fn serve_index(
    state: &AppState,
    headers: &HeaderMap,
    repository_id: Uuid,
    package_name: String,
    expected_shard: &str,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let name =
        PackageName::parse(package_name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    if cargo_index_shard_path(&name) != expected_shard {
        return Err(ApiError::NotFound(format!(
            "sparse-index path '{expected_shard}' does not match crate '{name}'"
        )));
    }

    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        headers,
        RepositoryId::from(repository_id),
    )
    .await?;

    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(state)?;
    let body = strategy.index(&repository, &name).await?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    Ok((headers, body))
}

/// Response body that `cargo publish` expects after a successful
/// publish. `cargo` does not fail if these fields are empty, but it
/// does expect the object to exist.
#[derive(Serialize, Default)]
struct PublishWarnings {
    #[serde(rename = "invalid_categories")]
    invalid_categories: Vec<String>,
    #[serde(rename = "invalid_badges")]
    invalid_badges: Vec<String>,
    other: Vec<String>,
}

#[derive(Serialize, Default)]
struct PublishResponse {
    warnings: PublishWarnings,
}

#[derive(Serialize)]
struct YankResponse {
    ok: bool,
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    #[serde(default = "default_per_page")]
    per_page: u8,
}

fn default_per_page() -> u8 {
    10
}

#[derive(Serialize)]
struct SearchCrate {
    name: String,
    max_version: String,
}

#[derive(Serialize)]
struct SearchMeta {
    total: usize,
}

#[derive(Serialize)]
struct SearchResponse {
    crates: Vec<SearchCrate>,
    meta: SearchMeta,
}

/// `PUT /cargo/{repository_id}/api/v1/crates/new`: the endpoint that
/// `cargo publish` invokes. Accepts the token raw (`Authorization: fb_…`,
/// as `cargo` sends it) or with a `Bearer` / `Token` scheme.
async fn publish(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    body: Bytes,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
    require_repo_write(
        &state.groups,
        &user,
        &token,
        RepositoryId::from(repository_id),
    )
    .await?;

    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(&state)?;

    strategy.publish(&repository, body).await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::PackagePublished,
        AuditTargetKind::Package,
        repository.name().to_string(),
        "cargo",
    )
    .await;

    Ok((StatusCode::OK, Json(PublishResponse::default())))
}

/// `GET /cargo/{repository_id}/api/v1/crates/{name}/{version}/download`.
async fn download(
    State(state): State<Arc<AppState>>,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
    headers: HeaderMap,
) -> Result<Bytes, ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(&state)?;

    let name = PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version =
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(PackageEcosystem::Cargo, name, version);
    state
        .admission
        .enforce_package_pull(
            repository.id(),
            coordinate.name().as_str(),
            coordinate.version().as_str(),
        )
        .await?;

    let content = strategy.download(&repository, &coordinate).await?;
    Ok(content)
}

async fn yank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<Json<YankResponse>, ApiError> {
    set_yanked(&state, user, &token, repository_id, name, version, true).await
}

async fn unyank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<Json<YankResponse>, ApiError> {
    set_yanked(&state, user, &token, repository_id, name, version, false).await
}

async fn set_yanked(
    state: &AppState,
    user: ferrobox_domain::user::User,
    token: &ferrobox_domain::api_token::ApiToken,
    repository_id: Uuid,
    name: String,
    version: String,
    yanked: bool,
) -> Result<Json<YankResponse>, ApiError> {
    require_repo_write(
        &state.groups,
        &user,
        token,
        RepositoryId::from(repository_id),
    )
    .await?;
    state
        .worm
        .ensure_mutable(RepositoryId::from(repository_id))
        .await?;

    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(state)?;

    let name = PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version =
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(PackageEcosystem::Cargo, name, version);

    strategy
        .set_yanked(&repository, &coordinate, yanked)
        .await?;

    crate::audit::record(
        state,
        &user,
        if yanked {
            AuditAction::PackageYanked
        } else {
            AuditAction::PackageUnyanked
        },
        AuditTargetKind::Package,
        coordinate.name().as_str(),
        coordinate.version().as_str(),
    )
    .await;

    Ok(Json(YankResponse { ok: true }))
}

/// `GET /cargo/{repository_id}/api/v1/crates?q=…&per_page=…`.
/// An empty `q` lists indexed packages up to `per_page`.
async fn search(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Result<Json<SearchResponse>, ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let query = params.q.trim();

    let limit = usize::from(params.per_page.clamp(1, 100));
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(&state)?;
    let hits = strategy.search(&repository, query, limit).await?;
    let total = hits.len();

    Ok(Json(SearchResponse {
        crates: hits
            .into_iter()
            .map(|hit| SearchCrate {
                name: hit.name,
                max_version: hit.max_version,
            })
            .collect(),
        meta: SearchMeta { total },
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use bytes::{Bytes, BytesMut};
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
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryGroupStore,
        InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore, InMemoryReplicaStore,
        InMemoryRepositoryStore, InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore,
        InMemoryWebhookStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::assay::{Assay, AssayComponent, AssayComponentKind, AssayStatus};
    use ferrobox_domain::ids::{AssayId, RepositoryId};
    use ferrobox_domain::package_coordinate::{
        PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
    };
    use ferrobox_domain::repository::RepositoryName;
    use ferrobox_domain::user::Role;
    use ferrobox_ports::assay_store::AssayStore;
    use serde_json::Value;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::AppState;

    struct Fixture {
        app: Router,
        repo_id: Uuid,
        developer_token: String,
        reader_token: String,
        assay_store: Arc<InMemoryAssayStore>,
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
        let assay_store = Arc::new(InMemoryAssayStore::default());

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
            )
            .with_assays(assay_store.clone()),
            retention: ferrobox_application::retention::RetentionService::new(
                repository_store.clone(),
                artifact_store,
                package_index_store,
                storage,
                assay_store.clone(),
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
        let reader = state
            .create_user
            .seed("reader", Role::Reader)
            .await
            .unwrap();
        let developer_token = state
            .create_api_token
            .execute(developer.id(), ApiTokenName::parse("dev").unwrap(), None)
            .await
            .unwrap()
            .plaintext_secret;
        let reader_token = state
            .create_api_token
            .execute(reader.id(), ApiTokenName::parse("read").unwrap(), None)
            .await
            .unwrap()
            .plaintext_secret;

        let repo_id = state
            .create_repository
            .execute(
                RepositoryName::parse("crates-releases").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        Fixture {
            app: crate::build_router(state),
            repo_id: repo_id.into(),
            developer_token,
            reader_token,
            assay_store,
        }
    }

    fn encode_publish_payload(metadata_json: &str, crate_bytes: &[u8]) -> Bytes {
        let metadata_bytes = metadata_json.as_bytes();
        let mut payload = BytesMut::new();
        payload.extend_from_slice(&u32::try_from(metadata_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(metadata_bytes);
        payload.extend_from_slice(&u32::try_from(crate_bytes.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(crate_bytes);
        payload.freeze()
    }

    async fn send(app: Router, request: Request<Body>) -> (StatusCode, Bytes) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, body)
    }

    #[tokio::test]
    async fn config_json_is_public_and_does_not_require_auth() {
        let fx = fixture().await;
        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/cargo/{}/config.json", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["auth-required"], false);
        assert!(
            json["api"]
                .as_str()
                .unwrap()
                .contains(&fx.repo_id.to_string())
        );
    }

    #[tokio::test]
    async fn publish_without_a_token_is_unauthorized() {
        let fx = fixture().await;
        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball",
        );
        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn publish_accepts_cargo_raw_authorization_header() {
        let fx = fixture().await;
        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball",
        );
        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", fx.developer_token)
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn reader_cannot_publish() {
        let fx = fixture().await;
        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball",
        );
        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", format!("Token {}", fx.reader_token))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn publish_index_download_search_yank_and_unyank() {
        let fx = fixture().await;
        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball-bytes",
        );

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/cargo/{}/fe/rr/ferrobox-cli", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.windows(b"\"vers\":\"0.1.0\"".len())
                .any(|w| w == b"\"vers\":\"0.1.0\"")
        );
        assert!(
            body.windows(b"\"yanked\":false".len())
                .any(|w| w == b"\"yanked\":false")
        );

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!(
                    "/cargo/{}/api/v1/crates/ferrobox-cli/0.1.0/download",
                    fx.repo_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, Bytes::from_static(b"tarball-bytes"));

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/cargo/{}/api/v1/crates?q=ferro", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["crates"][0]["name"], "ferrobox-cli");
        assert_eq!(json["crates"][0]["max_version"], "0.1.0");
        assert_eq!(json["meta"]["total"], 1);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/cargo/{}/api/v1/crates?q=", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["crates"][0]["name"], "ferrobox-cli");

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/cargo/{}/api/v1/crates/ferrobox-cli/0.1.0/yank",
                    fx.repo_id
                ))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/cargo/{}/fe/rr/ferrobox-cli", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.windows(b"\"yanked\":true".len())
                .any(|w| w == b"\"yanked\":true")
        );

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/cargo/{}/api/v1/crates/ferrobox-cli/0.1.0/unyank",
                    fx.repo_id
                ))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/cargo/{}/fe/rr/ferrobox-cli", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.windows(b"\"yanked\":false".len())
                .any(|w| w == b"\"yanked\":false")
        );
    }

    #[tokio::test]
    async fn yank_without_a_token_is_unauthorized() {
        let fx = fixture().await;
        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/cargo/{}/api/v1/crates/ferrobox-cli/0.1.0/yank",
                    fx.repo_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn yank_is_forbidden_when_worm_is_enabled() {
        let fx = fixture().await;
        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball-bytes",
        );
        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/repositories/{}/worm", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app,
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/cargo/{}/api/v1/crates/ferrobox-cli/0.1.0/yank",
                    fx.repo_id
                ))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("WORM-enabled"));
    }

    #[tokio::test]
    async fn settings_requires_auth_and_returns_instance_info() {
        let fx = fixture().await;

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .uri("/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri("/settings")
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["public_base_url"], "http://127.0.0.1:3000");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn change_password_rejects_wrong_current_and_accepts_the_right_one() {
        let fx = fixture().await;

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri("/auth/password")
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"current_password":"nope","new_password":"NewSecret1"}"#,
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "current password is incorrect");

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri("/auth/password")
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"current_password":"Secret1a","new_password":"NewSecret1"}"#,
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"username":"developer","password":"NewSecret1"}"#,
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user"]["username"], "developer");
        assert!(
            json["token"]
                .as_str()
                .is_some_and(|token| token.starts_with("fb_"))
        );
    }

    #[tokio::test]
    async fn listing_artifacts_exposes_yanked_and_source_repository() {
        let fx = fixture().await;
        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball",
        );
        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", fx.developer_token.clone())
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/cargo/{}/api/v1/crates/ferrobox-cli/0.1.0/yank",
                    fx.repo_id
                ))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/repositories/{}/artifacts", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json[0]["name"], "ferrobox-cli");
        assert_eq!(json[0]["version"], "0.1.0");
        assert_eq!(json[0]["yanked"], true);
        assert_eq!(json[0]["repository_id"], fx.repo_id.to_string());
    }

    async fn create_repo(app: Router, token: &str, body: Value) -> (StatusCode, Value) {
        let (status, bytes) = send(
            app,
            Request::builder()
                .method("POST")
                .uri("/repositories")
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn promote_copies_a_crate_to_another_forge() {
        let fx = fixture().await;
        let (_, created) = create_repo(
            fx.app.clone(),
            &fx.developer_token,
            serde_json::json!({ "name": "crates-prod", "ecosystem": "cargo" }),
        )
        .await;
        let target_id = created["id"].as_str().unwrap().to_string();

        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball-bytes",
        );
        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{}/promote", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "target_repository_id": target_id,
                        "name": "ferrobox-cli",
                        "version": "0.1.0"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "ferrobox-cli");
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["artifacts_copied"], 1);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!(
                    "/cargo/{target_id}/api/v1/crates/ferrobox-cli/0.1.0/download"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, Bytes::from_static(b"tarball-bytes"));

        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{}/promote", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "target_repository_id": target_id,
                        "name": "ferrobox-cli",
                        "version": "0.1.0"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn promote_rejects_a_mirror_and_a_reader_without_write() {
        let fx = fixture().await;
        let (_, mirror) = create_repo(
            fx.app.clone(),
            &fx.developer_token,
            serde_json::json!({
                "name": "crates-upstream",
                "ecosystem": "cargo",
                "kind": { "type": "mirror", "upstream": "https://index.crates.io/" }
            }),
        )
        .await;
        let mirror_id = mirror["id"].as_str().unwrap();

        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball",
        );
        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{}/promote", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "target_repository_id": mirror_id,
                        "name": "ferrobox-cli",
                        "version": "0.1.0"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (_, prod) = create_repo(
            fx.app.clone(),
            &fx.developer_token,
            serde_json::json!({ "name": "crates-prod", "ecosystem": "cargo" }),
        )
        .await;
        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{}/promote", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.reader_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "target_repository_id": prod["id"],
                        "name": "ferrobox-cli",
                        "version": "0.1.0"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn promote_copies_a_generic_artifact() {
        let fx = fixture().await;
        let (_, source) = create_repo(
            fx.app.clone(),
            &fx.developer_token,
            serde_json::json!({ "name": "generic-dev", "ecosystem": "generic" }),
        )
        .await;
        let (_, target) = create_repo(
            fx.app.clone(),
            &fx.developer_token,
            serde_json::json!({ "name": "generic-prod", "ecosystem": "generic" }),
        )
        .await;
        let source_id = source["id"].as_str().unwrap().to_string();
        let target_id = target["id"].as_str().unwrap().to_string();

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{source_id}/artifacts"))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/octet-stream")
                .body(Body::from(Bytes::from_static(b"generic-bytes")))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let published: Value = serde_json::from_slice(&body).unwrap();
        let artifact_id = published["id"].as_str().unwrap();

        let (status, body) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{source_id}/promote"))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "target_repository_id": target_id,
                        "artifact_id": artifact_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["artifacts_copied"], 1);
        assert_eq!(json["bytes_copied"], 13);
    }

    #[tokio::test]
    async fn promote_is_denied_by_the_destination_admission_policy() {
        let fx = fixture().await;
        let (_, created) = create_repo(
            fx.app.clone(),
            &fx.developer_token,
            serde_json::json!({ "name": "crates-prod", "ecosystem": "cargo" }),
        )
        .await;
        let target_id = created["id"].as_str().unwrap().to_string();

        let payload = encode_publish_payload(
            r#"{"name":"ferrobox-cli","vers":"0.1.0","deps":[],"features":{}}"#,
            b"tarball-bytes",
        );
        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/cargo/{}/api/v1/crates/new", fx.repo_id))
                .header("Authorization", format!("Token {}", fx.developer_token))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let mut root = AssayComponent::new("ferrobox-cli", "0.1.0", None, AssayComponentKind::Root);
        root.add_licenses(["AGPL-3.0-only".to_string()]);
        fx.assay_store
            .upsert(&Assay::from_parts(
                AssayId::new(),
                RepositoryId::from(fx.repo_id),
                PackageCoordinate::new(
                    PackageEcosystem::Cargo,
                    PackageName::parse("ferrobox-cli").unwrap(),
                    PackageVersion::parse("0.1.0").unwrap(),
                ),
                AssayStatus::Ready,
                Some("2026-09-26T00:00:00Z".to_string()),
                None,
                vec![root],
                Vec::new(),
            ))
            .await
            .unwrap();

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/repositories/{target_id}/admission"))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "enabled": true,
                        "when": "pull",
                        "predicate": "not_signed",
                        "effect": "deny",
                        "profile": "copyleft_restrict",
                        "forbidden_licenses": ["AGPL-3.0-only"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{}/promote", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "target_repository_id": target_id,
                        "name": "ferrobox-cli",
                        "version": "0.1.0"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"]
                .as_str()
                .unwrap_or_default()
                .contains("promote"),
            "{json}"
        );

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/repositories/{target_id}/admission"))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "enabled": false,
                        "when": "pull",
                        "predicate": "not_signed",
                        "effect": "deny"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri(format!("/repositories/{}/promote", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "target_repository_id": target_id,
                        "name": "ferrobox-cli",
                        "version": "0.1.0"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
}
