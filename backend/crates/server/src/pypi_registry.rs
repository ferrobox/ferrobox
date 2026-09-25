//! Rutas HTTP que implementan el subconjunto del protocolo de `PyPI` que
//! `twine upload` y `pip install` necesitan.
//!
//! Referencias:
//! - índice simple: <https://peps.python.org/pep-0503/>
//! - índice JSON: <https://peps.python.org/pep-0691/>
//! - subida *legacy*: <https://docs.pypi.org/api/upload/>
//!
//! Las lecturas (`/simple/` y `/packages/`) son públicas. Las escrituras
//! (`POST` de subida y yank) exigen `Authorization` (`Bearer`, `Token` o
//! Basic) y rol de escritura.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_application::packaging::pypi::{
    normalize_pypi_name, project_page_json, simple_root_json, simple_root_page,
};
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

/// Tamaño máximo de una subida `twine` (sdist o wheel).
const PYPI_UPLOAD_LIMIT: usize = 100 * 1024 * 1024;

/// Rutas de solo lectura del protocolo de `PyPI`.
pub(crate) fn public_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/pypi/{repository_id}/simple/", get(simple_root))
        .route("/pypi/{repository_id}/simple", get(simple_root))
        .route("/pypi/{repository_id}/simple/{name}/", get(simple_project))
        .route("/pypi/{repository_id}/simple/{name}", get(simple_project))
        .route(
            "/pypi/{repository_id}/packages/{filename}",
            get(download_file),
        )
}

/// Rutas de escritura del protocolo de `PyPI`.
pub(crate) fn write_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/pypi/{repository_id}/", post(upload))
        .route("/pypi/{repository_id}", post(upload))
        .route("/pypi/{repository_id}/legacy/", post(upload))
        .route("/pypi/{repository_id}/legacy", post(upload))
        .route("/pypi/{repository_id}/{name}/{version}/yank", delete(yank))
        .route("/pypi/{repository_id}/{name}/{version}/unyank", put(unyank))
        .layer(DefaultBodyLimit::max(PYPI_UPLOAD_LIMIT))
}

fn pypi_strategy(state: &AppState) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    state
        .packaging
        .strategy_for(PackageEcosystem::PyPi)
        .ok_or_else(|| ApiError::Internal("no pypi packaging strategy is registered".to_string()))
}

#[derive(Serialize)]
struct PypiOk {
    ok: bool,
}

async fn simple_root(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let repository = load_pypi_repository(&state, repository_id).await?;
    let strategy = pypi_strategy(&state)?;
    let hits = strategy.search(&repository, "", 10_000).await?;
    if prefers_simple_json(&headers) {
        Ok(json_simple(simple_root_json(&hits)))
    } else {
        Ok(html_raw(simple_root_page(&hits)))
    }
}

async fn simple_project(
    State(state): State<Arc<AppState>>,
    Path((repository_id, name)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let repository = load_pypi_repository(&state, repository_id).await?;
    let package_name =
        PackageName::parse(name.clone()).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let strategy = pypi_strategy(&state)?;
    let html = strategy.index(&repository, &package_name).await?;
    if prefers_simple_json(&headers) {
        let page_url = format!(
            "{}/pypi/{}/simple/{}/",
            state.public_base_url.trim_end_matches('/'),
            repository_id,
            normalize_pypi_name(&name)
        );
        let json = project_page_json(package_name.as_str(), &html, &page_url)?;
        Ok(json_simple(json))
    } else {
        Ok(html_raw(html))
    }
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    Path((repository_id, filename)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Bytes), ApiError> {
    require_public_repo_read(
        &state.groups,
        &state.authenticate_token,
        &headers,
        RepositoryId::from(repository_id),
    )
    .await?;
    let repository = load_pypi_repository(&state, repository_id).await?;
    if filename.contains('/') || filename.contains('\\') {
        return Err(ApiError::BadRequest(
            "filename must not contain path separators".to_string(),
        ));
    }
    let strategy = pypi_strategy(&state)?;
    let body = strategy.download_file(&repository, &filename).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok((StatusCode::OK, headers, body))
}

async fn upload(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    multipart: Multipart,
) -> Result<(StatusCode, &'static str), ApiError> {
    require_repo_write(&state.groups, &user, RepositoryId::from(repository_id)).await?;
    let repository = load_pypi_repository(&state, repository_id).await?;
    let payload = multipart_to_publish_payload(multipart).await?;
    let strategy = pypi_strategy(&state)?;
    strategy.publish(&repository, payload).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::PackagePublished,
        AuditTargetKind::Package,
        repository.name().to_string(),
        "pypi",
    )
    .await;
    Ok((StatusCode::OK, "OK"))
}

async fn yank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<PypiOk>), ApiError> {
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
    Ok((StatusCode::OK, Json(PypiOk { ok: true })))
}

