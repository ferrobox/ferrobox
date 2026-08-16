//! Rutas HTTP del Distribution Spec v2 (`docker push` / `docker pull`).
//!
//! Referencia: <https://github.com/opencontainers/distribution-spec>.
//!
//! Las lecturas de manifiestos, blobs y etiquetas son públicas. `GET /v2/`
//! responde 200 sin desafío: Helm/ORAS (3.18+) hace ping sin credenciales y,
//! si recibe 401, cachea el token anónimo de `/v2/token` y lo reutiliza en
//! `helm push` sin reintentar. Las escrituras (subida de blobs y
//! manifiestos, yank) exigen un token de API (`Bearer`, `Token` o Basic)
//! y rol de escritura. Un `scope` con `push` en `/v2/token` no emite el
//! token anónimo.
//!
//! El registro vive en la raíz del host (`/v2/`). El primer componente
//! del nombre de imagen es el UUID del repositorio `FerroBox`; el resto
//! puede incluir barras: `127.0.0.1:3000/<UUID>/demo:latest` o
//! `127.0.0.1:3000/<UUID>/bitnami/nginx:latest`.
//!
//! Los repositorios **Helm** usan las mismas rutas: los charts son
//! artefactos OCI (`helm push` / `helm pull`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::routing::get;
use axum::{Json, Router};
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_application::packaging::oci::DEFAULT_MANIFEST_MEDIA_TYPE;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::{
    AuthenticatedUser, OCI_ANONYMOUS_TOKEN, extract_bearer_token, oci_bearer_challenge,
};
use crate::authz::require_write_artifacts;
use crate::error::ApiError;

/// Tamaño máximo de un blob o manifiesto OCI.
const OCI_UPLOAD_LIMIT: usize = 512 * 1024 * 1024;

static DOCKER_CONTENT_DIGEST: HeaderName = HeaderName::from_static("docker-content-digest");
static DOCKER_DISTRIBUTION_API_VERSION: HeaderName =
    HeaderName::from_static("docker-distribution-api-version");
static DOCKER_UPLOAD_UUID: HeaderName = HeaderName::from_static("docker-upload-uuid");

/// Rutas de solo lectura del protocolo OCI.
pub(crate) fn public_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v2/", get(version_check))
        .route("/v2", get(version_check))
        .route("/v2/token", get(issue_token).post(issue_token))
        .route("/v2/token/", get(issue_token).post(issue_token))
        .route(
            "/v2/{repository_id}/{*rest}",
            get(dispatch_read).head(dispatch_read),
        )
}

/// Rutas de escritura del protocolo OCI.
pub(crate) fn write_router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/v2/{repository_id}/{*rest}",
            axum::routing::put(dispatch_write)
                .post(dispatch_write)
                .patch(dispatch_write),
        )
        .route(
            "/oci/{repository_id}/{name}/{reference}/yank",
            axum::routing::delete(yank),
        )
        .route(
            "/oci/{repository_id}/{name}/{reference}/unyank",
            axum::routing::put(unyank),
        )
        .layer(DefaultBodyLimit::max(OCI_UPLOAD_LIMIT))
}

fn distribution_strategy(
    state: &AppState,
    repository: &ferrobox_domain::repository::Repository,
) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    match repository.ecosystem() {
        PackageEcosystem::Oci | PackageEcosystem::Helm => state
            .packaging
            .strategy_for(repository.ecosystem())
            .ok_or_else(|| {
                ApiError::Internal(format!(
                    "no {} packaging strategy is registered",
                    repository.ecosystem().label()
                ))
            }),
        other => Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'oci' or 'helm'",
            other.label()
        ))),
    }
}

