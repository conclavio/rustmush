//! **`+core/offer` and `+core/accept`: a governed core resold to a player** (doc 48 unit EX.2b;
//! Q-48-47, Q-48-3's correction and the core leg of Q-48-52) **(expensive to reverse)**.
//!
//! The second closed plan beside the fitting shop's buy-back (`super`). A governed core's title
//! is a managed title, so no open listing sells one (Q-48-3's correction): it changes hands only
//! by its seller's **private offer** to one named player, and the buyer's acceptance is **one
//! union**:
//!
//! * **the offer** (`+core/offer <core or hull>=<player> price=<n>`, the seller's own act): one
//!   record in this crate's `core_resale_offers` family, keyed by the core, naming the buyer and
//!   the price and pinning the state it was made on (the core's title revision; for a hull, hers
//!   and the digest of her parts). Nothing moves and nothing is held. It stands 28 IC days (the
//!   owner's floor for any wait on a player), until withdrawn, replaced or taken.
//! * **the acceptance** (`+core/accept <core or hull>=go`, the buyer's act) binds three closed
//!   legs in one coordinator union, and removes the offer in the same transaction:
//!   - the economy's closed resale plan (`EconomyRequest::CoreResale`): the buyer's own credit
//!     account pays the price, **at acceptance**, split to the seller, the venue's cut and the
//!     levy; the core's title passes to the buyer, and so do the hull's and her hold when the
//!     offer sells a cored hull (`mush_economy::registry_filing::core_sale::resale`);
//!   - the kernel's closed resale image (`mush_core::core_sale::resale`): a stored core leaves
//!     the seller's hands for the buyer's; a core fitted in the hull sold with it does not move
//!     (her place and her core's are fenced through custody);
//!   - the kernel's record of the resale, in this crate's `core_resales` family.
//!
//! **Who may accept**: only the player the offer names; a third party hears there is no offer for
//! them, as anyone does for a core never offered. **What is refused**: a core that is not the
//! seller's alone, not in their hands (or fitted in their own ship, offered with her), destroyed,
//! or changed since the offer; a cored hull in a bay, with work in flight, a rig aboard or a
//! laden hold; a buyer who cannot pay.
//!
//! **The cuts' rates** are provisional constants at Q-48-8's ruled fees (EX.4's fees have not
//! merged): `mush_economy::registry_filing::core_sale::resale::{VENUE_PERMILLE, LEVY_PERMILLE}`.
//! The venue's cut goes to the launch's venue and the levy to its Registry.
//!
//! A boot pass pairs the kernel's records with the economy's receipts one to one (`boot_audit`).

use std::sync::{Arc, Mutex};

