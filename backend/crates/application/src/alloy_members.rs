use std::collections::HashSet;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Reasons a set of `Alloy` members is invalid.
#[derive(Debug, Error)]
pub enum ResolveAlloyMembersError {
    /// An `Alloy` needs at least one member repository.
    #[error("an Alloy repository must aggregate at least one member repository")]
    EmptyAlloy,

    /// One of the given members does not exist.
    #[error("alloy member repository does not exist")]
    MemberNotFound,

    /// A member does not share the `Alloy` ecosystem.
    #[error("alloy members must use the same package ecosystem")]
    MemberEcosystemMismatch,

    /// An `Alloy` cannot aggregate another `Alloy` (avoids cycles).
    #[error("alloy members must be Forge or Mirror repositories")]
    NestedAlloy,

    /// Failed to query the repository store.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

/// Validates and deduplicates the members of an `Alloy`: same ecosystem,
/// only `Forge` or `Mirror`, and at least one.
///
/// # Errors
///
/// Returns [`ResolveAlloyMembersError`] if the list is empty, a member
/// does not exist, does not share the ecosystem, or is another `Alloy`.
pub async fn resolve_alloy_members(
    repository_store: &dyn RepositoryStore,
    ecosystem: PackageEcosystem,
    members: Vec<RepositoryId>,
) -> Result<Vec<RepositoryId>, ResolveAlloyMembersError> {
    if members.is_empty() {
        return Err(ResolveAlloyMembersError::EmptyAlloy);
    }

    let mut unique = HashSet::new();
    let mut resolved = Vec::new();
    for member_id in members {
        if !unique.insert(member_id) {
            continue;
        }
        let member = repository_store
            .find_by_id(member_id)
            .await?
            .ok_or(ResolveAlloyMembersError::MemberNotFound)?;
        if member.ecosystem() != ecosystem {
            return Err(ResolveAlloyMembersError::MemberEcosystemMismatch);
        }
        if matches!(member.kind(), RepositoryKind::Alloy { .. }) {
            return Err(ResolveAlloyMembersError::NestedAlloy);
        }
        resolved.push(member_id);
    }

    if resolved.is_empty() {
        return Err(ResolveAlloyMembersError::EmptyAlloy);
    }

    Ok(resolved)
}
