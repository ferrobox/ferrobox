//! Rutas HTTP que implementan el subconjunto del protocolo de registro
//! de Cargo que `cargo publish`, `cargo add` y `cargo build` necesitan
//! para publicar paquetes y resolver dependencias contra `FerroBox`
//! usando el protocolo de índice disperso (*sparse index*).
//!
//! Referencia: <https://doc.rust-lang.org/cargo/reference/registries.html>.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::{Json, Router};
use bytes::Bytes;
use ferrobox_application::packaging::PackagingStrategy;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageEcosystem, PackageName, PackageVersion};
use serde::Serialize;
use uuid::Uuid;

use crate::AppState;
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
/// encontrarlas mediante los campos `dl` y `api`, así que aquí se
/// anidan bajo `.../api/v1/crates/...` únicamente por claridad. `axum`
/// resuelve los conflictos aparentes entre estas rutas literales y el
/// comodín de índice dando prioridad a la coincidencia más específica.
pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/cargo/{repository_id}/config.json",
            axum::routing::get(config_json),
        )
        .route(
            "/cargo/{repository_id}/api/v1/crates/new",
            axum::routing::put(publish),
        )
        .route(
            "/cargo/{repository_id}/api/v1/crates/{name}/{version}/download",
            axum::routing::get(download),
        )
        .route("/cargo/{repository_id}/{*rest}", axum::routing::get(index))
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

/// `GET /cargo/{repository_id}/{*rest}`: responde al protocolo de
/// índice disperso para cualquier ruta que no coincida con ninguna otra
/// ruta más específica de este router (`config.json`, `api/v1/crates/...`).
/// `cargo` siempre calcula `rest` por sí mismo aplicando las reglas de
/// fragmentación oficiales (ver
/// [`ferrobox_application::packaging::cargo::cargo_index_shard_path`]);
/// a este manejador solo le importa el último segmento de la ruta, que
/// siempre es el nombre del paquete.
async fn index(
    State(state): State<Arc<AppState>>,
    Path((repository_id, rest)): Path<(Uuid, String)>,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(&state)?;

    let package_name = rest.rsplit('/').next().unwrap_or(rest.as_str());
    let name =
        PackageName::parse(package_name).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let body = strategy.index(&repository, &name).await?;

    // El índice disperso es dinámico -- cada publicación lo cambia --
    // así que se desactiva explícitamente cualquier caché intermedia
    // (proxy del cliente, CDN...) en vez de depender del comportamiento
    // heurístico por defecto del cliente HTTP. Sin esto, `cargo publish`
    // podría, en teoría, seguir sirviendo una respuesta 404 cacheada
    // justo después de publicar una versión nueva de un paquete que
    // antes no existía.
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
/// `cargo publish` invoca con los metadatos y el contenido del `.crate`
/// codificados según su formato binario propio (ver
/// [`ferrobox_application::packaging::cargo`]).
async fn publish(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
    body: Bytes,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    let strategy = cargo_strategy(&state)?;

    strategy.publish(&repository, body).await?;

    Ok((StatusCode::OK, Json(PublishResponse::default())))
}

/// `GET /cargo/{repository_id}/api/v1/crates/{name}/{version}/download`:
/// el endpoint al que resuelve la plantilla `dl` de `config.json`
/// cuando `cargo` necesita el contenido binario de una versión concreta.
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
