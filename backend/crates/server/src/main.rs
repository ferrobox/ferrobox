//! Punto de composición de `FerroBox`: el único lugar del proyecto que
//! conoce todas las implementaciones concretas de cada puerto.

mod admission;
mod artifacts;
mod assays;
mod audit;
mod auth;
mod auth_extract;
mod authz;
mod cargo_registry;
mod conan_registry;
mod config;
mod dto;
mod error;
mod go_registry;
mod groups;
mod maven_registry;
mod npm_registry;
mod nuget_registry;
mod oci_registry;
mod oidc;
mod pypi_registry;
mod quota;
mod replica;
mod repositories;
mod retention;
mod search;
mod settings;
mod users;
mod webhooks;

use std::net::SocketAddr;
use std::sync::Arc;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, patch, post, put};
use config::Config;
use ferrobox_adapter_http::ReqwestHttpClient;
use ferrobox_adapter_postgres::admission_store::PostgresAdmissionStore;
use ferrobox_adapter_postgres::api_token_store::PostgresApiTokenStore;
use ferrobox_adapter_postgres::artifact_store::PostgresArtifactStore;
use ferrobox_adapter_postgres::assay_store::PostgresAssayStore;
use ferrobox_adapter_postgres::audit_store::PostgresAuditStore;
use ferrobox_adapter_postgres::group_store::PostgresGroupStore;
use ferrobox_adapter_postgres::package_index_store::PostgresPackageIndexStore;
use ferrobox_adapter_postgres::quota_store::PostgresQuotaStore;
use ferrobox_adapter_postgres::replica_store::PostgresReplicaStore;
use ferrobox_adapter_postgres::repository_store::PostgresRepositoryStore;
use ferrobox_adapter_postgres::retention_store::PostgresRetentionStore;
use ferrobox_adapter_postgres::user_store::PostgresUserStore;
use ferrobox_adapter_postgres::webhook_store::PostgresWebhookStore;
use ferrobox_adapter_s3_storage::S3StorageAdapter;
use ferrobox_application::admission::AdmissionService;
use ferrobox_application::assay::AssayService;
use ferrobox_application::audit::AuditService;
use ferrobox_application::authenticate_token::AuthenticateTokenUseCase;
use ferrobox_application::bootstrap_admin::{BootstrapAdminOutcome, BootstrapAdminUseCase};
use ferrobox_application::change_password::ChangePasswordUseCase;
use ferrobox_application::create_repository::CreateRepositoryUseCase;
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
use ferrobox_application::packaging::conan::ConanPackagingStrategy;
use ferrobox_application::packaging::golang::GoPackagingStrategy;
use ferrobox_application::packaging::maven::MavenPackagingStrategy;
use ferrobox_application::packaging::npm::NpmPackagingStrategy;
use ferrobox_application::packaging::nuget::NugetPackagingStrategy;
use ferrobox_application::packaging::oci::OciPackagingStrategy;
use ferrobox_application::packaging::pypi::PypiPackagingStrategy;
use ferrobox_application::promote_package::PromotePackageUseCase;
use ferrobox_application::publish_artifact::PublishArtifactUseCase;
use ferrobox_application::quota::QuotaService;
use ferrobox_application::replica::ReplicaService;
use ferrobox_application::retention::RetentionService;
use ferrobox_application::search_packages::SearchPackagesUseCase;
use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
use ferrobox_application::webhooks::WebhookService;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::user::Username;
use sqlx::postgres::PgPoolOptions;
use tower_http::services::{ServeDir, ServeFile};

/// Estado compartido por todos los manejadores de rutas.
struct AppState {
    create_repository: CreateRepositoryUseCase,
    list_repositories: ListRepositoriesUseCase,
    get_repository: GetRepositoryUseCase,
    update_alloy_members: UpdateAlloyMembersUseCase,
    publish_artifact: PublishArtifactUseCase,
    download_artifact: DownloadArtifactUseCase,
    list_repository_artifacts: ListRepositoryArtifactsUseCase,
    delete_repository: DeleteRepositoryUseCase,
    delete_artifact: DeleteArtifactUseCase,
    promote_package: PromotePackageUseCase,
    repository_bundle: ferrobox_application::repository_bundle::RepositoryBundleService,
    replica: ReplicaService,
    packaging: PackagingRegistry,
    assays: AssayService,
    admission: AdmissionService,
    retention: RetentionService,
    quota: QuotaService,
    search_packages: SearchPackagesUseCase,
    public_base_url: String,
    login: LoginUseCase,
    change_password: ChangePasswordUseCase,
    authenticate_token: AuthenticateTokenUseCase,
    create_api_token: CreateApiTokenUseCase,
    list_api_tokens: ListApiTokensUseCase,
    revoke_api_token: RevokeApiTokenUseCase,
    create_user: CreateUserUseCase,
    list_users: ListUsersUseCase,
    delete_user: DeleteUserUseCase,
    change_user_role: ChangeUserRoleUseCase,
    reset_user_password: ResetUserPasswordUseCase,
    groups: GroupService,
    webhooks: WebhookService,
    audit: AuditService,
    oidc: Option<ferrobox_application::oidc::OidcLoginService>,
}

