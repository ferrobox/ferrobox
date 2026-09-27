//! Federated sign-in (`OIDC` Authorization Code + PKCE).
//!
//! Discovers the `IdP`, exchanges the code, validates the `id_token`
//! against JWKS, provisions the account (JIT), syncs the role and the
//! mapped groups, and issues a `FerroBox` session token.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use ferrobox_domain::api_token::{ApiToken, ApiTokenName};
use ferrobox_domain::group::Group;
use ferrobox_domain::ids::UserId;
use ferrobox_domain::oidc::{
    group_name_from_claim, username_from_claims, OidcIdentity, OidcRoleMapping,
};
use ferrobox_domain::user::{Email, Role, User, Username};
use ferrobox_ports::api_token_store::{ApiTokenStore, ApiTokenStoreError};
use ferrobox_ports::group_store::{GroupStore, GroupStoreError};
use ferrobox_ports::http_client::{HttpClient, HttpClientError};
use ferrobox_ports::user_store::{UserStore, UserStoreError};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rand::RngCore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

use crate::auth_crypto::{
    generate_api_token_secret, hash_api_token_secret, hash_password,
};
use crate::login::LoginResult;

const PENDING_TTL: Duration = Duration::from_mins(10);
const DEFAULT_SCOPES: &str = "openid profile email";
const DEFAULT_GROUP_CLAIM: &str = "groups";

/// Configuration of a confidential or public `OIDC` client (PKCE).
#[derive(Debug, Clone)]
pub struct OidcSettings {
    issuer: String,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    success_redirect: String,
    scopes: String,
    role_mapping: OidcRoleMapping,
    extra_role_claim: Option<String>,
    group_claim: String,
    auto_create_groups: bool,
}

impl OidcSettings {
    /// Builds the configuration. The issuer is stored without a trailing slash.
    #[must_use]
    pub fn new(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Option<String>,
        redirect_uri: impl Into<String>,
        success_redirect: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into().trim().trim_end_matches('/').to_string(),
            client_id: client_id.into(),
            client_secret: client_secret.and_then(|value| {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }),
            redirect_uri: redirect_uri.into(),
            success_redirect: success_redirect.into(),
            scopes: DEFAULT_SCOPES.to_string(),
            role_mapping: OidcRoleMapping::ferrobox_defaults(),
            extra_role_claim: None,
            group_claim: DEFAULT_GROUP_CLAIM.to_string(),
            auto_create_groups: true,
        }
    }

    /// Replaces the role mapping.
    #[must_use]
    pub fn with_role_mapping(mut self, mapping: OidcRoleMapping) -> Self {
        self.role_mapping = mapping;
        self
    }

    /// Extra role *claim* (in addition to `realm_access` / `roles`).
    #[must_use]
    pub fn with_extra_role_claim(mut self, claim: Option<String>) -> Self {
        self.extra_role_claim = claim.filter(|value| !value.trim().is_empty());
        self
    }

    /// Group *claim*. Empty → `groups`.
    #[must_use]
    pub fn with_group_claim(mut self, claim: impl Into<String>) -> Self {
        let claim = claim.into();
        self.group_claim = if claim.trim().is_empty() {
            DEFAULT_GROUP_CLAIM.to_string()
        } else {
            claim
        };
        self
    }

    /// If `false`, only groups that already exist in `FerroBox` are assigned.
    #[must_use]
    pub fn with_auto_create_groups(mut self, auto_create: bool) -> Self {
        self.auto_create_groups = auto_create;
        self
    }

    /// Scopes sent to the `IdP`.
    #[must_use]
    pub fn with_scopes(mut self, scopes: impl Into<String>) -> Self {
        let scopes = scopes.into();
        if !scopes.trim().is_empty() {
            self.scopes = scopes;
        }
        self
    }

    /// Issuer (`iss`), without a trailing slash.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Client identifier.
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Return URL registered with the `IdP`.
    #[must_use]
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// UI destination after a successful login (`#sso_token=`).
    #[must_use]
    pub fn success_redirect(&self) -> &str {
        &self.success_redirect
    }
}