use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::action_state::StateFamilyHandle;
use mush_core::core_sale::resale::{
    CoreResalePolicy, CoreResales, ResaleFacts, ResaleRequest, ResoldCore,
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::{ParticipantError, ParticipantHandle};
use mush_core::{
    ActionInvocation, ActionTxn, CmdError, Ctx, DefinitionRef, ModError, ModuleCtx, ObjId, ObjKind,
    PerceivedRef, PrepareCtx, Registrar, World,
};
use mush_economy::managed_entitlement::Recipient;
use mush_economy::registry_filing::core_sale::resale::{
    CoreResaleConfig, RegistryCoreResale, ResaleHull,
};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_economy::{Amount, EconomyStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::SaleTerms;

/// The offer's action id (the seller's act).
pub const OFFER_ACTION: &str = "offer_core_resale";
/// The offer's outcome id.
pub const OFFER_OUTCOME: &str = "core_resale_offered";
/// The acceptance's action id (the kernel's and the economy's closed legs name it).
pub const RESALE_ACTION: &str = mush_core::core_sale::resale::ACTION;
/// The acceptance's outcome id.
pub const RESALE_OUTCOME: &str = "governed_core_resold";
/// The offers' family.
pub const OFFER_FAMILY: &str = "core_resale_offers";
/// **How long an offer stands**, IC seconds: 28 IC days, the owner's floor for any wait on a
/// player's response (2026-11-04).
pub const OFFER_IC_SECONDS: i64 = 28 * 24 * 60 * 60;

/// The help.
pub const RESALE_HELP: &str = concat!(
    "+core/offer <core>=<player> price=<n>  --  Offers a governed core in your hands to one player,\n",
    "for 28 days. Name your ship instead to offer her with the core fitted in her: both titles\n",
    "and her hold pass together. price is in centicredits; the venue takes 3% and the Registry 1%.\n",
    "+core/offer <core>=withdraw  withdraws it.  +core/offers  lists offers to and from you.\n",
    "+core/accept <core>  shows an offer made to you; +core/accept <core>=go buys it, paying\n",
    "from your own credit account as the core and its title pass to you."
);
/// The offer's grammar.
pub const RESALE_OFFER_USAGE: &str =
    "Usage: +core/offer <core or ship>=<player> price=<n>, or +core/offer <core or ship>=withdraw";
/// The acceptance's grammar.
pub const RESALE_ACCEPT_USAGE: &str = "Usage: +core/accept <core or ship>[=go]";
/// No resale on this world.
pub const RESALE_OFF: &str = "Governed cores do not change hands between players on this world.";
/// Neither a governed core nor a cored ship of the seller's.
pub const RESALE_NOT_CORE: &str =
    "That is neither a governed core nor a ship of yours with one fitted.";
/// The core's title is not the seller's alone.
pub const RESALE_NOT_YOURS: &str = "That core's title is not yours alone, so you cannot offer it.";
/// Not in the seller's hands.
pub const RESALE_NOT_HELD: &str = "You are not holding that core.";
/// Fitted: offer the ship (`{name}`, `{n}`).
pub const RESALE_FITTED: &str =
    "That core is fitted to the {name}; offer her with it: +core/offer #{n}=<player> price=<n>.";
/// Destroyed.
pub const RESALE_DESTROYED: &str = "A destroyed core is not offered for sale.";
/// No such buyer.
pub const RESALE_NO_PLAYER: &str = "There is no player by that name to offer it to.";
/// The seller as buyer.
pub const RESALE_SELF: &str = "You cannot sell a core to yourself.";
/// A price that leaves nothing.
pub const RESALE_PRICE: &str =
    "Name a price in centicredits that leaves you something after the venue's cut and the levy.";
/// Offered (`{core}`, `{buyer}`, `{price}`).
pub const RESALE_OFFERED: &str = "You offer core #{core} to {buyer} for {price}; it stands 28 days. Nothing moves until they accept.";
/// Offered with her hull (`{name}`, `{core}`, `{buyer}`, `{price}`).
pub const RESALE_OFFERED_HULL: &str =
    "You offer the {name} with her core #{core} to {buyer} for {price}; it stands 28 days.";
/// Withdrawn (`{core}`).
pub const RESALE_WITHDRAWN: &str = "Your offer of core #{core} is withdrawn.";
/// No offer of the seller's to withdraw.
pub const RESALE_NO_OWN_OFFER: &str = "You have no standing offer of that core.";
/// No offer for this player (a third party hears the same).
pub const RESALE_NO_OFFER: &str = "No offer of that stands for you.";
/// Lapsed.
pub const RESALE_LAPSED: &str = "That offer has lapsed; ask the seller to offer it again.";
/// **Doc 48e (48d H-07)**: a stored core changes hands where both stand (Q-48-6's local delivery).
pub const RESALE_APART: &str =
    "A stored core changes hands where you and its seller both stand; meet them, then accept.";
/// Changed since the offer.
pub const RESALE_CHANGED: &str =
    "That offer no longer stands: what it sells has changed since it was made.";
/// The quote of a stored core (`{seller}`, `{core}`, `{price}`, `{net}`, `{cut}`, `{levy}`).
pub const RESALE_QUOTE: &str =
    "{seller} offers you core #{core} for {price}: {net} to them, {cut} to the venue, {levy} levy.";
/// The quote of a cored hull (`{seller}`, `{name}`, `{core}`, `{price}`).
pub const RESALE_QUOTE_HULL: &str =
    "{seller} offers you the {name}, her hold and her core #{core}, for {price}, cuts included.";
/// How to buy (`{core}`).
pub const RESALE_HOW: &str =
    "+core/accept #{core}=go buys it; your money leaves your account only as it passes to you.";
/// The buyer cannot pay (`{price}`).
pub const RESALE_SHORT: &str = "Your credit account cannot cover {price}.";
/// No account to pay from.
pub const RESALE_NO_ACCOUNT: &str = "You hold no credit account to pay from.";
/// Submitted (`{core}`).
pub const RESALE_ACCEPTING: &str = "You accept the offer of core #{core}.";
/// The buyer's receipt (`{core}`, `{price}`).
pub const RESALE_BOUGHT: &str = "Core #{core} and its title are yours, for {price}.";
/// The buyer's receipt for a cored hull (`{name}`, `{core}`, `{price}`).
pub const RESALE_BOUGHT_HULL: &str =
    "The {name}, her hold and her core #{core} are yours, with both titles, for {price}.";
/// The seller's receipt (`{buyer}`, `{core}`, `{net}`).
pub const RESALE_SOLD: &str = "{buyer} takes core #{core}; you are paid {net}.";
/// No offers either way.
pub const RESALE_NO_OFFERS: &str = "No core offers stand to or from you.";
/// One offer listed (`{core}`, `{with}`, `{seller}`, `{buyer}`, `{price}`, `{days}`).
pub const RESALE_OFFER_ROW: &str =
    "Core #{core}{with}: {seller} to {buyer}, {price}; {days} days left to accept it.";
/// Busy.
pub const RESALE_BUSY: &str = "You are already doing something; try again in a moment.";

fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// **What the resale composes with**: who takes the venue's cut and the levy, and at what rates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResaleTerms {
    pub venue: Recipient,
    pub levy: Recipient,
    pub venue_permille: u16,
    pub levy_permille: u16,
}
impl ResaleTerms {
    /// **The launch's resale** (provisional rates, Q-48-8's): the venue's cut to the venue, the
    /// levy to the Registry.
    #[must_use]
    pub fn provisional(venue: Recipient, levy: Recipient) -> Self {
        // Doc 48e (48d M-03): EX.4's exchange rates, one source for every sale's cut and levy.
        Self {
            venue,
            levy,
            // A rate that does not fit is refused loudly by the config's own check at composition.
            venue_permille: u16::try_from(mush_market::fees::SALE_CUT_PERMILLE).unwrap_or(u16::MAX),
            levy_permille: u16::try_from(mush_market::fees::LEVY_PERMILLE).unwrap_or(u16::MAX),
        }
    }
    /// The economy's closed leg.
    ///
    /// # Errors
    /// A descriptor that does not register, or terms the economy refuses.
    pub(crate) fn economy_config(&self) -> Result<CoreResaleConfig, ModError> {
        let config = CoreResaleConfig {
            action: resale_action_ref()?,
            venue: self.venue.clone(),
            levy: self.levy.clone(),
            venue_permille: self.venue_permille,
            levy_permille: self.levy_permille,
        };
        config.validate().map_err(bad)?;
        Ok(config)
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(bad)?,
        DefinitionId::new(id).map_err(bad)?,
        1,
        payload,
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(bad)
}
/// The offer's action descriptor (tacsim, version 1).
///
/// # Errors
/// A descriptor that does not build.
pub(crate) fn offer_action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "action",
        OFFER_ACTION,
        json!({"format":1,"protocol":"core_resale_offer_v1","subject":"self_actor","targets":0,
            "window_ic_seconds":OFFER_IC_SECONDS,"settlement":"none_offer_record_only"}),
    )
}
/// The acceptance's action descriptor (tacsim, version 1).
///
/// # Errors
/// A descriptor that does not build.
pub(crate) fn resale_action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "action",
        RESALE_ACTION,
        json!({"format":1,"protocol":"governed_core_resale_v1","subject":"self_actor","targets":0,
            "buyer":"the_offer_named_player",
            "settlement":"buyer_money_titles_and_object_same_union_at_acceptance"}),
    )
}
/// The offer's and the acceptance's outcome descriptors.
///
/// # Errors
/// A descriptor that does not build.
pub(crate) fn outcome_descriptors() -> Result<[DefinitionDescriptor; 2], ModError> {
    Ok([
        descriptor("outcome", OFFER_OUTCOME, json!({"format":1}))?,
        descriptor("outcome", RESALE_OUTCOME, json!({"format":1}))?,
    ])
}
fn reference(descriptor: DefinitionDescriptor) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(bad)?, descriptor)
        .map_err(bad)
}
/// The registered acceptance reference, for the economy's closed leg.
///
/// # Errors
/// A descriptor that does not register.
pub(crate) fn resale_action_ref() -> Result<DefinitionRef, ModError> {
    reference(resale_action_descriptor()?)
}