async fn load_distribution_repository(
    state: &AppState,
    repository_id: Uuid,
) -> Result<ferrobox_domain::repository::Repository, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    match repository.ecosystem() {
        PackageEcosystem::Oci | PackageEcosystem::Helm => Ok(repository),
        other => Err(ApiError::BadRequest(format!(
            "repository is configured for ecosystem '{}', not 'oci' or 'helm'",
            other.label()
        ))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DistributionPath {
    Manifest { name: String, reference: String },
    Blob { name: String, digest: String },
    Tags { name: String },
    StartUpload { name: String },
    Upload { name: String, upload_id: Uuid },
}

fn parse_distribution_path(rest: &str) -> Result<DistributionPath, OciApiError> {
    let trimmed = rest.trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(OciApiError::from_code(
            StatusCode::NOT_FOUND,
            "NAME_UNKNOWN",
            "empty OCI image name",
        ));
    }
    if let Some(name) = trimmed.strip_suffix("/tags/list") {
        return require_image_name(name).map(|name| DistributionPath::Tags { name });
    }
    if let Some((name, reference)) = trimmed.rsplit_once("/manifests/") {
        if reference.is_empty() {
            return Err(OciApiError::from_code(
                StatusCode::NOT_FOUND,
                "MANIFEST_UNKNOWN",
                "missing manifest reference",
            ));
        }
        return require_image_name(name).map(|name| DistributionPath::Manifest {
            name,
            reference: reference.to_string(),
        });
    }
    if let Some((name, upload_rest)) = trimmed.split_once("/blobs/uploads") {
        let upload_rest = upload_rest.trim_start_matches('/');
        if upload_rest.is_empty() {
            return require_image_name(name).map(|name| DistributionPath::StartUpload { name });
        }
        let upload_id = Uuid::parse_str(upload_rest).map_err(|_| {
            OciApiError::from_code(
                StatusCode::BAD_REQUEST,
                "BLOB_UPLOAD_UNKNOWN",
                "invalid upload id",
            )
        })?;
        return require_image_name(name).map(|name| DistributionPath::Upload { name, upload_id });
    }
    if let Some((name, digest)) = trimmed.rsplit_once("/blobs/")
        && !digest.is_empty()
        && digest != "uploads"
    {
        return require_image_name(name).map(|name| DistributionPath::Blob {
            name,
            digest: digest.to_string(),
        });
    }
    Err(OciApiError::from_code(
        StatusCode::NOT_FOUND,
        "NAME_UNKNOWN",
        format!("unknown OCI path '{rest}'"),
    ))
}

fn require_image_name(name: &str) -> Result<String, OciApiError> {
    let name = name.trim_matches('/');
    if name.is_empty() {
        return Err(OciApiError::from_code(
            StatusCode::NOT_FOUND,
            "NAME_UNKNOWN",
            "empty OCI image name",
        ));
    }
    Ok(name.to_string())
}

fn method_not_allowed() -> OciApiError {
    OciApiError::from_code(
        StatusCode::METHOD_NOT_ALLOWED,
        "UNSUPPORTED",
        "method is not supported for this OCI path",
    )
}

async fn dispatch_read(
    State(state): State<Arc<AppState>>,
    Path((repository_id, rest)): Path<(Uuid, String)>,
    method: Method,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    match parse_distribution_path(&rest)? {
        DistributionPath::Manifest { name, reference } => {
            get_manifest(&state, repository_id, &name, &reference, method).await
        }
        DistributionPath::Blob { name, digest } => {
            get_blob(&state, repository_id, &name, &digest, method).await
        }
        DistributionPath::Tags { name } => list_tags(&state, repository_id, &name).await,
        DistributionPath::StartUpload { .. } | DistributionPath::Upload { .. } => {
            Err(method_not_allowed())
        }
    }
}

async fn dispatch_write(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((repository_id, rest)): Path<(Uuid, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<DigestQuery>,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    match (method, parse_distribution_path(&rest)?) {
        (Method::PUT, DistributionPath::Manifest { name, reference }) => {
            put_manifest(&state, user, repository_id, &name, &reference, headers, body).await
        }
        (Method::POST, DistributionPath::StartUpload { name }) => {
            start_or_monolithic_upload(&state, user, repository_id, &name, query, body).await
        }
        (Method::PATCH, DistributionPath::Upload { name, upload_id }) => {
            patch_upload(user, repository_id, &name, upload_id, &body)
        }
        (Method::PUT, DistributionPath::Upload { name, upload_id }) => {
            finish_upload(&state, user, repository_id, &name, upload_id, query, body).await
        }
        _ => Err(method_not_allowed()),
    }
}

async fn version_check() -> (StatusCode, HeaderMap, Bytes) {
    oci_json(StatusCode::OK, b"{}")
}

async fn issue_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    let query = uri.query().unwrap_or("");
    let form = std::str::from_utf8(&body).unwrap_or("");
    let wants_push = token_query_requests_push(query) || token_query_requests_push(form);
    let token = match extract_bearer_token(&headers) {
        None if wants_push => {
            return Err(OciApiError::unauthorized(
                &state.public_base_url,
                "/v2/token",
            ));
        }
        None => OCI_ANONYMOUS_TOKEN.to_string(),
        Some(secret) => {
            state.authenticate_token.execute(&secret).await.map_err(|_| {
                OciApiError::unauthorized(&state.public_base_url, "/v2/token")
            })?;
            secret
        }
    };
    let payload = serde_json::json!({
        "token": token,
        "access_token": token,
        "expires_in": 3600,
    });
    let body = serde_json::to_vec(&payload).map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(oci_json(StatusCode::OK, &body))
}

/// `scope=repository:<name>:<actions>` puede repetirse. `push` exige
/// credenciales reales: un token anónimo aquí es lo que Helm/ORAS cachea
/// tras el ping y reutiliza en el POST de blobs.
fn token_query_requests_push(query: &str) -> bool {
    query.split('&').any(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if !key.eq_ignore_ascii_case("scope") {
            return false;
        }
        token_scope_includes_push(&percent_decode_www_form(value))
    })
}