/// Reasons the `OIDC` flow can fail.
#[derive(Debug, Error)]
pub enum OidcError {
    /// The `IdP` is not configured on this instance.
    #[error("single sign-on is not configured")]
    Disabled,

    /// The `state` does not exist, expired, or was already used.
    #[error("invalid or expired sign-on state")]
    InvalidState,

    /// The `IdP` returned an error on the callback.
    #[error("identity provider error: {0}")]
    Provider(String),

    /// The `id_token` is not valid (signature, issuer, audience, or nonce).
    #[error("invalid identity token: {0}")]
    InvalidToken(String),

    /// `sub` or another required field is missing.
    #[error("identity token is missing required claims")]
    MissingClaims,

    /// A valid username could not be built.
    #[error("could not derive a username from the identity token")]
    InvalidUsername,

    /// Failed to discover or talk to the `IdP`.
    #[error(transparent)]
    Http(#[from] HttpClientError),

    /// Failed to persist the user.
    #[error(transparent)]
    Users(#[from] UserStoreError),

    /// Failed to persist groups.
    #[error(transparent)]
    Groups(#[from] GroupStoreError),

    /// Failed to issue the session token.
    #[error(transparent)]
    Tokens(#[from] ApiTokenStoreError),

    /// Failed to hash the random password of a JIT account.
    #[error("failed to hash generated password")]
    PasswordHash,
}

/// Public status: whether the SSO button should be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcStatus {
    /// `true` if issuer and client are configured.
    pub enabled: bool,
    /// Issuer, if enabled.
    pub issuer: Option<String>,
}

/// Result of provisioning, before issuing the session token.
#[derive(Debug, Clone)]
pub struct OidcProvision {
    /// Already persisted user (role and `OIDC` link up to date).
    pub user: User,
    /// `true` if the account did not exist.
    pub created: bool,
}

struct PendingAuth {
    verifier: String,
    nonce: String,
    created_at: Instant,
}

#[derive(Debug, Clone, Deserialize)]
struct OidcDiscovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    end_session_endpoint: Option<String>,
}

/// Federated sign-in service.
pub struct OidcLoginService {
    settings: OidcSettings,
    http_client: std::sync::Arc<dyn HttpClient>,
    user_store: std::sync::Arc<dyn UserStore>,
    group_store: std::sync::Arc<dyn GroupStore>,
    api_token_store: std::sync::Arc<dyn ApiTokenStore>,
    pending: Mutex<HashMap<String, PendingAuth>>,
    discovery: Mutex<Option<OidcDiscovery>>,
    session_ttl: Duration,
}

impl OidcLoginService {
    /// Builds the service.
    #[must_use]
    pub fn new(
        settings: OidcSettings,
        http_client: std::sync::Arc<dyn HttpClient>,
        user_store: std::sync::Arc<dyn UserStore>,
        group_store: std::sync::Arc<dyn GroupStore>,
        api_token_store: std::sync::Arc<dyn ApiTokenStore>,
    ) -> Self {
        Self {
            settings,
            http_client,
            user_store,
            group_store,
            api_token_store,
            pending: Mutex::new(HashMap::new()),
            discovery: Mutex::new(None),
            session_ttl: Duration::from_hours(12),
        }
    }

    /// TTL of the session token issued after the callback.
    #[must_use]
    pub fn with_session_ttl(mut self, session_ttl: Duration) -> Self {
        self.session_ttl = session_ttl;
        self
    }

    /// Whether SSO is ready to use.
    #[must_use]
    pub fn status(&self) -> OidcStatus {
        OidcStatus {
            enabled: true,
            issuer: Some(self.settings.issuer.clone()),
        }
    }

    /// Settings (issuer, redirects) for the HTTP layer.
    #[must_use]
    pub fn settings(&self) -> &OidcSettings {
        &self.settings
    }

    /// Builds the authorization URL and stores `state` + PKCE.
    ///
    /// # Errors
    ///
    /// [`OidcError`] if `IdP` discovery fails.
    ///
    /// # Panics
    ///
    /// If the pending-state mutex is poisoned.
    pub async fn start(&self) -> Result<String, OidcError> {
        let discovery = self.discovery().await?;
        let state = random_urlsafe(24);
        let nonce = random_urlsafe(24);
        let verifier = random_urlsafe(48);
        let challenge = pkce_challenge(&verifier);

        {
            let mut pending = self.pending.lock().expect("oidc pending lock");
            pending.retain(|_, item| item.created_at.elapsed() < PENDING_TTL);
            pending.insert(
                state.clone(),
                PendingAuth {
                    verifier,
                    nonce: nonce.clone(),
                    created_at: Instant::now(),
                },
            );
        }

        let mut url = Url::parse(&discovery.authorization_endpoint).map_err(|err| {
            OidcError::InvalidToken(format!("invalid authorization endpoint: {err}"))
        })?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("response_type", "code");
            query.append_pair("client_id", &self.settings.client_id);
            query.append_pair("redirect_uri", &self.settings.redirect_uri);
            query.append_pair("scope", &self.settings.scopes);
            query.append_pair("state", &state);
            query.append_pair("nonce", &nonce);
            query.append_pair("code_challenge", &challenge);
            query.append_pair("code_challenge_method", "S256");
            query.append_pair("prompt", "login");
        }
        Ok(url.to_string())
    }

    /// *Logout* URL at the `IdP` (RP-initiated). If discovery does not
    /// publish `end_session_endpoint`, returns the local destination.
    ///
    /// # Errors
    ///
    /// [`OidcError`] if `IdP` discovery fails.
    ///
    /// # Panics
    ///
    /// If the discovery mutex is poisoned.
    pub async fn logout_url(&self) -> Result<String, OidcError> {
        let discovery = self.discovery().await?;
        let Some(endpoint) = discovery
            .end_session_endpoint
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(self.settings.success_redirect.clone());
        };
        let mut url = Url::parse(&endpoint).map_err(|err| {
            OidcError::InvalidToken(format!("invalid end_session endpoint: {err}"))
        })?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_id", &self.settings.client_id);
            query.append_pair(
                "post_logout_redirect_uri",
                self.settings.success_redirect(),
            );
        }
        Ok(url.to_string())
    }

    /// Exchanges the code, validates the token, provisions, and issues a session.
    ///
    /// # Errors
    ///
    /// [`OidcError`] if the `state` is invalid, the `IdP` fails, or the
    /// *claims* are not enough to create the account.
    ///
    /// # Panics
    ///
    /// If the pending-state mutex is poisoned.
    pub async fn finish(&self, code: &str, state: &str) -> Result<LoginResult, OidcError> {
        let pending = {
            let mut pending = self.pending.lock().expect("oidc pending lock");
            pending.retain(|_, item| item.created_at.elapsed() < PENDING_TTL);
            pending.remove(state).ok_or(OidcError::InvalidState)?
        };

        let discovery = self.discovery().await?;
        let id_token = self
            .exchange_code(code, &pending.verifier, &discovery.token_endpoint)
            .await?;
        let claims = self
            .validate_id_token(&id_token, &pending.nonce, &discovery)
            .await?;
        let provision = self.provision(&claims).await?;
        self.issue_session(provision.user).await
    }

    /// Provisions or updates an account from already validated *claims*.
    /// Visible for tests and to reuse JIT without HTTP.
    ///
    /// # Errors
    ///
    /// [`OidcError`] if *claims* are missing, the name is not valid, or
    /// persistence fails.
    pub async fn provision(&self, claims: &Value) -> Result<OidcProvision, OidcError> {
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(OidcError::MissingClaims)?;
        let identity = OidcIdentity::new(&self.settings.issuer, subject);
        let preferred = claims
            .get("preferred_username")
            .and_then(Value::as_str)
            .or_else(|| claims.get("username").and_then(Value::as_str));
        let email = claims
            .get("email")
            .and_then(Value::as_str)
            .and_then(|value| Email::parse(value).ok());
        let idp_roles = collect_idp_roles(
            claims,
            &self.settings.client_id,
            self.settings.extra_role_claim.as_deref(),
        );
        let mapped_role = self.settings.role_mapping.map_role(&idp_roles);
        let idp_groups = collect_idp_groups(claims, &self.settings.group_claim);

        let existing = self.user_store.find_by_oidc(&identity).await?;
        let existing = match existing {
            Some(user) if !user.is_robot() => Some(user),
            Some(_) => None,
            None => match &email {
                Some(email) => {
                    let found = self.user_store.find_by_email(email).await?;
                    found.filter(|user| !user.is_robot())
                }
                None => None,
            },
        };

        let (user, created) = if let Some(user) = existing {
            let role = self.effective_role(user.id(), user.role(), mapped_role).await?;
            let updated = user
                .clone()
                .with_email(email.or_else(|| user.email().cloned()))
                .with_role(role)
                .with_oidc(Some(identity));
            let hash = self
                .user_store
                .find_by_id_with_password_hash(updated.id())
                .await?
                .map(|(_, hash)| hash)
                .ok_or(OidcError::MissingClaims)?;
            self.user_store
                .save_with_password_hash(&updated, &hash)
                .await?;
            (updated, false)
        } else {
            let username = self
                .allocate_username(preferred, subject)
                .await?;
            let user = User::new(username, mapped_role)
                .with_email(email)
                .with_oidc(Some(identity));
            let random = random_urlsafe(32);
            let hash = hash_password(&random).map_err(|_| OidcError::PasswordHash)?;
            self.user_store.save_with_password_hash(&user, &hash).await?;
            (user, true)
        };

        self.sync_groups(user.id(), &idp_groups).await?;
        Ok(OidcProvision { user, created })
    }

    async fn issue_session(&self, user: User) -> Result<LoginResult, OidcError> {
        let (plaintext_secret, prefix) = generate_api_token_secret();
        let expires_at = Utc::now() + self.session_ttl;
        let token = ApiToken::new(
            user.id(),
            ApiTokenName::parse("session").expect("literal 'session' is a valid token name"),
            prefix,
        )
        .with_expires_at(Some(expires_at));
        let token_hash = hash_api_token_secret(&plaintext_secret);
        self.api_token_store.save(&token, &token_hash).await?;
        Ok(LoginResult {
            user,
            token,
            plaintext_secret,
        })
    }

    async fn allocate_username(
        &self,
        preferred: Option<&str>,
        subject: &str,
    ) -> Result<Username, OidcError> {
        let base = username_from_claims(preferred, subject).map_err(|_| OidcError::InvalidUsername)?;
        if self
            .user_store
            .find_by_username_with_password_hash(&base)
            .await?
            .is_none()
        {
            return Ok(base);
        }
        let suffix = username_from_claims(None, subject)
            .map_err(|_| OidcError::InvalidUsername)?
            .as_str()
            .trim_start_matches("sso_")
            .chars()
            .take(8)
            .collect::<String>();
        let mut candidate = format!("{}_{suffix}", trim_to(base.as_str(), 64 - suffix.len() - 1));
        for extra in 2..20 {
            let parsed = Username::parse(&candidate).map_err(|_| OidcError::InvalidUsername)?;
            if self
                .user_store
                .find_by_username_with_password_hash(&parsed)
                .await?
                .is_none()
            {
                return Ok(parsed);
            }
            candidate = format!(
                "{}_{suffix}{extra}",
                trim_to(base.as_str(), 64 - suffix.len() - 2)
            );
        }
        Err(OidcError::InvalidUsername)
    }

    async fn effective_role(
        &self,
        user_id: UserId,
        current: Role,
        mapped: Role,
    ) -> Result<Role, OidcError> {
        if mapped != Role::Admin && current == Role::Admin {
            let admins = self.user_store.count_admins().await?;
            if admins <= 1 {
                let Some(user) = self.user_store.find_by_id(user_id).await? else {
                    return Ok(mapped);
                };
                if user.role() == Role::Admin {
                    return Ok(Role::Admin);
                }
            }
        }
        Ok(mapped)
    }

    async fn sync_groups(&self, user_id: UserId, idp_groups: &[String]) -> Result<(), OidcError> {
        let mut desired = Vec::new();
        for raw in idp_groups {
            let Ok(name) = group_name_from_claim(raw) else {
                continue;
            };
            let group = match self.group_store.find_by_name(&name).await? {
                Some(group) => group,
                None if self.settings.auto_create_groups => {
                    let group = Group::new(name);
                    self.group_store.save(&group).await?;
                    group
                }
                None => continue,
            };
            if desired.contains(&group.id()) {
                continue;
            }
            self.group_store.add_member(group.id(), user_id).await?;
            desired.push(group.id());
        }

        let previous = self.group_store.sso_memberships(user_id).await?;
        for old in previous {
            if !desired.contains(&old) {
                self.group_store.remove_member(old, user_id).await?;
            }
        }
        self.group_store
            .set_sso_memberships(user_id, &desired)
            .await?;
        Ok(())
    }

    async fn discovery(&self) -> Result<OidcDiscovery, OidcError> {
        if let Some(cached) = self.discovery.lock().expect("oidc discovery lock").clone() {
            return Ok(cached);
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.settings.issuer
        );
        let response = self.http_client.get(&url).await?;
        let discovery: OidcDiscovery = serde_json::from_slice(&response.body)
            .map_err(|err| OidcError::InvalidToken(err.to_string()))?;
        if discovery.issuer.trim_end_matches('/') != self.settings.issuer {
            return Err(OidcError::InvalidToken(format!(
                "discovered issuer '{}' does not match '{}'",
                discovery.issuer, self.settings.issuer
            )));
        }
        *self.discovery.lock().expect("oidc discovery lock") = Some(discovery.clone());
        Ok(discovery)
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        token_endpoint: &str,
    ) -> Result<String, OidcError> {
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", self.settings.redirect_uri.clone()),
            ("client_id", self.settings.client_id.clone()),
            ("code_verifier", verifier.to_string()),
        ];
        if let Some(secret) = &self.settings.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let body = form
            .into_iter()
            .map(|(key, value)| format!("{key}={}", urlencoding(value.as_str())))
            .collect::<Vec<_>>()
            .join("&");
        let response = self
            .http_client
            .post(
                token_endpoint,
                Bytes::from(body),
                "application/x-www-form-urlencoded",
            )
            .await?;
        let payload: Value = serde_json::from_slice(&response.body)
            .map_err(|err| OidcError::InvalidToken(err.to_string()))?;
        payload
            .get("id_token")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| OidcError::InvalidToken("token response has no id_token".into()))
    }

    async fn validate_id_token(
        &self,
        token: &str,
        nonce: &str,
        discovery: &OidcDiscovery,
    ) -> Result<Value, OidcError> {
        let header = decode_header(token).map_err(|err| OidcError::InvalidToken(err.to_string()))?;
        if header.alg != Algorithm::RS256 {
            return Err(OidcError::InvalidToken(format!(
                "unsupported id_token algorithm {:?}",
                header.alg
            )));
        }
        let jwks = self.http_client.get(&discovery.jwks_uri).await?;
        let jwks: Value = serde_json::from_slice(&jwks.body)
            .map_err(|err| OidcError::InvalidToken(err.to_string()))?;
        let key = select_rsa_key(&jwks, header.kid.as_deref())
            .ok_or_else(|| OidcError::InvalidToken("no matching JWKS RSA key".into()))?;
        let decoding = DecodingKey::from_rsa_components(&key.0, &key.1)
            .map_err(|err| OidcError::InvalidToken(err.to_string()))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[self.settings.issuer.as_str()]);
        validation.set_audience(&[self.settings.client_id.as_str()]);
        let token_data = decode::<Value>(token, &decoding, &validation)
            .map_err(|err| OidcError::InvalidToken(err.to_string()))?;
        let actual_nonce = token_data
            .claims
            .get("nonce")
            .and_then(Value::as_str)
            .unwrap_or("");
        if actual_nonce != nonce {
            return Err(OidcError::InvalidToken("nonce mismatch".into()));
        }
        Ok(token_data.claims)
    }
}