async fn unyank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, version)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<PypiOk>), ApiError> {
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
    Ok((StatusCode::OK, Json(PypiOk { ok: true })))
}

async fn set_yanked(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    version: &str,
    yanked: bool,
) -> Result<(), ApiError> {
    let repository = load_pypi_repository(state, repository_id).await?;
    let strategy = pypi_strategy(state)?;
    let name = PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version =
        PackageVersion::parse(version).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(PackageEcosystem::PyPi, name, version);
    strategy
        .set_yanked(&repository, &coordinate, yanked)
        .await?;
    Ok(())
}

async fn load_pypi_repository(
    state: &AppState,
    repository_id: Uuid,
) -> Result<ferrobox_domain::repository::Repository, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    if repository.ecosystem() != PackageEcosystem::PyPi {
        return Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'pypi'",
            repository.ecosystem().label()
        )));
    }
    Ok(repository)
}

struct UploadForm {
    action: Option<String>,
    name: Option<String>,
    version: Option<String>,
    filename: Option<String>,
    sha256_digest: Option<String>,
    content: Option<Bytes>,
}

async fn multipart_to_publish_payload(mut multipart: Multipart) -> Result<Bytes, ApiError> {
    let mut form = UploadForm {
        action: None,
        name: None,
        version: None,
        filename: None,
        sha256_digest: None,
        content: None,
    };

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        let file_name = field.file_name().map(ToOwned::to_owned);
        let bytes = field
            .bytes()
            .await
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;

        match field_name.as_str() {
            ":action" => form.action = Some(utf8_field(&bytes)),
            "name" => form.name = Some(utf8_field(&bytes)),
            "version" => form.version = Some(utf8_field(&bytes)),
            "filename" => form.filename = Some(utf8_field(&bytes)),
            "sha256_digest" => form.sha256_digest = Some(utf8_field(&bytes)),
            "content" => {
                if form.filename.is_none() {
                    form.filename = file_name;
                }
                form.content = Some(bytes);
            }
            _ => {}
        }
    }

    if let Some(action) = form.action.as_deref()
        && action != "file_upload"
    {
        return Err(ApiError::BadRequest(format!(
            "unsupported :action '{action}'"
        )));
    }

    let name = form
        .name
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::BadRequest("upload is missing package name".to_string()))?;
    let version = form
        .version
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::BadRequest("upload is missing package version".to_string()))?;
    let filename = form
        .filename
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::BadRequest("upload is missing filename".to_string()))?;
    let content = form
        .content
        .ok_or_else(|| ApiError::BadRequest("upload is missing file content".to_string()))?;

    let mut body = serde_json::json!({
        "name": name,
        "version": version,
        "filename": filename,
        "content": BASE64.encode(&content),
    });
    if let Some(digest) = form.sha256_digest.filter(|value| !value.is_empty()) {
        body["sha256_digest"] = serde_json::Value::String(digest);
    }

    Ok(Bytes::from(
        serde_json::to_vec(&body).map_err(|err| ApiError::Internal(err.to_string()))?,
    ))
}