#[tokio::main]
async fn main() {
    let dotenv_files = config::load_dotenv();
    let config = Config::from_env().expect("invalid configuration");
    if !dotenv_files.is_empty() {
        eprintln!(
            "ferrobox: loaded {}",
            dotenv_files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    eprintln!(
        "ferrobox: s3 endpoint={} bucket={}",
        config.s3_endpoint_url, config.s3_bucket
    );

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("failed to connect to PostgreSQL");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("failed to run database migrations");

    let s3_client = build_s3_client(&config);

    let repository_store = Arc::new(PostgresRepositoryStore::new(pool.clone()));
    let artifact_store = Arc::new(PostgresArtifactStore::new(pool.clone()));
    let package_index_store = Arc::new(PostgresPackageIndexStore::new(pool.clone()));
    let assay_store = Arc::new(PostgresAssayStore::new(pool.clone()));
    let retention_store = Arc::new(PostgresRetentionStore::new(pool.clone()));
    let admission_store = Arc::new(PostgresAdmissionStore::new(pool.clone()));
    let audit_store = Arc::new(PostgresAuditStore::new(pool.clone()));
    let quota_store = Arc::new(PostgresQuotaStore::new(pool.clone()));
    let user_store = Arc::new(PostgresUserStore::new(pool.clone()));
    let group_store = Arc::new(PostgresGroupStore::new(pool.clone()));
    let api_token_store = Arc::new(PostgresApiTokenStore::new(pool.clone()));
    let webhook_store = Arc::new(PostgresWebhookStore::new(pool.clone()));
    let replica_store = Arc::new(PostgresReplicaStore::new(pool));
    let storage = Arc::new(S3StorageAdapter::new(s3_client, config.s3_bucket.clone()));
    storage.ensure_reachable().await.unwrap_or_else(|err| {
        panic!(
            "S3 bucket '{}' is not reachable at {}: {err}. \
             For host cargo run, set S3_ENDPOINT_URL=http://127.0.0.1:3900 and use the same \
             S3_BUCKET / S3_ACCESS_KEY_ID / S3_SECRET_ACCESS_KEY as infra/.env (compose default bucket: ferrobox).",
            config.s3_bucket, config.s3_endpoint_url
        );
    });
    let http_client = Arc::new(ReqwestHttpClient::new());

    bootstrap_admin(&config, user_store.clone()).await;

    let state = Arc::new(build_app_state(
        &config,
        repository_store,
        artifact_store,
        package_index_store,
        assay_store,
        retention_store,
        admission_store,
        audit_store,
        quota_store,
        user_store,
        group_store,
        api_token_store,
        webhook_store,
        replica_store,
        storage,
        http_client,
    ));

    let app = build_router_with_frontend(state, config.frontend_dir.as_deref());

    let addr: SocketAddr = config.bind_address.parse().expect("invalid bind address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to address");

    println!("FerroBox escuchando en http://{addr}");

    axum::serve(listener, app).await.expect("server error");
}

fn protocol_public_router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(cargo_registry::public_router())
        .merge(npm_registry::public_router())
        .merge(pypi_registry::public_router())
        .merge(oci_registry::public_router())
        .merge(conan_registry::public_router())
        .merge(maven_registry::public_router())
        .merge(nuget_registry::public_router())
        .merge(go_registry::public_router())
}

fn protocol_write_router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(cargo_registry::write_router())
        .merge(npm_registry::write_router())
        .merge(pypi_registry::write_router())
        .merge(oci_registry::write_router())
        .merge(conan_registry::write_router())
        .merge(maven_registry::write_router())
        .merge(nuget_registry::write_router())
        .merge(go_registry::write_router())
}

