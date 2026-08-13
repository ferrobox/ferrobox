//! Rutas HTTP que implementan el subconjunto del protocolo de registro
//! de Cargo que `cargo publish`, `cargo yank`, `cargo search`, `cargo add`
//! y `cargo build` necesitan para publicar paquetes y resolver
//! dependencias contra `FerroBox` usando el protocolo de índice disperso
//! (*sparse index*).
//!
//! Referencia: <https://doc.rust-lang.org/cargo/reference/registries.html>.
//!
//! Las lecturas (`config.json`, índice, descarga y búsqueda) son
//! públicas: `cargo build` / `cargo add` no envían credenciales salvo
//! que `auth-required` sea `true`. Las escrituras (`publish`, `yank`,
//! `unyank`) exigen `Authorization: Bearer` o `Token` y rol de escritura.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_application::packaging::cargo::cargo_index_shard_path;
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

/// Rutas de solo lectura del protocolo de Cargo. Van en el router
/// público para que `cargo` pueda resolver dependencias sin token.
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

/// Rutas de escritura del protocolo de Cargo. Van detrás de
/// autenticación y de [`require_write_artifacts`].
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

/// Respuesta de `GET /cargo/{repository_id}/config.json`: le indica a
/// `cargo` dónde descargar el contenido de un `.crate` (`dl`) y dónde
/// enviar peticiones de publicación (`api`). `auth-required` es `false`
/// para que el índice y las descargas no exijan token.
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

    let base = format!("{}/cargo/{repository_id}", state.public_base_url);
    Ok(Json(RegistryConfig {
        dl: format!("{base}/api/v1/crates"),
        api: base,
        auth_required: false,
    }))
}

async fn index_len1(
    State(state): State<Arc<AppState>>,
    Path((repository_id, name)): Path<(Uuid, String)>,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("1/{name}");
    serve_index(&state, repository_id, name, &expected).await
}

async fn index_len2(
    State(state): State<Arc<AppState>>,
    Path((repository_id, name)): Path<(Uuid, String)>,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("2/{name}");
    serve_index(&state, repository_id, name, &expected).await
}

async fn index_len3(
    State(state): State<Arc<AppState>>,
    Path((repository_id, prefix, name)): Path<(Uuid, String, String)>,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("3/{prefix}/{name}");
    serve_index(&state, repository_id, name, &expected).await
}

async fn index_len4(
    State(state): State<Arc<AppState>>,
    Path((repository_id, prefix, suffix, name)): Path<(Uuid, String, String, String)>,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let expected = format!("{prefix}/{suffix}/{name}");
    serve_index(&state, repository_id, name, &expected).await
}

/// Sirve el índice disperso de un crate. `expected_shard` es la ruta de
/// fragmentación que `cargo` debería haber pedido para `name`; si no
/// coincide, se responde 404 para no filtrar paquetes por rutas
/// incorrectas.
async fn serve_index(
    state: &AppState,
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

/// Cuerpo de respuesta que `cargo publish` espera tras una publicación
/// correcta. `cargo` no falla si estos campos vienen vacíos, pero sí
/// espera que el objeto exista.
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

/// `PUT /cargo/{repository_id}/api/v1/crates/new`: el endpoint que
/// `cargo publish` invoca. Exige `Authorization: Bearer <token>`
/// (también se acepta el esquema `Token` que envía `cargo` por defecto).
async fn publish(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    body: Bytes,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
    require_write_artifacts(&user)?;

    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(&state)?;

    strategy.publish(&repository, body).await?;

    Ok((StatusCode::OK, Json(PublishResponse::default())))
}

/// `GET /cargo/{repository_id}/api/v1/crates/{name}/{version}/download`.
async fn download(
    State(state): State<Arc<AppState>>,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<Bytes, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(&state)?;

    let name = PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version =
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(PackageEcosystem::Cargo, name, version);

    let content = strategy.download(&repository, &coordinate).await?;
    Ok(content)
}

async fn yank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<Json<YankResponse>, ApiError> {
    set_yanked(&state, &user, repository_id, name, version, true).await
}

async fn unyank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<Json<YankResponse>, ApiError> {
    set_yanked(&state, &user, repository_id, name, version, false).await
}

async fn set_yanked(
    state: &AppState,
    user: &ferrobox_domain::user::User,
    repository_id: Uuid,
    name: String,
    version: String,
    yanked: bool,
) -> Result<Json<YankResponse>, ApiError> {
    require_write_artifacts(user)?;

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

    Ok(Json(YankResponse { ok: true }))
}

/// `GET /cargo/{repository_id}/api/v1/crates?q=…&per_page=…`.
async fn search(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, ApiError> {
    let query = params.q.trim();
    if query.is_empty() {
        return Err(ApiError::BadRequest(
            "search query parameter 'q' cannot be empty".to_string(),
        ));
    }

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
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryHttpClient,
        InMemoryPackageIndexStore, InMemoryRepositoryStore, InMemoryStorage, InMemoryUserStore,
    };
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
        reader_token: String,
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

        let packaging = PackagingRegistry::new().register(Arc::new(CargoPackagingStrategy::new(
            artifact_store.clone(),
            package_index_store.clone(),
            storage.clone(),
            http_client,
        )));

        let state = Arc::new(AppState {
            create_repository: CreateRepositoryUseCase::new(repository_store.clone()),
            list_repositories: ListRepositoriesUseCase::new(repository_store.clone()),
            get_repository: GetRepositoryUseCase::new(repository_store.clone()),
            publish_artifact: PublishArtifactUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                storage.clone(),
            ),
            download_artifact: DownloadArtifactUseCase::new(
                artifact_store.clone(),
                storage.clone(),
            ),
            list_repository_artifacts: ListRepositoryArtifactsUseCase::new(artifact_store.clone()),
            delete_repository: DeleteRepositoryUseCase::new(
                repository_store.clone(),
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
            ),
            delete_artifact: DeleteArtifactUseCase::new(
                repository_store.clone(),
                artifact_store,
                package_index_store,
                storage,
            ),
            packaging,
            public_base_url: "http://127.0.0.1:3000".to_string(),
            login: LoginUseCase::new(user_store.clone(), api_token_store.clone()),
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
        let reader = state
            .create_user
            .execute(Username::parse("reader").unwrap(), "secret", Role::Reader)
            .await
            .unwrap();
        let developer_token = state
            .create_api_token
            .execute(developer.id(), ApiTokenName::parse("dev").unwrap())
            .await
            .unwrap()
            .plaintext_secret;
        let reader_token = state
            .create_api_token
            .execute(reader.id(), ApiTokenName::parse("read").unwrap())
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
}