fn token_scope_includes_push(scope: &str) -> bool {
    scope
        .rsplit_once(':')
        .map_or(scope, |(_, actions)| actions)
        .split(',')
        .any(|action| action.eq_ignore_ascii_case("push"))
}

fn percent_decode_www_form(value: &str) -> String {
    let plus = value.replace('+', " ");
    let bytes = plus.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) =
                (from_hex_digit(bytes[i + 1]), from_hex_digit(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

async fn get_manifest(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    reference: &str,
    method: Method,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    let repository = load_distribution_repository(state, repository_id).await?;
    let strategy = distribution_strategy(state, &repository)?;
    let document = strategy
        .get_manifest(&repository, name, reference)
        .await?;
    let mut headers = oci_headers();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&document.media_type)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_MANIFEST_MEDIA_TYPE)),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&document.body.len().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(
        DOCKER_CONTENT_DIGEST.clone(),
        HeaderValue::from_str(&document.digest)
            .unwrap_or_else(|_| HeaderValue::from_static("sha256:invalid")),
    );
    let body = if method == Method::HEAD {
        Bytes::new()
    } else {
        document.body
    };
    Ok((StatusCode::OK, headers, body))
}

async fn put_manifest(
    state: &AppState,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    repository_id: Uuid,
    name: &str,
    reference: &str,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    require_write_artifacts(&user)?;
    let repository = load_distribution_repository(state, repository_id).await?;
    let strategy = distribution_strategy(state, &repository)?;
    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or(DEFAULT_MANIFEST_MEDIA_TYPE);
    let digest = strategy
        .put_manifest(&repository, name, reference, media_type, body)
        .await?;
    let location = format!("/v2/{repository_id}/{name}/manifests/{digest}");
    let mut response_headers = oci_headers();
    response_headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("/v2/")),
    );
    response_headers.insert(
        DOCKER_CONTENT_DIGEST.clone(),
        HeaderValue::from_str(digest.as_str()).unwrap_or_else(|_| HeaderValue::from_static("sha256:invalid")),
    );
    Ok((StatusCode::CREATED, response_headers, Bytes::new()))
}

async fn get_blob(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    digest: &str,
    method: Method,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    let repository = load_distribution_repository(state, repository_id).await?;
    let strategy = distribution_strategy(state, &repository)?;
    let body = strategy.get_blob(&repository, name, digest).await?;
    let mut headers = oci_headers();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(
        DOCKER_CONTENT_DIGEST.clone(),
        HeaderValue::from_str(digest).unwrap_or_else(|_| HeaderValue::from_static("sha256:invalid")),
    );
    let body = if method == Method::HEAD {
        Bytes::new()
    } else {
        body
    };
    Ok((StatusCode::OK, headers, body))
}

#[derive(Deserialize)]
struct DigestQuery {
    digest: Option<String>,
}

async fn start_or_monolithic_upload(
    state: &AppState,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    repository_id: Uuid,
    name: &str,
    query: DigestQuery,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    require_write_artifacts(&user)?;
    let repository = load_distribution_repository(state, repository_id).await?;
    if let Some(digest) = query.digest {
        let strategy = distribution_strategy(state, &repository)?;
        strategy.put_blob(&repository, &digest, body).await?;
        return Ok(blob_created(repository_id, name, &digest));
    }

    let upload_id = Uuid::now_v7();
    uploads().insert(
        upload_id,
        UploadSession {
            repository_id,
            name: name.to_string(),
            body: Vec::new(),
        },
    );
    Ok(upload_accepted(repository_id, name, upload_id, 0))
}

