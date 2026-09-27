//! Federated identity (`OIDC`) and the mapping of IdP roles and groups
//! onto the `FerroBox` model.
//!
//! It does not speak HTTP or parse JSON: the application layer delivers
//! lists of labels already extracted from the *claims*.

use crate::group::{GroupName, GroupNameError};
use crate::user::{Role, Username, UsernameError};

const MAX_USERNAME_LENGTH: usize = 64;
const MAX_GROUP_NAME_LENGTH: usize = 64;

/// How IdP roles are translated into an instance [`Role`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcRoleMapping {
    admin: Vec<String>,
    developer: Vec<String>,
    reader: Vec<String>,
}

impl OidcRoleMapping {
    /// Default mapping: `ferrobox-admin`, `ferrobox-developer`, and
    /// `ferrobox-reader`. Does not use the generic `admin` role of the IdP so
    /// as not to promote anyone with console privileges.
    #[must_use]
    pub fn ferrobox_defaults() -> Self {
        Self::new(
            ["ferrobox-admin"],
            ["ferrobox-developer"],
            ["ferrobox-reader"],
        )
    }

    /// Builds the mapping from lists (they are normalized to
    /// lowercase and empty values are discarded).
    #[must_use]
    pub fn new(
        admin: impl IntoIterator<Item = impl AsRef<str>>,
        developer: impl IntoIterator<Item = impl AsRef<str>>,
        reader: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        Self {
            admin: normalize_list(admin),
            developer: normalize_list(developer),
            reader: normalize_list(reader),
        }
    }

    /// IdP roles that grant [`Role::Admin`].
    #[must_use]
    pub fn admin(&self) -> &[String] {
        &self.admin
    }

    /// IdP roles that grant [`Role::Developer`].
    #[must_use]
    pub fn developer(&self) -> &[String] {
        &self.developer
    }

    /// IdP roles that grant [`Role::Reader`].
    #[must_use]
    pub fn reader(&self) -> &[String] {
        &self.reader
    }

    /// Chooses the instance role. If there are several matches, the
    /// most privileged wins. With no matches, [`Role::Reader`].
    #[must_use]
    pub fn map_role(&self, idp_roles: &[impl AsRef<str>]) -> Role {
        let roles: Vec<String> = idp_roles
            .iter()
            .map(|role| normalize_label(role.as_ref()))
            .collect();
        if roles.iter().any(|role| self.admin.contains(role)) {
            Role::Admin
        } else if roles.iter().any(|role| self.developer.contains(role)) {
            Role::Developer
        } else {
            Role::Reader
        }
    }
}

/// Converts `preferred_username` (or, failing that, `sub`) into a
/// valid [`Username`]: only letters, digits, `-`, and `_`.
///
/// # Errors
///
/// [`UsernameError`] if, after sanitizing, the result is still empty.
pub fn username_from_claims(
    preferred: Option<&str>,
    subject: &str,
) -> Result<Username, UsernameError> {
    let source = preferred.map(str::trim).filter(|value| !value.is_empty());
    let mut sanitized = source.map(sanitize_identifier).unwrap_or_default();
    if sanitized.is_empty() {
        sanitized = format!("sso_{}", sanitize_identifier(subject));
    }
    if sanitized.is_empty() {
        return Err(UsernameError::Empty);
    }
    if sanitized.len() > MAX_USERNAME_LENGTH {
        sanitized.truncate(MAX_USERNAME_LENGTH);
    }
    Username::parse(sanitized)
}

/// Converts an IdP group path or name into a [`GroupName`].
/// Takes the last segment (`/org/backend` → `backend`) and replaces
/// characters outside the alphabet with `_`.
///
/// # Errors
///
/// [`GroupNameError`] if the result is empty or invalid.
pub fn group_name_from_claim(raw: &str) -> Result<GroupName, GroupNameError> {
    let segment = raw.trim().trim_matches('/').rsplit('/').next().unwrap_or("");
    let mut sanitized = sanitize_identifier(segment);
    if sanitized.is_empty() {
        return Err(GroupNameError::Empty);
    }
    if sanitized.len() > MAX_GROUP_NAME_LENGTH {
        sanitized.truncate(MAX_GROUP_NAME_LENGTH);
    }
    GroupName::parse(sanitized)
}

/// Subject (`sub`) and issuer already accepted: they identify an account on
/// a concrete IdP.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OidcIdentity {
    issuer: String,
    subject: String,
}

impl OidcIdentity {
    /// Builds the identity. The issuer is stored without a trailing slash.
    #[must_use]
    pub fn new(issuer: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into().trim().trim_end_matches('/').to_string(),
            subject: subject.into(),
        }
    }

    /// Issuer URL (`iss`), without a trailing slash.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Stable identifier of the user in the IdP (`sub`).
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }
}

fn sanitize_identifier(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_sep = false;
    for ch in value.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch)
        } else if matches!(ch, '-' | '_' | '.' | '@' | ' ' | '/') {
            Some('_')
        } else {
            None
        };
        match mapped {
            Some('_' | '-') if last_was_sep || out.is_empty() => {}
            Some(sep @ ('_' | '-')) => {
                out.push(sep);
                last_was_sep = true;
            }
            Some(ch) => {
                out.push(ch);
                last_was_sep = false;
            }
            None => {}
        }
    }
    while out.ends_with(['_', '-']) {
        out.pop();
    }
    out
}

fn normalize_list(values: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    let mut out: Vec<String> = values
        .into_iter()
        .map(|value| normalize_label(value.as_ref()))
        .filter(|value| !value.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn normalize_label(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_most_privileged_matching_role() {
        let mapping = OidcRoleMapping::ferrobox_defaults();
        assert_eq!(
            mapping.map_role(&["ferrobox-developer", "ferrobox-admin"]),
            Role::Admin
        );
        assert_eq!(mapping.map_role(&["ferrobox-developer"]), Role::Developer);
        assert_eq!(mapping.map_role(&["unrelated"]), Role::Reader);
    }

    #[test]
    fn default_mapping_ignores_generic_admin() {
        let mapping = OidcRoleMapping::ferrobox_defaults();
        assert_eq!(mapping.map_role(&["admin"]), Role::Reader);
    }

    #[test]
    fn username_replaces_dots_and_falls_back_to_subject() {
        let name = username_from_claims(Some("ada.lovelace"), "sub-1").unwrap();
        assert_eq!(name.as_str(), "ada_lovelace");
        let fallback = username_from_claims(None, "abc-123").unwrap();
        assert_eq!(fallback.as_str(), "sso_abc_123");
    }

    #[test]
    fn group_name_uses_the_last_path_segment() {
        let name = group_name_from_claim("/Engineering/backend").unwrap();
        assert_eq!(name.as_str(), "backend");
        assert!(group_name_from_claim("///").is_err());
    }
}
