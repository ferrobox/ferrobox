//! Rutas HTTP que implementan el subconjunto del protocolo de registro
//! de Cargo que `cargo publish`, `cargo add` y `cargo build` necesitan
//! para publicar paquetes y resolver dependencias contra `FerroBox`
//! usando el protocolo de índice disperso (*sparse index*).
//!
//! Referencia: <https://doc.rust-lang.org/cargo/reference/registries.html>.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{get, put};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_application::packaging::cargo::cargo_index_shard_path;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{
    PackageCoordinate, PackageEcosystem, PackageName, PackageVersion,
};
use serde::Serialize;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_write_artifacts;
use crate::error::ApiError;

/// Construye el subrouter con todas las rutas del protocolo de Cargo,
/// anidadas bajo `/cargo/{repository_id}`.
///
/// Es importante que `config.json` y las rutas de índice compartan
/// exactamente la misma raíz: el protocolo de índice disperso no separa
/// "dónde vive `config.json`" de "dónde viven las entradas de índice" --
/// ambas cosas viven bajo la misma URL base, la que el cliente configura
/// como `index` de su registro (`sparse+http://.../cargo/{repository_id}/`).
/// Las rutas de publicación y descarga, en cambio, sí pueden vivir en
/// cualquier otra ruta -- `config.json` les indica al cliente dónde
/// encontrarlas mediante los campos `dl` y `api`.
///
/// El índice disperso se expone con las cuatro plantillas de
/// fragmentación oficiales (`1/{name}`, `2/{name}`, `3/{p}/{name}`,
/// `{ab}/{cd}/{name}`) y con el mismo conjunto bajo el prefijo
/// `/index/…`, para que `cargo build` / `cargo add` resuelvan
/// dependencias tanto contra la raíz del índice como contra una
/// ubicación `index/` explícita.
pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/cargo/{repository_id}/config.json", get(config_json))
        .route("/cargo/{repository_id}/api/v1/crates/new", put(publish))
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

fn cargo_strategy(state: &AppState) -> Result<Arc<dyn PackagingStrategy>, ApiError> {
    state
        .packaging
        .strategy_for(PackageEcosystem::Cargo)
        .ok_or_else(|| ApiError::Internal("no Cargo packaging strategy is registered".to_string()))
}

/// Respuesta de `GET /cargo/{repository_id}/config.json`: le indica a
/// `cargo` dónde descargar el contenido de un `.crate` (`dl`) y dónde
/// enviar peticiones de publicación (`api`).
#[derive(Serialize)]
struct RegistryConfig {
    dl: String,
    api: String,
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
/// Exige `Authorization: Bearer <token>` (también se acepta `Token`).
async fn download(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { .. }: AuthenticatedUser,
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