fn patch_upload(
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    repository_id: Uuid,
    name: &str,
    upload_id: Uuid,
    body: &Bytes,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    require_write_artifacts(&user)?;
    let size = uploads().append(upload_id, repository_id, name, body)?;
    Ok(upload_accepted(repository_id, name, upload_id, size))
}

async fn finish_upload(
    state: &AppState,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    repository_id: Uuid,
    name: &str,
    upload_id: Uuid,
    query: DigestQuery,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    require_write_artifacts(&user)?;
    let digest = query.digest.ok_or_else(|| {
        OciApiError::from_code(
            StatusCode::BAD_REQUEST,
            "DIGEST_INVALID",
            "missing digest query parameter",
        )
    })?;
    let mut session = uploads().take(upload_id, repository_id, name)?;
    session.extend_from_slice(&body);
    let repository = load_distribution_repository(state, repository_id).await?;
    let strategy = distribution_strategy(state, &repository)?;
    strategy
        .put_blob(&repository, &digest, Bytes::from(session))
        .await?;
    Ok(blob_created(repository_id, name, &digest))
}

async fn list_tags(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
) -> Result<(StatusCode, HeaderMap, Bytes), OciApiError> {
    let repository = load_distribution_repository(state, repository_id).await?;
    let strategy = distribution_strategy(state, &repository)?;
    let tags = strategy.list_tags(&repository, name).await?;
    let payload = serde_json::to_vec(&serde_json::json!({ "name": name, "tags": tags }))
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(oci_json(StatusCode::OK, &payload))
}

#[derive(Serialize)]
struct OciOk {
    ok: bool,
}

async fn yank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, reference)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<OciOk>), ApiError> {
    require_write_artifacts(&user)?;
    set_yanked(&state, repository_id, &name, &reference, true).await?;
    Ok((StatusCode::OK, Json(OciOk { ok: true })))
}

async fn unyank(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, name, reference)): Path<(Uuid, String, String)>,
) -> Result<(StatusCode, Json<OciOk>), ApiError> {
    require_write_artifacts(&user)?;
    set_yanked(&state, repository_id, &name, &reference, false).await?;
    Ok((StatusCode::OK, Json(OciOk { ok: true })))
}

async fn set_yanked(
    state: &AppState,
    repository_id: Uuid,
    name: &str,
    reference: &str,
    yanked: bool,
) -> Result<(), ApiError> {
    let repository = load_distribution_repository(state, repository_id).await?;
    let strategy = distribution_strategy(state, &repository)?;
    let name = PackageName::parse(name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version =
        PackageVersion::parse(reference).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let coordinate = PackageCoordinate::new(repository.ecosystem(), name, version);
    strategy
        .set_yanked(&repository, &coordinate, yanked)
        .await?;
    Ok(())
}

fn oci_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        DOCKER_DISTRIBUTION_API_VERSION.clone(),
        HeaderValue::from_static("registry/2.0"),
    );
    headers
}

fn oci_json(status: StatusCode, body: &[u8]) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = oci_headers();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (status, headers, Bytes::from(body.to_vec()))
}

fn blob_created(repository_id: Uuid, name: &str, digest: &str) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = oci_headers();
    let location = format!("/v2/{repository_id}/{name}/blobs/{digest}");
    headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("/v2/")),
    );
    headers.insert(
        DOCKER_CONTENT_DIGEST.clone(),
        HeaderValue::from_str(digest).unwrap_or_else(|_| HeaderValue::from_static("sha256:invalid")),
    );
    (StatusCode::CREATED, headers, Bytes::new())
}

