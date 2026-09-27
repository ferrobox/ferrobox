//! Version retention policy for a repository.

use thiserror::Error;

const MAX_KEEP_LAST: u32 = 10_000;
const MAX_KEEP_DAYS: u32 = 3_650;

/// How many versions to keep per package.
///
/// A version is kept if it matches **any** of the defined
/// rules: it is among the `keep_last` most recent, or it was published
/// `keep_days` days ago or less. With no rules, everything is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    keep_last: Option<u32>,
    keep_days: Option<u32>,
}

/// Reasons why a retention policy is not valid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RetentionPolicyError {
    /// `keep_last` is outside the allowed range.
    #[error("keep_last must be between 1 and {max}, got {actual}")]
    KeepLastOutOfRange {
        /// Maximum allowed.
        max: u32,
        /// Value received.
        actual: u32,
    },

    /// `keep_days` is outside the allowed range.
    #[error("keep_days must be between 1 and {max}, got {actual}")]
    KeepDaysOutOfRange {
        /// Maximum allowed.
        max: u32,
        /// Value received.
        actual: u32,
    },
}

impl RetentionPolicy {
    /// Keeps all versions (deletes nothing when applied).
    #[must_use]
    pub fn keep_all() -> Self {
        Self {
            keep_last: None,
            keep_days: None,
        }
    }

    /// Builds a policy from optional limits.
    ///
    /// # Errors
    ///
    /// Returns [`RetentionPolicyError`] if a limit is out of range.
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

    /// Maximum number of recent versions to keep per package.
    #[must_use]
    pub fn keep_last(self) -> Option<u32> {
        self.keep_last
    }

    /// Maximum age in days of the versions to keep.
    #[must_use]
    pub fn keep_days(self) -> Option<u32> {
        self.keep_days
    }

    /// `true` if there is no rule and, therefore, nothing is deleted.
    #[must_use]
    pub fn is_keep_all(self) -> bool {
        self.keep_last.is_none() && self.keep_days.is_none()
    }

    /// Keeps the version if it matches any rule.
    ///
    /// `rank_from_newest` is 0 for the most recent of the package.
    /// `age_days` is the age since it was indexed, in whole days.
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
