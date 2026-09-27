//! Admission policy for a repository: structured rules that decide
//! whether an artifact may be pulled or promoted into another Forge.
//!
//! Several clauses can be armed at once (signature, OSV finding,
//! denied license). The first one that matches fires the effect.

use thiserror::Error;

use crate::assay::AssaySeverity;
use crate::ids::{AdmissionEventId, RepositoryId};

/// Moment at which the rule is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionWhen {
    /// When resolving a manifest or a package to install it.
    Pull,
}

/// Signature condition persisted for compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPredicate {
    /// The artifact has no linked Cosign / Notation signature.
    NotSigned,
    /// The artifact has no Cosign signature valid against the configured
    /// public keys.
    NotVerified,
}

/// What to do if the condition matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionEffect {
    /// Blocks the operation.
    Deny,
    /// Lets it through and only records it (operational dry-run).
    Warn,
}

/// Reasons why an admission policy is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdmissionPolicyError {
    /// The moment is not one of the supported ones.
    #[error("unsupported admission moment '{0}'")]
    UnknownWhen(String),

    /// The predicate is not one of the supported ones.
    #[error("unsupported admission predicate '{0}'")]
    UnknownPredicate(String),

    /// The effect is not one of the supported ones.
    #[error("unsupported admission effect '{0}'")]
    UnknownEffect(String),

    /// The finding threshold is not valid.
    #[error("unsupported admission finding threshold '{0}'")]
    UnknownFinding(String),
}

/// Identifier of a preconfigured profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionProfile {
    /// ISO/IEC 18974: detect and act on findings ≥ Medium.
    OpenChainSecurity,
    /// Typical inbound policy: strong copyleft denied.
    CopyleftRestrict,
    /// Only Critical findings, deny.
    CriticalOnly,
}

impl AdmissionProfile {
    /// Persisted label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenChainSecurity => "openchain_security",
            Self::CopyleftRestrict => "copyleft_restrict",
            Self::CriticalOnly => "critical_only",
        }
    }

    /// Parses the label.
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

/// SPDX licenses of the restrictive copyleft profile.
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

/// Optional clauses that, if they match, fire the effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionClauses {
    require_signed: bool,
    require_verified: bool,
    min_finding: Option<AssaySeverity>,
    forbidden_licenses: Vec<String>,
    profile: Option<AdmissionProfile>,
}

impl AdmissionClauses {
    /// No extra clauses: only the legacy signature predicate.
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

    /// Builds the clauses from loose fields.
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

    /// `true` if it must be signed (Cosign / Notation).
    #[must_use]
    pub fn require_signed(&self) -> bool {
        self.require_signed
    }

    /// `true` if the signature must be verified against the PEM keys.
    #[must_use]
    pub fn require_verified(&self) -> bool {
        self.require_verified
    }

    /// OSV finding threshold, if the clause is armed.
    #[must_use]
    pub fn min_finding(&self) -> Option<AssaySeverity> {
        self.min_finding
    }

    /// Denied licenses (SPDX ids).
    #[must_use]
    pub fn forbidden_licenses(&self) -> &[String] {
        &self.forbidden_licenses
    }

    /// Profile that filled these clauses, if one was chosen.
    #[must_use]
    pub fn profile(&self) -> Option<AdmissionProfile> {
        self.profile
    }

    /// Legacy predicate for the `predicate` column.
    #[must_use]
    pub fn legacy_predicate(&self) -> AdmissionPredicate {
        if self.require_verified && !self.require_signed {
            AdmissionPredicate::NotVerified
        } else {
            AdmissionPredicate::NotSigned
        }
    }

    /// Encodes the clauses so they can be persisted.
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

    /// Restores persisted clauses. Empty = `None` (use the predicate).
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

/// Facts of a pull for evaluating the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionFacts {
    /// There is a linked signature.
    pub signed: bool,
    /// The signature verifies against the repository keys.
    pub verified: bool,
    /// `false` in ecosystems without Cosign (Cargo, npm…): signature
    /// clauses are ignored.
    pub consider_signature: bool,
    /// Most severe finding of the ready assay. `None` = there is no assay or
    /// it does not apply: CVE and license clauses do not fire.
    pub max_finding: Option<AssaySeverity>,
    /// Licenses declared in the inventory of the ready assay.
    pub licenses: Vec<String>,
}

