//! Write-once / read-many lock for a repository.

/// Per-repository WORM switch.
///
/// When enabled, delete, yank/unyank, retention apply, and per-repository
/// garbage collection are rejected. Turning the lock off is still allowed
/// in this first cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WormPolicy {
    enabled: bool,
}

impl WormPolicy {
    /// Lock off: mutations that drop published bits are allowed.
    #[must_use]
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Builds a policy from the stored flag.
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// `true` when the repository must stay immutable.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        assert!(!WormPolicy::disabled().enabled());
        assert!(WormPolicy::new(true).enabled());
    }
}
