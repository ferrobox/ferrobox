//! Admission policy for a repository: structured rules that decide
//! whether an artifact may be pulled or promoted into another Forge.
//!
//! Varias cláusulas pueden estar armadas a la vez (firma, hallazgo OSV,
//! licencia denegada). La primera que se cumple dispara el efecto.

use thiserror::Error;

use crate::assay::AssaySeverity;
use crate::ids::{AdmissionEventId, RepositoryId};

/// Momento en el que se evalúa la regla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionWhen {
    /// Al resolver un manifiesto o un paquete para instalarlo.
    Pull,
}

/// Condición de firma persistida por compatibilidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPredicate {
    /// El artefacto no tiene una firma Cosign / Notation enlazada.
    NotSigned,
    /// El artefacto no tiene una firma Cosign válida contra las claves
    /// públicas configuradas.
    NotVerified,
}

/// Qué hacer si la condición se cumple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionEffect {
    /// Bloquea la operación.
    Deny,
    /// Deja pasar y solo deja constancia (dry-run operativo).
    Warn,
}

/// Motivos por los que una política de admisión no es válida.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdmissionPolicyError {
    /// El momento no es uno de los soportados.
    #[error("unsupported admission moment '{0}'")]
    UnknownWhen(String),

    /// El predicado no es uno de los soportados.
    #[error("unsupported admission predicate '{0}'")]
    UnknownPredicate(String),

    /// El efecto no es uno de los soportados.
    #[error("unsupported admission effect '{0}'")]
    UnknownEffect(String),

    /// El umbral de hallazgo no es válido.
    #[error("unsupported admission finding threshold '{0}'")]
    UnknownFinding(String),
}

/// Identificador de un perfil preconfigurado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionProfile {
    /// ISO/IEC 18974: detectar y actuar ante hallazgos ≥ Medium.
    OpenChainSecurity,
    /// Política inbound habitual: copyleft fuerte denegado.
    CopyleftRestrict,
    /// Solo hallazgos Critical, denegar.
    CriticalOnly,
}

impl AdmissionProfile {
    /// Etiqueta persistida.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenChainSecurity => "openchain_security",
            Self::CopyleftRestrict => "copyleft_restrict",
            Self::CriticalOnly => "critical_only",
        }
    }

    /// Parsea la etiqueta.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "openchain_security" => Some(Self::OpenChainSecurity),
            "copyleft_restrict" => Some(Self::CopyleftRestrict),
            "critical_only" => Some(Self::CriticalOnly),
            _ => None,
        }
    }
}

/// Licencias SPDX del perfil de copyleft restrictivo.
#[must_use]
pub fn copyleft_restricted_licenses() -> &'static [&'static str] {
    &[
        "GPL-2.0",
        "GPL-2.0-only",
        "GPL-2.0-or-later",
        "GPL-3.0",
        "GPL-3.0-only",
        "GPL-3.0-or-later",
        "AGPL-3.0",
        "AGPL-3.0-only",
        "AGPL-3.0-or-later",
        "SSPL-1.0",
    ]
}

/// Cláusulas opcionales que, si se cumplen, disparan el efecto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionClauses {
    require_signed: bool,
    require_verified: bool,
    min_finding: Option<AssaySeverity>,
    forbidden_licenses: Vec<String>,
    profile: Option<AdmissionProfile>,
}

impl AdmissionClauses {
    /// Ninguna cláusula extra: solo el predicado de firma legado.
    #[must_use]
    pub fn from_predicate(predicate: AdmissionPredicate) -> Self {
        Self {
            require_signed: matches!(predicate, AdmissionPredicate::NotSigned),
            require_verified: matches!(predicate, AdmissionPredicate::NotVerified),
            min_finding: None,
            forbidden_licenses: Vec::new(),
            profile: None,
        }
    }

    /// Construye las cláusulas a partir de campos sueltos.
    #[must_use]
    pub fn new(
        require_signed: bool,
        require_verified: bool,
        min_finding: Option<AssaySeverity>,
        forbidden_licenses: Vec<String>,
        profile: Option<AdmissionProfile>,
    ) -> Self {
        Self {
            require_signed,
            require_verified,
            min_finding,
            forbidden_licenses: normalize_licenses(forbidden_licenses),
            profile,
        }
    }

    /// `true` si hay que firmar (Cosign / Notation).
    #[must_use]
    pub fn require_signed(&self) -> bool {
        self.require_signed
    }