/// Reason why a policy would fire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionHit {
    /// The signature is missing.
    Unsigned,
    /// The signature does not verify.
    Unverified,
    /// A finding meets the threshold.
    Finding(AssaySeverity),
    /// A license is on the denied list.
    ForbiddenLicense(String),
}

impl AdmissionHit {
    /// Short label for the event log.
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

/// An admission rule of a repository.
///
/// While disabled, it is not applied on pull: it is used to save it and
/// preview the impact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPolicy {
    enabled: bool,
    when: AdmissionWhen,
    effect: AdmissionEffect,
    clauses: AdmissionClauses,
}

impl AdmissionPolicy {
    /// Inactive rule: pull + unsigned + deny, disconnected.
    #[must_use]
    pub fn inactive() -> Self {
        Self {
            enabled: false,
            when: AdmissionWhen::Pull,
            effect: AdmissionEffect::Deny,
            clauses: AdmissionClauses::from_predicate(AdmissionPredicate::NotSigned),
        }
    }

    /// `OpenChain` security profile (ISO/IEC 18974): finding ≥ Medium, warn.
    ///
    /// It is left disabled: it must be simulated and enabled on purpose.
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

    /// Restrictive copyleft profile: denies GPL/AGPL/SSPL.
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

    /// Conservative profile: only Critical, deny.
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

    /// Builds a policy from persisted labels.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionPolicyError`] if any label is not valid.
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

    /// Builds a policy with explicit clauses.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionPolicyError`] if `when` or `effect` are not valid.
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

    /// Restores a persisted row, using `clauses` if it is not empty.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionPolicyError`] if any label is not valid.
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

    /// `true` if the rule is armed.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Evaluation moment.
    #[must_use]
    pub fn when(&self) -> AdmissionWhen {
        self.when
    }

    /// Legacy predicate (signature).
    #[must_use]
    pub fn predicate(&self) -> AdmissionPredicate {
        self.clauses.legacy_predicate()
    }

    /// Effect if the condition matches.
    #[must_use]
    pub fn effect(&self) -> AdmissionEffect {
        self.effect
    }

    /// Armed clauses.
    #[must_use]
    pub fn clauses(&self) -> &AdmissionClauses {
        &self.clauses
    }

    /// Effect that would be applied on a pull of an artifact with `signed`
    /// and `verified`.
    ///
    /// Ignores `enabled`: used for the dry-run ("if I turn this on…").
    /// Only looks at the signature; use [`Self::preview_facts`] for CVE/license.
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

    /// First clause that fires, ignoring `enabled`.
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

    /// Effect that **blocks** a pull right now (active rule + deny).
    #[must_use]
    pub fn deny_pull(&self, signed: bool, verified: bool) -> bool {
        matches!(
            self.apply_pull(signed, verified),
            Some(AdmissionEffect::Deny)
        )
    }

    /// Effect that is applied right now (active rule).
    #[must_use]
    pub fn apply_pull(&self, signed: bool, verified: bool) -> Option<AdmissionEffect> {
        if !self.enabled {
            return None;
        }
        self.preview_pull(signed, verified)
    }

    /// Effect and reason if the rule is active.
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

/// A warning or a denial recorded on a pull.
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
    /// Builds an event that is already persisted or just emitted.
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

    /// Identifier.
    #[must_use]
    pub fn id(&self) -> AdmissionEventId {
        self.id
    }

    /// Repository.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Image or package name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tag, digest, or version.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// `deny` or `warn`.
    #[must_use]
    pub fn effect(&self) -> AdmissionEffect {
        self.effect
    }

    /// Human-readable reason.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// RFC 3339 timestamp.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.created_at
    }
}

impl AdmissionWhen {
    /// Persisted label.
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
    /// Persisted label.
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
    /// Persisted label.
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