#[allow(clippy::too_many_lines)]
fn admin_protected_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/me", get(auth::me))
        .route("/auth/me/groups", get(groups::my_groups))
        .route("/auth/password", post(auth::change_password))
        .route("/settings", get(settings::get_settings))
        .route(
            "/auth/tokens",
            get(auth::list_tokens).post(auth::create_token),
        )
        .route("/auth/tokens/{token_id}", delete(auth::revoke_token))
        .route("/audit", get(audit::list_events))
        .route("/users", get(users::list_users).post(users::create_user))
        .route("/users/robots", post(users::create_robot))
        .route(
            "/users/{user_id}",
            patch(users::update_user_role).delete(users::delete_user),
        )
        .route(
            "/users/{user_id}/password",
            post(users::reset_user_password),
        )
        .route(
            "/groups",
            get(groups::list_groups).post(groups::create_group),
        )
        .route(
            "/groups/{group_id}",
            get(groups::get_group).delete(groups::delete_group),
        )
        .route("/groups/{group_id}/members", put(groups::set_members))
        .route(
            "/groups/{group_id}/repositories",
            put(groups::set_repositories),
        )
        .route(
            "/repositories",
            post(repositories::create_repository).get(repositories::list_repositories),
        )
        .route(
            "/repositories/{repository_id}",
            get(repositories::get_repository)
                .patch(repositories::update_alloy_members)
                .delete(repositories::delete_repository),
        )
        .route(
            "/repositories/{repository_id}/access",
            get(groups::get_repository_access).put(groups::set_repository_access),
        )
        .route(
            "/repositories/{repository_id}/artifacts",
            post(artifacts::publish_artifact).get(artifacts::list_repository_artifacts),
        )
        .route(
            "/repositories/{repository_id}/artifacts/{artifact_id}",
            delete(artifacts::delete_artifact),
        )
        .route(
            "/repositories/{repository_id}/promote",
            post(artifacts::promote_package),
        )
        .route(
            "/repositories/{repository_id}/prefetch",
            post(artifacts::prefetch_package),
        )
        .route(
            "/repositories/{repository_id}/export",
            get(artifacts::export_repository),
        )
        .route(
            "/repositories/{repository_id}/import",
            post(artifacts::import_repository).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/repositories/{repository_id}/replica",
            get(replica::get_policy).put(replica::save_policy),
        )
        .route(
            "/repositories/{repository_id}/replica/push",
            post(replica::push_now).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/repositories/{repository_id}/schedule",
            put(repositories::set_mirror_schedule),
        )
        .route(
            "/artifacts/{artifact_id}",
            get(artifacts::download_artifact),
        )
        .route("/search", get(search::search_packages))
        .route("/assays", get(assays::list_all))
        .route("/assays/rerun", post(assays::rerun_all))
        .route("/assays/{assay_id}", get(assays::get_by_id))
        .route("/assays/{assay_id}/sbom", get(assays::download_sbom))
        .route(
            "/repositories/{repository_id}/assays",
            get(assays::list_for_repository).post(assays::run),
        )
        .route(
            "/repositories/{repository_id}/assay",
            get(assays::get_or_run),
        )
        .route(
            "/repositories/{repository_id}/admission",
            get(admission::get_policy).put(admission::save_policy),
        )
        .route(
            "/repositories/{repository_id}/admission/dry-run",
            post(admission::dry_run),
        )
        .route(
            "/repositories/{repository_id}/admission/events",
            get(admission::list_events),
        )
        .route(
            "/repositories/{repository_id}/quota",
            get(quota::get_quota).put(quota::save_quota),
        )
        .route(
            "/repositories/{repository_id}/retention",
            get(retention::get_policy).put(retention::save_policy),
        )
        .route(
            "/repositories/{repository_id}/retention/dry-run",
            post(retention::dry_run),
        )
        .route(
            "/repositories/{repository_id}/retention/apply",
            post(retention::apply),
        )
        .route(
            "/repositories/{repository_id}/gc",
            post(retention::collect_garbage),
        )
        .route(
            "/repositories/{repository_id}/webhooks",
            get(webhooks::list_webhooks).post(webhooks::create_webhook),
        )
        .route(
            "/repositories/{repository_id}/webhooks/{webhook_id}",
            put(webhooks::update_webhook).delete(webhooks::delete_webhook),
        )
        .route(
            "/repositories/{repository_id}/webhooks/{webhook_id}/deliveries",
            get(webhooks::list_deliveries),
        )
        .route(
            "/repositories/{repository_id}/webhooks/{webhook_id}/ping",
            post(webhooks::ping_webhook),
        )
        .route("/gc/dry-run", post(retention::dry_run_garbage_collection))
        .route("/gc", post(retention::collect_garbage_all))
}