    /// `true` si hay que verificar la firma contra las claves PEM.
    #[must_use]
    pub fn require_verified(&self) -> bool {
        self.require_verified
    }

    /// Umbral de hallazgo OSV, si la cláusula está armada.
    #[must_use]
    pub fn min_finding(&self) -> Option<AssaySeverity> {
        self.min_finding
    }

    /// Licencias denegadas (ids SPDX).
    #[must_use]
    pub fn forbidden_licenses(&self) -> &[String] {
        &self.forbidden_licenses
    }

    /// Perfil que rellenó estas cláusulas, si se eligió uno.
    #[must_use]
    pub fn profile(&self) -> Option<AdmissionProfile> {
        self.profile
    }

    /// Predicado legado para la columna `predicate`.
    #[must_use]
    pub fn legacy_predicate(&self) -> AdmissionPredicate {
        if self.require_verified && !self.require_signed {
            AdmissionPredicate::NotVerified
        } else {
            AdmissionPredicate::NotSigned
        }
    }

    /// Codifica las cláusulas para persistirlas.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut parts = vec![
            format!("signed={}", u8::from(self.require_signed)),
            format!("verified={}", u8::from(self.require_verified)),
        ];
        if let Some(severity) = self.min_finding {
            parts.push(format!("finding={}", severity.as_str()));
        }
        if !self.forbidden_licenses.is_empty() {
            parts.push(format!("licenses={}", self.forbidden_licenses.join(",")));
        }
        if let Some(profile) = self.profile {
            parts.push(format!("profile={}", profile.as_str()));
        }
        parts.join("|")
    }

    /// Restaura las cláusulas persistidas. Vacío = `None` (usar el predicado).
    #[must_use]
    pub fn decode(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut require_signed = false;
        let mut require_verified = false;
        let mut min_finding = None;
        let mut forbidden_licenses = Vec::new();
        let mut profile = None;
        for part in trimmed.split('|') {
            let Some((key, raw)) = part.split_once('=') else {
                continue;
            };
            match key {
                "signed" => require_signed = raw == "1" || raw == "true",
                "verified" => require_verified = raw == "1" || raw == "true",
                "finding" => {
                    min_finding = match raw {
                        "critical" => Some(AssaySeverity::Critical),
                        "high" => Some(AssaySeverity::High),
                        "medium" => Some(AssaySeverity::Medium),
                        "low" => Some(AssaySeverity::Low),
                        _ => None,
                    };
                }
                "licenses" => {
                    forbidden_licenses = raw
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToOwned::to_owned)
                        .collect();
                }
                "profile" => profile = AdmissionProfile::parse(raw),
                _ => {}
            }
        }
        Some(Self::new(
            require_signed,
            require_verified,
            min_finding,
            forbidden_licenses,
            profile,
        ))
    }
}

/// Hechos de un pull para evaluar la política.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionFacts {
    /// Hay firma enlazada.
    pub signed: bool,
    /// La firma verifica contra las claves del repositorio.
    pub verified: bool,
    /// `false` en ecosistemas sin Cosign (Cargo, npm…): se ignoran las
    /// cláusulas de firma.
    pub consider_signature: bool,
    /// Hallazgo más grave del ensaye listo. `None` = no hay ensaye o
    /// no aplica: las cláusulas de CVE y licencia no disparan.
    pub max_finding: Option<AssaySeverity>,
    /// Licencias declaradas en el inventario del ensaye listo.
    pub licenses: Vec<String>,
}

/// Motivo por el que una política dispararía.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionHit {
    /// Falta la firma.
    Unsigned,
    /// La firma no verifica.
    Unverified,
    /// Un hallazgo alcanza el umbral.
    Finding(AssaySeverity),
    /// Una licencia está en la lista denegada.
    ForbiddenLicense(String),
}

impl AdmissionHit {
    /// Etiqueta corta para el registro de eventos.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unsigned => "unsigned",
            Self::Unverified => "unverified",
            Self::Finding(_) => "finding",
            Self::ForbiddenLicense(_) => "license",
        }
    }
}

/// Una regla de admisión de un repositorio.
///
/// Sin activar, no se aplica en el pull: sirve para guardarla y
/// previsualizar el impacto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPolicy {
    enabled: bool,
    when: AdmissionWhen,
    effect: AdmissionEffect,
    clauses: AdmissionClauses,
}

