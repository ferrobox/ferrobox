use std::collections::HashSet;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::package_coordinate::PackageEcosystem;
use ferrobox_domain::repository::RepositoryKind;
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

/// Motivos por los que un conjunto de miembros de `Alloy` no es válido.
#[derive(Debug, Error)]
pub enum ResolveAlloyMembersError {
    /// Un `Alloy` necesita al menos un repositorio miembro.
    #[error("an Alloy repository must aggregate at least one member repository")]
    EmptyAlloy,

    /// Uno de los miembros indicados no existe.
    #[error("alloy member repository does not exist")]
    MemberNotFound,

    /// Un miembro no comparte el ecosistema del `Alloy`.
    #[error("alloy members must use the same package ecosystem")]
    MemberEcosystemMismatch,

    /// Un `Alloy` no puede agregar a otro `Alloy` (evita ciclos).
    #[error("alloy members must be Forge or Mirror repositories")]
    NestedAlloy,

    /// Fallo al consultar el almacén de repositorios.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

/// Valida y deduplica los miembros de un `Alloy`: mismo ecosistema,
/// solo `Forge` o `Mirror`, y al menos uno.
///
/// # Errors
///
/// Devuelve [`ResolveAlloyMembersError`] si la lista está vacía, un
/// miembro no existe, no comparte ecosistema, o es otro `Alloy`.
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
