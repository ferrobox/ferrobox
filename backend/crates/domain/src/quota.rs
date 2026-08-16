//! Cuota de almacenamiento de un repositorio.

use thiserror::Error;

/// Máximo aceptado al guardar una cuota (10 TiB).
const MAX_LIMIT_BYTES: u64 = 10 * 1024 * 1024 * 1024 * 1024;

/// Tope de bytes que un repositorio puede ocupar en disco.
///
/// `None` significa sin límite. La cuota cuenta **todos** los binarios
/// del repositorio, también los que ya no están en el catálogo hasta
/// que corre la recolección de basura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuota {
    limit_bytes: Option<u64>,
}

/// Motivos por los que una cuota no es válida.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StorageQuotaError {
    /// El tope está fuera del rango permitido.
    #[error("quota limit must be between 1 and {max} bytes, got {actual}")]
    LimitOutOfRange {
        /// Máximo permitido.
        max: u64,
        /// Valor recibido.
        actual: u64,
    },
}

impl StorageQuota {
    /// Sin tope: cualquier `publish` cabe.
    #[must_use]
    pub fn unlimited() -> Self {
        Self { limit_bytes: None }
    }

    /// Construye una cuota. `None` = ilimitada.
    ///
    /// # Errors
    ///
    /// [`StorageQuotaError::LimitOutOfRange`] si el tope es 0 o mayor que 10 TiB.
    pub fn new(limit_bytes: Option<u64>) -> Result<Self, StorageQuotaError> {
        if let Some(actual) = limit_bytes
            && !(1..=MAX_LIMIT_BYTES).contains(&actual)
        {
            return Err(StorageQuotaError::LimitOutOfRange {
                max: MAX_LIMIT_BYTES,
                actual,
            });
        }
        Ok(Self { limit_bytes })
    }

    /// Tope en bytes, o `None` si no hay límite.
    #[must_use]
    pub fn limit_bytes(&self) -> Option<u64> {
        self.limit_bytes
    }

    /// `true` si `used + additional` cabe (o no hay tope).
    #[must_use]
    pub fn allows(&self, used_bytes: u64, additional_bytes: u64) -> bool {
        let Some(limit) = self.limit_bytes else {
            return true;
        };
        used_bytes.saturating_add(additional_bytes) <= limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_allows_any_size() {
        assert!(StorageQuota::unlimited().allows(0, u64::MAX));
    }

    #[test]
    fn limited_rejects_overflow() {
        let quota = StorageQuota::new(Some(100)).unwrap();
        assert!(quota.allows(40, 60));
        assert!(!quota.allows(40, 61));
    }

    #[test]
    fn rejects_zero_and_too_large() {
        assert!(StorageQuota::new(Some(0)).is_err());
        assert!(StorageQuota::new(Some(MAX_LIMIT_BYTES + 1)).is_err());
    }
}