fn utf8_field(bytes: &Bytes) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

fn html_raw(body: Bytes) -> (StatusCode, HeaderMap, Bytes) {
    simple_index_response("text/html; charset=utf-8", body)
}

fn json_simple(body: Bytes) -> (StatusCode, HeaderMap, Bytes) {
    simple_index_response("application/vnd.pypi.simple.v1+json", body)
}

fn simple_index_response(
    content_type: &'static str,
    body: Bytes,
) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::VARY, HeaderValue::from_static("Accept"));
    (StatusCode::OK, headers, body)
}

/// Elige JSON PEP 691 si el cliente lo prefiere sobre HTML (como `pip` y `uv`).
/// Sin `Accept`, o con solo `*/*`, se sirve HTML PEP 503.
fn prefers_simple_json(headers: &HeaderMap) -> bool {
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };

    let mut json_q: Option<f32> = None;
    let mut html_q: Option<f32> = None;
    for part in accept.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (media, params) = part
            .split_once(';')
            .map_or((part, ""), |(media, rest)| (media, rest));
        let media = media.trim().to_ascii_lowercase();
        let mut quality = 1.0_f32;
        for param in params.split(';') {
            let param = param.trim();
            if let Some(value) = param
                .strip_prefix("q=")
                .or_else(|| param.strip_prefix("Q="))
            {
                quality = value.trim().parse().unwrap_or(0.0);
            }
        }
        if media == "application/vnd.pypi.simple.v1+json" || media == "application/json" {
            json_q = Some(json_q.map_or(quality, |old| old.max(quality)));
        }
        if media == "text/html" || media == "application/vnd.pypi.simple.v1+html" || media == "*/*"
        {
            html_q = Some(html_q.map_or(quality, |old| old.max(quality)));
        }
    }

    match (json_q, html_q) {
        (Some(json), Some(html)) => json >= html,
        (Some(_), None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
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
    use ferrobox_application::packaging::npm::NpmPackagingStrategy;
    use ferrobox_application::packaging::pypi::PypiPackagingStrategy;
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
            .register(Arc::new(NpmPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
                "http://127.0.0.1:3000".to_string(),
            )))
            .register(Arc::new(PypiPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
                "http://127.0.0.1:3000".to_string(),
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
                RepositoryName::parse("pypi-releases").unwrap(),
                PackageEcosystem::PyPi,
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

    fn multipart_body(
        name: &str,
        version: &str,
        filename: &str,
        content: &[u8],
    ) -> (String, Bytes) {
        let boundary = "----FerroBoxBoundary";
        let mut body = Vec::new();
        for (field, value) in [
            (":action", "file_upload"),
            ("name", name),
            ("version", version),
        ] {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{field}\"\r\n\r\n").as_bytes(),
            );
            body.extend_from_slice(value.as_bytes());
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"content\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (
            format!("multipart/form-data; boundary={boundary}"),
            Bytes::from(body),
        )
    }

    async fn send(app: Router, request: Request<Body>) -> (StatusCode, Bytes) {
        let (_, status, body) = send_with_headers(app, request).await;
        (status, body)
    }

    async fn send_with_headers(
        app: Router,
        request: Request<Body>,
    ) -> (HeaderMap, StatusCode, Bytes) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (headers, status, body)
    }

    fn accept_headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn prefers_json_when_pip_advertises_it() {
        assert!(super::prefers_simple_json(&accept_headers(
            "application/vnd.pypi.simple.v1+json, application/vnd.pypi.simple.v1+html;q=0.1, text/html;q=0.01",
        )));
    }

    #[test]
    fn prefers_html_when_accept_is_missing() {
        assert!(!super::prefers_simple_json(&HeaderMap::new()));
    }

    #[test]
    fn prefers_html_when_accept_is_star() {
        assert!(!super::prefers_simple_json(&accept_headers("*/*")));
    }

    #[test]
    fn prefers_html_when_html_has_higher_quality() {
        assert!(!super::prefers_simple_json(&accept_headers(
            "text/html, application/vnd.pypi.simple.v1+json;q=0.5",
        )));
    }

    #[tokio::test]
    async fn simple_root_is_public() {
        let fx = fixture().await;
        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/pypi/{}/simple/", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("Simple Index"));
    }

    #[tokio::test]
    async fn upload_without_a_token_is_unauthorized() {
        let fx = fixture().await;
        let (content_type, body) =
            multipart_body("demo-pypi", "1.0.0", "demo_pypi-1.0.0.tar.gz", b"sdist");
        let (status, _) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri(format!("/pypi/{}/", fx.repo_id))
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn upload_simple_download_and_yank() {
        let fx = fixture().await;
        let filename = "demo_pypi-1.0.0.tar.gz";
        let (content_type, body) = multipart_body("Demo_Pypi", "1.0.0", filename, b"sdist-bytes");
        let basic = BASE64.encode(format!("__token__:{}", fx.developer_token));

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/pypi/{}/legacy/", fx.repo_id))
                .header("Authorization", format!("Basic {basic}"))
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/pypi/{}/simple/", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("href=\"demo-pypi/\""));

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/pypi/{}/simple/demo-pypi/", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains(filename));
        assert!(html.contains("#sha256="));
        assert!(html.contains(&format!("/pypi/{}/packages/{filename}", fx.repo_id)));

        let (status, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/pypi/{}/packages/{filename}", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"sdist-bytes");

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("DELETE")
                .uri(format!("/pypi/{}/demo-pypi/1.0.0/yank", fx.repo_id))
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/pypi/{}/simple/demo-pypi/", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("data-yanked"));
    }

    #[tokio::test]
    async fn simple_index_negotiates_pep691_json() {
        let fx = fixture().await;
        let filename = "demo_pypi-1.0.0.tar.gz";
        let (content_type, body) = multipart_body("Demo_Pypi", "1.0.0", filename, b"sdist-bytes");
        let basic = BASE64.encode(format!("__token__:{}", fx.developer_token));
        let pip_accept = "application/vnd.pypi.simple.v1+json, application/vnd.pypi.simple.v1+html;q=0.1, text/html;q=0.01";

        let (status, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/pypi/{}/legacy/", fx.repo_id))
                .header("Authorization", format!("Basic {basic}"))
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (headers, status, body) = send_with_headers(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/pypi/{}/simple/", fx.repo_id))
                .header("Accept", pip_accept)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/vnd.pypi.simple.v1+json"
        );
        assert_eq!(headers.get(header::VARY).unwrap(), "Accept");
        let root: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(root["meta"]["api-version"], "1.0");
        assert_eq!(root["projects"][0]["name"], "demo-pypi");

        let (headers, status, body) = send_with_headers(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/pypi/{}/simple/demo-pypi/", fx.repo_id))
                .header("Accept", pip_accept)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/vnd.pypi.simple.v1+json"
        );
        let project: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(project["name"], "demo-pypi");
        assert_eq!(project["files"][0]["filename"], filename);
        assert_eq!(
            project["files"][0]["hashes"]["sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert!(
            project["files"][0]["url"]
                .as_str()
                .unwrap()
                .contains(&format!("/pypi/{}/packages/{filename}", fx.repo_id))
        );

        let (headers, status, body) = send_with_headers(
            fx.app,
            Request::builder()
                .uri(format!("/pypi/{}/simple/", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(std::str::from_utf8(&body).unwrap().contains("Simple Index"));
    }

    #[tokio::test]
    async fn missing_package_file_is_not_found() {
        let fx = fixture().await;
        let (status, _) = send(
            fx.app,
            Request::builder()
                .uri(format!(
                    "/pypi/{}/packages/missing-1.0.0.tar.gz",
                    fx.repo_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