fn collect_idp_roles(claims: &Value, client_id: &str, extra_claim: Option<&str>) -> Vec<String> {
    let mut roles = Vec::new();
    push_string_array(&mut roles, claims.pointer("/realm_access/roles"));
    if !client_id.is_empty() {
        push_string_array(
            &mut roles,
            claims.pointer(&format!("/resource_access/{client_id}/roles")),
        );
    }
    push_string_array(&mut roles, claims.get("roles"));
    if let Some(claim) = extra_claim.filter(|name| !name.is_empty() && *name != "roles") {
        push_string_array(&mut roles, claims.get(claim));
    }
    roles
}

fn collect_idp_groups(claims: &Value, group_claim: &str) -> Vec<String> {
    let mut groups = Vec::new();
    push_string_array(&mut groups, claims.get(group_claim));
    groups
}

fn push_string_array(out: &mut Vec<String>, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Array(items) => {
            for item in items {
                if let Some(text) = item.as_str() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
                        out.push(trimmed.to_string());
                    }
                }
            }
        }
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
                out.push(trimmed.to_string());
            }
        }
        _ => {}
    }
}

fn select_rsa_key(jwks: &Value, kid: Option<&str>) -> Option<(String, String)> {
    let keys = jwks.get("keys")?.as_array()?;
    let chosen = kid
        .and_then(|kid| {
            keys.iter().find(|key| {
                key.get("kid").and_then(Value::as_str) == Some(kid)
                    && key.get("kty").and_then(Value::as_str) == Some("RSA")
            })
        })
        .or_else(|| {
            keys.iter()
                .find(|key| key.get("kty").and_then(Value::as_str) == Some("RSA"))
        })?;
    Some((
        chosen.get("n")?.as_str()?.to_string(),
        chosen.get("e")?.as_str()?.to_string(),
    ))
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0_u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn urlencoding(value: &str) -> String {
    let mut encoded = String::new();
    for ch in value.bytes() {
        match ch {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(ch));
            }
            other => {
                let _ = write!(encoded, "%{other:02X}");
            }
        }
    }
    encoded
}

