//! Identidad federada (`OIDC`) y el mapeo de roles y grupos del `IdP`
//! hacia el modelo de `FerroBox`.
//!
//! No habla HTTP ni interpreta JSON: la capa de aplicación entrega
//! listas de etiquetas ya extraídas de las *claims*.

use crate::group::{GroupName, GroupNameError};
use crate::user::{Role, Username, UsernameError};

const MAX_USERNAME_LENGTH: usize = 64;
const MAX_GROUP_NAME_LENGTH: usize = 64;

/// Cómo se traducen los roles del `IdP` a un [`Role`] de instancia.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcRoleMapping {
    admin: Vec<String>,
    developer: Vec<String>,
    reader: Vec<String>,
}

impl OidcRoleMapping {
    /// Mapeo por defecto: `ferrobox-admin`, `ferrobox-developer` y
    /// `ferrobox-reader`. No usa el rol `admin` genérico del `IdP` para
    /// no promover a cualquiera con privilegios de consola.
    #[must_use]
    pub fn ferrobox_defaults() -> Self {
        Self::new(
            ["ferrobox-admin"],
            ["ferrobox-developer"],
            ["ferrobox-reader"],
        )
    }

    /// Construye el mapeo a partir de listas (se normalizan a
    /// minúsculas y se descartan vacíos).
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

    /// Roles del `IdP` que conceden [`Role::Admin`].
    #[must_use]
    pub fn admin(&self) -> &[String] {
        &self.admin
    }

    /// Roles del `IdP` que conceden [`Role::Developer`].
    #[must_use]
    pub fn developer(&self) -> &[String] {
        &self.developer
    }

    /// Roles del `IdP` que conceden [`Role::Reader`].
    #[must_use]
    pub fn reader(&self) -> &[String] {
        &self.reader
    }

    /// Elige el rol de instancia. Si hay varias coincidencias, gana el
    /// más privilegiado. Sin coincidencias, [`Role::Reader`].
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

/// Convierte `preferred_username` (o, en su defecto, `sub`) en un
/// [`Username`] válido: solo letras, dígitos, `-` y `_`.
///
/// # Errors
///
/// [`UsernameError`] si, tras sanear, el resultado sigue vacío.
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

/// Convierte una ruta o nombre de grupo del `IdP` en un [`GroupName`].
/// Toma el último segmento (`/org/backend` → `backend`) y sustituye
/// caracteres fuera del alfabeto por `_`.
///
/// # Errors
///
/// [`GroupNameError`] si el resultado queda vacío o es inválido.
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

/// Sujeto (`sub`) y emisor ya aceptados: identifican a una cuenta en
/// un `IdP` concreto.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OidcIdentity {
    issuer: String,
    subject: String,
}

impl OidcIdentity {
    /// Construye la identidad. El emisor se guarda sin barra final.
    #[must_use]
    pub fn new(issuer: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into().trim().trim_end_matches('/').to_string(),
            subject: subject.into(),
        }
    }

    /// URL del emisor (`iss`), sin barra final.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Identificador estable del usuario en el `IdP` (`sub`).
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