fn upload_accepted(
    repository_id: Uuid,
    name: &str,
    upload_id: Uuid,
    size: usize,
) -> (StatusCode, HeaderMap, Bytes) {
    let mut headers = oci_headers();
    let location = format!("/v2/{repository_id}/{name}/blobs/uploads/{upload_id}");
    headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("/v2/")),
    );
    headers.insert(
        DOCKER_UPLOAD_UUID.clone(),
        HeaderValue::from_str(&upload_id.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    if size > 0 {
        headers.insert(
            header::RANGE,
            HeaderValue::from_str(&format!("0-{}", size.saturating_sub(1)))
                .unwrap_or_else(|_| HeaderValue::from_static("0-0")),
        );
    }
    (StatusCode::ACCEPTED, headers, Bytes::new())
}

struct UploadSession {
    repository_id: Uuid,
    name: String,
    body: Vec<u8>,
}

struct UploadStore {
    sessions: Mutex<HashMap<Uuid, UploadSession>>,
}

impl UploadStore {
    fn insert(&self, id: Uuid, session: UploadSession) {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, session);
    }

    fn append(
        &self,
        id: Uuid,
        repository_id: Uuid,
        name: &str,
        chunk: &[u8],
    ) -> Result<usize, OciApiError> {
        let mut sessions = self.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = sessions.get_mut(&id).ok_or_else(|| {
            OciApiError::from_code(StatusCode::NOT_FOUND, "BLOB_UPLOAD_UNKNOWN", "unknown upload")
        })?;
        if session.repository_id != repository_id || session.name != name {
            return Err(OciApiError::from_code(
                StatusCode::BAD_REQUEST,
                "NAME_INVALID",
                "upload does not belong to this repository",
            ));
        }
        session.body.extend_from_slice(chunk);
        Ok(session.body.len())
    }

    fn take(
        &self,
        id: Uuid,
        repository_id: Uuid,
        name: &str,
    ) -> Result<Vec<u8>, OciApiError> {
        let mut sessions = self.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = sessions.remove(&id).ok_or_else(|| {
            OciApiError::from_code(StatusCode::NOT_FOUND, "BLOB_UPLOAD_UNKNOWN", "unknown upload")
        })?;
        if session.repository_id != repository_id || session.name != name {
            return Err(OciApiError::from_code(
                StatusCode::BAD_REQUEST,
                "NAME_INVALID",
                "upload does not belong to this repository",
            ));
        }
        Ok(session.body)
    }
}

fn uploads() -> &'static UploadStore {
    static STORE: OnceLock<UploadStore> = OnceLock::new();
    STORE.get_or_init(|| UploadStore {
        sessions: Mutex::new(HashMap::new()),
    })
}

/// Error HTTP con el JSON de errores del Distribution Spec.
struct OciApiError {
    status: StatusCode,
    body: Bytes,
    challenges: Vec<HeaderValue>,
}

