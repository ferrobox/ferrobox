//! Política de retención de versiones de un repositorio.

use thiserror::Error;

const MAX_KEEP_LAST: u32 = 10_000;
const MAX_KEEP_DAYS: u32 = 3_650;

/// Cuántas versiones conservar por paquete.
///
/// Una versión se conserva si cumple **cualquiera** de las reglas
/// definidas: está entre las `keep_last` más recientes, o se publicó
/// hace `keep_days` días o menos. Sin reglas, se conserva todo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    keep_last: Option<u32>,
    keep_days: Option<u32>,
}

/// Motivos por los que una política de retención no es válida.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RetentionPolicyError {
    /// `keep_last` está fuera del rango permitido.
    #[error("keep_last must be between 1 and {max}, got {actual}")]
    KeepLastOutOfRange {
        /// Máximo permitido.
        max: u32,
        /// Valor recibido.
        actual: u32,
    },

    /// `keep_days` está fuera del rango permitido.
    #[error("keep_days must be between 1 and {max}, got {actual}")]
    KeepDaysOutOfRange {
        /// Máximo permitido.
        max: u32,
        /// Valor recibido.
        actual: u32,
    },
}

impl RetentionPolicy {
    /// Conserva todas las versiones (no borra nada al aplicar).
    #[must_use]
    pub fn keep_all() -> Self {
        Self {
            keep_last: None,
            keep_days: None,
        }
    }

    /// Construye una política a partir de límites opcionales.
    ///
    /// # Errors
    ///
    /// Devuelve [`RetentionPolicyError`] si un límite está fuera de rango.
    pub fn new(
        keep_last: Option<u32>,
        keep_days: Option<u32>,
    ) -> Result<Self, RetentionPolicyError> {
        if let Some(actual) = keep_last
            && !(1..=MAX_KEEP_LAST).contains(&actual)
        {
            return Err(RetentionPolicyError::KeepLastOutOfRange {
                max: MAX_KEEP_LAST,
                actual,
            });
        }
        if let Some(actual) = keep_days
            && !(1..=MAX_KEEP_DAYS).contains(&actual)
        {
            return Err(RetentionPolicyError::KeepDaysOutOfRange {
                max: MAX_KEEP_DAYS,
                actual,
            });
        }
        Ok(Self {
            keep_last,
            keep_days,
        })
    }

    /// Número máximo de versiones recientes a conservar por paquete.
    #[must_use]
    pub fn keep_last(self) -> Option<u32> {
        self.keep_last
    }

    /// Edad máxima en días de las versiones a conservar.
    #[must_use]
    pub fn keep_days(self) -> Option<u32> {
        self.keep_days
    }

    /// `true` si no hay ninguna regla y, por tanto, no se borra nada.
    #[must_use]
    pub fn is_keep_all(self) -> bool {
        self.keep_last.is_none() && self.keep_days.is_none()
    }

    /// Conserva la versión si encaja en alguna regla.
    ///
    /// `rank_from_newest` es 0 para la más reciente del paquete.
    /// `age_days` es la edad desde que se indexó, en días completos.
    #[must_use]
    pub fn keeps(self, rank_from_newest: u32, age_days: u64) -> bool {
        match (self.keep_last, self.keep_days) {
            (None, None) => true,
            (Some(limit), None) => rank_from_newest < limit,
            (None, Some(days)) => age_days <= u64::from(days),
            (Some(limit), Some(days)) => {
                rank_from_newest < limit || age_days <= u64::from(days)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_all_preserves_every_version() {
        let policy = RetentionPolicy::keep_all();
        assert!(policy.keeps(0, 0));
        assert!(policy.keeps(99, 10_000));
        assert!(policy.is_keep_all());
    }

    #[test]
    fn keep_last_drops_older_ranks() {
        let policy = RetentionPolicy::new(Some(2), None).unwrap();
        assert!(policy.keeps(0, 400));
        assert!(policy.keeps(1, 400));
        assert!(!policy.keeps(2, 400));
    }

    #[test]
    fn keep_days_drops_stale_versions() {
        let policy = RetentionPolicy::new(None, Some(30)).unwrap();
        assert!(policy.keeps(99, 30));
        assert!(!policy.keeps(0, 31));
    }

    #[test]
    fn combined_rules_keep_if_either_matches() {
        let policy = RetentionPolicy::new(Some(1), Some(7)).unwrap();
        assert!(policy.keeps(0, 400), "newest is kept even if stale");
        assert!(policy.keeps(5, 3), "recent is kept even if not newest");
        assert!(!policy.keeps(5, 40), "old and not newest is dropped");
    }

    #[test]
    fn rejects_zero_and_too_large_limits() {
        assert!(matches!(
            RetentionPolicy::new(Some(0), None),
            Err(RetentionPolicyError::KeepLastOutOfRange { .. })
        ));
        assert!(matches!(
            RetentionPolicy::new(None, Some(10_000)),
            Err(RetentionPolicyError::KeepDaysOutOfRange { .. })
        ));
    }
}
