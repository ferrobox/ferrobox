//! Rutas HTTP del layout Maven (`mvn deploy` / `mvn dependency:get` /
//! Gradle).
//!
//! El remoto se monta en `/maven/<UUID>/`. Las lecturas son públicas;
//! las escrituras (`PUT`) exigen token de API (Bearer, Token o Basic)
//! y rol de escritura. *Releases* y *SNAPSHOT* comparten la misma URL.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
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

const MAVEN_UPLOAD_LIMIT: usize = 512 * 1024 * 1024;

/// Rutas de solo lectura (artefactos, metadatos y checksums).
pub(crate) fn public_router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/maven/{repository_id}/{*path}",
        get(maven_get).head(maven_head),
    )
}

/// Rutas de escritura (`PUT` de ficheros y yank).
pub(crate) fn write_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/maven/{repository_id}/{*path}", put(maven_put))
        .route(
            "/index/maven/{repository_id}/{name}/{version}/yank",
            delete(yank),
        )
        .route(
            "/index/maven/{repository_id}/{name}/{version}/unyank",
            put(unyank),
        )
        .layer(DefaultBodyLimit::max(MAVEN_UPLOAD_LIMIT))
}

fn maven_strategy(state: &AppState) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    state
        .packaging
        .strategy_for(PackageEcosystem::Maven)
        .ok_or_else(|| ApiError::Internal("no maven packaging strategy is registered".to_string()))
}

async fn load_maven_repository(
    state: &AppState,
    repository_id: Uuid,
) -> Result<ferrobox_domain::repository::Repository, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    if repository.ecosystem() != PackageEcosystem::Maven {
        return Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'maven'",
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
    if extension.eq_ignore_ascii_case("md5")
        || extension.eq_ignore_ascii_case("sha1")
        || extension.eq_ignore_ascii_case("sha256")
        || extension.eq_ignore_ascii_case("sha512")
    {
        return "text/plain";
    }
    if filename.eq_ignore_ascii_case("maven-metadata.xml")
        || extension.eq_ignore_ascii_case("pom")
        || extension.eq_ignore_ascii_case("xml")
    {
        "application/xml"
    } else if extension.eq_ignore_ascii_case("jar")
        || extension.eq_ignore_ascii_case("war")
        || extension.eq_ignore_ascii_case("ear")
    {
        "application/java-archive"
    } else {
        "application/octet-stream"
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

async fn maven_head(
    State(state): State<Arc<AppState>>,
    Path((repository_id, path)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    let (status, headers, _) =
        maven_get(State(state), Path((repository_id, path)), headers).await?;
    Ok((status, headers, Bytes::new()))
}

async fn maven_get(
    State(state): State<Arc<AppState>>,
    Path((repository_id, path)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let repository = load_maven_repository(&state, repository_id).await?;
    let strategy = maven_strategy(&state)?;
    let path = path.trim_matches('/');
    let body = strategy.get_protocol_file(&repository, path).await?;
    Ok(bytes_response(path, body))
}

async fn maven_put(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, path)): Path<(Uuid, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    require_repo_write(&state.groups, &user, RepositoryId::from(repository_id)).await?;
    let repository = load_maven_repository(&state, repository_id).await?;
    let strategy = maven_strategy(&state)?;
    let path = path.trim_matches('/');
    strategy.put_protocol_file(&repository, path, body).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Serialize)]
struct MavenOk {
    ok: bool,
}

async fn yank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<MavenOk>), ApiError> {
    require_repo_write(&state.groups, &user, RepositoryId::from(repository_id)).await?;
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
    Ok((StatusCode::OK, Json(MavenOk { ok: true })))
}

async fn unyank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<MavenOk>), ApiError> {
    require_repo_write(&state.groups, &user, RepositoryId::from(repository_id)).await?;
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
    Ok((StatusCode::OK, Json(MavenOk { ok: true })))
}

async fn set_yanked(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    version: &str,
    yanked: bool,
) -> Result<(), ApiError> {
    let repository = load_maven_repository(state, repository_id).await?;
    let strategy = maven_strategy(state)?;
    let coordinate = PackageCoordinate::new(
        PackageEcosystem::Maven,
        PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?,
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
    use ferrobox_application::packaging::maven::MavenPackagingStrategy;
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryGroupStore,
        InMemoryHttpClient, InMemoryPackageIndexStore, InMemoryQuotaStore, InMemoryRepositoryStore,
        InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore, InMemoryWebhookStore,
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
            .register(Arc::new(MavenPackagingStrategy::new(
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
            .execute(developer.id(), ApiTokenName::parse("dev").unwrap())
            .await
            .unwrap()
            .plaintext_secret;

        let repo_id = state
            .create_repository
            .execute(
                RepositoryName::parse("maven-releases").unwrap(),
                PackageEcosystem::Maven,
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

    #[tokio::test]
    async fn put_then_get_jar_metadata_and_checksum() {
        let fixture = fixture().await;
        let path = format!(
            "/maven/{}/org/example/hello/1.0.0/hello-1.0.0.jar",
            fixture.repo_id
        );
        let put = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&path)
                    .header(
                        "authorization",
                        format!("Bearer {}", fixture.developer_token),
                    )
                    .body(Body::from("jar-bytes"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        let get = fixture
            .app
            .clone()
            .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        let body = axum::body::to_bytes(get.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"jar-bytes");

        let metadata = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/maven/{}/org/example/hello/maven-metadata.xml",
                        fixture.repo_id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metadata.status(), StatusCode::OK);
        let xml = String::from_utf8(
            axum::body::to_bytes(metadata.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(xml.contains("<version>1.0.0</version>"));
        assert!(xml.contains("<release>1.0.0</release>"));

        let checksum = fixture
            .app
            .oneshot(
                Request::builder()
                    .uri(format!("{path}.sha1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(checksum.status(), StatusCode::OK);
        assert_eq!(
            checksum.headers().get("content-type").unwrap(),
            "text/plain"
        );
    }

    #[tokio::test]
    async fn put_without_token_is_unauthorized() {
        let fixture = fixture().await;
        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/maven/{}/org/example/hello/1.0.0/hello-1.0.0.jar",
                        fixture.repo_id
                    ))
                    .body(Body::from("jar"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
