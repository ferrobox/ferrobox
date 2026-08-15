//! Cuarentena de versiones recién publicadas en registros públicos.
//!
//! Los Mirrors npm y `PyPI` ocultan (y no descargan) una versión cuya
//! fecha de publicación todavía no cumple `min_age_days`. Así se reduce
//! la ventana de typosquatting y de paquetes maliciosos recién subidos.
//! Si el *upstream* no informa fecha, la versión se sirve (fail-open)
//! para no romper índices privados sin metadatos de tiempo.

use chrono::{DateTime, Duration, SecondsFormat, Utc};

use super::PackagingError;

/// Política de edad mínima para versiones servidas por un `Mirror`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirrorQuarantine {
    min_age_days: u32,
}

impl MirrorQuarantine {
    /// Política desactivada: se sirve cualquier versión.
    pub const DISABLED: Self = Self { min_age_days: 0 };

    /// Construye la política. `0` la desactiva. El máximo es 3650 días.
    #[must_use]
    pub fn from_days(min_age_days: u32) -> Self {
        Self {
            min_age_days: min_age_days.min(3650),
        }
    }

    /// Días de edad mínima configurados (`0` = desactivada).
    #[must_use]
    pub fn min_age_days(self) -> u32 {
        self.min_age_days
    }

    /// `true` si hay que aplicar la cuarentena.
    #[must_use]
    pub fn is_active(self) -> bool {
        self.min_age_days > 0
    }

    /// Interpreta un instante RFC 3339 (npm `time`, PyPI `upload_time`).
    #[must_use]
    pub fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(value.trim())
            .ok()
            .map(|stamp| stamp.with_timezone(&Utc))
    }

    /// Instante a partir del cual la versión puede servirse, si aún no
    /// cumple la edad mínima.
    #[must_use]
    pub fn held_until(
        self,
        published_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        if !self.is_active() {
            return None;
        }
        let available_at = published_at + Duration::days(i64::from(self.min_age_days));
        (now < available_at).then_some(available_at)
    }

    /// `true` si `published_at` (RFC 3339) está en cuarentena. Sin fecha,
    /// no se retiene.
    #[must_use]
    pub fn is_held(self, published_at: Option<&str>, now: DateTime<Utc>) -> bool {
        let Some(published) = published_at.and_then(Self::parse_timestamp) else {
            return false;
        };
        self.held_until(published, now).is_some()
    }

    /// Falla con [`PackagingError::Quarantined`] si la versión es demasiado
    /// reciente.
    ///
    /// # Errors
    ///
    /// Devuelve [`PackagingError::Quarantined`] cuando la fecha de
    /// publicación aún no cumple `min_age_days`.
    pub fn reject_if_held(
        self,
        package: &str,
        version: &str,
        published_at: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), PackagingError> {
        let Some(published) = published_at.and_then(Self::parse_timestamp) else {
            return Ok(());
        };
        let Some(available_at) = self.held_until(published, now) else {
            return Ok(());
        };
        Err(PackagingError::Quarantined {
            package: package.to_string(),
            version: version.to_string(),
            available_at: available_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(value: &str) -> DateTime<Utc> {
        MirrorQuarantine::parse_timestamp(value).unwrap()
    }

    #[test]
    fn disabled_policy_never_holds() {
        let policy = MirrorQuarantine::DISABLED;
        let published = stamp("2026-08-15T12:00:00Z");
        assert!(policy.held_until(published, published).is_none());
        assert!(!policy.is_held(Some("2026-08-15T12:00:00Z"), published));
    }

    #[test]
    fn recent_publish_is_held_until_min_age() {
        let policy = MirrorQuarantine::from_days(14);
        let published = stamp("2026-08-01T00:00:00Z");
        let now = stamp("2026-08-10T00:00:00Z");
        let until = policy.held_until(published, now).unwrap();
        assert_eq!(until, stamp("2026-08-15T00:00:00Z"));
    }

    #[test]
    fn old_enough_publish_is_released() {
        let policy = MirrorQuarantine::from_days(14);
        let published = stamp("2026-08-01T00:00:00Z");
        let now = stamp("2026-08-15T00:00:00Z");
        assert!(policy.held_until(published, now).is_none());
    }

    #[test]
    fn missing_timestamp_is_not_held() {
        let policy = MirrorQuarantine::from_days(14);
        assert!(!policy.is_held(None, Utc::now()));
        assert!(!policy.is_held(Some("not-a-date"), Utc::now()));
    }

    #[test]
    fn reject_if_held_returns_quarantined_error() {
        let policy = MirrorQuarantine::from_days(14);
        let err = policy
            .reject_if_held(
                "left-pad",
                "1.0.0",
                Some("2026-08-14T00:00:00Z"),
                stamp("2026-08-15T00:00:00Z"),
            )
            .unwrap_err();
        match err {
            PackagingError::Quarantined {
                package,
                version,
                available_at,
            } => {
                assert_eq!(package, "left-pad");
                assert_eq!(version, "1.0.0");
                assert_eq!(available_at, "2026-08-28T00:00:00Z");
            }
            other => panic!("expected Quarantined, got {other:?}"),
        }
    }
}