impl AdmissionPolicy {
    /// Regla inactiva: pull + no firmada + denegar, desconectada.
    #[must_use]
    pub fn inactive() -> Self {
        Self {
            enabled: false,
            when: AdmissionWhen::Pull,
            effect: AdmissionEffect::Deny,
            clauses: AdmissionClauses::from_predicate(AdmissionPredicate::NotSigned),
        }
    }

    /// Perfil `OpenChain` seguridad (ISO/IEC 18974): hallazgo ≥ Medium, avisar.
    ///
    /// Queda desactivada: hay que simular y activar a propósito.
    #[must_use]
    pub fn profile_openchain_security() -> Self {
        Self {
            enabled: false,
            when: AdmissionWhen::Pull,
            effect: AdmissionEffect::Warn,
            clauses: AdmissionClauses::new(
                false,
                false,
                Some(AssaySeverity::Medium),
                Vec::new(),
                Some(AdmissionProfile::OpenChainSecurity),
            ),
        }
    }

    /// Perfil de copyleft restrictivo: deniega GPL/AGPL/SSPL.
    #[must_use]
    pub fn profile_copyleft_restrict() -> Self {
        Self {
            enabled: false,
            when: AdmissionWhen::Pull,
            effect: AdmissionEffect::Deny,
            clauses: AdmissionClauses::new(
                false,
                false,
                None,
                copyleft_restricted_licenses()
                    .iter()
                    .map(|item| (*item).to_string())
                    .collect(),
                Some(AdmissionProfile::CopyleftRestrict),
            ),
        }
    }

    /// Perfil conservador: solo Critical, denegar.
    #[must_use]
    pub fn profile_critical_only() -> Self {
        Self {
            enabled: false,
            when: AdmissionWhen::Pull,
            effect: AdmissionEffect::Deny,
            clauses: AdmissionClauses::new(
                false,
                false,
                Some(AssaySeverity::Critical),
                Vec::new(),
                Some(AdmissionProfile::CriticalOnly),
            ),
        }
    }

    /// Construye una política a partir de etiquetas persistidas.
    ///
    /// # Errors
    ///
    /// Devuelve [`AdmissionPolicyError`] si alguna etiqueta no es válida.
    pub fn parse(
        enabled: bool,
        when: &str,
        predicate: &str,
        effect: &str,
    ) -> Result<Self, AdmissionPolicyError> {
        let predicate = AdmissionPredicate::parse(predicate)?;
        Ok(Self {
            enabled,
            when: AdmissionWhen::parse(when)?,
            effect: AdmissionEffect::parse(effect)?,
            clauses: AdmissionClauses::from_predicate(predicate),
        })
    }

    /// Construye una política con cláusulas explícitas.
    ///
    /// # Errors
    ///
    /// Devuelve [`AdmissionPolicyError`] si `when` o `effect` no son válidos.
    pub fn compose(
        enabled: bool,
        when: &str,
        effect: &str,
        clauses: AdmissionClauses,
    ) -> Result<Self, AdmissionPolicyError> {
        Ok(Self {
            enabled,
            when: AdmissionWhen::parse(when)?,
            effect: AdmissionEffect::parse(effect)?,
            clauses,
        })
    }

    /// Restaura una fila persistida, usando `clauses` si no está vacío.
    ///
    /// # Errors
    ///
    /// Devuelve [`AdmissionPolicyError`] si alguna etiqueta no es válida.
    pub fn parse_stored(
        enabled: bool,
        when: &str,
        predicate: &str,
        effect: &str,
        clauses: &str,
    ) -> Result<Self, AdmissionPolicyError> {
        let mut policy = Self::parse(enabled, when, predicate, effect)?;
        if let Some(decoded) = AdmissionClauses::decode(clauses) {
            policy.clauses = decoded;
        }
        Ok(policy)
    }

    /// `true` si la regla está armada.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Momento de evaluación.
    #[must_use]
    pub fn when(&self) -> AdmissionWhen {
        self.when
    }

    /// Predicado legado (firma).
    #[must_use]
    pub fn predicate(&self) -> AdmissionPredicate {
        self.clauses.legacy_predicate()
    }

    /// Efecto si la condición se cumple.
    #[must_use]
    pub fn effect(&self) -> AdmissionEffect {
        self.effect
    }

    /// Cláusulas armadas.
    #[must_use]
    pub fn clauses(&self) -> &AdmissionClauses {
        &self.clauses
    }

