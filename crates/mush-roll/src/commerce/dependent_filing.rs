//! Current saved-parent observation for private dependent filing only.
//! Registry admission, authored editing and future resale have separate readers.

use mush_core::action::{PerceivedRef, PrincipalId};
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind, OwnerId};
use mush_core::participant::OwnerPrepareContext;
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use mush_core::{CommitToken, DefinitionRef, ModError, Registrar};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;

use super::families::CommerceFamilies;
use super::marked_rating::{SavedParentObservation, resolve_saved_parent};
use super::rating::{RatingCompatibility, RatingFamilies};
use super::records::PublicationError;
use super::roots::InitialRoots;
use super::{TermPhase, TermWindow, WorldIcSeconds};
use crate::{Revision, RevisionRef, RollStore};

pub(crate) const ACTION: &str = "dependent_initial_filing";

#[derive(Serialize)]
pub(crate) struct FilingParentRequest {
    pub(crate) revision: RevisionRef,
}

#[derive(Serialize)]
pub(crate) struct FilingParentAccess {
    saved: SavedParentObservation,
}
impl FilingParentAccess {
    pub(crate) fn revision(&self) -> &Revision {
        self.saved.revision()
    }

    pub(crate) fn parent(
        &self,
        term_seconds: i64,
        grace_seconds: i64,
    ) -> Result<super::dependent_owner::DependentParent, PublicationError> {
        super::dependent_owner::DependentParent::capture(
            self.saved.grant(),
            term_seconds,
            grace_seconds,
        )
    }

    pub(crate) fn into_roots(
        self,
        action: DefinitionRef,
        roots: InitialRoots,
        window: TermWindow,
    ) -> DependentRoots {
        DependentRoots {
            roots,
            fence: FilingFence {
                action,
                saved: self.saved,
                window,
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct FilingFence {
    action: DefinitionRef,
    saved: SavedParentObservation,
    window: TermWindow,
}
impl FilingFence {
    pub(crate) fn check(
        &self,
        commit: CommitToken,
        context: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        let actor = self.saved.grant().actor();
        if self.action != *context.action()
            || self.action.key().owner.as_str() != crate::NS
            || self.action.key().kind.as_str() != "action"
            || self.action.key().id.as_str() != ACTION
            || self.action.version() != 1
            || commit != context.commit_token()
            || context.invocation().actor() != actor
            || context.invocation().principal() != &(PrincipalId::Object { id: actor })
            || context.invocation().subject() != &PerceivedRef::SelfActor
            || !context.invocation().targets().is_empty()
        {
            return Err(PublicationError::Mismatch);
        }
        self.saved.grant().check_fresh(actor, context.world())?;
        let now = WorldIcSeconds::from_stamp(&context.world().world_ic_now())
            .map_err(|_| PublicationError::Invalid)?;
        if !matches!(
            self.window.phase_at(now),
            TermPhase::Active | TermPhase::Grace
        ) {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}

pub(crate) struct DependentRoots {
    roots: InitialRoots,
    fence: FilingFence,
}
impl DependentRoots {
    pub(crate) fn commit(&self) -> CommitToken {
        self.roots.commit()
    }
    pub(crate) fn roots(&self) -> &InitialRoots {
        &self.roots
    }
    pub(crate) fn fence(&self) -> &FilingFence {
        &self.fence
    }
}

pub(crate) struct FilingParentReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("dependent_filing_parent").map_err(error)?,
        1,
        json!({"format":1,"operation":"private_dependent_filing","subject":"actual_actor",
            "revision":"exact_owned_saved","parent":"selected_immediate_paid_pin",
            "forms":["open","open_resale"],"sales":false,"manifest":manifest}),
    )
    .map_err(error)?
    .with_references(vec![manifest.clone()]))
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<DefinitionRef, ModError> {
    if reg.module() != crate::NS || expected.digest() != manifest.digest() {
        return Err(ModError::msg("dependent filing parent composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition.clone(),
        BTreeSet::from([owner(crate::NS)?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        FilingParentReader {
            families,
            ratings,
            expected,
        },
    )?;
    Ok(definition)
}
impl OwnerReadService for FilingParentReader {
    type State = RollStore;
    type Request = FilingParentRequest;
    type Output = FilingParentAccess;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: FilingParentRequest,
    ) -> Result<FilingParentAccess, OwnerReadError> {
        if ctx.requester().as_str() != crate::NS {
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
        .map(|saved| FilingParentAccess { saved })
    }
}
