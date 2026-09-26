//! Rutas HTTP de la API V3 de `NuGet` (`dotnet nuget push` /
//! `dotnet restore` / unlist).
//!
//! El origen se monta en `/nuget/<UUID>/v3/`. Las lecturas son públicas;
//! el *push* y el unlist exigen token de API (`X-NuGet-ApiKey`, Bearer,
//! Token o Basic) y rol de escritura.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_application::packaging::nuget::extract_nupkg_bytes;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use serde::Serialize;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_public_repo_read, require_repo_write};
use crate::error::ApiError;

const NUGET_UPLOAD_LIMIT: usize = 512 * 1024 * 1024;

/// Rutas de solo lectura (índice de servicio, flat container, search).
pub(crate) fn public_router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/nuget/{repository_id}/index.json",
            get(nuget_root_index).head(nuget_root_head),
        )
        .route(
            "/nuget/{repository_id}/{*path}",
            get(nuget_get).head(nuget_head),
        )
}

/// Rutas de escritura (push, unlist y relist).
pub(crate) fn write_router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/nuget/{repository_id}/v3/package",
            put(nuget_push).post(nuget_push),
        )
        .route(
            "/nuget/{repository_id}/v3/package/{name}/{version}",
            delete(nuget_unlist).post(nuget_relist),
        )
        .route(
            "/index/nuget/{repository_id}/{name}/{version}/yank",
            delete(yank),
        )
        .route(
            "/index/nuget/{repository_id}/{name}/{version}/unyank",
            put(unyank),
        )
        .layer(DefaultBodyLimit::max(NUGET_UPLOAD_LIMIT))
}

fn nuget_strategy(state: &AppState) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    state
        .packaging
        .strategy_for(PackageEcosystem::Nuget)
        .ok_or_else(|| ApiError::Internal("no nuget packaging strategy is registered".to_string()))
}

async fn load_nuget_repository(
    state: &AppState,
    repository_id: Uuid,
) -> Result<ferrobox_domain::repository::Repository, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    if repository.ecosystem() != PackageEcosystem::Nuget {
        return Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'nuget'",
            repository.ecosystem().label()
        )));
    }
    Ok(repository)
}

fn content_type_for(path: &str) -> &'static str {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let extension = std::path::Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");
    if extension.eq_ignore_ascii_case("nupkg") {
        "application/octet-stream"
    } else if extension.eq_ignore_ascii_case("nuspec") || extension.eq_ignore_ascii_case("xml") {
        "application/xml"
    } else {
        "application/json"
    }
}

fn bytes_response(path: &str, body: Bytes) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type_for(path)),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string()).expect("content-length is ASCII"),
    );
    (StatusCode::OK, headers, body)
}

fn protocol_path(path: &str, uri: &Uri) -> String {
    let trimmed = path.trim_matches('/');
    let Some(query) = uri.query() else {
        return trimmed.to_string();
    };
    if trimmed == "v3/query"
        || trimmed == "query"
        || trimmed == "v3/autocomplete"
        || trimmed == "autocomplete"
    {
        format!("{trimmed}?{query}")
    } else {
        trimmed.to_string()
    }
}

async fn nuget_root_index(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    nuget_get(
        State(state),
        Path((repository_id, "v3/index.json".to_string())),
        headers,
        uri,
    )
    .await
}

async fn nuget_root_head(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let (status, headers, _) =
        nuget_root_index(State(state), Path(repository_id), headers, uri).await?;
    Ok((status, headers, Bytes::new()))
}

