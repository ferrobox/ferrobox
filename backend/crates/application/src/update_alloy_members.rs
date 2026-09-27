use std::sync::Arc;

use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::{Repository, RepositoryKind};
use ferrobox_ports::repository_store::{RepositoryStore, RepositoryStoreError};
use thiserror::Error;

use crate::alloy_members::{resolve_alloy_members, ResolveAlloyMembersError};

/// Reasons updating the members of an `Alloy` can fail.
#[derive(Debug, Error)]
pub enum UpdateAlloyMembersError {
    /// The repository does not exist.
    #[error("repository {0} does not exist")]
    NotFound(RepositoryId),

    /// Only an `Alloy` has members that can be updated.
    #[error("only Alloy repositories have members that can be updated")]
    NotAnAlloy,

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

    /// Failed to persist the repository.
    #[error(transparent)]
    Persistence(#[from] RepositoryStoreError),
}

impl From<ResolveAlloyMembersError> for UpdateAlloyMembersError {
    fn from(err: ResolveAlloyMembersError) -> Self {
        match err {
            ResolveAlloyMembersError::EmptyAlloy => Self::EmptyAlloy,
            ResolveAlloyMembersError::MemberNotFound => Self::MemberNotFound,
            ResolveAlloyMembersError::MemberEcosystemMismatch => Self::MemberEcosystemMismatch,
            ResolveAlloyMembersError::NestedAlloy => Self::NestedAlloy,
            ResolveAlloyMembersError::Persistence(inner) => Self::Persistence(inner),
        }
    }
}

/// Use case: replace the members of an `Alloy` repository.
pub struct UpdateAlloyMembersUseCase {
    repository_store: Arc<dyn RepositoryStore>,
}

impl UpdateAlloyMembersUseCase {
    /// Builds the use case from its port.
    #[must_use]
    pub fn new(repository_store: Arc<dyn RepositoryStore>) -> Self {
        Self { repository_store }
    }

    /// Replaces the members of the given `Alloy`, in the given
    /// resolution order.
    ///
    /// # Errors
    ///
    /// Returns [`UpdateAlloyMembersError`] if the repository does not
    /// exist, is not an `Alloy`, or the members are invalid.
    ///
    /// # Panics
    ///
    /// In practice this never panics: the members are already validated
    /// as a non-empty `Alloy` before calling [`Repository::with_kind`].
    pub async fn execute(
        &self,
        repository_id: RepositoryId,
        members: Vec<RepositoryId>,
    ) -> Result<Repository, UpdateAlloyMembersError> {
        let repository = self
            .repository_store
            .find_by_id(repository_id)
            .await?
            .ok_or(UpdateAlloyMembersError::NotFound(repository_id))?;

        if !matches!(repository.kind(), RepositoryKind::Alloy { .. }) {
            return Err(UpdateAlloyMembersError::NotAnAlloy);
        }

        let members =
            resolve_alloy_members(self.repository_store.as_ref(), repository.ecosystem(), members)
                .await?;
        let updated = repository
            .with_kind(RepositoryKind::Alloy { members })
            .expect("resolved Alloy members never violate the empty-Alloy invariant");

        self.repository_store.save(&updated).await?;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrobox_domain::package_coordinate::PackageEcosystem;
    use ferrobox_domain::repository::{RepositoryKind, RepositoryName};
    use ferrobox_ports::repository_store::RepositoryStore;

    use crate::create_repository::{CreateRepositoryKind, CreateRepositoryUseCase};
    use crate::test_support::InMemoryRepositoryStore;

    use super::*;

    async fn fixture() -> (
        UpdateAlloyMembersUseCase,
        Arc<InMemoryRepositoryStore>,
        RepositoryId,
        RepositoryId,
        RepositoryId,
    ) {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let create = CreateRepositoryUseCase::new(store.clone());
        let first = create
            .execute(
                RepositoryName::parse("crates-local").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let second = create
            .execute(
                RepositoryName::parse("crates-other").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Forge,
            )
            .await
            .unwrap();
        let alloy = create
            .execute(
                RepositoryName::parse("crates-alloy").unwrap(),
                PackageEcosystem::Cargo,
                CreateRepositoryKind::Alloy {
                    members: vec![first],
                },
            )
            .await
            .unwrap();
        (
            UpdateAlloyMembersUseCase::new(store.clone()),
            store,
            first,
            second,
            alloy,
        )
    }

    #[tokio::test]
    async fn replaces_alloy_members_in_order() {
        let (use_case, store, first, second, alloy_id) = fixture().await;

        let updated = use_case
            .execute(alloy_id, vec![second, first])
            .await
            .unwrap();

        match updated.kind() {
            RepositoryKind::Alloy { members } => assert_eq!(members, &vec![second, first]),
            other => panic!("expected alloy, got {other:?}"),
        }

        let persisted = store.find_by_id(alloy_id).await.unwrap().unwrap();
        match persisted.kind() {
            RepositoryKind::Alloy { members } => assert_eq!(members, &vec![second, first]),
            other => panic!("expected alloy, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_updating_a_forge() {
        let (use_case, _, first, _, _) = fixture().await;
        let result = use_case.execute(first, vec![first]).await;
        assert!(matches!(result, Err(UpdateAlloyMembersError::NotAnAlloy)));
    }

    #[tokio::test]
    async fn rejects_an_empty_member_list() {
        let (use_case, _, _, _, alloy_id) = fixture().await;
        let result = use_case.execute(alloy_id, vec![]).await;
        assert!(matches!(result, Err(UpdateAlloyMembersError::EmptyAlloy)));
    }

    #[tokio::test]
    async fn rejects_a_missing_repository() {
        let store = Arc::new(InMemoryRepositoryStore::default());
        let use_case = UpdateAlloyMembersUseCase::new(store);
        let result = use_case
            .execute(RepositoryId::new(), vec![RepositoryId::new()])
            .await;
        assert!(matches!(result, Err(UpdateAlloyMembersError::NotFound(_))));
    }
}