impl OciApiError {
    fn from_code(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        let payload = serde_json::json!({
            "errors": [{ "code": code, "message": message.into() }]
        });
        Self {
            status,
            body: Bytes::from(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
            challenges: Vec::new(),
        }
    }

    fn unauthorized(public_base_url: &str, path: &str) -> Self {
        let mut error = Self::from_code(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "authentication required",
        );
        error.challenges.push(oci_bearer_challenge(public_base_url, path));
        error
            .challenges
            .push(HeaderValue::from_static(r#"Basic realm="ferrobox""#));
        error
    }
}

impl axum::response::IntoResponse for OciApiError {
    fn into_response(self) -> axum::response::Response {
        let mut headers = oci_headers();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        for challenge in self.challenges {
            headers.append(header::WWW_AUTHENTICATE, challenge);
        }
        (self.status, headers, self.body).into_response()
    }
}

impl From<ApiError> for OciApiError {
    fn from(err: ApiError) -> Self {
        let (status, code) = match &err {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "DENIED"),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "DENIED"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "DENIED"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NAME_UNKNOWN"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "UNKNOWN"),
        };
        Self::from_code(status, code, err_message(&err))
    }
}

fn err_message(err: &ApiError) -> String {
    match err {
        ApiError::BadRequest(message)
        | ApiError::Unauthorized(message)
        | ApiError::Forbidden(message)
        | ApiError::Conflict(message)
        | ApiError::NotFound(message)
        | ApiError::Internal(message) => message.clone(),
    }
}

impl From<ferrobox_application::packaging::PackagingError> for OciApiError {
    fn from(err: ferrobox_application::packaging::PackagingError) -> Self {
        use ferrobox_application::packaging::PackagingError;
        match err {
            PackagingError::FileNotFound(digest) => {
                Self::from_code(StatusCode::NOT_FOUND, "BLOB_UNKNOWN", digest)
            }
            PackagingError::PackageNotFound(name) => {
                Self::from_code(StatusCode::NOT_FOUND, "MANIFEST_UNKNOWN", name)
            }
            PackagingError::VersionNotFound(coordinate) => Self::from_code(
                StatusCode::NOT_FOUND,
                "MANIFEST_UNKNOWN",
                coordinate.to_string(),
            ),
            PackagingError::InvalidPayload(message) => {
                let code = if message.contains("digest") {
                    "DIGEST_INVALID"
                } else {
                    "NAME_INVALID"
                };
                Self::from_code(StatusCode::BAD_REQUEST, code, message)
            }
            PackagingError::ReadOnlyRepository => Self::from_code(
                StatusCode::METHOD_NOT_ALLOWED,
                "UNSUPPORTED",
                err.to_string(),
            ),
            PackagingError::EcosystemMismatch { .. } => {
                Self::from_code(StatusCode::BAD_REQUEST, "NAME_UNKNOWN", err.to_string())
            }
            other => Self::from(ApiError::from(other)),
        }
    }
}

impl From<ferrobox_application::get_repository::GetRepositoryError> for OciApiError {
    fn from(err: ferrobox_application::get_repository::GetRepositoryError) -> Self {
        Self::from(ApiError::from(err))
    }
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
    use ferrobox_application::manage_users::{
        ChangeUserRoleUseCase, CreateUserUseCase, DeleteUserUseCase, ListUsersUseCase,
    };
    use ferrobox_application::packaging::PackagingRegistry;
    use ferrobox_application::packaging::cargo::CargoPackagingStrategy;
    use ferrobox_application::packaging::npm::NpmPackagingStrategy;
    use ferrobox_application::packaging::oci::{DEFAULT_MANIFEST_MEDIA_TYPE, OciPackagingStrategy};
    use ferrobox_application::packaging::pypi::PypiPackagingStrategy;
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
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::AppState;
    use super::{DistributionPath, parse_distribution_path, token_query_requests_push};

    struct Fixture {
        app: Router,
        repo_id: Uuid,
        developer_token: String,
        state: Arc<AppState>,
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
            )))
            .register(Arc::new(OciPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                repository_store.clone(),
                http_client.clone(),
            )))
            .register(Arc::new(OciPackagingStrategy::for_ecosystem(
                PackageEcosystem::Helm,
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                repository_store.clone(),
                http_client.clone(),
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
                storage.clone(),
            ),
            packaging,
            assays: ferrobox_application::assay::AssayService::new(
                Arc::new(InMemoryAssayStore::default()),
                package_index_store.clone(),
                repository_store.clone(),
                storage.clone(),
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
                RepositoryName::parse("oci-releases").unwrap(),
                PackageEcosystem::Oci,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        Fixture {
            app: crate::build_router(state.clone()),
            repo_id: repo_id.into(),
            developer_token,
            state,
            http: http_client,
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

    fn digest_of(content: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(content))
    }

    #[tokio::test]
    async fn version_check_is_public_so_oras_does_not_cache_anonymous() {
        let fx = fixture().await;
        let (status, headers, body) = send(
            fx.app,
            Request::builder().uri("/v2/").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"{}");
        assert!(headers.get("www-authenticate").is_none());
        assert_eq!(
            headers.get("docker-distribution-api-version").unwrap(),
            "registry/2.0"
        );
    }

    #[tokio::test]
    async fn token_endpoint_exchanges_basic_for_bearer() {
        let fx = fixture().await;
        let (status, _, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri("/v2/token")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["token"], "anonymous");

        let pull_scope = format!(
            "/v2/token?service=ferrobox&scope=repository:{}/demo:pull",
            fx.repo_id
        );
        let (status, _, body) = send(
            fx.app.clone(),
            Request::builder().uri(&pull_scope).body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["token"], "anonymous");

        let push_scope = format!(
            "/v2/token?service=ferrobox&scope=repository:{}/demo:pull,push",
            fx.repo_id
        );
        let (status, headers, _) = send(
            fx.app.clone(),
            Request::builder()
                .uri(&push_scope)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let challenge = headers
            .get_all("www-authenticate")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        assert!(challenge.contains("/v2/token"));

        let basic = BASE64.encode(format!("__token__:{}", fx.developer_token));
        let (status, _, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(&push_scope)
                .header("Authorization", format!("Basic {basic}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["token"], fx.developer_token);

        let (status, _, body) = send(
            fx.app,
            Request::builder()
                .uri("/v2/")
                .header("Authorization", format!("Bearer {}", fx.developer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"{}");
    }

    #[tokio::test]
    async fn anonymous_bearer_cannot_start_a_blob_upload() {
        let fx = fixture().await;
        let (status, headers, _) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri(format!("/v2/{}/demo/blobs/uploads/", fx.repo_id))
                .header("Authorization", "Bearer anonymous")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let challenge = headers
            .get_all("www-authenticate")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        assert!(challenge.contains("/v2/token"));
        assert!(challenge.contains(&format!("repository:{}/demo:pull,push", fx.repo_id)));
    }

    #[tokio::test]
    async fn blob_upload_without_a_token_is_unauthorized() {
        let fx = fixture().await;
        let (status, headers, _) = send(
            fx.app,
            Request::builder()
                .method("POST")
                .uri(format!("/v2/{}/demo/blobs/uploads/", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let challenge = headers
            .get_all("www-authenticate")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        assert!(challenge.contains("/v2/token"));
        assert!(challenge.contains(&format!("repository:{}/demo:pull,push", fx.repo_id)));
    }

    #[tokio::test]
    async fn push_then_pull_manifest_and_blob() {
        let fx = fixture().await;
        let token = format!("Bearer {}", fx.developer_token);
        let config = br#"{"architecture":"amd64"}"#;
        let layer = b"layer-bytes";
        let config_digest = digest_of(config);
        let layer_digest = digest_of(layer);

        for (digest, content) in [
            (config_digest.as_str(), config.as_slice()),
            (layer_digest.as_str(), layer.as_slice()),
        ] {
            let (status, headers, _) = send(
                fx.app.clone(),
                Request::builder()
                    .method("POST")
                    .uri(format!("/v2/{}/demo/blobs/uploads/", fx.repo_id))
                    .header("Authorization", token.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::ACCEPTED);
            let location = headers.get("location").unwrap().to_str().unwrap();
            let (status, _, _) = send(
                fx.app.clone(),
                Request::builder()
                    .method("PUT")
                    .uri(format!("{location}?digest={digest}"))
                    .header("Authorization", token.clone())
                    .body(Body::from(content.to_vec()))
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
        }

        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": config_digest,
                "size": config.len()
            },
            "layers": [{
                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                "digest": layer_digest,
                "size": layer.len()
            }]
        });
        let (status, headers, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/v2/{}/demo/manifests/latest", fx.repo_id))
                .header("Authorization", token)
                .header("Content-Type", DEFAULT_MANIFEST_MEDIA_TYPE)
                .body(Body::from(serde_json::to_vec(&manifest).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(headers.get("docker-content-digest").is_some());

        let (status, headers, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/v2/{}/demo/manifests/latest", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get("content-type").unwrap(),
            DEFAULT_MANIFEST_MEDIA_TYPE
        );
        let pulled: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pulled["config"]["digest"], config_digest);

        let (status, _, body) = send(
            fx.app.clone(),
            Request::builder()
                .uri(format!("/v2/{}/demo/blobs/{layer_digest}", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), layer);

        let (status, _, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/v2/{}/demo/tags/list", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let tags: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(tags["tags"][0], "latest");
    }

    #[tokio::test]
    async fn mirror_pulls_a_manifest_from_upstream() {
        let fx = fixture().await;
        let mirror_id = fx
            .state
            .create_repository
            .execute(
                RepositoryName::parse("oci-hub").unwrap(),
                PackageEcosystem::Oci,
                CreateRepositoryKind::Mirror {
                    upstream: "https://registry-1.docker.io".to_string(),
                },
            )
            .await
            .unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
            "layers": []
        });
        let body = Bytes::from(serde_json::to_vec(&manifest).unwrap());
        fx.http.stub(
            "https://registry-1.docker.io/v2/library/alpine/manifests/latest",
            200,
            body.clone(),
        );

        let (status, headers, pulled) = send(
            fx.app,
            Request::builder()
                .uri(format!("/v2/{mirror_id}/alpine/manifests/latest"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get("content-type").unwrap(),
            DEFAULT_MANIFEST_MEDIA_TYPE
        );
        assert_eq!(pulled, body);
    }

    #[test]
    fn parse_distribution_path_accepts_nested_names() {
        assert_eq!(
            parse_distribution_path("bitnami/nginx/manifests/latest").ok(),
            Some(DistributionPath::Manifest {
                name: "bitnami/nginx".to_string(),
                reference: "latest".to_string(),
            })
        );
        assert_eq!(
            parse_distribution_path("bitnami/nginx/blobs/sha256:abc").ok(),
            Some(DistributionPath::Blob {
                name: "bitnami/nginx".to_string(),
                digest: "sha256:abc".to_string(),
            })
        );
        assert_eq!(
            parse_distribution_path("demo/blobs/uploads/").ok(),
            Some(DistributionPath::StartUpload {
                name: "demo".to_string(),
            })
        );
        let upload_id = Uuid::now_v7();
        assert_eq!(
            parse_distribution_path(&format!("demo/blobs/uploads/{upload_id}")).ok(),
            Some(DistributionPath::Upload {
                name: "demo".to_string(),
                upload_id,
            })
        );
        assert_eq!(
            parse_distribution_path("demo/tags/list").ok(),
            Some(DistributionPath::Tags {
                name: "demo".to_string(),
            })
        );
    }

    #[test]
    fn token_query_detects_push_action_in_scope() {
        assert!(!token_query_requests_push(""));
        assert!(!token_query_requests_push("service=ferrobox"));
        assert!(!token_query_requests_push(
            "service=ferrobox&scope=repository:demo:pull"
        ));
        assert!(token_query_requests_push(
            "service=ferrobox&scope=repository:demo:pull,push"
        ));
        assert!(token_query_requests_push(
            "scope=repository%3A01a005cb-1139-7921-b251-acffa5ad2cb1%2Fdemo%3Apull%2Cpush"
        ));
        assert!(token_query_requests_push(
            "scope=repository:a:pull&scope=repository:b:pull,push"
        ));
    }

    #[tokio::test]
    async fn push_then_pull_a_nested_image_name() {
        let fx = fixture().await;
        let token = format!("Bearer {}", fx.developer_token);
        let layer = b"nested-layer";
        let layer_digest = digest_of(layer);
        let (status, headers, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/v2/{}/bitnami/nginx/blobs/uploads/", fx.repo_id))
                .header("Authorization", token.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let location = headers.get("location").unwrap().to_str().unwrap();
        let (status, _, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("{location}?digest={layer_digest}"))
                .header("Authorization", token.clone())
                .body(Body::from(layer.to_vec()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": DEFAULT_MANIFEST_MEDIA_TYPE,
            "layers": [{
                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                "digest": layer_digest,
                "size": layer.len()
            }]
        });
        let (status, _, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/v2/{}/bitnami/nginx/manifests/latest", fx.repo_id))
                .header("Authorization", token)
                .header("Content-Type", DEFAULT_MANIFEST_MEDIA_TYPE)
                .body(Body::from(serde_json::to_vec(&manifest).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, _, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/v2/{}/bitnami/nginx/manifests/latest", fx.repo_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let pulled: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pulled["layers"][0]["digest"], layer_digest);
    }

    #[tokio::test]
    async fn helm_repository_serves_the_distribution_api() {
        let fx = fixture().await;
        let helm_id = fx
            .state
            .create_repository
            .execute(
                RepositoryName::parse("charts-local").unwrap(),
                PackageEcosystem::Helm,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let token = format!("Bearer {}", fx.developer_token);
        let chart = b"chart-bytes";
        let digest = digest_of(chart);
        let (status, headers, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/v2/{helm_id}/demo/blobs/uploads/"))
                .header("Authorization", token.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let location = headers.get("location").unwrap().to_str().unwrap();
        let (status, _, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("{location}?digest={digest}"))
                .header("Authorization", token.clone())
                .body(Body::from(chart.to_vec()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.cncf.helm.chart.manifest.v1+json",
            "layers": [{
                "mediaType": "application/vnd.cncf.helm.chart.content.v1.tar+gzip",
                "digest": digest,
                "size": chart.len()
            }]
        });
        let (status, _, _) = send(
            fx.app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/v2/{helm_id}/demo/manifests/0.1.0"))
                .header("Authorization", token)
                .header("Content-Type", "application/vnd.cncf.helm.chart.manifest.v1+json")
                .body(Body::from(serde_json::to_vec(&manifest).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, _, body) = send(
            fx.app,
            Request::builder()
                .uri(format!("/v2/{helm_id}/demo/manifests/0.1.0"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let pulled: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pulled["layers"][0]["digest"], digest);
    }
}