async fn nuget_head(
    State(state): State<Arc<AppState>>,
    Path((repository_id, path)): Path<(Uuid, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let (status, headers, _) =
        nuget_get(State(state), Path((repository_id, path)), headers, uri).await?;
    Ok((status, headers, Bytes::new()))
}

async fn nuget_get(
    State(state): State<Arc<AppState>>,
    Path((repository_id, path)): Path<(Uuid, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let repository = load_nuget_repository(&state, repository_id).await?;
    let strategy = nuget_strategy(&state)?;
    let path = protocol_path(&path, &uri);
    crate::admission::enforce_download(
        &state,
        repository.id(),
        ferrobox_application::packaging::nuget::admission_download_target(&path),
    )
    .await?;
    let body = strategy.get_protocol_file(&repository, &path).await?;
    Ok(bytes_response(&path, body))
}

async fn nuget_push(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    require_repo_write(&state.groups, &user, &token, RepositoryId::from(repository_id)).await?;
    let repository = load_nuget_repository(&state, repository_id).await?;
    let strategy = nuget_strategy(&state)?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let nupkg = extract_nupkg_bytes(content_type, body)?;
    let coordinate = strategy.publish(&repository, nupkg).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::PackagePublished,
        AuditTargetKind::Package,
        coordinate.name().as_str().to_string(),
        coordinate.version().as_str().to_string(),
    )
    .await;
    Ok(StatusCode::CREATED)
}

#[derive(Serialize)]
struct NugetOk {
    ok: bool,
}

async fn nuget_unlist(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<StatusCode, ApiError> {
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
    Ok(StatusCode::OK)
}

async fn nuget_relist(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<StatusCode, ApiError> {
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
    Ok(StatusCode::OK)
}

async fn yank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<NugetOk>), ApiError> {
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
    Ok((StatusCode::OK, Json(NugetOk { ok: true })))
}

async fn unyank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<NugetOk>), ApiError> {
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
    Ok((StatusCode::OK, Json(NugetOk { ok: true })))
}

async fn set_yanked(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    version: &str,
    yanked: bool,
) -> Result<(), ApiError> {
    let repository = load_nuget_repository(state, repository_id).await?;
    let strategy = nuget_strategy(state)?;
    let coordinate = PackageCoordinate::new(
        PackageEcosystem::Nuget,
        PackageName::parse(name.to_ascii_lowercase())
            .map_err(|err| ApiError::BadRequest(err.to_string()))?,
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?,
    );
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
    use ferrobox_application::packaging::nuget::{NugetPackagingStrategy, build_nupkg};
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
            .register(Arc::new(NugetPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
                "http://127.0.0.1:3000",
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
                RepositoryName::parse("nuget-local").unwrap(),
                PackageEcosystem::Nuget,
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

    async fn body_bytes(response: axum::http::Response<Body>) -> bytes::Bytes {
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn push_then_service_index_and_nupkg() {
        let fixture = fixture().await;
        let nupkg = build_nupkg("Hello.World", "1.0.0");
        let put = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/nuget/{}/v3/package", fixture.repo_id))
                    .header("X-NuGet-ApiKey", &fixture.developer_token)
                    .body(Body::from(nupkg.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        let index = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/nuget/{}/v3/index.json", fixture.repo_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(index.status(), StatusCode::OK);
        let index_json: serde_json::Value =
            serde_json::from_slice(&body_bytes(index).await).unwrap();
        assert_eq!(index_json["version"], "3.0.0");

        let nupkg_get = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/nuget/{}/v3/flat/hello.world/1.0.0/hello.world.1.0.0.nupkg",
                        fixture.repo_id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(nupkg_get.status(), StatusCode::OK);

        let search = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/nuget/{}/v3/query?q=hello", fixture.repo_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(search.status(), StatusCode::OK);
        let search_json: serde_json::Value =
            serde_json::from_slice(&body_bytes(search).await).unwrap();
        assert_eq!(search_json["totalHits"], 1);
    }

    #[tokio::test]
    async fn unlist_hides_from_search_and_rejects_anonymous_push() {
        let fixture = fixture().await;
        let nupkg = build_nupkg("Hello.World", "1.0.0");
        let put = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/nuget/{}/v3/package", fixture.repo_id))
                    .header(
                        "Authorization",
                        format!("Bearer {}", fixture.developer_token),
                    )
                    .body(Body::from(nupkg.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        let unlist = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/nuget/{}/v3/package/Hello.World/1.0.0",
                        fixture.repo_id
                    ))
                    .header("X-NuGet-ApiKey", &fixture.developer_token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unlist.status(), StatusCode::OK);

        let search = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/nuget/{}/v3/query?q=hello", fixture.repo_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let search_json: serde_json::Value =
            serde_json::from_slice(&body_bytes(search).await).unwrap();
        assert_eq!(search_json["totalHits"], 0);

        let anon = fixture
            .app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/nuget/{}/v3/package", fixture.repo_id))
                    .body(Body::from(build_nupkg("Other.Lib", "2.0.0").to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
    }
}
