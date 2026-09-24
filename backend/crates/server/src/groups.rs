//! Rutas HTTP de administración de grupos (solo rol `Admin`).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_application::manage_groups::{GroupDetail, GroupMembership, GroupSummary};
use ferrobox_domain::group::GroupName;
use ferrobox_domain::ids::{GroupId, RepositoryId, UserId};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_manage_groups;
use crate::dto::{
    CreateGroupRequest, GroupDetailResponse, GroupRepositoryGrantResponse, GroupSummaryResponse,
    MyGroupMembershipResponse, RepositoryAccessGrantResponse, SetGroupMembersRequest,
    SetGroupRepositoriesRequest, SetRepositoryAccessRequest, UserResponse,
};
use crate::error::ApiError;

impl From<&GroupSummary> for GroupSummaryResponse {
    fn from(summary: &GroupSummary) -> Self {
        Self {
            id: summary.group.id().to_string(),
            name: summary.group.name().to_string(),
            member_count: summary.member_count,
            repository_count: summary.repository_count,
            member_names: summary.member_names.clone(),
            repository_names: summary.repository_names.clone(),
        }
    }
}

impl From<&GroupMembership> for MyGroupMembershipResponse {
    fn from(membership: &GroupMembership) -> Self {
        Self {
            id: membership.group.id().to_string(),
            name: membership.group.name().to_string(),
            repositories: membership
                .repositories
                .iter()
                .map(|(repository, role)| GroupRepositoryGrantResponse {
                    repository_id: repository.id().to_string(),
                    repository_name: repository.name().to_string(),
                    role: (*role).into(),
                })
                .collect(),
        }
    }
}

impl From<&GroupDetail> for GroupDetailResponse {
    fn from(detail: &GroupDetail) -> Self {
        Self {
            id: detail.group.id().to_string(),
            name: detail.group.name().to_string(),
            members: detail.members.iter().map(UserResponse::from).collect(),
            repositories: detail
                .repositories
                .iter()
                .map(|(repository, role)| GroupRepositoryGrantResponse {
                    repository_id: repository.id().to_string(),
                    repository_name: repository.name().to_string(),
                    role: (*role).into(),
                })
                .collect(),
        }
    }
}

