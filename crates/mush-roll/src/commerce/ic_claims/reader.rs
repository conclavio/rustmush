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

use super::super::families::CommerceFamilies;
use super::super::owner_holdings::{OwnerProjectionError, project_owner_with_parent_for};
use super::super::personal_filed_reader::PersonalFiledHolding;
use super::super::personal_paid_holdings::{project_entitlement_paid_for, reader_holder};
use super::super::rating::{RatingCompatibility, RatingFamilies};
use super::super::records::PublicationError;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    charter_ownership: Option<super::super::charter_transfer::records::CharterOwnershipRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paid_ownership: Option<mush_economy::managed_paid_resale::PaidHolderRefV1>,
    observed_ic: ClockStamp,
}
impl IcAdmissionFacts {
    pub(crate) fn paid_ownership(
        &self,
    ) -> Option<&mush_economy::managed_paid_resale::PaidHolderRefV1> {
        self.paid_ownership.as_ref()
    }
    pub(crate) fn charter_ownership(
        &self,
    ) -> Option<&super::super::charter_transfer::records::CharterOwnershipRef> {
        self.charter_ownership.as_ref()
    }
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
    /// Stable immutable admission identity for the actual seal owner. This
    /// calculation carries no authority; IcWriter compares it only with the
    /// output of its exact typed participant preparation.
    pub fn binding_digest(&self) -> Result<mush_core::DefinitionDigest, PublicationError> {
        let h = self.holding();
        let mut value = serde_json::json!({
            "unit": self.unit(), "dispatcher": self.dispatcher(),
            "hull_title_revision": self.hull_title_revision(),
            "licence": h.licence(), "term": h.term(), "charter": h.charter(),
            "line": h.line(), "filed": h.filed(), "revision": h.identity(),
            "rating": h.rating(), "window": h.window(), "capacity": self.capacity()
        });
        if let Some(proof) = &self.charter_ownership {
            value
                .as_object_mut()
                .ok_or(PublicationError::Invalid)?
                .insert(
                    "charter_ownership".into(),
                    serde_json::to_value(proof).map_err(|_| PublicationError::Invalid)?,
                );
        }
        if let Some(proof) = &self.paid_ownership {
            value
                .as_object_mut()
                .ok_or(PublicationError::Invalid)?
                .insert(
                    "paid_ownership".into(),
                    serde_json::to_value(proof).map_err(|_| PublicationError::Invalid)?,
                );
        }
        super::records::admission_digest(&value)
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
        let (holding, charter, parent_window, charter_ownership) = match (owner, paid) {
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
                super::super::charter_pending::current::require_clear(
                    ctx,
                    &self.families,
                    holding.charter(),
                )
                .map_err(|_| OwnerReadError::Refused)?;
                let charter =
                    TitleFence::capture(ctx.world(), ObjId(holding.charter().get()), ctx.actor())
                        .map_err(|_| OwnerReadError::Integrity)?;
                if charter.revision != holding.title_revision() {
                    return Err(OwnerReadError::Integrity);
                }
                // Use the selected current qualification already proved by
                // this owner projection; birth ancestry is a separate record.
                let parent_window = holding.qualification().map(|q| q.window());
                let charter_ownership =
                    (holding.ownership().generation > 1).then(|| holding.ownership().clone());
                (
                    PersonalFiledHolding::original(holding),
                    Some(charter),
                    parent_window,
                    charter_ownership,
                )
            }
            (false, true) => (
                PersonalFiledHolding::paid(
                    paid_projection(
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
                None,
            ),
            (false, false) => return Err(OwnerReadError::Refused),
            (true, true) => return Err(OwnerReadError::Integrity),
        };
        let paid_ownership = if paid {
            use super::super::entitlement::history::read;
            let licence = read(ctx, &self.families.paid_licences, request.licence.key())
                .map_err(|_| OwnerReadError::Integrity)?;
            super::super::paid_resale::history::require_not_pending(ctx, &self.families, licence)
                .map_err(|_| OwnerReadError::Refused)?;
            // The full paid projection already proved today's exact holder;
            // preserve that immutable generation in this new captured warrant.
            licence.ownership().copied()
        } else {
            None
        };
        Ok(IcAdmissionFacts {
            holding,
            hull,
            charter,
            parent_window,
            charter_ownership,
            paid_ownership,
            observed_ic: ctx.world().world_ic_now(),
        })
    }
}

// The IC reader already owns closed paid-seat admission. Select its actual
// immutable birth protocol, then invoke that protocol's full current proof.
fn paid_projection(
    ctx: &OwnerReadContext<'_>,
    holder: ObjId,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: LicenceId,
    expected: &RatingCompatibility,
) -> Result<
    super::super::personal_paid_holdings::PersonalPaidHolding,
    super::super::owner_holdings::OwnerProjectionError,
> {
    use super::super::entitlement::history::{Birth, read, resolve};
    use super::super::owner_holdings::OwnerProjectionError as E;
    let paid = read(ctx, &f.paid_licences, licence.key()).map_err(|_| E::Integrity)?;
    // M8c F1: the dispatcher, typed by the pointer's arm (`reader_holder`):
    // an organisation's licence dispatches for any character acting for it.
    let holder = reader_holder(ctx, f, holder, licence)?;
    match resolve(ctx, f, paid.operation()).map_err(|_| E::Integrity)? {
        Birth::House(_) => super::super::personal_paid_holdings::project_house_paid_for(
            ctx, holder, f, ratings, licence, expected,
        ),
        _ => project_entitlement_paid_for(ctx, holder, f, ratings, licence, expected),
    }
}
