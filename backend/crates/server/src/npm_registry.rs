//! Rutas HTTP que implementan el subconjunto del protocolo de registro
//! de npm que `npm publish` y `npm install` necesitan.
//!
//! Referencia:
//! <https://github.com/npm/registry/blob/main/docs/REGISTRY-API.md>.
//!
//! Las lecturas (packument, tarball, búsqueda y `/-/ping`) son públicas.
//! Las escrituras (`PUT` de publicación y yank) exigen `Authorization`
//! (`Bearer` o `Token`) y rol de escritura.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{get, put};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_application::packaging::npm::tarball_filename;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_write_artifacts;
use crate::error::ApiError;

/// Rutas de solo lectura del protocolo de npm.
pub(crate) fn public_router() -> Router<Arc<AppState>> {
    Router::new().route("/npm/{repository_id}/{*path}", get(npm_get))
}

/// Rutas de escritura del protocolo de npm.
pub(crate) fn write_router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/npm/{repository_id}/{*path}",
        put(npm_put).delete(npm_delete),
    )
}

fn npm_strategy(state: &AppState) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    state
        .packaging
        .strategy_for(PackageEcosystem::Npm)
        .ok_or_else(|| ApiError::Internal("no npm packaging strategy is registered".to_string()))
}

fn decode_npm_path(path: &str) -> String {
    path.trim_end_matches('/')
        .replace("%40", "@")
        .replace("%2F", "/")
        .replace("%2f", "/")
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    text: String,
    #[serde(default = "default_search_size")]
    size: u8,
}

fn default_search_size() -> u8 {
    20
}

#[derive(Serialize)]
struct NpmSearchPackage {
    name: String,
    version: String,
}

#[derive(Serialize)]
struct NpmSearchObject {
    package: NpmSearchPackage,
}

#[derive(Serialize)]
struct NpmSearchResponse {
    objects: Vec<NpmSearchObject>,
    total: usize,
}

#[derive(Serialize)]
struct NpmOk {
    ok: bool,
}

async fn npm_get(
    State(state): State<Arc<AppState>>,
    Path((repository_id, path)): Path<(Uuid, String)>,
    Query(search): Query<SearchParams>,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let path = decode_npm_path(&path);

    if path == "-/ping" {
        return Ok(json_raw(StatusCode::OK, Bytes::from_static(b"{}")));
    }

    if path == "-/v1/search" {
        return search_packages(&state, repository_id, &search).await;
    }

    if let Some((name, filename)) = path.split_once("/-/") {
        return download_tarball(&state, repository_id, name, filename).await;
    }

    serve_packument(&state, repository_id, &path).await
}

async fn npm_put(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, path)): Path<(Uuid, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<NpmOk>), ApiError> {
    require_write_artifacts(&user)?;
    let path = decode_npm_path(&path);

    if let Some((name, version)) = strip_suffix_action(&path, "/unyank") {
        set_yanked(&state, repository_id, &name, &version, false).await?;
        return Ok((StatusCode::OK, Json(NpmOk { ok: true })));
    }

    publish_package(&state, repository_id, body).await
}

async fn npm_delete(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, path)): Path<(Uuid, String)>,
) -> Result<(StatusCode, Json<NpmOk>), ApiError> {
    require_write_artifacts(&user)?;
    let path = decode_npm_path(&path);

    let Some((name, version)) = strip_suffix_action(&path, "/yank") else {
        return Err(ApiError::NotFound(format!(
            "npm path '{path}' is not a yank endpoint"
        )));
    };

    set_yanked(&state, repository_id, &name, &version, true).await?;
    Ok((StatusCode::OK, Json(NpmOk { ok: true })))
}

fn strip_suffix_action(path: &str, suffix: &str) -> Option<(String, String)> {
    let rest = path.strip_suffix(suffix)?.trim_end_matches('/');
    let (name, version) = rest.rsplit_once('/')?;
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

async fn serve_packument(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    if repository.ecosystem() != PackageEcosystem::Npm {
        return Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'npm'",
            repository.ecosystem().label()
        )));
    }

    let package_name =
        PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let strategy = npm_strategy(state)?;
    let body = strategy.index(&repository, &package_name).await?;
    Ok(json_raw(StatusCode::OK, body))
}

async fn download_tarball(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    filename: &str,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = npm_strategy(state)?;
    let package_name =
        PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version = version_from_tarball(name, filename).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "tarball filename '{filename}' does not match package '{name}'"
        ))
    })?;
    let version =
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(PackageEcosystem::Npm, package_name, version);
    let body = strategy.download(&repository, &coordinate).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok((StatusCode::OK, headers, body))
}

