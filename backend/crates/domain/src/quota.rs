//! Storage quota for a repository.

use thiserror::Error;

/// Maximum accepted when saving a quota (10 TiB).
const MAX_LIMIT_BYTES: u64 = 10 * 1024 * 1024 * 1024 * 1024;

/// Byte cap that a repository may occupy on disk.
///
/// `None` means no limit. The quota counts **all** binaries
/// in the repository, including those no longer in the catalog until
/// garbage collection runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuota {
    limit_bytes: Option<u64>,
}

/// Reasons why a quota is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StorageQuotaError {
    /// The cap is outside the allowed range.
    #[error("quota limit must be between 1 and {max} bytes, got {actual}")]
    LimitOutOfRange {
        /// Maximum allowed.
        max: u64,
        /// Value received.
        actual: u64,
    },
}

impl StorageQuota {
    /// No cap: any `publish` fits.
    #[must_use]
    pub fn unlimited() -> Self {
        Self { limit_bytes: None }
    }

    /// Builds a quota. `None` = unlimited.
    ///
    /// # Errors
    ///
    /// [`StorageQuotaError::LimitOutOfRange`] if the cap is 0 or greater than 10 TiB.
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

    /// Cap in bytes, or `None` if there is no limit.
    #[must_use]
    pub fn limit_bytes(&self) -> Option<u64> {
        self.limit_bytes
    }

    /// `true` if `used + additional` fits (or there is no cap).
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
