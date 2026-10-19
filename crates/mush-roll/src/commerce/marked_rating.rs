//! Current rights for a new Registry computation of one exact marked saved revision.
//! Measurement does not grant source display, filing, sale or owner execution.

use super::families::CommerceFamilies;
use super::parent_grant::CurrentParentGrant;
use super::parent_selector::DraftParentSelection;
use super::rating::{RatingCompatibility, RatingFamilies};
use super::read::CommerceRead;
use super::records::{PublicationError, Record};
use crate::{Revision, RevisionRef, RollStore};
use mush_core::action::{PerceivedRef, PrincipalId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{CommitToken, DefinitionRef, ModError, ObjId, Registrar};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;

/// An exact owned immutable revision, never a supplied grant or source program.
#[derive(Serialize)]
pub struct MarkedRatingRequest {
    revision: RevisionRef,
}
impl MarkedRatingRequest {
    pub fn new(revision: RevisionRef) -> Self {
        Self { revision }
    }
}

/// Private common resolved facts. Each consumer still needs its own operation,
/// registered reader and freshness boundary; these facts are not a public ACL.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SavedParentObservation {
    revision: Revision,
    selector: DraftParentSelection,
    parent: CurrentParentGrant,
}
impl SavedParentObservation {
    pub(crate) fn revision(&self) -> &Revision {
        &self.revision
    }
    pub(crate) fn selector(&self) -> &DraftParentSelection {
        &self.selector
    }
    pub(crate) fn grant(&self) -> &CurrentParentGrant {
        &self.parent
    }
}

/// Only the registered saved-revision reader constructs this observation. It
/// has no decoder/Clone and conveys no public source-display permission.
#[derive(Debug, Serialize)]
pub struct MarkedRatingAccess(SavedParentObservation);
impl MarkedRatingAccess {
    pub fn actor(&self) -> ObjId {
        self.0.parent.actor()
    }
    pub fn revision(&self) -> &Revision {
        self.0.revision()
    }
    pub(crate) fn into_fence(self, action: DefinitionRef, commit: CommitToken) -> AdmissionFence {
        AdmissionFence {
            action,
            commit,
            saved: self.0,
        }
    }
}

/// Carried only by the closed rating-admit Root request and its prepared image.
#[derive(Clone)]
pub(crate) struct AdmissionFence {
    action: DefinitionRef,
    commit: CommitToken,
    saved: SavedParentObservation,
}
impl AdmissionFence {
    pub(crate) fn revision(&self) -> &Revision {
        self.saved.revision()
    }
    pub(crate) fn check(&self, context: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if self.action != *context.action()
            || self.action.key().owner.as_str() != crate::NS
            || self.action.key().kind.as_str() != "action"
            || self.action.key().id.as_str() != "rating_admit"
            || self.action.version() != 1
            || self.commit != context.commit_token()
            || context.invocation().principal()
                != &(PrincipalId::Object {
                    id: self.saved.parent.actor(),
                })
            || context.invocation().subject() != &PerceivedRef::SelfActor
            || !context.invocation().targets().is_empty()
        {
            return Err(PublicationError::Mismatch);
        }
        self.saved
            .parent
            .check_fresh(context.invocation().actor(), context.world())
    }
}

/// Resolve exact ancestry and the selected current own grant. Historical boot
/// audits intentionally do not call this operation-specific admission resolver.
pub(crate) fn resolve_saved_parent(
    roll: &RollStore,
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    expected: &RatingCompatibility,
    reference: RevisionRef,
) -> Result<SavedParentObservation, OwnerReadError> {
    if reference.key.character != ctx.actor() {
        return Err(OwnerReadError::Authority);
    }
    roll.commerce_ready()
        .map_err(|_| OwnerReadError::Unavailable)?;
    let mind = roll
        .owned_mind(ctx.actor(), reference.key.mind)
        .map_err(|_| OwnerReadError::Refused)?;
    let provenance = mind.provenance.as_ref().ok_or(OwnerReadError::Refused)?;
    let revision = roll
        .revision(ctx.actor(), reference.key)
        .map_err(|_| OwnerReadError::Refused)?;
    if revision.reference() != reference || revision.provenance.as_ref() != Some(provenance) {
        return Err(OwnerReadError::Integrity);
    }
    revision.validate().map_err(|_| OwnerReadError::Integrity)?;
    let selector = ctx
        .get(
            &families.draft_parents,
            &DraftParentSelection::key_for(ctx.actor(), reference.key.mind),
        )
        .map_err(|_| OwnerReadError::Unavailable)?
        .ok_or(OwnerReadError::Integrity)?;
    selector.validate().map_err(|_| OwnerReadError::Integrity)?;
    selector
        .check_mind(mind)
        .map_err(|_| OwnerReadError::Integrity)?;
    let parent =
        super::parent_grant::resolve(roll, ctx, families, ratings, expected, selector.licence)?;
    if parent.actor() != ctx.actor()
        || parent.provenance() != *provenance
        || selector.provenance() != parent.provenance()
    {
        return Err(OwnerReadError::Integrity);
    }
    parent
        .check_fresh(ctx.actor(), ctx.world())
        .map_err(|_| OwnerReadError::Refused)?;
    Ok(SavedParentObservation {
        revision: revision.clone(),
        selector: selector.clone(),
        parent,
    })
}

pub struct MarkedRatingReader {
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
            "marked rating requires exact combat manifest",
        ));
    }
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("marked_rating_admission").map_err(error)?,
        1,
        json!({"format":1,"operation":"registry_admission","subject":"actual_actor",
            "revision":"exact_owned_saved","selector":"exact_immediate_parent",
            "forms":["open","open_resale"],"manifest":manifest}),
    )
    .map_err(error)?
    .with_references(vec![manifest.clone()]))
}
impl MarkedRatingReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let reference = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(reference)
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
        return Err(ModError::msg("marked rating composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition,
        BTreeSet::from([owner("roll")?, owner("tacsim")?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        MarkedRatingReader {
            families,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for MarkedRatingReader {
    type State = RollStore;
    type Request = MarkedRatingRequest;
    type Output = MarkedRatingAccess;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: MarkedRatingRequest,
    ) -> Result<MarkedRatingAccess, OwnerReadError> {
        if !matches!(ctx.requester().as_str(), "roll" | "tacsim") {
            return Err(OwnerReadError::Authority);
        }
        resolve_saved_parent(
            roll,
            ctx,
            &self.families,
            &self.ratings,
            &self.expected,
            request.revision,
        )
        .map(MarkedRatingAccess)
    }
}