/// **A cored hull sold with her core**: her object, the title revision and the digest of her
/// parts the offer read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferedHull {
    pub hull: ObjId,
    pub title_revision: u64,
    pub digest: u64,
}

/// **A standing private offer**, keyed by its core: the seller, the one buyer, the price, the
/// state it pinned and its IC window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResaleOffer {
    pub format: u16,
    pub core: ObjId,
    pub seller: ObjId,
    pub buyer: ObjId,
    pub price_minor: i64,
    pub core_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<OfferedHull>,
    pub made_ic: i64,
    pub lapses_ic: i64,
}
impl ResaleOffer {
    fn check(&self) -> Result<(), String> {
        let ids = [
            Some(self.core),
            Some(self.seller),
            Some(self.buyer),
            self.hull.as_ref().map(|h| h.hull),
        ];
        let flat: Vec<ObjId> = ids.iter().flatten().copied().collect();
        let distinct = flat.iter().collect::<std::collections::BTreeSet<_>>().len() == flat.len();
        if self.format != 1
            || !distinct
            || flat.iter().any(|id| id.0 == 0)
            || self.price_minor <= 0
            || self.core_revision == 0
            || self.hull.as_ref().is_some_and(|h| h.title_revision == 0)
            || self.lapses_ic <= self.made_ic
            || self.lapses_ic.checked_sub(self.made_ic) > Some(OFFER_IC_SECONDS)
        {
            return Err("malformed core resale offer".into());
        }
        Ok(())
    }
    /// The pin the policy reads again: the core's title revision and, with a hull, hers and the
    /// digest of her parts.
    fn pin(&self) -> u64 {
        pin_of(
            self.core_revision,
            self.hull.as_ref().map(|h| (h.title_revision, h.digest)),
        )
    }
}

fn pin_of(core_revision: u64, hull: Option<(u64, u64)>) -> u64 {
    let (revision, digest) = hull.unwrap_or((0, 0));
    mush_market::declared_hash(&format!("{core_revision}:{revision}:{digest}"))
}

/// Why an offer or an acceptance is refused, in the player's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotCore,
    NotYours,
    NotHeld,
    Fitted(String, ObjId),
    Destroyed,
    Hull(String),
}
impl Refusal {
    fn line(&self) -> String {
        match self {
            Self::NotCore => RESALE_NOT_CORE.to_owned(),
            Self::NotYours => RESALE_NOT_YOURS.to_owned(),
            Self::NotHeld => RESALE_NOT_HELD.to_owned(),
            Self::Fitted(name, hull) => RESALE_FITTED
                .replace("{name}", name)
                .replace("{n}", &hull.0.to_string()),
            Self::Destroyed => RESALE_DESTROYED.to_owned(),
            Self::Hull(line) => line.clone(),
        }
    }
}

/// **What the seller holds to sell**: the core and its title revision, and the hull it is fitted
/// in when it goes with her.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Holding {
    core: ObjId,
    core_revision: u64,
    hull: Option<(ObjId, u64, crate::port::hull_sale::CoreLeg)>,
}
impl Holding {
    fn pin(&self) -> u64 {
        pin_of(
            self.core_revision,
            self.hull
                .as_ref()
                .map(|(_, revision, leg)| (*revision, leg.digest)),
        )
    }
}

/// A title the seller holds alone: theirs, in no scope, unclaimed, unfrozen, not stolen.
fn alone(economy: &EconomyStore, object: ObjId, seller: ObjId) -> Option<u64> {
    let title = economy.snapshot().asset(object)?;
    (title.title_owner == mush_org::PrincipalId::Object(seller)
        && title.scope.is_none()
        && title.encumbrances.is_empty()
        && !title.frozen
        && title.stolen.is_none()
        && title.lifecycle == mush_economy::AssetLifecycle::Active)
        .then_some(title.revision)
}

/// **The one read** the offer, the acceptance and the kernel policy share: what `seller` holds
/// to sell when they name `object`, a governed core in their own hands or a ship of theirs with
/// one fitted.
pub(crate) fn holding(
    world: &World,
    sale: &SaleTerms,
    seller: ObjId,
    object: ObjId,
) -> Result<Holding, Refusal> {
    let economy = world
        .module_state::<EconomyStore>()
        .ok_or(Refusal::NotCore)?;
    let thing = world.get(object).ok_or(Refusal::NotCore)?;
    if thing.kind == ObjKind::Thing && economy.is_managed_core(object) {
        let core = object;
        let place = thing.location.ok_or(Refusal::NotHeld)?;
        if place != seller {
            let fitted = crate::minds::registry_inspection::seizure::fitted_core(world, place);
            if fitted == Some(core) {
                let name = world
                    .get(place)
                    .map(|o| o.name.to_string())
                    .unwrap_or_default();
                return Err(Refusal::Fitted(name, place));
            }
            return Err(Refusal::NotHeld);
        }
        let revision = alone(economy, core, seller)
            .filter(|_| !economy.core_impounded(core))
            .ok_or(Refusal::NotYours)?;
        if sale.refit.stored_core_destroyed(world, core) != Some(false) {
            return Err(Refusal::Destroyed);
        }
        return Ok(Holding {
            core,
            core_revision: revision,
            hull: None,
        });
    }
    // A ship of theirs, with her governed core.
    let leg = crate::port::hull_sale::core_leg(world, object, seller)
        .ok_or(Refusal::NotCore)?
        .map_err(Refusal::Hull)?;
    let hull_revision = alone(economy, object, seller).ok_or(Refusal::NotCore)?;
    let core = leg.core;
    if world.get(core).and_then(|o| o.location) != Some(object) {
        return Err(Refusal::NotCore);
    }
    let revision = alone(economy, core, seller)
        .filter(|_| !economy.core_impounded(core))
        .ok_or(Refusal::NotYours)?;
    Ok(Holding {
        core,
        core_revision: revision,
        hull: Some((object, hull_revision, leg)),
    })
}