    /// Efecto que se aplicaría en un pull de un artefacto con `signed`
    /// y `verified`.
    ///
    /// Ignora `enabled`: sirve para el dry-run («si activo esto…»).
    /// Solo mira la firma; usa [`Self::preview_facts`] para CVE/licencia.
    #[must_use]
    pub fn preview_pull(&self, signed: bool, verified: bool) -> Option<AdmissionEffect> {
        self.preview_facts(&AdmissionFacts {
            signed,
            verified,
            consider_signature: true,
            max_finding: None,
            licenses: Vec::new(),
        })
        .map(|_| self.effect)
    }

    /// Primera cláusula que dispara, ignorando `enabled`.
    #[must_use]
    pub fn preview_facts(&self, facts: &AdmissionFacts) -> Option<AdmissionHit> {
        if !matches!(self.when, AdmissionWhen::Pull) {
            return None;
        }
        if facts.consider_signature {
            if self.clauses.require_signed && !facts.signed {
                return Some(AdmissionHit::Unsigned);
            }
            if self.clauses.require_verified && !facts.verified {
                return Some(AdmissionHit::Unverified);
            }
        }
        if let Some(threshold) = self.clauses.min_finding
            && let Some(severity) = facts.max_finding
            && severity.meets_threshold(threshold)
        {
            return Some(AdmissionHit::Finding(severity));
        }
        for denied in &self.clauses.forbidden_licenses {
            if facts
                .licenses
                .iter()
                .any(|license| license_matches(license, denied))
            {
                return Some(AdmissionHit::ForbiddenLicense(denied.clone()));
            }
        }
        None
    }

    /// Efecto que **bloquea** un pull ahora mismo (regla activa + deny).
    #[must_use]
    pub fn deny_pull(&self, signed: bool, verified: bool) -> bool {
        matches!(
            self.apply_pull(signed, verified),
            Some(AdmissionEffect::Deny)
        )
    }

    /// Efecto que se aplica ahora mismo (regla activa).
    #[must_use]
    pub fn apply_pull(&self, signed: bool, verified: bool) -> Option<AdmissionEffect> {
        if !self.enabled {
            return None;
        }
        self.preview_pull(signed, verified)
    }

    /// Efecto y motivo si la regla está activa.
    #[must_use]
    pub fn apply_facts(&self, facts: &AdmissionFacts) -> Option<(AdmissionEffect, AdmissionHit)> {
        if !self.enabled {
            return None;
        }
        self.preview_facts(facts).map(|hit| (self.effect, hit))
    }
}

fn normalize_licenses(licenses: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for license in licenses {
        let license = license.trim();
        if license.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(license))
        {
            out.push(license.to_string());
        }
    }
    out
}

fn license_matches(declared: &str, denied: &str) -> bool {
    declared.trim().eq_ignore_ascii_case(denied.trim())
}

/// Un aviso o una denegación registrados en un pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionEvent {
    id: AdmissionEventId,
    repository_id: RepositoryId,
    name: String,
    reference: String,
    effect: AdmissionEffect,
    reason: String,
    created_at: String,
}

impl AdmissionEvent {
    /// Construye un evento ya persistido o recién emitido.
    #[must_use]
    pub fn from_parts(
        id: AdmissionEventId,
        repository_id: RepositoryId,
        name: impl Into<String>,
        reference: impl Into<String>,
        effect: AdmissionEffect,
        reason: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id,
            repository_id,
            name: name.into(),
            reference: reference.into(),
            effect,
            reason: reason.into(),
            created_at: created_at.into(),
        }
    }

    /// Identificador.
    #[must_use]
    pub fn id(&self) -> AdmissionEventId {
        self.id
    }

    /// Repositorio.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Nombre de la imagen o del paquete.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Etiqueta, digest o versión.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// `deny` o `warn`.
    #[must_use]
    pub fn effect(&self) -> AdmissionEffect {
        self.effect
    }

    /// Motivo legible.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Instante RFC 3339.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.created_at
    }
}

impl AdmissionWhen {
    /// Etiqueta persistida.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pull => "pull",
        }
    }

    fn parse(value: &str) -> Result<Self, AdmissionPolicyError> {
        match value {
            "pull" => Ok(Self::Pull),
            other => Err(AdmissionPolicyError::UnknownWhen(other.to_string())),
        }
    }
}

impl AdmissionPredicate {
    /// Etiqueta persistida.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotSigned => "not_signed",
            Self::NotVerified => "not_verified",
        }
    }

    fn parse(value: &str) -> Result<Self, AdmissionPolicyError> {
        match value {
            "not_signed" => Ok(Self::NotSigned),
            "not_verified" => Ok(Self::NotVerified),
            other => Err(AdmissionPolicyError::UnknownPredicate(other.to_string())),
        }
    }
}

