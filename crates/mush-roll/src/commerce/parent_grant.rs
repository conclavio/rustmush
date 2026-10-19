//! Current immediate-parent entitlement for a new authored fork.
//!
//! The paid grant remains independent of its issuer's current title or parent
//! rights. This observation is private, not decodable, and never substitutes
//! for the downstream paid licence's own admission/source predicate.

use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind, OwnerId};
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use mush_core::{ClockStamp, DefinitionRef, ModError, ObjId, Registrar, World};
use serde::Serialize;
use serde_json::json;
use std::{any::TypeId, collections::BTreeSet};

use super::families::CommerceFamilies;
use super::filed::ParentContent;
use super::owner_holdings::OwnerProjectionError;
use super::paid::PaidForm;
use super::personal_paid_holdings::{PersonalPaidHolding, project_personal_paid_for};
use super::rating::{RatingCompatibility, RatingFamilies};
use super::records::PublicationError;
use super::{LicenceId, TermPhase, WorldIcSeconds};
use crate::{DerivativeProvenance, RollStore};

#[derive(Serialize)]
pub(crate) struct ParentGrantRequest {
    pub(crate) licence: LicenceId,
}

/// Captured only by the actual registered owner reader. It contains no source
/// bytes, caller-provided authority, cached future eligibility or issuer state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CurrentParentGrant {
    holding: PersonalPaidHolding,
    observed_ic: ClockStamp,
}
impl CurrentParentGrant {
    pub(crate) fn actor(&self) -> ObjId {
        self.holding.actor()
    }
    pub(crate) fn licence(&self) -> LicenceId {
        self.holding.licence()
    }
    pub(crate) fn holding(&self) -> &PersonalPaidHolding {
        &self.holding
    }
    pub(crate) fn provenance(&self) -> DerivativeProvenance {
        DerivativeProvenance {
            parent: ParentContent {
                filed: self.holding.filed(),
                revision: self.holding.identity(),
            },
            charter: self.holding.charter(),
            line: self.holding.line(),
        }
    }
    /// Compare a freshly resolved later observation, allowing elapsed time but
    /// no clock reset or replacement of the captured exact paid grant. This
    /// structural comparison alone grants no current permission.
    pub(crate) fn same_grant_later(&self, current: &Self) -> bool {
        self.holding == current.holding
            && self.observed_ic.clock == current.observed_ic.clock
            && self.observed_ic.at <= current.observed_ic.at
    }

    /// Typed/preimage and legacy generation fences handle record changes. This
    /// additional check handles elapsed IC time while a preparation is held.
    pub(crate) fn check_fresh(&self, actor: ObjId, world: &World) -> Result<(), PublicationError> {
        let now = world.world_ic_now();
        if actor != self.actor()
            || now.clock != self.observed_ic.clock
            || now.at < self.observed_ic.at
            || world.get(actor).is_none_or(|object| {
                object.kind != mush_core::ObjKind::Player
                    || object.flags.contains(mush_core::Flags::GOING)
            })
        {
            return Err(PublicationError::Mismatch);
        }
        let at = WorldIcSeconds::from_stamp(&now).map_err(|_| PublicationError::Invalid)?;
        if !matches!(
            self.holding.window().phase_at(at),
            TermPhase::Active | TermPhase::Grace
        ) {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}

pub(crate) struct ParentGrantReader {
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
        DefinitionId::new("authored_fork_parent").map_err(error)?,
        1,
        json!({"format":1,"operation":"authored_fork","subject":"actual_actor",
            "original_personal_pin":true,"forms":["open","open_resale"],"manifest":manifest}),
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
        return Err(ModError::msg("authored fork parent composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let owner = |v| OwnerId::new(v).map_err(error);
    reg.owner_read_service(
        definition.clone(),
        BTreeSet::from([owner(crate::NS)?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        ParentGrantReader {
            families,
            ratings,
            expected,
        },
    )?;
    Ok(definition)
}
impl OwnerReadService for ParentGrantReader {
    type State = RollStore;
    type Request = ParentGrantRequest;
    type Output = CurrentParentGrant;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: ParentGrantRequest,
    ) -> Result<CurrentParentGrant, OwnerReadError> {
        if ctx.requester().as_str() != crate::NS
            || ctx.principal() != &(mush_core::action::PrincipalId::Object { id: ctx.actor() })
        {
            return Err(OwnerReadError::Authority);
        }
        resolve(
            roll,
            ctx,
            &self.families,
            &self.ratings,
            &self.expected,
            request.licence,
        )
    }
}

/// Shared implementation behind exact registered Roll readers, never a public
/// constructor from caller-supplied form, grant or ParentRights.
pub(crate) fn resolve(
    roll: &RollStore,
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    expected: &RatingCompatibility,
    licence: LicenceId,
) -> Result<CurrentParentGrant, OwnerReadError> {
    if ctx.principal() != &(mush_core::action::PrincipalId::Object { id: ctx.actor() }) {
        return Err(OwnerReadError::Authority);
    }
    roll.commerce_ready()
        .map_err(|_| OwnerReadError::Unavailable)?;
    if ctx
        .world()
        .module_state_owner(TypeId::of::<mush_economy::EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(OwnerReadError::Integrity);
    }
    let holding = project_personal_paid_for(ctx, ctx.actor(), families, ratings, licence, expected)
        .map_err(|e| match e {
            OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
            OwnerProjectionError::NotHeld => OwnerReadError::Refused,
            OwnerProjectionError::Integrity => OwnerReadError::Integrity,
        })?;
    // These private fork/source/working-edit readers never turn Open into a
    // NewSale/PaidRenewal permission; those future resolvers need OpenResale.
    if !matches!(
        holding.terms().form(),
        PaidForm::Open {} | PaidForm::OpenResale {}
    ) {
        return Err(OwnerReadError::Refused);
    }
    Ok(CurrentParentGrant {
        holding,
        observed_ic: ctx.world().world_ic_now(),
    })
}