/// **Whether two characters stand together** (doc 48e, 48d H-07): in one room, or in rooms of one
/// venue. A stored core is handed over, so it changes hands where both stand; a cored hull's
/// title passes where she lies, and her core does not move.
fn together(world: &World, a: ObjId, b: ObjId) -> bool {
    let room = |who: ObjId| world.get(who).and_then(|o| o.location);
    let (Some(ra), Some(rb)) = (room(a), room(b)) else {
        return false;
    };
    if ra == rb {
        return true;
    }
    let venue = |room| mush_market::venue_gate(world)?.venue_of_room(world, room);
    matches!((venue(ra), venue(rb)), (Some(x), Some(y)) if x == y)
}

/// A live player other than `seller`.
fn buyer_ok(world: &World, seller: ObjId, buyer: ObjId) -> bool {
    buyer != seller
        && world.get(buyer).is_some_and(|o| {
            o.kind == ObjKind::Player && !o.flags.contains(mush_core::Flags::GOING)
        })
}

/// **The kernel's trusted resale policy**: the one read, for the request's seller and object,
/// on the state the offer pinned.
pub(crate) struct ResalePolicy {
    sale: Arc<SaleTerms>,
}
impl CoreResalePolicy for ResalePolicy {
    fn read(
        &self,
        world: &World,
        buyer: ObjId,
        request: &ResaleRequest,
    ) -> Result<ResaleFacts, ParticipantError> {
        let refused = || ParticipantError::refused("The core resale is unavailable or changed.");
        let seller = request.seller();
        let named = request.hull().unwrap_or(request.core());
        let held = holding(world, &self.sale, seller, named).map_err(|_| refused())?;
        if held.core != request.core()
            || held.hull.as_ref().map(|(hull, _, _)| *hull) != request.hull()
            || held.pin() != request.pin()
            || !buyer_ok(world, seller, buyer)
            || (request.hull().is_none() && !together(world, seller, buyer))
        {
            return Err(refused());
        }
        Ok(ResaleFacts::new(held.core, seller, buyer, request.hull()))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferParameters {
    object: u32,
    #[serde(default)]
    buyer: Option<u32>,
    #[serde(default)]
    price_minor: Option<i64>,
    #[serde(default)]
    withdraw: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptParameters {
    core: u32,
}

/// **The offer a character is projected** for either act: to any player, about themselves; the
/// parameters say what to name, and the prepare's read says whether it may be done.
fn draft(
    (action, outcome): (&DefinitionRef, &DefinitionRef),
    query: &mush_core::LegalActionQuery,
    engine: &mush_core::Engine,
    parameters: &[(&str, bool, mush_core::ProjectedParameterKind)],
    (id, label, description): (&str, &str, &str),
    refusal: &ActionError,
) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
    let player = engine
        .world
        .get(query.actor())
        .is_some_and(|o| o.kind == ObjKind::Player);
    if query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
        || query.projected_subject() != &PerceivedRef::SelfActor
        || !player
    {
        return Ok(None);
    }
    let fail = || refusal.clone();
    let definitions = engine.registry.definitions().ok_or_else(fail)?;
    let mut guards = engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())
        .map_err(|_| fail())?;
    for r in [action, outcome] {
        guards
            .add_definition(r.clone(), definitions)
            .map_err(|_| fail())?;
    }
    let parameters = parameters
        .iter()
        .map(|(name, required, kind)| {
            Ok(mush_core::ProjectedParameter {
                id: mush_core::mechanics::CanonicalId::new(*name).map_err(|_| fail())?,
                required: *required,
                kind: kind.clone(),
            })
        })
        .collect::<Result<Vec<_>, ActionError>>()?;
    Ok(Some(mush_core::ProviderOfferDraft {
        fixed_parameters: json!({}),
        parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
        targets: mush_core::ProjectedTargetSchema::None,
        required_channels: Vec::new(),
        expires: None,
        presentation: mush_core::ActionPresentation::new(id, label, description)
            .map_err(|_| fail())?,
        guards: guards.finish().map_err(|_| fail())?,
    }))
}

/// An object's number, as a projected parameter.
const OBJECT: mush_core::ProjectedParameterKind = mush_core::ProjectedParameterKind::Integer {
    minimum: 1,
    maximum: 4_294_967_295,
};

fn self_actor(inv: &ActionInvocation) -> bool {
    inv.principal() == &(PrincipalId::Object { id: inv.actor() })
        && inv.subject() == &PerceivedRef::SelfActor
        && inv.targets().is_empty()
}

/// **The offer's provider**: the seller's own act, one record written or removed.
struct OfferProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    offers: StateFamilyHandle<ObjId, ResaleOffer>,
    sale: Arc<SaleTerms>,
    resale: Arc<ResaleTerms>,
    refusal: ActionError,
}
impl ActionProvider for OfferProvider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        if !self_actor(inv) {
            return Err(self.refusal.clone());
        }
        serde_json::from_value::<OfferParameters>(inv.parameters().clone())
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }

    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let fail = || self.refusal.clone();
        self.validate_invocation(inv)?;
        let p: OfferParameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| fail())?;
        let seller = inv.actor();
        let world = ctx.world().map_err(|_| fail())?;
        let held = holding(world, &self.sale, seller, ObjId(p.object)).map_err(|_| fail())?;
        let standing = txn
            .get(&self.offers, &held.core)
            .map_err(|_| fail())?
            .cloned();
        let summary = if p.withdraw {
            if p.buyer.is_some() || p.price_minor.is_some() {
                return Err(fail());
            }
            if standing.as_ref().is_none_or(|o| o.seller != seller) {
                return Err(fail());
            }
            txn.remove(&self.offers, &held.core).map_err(|_| fail())?;
            RESALE_WITHDRAWN.replace("{core}", &held.core.0.to_string())
        } else {
            let buyer = ObjId(p.buyer.ok_or_else(fail)?);
            let price = p.price_minor.ok_or_else(fail)?;
            let config = self.resale.economy_config().map_err(|_| fail())?;
            if !buyer_ok(world, seller, buyer) || config.split(Amount::from_minor(price)).is_none()
            {
                return Err(fail());
            }
            let now = world.world_ic_now().at;
            let offer = ResaleOffer {
                format: 1,
                core: held.core,
                seller,
                buyer,
                price_minor: price,
                core_revision: held.core_revision,
                hull: held.hull.as_ref().map(|(hull, revision, leg)| OfferedHull {
                    hull: *hull,
                    title_revision: *revision,
                    digest: leg.digest,
                }),
                made_ic: now,
                lapses_ic: now.checked_add(OFFER_IC_SECONDS).ok_or_else(fail)?,
            };
            offer.check().map_err(|_| fail())?;
            // A standing offer of the same core is replaced: the title check above proves the
            // core is this seller's now, whoever made the old one.
            if standing.is_some() {
                txn.replace(&self.offers, &held.core, offer)
                    .map_err(|_| fail())?;
            } else {
                txn.insert(&self.offers, &held.core, offer)
                    .map_err(|_| fail())?;
            }
            let buyer_name = world
                .get(buyer)
                .map(|o| o.name.to_string())
                .unwrap_or_default();
            let template = match &held.hull {
                Some((hull, _, _)) => RESALE_OFFERED_HULL.replace(
                    "{name}",
                    &world
                        .get(*hull)
                        .map(|o| o.name.to_string())
                        .unwrap_or_default(),
                ),
                None => RESALE_OFFERED.to_owned(),
            };
            template
                .replace("{core}", &held.core.0.to_string())
                .replace("{buyer}", &buyer_name)
                .replace("{price}", &super::credits(price))
        };
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            json!({"core": held.core.0, "seller": seller.0, "withdraw": p.withdraw, "summary": summary}),
        )
        .map_err(|_| fail())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(vec![self.action.clone(), self.outcome.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action {
            return Ok(None);
        }
        draft(
            (&self.action, &self.outcome),
            query,
            engine,
            &[
                ("object", true, OBJECT),
                ("buyer", false, OBJECT),
                (
                    "price_minor",
                    false,
                    mush_core::ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::MAX,
                    },
                ),
                (
                    "withdraw",
                    false,
                    mush_core::ProjectedParameterKind::Boolean,
                ),
            ],
            (
                "tacsim-core-offer",
                "Offer a governed core",
                "Offer a governed core in your hands, or your ship with one fitted, to one player.",
            ),
            &self.refusal,
        )
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &mush_core::mechanics::RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        project(
            (action, &self.action),
            (outcome, &self.outcome),
            viewer,
            OFFER_ACTION,
            &self.refusal,
        )
    }
}