fn trim_to(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::group::{Group, GroupName};
    use ferrobox_domain::user::Username;
    use serde_json::json;

    use super::*;
    use crate::test_support::{
        InMemoryApiTokenStore, InMemoryGroupStore, InMemoryHttpClient, InMemoryUserStore,
    };

    fn settings() -> OidcSettings {
        OidcSettings::new(
            "https://idp.example/realms/ferrobox",
            "ferrobox",
            Some("secret".into()),
            "http://127.0.0.1:3000/api/auth/oidc/callback",
            "http://127.0.0.1:3000/login",
        )
    }

    fn service(
        users: Arc<InMemoryUserStore>,
        groups: Arc<InMemoryGroupStore>,
        tokens: Arc<InMemoryApiTokenStore>,
        http: Arc<InMemoryHttpClient>,
    ) -> OidcLoginService {
        OidcLoginService::new(settings(), http, users, groups, tokens)
    }

    fn claims(sub: &str, username: &str, email: &str, roles: &[&str], groups: &[&str]) -> Value {
        json!({
            "sub": sub,
            "preferred_username": username,
            "email": email,
            "realm_access": { "roles": roles },
            "groups": groups
        })
    }

    #[tokio::test]
    async fn jit_creates_a_developer_and_auto_creates_groups() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let oidc = service(users.clone(), groups.clone(), tokens, http);

        let provision = oidc
            .provision(&claims(
                "sub-ada",
                "ada.lovelace",
                "ada@example.com",
                &["ferrobox-developer"],
                &["/platform", "backend"],
            ))
            .await
            .unwrap();

        assert!(provision.created);
        assert_eq!(provision.user.username().as_str(), "ada_lovelace");
        assert_eq!(provision.user.role(), Role::Developer);
        assert!(provision.user.is_sso_linked());
        let platform = groups
            .find_by_name(&GroupName::parse("platform").unwrap())
            .await
            .unwrap()
            .unwrap();
        let members = groups.members(platform.id()).await.unwrap();
        assert_eq!(members, vec![provision.user.id()]);
    }

    #[tokio::test]
    async fn second_login_updates_role_and_syncs_sso_groups() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let oidc = service(users.clone(), groups.clone(), tokens, http);

        let first = oidc
            .provision(&claims(
                "sub-ada",
                "ada",
                "ada@example.com",
                &["ferrobox-developer"],
                &["platform", "backend"],
            ))
            .await
            .unwrap();
        let second = oidc
            .provision(&claims(
                "sub-ada",
                "ada",
                "ada@example.com",
                &["ferrobox-admin"],
                &["platform"],
            ))
            .await
            .unwrap();

        assert!(!second.created);
        assert_eq!(second.user.id(), first.user.id());
        assert_eq!(second.user.role(), Role::Admin);
        let backend = groups
            .find_by_name(&GroupName::parse("backend").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(groups.members(backend.id()).await.unwrap().is_empty());
        let platform = groups
            .find_by_name(&GroupName::parse("platform").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            groups.members(platform.id()).await.unwrap(),
            vec![second.user.id()]
        );
    }

    #[tokio::test]
    async fn links_an_existing_local_user_by_email() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let local = User::new(Username::parse("ada").unwrap(), Role::Reader)
            .with_email(Some(Email::parse("ada@example.com").unwrap()));
        users
            .save_with_password_hash(&local, "hash")
            .await
            .unwrap();
        let oidc = service(users.clone(), groups, tokens, http);

        let provision = oidc
            .provision(&claims(
                "sub-ada",
                "other",
                "ada@example.com",
                &["ferrobox-developer"],
                &[],
            ))
            .await
            .unwrap();

        assert!(!provision.created);
        assert_eq!(provision.user.id(), local.id());
        assert_eq!(provision.user.username().as_str(), "ada");
        assert_eq!(provision.user.role(), Role::Developer);
        assert!(provision.user.is_sso_linked());
        let stored = users
            .find_by_id_with_password_hash(local.id())
            .await
            .unwrap()
            .expect("linked user must still have a password hash");
        assert_eq!(
            stored.1, "hash",
            "SSO link must keep the local password hash"
        );
    }

    #[tokio::test]
    async fn local_password_still_logs_in_after_sso_link() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let local = User::new(Username::parse("linus").unwrap(), Role::Reader)
            .with_email(Some(Email::parse("linus@example.com").unwrap()));
        users
            .save_with_password_hash(&local, &hash_password("LocalPass1").unwrap())
            .await
            .unwrap();
        let oidc = service(users.clone(), groups, tokens.clone(), http);
        oidc.provision(&claims(
            "sub-linus",
            "linus",
            "linus@example.com",
            &["ferrobox-reader"],
            &[],
        ))
        .await
        .unwrap();

        crate::login::LoginUseCase::new(users, tokens)
            .execute(Username::parse("linus").unwrap(), "LocalPass1")
            .await
            .expect("admin-set local password must still work after SSO link");
    }

    #[tokio::test]
    async fn does_not_demote_the_last_admin() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let admin = User::new(Username::parse("admin").unwrap(), Role::Admin)
            .with_email(Some(Email::parse("admin@example.com").unwrap()));
        users
            .save_with_password_hash(&admin, "hash")
            .await
            .unwrap();
        let oidc = service(users, groups, tokens, http);

        let provision = oidc
            .provision(&claims(
                "sub-admin",
                "admin",
                "admin@example.com",
                &["ferrobox-reader"],
                &[],
            ))
            .await
            .unwrap();

        assert_eq!(provision.user.role(), Role::Admin);
    }

    #[tokio::test]
    async fn keeps_locally_granted_groups_that_the_idp_does_not_list() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        let oidc = service(users.clone(), groups.clone(), tokens, http);

        let first = oidc
            .provision(&claims(
                "sub-ada",
                "ada",
                "ada@example.com",
                &["ferrobox-developer"],
                &["platform"],
            ))
            .await
            .unwrap();
        let local = Group::new(GroupName::parse("ops").unwrap());
        groups.save(&local).await.unwrap();
        groups
            .add_member(local.id(), first.user.id())
            .await
            .unwrap();

        let second = oidc
            .provision(&claims(
                "sub-ada",
                "ada",
                "ada@example.com",
                &["ferrobox-developer"],
                &["platform"],
            ))
            .await
            .unwrap();

        assert!(!second.created);
        assert_eq!(
            groups.members(local.id()).await.unwrap(),
            vec![second.user.id()]
        );
        let platform = groups
            .find_by_name(&GroupName::parse("platform").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            groups.members(platform.id()).await.unwrap(),
            vec![second.user.id()]
        );
    }

    #[tokio::test]
    async fn start_redirects_to_the_authorization_endpoint_with_pkce() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            "https://idp.example/realms/ferrobox/.well-known/openid-configuration",
            200,
            serde_json::to_vec(&json!({
                "issuer": "https://idp.example/realms/ferrobox",
                "authorization_endpoint": "https://idp.example/realms/ferrobox/protocol/openid-connect/auth",
                "token_endpoint": "https://idp.example/realms/ferrobox/protocol/openid-connect/token",
                "jwks_uri": "https://idp.example/realms/ferrobox/protocol/openid-connect/certs",
                "end_session_endpoint": "https://idp.example/realms/ferrobox/protocol/openid-connect/logout"
            }))
            .unwrap(),
        );
        let oidc = service(users, groups, tokens, http);
        let url = oidc.start().await.unwrap();
        assert!(url.starts_with(
            "https://idp.example/realms/ferrobox/protocol/openid-connect/auth?"
        ));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("client_id=ferrobox"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("prompt=login"));
    }

    #[tokio::test]
    async fn logout_url_uses_the_discovered_end_session_endpoint() {
        let users = Arc::new(InMemoryUserStore::default());
        let groups = Arc::new(InMemoryGroupStore::default());
        let tokens = Arc::new(InMemoryApiTokenStore::default());
        let http = Arc::new(InMemoryHttpClient::default());
        http.stub(
            "https://idp.example/realms/ferrobox/.well-known/openid-configuration",
            200,
            serde_json::to_vec(&json!({
                "issuer": "https://idp.example/realms/ferrobox",
                "authorization_endpoint": "https://idp.example/realms/ferrobox/protocol/openid-connect/auth",
                "token_endpoint": "https://idp.example/realms/ferrobox/protocol/openid-connect/token",
                "jwks_uri": "https://idp.example/realms/ferrobox/protocol/openid-connect/certs",
                "end_session_endpoint": "https://idp.example/realms/ferrobox/protocol/openid-connect/logout"
            }))
            .unwrap(),
        );
        let oidc = service(users, groups, tokens, http);
        let url = oidc.logout_url().await.unwrap();
        assert!(url.starts_with(
            "https://idp.example/realms/ferrobox/protocol/openid-connect/logout?"
        ));
        assert!(url.contains("client_id=ferrobox"));
        assert!(url.contains("post_logout_redirect_uri="));
    }

    #[tokio::test]
    async fn finish_rejects_an_unknown_state() {
        let oidc = service(
            Arc::new(InMemoryUserStore::default()),
            Arc::new(InMemoryGroupStore::default()),
            Arc::new(InMemoryApiTokenStore::default()),
            Arc::new(InMemoryHttpClient::default()),
        );
        let error = oidc.finish("code", "nope").await.unwrap_err();
        assert!(matches!(error, OidcError::InvalidState));
    }
}