async fn search_packages(
    state: &AppState,
    repository_id: Uuid,
    search: &SearchParams,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = npm_strategy(state)?;
    let hits = strategy
        .search(&repository, &search.text, usize::from(search.size.max(1)))
        .await?;
    let objects: Vec<NpmSearchObject> = hits
        .into_iter()
        .map(|hit| NpmSearchObject {
            package: NpmSearchPackage {
                name: hit.name,
                version: hit.max_version,
            },
        })
        .collect();
    let total = objects.len();
    json_bytes(
        StatusCode::OK,
        &NpmSearchResponse { objects, total },
    )
}

async fn publish_package(
    state: &AppState,
    repository_id: Uuid,
    body: Bytes,
) -> Result<(StatusCode, Json<NpmOk>), ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = npm_strategy(state)?;
    strategy.publish(&repository, body).await?;
    Ok((StatusCode::CREATED, Json(NpmOk { ok: true })))
}

async fn set_yanked(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    version: &str,
    yanked: bool,
) -> Result<(), ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = npm_strategy(state)?;
    let name = PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version =
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(PackageEcosystem::Npm, name, version);
    strategy.set_yanked(&repository, &coordinate, yanked).await?;
    Ok(())
}

fn version_from_tarball(name: &str, filename: &str) -> Option<String> {
    let expected = tarball_filename(name, "VERSION");
    let (prefix, suffix) = expected.split_once("VERSION")?;
    let rest = filename.strip_prefix(prefix)?.strip_suffix(suffix)?;
    Some(rest.to_string())
}

fn json_raw(status: StatusCode, body: Bytes) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (status, headers, body)
}

fn json_bytes<T: Serialize>(
    status: StatusCode,
    value: &T,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let body = serde_json::to_vec(value).map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(json_raw(status, Bytes::from(body)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
    use ferrobox_application::manage_users::{
        ChangeUserRoleUseCase, CreateUserUseCase, DeleteUserUseCase, ListUsersUseCase,
    };
    use ferrobox_application::packaging::PackagingRegistry;
    use ferrobox_application::packaging::cargo::CargoPackagingStrategy;
    use ferrobox_application::packaging::npm::NpmPackagingStrategy;
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryHttpClient,
        InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryStorage, InMemoryUserStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::RepositoryName;
    use ferrobox_domain::user::{Role, Username};
    use serde_json::Value;
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
            .register(Arc::new(NpmPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
                "http://127.0.0.1:3000".to_string(),
            )));

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
                artifact_store,
                package_index_store.clone(),
                storage,
            ),
            packaging,
            assays: ferrobox_application::assay::AssayService::new(
                Arc::new(InMemoryAssayStore::default()),
                package_index_store.clone(),
                repository_store.clone(),
                http_client.clone(),
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
        let developer_token = state
            .create_api_token
            .execute(developer.id(), ApiTokenName::parse("dev").unwrap())
            .await
            .unwrap()
            .plaintext_secret;

        let repo_id = state
            .create_repository
            .execute(
                RepositoryName::parse("npm-releases").unwrap(),
                PackageEcosystem::Npm,
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

    fn publish_body(name: &str, version: &str) -> Bytes {
        Bytes::from(
            serde_json::json!({
                "name": name,
                "dist-tags": { "latest": version },
                "versions": {
                    version: { "name": name, "version": version }
                },
                "_attachments": {
                    format!("{name}-{version}.tgz"): {
                        "content_type": "application/octet-stream",
                        "data": "dGFyYmFsbA==",
                        "length": 7
                    }
                }
            })
            .to_string(),
        )
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
    async fn ping_is_public() {
        let fx = fixture().await;
        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/npm/{}/-/ping", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!({}));
    }

    #[tokio::test]
    async fn publish_without_a_token_is_unauthorized() {
        let fx = fixture().await;
        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("PUT")
                .uri(format!("/npm/{}/demo-pkg", fx.repo_id))
                .body(Body::from(publish_body("demo-pkg", "1.0.0")))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn publish_packument_tarball_search_and_yank() {
        let fx = fixture().await;

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/npm/{}/demo-pkg", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .header("content-type", "application/json")
                .body(Body::from(publish_body("demo-pkg", "1.0.0")))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/npm/{}/demo-pkg", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "demo-pkg");
        assert_eq!(json["dist-tags"]["latest"], "1.0.0");

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!(
                    "/npm/{}/demo-pkg/-/demo-pkg-1.0.0.tgz",
                    fx.repo_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"tarball");

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/npm/{}/-/v1/search?text=demo", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["objects"][0]["package"]["name"], "demo-pkg");

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("DELETE")
                .uri(format!("/npm/{}/demo-pkg/1.0.0/yank", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/npm/{}/demo-pkg", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["versions"]["1.0.0"]["deprecated"], "yanked");
    }
}