/// The receipt the prepare rendered: the summary for the seller, and (an acceptance's) the
/// seller's own line for them.
fn project(
    (action, expected): (&DefinitionRef, &DefinitionRef),
    (outcome, schema): (&mush_core::mechanics::RulesValue, &DefinitionRef),
    viewer: &PrincipalId,
    id: &str,
    refusal: &ActionError,
) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
    if action != expected || outcome.schema() != schema {
        return Err(refusal.clone());
    }
    let data = outcome.data();
    let who = |key: &str| {
        data.get(key)
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(|id| PrincipalId::Object { id: ObjId(id) })
    };
    let key = if who("buyer").as_ref() == Some(viewer) && id == RESALE_ACTION {
        "summary"
    } else if who("seller").as_ref() == Some(viewer) {
        if id == RESALE_ACTION {
            "seller_summary"
        } else {
            "summary"
        }
    } else {
        return Err(refusal.clone());
    };
    let summary = data
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| refusal.clone())?;
    mush_proto::ProjectedActionResult::new(id, summary, data.clone())
        .map(Some)
        .map_err(|_| refusal.clone())
}

/// **The acceptance's provider**: read the offer, bind the economy's resale, bind the kernel's
/// image, remove the offer, answer both parties' receipts.
struct AcceptProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    economy: ParticipantHandle<InitialFilingParticipant>,
    resales: CoreResales<ResalePolicy>,
    offers: StateFamilyHandle<ObjId, ResaleOffer>,
    sale: Arc<SaleTerms>,
    refusal: ActionError,
}
impl ActionProvider for AcceptProvider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        if !self_actor(inv) {
            return Err(self.refusal.clone());
        }
        serde_json::from_value::<AcceptParameters>(inv.parameters().clone())
            .ok()
            .filter(|p| p.core != 0)
            .map(|_| ())
            .ok_or_else(|| self.refusal.clone())
    }

    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let fail = || self.refusal.clone();
        self.validate_invocation(inv)?;
        let p: AcceptParameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| fail())?;
        let core = ObjId(p.core);
        let buyer = inv.actor();
        // The offer: it names this buyer, and it stands.
        let offer = txn
            .get(&self.offers, &core)
            .map_err(|_| fail())?
            .cloned()
            .ok_or_else(fail)?;
        let (request, names) = {
            let world = ctx.world().map_err(|_| fail())?;
            if offer.check().is_err()
                || offer.core != core
                || offer.buyer != buyer
                || world.world_ic_now().at > offer.lapses_ic
            {
                return Err(fail());
            }
            let named = offer.hull.as_ref().map_or(core, |h| h.hull);
            let held = holding(world, &self.sale, offer.seller, named).map_err(|_| fail())?;
            if held.core != core
                || held.pin() != offer.pin()
                || !buyer_ok(world, offer.seller, buyer)
            {
                return Err(fail());
            }
            let account = |who| crate::port::credit::own_account(world, who).map(|a| a.id);
            let request = RegistryCoreResale {
                core,
                title_revision: offer.core_revision,
                hull: held.hull.as_ref().map(|(hull, revision, leg)| ResaleHull {
                    hull: *hull,
                    title_revision: *revision,
                    hold: leg.hold,
                }),
                seller: offer.seller,
                seller_account: account(offer.seller).ok_or_else(fail)?,
                buyer_account: account(buyer).ok_or_else(fail)?,
                price: Amount::from_minor(offer.price_minor),
            };
            let name = |id: ObjId| {
                world
                    .get(id)
                    .map(|o| o.name.to_string())
                    .unwrap_or_default()
            };
            let names = (name(buyer), offer.hull.as_ref().map(|h| name(h.hull)));
            (request, names)
        };
        let native = ctx
            .prepare_owner(&self.economy, EconomyRequest::CoreResale(request.clone()))
            .map_err(|_| fail())?
            .core_resale()
            .map_err(|_| fail())?;
        if native.action() != &self.action || native.commit_token() != ctx.commit_token() {
            return Err(fail());
        }
        let resold = ctx
            .resell_core(
                &self.resales,
                txn,
                ResaleRequest::new(
                    core,
                    offer.seller,
                    offer.hull.as_ref().map(|h| h.hull),
                    offer.pin(),
                ),
            )
            .map_err(|_| fail())?;
        if resold.commit() != ctx.commit_token()
            || resold.core() != core
            || resold.seller() != offer.seller
            || resold.buyer() != buyer
        {
            return Err(fail());
        }
        txn.remove(&self.offers, &core).map_err(|_| fail())?;
        let receipt = native.subject();
        let price = super::credits(receipt.price.minor());
        let summary = match &names.1 {
            Some(hull) => RESALE_BOUGHT_HULL.replace("{name}", hull),
            None => RESALE_BOUGHT.to_owned(),
        }
        .replace("{core}", &core.0.to_string())
        .replace("{price}", &price);
        let seller_summary = RESALE_SOLD
            .replace("{buyer}", &names.0)
            .replace("{core}", &core.0.to_string())
            .replace("{net}", &super::credits(receipt.proceeds().minor()));
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            json!({
                "core": core.0,
                "hull": offer.hull.as_ref().map(|h| h.hull.0),
                "seller": offer.seller.0,
                "buyer": buyer.0,
                "price_minor": receipt.price.minor(),
                "summary": summary,
                "seller_summary": seller_summary,
            }),
        )
        .map_err(|_| fail())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(vec![self.action.clone(), self.outcome.clone()])
            .with_eligible_witnesses(vec![
                inv.principal().clone(),
                PrincipalId::Object { id: offer.seller },
            ]))
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action {
            return Ok(None);
        }
        draft(
            (&self.action, &self.outcome),
            query,
            engine,
            &[("core", true, OBJECT)],
            (
                "tacsim-core-accept",
                "Accept a core offer",
                "Buy a governed core a player has offered you, paying as it passes to you.",
            ),
            &self.refusal,
        )
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &mush_core::mechanics::RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        project(
            (action, &self.action),
            (outcome, &self.outcome),
            viewer,
            RESALE_ACTION,
            &self.refusal,
        )
    }
}