fn with_auth(router: Router<Arc<AppState>>, state: &Arc<AppState>) -> Router<Arc<AppState>> {
    router.route_layer(middleware::from_fn_with_state(
        state.clone(),
        auth_extract::require_auth,
    ))
}

fn build_api_router(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(auth::login))
        .route("/auth/oidc", get(oidc::status))
        .route("/auth/oidc/start", get(oidc::start))
        .route("/auth/oidc/callback", get(oidc::callback))
        .route("/auth/oidc/logout", get(oidc::logout))
        .merge(protocol_public_router());
    let protected = with_auth(
        admin_protected_router().merge(protocol_write_router()),
        state,
    );
    public.merge(protected)
}

fn spa_service(dir: &str) -> ServeDir<ServeFile> {
    let index = std::path::Path::new(dir).join("index.html");
    ServeDir::new(dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index))
}

#[cfg(test)]
fn build_router(state: Arc<AppState>) -> axum::Router {
    build_router_with_frontend(state, None)
}

fn build_router_with_frontend(state: Arc<AppState>, frontend_dir: Option<&str>) -> axum::Router {
    let api = build_api_router(&state);
    let router = match frontend_dir {
        Some(dir) => {
            let protocols =
                protocol_public_router().merge(with_auth(protocol_write_router(), &state));
            Router::new()
                .route("/health", get(health))
                .merge(protocols)
                .nest("/api", api)
                .fallback_service(spa_service(dir))
        }
        None => api,
    };
    router.with_state(state)
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_app_state(
    config: &Config,
    repository_store: Arc<PostgresRepositoryStore>,
    artifact_store: Arc<PostgresArtifactStore>,
    package_index_store: Arc<PostgresPackageIndexStore>,
    assay_store: Arc<PostgresAssayStore>,
    retention_store: Arc<PostgresRetentionStore>,
    admission_store: Arc<PostgresAdmissionStore>,
    audit_store: Arc<PostgresAuditStore>,
    quota_store: Arc<PostgresQuotaStore>,
    user_store: Arc<PostgresUserStore>,
    group_store: Arc<PostgresGroupStore>,
    api_token_store: Arc<PostgresApiTokenStore>,
    webhook_store: Arc<PostgresWebhookStore>,
    replica_store: Arc<PostgresReplicaStore>,
    storage: Arc<S3StorageAdapter>,
    http_client: Arc<ReqwestHttpClient>,
) -> AppState {
    let webhooks =
        WebhookService::new(webhook_store, http_client.clone(), repository_store.clone());
    let assays = AssayService::new(
        assay_store.clone(),
        package_index_store.clone(),
        repository_store.clone(),
        storage.clone(),
        http_client.clone(),
    )
    .with_webhooks(webhooks.clone());
    let quota = QuotaService::new(
        repository_store.clone(),
        artifact_store.clone(),
        quota_store,
    );
    let list_repository_artifacts = ListRepositoryArtifactsUseCase::new(
        repository_store.clone(),
        artifact_store.clone(),
        package_index_store.clone(),
    )
    .with_cosign_verify(storage.clone(), admission_store.clone());
    let admission = AdmissionService::new(
        admission_store,
        repository_store.clone(),
        list_repository_artifacts.clone(),
    )
    .with_assays(assay_store.clone());
    let packaging = packaging_registry(
        &config.public_base_url,
        &repository_store,
        &artifact_store,
        &package_index_store,
        &storage,
        http_client.clone(),
        &assays,
        quota.clone(),
        admission.clone(),
    );
    spawn_mirror_prefetch_loop(
        repository_store.clone(),
        package_index_store.clone(),
        packaging.clone(),
    );
    let search_packages =
        SearchPackagesUseCase::new(repository_store.clone(), package_index_store.clone());
    let retention = RetentionService::new(
        repository_store.clone(),
        artifact_store.clone(),
        package_index_store.clone(),
        storage.clone(),
        assay_store,
        retention_store,
    );
    let repository_bundle = ferrobox_application::repository_bundle::RepositoryBundleService::new(
        repository_store.clone(),
        artifact_store.clone(),
        package_index_store.clone(),
        storage.clone(),
        quota.clone(),
    );
    let replica = ReplicaService::new(
        replica_store,
        repository_store.clone(),
        repository_bundle.clone(),
        http_client.clone(),
    );

    AppState {
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
        download_artifact: DownloadArtifactUseCase::new(artifact_store.clone(), storage.clone()),
        list_repository_artifacts,
        delete_repository: DeleteRepositoryUseCase::new(
            repository_store.clone(),
            artifact_store.clone(),
            package_index_store.clone(),
            storage.clone(),
        ),
        promote_package: PromotePackageUseCase::new(
            repository_store.clone(),
            artifact_store.clone(),
            storage.clone(),
            quota.clone(),
        ),
        repository_bundle,
        replica,
        delete_artifact: DeleteArtifactUseCase::new(
            repository_store.clone(),
            artifact_store,
            package_index_store,
            storage,
        ),
        packaging,
        assays,
        admission,
        retention,
        quota,
        search_packages,
        public_base_url: config.public_base_url.clone(),
        login: LoginUseCase::with_session_ttl(
            user_store.clone(),
            api_token_store.clone(),
            config.session_ttl,
        ),
        change_password: ChangePasswordUseCase::new(user_store.clone()),
        authenticate_token: AuthenticateTokenUseCase::new(
            user_store.clone(),
            api_token_store.clone(),
        ),
        create_api_token: CreateApiTokenUseCase::new(api_token_store.clone()),
        list_api_tokens: ListApiTokensUseCase::new(api_token_store.clone()),
        revoke_api_token: RevokeApiTokenUseCase::new(api_token_store.clone()),
        create_user: CreateUserUseCase::new(user_store.clone()),
        list_users: ListUsersUseCase::new(user_store.clone()),
        delete_user: DeleteUserUseCase::new(user_store.clone()),
        change_user_role: ChangeUserRoleUseCase::new(user_store.clone()),
        reset_user_password: ResetUserPasswordUseCase::new(user_store.clone()),
        groups: GroupService::new(group_store.clone(), user_store.clone(), repository_store),
        webhooks,
        audit: AuditService::new(audit_store),
        oidc: crate::oidc::service_from_config(
            config,
            http_client,
            user_store,
            group_store,
            api_token_store,
        ),
    }
}

fn spawn_mirror_prefetch_loop(
    repositories: Arc<PostgresRepositoryStore>,
    package_index: Arc<PostgresPackageIndexStore>,
    packaging: PackagingRegistry,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(err) = ferrobox_application::mirror_schedule::run_due(
                repositories.as_ref(),
                package_index.as_ref(),
                &packaging,
                chrono::Utc::now(),
            )
            .await
            {
                eprintln!("ferrobox: scheduled mirror prefetch failed: {err}");
            }
        }
    });
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn packaging_registry(
    public_base_url: &str,
    repository_store: &Arc<PostgresRepositoryStore>,
    artifact_store: &Arc<PostgresArtifactStore>,
    package_index_store: &Arc<PostgresPackageIndexStore>,
    storage: &Arc<S3StorageAdapter>,
    http_client: Arc<ReqwestHttpClient>,
    assays: &AssayService,
    quota: QuotaService,
    admission: AdmissionService,
) -> PackagingRegistry {
    PackagingRegistry::new()
        .register(Arc::new(
            CargoPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone()),
        ))
        .register(Arc::new(
            NpmPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
                public_base_url.to_string(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone()),
        ))
        .register(Arc::new(
            PypiPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
                public_base_url.to_string(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone()),
        ))
        .register(Arc::new(
            OciPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                repository_store.clone(),
                http_client.clone(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone())
            .with_admission(admission.clone()),
        ))
        .register(Arc::new(
            OciPackagingStrategy::for_ecosystem(
                PackageEcosystem::Helm,
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                repository_store.clone(),
                http_client.clone(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone())
            .with_admission(admission),
        ))
        .register(Arc::new(
            ConanPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone()),
        ))
        .register(Arc::new(
            MavenPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone()),
        ))
        .register(Arc::new(
            NugetPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client.clone(),
                repository_store.clone(),
                public_base_url.to_string(),
            )
            .with_assays(assays.clone())
            .with_quota(quota.clone()),
        ))
        .register(Arc::new(
            GoPackagingStrategy::new(
                artifact_store.clone(),
                package_index_store.clone(),
                storage.clone(),
                http_client,
                repository_store.clone(),
            )
            .with_assays(assays.clone())
            .with_quota(quota),
        ))
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
        // Garage rejects the AWS SDK default CRC32 checksums on PutObject.
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
        .build();

    S3Client::from_conf(s3_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn spa_serves_index_for_unknown_paths() {
        let dir = std::env::temp_dir().join(format!("ferrobox-spa-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><title>FerroBox</title>",
        )
        .unwrap();
        let app = Router::new().fallback_service(spa_service(dir.to_str().unwrap()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/repositories")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains("FerroBox"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
