//! Punto de composición de `FerroBox`: el único lugar del proyecto que
//! conoce todas las implementaciones concretas de cada puerto.

mod artifacts;
mod auth;
mod auth_extract;
mod cargo_registry;
mod config;
mod dto;
mod error;
mod repositories;

use std::net::SocketAddr;
use std::sync::Arc;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use axum::middleware;
use axum::routing::{delete, get, post};
use axum::Router;
use config::Config;
use ferrobox_adapter_postgres::api_token_store::PostgresApiTokenStore;
use ferrobox_adapter_postgres::artifact_store::PostgresArtifactStore;
use ferrobox_adapter_postgres::package_index_store::PostgresPackageIndexStore;
use ferrobox_adapter_postgres::repository_store::PostgresRepositoryStore;
use ferrobox_adapter_postgres::user_store::PostgresUserStore;
use ferrobox_adapter_s3_storage::S3StorageAdapter;
use ferrobox_application::authenticate_token::AuthenticateTokenUseCase;
use ferrobox_application::bootstrap_admin::{BootstrapAdminOutcome, BootstrapAdminUseCase};
use ferrobox_application::create_repository::CreateRepositoryUseCase;
use ferrobox_application::download_artifact::DownloadArtifactUseCase;
use ferrobox_application::get_repository::GetRepositoryUseCase;
use ferrobox_application::list_repositories::ListRepositoriesUseCase;
use ferrobox_application::list_repository_artifacts::ListRepositoryArtifactsUseCase;
use ferrobox_application::login::LoginUseCase;
use ferrobox_application::manage_api_tokens::{
    CreateApiTokenUseCase, ListApiTokensUseCase, RevokeApiTokenUseCase,
};
use ferrobox_application::packaging::PackagingRegistry;
use ferrobox_application::packaging::cargo::CargoPackagingStrategy;
use ferrobox_application::publish_artifact::PublishArtifactUseCase;
use ferrobox_domain::user::Username;
use sqlx::postgres::PgPoolOptions;

/// Estado compartido por todos los manejadores de rutas.
struct AppState {
    create_repository: CreateRepositoryUseCase,
    list_repositories: ListRepositoriesUseCase,
    get_repository: GetRepositoryUseCase,
    publish_artifact: PublishArtifactUseCase,
    download_artifact: DownloadArtifactUseCase,
    list_repository_artifacts: ListRepositoryArtifactsUseCase,
    packaging: PackagingRegistry,
    public_base_url: String,
    login: LoginUseCase,
    authenticate_token: AuthenticateTokenUseCase,
    create_api_token: CreateApiTokenUseCase,
    list_api_tokens: ListApiTokensUseCase,
    revoke_api_token: RevokeApiTokenUseCase,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let config = Config::from_env().expect("invalid configuration");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("failed to connect to PostgreSQL");

    let s3_client = build_s3_client(&config);

    let repository_store = Arc::new(PostgresRepositoryStore::new(pool.clone()));
    let artifact_store = Arc::new(PostgresArtifactStore::new(pool.clone()));
    let package_index_store = Arc::new(PostgresPackageIndexStore::new(pool.clone()));
    let user_store = Arc::new(PostgresUserStore::new(pool.clone()));
    let api_token_store = Arc::new(PostgresApiTokenStore::new(pool));
    let storage = Arc::new(S3StorageAdapter::new(s3_client, config.s3_bucket.clone()));

    bootstrap_admin(&config, user_store.clone()).await;

    let packaging = PackagingRegistry::new().register(Arc::new(CargoPackagingStrategy::new(
        artifact_store.clone(),
        package_index_store,
        storage.clone(),
    )));

    let state = Arc::new(AppState {
        create_repository: CreateRepositoryUseCase::new(repository_store.clone()),
        list_repositories: ListRepositoriesUseCase::new(repository_store.clone()),
        get_repository: GetRepositoryUseCase::new(repository_store.clone()),
        publish_artifact: PublishArtifactUseCase::new(
            repository_store,
            artifact_store.clone(),
            storage.clone(),
        ),
        download_artifact: DownloadArtifactUseCase::new(artifact_store.clone(), storage),
        list_repository_artifacts: ListRepositoryArtifactsUseCase::new(artifact_store),
        packaging,
        public_base_url: config.public_base_url.clone(),
        login: LoginUseCase::new(user_store.clone(), api_token_store.clone()),
        authenticate_token: AuthenticateTokenUseCase::new(
            user_store.clone(),
            api_token_store.clone(),
        ),
        create_api_token: CreateApiTokenUseCase::new(api_token_store.clone()),
        list_api_tokens: ListApiTokensUseCase::new(api_token_store.clone()),
        revoke_api_token: RevokeApiTokenUseCase::new(api_token_store),
    });

    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(auth::login));

    let protected = Router::new()
        .route("/auth/me", get(auth::me))
        .route(
            "/auth/tokens",
            get(auth::list_tokens).post(auth::create_token),
        )
        .route("/auth/tokens/{token_id}", delete(auth::revoke_token))
        .route(
            "/repositories",
            post(repositories::create_repository).get(repositories::list_repositories),
        )
        .route("/repositories/{repository_id}", get(repositories::get_repository))
        .route(
            "/repositories/{repository_id}/artifacts",
            post(artifacts::publish_artifact).get(artifacts::list_repository_artifacts),
        )
        .route("/artifacts/{artifact_id}", get(artifacts::download_artifact))
        .merge(cargo_registry::router())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_extract::require_auth,
        ));

    let app = public.merge(protected).with_state(state);

    let addr: SocketAddr = config.bind_address.parse().expect("invalid bind address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to address");

    println!("FerroBox escuchando en http://{addr}");

    axum::serve(listener, app).await.expect("server error");
}

async fn health() -> &'static str {
    "ok"
}

async fn bootstrap_admin(config: &Config, user_store: Arc<PostgresUserStore>) {
    let username = Username::parse(config.admin_username.clone()).unwrap_or_else(|err| {
        panic!(
            "ADMIN_USERNAME '{}' is invalid: {err}",
            config.admin_username
        )
    });

    let outcome = BootstrapAdminUseCase::new(user_store)
        .execute(username, &config.admin_password)
        .await
        .expect("failed to bootstrap admin user");

    match outcome {
        BootstrapAdminOutcome::Created => {
            println!(
                "Usuario administrador inicial creado: '{}' \
                 (cambia ADMIN_PASSWORD en producción)",
                config.admin_username
            );
        }
        BootstrapAdminOutcome::AlreadyInitialized => {}
    }
}

fn build_s3_client(config: &Config) -> S3Client {
    let credentials = Credentials::new(
        &config.s3_access_key_id,
        &config.s3_secret_access_key,
        None,
        None,
        "ferrobox-server",
    );

    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(config.s3_region.clone()))
        .endpoint_url(&config.s3_endpoint_url)
        .credentials_provider(credentials)
        .force_path_style(true)
        .build();

    S3Client::from_conf(s3_config)
}