/// **The slot** this crate registers when the resale composes: both actions, the terms, the
/// kernel's service (the boot audit), the offers' family and the commands' idempotency
/// attempts (RAM only).
#[derive(Clone)]
pub struct ResaleHost {
    offer: DefinitionRef,
    accept: DefinitionRef,
    sale: Arc<SaleTerms>,
    resale: Arc<ResaleTerms>,
    resales: CoreResales<ResalePolicy>,
    offers: StateFamilyHandle<ObjId, ResaleOffer>,
    attempts: Arc<Mutex<(i64, u64)>>,
}
impl ResaleHost {
    /// The slot in `world`, owner-checked.
    #[must_use]
    pub fn of(world: &World) -> Option<&ResaleHost> {
        if world
            .module_state_owner(std::any::TypeId::of::<ResaleHost>())
            .map(|owner| owner.as_str())
            != Some(crate::NS)
        {
            return None;
        }
        world.module_state::<ResaleHost>()
    }
}

/// **Register both providers, the kernel's service, the offers' family and the slot** over the
/// registered actions and outcomes.
///
/// # Errors
/// A registrar that is not this crate's, a reference that is not the resale's, or a refused
/// registration.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    sale: &SaleTerms,
    resale: &ResaleTerms,
    (offer, offered): (DefinitionRef, DefinitionRef),
    (accept, resold): (DefinitionRef, DefinitionRef),
) -> Result<(), ModError> {
    if reg.module() != crate::NS
        || accept != resale_action_ref()?
        || offer != reference(offer_action_descriptor()?)?
    {
        return Err(bad("core resale registration differs from its composition"));
    }
    let sale = Arc::new(sale.clone());
    let resale = Arc::new(resale.clone());
    let offers = reg.state_family::<ObjId, ResaleOffer, _, _, _, _>(
        crate::port::families::family_descriptor(
            OFFER_FAMILY,
            "tacsim.core_resale_offer_key.v1",
            "tacsim.core_resale_offer",
        )?,
        |key: &ObjId| Ok(key.0.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        ResaleOffer::check,
        |value: &ResaleOffer| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let resales = CoreResales::register(
        reg,
        accept.clone(),
        ResalePolicy {
            sale: Arc::clone(&sale),
        },
    )?;
    let economy = reg.request_owner_participant::<InitialFilingParticipant>(accept.clone())?;
    let refusal =
        |line: &str| ActionError::new(ActionErrorCode::ProviderRejected, false, line).map_err(bad);
    reg.action_provider(
        offer.clone(),
        OfferProvider {
            action: offer.clone(),
            outcome: offered,
            offers: offers.clone(),
            sale: Arc::clone(&sale),
            resale: Arc::clone(&resale),
            refusal: refusal("The core offer is unavailable or changed.")?,
        },
    )?;
    reg.action_provider(
        accept.clone(),
        AcceptProvider {
            action: accept.clone(),
            outcome: resold,
            economy,
            resales: resales.clone(),
            offers: offers.clone(),
            sale: Arc::clone(&sale),
            refusal: refusal("The core resale is unavailable or changed.")?,
        },
    )?;
    reg.state(ResaleHost {
        offer,
        accept,
        sale,
        resale,
        resales,
        offers,
        attempts: Arc::new(Mutex::new((i64::MIN, 0))),
    });
    Ok(())
}

/// **The boot pass**: the kernel's resale records and the economy's replayed resale receipts
/// pair one to one, by operation, core, seller, buyer and hull. A roster without the slot audits
/// nothing.
///
/// # Errors
/// A record either side holds that the other does not, or one that differs from its receipt.
pub(crate) fn boot_audit(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let Some(host) = ctx.state::<ResaleHost>() else {
        return Ok(());
    };
    let mut resold = std::collections::BTreeMap::new();
    host.resales
        .audit(ctx, |row: ResoldCore| {
            resold.insert(row.commit(), row);
            Ok(())
        })
        .map_err(|_| bad("tacsim: the core resale records do not read"))?;
    let economy = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or_else(|| bad("tacsim: the core resale needs the economy"))?;
    let receipts = economy
        .core_resale_receipts()
        .map_err(|_| bad("tacsim: the economy's core resale receipts are not complete"))?;
    let mut paired = 0usize;
    for receipt in receipts {
        let row = resold
            .get(&receipt.commit)
            .ok_or_else(|| bad("tacsim: a core resale receipt has no resale record"))?;
        if row.core() != receipt.core
            || row.seller() != receipt.seller
            || row.buyer() != receipt.buyer
            || row.hull() != receipt.hull
        {
            return Err(bad("tacsim: a core resale record differs from its receipt"));
        }
        paired += 1;
    }
    if paired != resold.len() {
        return Err(bad("tacsim: a core resale record has no economy receipt"));
    }
    Ok(())
}

/// The object a line names: `#<n>` or `<n>`.
fn object_of(text: &str) -> Option<ObjId> {
    let text = text.trim();
    text.strip_prefix('#')
        .unwrap_or(text)
        .parse::<u32>()
        .ok()
        .filter(|n| *n != 0)
        .map(ObjId)
}

/// The standing offer keyed by `core`, read through `ctx`.
fn offer_of(ctx: &Ctx<'_>, host: &ResaleHost, core: ObjId) -> Option<ResaleOffer> {
    ctx.action_state(&host.offers, &core)
        .ok()
        .flatten()
        .cloned()
}

/// The core an acceptance names: the core itself, or the ship an offer sells it with.
fn offered_core(ctx: &Ctx<'_>, host: &ResaleHost, object: ObjId) -> Option<ObjId> {
    if offer_of(ctx, host, object).is_some() {
        return Some(object);
    }
    crate::minds::registry_inspection::seizure::fitted_core(ctx.w, object).filter(|core| {
        offer_of(ctx, host, *core)
            .is_some_and(|o| o.hull.as_ref().is_some_and(|h| h.hull == object))
    })
}

/// Submit `action` with `params` for the actor, keyed per object and IC second.
fn submit(
    ctx: &mut Ctx<'_>,
    host: &ResaleHost,
    action: DefinitionRef,
    kind: &str,
    object: ObjId,
    params: Value,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
        return Err(CmdError::msg(RESALE_BUSY));
    }
    let key = crate::minds::market_commands::next_key(
        &host.attempts,
        kind,
        &object.0.to_string(),
        ctx.w.world_ic_now().at,
    )?;
    ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), params, key);
    Ok(())
}

