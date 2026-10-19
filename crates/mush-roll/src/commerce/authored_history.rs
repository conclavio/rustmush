//! Same-dispatch source/diff access for an exact owned marked saved revision.
//! This observation is not a current-draft, copy, execution or filing grant.
use super::families::CommerceFamilies;
use super::marked_rating::{SavedParentObservation, resolve_saved_parent};
use super::rating::{RatingCompatibility, RatingFamilies};
use crate::{Revision, RevisionRef, RollStore};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionRef, ModError, ObjId, Registrar};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Source,
    Diff,
}

/// No requester, owner, grant or source is accepted from the caller.
#[derive(Serialize)]
pub struct AuthoredSavedRequest {
    revision: RevisionRef,
    operation: Operation,
}
impl AuthoredSavedRequest {
    pub fn source(revision: RevisionRef) -> Self {
        Self {
            revision,
            operation: Operation::Source,
        }
    }
    pub fn diff(revision: RevisionRef) -> Self {
        Self {
            revision,
            operation: Operation::Diff,
        }
    }
}
/// Constructed only by the actual registered reader, without Clone/Deserialize.
#[derive(Debug, Serialize)]
pub struct AuthoredSavedAccess {
    operation: Operation,
    saved: SavedParentObservation,
}
impl AuthoredSavedAccess {
    pub fn actor(&self) -> ObjId {
        self.saved.grant().actor()
    }
    pub fn revision(&self) -> &Revision {
        self.saved.revision()
    }
    /// Only compares evidence; the other value must be a new owner read at use.
    pub fn same_observation(&self, current: &Self) -> bool {
        self.operation == current.operation
            && self.saved.revision() == current.saved.revision()
            && self.saved.selector() == current.saved.selector()
            && self.saved.grant() == current.saved.grant()
    }
}
pub struct AuthoredSavedReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "authored history requires exact combat manifest",
        ));
    }
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("authored_saved_source").map_err(error)?,
        1,
        json!({"format":1,"subject":"actual_actor","revision":"exact_saved_identity",
            "operations":["source","diff"],"selector":"current_exact_immediate_parent",
            "forms":["open","open_resale"],"lifetime":"same_dispatch","manifest":manifest}),
    )
    .map_err(error)?
    .with_references(vec![manifest.clone()]))
}
impl AuthoredSavedReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(definition)
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || expected.digest() != manifest.digest() {
        return Err(ModError::msg("authored history composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition,
        BTreeSet::from([owner("tacsim")?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        AuthoredSavedReader {
            families,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for AuthoredSavedReader {
    type State = RollStore;
    type Request = AuthoredSavedRequest;
    type Output = AuthoredSavedAccess;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != "tacsim" {
            return Err(OwnerReadError::Authority);
        }
        let saved = resolve_saved_parent(
            roll,
            ctx,
            &self.families,
            &self.ratings,
            &self.expected,
            request.revision,
        )?;
        Ok(AuthoredSavedAccess {
            operation: request.operation,
            saved,
        })
    }
}