impl AdmissionEffect {
    /// Etiqueta persistida.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Warn => "warn",
        }
    }

    fn parse(value: &str) -> Result<Self, AdmissionPolicyError> {
        match value {
            "deny" => Ok(Self::Deny),
            "warn" => Ok(Self::Warn),
            other => Err(AdmissionPolicyError::UnknownEffect(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_never_denies() {
        let policy = AdmissionPolicy::inactive();
        assert!(!policy.deny_pull(false, false));
        assert_eq!(
            policy.preview_pull(false, false),
            Some(AdmissionEffect::Deny)
        );
        assert_eq!(policy.preview_pull(true, false), None);
    }

    #[test]
    fn enabled_deny_blocks_unsigned_pulls() {
        let policy = AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap();
        assert!(policy.deny_pull(false, false));
        assert!(!policy.deny_pull(true, false));
    }

    #[test]
    fn warn_does_not_block() {
        let policy = AdmissionPolicy::parse(true, "pull", "not_signed", "warn").unwrap();
        assert!(!policy.deny_pull(false, false));
        assert_eq!(
            policy.preview_pull(false, false),
            Some(AdmissionEffect::Warn)
        );
        assert_eq!(policy.apply_pull(false, false), Some(AdmissionEffect::Warn));
        assert_eq!(policy.apply_pull(true, false), None);
    }

    #[test]
    fn not_verified_ignores_detect_only_signatures() {
        let policy = AdmissionPolicy::parse(true, "pull", "not_verified", "deny").unwrap();
        assert!(policy.deny_pull(true, false));
        assert!(!policy.deny_pull(true, true));
        assert!(policy.deny_pull(false, false));
    }

    #[test]
    fn rejects_unknown_labels() {
        assert!(matches!(
            AdmissionPolicy::parse(true, "push", "not_signed", "deny"),
            Err(AdmissionPolicyError::UnknownWhen(_))
        ));
        assert!(matches!(
            AdmissionPolicy::parse(true, "pull", "gpl", "deny"),
            Err(AdmissionPolicyError::UnknownPredicate(_))
        ));
        assert!(matches!(
            AdmissionPolicy::parse(true, "pull", "not_signed", "quarantine"),
            Err(AdmissionPolicyError::UnknownEffect(_))
        ));
    }

    #[test]
    fn finding_clause_uses_threshold_and_fail_open() {
        let policy = AdmissionPolicy::profile_openchain_security();
        let missing = AdmissionFacts {
            signed: true,
            verified: true,
            consider_signature: false,
            max_finding: None,
            licenses: Vec::new(),
        };
        assert_eq!(policy.preview_facts(&missing), None);
        let medium = AdmissionFacts {
            max_finding: Some(AssaySeverity::Medium),
            ..missing.clone()
        };
        assert_eq!(
            policy.preview_facts(&medium),
            Some(AdmissionHit::Finding(AssaySeverity::Medium))
        );
        let low = AdmissionFacts {
            max_finding: Some(AssaySeverity::Low),
            ..missing
        };
        assert_eq!(policy.preview_facts(&low), None);
    }

    #[test]
    fn copyleft_profile_matches_declared_license() {
        let policy = AdmissionPolicy::profile_copyleft_restrict();
        let facts = AdmissionFacts {
            signed: true,
            verified: true,
            consider_signature: false,
            max_finding: None,
            licenses: vec!["AGPL-3.0-only".to_string()],
        };
        assert!(matches!(
            policy.preview_facts(&facts),
            Some(AdmissionHit::ForbiddenLicense(_))
        ));
    }

    #[test]
    fn cargo_ignores_signature_clauses() {
        let policy = AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap();
        let facts = AdmissionFacts {
            signed: false,
            verified: false,
            consider_signature: false,
            max_finding: None,
            licenses: Vec::new(),
        };
        assert_eq!(policy.preview_facts(&facts), None);
    }

    #[test]
    fn clauses_roundtrip() {
        let clauses = AdmissionClauses::new(
            false,
            true,
            Some(AssaySeverity::High),
            vec!["GPL-3.0-only".to_string()],
            Some(AdmissionProfile::CriticalOnly),
        );
        let decoded = AdmissionClauses::decode(&clauses.encode()).unwrap();
        assert_eq!(decoded, clauses);
    }
}