/// **`+core/offer`, `+core/offers` and `+core/accept`**: the offer, the list, the quote or the
/// acceptance. Every refusal is one sentence.
///
/// # Errors
/// The refusal line.
pub(crate) fn resale_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(host) = ResaleHost::of(ctx.w).cloned() else {
        return Err(CmdError::msg(RESALE_OFF));
    };
    if ctx.switches.has("offers") {
        offers(ctx, &host);
        return Ok(());
    }
    if ctx.switches.has("offer") {
        return offer(ctx, &host);
    }
    accept(ctx, &host)
}

fn offer(ctx: &mut Ctx<'_>, host: &ResaleHost) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    let (target, rest) = raw
        .split_once('=')
        .ok_or_else(|| CmdError::msg(RESALE_OFFER_USAGE))?;
    let object = object_of(target).ok_or_else(|| CmdError::msg(RESALE_OFFER_USAGE))?;
    let seller = ctx.actor;
    let held =
        holding(ctx.w, &host.sale, seller, object).map_err(|why| CmdError::msg(why.line()))?;
    if rest.trim().eq_ignore_ascii_case("withdraw") {
        if offer_of(ctx, host, held.core).is_none_or(|o| o.seller != seller) {
            return Err(CmdError::msg(RESALE_NO_OWN_OFFER));
        }
        submit(
            ctx,
            host,
            host.offer.clone(),
            "core_resale_withdraw",
            held.core,
            json!({"object": object.0, "withdraw": true}),
        )?;
        return Ok(());
    }
    let mut words = rest.split_whitespace();
    let who = words
        .next()
        .ok_or_else(|| CmdError::msg(RESALE_OFFER_USAGE))?;
    let price = match (
        words.next().and_then(|w| w.strip_prefix("price=")),
        words.next(),
    ) {
        (Some(n), None) => n.parse::<i64>().ok(),
        _ => None,
    }
    .ok_or_else(|| CmdError::msg(RESALE_OFFER_USAGE))?;
    let buyer = match object_of(who) {
        Some(id) => ctx
            .w
            .get(id)
            .filter(|o| o.kind == ObjKind::Player)
            .map(|_| id),
        None => ctx.w.resolve_player(who),
    }
    .ok_or_else(|| CmdError::msg(RESALE_NO_PLAYER))?;
    if buyer == seller {
        return Err(CmdError::msg(RESALE_SELF));
    }
    let config = host
        .resale
        .economy_config()
        .map_err(|e| CmdError::msg(e.to_string()))?;
    if config.split(Amount::from_minor(price)).is_none() {
        return Err(CmdError::msg(RESALE_PRICE));
    }
    if let Some((hull, _, _)) = &held.hull {
        crate::port::hull_sale::core_leg_hold_empty(ctx, *hull).map_err(CmdError::msg)?;
    }
    submit(
        ctx,
        host,
        host.offer.clone(),
        "core_resale_offer",
        held.core,
        json!({"object": object.0, "buyer": buyer.0, "price_minor": price}),
    )
}

