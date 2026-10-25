//! Source-free current property observations. Fresh source, current certificate,
//! issuer rights and term phase are intentionally not transfer prerequisites.
//! This reader conveys facts to the closed Market provider, never seller consent.
use super::{
    PaidResaleBinding, PaidResaleRecord, PaidResaleState, ResaleClaimObservation, ResaleEconomics,
    economics, history,
};
use crate::{
    RollStore,
    commerce::{
        LicenceId, RevisionIdentity, TermPhase, WorldIcSeconds,
        entitlement::history::{self as acquired, optional, read},
        families::CommerceFamilies,
        paid::PersonalPaidLicence,
        rating::RatingFamilies,
        read::CommerceRead,
        records::PublicationError,
    },
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, Flags, ModError,
    ObjId, ObjKind, OwnerId, Registrar, World,
    action::PrincipalId,
    participant::OwnerPrepareContext,
    read_service::{OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService},
};
use serde::Serialize;
use std::collections::BTreeSet;
fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn fail(_: impl std::fmt::Display) -> OwnerReadError {
    OwnerReadError::Integrity
}
fn module_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn live(world: &World, actor: ObjId) -> Result<(), PublicationError> {
    if world
        .get(actor)
        .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
    {
        return Err(PublicationError::Unavailable);
    }
    Ok(())
}
/// Constructors are used only by the registered Market owner. The buyer path
/// must additionally join the exact actual retained named-seller offer before
/// any publication. A caller-provided counterparty is not a consent receipt.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaidResaleRequest {
    Seller { licence: LicenceId, buyer: ObjId },
    Buyer { licence: LicenceId, seller: ObjId },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaidResaleFacts {
    requester: ObjId,
    seller: ObjId,
    buyer: ObjId,
    pointer: PersonalPaidLicence,
    binding: PaidResaleBinding,
    revision: RevisionIdentity,
    economics: ResaleEconomics,
    previous: Option<PaidResaleRecord>,
    claims: ResaleClaimObservation,
}
impl PaidResaleFacts {
    pub const fn requester(&self) -> ObjId {
        self.requester
    }
    pub const fn seller(&self) -> ObjId {
        self.seller
    }
    pub const fn buyer(&self) -> ObjId {
        self.buyer
    }
    pub fn pointer(&self) -> &PersonalPaidLicence {
        &self.pointer
    }
    pub fn binding(&self) -> &PaidResaleBinding {
        &self.binding
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub fn economics(&self) -> &ResaleEconomics {
        &self.economics
    }
    pub fn previous(&self) -> Option<&PaidResaleRecord> {
        self.previous.as_ref()
    }
    pub fn claims(&self) -> &ResaleClaimObservation {
        &self.claims
    }
    pub fn phase_at(&self, now: WorldIcSeconds) -> TermPhase {
        self.binding.term().window().phase_at(now)
    }
    /// The actual typed preimage read is retained separately by the writer.
    /// This repeats volatile lifecycle/clock/claim facts under Root custody.
    pub(crate) fn check_fresh(
        &self,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        if ctx.invocation().actor() != self.requester
            || ctx.invocation().principal() != &(PrincipalId::Object { id: self.requester })
        {
            return Err(PublicationError::Mismatch);
        }
        self.check_world(ctx.world())
    }
    pub(crate) fn check_world(&self, world: &World) -> Result<(), PublicationError> {
        live(world, self.seller)?;
        live(world, self.buyer)?;
        if self.seller == self.buyer {
            return Err(PublicationError::Mismatch);
        }
        let roll = world
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        if roll.paid_resale_claims(self.pointer.id()).map_err(bad)? != self.claims {
            return Err(PublicationError::Mismatch);
        }
        let e = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        if e.is_frozen() {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}

pub struct PaidResaleReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
}
pub(crate) fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(module_error)?,
        DefinitionId::new("personal_paid_resale").map_err(module_error)?,
        1,
        serde_json::json!({"format":1,"protocol":"personal_paid_resale_v1","current_holder":true,
            "accepted_term_economics":true,"source_bytes":false,"lapsed_property":true,
            "current_claims":true,"seller_consent":"separate_actual_market_offer"}),
    )
    .map_err(module_error)
}
impl PaidResaleReader {
    pub fn request(reg: &mut Registrar<'_>) -> Result<OwnerReadHandle<Self>, ModError> {
        let reference = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(module_error)?,
                descriptor()?,
            )
            .map_err(module_error)?;
        reg.request_owner_read::<Self>(reference)
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("paid resale reader owner differs"));
    }
    let reference = reg.definition(descriptor()?)?;
    reg.owner_read_service(
        reference,
        BTreeSet::from([OwnerId::new("market").map_err(module_error)?]),
        BTreeSet::from([OwnerId::new("economy").map_err(module_error)?]),
        PaidResaleReader { families, ratings },
    )
}
impl OwnerReadService for PaidResaleReader {
    type State = RollStore;
    type Request = PaidResaleRequest;
    type Output = PaidResaleFacts;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: PaidResaleRequest,
    ) -> Result<PaidResaleFacts, OwnerReadError> {
        if ctx.requester().as_str() != "market"
            || ctx.principal() != &(PrincipalId::Object { id: ctx.actor() })
        {
            return Err(OwnerReadError::Authority);
        }
        let (licence, seller, buyer) = match request {
            PaidResaleRequest::Seller { licence, buyer } => (licence, ctx.actor(), buyer),
            PaidResaleRequest::Buyer { licence, seller } => (licence, seller, ctx.actor()),
        };
        observe(
            roll,
            ctx,
            &self.families,
            &self.ratings,
            ctx.actor(),
            seller,
            buyer,
            licence,
        )
        .map_err(fail)
    }
}
#[allow(clippy::too_many_arguments)]
fn observe(
    roll: &RollStore,
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    requester: ObjId,
    seller: ObjId,
    buyer: ObjId,
    id: LicenceId,
) -> Result<PaidResaleFacts, PublicationError> {
    roll.commerce_ready().map_err(bad)?;
    live(ctx.world(), seller)?;
    live(ctx.world(), buyer)?;
    if seller == buyer || (requester != seller && requester != buyer) {
        return Err(PublicationError::Mismatch);
    }
    let pointer = read(ctx, &f.paid_licences, id.key())?;
    if pointer.holder() != seller {
        return Err(PublicationError::Missing);
    }
    let term = read(
        ctx,
        &f.paid_terms,
        format!("{}/{}", id.key(), pointer.term().key()),
    )?;
    acquired::current(ctx, f, ratings, pointer, term)?;
    if !term.terms().transferable() {
        return Err(PublicationError::Unavailable);
    }
    let now = ctx.world().world_ic_now();
    let seconds = WorldIcSeconds::from_stamp(&now).map_err(bad)?;
    // Property may transfer after the accepted sanction's original grace_end;
    // this does not restore use of the sanctioned generation.
    crate::commerce::registry_revocation::current::require(
        ctx,
        f,
        id,
        term.generation(),
        Some(seconds),
    )?;
    let personal = optional(ctx, &f.renewal_heads, id.key())?;
    let house = optional(ctx, &f.house_renewal_heads, id.key())?;
    if personal.is_some_and(|h| h.pending().is_some())
        || house.is_some_and(|h| h.pending().is_some())
    {
        return Err(PublicationError::Unavailable);
    }
    let ownership = history::current(ctx, f, ratings, pointer)?;
    let birth = history::birth(ctx, f, ratings, pointer)?;
    let previous = if let Some(head) = optional(ctx, &f.paid_resale_heads, id.key())? {
        let row = read(ctx, &f.paid_resales, head.current().operation().to_hex())?;
        head.matches(row)?;
        history::native(ctx.world(), row)?;
        history::roots(ctx.world(), row, birth.revision)?;
        if matches!(row.state(), PaidResaleState::Funded {}) {
            return Err(PublicationError::Unavailable);
        }
        let terminal_utc = row
            .economic()
            .terminal()
            .map_or(row.economic().consent().accepted_at_utc, |t| {
                t.decision().posted_at_utc
            });
        if ctx.world().now() < terminal_utc {
            return Err(PublicationError::Mismatch);
        }
        Some(row.clone())
    } else {
        None
    };
    if pointer.ownership().is_some() && previous.is_none() {
        return Err(PublicationError::Missing);
    }
    let economics = economics::accepted(ctx, f, ratings, pointer, term)?;
    let binding = PaidResaleBinding {
        licence: id,
        charter: pointer.charter(),
        line: pointer.line(),
        birth: birth.reference,
        ownership,
        term: term.clone(),
        term_clock: history::issued_clock(ctx, f, pointer, term)?,
        economics_digest: economics.economics_digest(),
        royalty_chain_digest: economics.royalty_chain_digest(),
    };
    binding.validate()?;
    let claims = roll.paid_resale_claims(id).map_err(bad)?;
    claims.validate()?;
    if claims.open_paid() > u32::from(term.terms().seats().get()) {
        return Err(PublicationError::Mismatch);
    }
    let facts = PaidResaleFacts {
        requester,
        seller,
        buyer,
        pointer: pointer.clone(),
        binding,
        revision: birth.revision,
        economics,
        previous,
        claims,
    };
    facts.check_world(ctx.world())?;
    Ok(facts)
}
