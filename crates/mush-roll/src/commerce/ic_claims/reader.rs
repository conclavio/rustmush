//! Fresh actor-bound IC admission. A holding is not custody: this observation
//! must be consumed by the exact composed Open and retained through its Root
//! participant. No OOC selector or public receipt is accepted as permission.

use mush_core::action::PrincipalId;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{ClockStamp, DefinitionRef, Flags, ModError, ObjId, ObjKind, Registrar, World};
use mush_economy::{AssetLifecycle, EconomyStore};
use serde::Serialize;
use std::{any::TypeId, collections::BTreeSet};

use super::super::dependent_owner::OwnerParentBinding;
use super::super::families::CommerceFamilies;
use super::super::owner_holdings::{OwnerProjectionError, project_owner_with_parent_for};
use super::super::personal_filed_reader::PersonalFiledHolding;
use super::super::personal_paid_holdings::project_entitlement_paid_for;
use super::super::rating::{RatingCompatibility, RatingFamilies};
use super::super::records::{PublicationError, Record};
use super::super::{Capacity, LicenceId, TermPhase, TermWindow, WorldIcSeconds};
use crate::RollStore;

/// Exact personal dispatch selector. No actor, capacity, currentness bit,
/// source, executable, title or caller-provided warrant appears here.
#[derive(Serialize)]
pub struct IcAdmissionRequest {
    unit: ObjId,
    licence: LicenceId,
}
impl IcAdmissionRequest {
    pub fn own(unit: ObjId, licence: LicenceId) -> Self {
        Self { unit, licence }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct TitleFence {
    object: ObjId,
    revision: u64,
    holder: ObjId,
}
impl TitleFence {
    fn capture(world: &World, object: ObjId, holder: ObjId) -> Result<Self, PublicationError> {
        if world
            .module_state_owner(TypeId::of::<EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
        {
            return Err(PublicationError::Unavailable);
        }
        let economy = world
            .module_state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let title = economy
            .snapshot()
            .asset(object)
            .ok_or(PublicationError::Missing)?;
        if economy.is_frozen()
            || world
                .get(object)
                .is_none_or(|row| row.kind != ObjKind::Thing || row.flags.contains(Flags::GOING))
            || title.object != object
            || title.title_owner != mush_org::PrincipalId::Object(holder)
            || title.scope.is_some()
            || title.frozen
            || title.stolen.is_some()
            || title.lifecycle != AssetLifecycle::Active
            || title.revision == 0
        {
            return Err(PublicationError::Unavailable);
        }
        Ok(Self {
            object,
            revision: title.revision,
            holder,
        })
    }
    fn check(&self, world: &World) -> Result<(), PublicationError> {
        if Self::capture(world, self.object, self.holder)? != *self {
            return Err(PublicationError::Stale);
        }
        Ok(())
    }
}

/// Opaque ephemeral observation from the actual registered Roll reader.
/// Serialize exists only for the internal bounded result envelope. It has no
/// Deserialize/constructor and is never accepted from a player-facing request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IcAdmissionFacts {
    holding: PersonalFiledHolding,
    hull: TitleFence,
    charter: Option<TitleFence>,
    parent_window: Option<TermWindow>,
    observed_ic: ClockStamp,
}
impl IcAdmissionFacts {
    pub fn holding(&self) -> &PersonalFiledHolding {
        &self.holding
    }
    pub const fn unit(&self) -> ObjId {
        self.hull.object
    }
    pub fn dispatcher(&self) -> ObjId {
        self.holding.actor()
    }
    pub const fn hull_title_revision(&self) -> u64 {
        self.hull.revision
    }
    pub fn capacity(&self) -> Capacity {
        self.holding
            .paid_terms()
            .map_or(Capacity::OwnedDispatch {}, |terms| Capacity::PaidSeats {
                seats: terms.seats(),
            })
    }
    pub(crate) fn check_fresh(
        &self,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        let world = ctx.world();
        let now = world.world_ic_now();
        if ctx.invocation().actor() != self.dispatcher()
            || ctx.invocation().principal()
                != &(PrincipalId::Object {
                    id: self.dispatcher(),
                })
            || now.clock != self.observed_ic.clock
            || now.at < self.observed_ic.at
            || world
                .get(self.dispatcher())
                .is_none_or(|row| row.kind != ObjKind::Player || row.flags.contains(Flags::GOING))
        {
            return Err(PublicationError::Stale);
        }
        let at = WorldIcSeconds::from_stamp(&now).map_err(|_| PublicationError::Invalid)?;
        let usable = |window: TermWindow| {
            matches!(window.phase_at(at), TermPhase::Active | TermPhase::Grace)
        };
        if !usable(self.holding.window())
            || self.parent_window.is_some_and(|window| !usable(window))
        {
            return Err(PublicationError::Unavailable);
        }
        self.hull.check(world)?;
        if let Some(charter) = &self.charter {
            charter.check(world)?;
        }
        Ok(())
    }
}

pub struct IcAdmissionReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn refusal(e: OwnerProjectionError) -> OwnerReadError {
    match e {
        OwnerProjectionError::NotHeld => OwnerReadError::Refused,
        OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
        OwnerProjectionError::Integrity => OwnerReadError::Integrity,
    }
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("personal_ic_admission").map_err(error)?,1,
        serde_json::json!({"format":1,"protocol":"personal_ic_admission_v1","subject":"actual_actor","hull":"current_personal_title","owner":"current_owner_and_parent","paid":"own_term_status","source_display":false,"manifest":manifest})).map_err(error)?.with_references(vec![manifest.clone()]))
}
impl IcAdmissionReader {
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
        return Err(ModError::msg("IC admission composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        ["economy", "tacsim", "tactics", "sim"]
            .into_iter()
            .map(|owner| OwnerId::new(owner).map_err(error))
            .collect::<Result<BTreeSet<_>, _>>()?,
        IcAdmissionReader {
            families,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for IcAdmissionReader {
    type State = RollStore;
    type Request = IcAdmissionRequest;
    type Output = IcAdmissionFacts;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != "tacsim"
            || ctx.principal() != &(PrincipalId::Object { id: ctx.actor() })
            || ctx
                .world()
                .get(ctx.actor())
                .is_none_or(|row| row.kind != ObjKind::Player || row.flags.contains(Flags::GOING))
            || request.unit == ctx.actor()
        {
            return Err(OwnerReadError::Authority);
        }
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        let hull = TitleFence::capture(ctx.world(), request.unit, ctx.actor())
            .map_err(|_| OwnerReadError::Refused)?;
        let owner = ctx
            .action_state(&self.families.licences, &request.licence.key())?
            .is_some();
        let paid = ctx
            .action_state(&self.families.paid_licences, &request.licence.key())?
            .is_some();
        let (holding, charter, parent_window) = match (owner, paid) {
            (true, false) => {
                let holding = project_owner_with_parent_for(
                    ctx,
                    ctx.actor(),
                    &self.families,
                    &self.ratings,
                    request.licence,
                    &self.expected,
                )
                .map_err(refusal)?;
                let charter =
                    TitleFence::capture(ctx.world(), ObjId(holding.charter().get()), ctx.actor())
                        .map_err(|_| OwnerReadError::Integrity)?;
                if charter.revision != holding.title_revision() {
                    return Err(OwnerReadError::Integrity);
                }
                // The full projection above already proved the immediate
                // parent's exact current grant. Capture only its actual current
                // term window here; do not walk that certificate a second time.
                let binding = ctx.action_state(
                    &self.families.owner_parents,
                    &OwnerParentBinding::key_for(holding.licence(), ctx.actor()),
                )?;
                let parent_window = match binding {
                    None => None,
                    Some(binding) => {
                        binding.validate().map_err(|_| OwnerReadError::Integrity)?;
                        let licence = ctx
                            .action_state(
                                &self.families.paid_licences,
                                &binding.parent.licence.key(),
                            )?
                            .ok_or(OwnerReadError::Integrity)?;
                        let key = format!("{}/{}", licence.id().key(), licence.term().key());
                        let term = ctx
                            .action_state(&self.families.paid_terms, &key)?
                            .ok_or(OwnerReadError::Integrity)?;
                        term.matches_current(licence)
                            .map_err(|_| OwnerReadError::Integrity)?;
                        Some(term.window())
                    }
                };
                (
                    PersonalFiledHolding::original(holding),
                    Some(charter),
                    parent_window,
                )
            }
            (false, true) => (
                PersonalFiledHolding::paid(
                    project_entitlement_paid_for(
                        ctx,
                        ctx.actor(),
                        &self.families,
                        &self.ratings,
                        request.licence,
                        &self.expected,
                    )
                    .map_err(refusal)?,
                ),
                None,
                None,
            ),
            (false, false) => return Err(OwnerReadError::Refused),
            (true, true) => return Err(OwnerReadError::Integrity),
        };
        Ok(IcAdmissionFacts {
            holding,
            hull,
            charter,
            parent_window,
            observed_ic: ctx.world().world_ic_now(),
        })
    }
}