fn accept(ctx: &mut Ctx<'_>, host: &ResaleHost) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    let (target, go) = match raw.split_once('=') {
        Some((target, word)) if word.trim().eq_ignore_ascii_case("go") => (target.trim(), true),
        Some(_) => return Err(CmdError::msg(RESALE_ACCEPT_USAGE)),
        None => (raw.as_str(), false),
    };
    let object = object_of(target).ok_or_else(|| CmdError::msg(RESALE_ACCEPT_USAGE))?;
    let buyer = ctx.actor;
    // One answer for no offer and an offer to someone else: a private offer is private.
    let core = offered_core(ctx, host, object).ok_or_else(|| CmdError::msg(RESALE_NO_OFFER))?;
    let offer = offer_of(ctx, host, core)
        .filter(|o| o.buyer == buyer)
        .ok_or_else(|| CmdError::msg(RESALE_NO_OFFER))?;
    if ctx.w.world_ic_now().at > offer.lapses_ic {
        return Err(CmdError::msg(RESALE_LAPSED));
    }
    let named = offer.hull.as_ref().map_or(core, |h| h.hull);
    let current = holding(ctx.w, &host.sale, offer.seller, named)
        .ok()
        .filter(|held| held.core == core && held.pin() == offer.pin());
    if current.is_none() {
        return Err(CmdError::msg(RESALE_CHANGED));
    }
    if offer.hull.is_none() && !together(ctx.w, offer.seller, buyer) {
        return Err(CmdError::msg(RESALE_APART));
    }
    if let Some(hull) = offer.hull.as_ref().map(|h| h.hull) {
        crate::port::hull_sale::core_leg_hold_empty(ctx, hull).map_err(CmdError::msg)?;
    }
    let account = crate::port::credit::own_account(ctx.w, buyer)
        .cloned()
        .ok_or_else(|| CmdError::msg(RESALE_NO_ACCOUNT))?;
    let price = crate::port::coop::money_minor(ctx, offer.price_minor);
    let free = account
        .posted_balance
        .minor()
        .checked_sub(account.held_amount.minor())
        .unwrap_or(i64::MIN);
    if free < offer.price_minor {
        return Err(CmdError::msg(RESALE_SHORT.replace("{price}", &price)));
    }
    if go {
        submit(
            ctx,
            host,
            host.accept.clone(),
            "core_resale_accept",
            core,
            json!({"core": core.0}),
        )?;
        crate::port::coop::say(ctx, RESALE_ACCEPTING.replace("{core}", &core.0.to_string()));
        return Ok(());
    }
    let config = host
        .resale
        .economy_config()
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let (cut, levy, net) = config
        .split(Amount::from_minor(offer.price_minor))
        .ok_or_else(|| CmdError::msg(RESALE_CHANGED))?;
    let seller = ctx
        .w
        .get(offer.seller)
        .map(|o| o.name.to_string())
        .unwrap_or_default();
    let line = match offer.hull.as_ref() {
        Some(h) => RESALE_QUOTE_HULL.replace(
            "{name}",
            &ctx.w
                .get(h.hull)
                .map(|o| o.name.to_string())
                .unwrap_or_default(),
        ),
        None => RESALE_QUOTE.to_owned(),
    }
    .replace("{seller}", &seller)
    .replace("{core}", &core.0.to_string())
    .replace("{price}", &price)
    .replace("{net}", &crate::port::coop::money_minor(ctx, net))
    .replace("{cut}", &crate::port::coop::money_minor(ctx, cut))
    .replace("{levy}", &crate::port::coop::money_minor(ctx, levy));
    crate::port::coop::say(ctx, line);
    crate::port::coop::say(ctx, RESALE_HOW.replace("{core}", &core.0.to_string()));
    Ok(())
}

/// `+core/offers`: every standing offer to or from the actor, bounded by the family's own scan.
fn offers(ctx: &mut Ctx<'_>, host: &ResaleHost) {
    let actor = ctx.actor;
    let now = ctx.w.world_ic_now().at;
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..64 {
        let Ok(page) = ctx.scan_action_state(&host.offers, cursor.as_deref(), 128) else {
            break;
        };
        for entry in page.entries() {
            let o = entry.value();
            if (o.seller == actor || o.buyer == actor) && o.lapses_ic >= now {
                rows.push(o.clone());
            }
        }
        cursor = page.next_after().map(ToString::to_string);
        if cursor.is_none() {
            break;
        }
    }
    if rows.is_empty() {
        crate::port::coop::say(ctx, RESALE_NO_OFFERS);
        return;
    }
    let name = |w: &World, id: ObjId| w.get(id).map(|o| o.name.to_string()).unwrap_or_default();
    for o in rows {
        let with = o
            .hull
            .as_ref()
            .map(|h| format!(" with the {}", name(ctx.w, h.hull)))
            .unwrap_or_default();
        let line = RESALE_OFFER_ROW
            .replace("{core}", &o.core.0.to_string())
            .replace("{with}", &with)
            .replace("{seller}", &name(ctx.w, o.seller))
            .replace("{buyer}", &name(ctx.w, o.buyer))
            .replace(
                "{price}",
                &crate::port::coop::money_minor(ctx, o.price_minor),
            )
            .replace("{days}", &((o.lapses_ic - now) / 86_400).to_string());
        crate::port::coop::say(ctx, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The help and every line hold the 120-column floor.
    #[test]
    fn ex2b_the_lines_fit_and_the_offer_is_closed() {
        for line in RESALE_HELP.lines() {
            assert!(line.chars().count() <= 120, "{line}");
        }
        for line in [
            RESALE_OFFER_USAGE,
            RESALE_ACCEPT_USAGE,
            RESALE_OFF,
            RESALE_NOT_CORE,
            RESALE_NOT_YOURS,
            RESALE_NOT_HELD,
            RESALE_FITTED,
            RESALE_DESTROYED,
            RESALE_NO_PLAYER,
            RESALE_SELF,
            RESALE_PRICE,
            RESALE_OFFERED,
            RESALE_OFFERED_HULL,
            RESALE_WITHDRAWN,
            RESALE_NO_OWN_OFFER,
            RESALE_NO_OFFER,
            RESALE_LAPSED,
            RESALE_CHANGED,
            RESALE_QUOTE,
            RESALE_QUOTE_HULL,
            RESALE_HOW,
            RESALE_SHORT,
            RESALE_NO_ACCOUNT,
            RESALE_ACCEPTING,
            RESALE_BOUGHT,
            RESALE_BOUGHT_HULL,
            RESALE_SOLD,
            RESALE_NO_OFFERS,
            RESALE_OFFER_ROW,
            RESALE_BUSY,
        ] {
            assert!(line.chars().count() < 120, "{line}");
        }
        let offer = ResaleOffer {
            format: 1,
            core: ObjId(10),
            seller: ObjId(11),
            buyer: ObjId(12),
            price_minor: 500_000,
            core_revision: 3,
            hull: None,
            made_ic: 100,
            lapses_ic: 100 + OFFER_IC_SECONDS,
        };
        offer.check().expect("a well-formed offer");
        for broken in [
            ResaleOffer {
                buyer: ObjId(11),
                ..offer.clone()
            },
            ResaleOffer {
                price_minor: 0,
                ..offer.clone()
            },
            ResaleOffer {
                lapses_ic: 101 + OFFER_IC_SECONDS,
                ..offer.clone()
            },
            ResaleOffer {
                hull: Some(OfferedHull {
                    hull: ObjId(10),
                    title_revision: 1,
                    digest: 0,
                }),
                ..offer.clone()
            },
        ] {
            assert!(broken.check().is_err(), "{broken:?}");
        }
        // The pin moves with any pinned part.
        assert_ne!(pin_of(3, None), pin_of(4, None));
        assert_ne!(pin_of(3, Some((1, 9))), pin_of(3, Some((1, 8))));
        assert_eq!(object_of("#42"), Some(ObjId(42)));
        assert_eq!(object_of("0"), None);
    }
}
