//! Política de admisión de un repositorio: reglas estructuradas que
//! deciden si un artefacto puede bajarse (o, más adelante, publicarse).
//!
//! El primer predicado es «no está firmada» y el primer momento es el
//! pull. El modelo deja sitio para más condiciones sin cambiar el
//! contrato de evaluación.

use thiserror::Error;

/// Momento en el que se evalúa la regla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionWhen {
    /// Al resolver un manifiesto o un paquete para instalarlo.
    Pull,
}

/// Condición que, si se cumple, dispara el efecto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPredicate {
    /// El artefacto no tiene una firma Cosign / Notation enlazada.
    NotSigned,
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
}

/// Una regla de admisión de un repositorio.
///
/// Sin activar, no se aplica en el pull: sirve para guardarla y
/// previsualizar el impacto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionPolicy {
    enabled: bool,
    when: AdmissionWhen,
    predicate: AdmissionPredicate,
    effect: AdmissionEffect,
}

impl AdmissionPolicy {
    /// Regla inactiva: pull + no firmada + denegar, desconectada.
    #[must_use]
    pub fn inactive() -> Self {
        Self {
            enabled: false,
            when: AdmissionWhen::Pull,
            predicate: AdmissionPredicate::NotSigned,
            effect: AdmissionEffect::Deny,
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
        Ok(Self {
            enabled,
            when: AdmissionWhen::parse(when)?,
            predicate: AdmissionPredicate::parse(predicate)?,
            effect: AdmissionEffect::parse(effect)?,
        })
    }

    /// `true` si la regla está armada.
    #[must_use]
    pub fn enabled(self) -> bool {
        self.enabled
    }

    /// Momento de evaluación.
    #[must_use]
    pub fn when(self) -> AdmissionWhen {
        self.when
    }

    /// Condición que dispara el efecto.
    #[must_use]
    pub fn predicate(self) -> AdmissionPredicate {
        self.predicate
    }

    /// Efecto si la condición se cumple.
    #[must_use]
    pub fn effect(self) -> AdmissionEffect {
        self.effect
    }

    /// Efecto que se aplicaría en un pull de un artefacto con `signed`.
    ///
    /// Ignora `enabled`: sirve para el dry-run («si activo esto…»).
    #[must_use]
    pub fn preview_pull(self, signed: bool) -> Option<AdmissionEffect> {
        if !matches!(self.when, AdmissionWhen::Pull) {
            return None;
        }
        match self.predicate {
            AdmissionPredicate::NotSigned if !signed => Some(self.effect),
            AdmissionPredicate::NotSigned => None,
        }
    }

    /// Efecto que **bloquea** un pull ahora mismo (regla activa + deny).
    #[must_use]
    pub fn deny_pull(self, signed: bool) -> bool {
        self.enabled && matches!(self.preview_pull(signed), Some(AdmissionEffect::Deny))
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
        }
    }

    fn parse(value: &str) -> Result<Self, AdmissionPolicyError> {
        match value {
            "not_signed" => Ok(Self::NotSigned),
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
        assert!(!policy.deny_pull(false));
        assert_eq!(policy.preview_pull(false), Some(AdmissionEffect::Deny));
        assert_eq!(policy.preview_pull(true), None);
    }

    #[test]
    fn enabled_deny_blocks_unsigned_pulls() {
        let policy = AdmissionPolicy::parse(true, "pull", "not_signed", "deny").unwrap();
        assert!(policy.deny_pull(false));
        assert!(!policy.deny_pull(true));
    }

    #[test]
    fn warn_does_not_block() {
        let policy = AdmissionPolicy::parse(true, "pull", "not_signed", "warn").unwrap();
        assert!(!policy.deny_pull(false));
        assert_eq!(policy.preview_pull(false), Some(AdmissionEffect::Warn));
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
}