pub(crate) async fn list_groups(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<Vec<GroupSummaryResponse>>, ApiError> {
    require_manage_groups(&user)?;
    let groups = state.groups.list().await?;
    Ok(Json(groups.iter().map(GroupSummaryResponse::from).collect()))
}

pub(crate) async fn create_group(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Json(payload): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<GroupSummaryResponse>), ApiError> {
    require_manage_groups(&user)?;
    let name =
        GroupName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let group = state.groups.create(name).await?;
    Ok((
        StatusCode::CREATED,
        Json(GroupSummaryResponse {
            id: group.id().to_string(),
            name: group.name().to_string(),
            member_count: 0,
            repository_count: 0,
            member_names: Vec::new(),
            repository_names: Vec::new(),
        }),
    ))
}

pub(crate) async fn get_group(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(group_id): Path<Uuid>,
) -> Result<Json<GroupDetailResponse>, ApiError> {
    require_manage_groups(&user)?;
    let detail = state.groups.get(GroupId::from(group_id)).await?;
    Ok(Json(GroupDetailResponse::from(&detail)))
}

pub(crate) async fn my_groups(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<Vec<MyGroupMembershipResponse>>, ApiError> {
    let memberships = state.groups.memberships_for(user.id()).await?;
    Ok(Json(
        memberships
            .iter()
            .map(MyGroupMembershipResponse::from)
            .collect(),
    ))
}

pub(crate) async fn delete_group(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(group_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_manage_groups(&user)?;
    state.groups.delete(GroupId::from(group_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn set_members(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(group_id): Path<Uuid>,
    Json(payload): Json<SetGroupMembersRequest>,
) -> Result<StatusCode, ApiError> {
    require_manage_groups(&user)?;
    let user_ids = parse_ids(&payload.user_ids, UserId::from, "invalid user id")?;
    state
        .groups
        .set_members(GroupId::from(group_id), user_ids)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn set_repositories(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(group_id): Path<Uuid>,
    Json(payload): Json<SetGroupRepositoriesRequest>,
) -> Result<StatusCode, ApiError> {
    require_manage_groups(&user)?;
    let mut grants = Vec::with_capacity(payload.grants.len());
    for grant in payload.grants {
        let repository_id = parse_uuid(&grant.repository_id, "invalid repository id")?;
        grants.push((RepositoryId::from(repository_id), grant.role.into()));
    }
    state
        .groups
        .set_group_repositories(GroupId::from(group_id), grants)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn get_repository_access(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<Vec<RepositoryAccessGrantResponse>>, ApiError> {
    require_manage_groups(&user)?;
    let grants = state
        .groups
        .repository_grants(RepositoryId::from(repository_id))
        .await?;
    Ok(Json(
        grants
            .into_iter()
            .map(|(group, role)| RepositoryAccessGrantResponse {
                group_id: group.id().to_string(),
                group_name: group.name().to_string(),
                role: role.into(),
            })
            .collect(),
    ))
}

pub(crate) async fn set_repository_access(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<SetRepositoryAccessRequest>,
) -> Result<StatusCode, ApiError> {
    require_manage_groups(&user)?;
    let mut grants = Vec::with_capacity(payload.grants.len());
    for grant in payload.grants {
        let group_id = parse_uuid(&grant.group_id, "invalid group id")?;
        grants.push((GroupId::from(group_id), grant.role.into()));
    }
    state
        .groups
        .set_repository_groups(RepositoryId::from(repository_id), grants)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_ids<T>(
    values: &[String],
    from: fn(Uuid) -> T,
    message: &str,
) -> Result<Vec<T>, ApiError> {
    values
        .iter()
        .map(|value| parse_uuid(value, message).map(from))
        .collect()
}

fn parse_uuid(value: &str, message: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(value).map_err(|_| ApiError::BadRequest(message.to_string()))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ferrobox_application::assay::AssayService;
    use ferrobox_application::authenticate_token::AuthenticateTokenUseCase;
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
    use ferrobox_application::publish_artifact::PublishArtifactUseCase;
    use ferrobox_application::quota::QuotaService;
    use ferrobox_application::retention::RetentionService;
    use ferrobox_application::search_packages::SearchPackagesUseCase;
    use ferrobox_application::test_support::{
        InMemoryApiTokenStore, InMemoryArtifactStore, InMemoryAssayStore, InMemoryGroupStore,
        InMemoryHttpClient,
        InMemoryWebhookStore, InMemoryPackageIndexStore, InMemoryQuotaStore,
        InMemoryRepositoryStore, InMemoryRetentionStore, InMemoryStorage, InMemoryUserStore,
    };
    use ferrobox_application::update_alloy_members::UpdateAlloyMembersUseCase;
    use ferrobox_domain::api_token::ApiTokenName;
    use ferrobox_domain::user::Role;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::AppState;

    struct Fixture {
        app: Router,
        admin_token: String,
        reader_token: String,
        reader_id: String,
        repo_id: String,
    }

    #[allow(clippy::too_many_lines)]
    async fn fixture() -> Fixture {
        let repository_store = Arc::new(InMemoryRepositoryStore::default());
        let artifact_store = Arc::new(InMemoryArtifactStore::default());
        let package_index_store = Arc::new(InMemoryPackageIndexStore::default());
        let storage = Arc::new(InMemoryStorage::default());
        let user_store = Arc::new(InMemoryUserStore::default());
        let api_token_store = Arc::new(InMemoryApiTokenStore::default());
        let assay_store = Arc::new(InMemoryAssayStore::default());
        let retention_store = Arc::new(InMemoryRetentionStore::default());
        let http_client = Arc::new(InMemoryHttpClient::default());
        let quota = QuotaService::new(
            repository_store.clone(),
            artifact_store.clone(),
            Arc::new(InMemoryQuotaStore::default()),
        );
        let search_packages =
            SearchPackagesUseCase::new(repository_store.clone(), package_index_store.clone());

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
            download_artifact: DownloadArtifactUseCase::new(artifact_store.clone(), storage.clone()),
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
            packaging: PackagingRegistry::new(),
            assays: AssayService::new(
                assay_store.clone(),
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
            retention: RetentionService::new(
                repository_store.clone(),
                artifact_store,
                package_index_store,
                storage,
                assay_store,
                retention_store,
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
                user_store,
                repository_store.clone(),
            ),
            webhooks: ferrobox_application::webhooks::WebhookService::new(
                Arc::new(InMemoryWebhookStore::default()),
                http_client.clone(),
                repository_store,
            ),

        });

        let admin = state.create_user.seed("admin", Role::Admin).await.unwrap();
        let admin_token = state
            .create_api_token
            .execute(admin.id(), ApiTokenName::parse("admin").unwrap())
            .await
            .unwrap()
            .plaintext_secret;
        let reader = state
            .create_user
            .seed("reader", Role::Reader)
            .await
            .unwrap();
        let reader_token = state
            .create_api_token
            .execute(reader.id(), ApiTokenName::parse("read").unwrap())
            .await
            .unwrap()
            .plaintext_secret;

        let repo_id = state
            .create_repository
            .execute(
                ferrobox_domain::repository::RepositoryName::parse("secret-crates").unwrap(),
                ferrobox_domain::package_coordinate::PackageEcosystem::Generic,
                ferrobox_application::create_repository::CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();

        Fixture {
            app: crate::build_router(state),
            admin_token,
            reader_token,
            reader_id: reader.id().to_string(),
            repo_id: repo_id.to_string(),
        }
    }

    async fn send_json(
        app: Router,
        token: &str,
        method: &str,
        uri: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    async fn send(
        app: Router,
        token: &str,
        method: &str,
        uri: &str,
    ) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    #[tokio::test]
    async fn reader_cannot_manage_groups() {
        let fx = fixture().await;
        let (status, _) = send_json(
            fx.app,
            &fx.reader_token,
            "POST",
            "/groups",
            json!({ "name": "team-a" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn restricted_repository_is_hidden_until_the_reader_joins() {
        let fx = fixture().await;

        let (status, created) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "POST",
            "/groups",
            json!({ "name": "team-a" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let group_id = created["id"].as_str().unwrap();

        let (status, _) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "PUT",
            &format!("/groups/{group_id}/repositories"),
            json!({ "grants": [{ "repository_id": fx.repo_id, "role": "developer" }] }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _) = send(
            fx.app.clone(),
            &fx.reader_token,
            "GET",
            &format!("/repositories/{}", fx.repo_id),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, listed) = send(fx.app.clone(), &fx.reader_token, "GET", "/repositories").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 0);

        let (status, _) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "PUT",
            &format!("/groups/{group_id}/members"),
            json!({ "user_ids": [fx.reader_id] }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, repo) = send(
            fx.app.clone(),
            &fx.reader_token,
            "GET",
            &format!("/repositories/{}", fx.repo_id),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(repo["access"], "write");
        assert_eq!(repo["restricted"], true);

        let response = fx
            .app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/repositories/{}/artifacts", fx.repo_id))
                    .header("Authorization", format!("Bearer {}", fx.reader_token))
                    .body(Body::from("crate-bytes"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn group_member_does_not_list_unrestricted_repositories() {
        let fx = fixture().await;

        let (status, other) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "POST",
            "/repositories",
            json!({ "name": "other-repo", "ecosystem": "generic" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let other_id = other["id"].as_str().unwrap();

        let (status, created) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "POST",
            "/groups",
            json!({ "name": "team-a" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let group_id = created["id"].as_str().unwrap();

        let (status, _) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "PUT",
            &format!("/groups/{group_id}/members"),
            json!({ "user_ids": [fx.reader_id] }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "PUT",
            &format!("/groups/{group_id}/repositories"),
            json!({ "grants": [{ "repository_id": fx.repo_id, "role": "developer" }] }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, listed) = send(fx.app.clone(), &fx.reader_token, "GET", "/repositories").await;
        assert_eq!(status, StatusCode::OK);
        let ids: Vec<&str> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![fx.repo_id.as_str()]);

        let (status, _) = send(
            fx.app,
            &fx.reader_token,
            "GET",
            &format!("/repositories/{other_id}"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn reader_lists_own_memberships_but_cannot_manage_groups() {
        let fx = fixture().await;

        let (status, created) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "POST",
            "/groups",
            json!({ "name": "team-a" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let group_id = created["id"].as_str().unwrap();

        let (status, _) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "PUT",
            &format!("/groups/{group_id}/members"),
            json!({ "user_ids": [fx.reader_id] }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _) = send_json(
            fx.app.clone(),
            &fx.admin_token,
            "PUT",
            &format!("/groups/{group_id}/repositories"),
            json!({ "grants": [{ "repository_id": fx.repo_id, "role": "developer" }] }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, listed) = send(fx.app.clone(), &fx.admin_token, "GET", "/groups").await;
        assert_eq!(status, StatusCode::OK);
        let team = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "team-a")
            .expect("team-a");
        assert_eq!(team["member_names"], json!(["reader"]));
        assert_eq!(team["repository_names"], json!(["secret-crates"]));

        let (status, _) = send(fx.app.clone(), &fx.reader_token, "GET", "/groups").await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, memberships) =
            send(fx.app.clone(), &fx.reader_token, "GET", "/auth/me/groups").await;
        assert_eq!(status, StatusCode::OK);
        let items = memberships.as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "team-a");
        assert_eq!(items[0]["repositories"][0]["repository_name"], "secret-crates");
        assert_eq!(items[0]["repositories"][0]["role"], "developer");
        assert!(items[0].get("members").is_none());
    }

    #[tokio::test]
    async fn user_without_groups_sees_empty_memberships() {
        let fx = fixture().await;
        let (status, memberships) = send(fx.app, &fx.reader_token, "GET", "/auth/me/groups").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(memberships, json!([]));
    }
}
