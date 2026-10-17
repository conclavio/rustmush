//! ---- E7r2: P1 — the `Representation::Lot` lift, on the DELEGATED path ----
//!
//! §15.1 item 3, built the way 10b §15.5 was rewritten by cut 11's verify fold: **one
//! market-owned `Authoritative` action** whose `prepare_authoritative` calls cut 2's
//! [`InventoryFamilies::reserve`], [`InventoryFamilies::settle_sale`] and
//! [`InventoryFamilies::release`] on the families `mush-inventory` **delegates** to
//! this owner at composition — `mush-industry`'s shape, line for line, on an endpoint
//! the acting player does not own, under a deterministic
//! [`listing_holder`] `PrincipalId::Scoped` key.
//!
//! **What this file deliberately does not add.**
//!
//! * **No `InventoryVerb`.** §15.5's step (a) — *"three `InventoryVerb`s: `Reserve`,
//!   `Release`, `Consume`"* — is **superseded**, not discharged: the delegated path
//!   already exists and `mush-industry` has exercised it in production since cut 3.
//!   The absence of a player-reachable reserve/release verb is exactly what stops a
//!   seller taking a listing's hold off from under the market, and §15.5's constraint
//!   (3) calls that lockout **structural**.
//! * **No `ENGINE_OWNED_RESERVATIONS` entry.** §15.5's step (b) is **inapplicable**:
//!   that table is matched only against `HoldPurpose::Reservation` — a **money** hold
//!   on an account — while a lot reservation is a K4b `ReservationRecord` in a
//!   `ReservationBook`. Registering `"market.listing"` there would guard a hold this
//!   lift does not place. The money leg is E6c's, untouched.
//! * **No mint.** `mush_inventory::InventoryFamilies::mint_origin` is called from
//!   nowhere in this crate, and `e7r2.rs` probes the source for it: a sale that takes
//!   the money **moves** matter and creates none.
//!
//! # Where authority lives, and why there
//!
//! `prepare_authoritative` gets an `ActionTxn` and **no world**, so a vault's status,
//! `may_take_from_vault` and a `MarketStore` listing cannot be read inside it. Legality
//! therefore lives where the tree already puts it — [`ActionProvider::project_offer`],
//! which has an `&Engine` — and `Engine::resolve_legal_action` regenerates and
//! re-validates the offer immediately before preparation, so an endpoint whose custody
//! moved between the command and the commit drops out of the regenerated `Choice` list
//! and the submission is refused.
//!
//! * **Who may stock a lot** — the endpoint's custodian: `mush_economy::may_take_from_vault`
//!   over the seller's endpoint, published as a bounded `Choice` (cut 2's own
//!   `live_endpoints`). An actor who holds none gets **no offer at all**, so naming an
//!   endpoint they do not hold is refused identically to naming one that does not exist.
//! * **Who may accept** — any buyer with funds, into an endpoint **they** may take
//!   from. The *seller's* authority was frozen at listing time, in the reservation
//!   itself; the *buyer's* is E4's own `InsufficientAvailable` at `accept_contract`.
//! * **Who may release** — the endpoint's custodian again, which is the same predicate
//!   as stocking: a withdrawal gives the seller their own matter back.
//!
//! # Doc 18 cut IC7: the paid legs
//!
//! `consume` and the fourth op, `draw`, are the **paid** legs: each moves matter a buyer
//! has paid for. Both are offered **only over the actor's own paid contracts** — the
//! offer's server-authenticated fixed parameter `paid` carries, per contract, the terms
//! frozen on the quote at activation (`Quote::lot`), and the prepare requires the
//! submission to name one of those entries and to match it field for field. Before IC7
//! any holder of an inventory endpoint could submit `consume` naming a live hold-backed
//! listing's `(listing, source, commodity)` and take its lot with no contract at all;
//! the paid projection is what closes it (L12). A Draw (`Supply::Stock` on a lot entry)
//! transfers pool quantity from the vendor's storefront into the buyer's endpoint —
//! never a lot — and writes a durable receipt in its own bundle (`market_lot_draws`),
//! which is the purchase contract's proof the goods moved (the `market.lot.drawn` fact)
//! and the epoch-free guard against a second Draw for the same contract.
//!
//! # Doc 18 cut IC10: the fifth op, `deliver`
//!
//! A sale into a standing bid (`Supply::Wanted`) is a quote whose **seller** is the
//! player and whose buyer is the house. Its matter leg, `deliver`, is the third paid leg
//! and the only one offered to the **seller**: over the actor's own endpoint (the
//! quote's frozen source), with the destination (the house's storefront) and the
//! `inventory.transferred` marker id fixed by the server in the paid entry. Its prepare
//! calls inventory's own `InventoryFamilies::transfer_marked`, which moves the pool
//! quantity, carries lineage and writes the marker the sale contract is gated on — so
//! the house's money is paid out only once the goods are in its stores, and the marker
//! is the epoch-free replay guard.

use std::collections::BTreeMap;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, ClockStamp, CommitToken,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext,
    Engine, IdempotencyKey, IdempotencyKeyText, IdempotencyScope, LegalActionQuery, ModError,
    PerceivedRef, PrincipalId as CorePrincipalId, ProjectedParameter, ProjectedParameterKind,
    ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft, Registrar, ReservationId,
    RevisionSet, RulesValue, ScopedIdempotencyKey, SubjectKey,
};
use mush_economy::{ContractId, ContractState, VaultId};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFamilies, LotSale, MAX_GRADE, MAX_LOT_QUANTITY,
    pool_resource,
};
use serde_json::{Value, json};

use crate::ids::{ListingId, QuoteId};
use crate::module::{CapacityPlan, CapacityProvider, CapacityReadError, DrawReceipts};
use crate::record::{DrawReceipt, MAX_OPEN_QUOTES_PER_BUYER, PaidKind, Quote};

/// The one market-owned authoritative action.
pub const ACTION_LOT: &str = "market_lot";
/// The one durable outcome schema it references.
pub const OUTCOME_ID: &str = "market_lot_result";

/// Largest endpoint choice list one lot offer publishes — cut 2's own ceiling, cited
/// rather than redefined, so the two lists can never disagree.
pub const MAX_LOT_ENDPOINT_CHOICES: usize = mush_inventory::MAX_ENDPOINT_CHOICES;

/// The `listing/{id}` holder text, asserted once to be inside K's
/// `MAX_IDEMPOTENCY_KEY_BYTES` (128).
#[must_use]
pub fn listing_holder_text(listing: ListingId) -> String {
    format!("listing/{}", listing.0)
}

/// The **deterministic holder key** every reservation of one lot listing carries, so a
/// later, different request — the accept, the withdrawal, the sweep's probe — re-derives
/// exactly the reservation the stock placed.
///
/// `mush_industry::wo_holder`'s shape exactly: a `PrincipalId::Scoped` under this
/// crate's own `OwnerId`, class `"listing"`, id `listing{id:016x}`, with a deterministic
/// `CanonicalHash` key over `{"listing": id}`. Two consumers of one pattern is what
/// makes it a pattern (10b §3.4, amended).
///
/// # Errors
/// `LOT_IDENTITY` if the key cannot be built.
pub fn listing_holder(listing: ListingId) -> Result<ScopedIdempotencyKey, LotError> {
    let id = IdempotencyKeyText::parse_canonical(format!("listing{:016x}", listing.0))
        .map_err(|_| LOT_IDENTITY)?;
    let principal = CorePrincipalId::Scoped {
        owner: crate::store::owner(),
        class: CanonicalId::new("listing").map_err(|_| LOT_IDENTITY)?,
        id,
    };
    let scope = IdempotencyScope {
        principal,
        owner: crate::store::owner(),
        namespace: CanonicalId::new(crate::store::NS).map_err(|_| LOT_IDENTITY)?,
    };
    let key = CanonicalHash::for_canonical_value(
        "market.listing_holder.v1",
        1,
        &json!({ "listing": listing.0 }),
        1024,
    )
    .map_err(|_| LOT_IDENTITY)?;
    Ok(ScopedIdempotencyKey::new(
        scope,
        0,
        IdempotencyKey::Deterministic(key),
    ))
}

/// The `ReservationId` one lot listing's hold carries, **derived before the submit that
/// places it** (OPEN 2's ordering).
///
/// A command cannot observe its own authoritative submission, so the listing document
/// is written naming an id that is a pure function of the listing, the endpoint and the
/// commodity — `ReservationId::derive(&listing_holder(id), &pool_resource(endpoint,
/// commodity))`, cut 2's own shape. A crash between the document write and the
/// reservation therefore leaves a listing that names a hold nobody placed, which the
/// market sweep's own arm reconciles by withdrawing it and paging **per listing**.
///
/// # Errors
/// [`LotError`] if the holder or the resource cannot be derived.
pub fn listing_reservation(
    listing: ListingId,
    endpoint: EndpointRef,
    commodity: &CanonicalId,
) -> Result<ReservationId, LotError> {
    let holder = listing_holder(listing)?;
    let resource = pool_resource(endpoint, commodity).map_err(|_| LOT_IDENTITY)?;
    ReservationId::derive(&holder, &resource).map_err(|_| LOT_IDENTITY)
}

/// What this file can refuse before it reaches the families.
///
/// One arm, and deliberately no `thiserror` edge: `crate::record::RecordError` is this
/// crate's own shape for a bounded static refusal and this pass adds exactly one crate
/// edge in the whole of it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LotError(&'static str);

/// The one identity refusal.
pub const LOT_IDENTITY: LotError = LotError("a market lot identity cannot be derived");

impl std::fmt::Display for LotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for LotError {}

/// Which of the five the one action is doing.
///
/// One action, five ops, because the bundle each stages is the same shape and because
/// **one** market-owned `Authoritative` action is what §15.5 (c′) prescribes. Doc 18 cut
/// IC7 added `Draw`, with its own authority sentence in `project_offer`: the paid
/// projection. Doc 18 cut IC10 added `Deliver`: offered only to the **seller** of a paid
/// sale into a bid, over the actor's own endpoint, the destination and the marker fixed
/// by the server.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LotOp {
    /// Place the listing's hold, at `@market/vendor stock`.
    Reserve,
    /// Settle it and hand the matter over, at `+market/accept`.
    Consume,
    /// Give it back, at `@market/vendor withdraw`.
    Release,
    /// Doc 18 cut IC7: move a paid Draw's quantity from the vendor's storefront pool
    /// into the buyer's endpoint, at `+market/accept`.
    Draw,
    /// Doc 18 cut IC10: move a paid sale-into-a-bid's quantity from the seller's own
    /// endpoint into the house's storefront pool, writing the `inventory.transferred`
    /// marker its contract is gated on, at `+market/sell <listing>=<qty>`.
    Deliver,
}

impl LotOp {
    /// The parameter word.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            LotOp::Reserve => "reserve",
            LotOp::Consume => "consume",
            LotOp::Release => "release",
            LotOp::Draw => "draw",
            LotOp::Deliver => "deliver",
        }
    }

    fn parse(raw: &str) -> Option<LotOp> {
        match raw {
            "reserve" => Some(LotOp::Reserve),
            "consume" => Some(LotOp::Consume),
            "release" => Some(LotOp::Release),
            "draw" => Some(LotOp::Draw),
            "deliver" => Some(LotOp::Deliver),
            _ => None,
        }
    }

    /// Whether this op moves matter somebody paid for — the three legs offered only over
    /// the actor's own paid contracts and keyed by the contract (doc 18 cut IC7, L5; cut
    /// IC10 added `Deliver`).
    #[must_use]
    pub const fn is_paid(self) -> bool {
        matches!(self, LotOp::Consume | LotOp::Draw | LotOp::Deliver)
    }
}

/// The one market-owned action's provider.
pub(crate) struct MarketLotProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: InventoryFamilies,
    /// Doc 18 cut IC7: the draw-receipt family the Draw writes and guards on.
    draws: DrawReceipts,
    /// Doc 18 cut IC7: the capacity hook, asked again inside the Draw's own bundle.
    capacity: Option<std::sync::Arc<dyn CapacityProvider>>,
}

fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
    ActionError::new(code, false, message).expect("bounded market lot action error")
}

/// **One refusal for every reason a lot leg can fail** (§4 rule 4).
///
/// A buyer who names an endpoint they do not hold, a listing whose hold has vanished
/// and a listing that never existed all read the same line, so the prepared path is not
/// an existence oracle for another principal's inventory.
pub const LOT_REFUSED: &str = "That stock cannot be moved right now.";

fn refuse(_source: &InventoryError) -> ActionError {
    error(ActionErrorCode::ProviderRejected, LOT_REFUSED)
}

/// The one refusal with no inventory error behind it (a paid-entry mismatch, a replay,
/// a capacity refusal) — the same pinned line.
fn rejected() -> ActionError {
    error(ActionErrorCode::ProviderRejected, LOT_REFUSED)
}

fn parameter(
    id: &str,
    required: bool,
    kind: ProjectedParameterKind,
) -> Result<ProjectedParameter, ActionError> {
    Ok(ProjectedParameter {
        id: CanonicalId::new(id)
            .map_err(|_| error(ActionErrorCode::Integrity, "market lot parameter id"))?,
        required,
        kind,
    })
}

fn text_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))
}

fn u64_field(parameters: &serde_json::Map<String, Value>, key: &str) -> Result<u64, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))
}

fn endpoint_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<EndpointRef, ActionError> {
    text_field(parameters, key)?
        .parse::<u64>()
        .ok()
        .map(|id| EndpointRef(VaultId(id)))
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))
}

fn commodity_field(
    parameters: &serde_json::Map<String, Value>,
) -> Result<CanonicalId, ActionError> {
    let raw = text_field(parameters, "commodity")?;
    if raw.len() > mush_inventory::MAX_ENDPOINT_CHOICES.saturating_mul(4) {
        return Err(error(ActionErrorCode::InvalidRequest, LOT_REFUSED));
    }
    CanonicalId::new(&raw).map_err(|_| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))
}

/// The IC stamp the preparation uses, off the offer's own FIXED parameters — never a
/// wall clock, so two replays of one request stamp the identical deadline.
fn prepare_now(parameters: &serde_json::Map<String, Value>) -> Result<ClockStamp, ActionError> {
    parameters
        .get("now")
        .and_then(|value| serde_json::from_value::<ClockStamp>(value.clone()).ok())
        .ok_or_else(|| {
            error(
                ActionErrorCode::Unavailable,
                "the world clock is unavailable",
            )
        })
}

// ---- doc 18 cut IC7: the paid-leg projection --------------------------------

/// The most paid entries of each kind one offer carries — the per-buyer open-quote cap,
/// cited rather than redefined: a draw entry is an open quote, so the cap already bounds
/// it, and the consume entries take the same ceiling.
pub const MAX_PAID_ENTRIES_PER_KIND: usize = MAX_OPEN_QUOTES_PER_BUYER;

/// The paid legs one actor may fire right now, keyed by contract id in decimal.
struct PaidLegs {
    entries: serde_json::Map<String, Value>,
    consume: bool,
    draw: bool,
    /// Doc 18 cut IC10: the actor sells into a bid and owes its hand-over.
    deliver: bool,
}

/// One paid entry's JSON: the terms frozen on the quote, plus the capacity plan.
fn paid_entry(q: &Quote, lot: &crate::record::PaidLot, op: LotOp, plan: Value) -> Value {
    json!({
        "op": op.word(),
        "quote": q.id.0,
        "listing": q.listing.0,
        "source": lot.source.vault().0,
        "destination": lot.destination.vault().0,
        "commodity": lot.commodity.0.as_str(),
        "quantity": q.quantity,
        "grade": lot.grade,
        "plan": plan,
    })
}

/// **Build the actor's paid legs** (doc 18 cut IC7, L4/L12; the seller-side source
/// projection, built once).
///
/// The walk is the buyer's own quote index, which holds trade-tape rows too (F7), so the
/// visit bound is the buyer's retained quote rows — at most `MAX_QUOTES`; what reaches
/// the offer is bounded by [`MAX_PAID_ENTRIES_PER_KIND`] per kind.
///
/// - a **draw entry** is an open, activated Draw quote whose goods have not been seen to
///   move (`drawn_at` is `None`), whose contract is `Active`, whose cutoff `draw_by` has
///   not passed on the real clock (F1 — the driver projects immediately before it
///   prepares, so a Draw past its cutoff is never prepared), and whose destination the
///   actor still holds. It carries the capacity plan for that destination (JSON `null`
///   with no provider or an ungoverned endpoint; a plan over `MAX_CAPACITY_PLAN_BYTES`
///   omits the entry, fail closed);
/// - a **consume entry** is an activated hold-backed quote whose contract is `Active`,
///   `ReadyToSettle` or `Settled` (a `Before`-only hold contract can settle before its
///   leg lands; the reservation's own held state, checked in the prepare, is the replay
///   guard), destination held; unsettled first, then settled newest-first; its plan is
///   always `null` (a hold-backed sale never reaches a governed endpoint, lead 2e);
/// - a **deliver entry** (doc 18 cut IC10) is walked over the actor's quotes as
///   **seller**: an activated, unsettled sale into a bid whose contract is `Active`,
///   whose cutoff `draw_by` has not passed on the real clock, and whose source the actor
///   still holds. It carries the `marker` its contract is gated on, server-authenticated
///   — the prepare reads the marker from the entry, never from the submission.
fn paid_legs(
    world: &mush_core::World,
    actor: mush_core::ObjId,
    endpoints: &[VaultId],
    capacity: Option<&std::sync::Arc<dyn CapacityProvider>>,
) -> PaidLegs {
    let mut out = PaidLegs {
        entries: serde_json::Map::new(),
        consume: false,
        draw: false,
        deliver: false,
    };
    let (Some(market), Some(economy)) = (
        world.module_state::<crate::store::MarketStore>(),
        world.module_state::<mush_economy::EconomyStore>(),
    ) else {
        return out;
    };
    let now = world.now();
    let buyer = mush_org::PrincipalId::Object(actor);
    let state_of = |contract: ContractId| economy.snapshot().contract(contract).map(|c| c.state);
    let mut draws = 0usize;
    let mut consumes: Vec<(bool, QuoteId, ContractId, Value)> = Vec::new();
    for q in market.quotes_of_buyer(&buyer) {
        let (Some(lot), Some(contract)) = (&q.lot, q.contract) else {
            continue;
        };
        if !q.activated || !endpoints.contains(&lot.destination.vault()) {
            continue;
        }
        match lot.kind {
            PaidKind::Draw => {
                if draws >= MAX_PAID_ENTRIES_PER_KIND
                    || q.is_trade()
                    || q.drawn_at.is_some()
                    || !matches!(state_of(contract), Some(ContractState::Active))
                    || lot.draw_by.is_none_or(|by| now > by)
                {
                    continue;
                }
                let plan = match capacity.and_then(|c| c.plan(world, lot.destination.0)) {
                    None => Value::Null,
                    Some(plan) if plan.within_bounds() => plan.0,
                    // An oversized plan cannot ride the offer: omit the entry rather
                    // than draw with no rule.
                    Some(_) => continue,
                };
                out.entries.insert(
                    contract.0.to_string(),
                    paid_entry(q, lot, LotOp::Draw, plan),
                );
                out.draw = true;
                draws = draws.saturating_add(1);
            }
            PaidKind::Hold => {
                let settled = match state_of(contract) {
                    Some(ContractState::Active | ContractState::ReadyToSettle) => false,
                    Some(ContractState::Settled { .. }) => true,
                    _ => continue,
                };
                consumes.push((
                    settled,
                    q.id,
                    contract,
                    paid_entry(q, lot, LotOp::Consume, Value::Null),
                ));
            }
            // Doc 18 cut IC10: a sale into a bid's buyer is the house, never a player,
            // and its leg is the seller's — walked below.
            PaidKind::Deliver => {}
        }
    }
    // ---- doc 18 cut IC10: the seller's deliver entries ----
    let seller = mush_org::PrincipalId::Object(actor);
    let mut delivers = 0usize;
    for q in market.quotes_of_seller(&seller) {
        if delivers >= MAX_PAID_ENTRIES_PER_KIND {
            break;
        }
        let (Some(lot), Some(contract)) = (&q.lot, q.contract) else {
            continue;
        };
        let Some(marker) = lot.marker.as_ref() else {
            continue;
        };
        if lot.kind != PaidKind::Deliver
            || !q.activated
            || q.is_trade()
            || !endpoints.contains(&lot.source.vault())
            || !matches!(state_of(contract), Some(ContractState::Active))
            || lot.draw_by.is_none_or(|by| now > by)
        {
            continue;
        }
        let mut entry = paid_entry(q, lot, LotOp::Deliver, Value::Null);
        if let Some(map) = entry.as_object_mut() {
            map.insert("marker".to_owned(), json!(marker.as_str()));
        }
        out.entries.insert(contract.0.to_string(), entry);
        out.deliver = true;
        delivers = delivers.saturating_add(1);
    }
    // Unsettled first (in quote order), then settled newest-first.
    consumes.sort_by(|a, b| match (a.0, b.0) {
        (false, false) => a.1.cmp(&b.1),
        (true, true) => b.1.cmp(&a.1),
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
    });
    for (_, _, contract, entry) in consumes.into_iter().take(MAX_PAID_ENTRIES_PER_KIND) {
        out.entries.insert(contract.0.to_string(), entry);
        out.consume = true;
    }
    out
}

/// The paid entry a submission names, if the offer carried it with the right op.
fn paid_entry_of(
    parameters: &serde_json::Map<String, Value>,
    op: LotOp,
) -> Result<(ContractId, &serde_json::Map<String, Value>), ActionError> {
    let raw = text_field(parameters, "contract")?;
    let contract = raw
        .parse::<u64>()
        .ok()
        .filter(|id| id.to_string() == raw)
        .map(ContractId)
        .ok_or_else(rejected)?;
    let entry = parameters
        .get("paid")
        .and_then(Value::as_object)
        .and_then(|paid| paid.get(&raw))
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    if entry.get("op").and_then(Value::as_str) != Some(op.word()) {
        return Err(rejected());
    }
    Ok((contract, entry))
}

/// Whether the submitted leg matches the paid entry field for field: the fixed terms
/// are the truth, and the submitted fields are only a cross-check.
fn matches_paid(
    entry: &serde_json::Map<String, Value>,
    listing: ListingId,
    destination: EndpointRef,
    source: EndpointRef,
    commodity: &CanonicalId,
    quantity: u64,
    grade: Option<u8>,
) -> bool {
    let u64_of = |key: &str| entry.get(key).and_then(Value::as_u64);
    u64_of("listing") == Some(listing.0)
        && u64_of("destination") == Some(destination.vault().0)
        && u64_of("source") == Some(source.vault().0)
        && entry.get("commodity").and_then(Value::as_str) == Some(commodity.as_str())
        && u64_of("quantity") == Some(quantity)
        && grade.is_none_or(|g| u64_of("grade") == Some(u64::from(g)))
}

impl MarketLotProvider {
    fn parameters<'i>(
        &self,
        invocation: &'i ActionInvocation,
    ) -> Result<&'i serde_json::Map<String, Value>, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(error(ActionErrorCode::InvalidRequest, LOT_REFUSED));
        }
        invocation
            .parameters()
            .as_object()
            .ok_or_else(|| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))
    }

    fn op(&self, parameters: &serde_json::Map<String, Value>) -> Result<LotOp, ActionError> {
        LotOp::parse(&text_field(parameters, "op")?)
            .ok_or_else(|| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))
    }
}

impl ActionProvider for MarketLotProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = self.parameters(invocation)?;
        self.op(parameters).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        // **The whole authority of the lane, in one list** (the seam that has
        // `&Engine`). An actor holds an endpoint when `mush_economy::may_take_from_vault`
        // says so — E3's own vault-withdraw authority, asked through cut 2's exported
        // `live_endpoints` and rebuilt nowhere. An actor holding none gets no offer at
        // all, so a stranger naming a vendor's endpoint is refused identically to one
        // naming an endpoint that has never existed.
        let endpoints = mush_inventory::live_endpoints(&engine.world, query.actor());
        if endpoints.is_empty() {
            return Ok(None);
        }
        let choices: Vec<String> = endpoints.iter().map(|id| id.0.to_string()).collect();
        // ---- doc 18 cut IC7: the paid legs' authority sentence (L4, L12) ----
        //
        // `consume` and `draw` are offered only over the actor's OWN paid contracts:
        // the entries carry the terms frozen on the quote at activation, in the
        // server-authenticated fixed parameters, and the prepare requires a match. An
        // actor with no paid entry is offered `reserve` and `release` only, and no
        // `contract` parameter at all.
        let paid = paid_legs(
            &engine.world,
            query.actor(),
            &endpoints,
            self.capacity.as_ref(),
        );
        let mut ops = vec![LotOp::Reserve.word().to_owned()];
        if paid.consume {
            ops.push(LotOp::Consume.word().to_owned());
        }
        ops.push(LotOp::Release.word().to_owned());
        if paid.draw {
            ops.push(LotOp::Draw.word().to_owned());
        }
        if paid.deliver {
            ops.push(LotOp::Deliver.word().to_owned());
        }
        let contracts: Vec<String> = paid.entries.keys().cloned().collect();
        let mut parameters = vec![
            parameter("op", true, ProjectedParameterKind::Choice { choices: ops })?,
            // `reserve` and `release` act on the SELLER's endpoint and the paid legs on
            // the BUYER's, and in every case it is the acting principal's own — which is
            // why one `Choice` list serves all four and why the offer's regeneration is
            // the gate rather than a stored consent.
            parameter(
                "endpoint",
                true,
                ProjectedParameterKind::Choice {
                    choices: choices.clone(),
                },
            )?,
            // The seller's endpoint on a paid leg. It is NOT the authority (doc 18 cut
            // IC7): the paid entry in the fixed parameters names the source the money
            // was taken for, and this is a cross-check against it. On a consume the
            // reservation id is still derived from `(listing, source, commodity)`, so a
            // source the listing did not use could not settle anything either.
            parameter(
                "source",
                false,
                ProjectedParameterKind::Text { maximum_bytes: 32 },
            )?,
            // Doc 18 cut IC10: the receiving endpoint of a `deliver` (the house's
            // storefront), a cross-check against the paid entry's fixed destination.
            parameter(
                "destination",
                false,
                ProjectedParameterKind::Text { maximum_bytes: 32 },
            )?,
            parameter(
                "commodity",
                true,
                ProjectedParameterKind::Text { maximum_bytes: 64 },
            )?,
            // **Listing ids are allocated from zero** (`MarketStore::alloc_listing` on a
            // fresh world), and this floor was one (doc 18 cut IC0): the very first lot
            // listing a world ever published queued its reserve and the driver refused
            // it at resolution with `InvalidParameters`, so the listing advertised a
            // hold nobody placed until the sweep withdrew it and paged. The IC0 gate
            // (`tests/ic0.rs`) is what found it.
            parameter(
                "listing",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: i64::MAX,
                },
            )?,
            parameter(
                "quantity",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::try_from(MAX_LOT_QUANTITY).unwrap_or(i64::MAX),
                },
            )?,
            parameter(
                "grade",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: i64::from(MAX_GRADE),
                },
            )?,
        ];
        // The paid contract a consume or draw names, as a decimal string — present only
        // when the actor has a paid entry, so a supplied `contract` from anyone else is
        // `InvalidParameters` at resolution.
        if !contracts.is_empty() {
            parameters.push(parameter(
                "contract",
                false,
                ProjectedParameterKind::Choice { choices: contracts },
            )?);
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            error(
                ActionErrorCode::Integrity,
                "market definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "market guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "market definition guard is unavailable",
                )
            })?;
        // The fixed parameters' size, stated (doc 18 cut IC7; cut IC10 added a third
        // kind): at most eight draw entries of ≤ 2,048 plan bytes plus ≤ 512 bytes of
        // terms, eight consume entries of ≤ 512 bytes and eight deliver entries of
        // ≤ 512 bytes plus an 80-byte marker — 8 × 2,560 + 8 × 512 + 8 × 600 ≈ 30 KiB,
        // still well under the 64 KiB `MAX_ACTION_PARAMETER_BYTES` bound.
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({
                "now": engine.world.world_ic_now(),
                "paid": Value::Object(paid.entries),
            }),
            parameters: ProjectedParameterSchema::new(parameters).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "market lot parameter schema is invalid",
                )
            })?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "market-lot",
                "Move listed stock",
                "Reserve, hand over or give back the stock behind one lot listing.",
            )
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "market lot presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "market lot offer guards are invalid",
                )
            })?,
        }))
    }

    // **No `project_receipt` override, deliberately.** The trait's default answers
    // `Ok(None)`, which yields the kernel's generic committed receipt — and the typed
    // override would need `mush_proto::ProjectedActionResult`, i.e. a SECOND crate
    // edge, in a pass whose whole edge budget is `mush-market -> mush-inventory`. The
    // acting principal already learns the outcome from the command's own line, and the
    // one thing a typed receipt would add — the buyer's new `LotId` — is a fact about
    // another owner's record that `@inv/show` is the authority for.

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = self.parameters(invocation)?.clone();
        let op = self.op(&parameters)?;
        let now = prepare_now(&parameters)?;
        let actor = invocation.actor();
        let listing = ListingId(u64_field(&parameters, "listing")?);
        let commodity = commodity_field(&parameters)?;
        let quantity = u64_field(&parameters, "quantity")?;
        let mut data = BTreeMap::<String, Value>::new();
        data.insert("actor".to_owned(), json!(actor.0));
        data.insert("verb".to_owned(), json!(ACTION_LOT));
        data.insert("op".to_owned(), json!(op.word()));
        data.insert("listing".to_owned(), json!(listing.0));

        let summary = match op {
            LotOp::Reserve => {
                let endpoint = endpoint_field(&parameters, "endpoint")?;
                let holder = listing_holder(listing)
                    .map_err(|_| error(ActionErrorCode::Integrity, LOT_REFUSED))?;
                // **A lot listing is backed by a lineage root, or it is not published
                // at all.** `settle_sale` draws the sold quantity from the endpoint's
                // tranche, so a pool credited from somewhere that established no root
                // would reserve here and refuse at the sale — after the money leg is in
                // force. It is refused now instead, which is the fail-closed direction.
                if self
                    .families
                    .tranche_for(txn, endpoint, &commodity)
                    .map_err(|source| refuse(&source))?
                    .is_none()
                {
                    return Err(error(ActionErrorCode::ProviderRejected, LOT_REFUSED));
                }
                let id = self
                    .families
                    .reserve(txn, endpoint, &commodity, &holder, quantity, &now)
                    .map_err(|source| refuse(&source))?;
                data.insert("reservation".to_owned(), json!(id.0.to_hex()));
                data.insert("endpoint".to_owned(), json!(endpoint.vault().0));
                data.insert("quantity".to_owned(), json!(quantity));
                format!("Reserved {quantity} {commodity} for listing #{listing}.")
            }
            LotOp::Consume => {
                let destination = endpoint_field(&parameters, "endpoint")?;
                let source = endpoint_field(&parameters, "source")?;
                let raw_grade =
                    u8::try_from(parameters.get("grade").and_then(Value::as_u64).unwrap_or(0))
                        .map_err(|_| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))?;
                let grade = Grade::new(raw_grade)
                    .map_err(|_| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))?;
                // ---- doc 18 cut IC7 (L12): only the buyer of a paid contract ----
                //
                // The submission must name a consume entry of the fixed `paid` map and
                // match it field for field. Everything below is IC0's code unchanged.
                let (contract, entry) = paid_entry_of(&parameters, LotOp::Consume)?;
                if !matches_paid(
                    entry,
                    listing,
                    destination,
                    source,
                    &commodity,
                    quantity,
                    Some(raw_grade),
                ) {
                    return Err(rejected());
                }
                data.insert("contract".to_owned(), json!(contract.0));
                let reservation = listing_reservation(listing, source, &commodity)
                    .map_err(|_| error(ActionErrorCode::Integrity, LOT_REFUSED))?;
                let record = self
                    .families
                    .reservation(txn, source, &commodity, reservation)
                    .map_err(|source| refuse(&source))?
                    .ok_or_else(|| error(ActionErrorCode::ProviderRejected, LOT_REFUSED))?;
                // Both lot ids are DERIVED from this request, never allocated: a replay
                // presents the identical ids and `settle_sale`'s own `AlreadyExists`
                // check is then the replay guard, so a retried accept can never mint a
                // second parcel.
                let buyer_lot = mush_inventory::derive_lot_id(invocation.request(), 0)
                    .map_err(|source| refuse(&source))?;
                let remainder_lot = mush_inventory::derive_lot_id(invocation.request(), 1)
                    .map_err(|source| refuse(&source))?;
                let commit = CommitToken::from_bytes(*invocation.request().as_bytes());
                let settled = self
                    .families
                    .settle_sale(
                        txn,
                        source,
                        &commodity,
                        reservation,
                        record.revision(),
                        &now,
                        commit,
                        LotSale {
                            destination,
                            sold: quantity,
                            grade,
                            buyer_lot,
                            remainder_lot,
                        },
                    )
                    .map_err(|source| refuse(&source))?;
                data.insert("endpoint".to_owned(), json!(destination.vault().0));
                data.insert("source".to_owned(), json!(source.vault().0));
                data.insert("lot".to_owned(), json!(settled.buyer.id().0));
                data.insert("quantity".to_owned(), json!(quantity));
                data.insert("balance".to_owned(), json!(settled.balance));
                if let Some(remainder) = &settled.remainder {
                    data.insert("remainder".to_owned(), json!(remainder.id().0));
                }
                format!(
                    "Handed over {quantity} {commodity} as {}.",
                    settled.buyer.id()
                )
            }
            LotOp::Release => {
                let endpoint = endpoint_field(&parameters, "endpoint")?;
                let reservation = listing_reservation(listing, endpoint, &commodity)
                    .map_err(|_| error(ActionErrorCode::Integrity, LOT_REFUSED))?;
                let record = self
                    .families
                    .reservation(txn, endpoint, &commodity, reservation)
                    .map_err(|source| refuse(&source))?
                    .ok_or_else(|| error(ActionErrorCode::ProviderRejected, LOT_REFUSED))?;
                // A second release finds a terminal record and is refused by
                // `require_held`, so a retyped withdrawal writes nothing rather than
                // giving the quantity back twice.
                self.families
                    .release(txn, endpoint, &commodity, reservation, record.revision())
                    .map_err(|source| refuse(&source))?;
                data.insert("endpoint".to_owned(), json!(endpoint.vault().0));
                format!("Released listing #{listing}'s hold.")
            }
            LotOp::Draw => self.prepare_draw(
                invocation,
                txn,
                &parameters,
                &now,
                listing,
                &commodity,
                quantity,
                &mut data,
            )?,
            LotOp::Deliver => self.prepare_deliver(
                invocation,
                txn,
                &parameters,
                &now,
                listing,
                &commodity,
                quantity,
                &mut data,
            )?,
        };
        data.insert("summary".to_owned(), json!(summary));
        let outcome = RulesValue::new(self.outcome.clone(), json!(data)).map_err(|_| {
            error(
                ActionErrorCode::BoundsExceeded,
                "market lot outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl MarketLotProvider {
    /// **The Draw prepare** (doc 18 cut IC7 item 6). Every refusal is the pinned
    /// `LOT_REFUSED` line, and an `ActionTxn` discards its whole mutation set on any
    /// error, so a refused Draw moves nothing and writes no receipt — the contract then
    /// never sees its fact and expires with a refund at its window.
    #[allow(clippy::too_many_arguments)]
    fn prepare_draw(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        listing: ListingId,
        commodity: &CanonicalId,
        quantity: u64,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let destination = endpoint_field(parameters, "endpoint")?;
        let source = endpoint_field(parameters, "source")?;
        // 1–2. A draw entry of the fixed `paid` map (one past its cutoff was never
        // projected), and the submitted terms equal to it.
        let (contract, entry) = paid_entry_of(parameters, LotOp::Draw)?;
        if !matches_paid(
            entry,
            listing,
            destination,
            source,
            commodity,
            quantity,
            None,
        ) {
            return Err(rejected());
        }
        let quote = entry
            .get("quote")
            .and_then(Value::as_u64)
            .map(QuoteId)
            .ok_or_else(rejected)?;
        // 3. The epoch-free replay guard across a reboot: one receipt per contract,
        // ever (the inventory applied marker's rule).
        if txn
            .get(&self.draws, &contract)
            .map_err(|_| rejected())?
            .is_some()
        {
            return Err(rejected());
        }
        // 4. The capacity backstop, against the plan resolved where a World existed.
        let plan = entry.get("plan").cloned().unwrap_or(Value::Null);
        if !plan.is_null() {
            let Some(capacity) = &self.capacity else {
                return Err(rejected());
            };
            let plan = CapacityPlan(plan);
            if !plan.within_bounds() {
                return Err(rejected());
            }
            let families = &self.families;
            let mut read_pool = |c: &CanonicalId| -> Result<u64, CapacityReadError> {
                match families.pool(txn, destination, c) {
                    Ok(Some(pool)) => Ok(pool.quantity),
                    Ok(None) => Ok(0),
                    Err(_) => Err(CapacityReadError),
                }
            };
            if capacity
                .check(&plan, commodity, quantity, &mut read_pool)
                .is_err()
            {
                return Err(rejected());
            }
        }
        // 5. The move itself: pool quantity, storefront → buyer — no lot, no
        // `lot_census` entry, which is what every hull delivery is (§4.2).
        self.families
            .transfer(txn, source, destination, commodity, quantity)
            .map_err(|source| refuse(&source))?;
        // 6. The durable receipt, in the same bundle as the move.
        let receipt = DrawReceipt {
            contract,
            quote,
            listing,
            source: source.vault(),
            destination: destination.vault(),
            commodity: smol_str::SmolStr::new(commodity.as_str()),
            quantity,
            actor: invocation.actor(),
            at_ic: now.at,
        };
        txn.insert(&self.draws, &contract, receipt)
            .map_err(|_| rejected())?;
        // 7. The outcome.
        data.insert("contract".to_owned(), json!(contract.0));
        data.insert("quote".to_owned(), json!(quote.0));
        data.insert("source".to_owned(), json!(source.vault().0));
        data.insert("endpoint".to_owned(), json!(destination.vault().0));
        data.insert("quantity".to_owned(), json!(quantity));
        Ok(format!(
            "Drew {quantity} {commodity} for contract #{contract}."
        ))
    }

    /// **The Deliver prepare** (doc 18 §4.3 "Bids that take real lots", D-18-26; cut
    /// IC10). The seller's own endpoint is `endpoint`, which must equal the submitted
    /// `source` and the paid entry's; the destination and the marker are the entry's,
    /// never the submission's. The move is inventory's own `transfer_marked` — pool
    /// quantity, lineage carried, the `inventory.transferred` marker written under the
    /// id the sale contract names — and an existing marker refuses (the replay guard).
    /// Every refusal is the pinned `LOT_REFUSED` line. No capacity check: the bid's
    /// vault is ungoverned.
    #[allow(clippy::too_many_arguments)]
    fn prepare_deliver(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        listing: ListingId,
        commodity: &CanonicalId,
        quantity: u64,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let endpoint = endpoint_field(parameters, "endpoint")?;
        let source = endpoint_field(parameters, "source")?;
        let destination = endpoint_field(parameters, "destination")?;
        if source != endpoint {
            return Err(rejected());
        }
        let (contract, entry) = paid_entry_of(parameters, LotOp::Deliver)?;
        if !matches_paid(
            entry,
            listing,
            destination,
            endpoint,
            commodity,
            quantity,
            None,
        ) {
            return Err(rejected());
        }
        let quote = entry
            .get("quote")
            .and_then(Value::as_u64)
            .map(QuoteId)
            .ok_or_else(rejected)?;
        let marker = entry
            .get("marker")
            .and_then(Value::as_str)
            .filter(|marker| crate::record::is_marker_id(marker))
            .map(smol_str::SmolStr::new)
            .ok_or_else(rejected)?;
        self.families
            .transfer_marked(
                txn,
                endpoint,
                destination,
                commodity,
                quantity,
                &marker,
                invocation.actor(),
                now,
            )
            .map_err(|source| refuse(&source))?;
        data.insert("contract".to_owned(), json!(contract.0));
        data.insert("quote".to_owned(), json!(quote.0));
        data.insert("source".to_owned(), json!(source.vault().0));
        data.insert("endpoint".to_owned(), json!(endpoint.vault().0));
        data.insert("destination".to_owned(), json!(destination.vault().0));
        data.insert("quantity".to_owned(), json!(quantity));
        data.insert("marker".to_owned(), json!(marker.as_str()));
        Ok(format!(
            "Delivered {quantity} {commodity} for contract #{contract}."
        ))
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(|error| ModError::msg(error.to_string()))
}

fn exact_validator(
    descriptors: &[DefinitionDescriptor],
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    let expected: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| {
            let mut references = descriptor.references().to_vec();
            references.sort();
            (
                (descriptor.id().clone(), descriptor.version()),
                (
                    descriptor.payload().clone(),
                    references,
                    descriptor.is_current(),
                ),
            )
        })
        .collect();
    move |context| {
        let definition = context.definition();
        let key = (
            definition.reference().key().id.clone(),
            definition.reference().version(),
        );
        let Some((payload, references, current)) = expected.get(&key) else {
            return Err("unknown market definition".to_owned());
        };
        if definition.payload() != payload
            || definition.references() != references
            || definition.is_current() != *current
        {
            return Err("market definition shape mismatch".to_owned());
        }
        Ok(())
    }
}

fn state_canonical(name: &str) -> Result<CanonicalId, ModError> {
    CanonicalId::new(name).map_err(|error| ModError::msg(error.to_string()))
}

/// **Register the draw-receipt family** (doc 18 cut IC7 item 6), on the
/// `inventory_applied` model (`mush-inventory/src/module.rs`): one market-owned typed
/// record per contract, written by the Draw in its own bundle.
///
/// Called from `MarketModule::register` after `reg.classify_commands(..)` and before
/// `reg.state(..)`, and only when a lot supply is composed.
///
/// # Errors
/// [`ModError`] if the family is refused.
pub(crate) fn register_draws(reg: &mut Registrar<'_>) -> Result<DrawReceipts, ModError> {
    reg.state_family::<ContractId, DrawReceipt, _, _, _, _>(
        mush_core::StateFamilyDescriptor::new(
            mush_core::StateFamilyId::new("market_lot_draws")
                .map_err(|error| ModError::msg(error.to_string()))?,
            state_canonical("market.lot_draw_id.v1")?,
            state_canonical("market.lot_draw.v1")?,
            1,
            state_canonical("market.lot_draw.decode_json.v1")?,
            state_canonical("market.lot_draw.invariants.v1")?,
            state_canonical("market.lot_draw.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ContractId| Ok(format!("{:016x}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        DrawReceipt::check,
        |value: &DrawReceipt| serde_json::to_value(value).map_err(|e| e.to_string()),
    )
}

/// Register the outcome, the one action and its provider.
///
/// Called from `MarketModule::register` **after** `reg.classify_commands(..)`, and only
/// when the composition wired [`MarketModule::with_inventory`](crate::MarketModule): a
/// server with no lot supply registers no action at all and refuses a
/// `Representation::Lot` listing byte-identically to E6c.
///
/// # Errors
/// [`ModError`] if any definition or provider registration is refused.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: &InventoryFamilies,
    draws: DrawReceipts,
    capacity: Option<std::sync::Arc<dyn CapacityProvider>>,
) -> Result<(), ModError> {
    let outcome_descriptor = descriptor(
        "outcome",
        OUTCOME_ID,
        json!({ "fields": ["actor", "verb", "summary"] }),
    )?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(std::slice::from_ref(&outcome_descriptor)),
    )?;
    let outcome = reg.definition(outcome_descriptor)?;
    let action_descriptor = descriptor(
        "action",
        ACTION_LOT,
        json!({ "subject": "self_actor", "targets": 0, "family": "inventory.pool" }),
    )?
    .with_references(vec![outcome.clone()]);
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(std::slice::from_ref(&action_descriptor)),
    )?;
    let action = reg.definition(action_descriptor)?;
    reg.action_provider(
        action.clone(),
        MarketLotProvider {
            action,
            outcome,
            families: families.clone(),
            draws,
            capacity,
        },
    )?;
    Ok(())
}

/// What one command asks the lot action to do.
#[derive(Clone, Debug)]
pub struct LotLeg {
    /// Which of the five.
    pub op: LotOp,
    /// The listing the reservation is scoped to.
    pub listing: ListingId,
    /// The endpoint the ACTING principal owns — the seller's on a reserve, a release
    /// or a deliver, the buyer's on a consume or a draw.
    pub endpoint: EndpointRef,
    /// The seller's endpoint on a consume, and the same as `endpoint` otherwise.
    pub source: EndpointRef,
    /// The receiving endpoint of a `deliver` (doc 18 cut IC10); `None` otherwise.
    pub destination: Option<EndpointRef>,
    /// The commodity's pool at the source endpoint.
    pub commodity: CanonicalId,
    /// How much.
    pub quantity: u64,
    /// The grade the listing advertises.
    pub grade: u8,
    /// The paid contract a `consume`, `draw` or `deliver` delivers against (doc 18 cuts
    /// IC7 and IC10), and `None` on a `reserve` or `release`.
    pub contract: Option<ContractId>,
}

/// **Stage the ONE `Ctx::submit_authoritative` a lot leg costs its command** (cut 2's
/// fold C4: exactly one per command).
///
/// Two key forms (doc 18 §4.4, D-18-32, L5):
///
/// - **a paid leg** (`consume`, `draw`, `deliver`) is one request per contract,
///   `market.lot.{op}.{contract:x}`, and writes `"contract"` (decimal) into its
///   parameters. The driver's request uid is a digest of the actor, action, subject,
///   targets, parameters and key, so an identical re-stage is the same request: after a
///   landing it is a replay **HIT** that prepares nothing, and after a refused prepare
///   it is prepared again, because a refusal finalizes nothing;
/// - **a listing leg** (`reserve`, `release`) keeps `market.lot.{op}.{listing:x}.{s:x}`
///   with the IC second, so a retyped command inside one second is a replay HIT while a
///   deliberate retry in a later second is a new request.
///
/// The handler performs no mutation of its own and emits no success line: the mechanic
/// is the provider's and the confirmation is the receipt's, held behind the durable ack.
///
/// Returns `false` when this server composed no lot supply (or a paid leg names no
/// contract), so the caller can refuse rather than pretending something was staged.
pub(crate) fn stage(ctx: &mut mush_core::Ctx<'_>, leg: &LotLeg) -> bool {
    let Some(action) = action_ref(ctx.reg) else {
        return false;
    };
    let mut parameters = json!({
        "op": leg.op.word(),
        "listing": leg.listing.0,
        "endpoint": leg.endpoint.vault().0.to_string(),
        "source": leg.source.vault().0.to_string(),
        "commodity": leg.commodity.as_str(),
        "quantity": leg.quantity,
        "grade": leg.grade,
    });
    // Doc 18 cut IC10: only a `deliver` names a destination, so every other leg's
    // parameters stay byte-identical to what they were before IC10.
    if let (Some(destination), Some(map)) = (leg.destination, parameters.as_object_mut()) {
        map.insert(
            "destination".to_owned(),
            json!(destination.vault().0.to_string()),
        );
    }
    let discriminator = if leg.op.is_paid() {
        let Some(contract) = leg.contract else {
            return false;
        };
        if let Some(map) = parameters.as_object_mut() {
            map.insert("contract".to_owned(), json!(contract.0.to_string()));
        }
        format!("market.lot.{}.{:x}", leg.op.word(), contract.0)
    } else {
        let at = ctx.w.world_ic_now().at;
        format!(
            "market.lot.{}.{:x}.{:x}",
            leg.op.word(),
            leg.listing.0,
            at as u64
        )
    };
    let Ok(key) = IdempotencyKeyText::parse_canonical(discriminator) else {
        return false;
    };
    ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    true
}

/// **The `market.lot.drawn` fact id** for one Draw sale (doc 18 cut IC7):
/// `"{quote}/{accepted_at}"`, where `accepted_at` is the real `World::now()` second the
/// deal came into force. [`crate::MarketFactProvider`] parses it back.
#[must_use]
pub fn draw_fact_id(quote: QuoteId, accepted_at: i64) -> String {
    format!("{}/{accepted_at}", quote.0)
}

/// **Read one contract's draw receipt** (doc 18 cut IC7) through the market's own
/// family handle and the context's owner-scoped `Ctx::action_state`.
///
/// It answers only in a context the market owns — its own sweep tick and its own boot
/// hooks, which are the only callers. The settlement ear runs on whichever module's
/// context emitted the contract transition (the economy's sweep, for an expiry), where
/// the owner scope refuses the market's family; that is why `on_contract_changed` never
/// decides an undrawn Draw's release itself and leaves it to the mirror arm on the
/// market's own sweep (`boot::reconcile_draws`). `Err(())` when there is no lot supply
/// or the read is refused — every caller fails closed.
pub(crate) fn draw_receipt(
    ctx: &mush_core::Ctx<'_>,
    contract: ContractId,
) -> Result<Option<DrawReceipt>, ()> {
    let draws = crate::module::draw_receipts(ctx.w).ok_or(())?;
    ctx.action_state(&draws, &contract)
        .map(|found| found.cloned())
        .map_err(|_| ())
}

/// The registered `DefinitionRef` for the lot action, rebuilt from the frozen registry
/// so a command handler can name it without holding a handle.
#[must_use]
pub fn action_ref(registry: &mush_core::Registry) -> Option<DefinitionRef> {
    let definitions = registry.definitions()?;
    definitions
        .current(&mush_core::DefinitionKey {
            owner: crate::store::owner(),
            kind: DefinitionKind::new("action").ok()?,
            id: DefinitionId::new(ACTION_LOT).ok()?,
        })
        .cloned()
}

// ---- doc 18 cut IC0: the hold behind a lot listing, read where money moves ----

/// What the inventory holds behind one lot listing **right now** (doc 18 cut IC0).
///
/// Read through the typed-state seam the composition delegated to this crate — the
/// same `Ctx::action_state` read `boot::reconcile_lot_holds` makes — and never through
/// a probe of another principal's inventory: the listing names its endpoint and
/// commodity, the reservation id is a pure derivation of the listing, and the one
/// record read is the listing's own hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldStanding {
    /// `Held`, with its deadline still ahead of the world's IC clock.
    Live,
    /// `Held` past its deadline — or stamped on a clock generation the world no
    /// longer runs on, which K4a refuses to compare rather than expiring early and
    /// which the settle inside the bundle would refuse too.
    Lapsed,
    /// Consumed, released, expired, or never placed.
    Gone,
    /// This server composed no lot supply, the listing's identities would not
    /// derive, or the family could not be read from this context.
    Unreadable,
}

/// The standing of the hold behind `listing`, and `None` for a listing that is not a
/// `Supply::Lot` — the one accessor `trade::quote` and `trade::is_current` read.
#[must_use]
pub(crate) fn hold_standing(
    ctx: &mush_core::Ctx<'_>,
    listing: &crate::record::Listing,
) -> Option<HoldStanding> {
    let (endpoint, commodity, _) = listing.source.lot()?;
    let Some(families) = crate::module::lot_supply(ctx.w) else {
        return Some(HoldStanding::Unreadable);
    };
    let (Ok(reservation), Ok(key)) = (
        listing_reservation(listing.id, endpoint, commodity),
        mush_inventory::pool_key(endpoint, commodity),
    ) else {
        return Some(HoldStanding::Unreadable);
    };
    let book = match ctx.action_state(&families.books, &key) {
        Ok(Some(book)) => book,
        Ok(None) => return Some(HoldStanding::Gone),
        Err(_) => return Some(HoldStanding::Unreadable),
    };
    let Some(record) = book.records().iter().find(|r| r.id() == reservation) else {
        return Some(HoldStanding::Gone);
    };
    if !matches!(record.state(), mush_core::ReservationState::Held) {
        return Some(HoldStanding::Gone);
    }
    Some(match ctx.w.world_ic_now().checked_cmp(record.expires()) {
        Ok(std::cmp::Ordering::Less) => HoldStanding::Live,
        Ok(_) | Err(_) => HoldStanding::Lapsed,
    })
}

// ---- doc 18 cut IC7: the pool reads the command layer makes -----------------

/// **One pool's balance, read where money moves** (doc 18 cut IC7): `Ok(0)` when the
/// endpoint holds no pool of that commodity, `Err(())` when this server composed no lot
/// supply or the family cannot be read from this context (a module the composition did
/// not name a quantity delegate) — which every caller refuses on, never reading it as
/// zero. Read through `Ctx::action_state`, IC0's hold read's seam.
pub(crate) fn pool_balance(
    ctx: &mush_core::Ctx<'_>,
    endpoint: EndpointRef,
    commodity: &CanonicalId,
) -> Result<u64, ()> {
    let families = crate::module::lot_supply(ctx.w).ok_or(())?;
    let key = mush_inventory::pool_key(endpoint, commodity).map_err(|_| ())?;
    match ctx.action_state(&families.pools, &key) {
        Ok(None) => Ok(0),
        Ok(Some(scalar)) => u64::try_from(scalar.current()).map_err(|_| ()),
        Err(_) => Err(()),
    }
}

/// **One pool's availability** — balance minus every live hold, the arithmetic
/// `InventoryFamilies::available` makes inside a transaction — read the same way as
/// [`pool_balance`]. A half pool (a scalar with no book) is unreadable, as it is to the
/// families themselves.
pub(crate) fn pool_available(
    ctx: &mush_core::Ctx<'_>,
    endpoint: EndpointRef,
    commodity: &CanonicalId,
) -> Result<u64, ()> {
    let families = crate::module::lot_supply(ctx.w).ok_or(())?;
    let key = mush_inventory::pool_key(endpoint, commodity).map_err(|_| ())?;
    let scalar = ctx.action_state(&families.pools, &key).map_err(|_| ())?;
    let book = ctx.action_state(&families.books, &key).map_err(|_| ())?;
    match (scalar, book) {
        (None, _) => Ok(0),
        (Some(scalar), Some(book)) => book.checked_available(*scalar).map_err(|_| ()),
        (Some(_), None) => Err(()),
    }
}

/// How long a hold placed now lasts in **real** seconds at the world's current IC
/// rate — the longest a lot listing may be advertised for (doc 18 cut IC0).
///
/// `RESERVATION_TTL_SECS` counts IC seconds and the listing's deadline counts real
/// ones, so the conversion is the current epoch's rate. `None` while the IC clock is
/// paused (rate 0): a hold on a paused clock never lapses, so there is nothing to cap.
#[must_use]
pub fn hold_ttl_real_secs(w: &mush_core::World) -> Option<i64> {
    let rate = i64::from(w.world_timeline().current().rate);
    (rate > 0).then(|| mush_inventory::RESERVATION_TTL_SECS / rate)
}

// ---- doc 18 cut IC8: the lapsed hold's release, made by its holder ----------

/// **One lapsed hold-backed listing** (doc 18 cut IC8; D-18-26, Q-18-31): what a
/// releaser needs to stage the holder's own release and to tell them afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LapsedHold {
    /// The listing.
    pub listing: ListingId,
    /// The endpoint its hold sits in (the seller's own vault).
    pub endpoint: EndpointRef,
    /// The commodity's pool there.
    pub commodity: CanonicalId,
    /// How much the hold reserves (the listing's quantity).
    pub quantity: u64,
    /// The grade the listing advertised.
    pub grade: u8,
    /// The catalogue entry's name, for the holder's notice.
    pub name: String,
}

/// **The live hold-backed listings whose `RealUtc` deadline has passed** (doc 18 cut
/// IC8), in ascending id order, at most `limit` — the lapsed listings the market sweep
/// keeps until their hold reads `Gone` (`boot::sweep`). A pure read of the market's
/// documents; whether the hold is still held is the releaser's to find out, by staging.
#[must_use]
pub fn lapsed_holds(w: &mush_core::World, limit: usize) -> Vec<LapsedHold> {
    lapsed_holds_where(w, limit, |_| true)
}

/// [`lapsed_holds`], keeping only the entries `keep` accepts, **filtered before the
/// limit is taken** — so a releaser that skips some listings (an org-owned vault, a
/// holder it has just been refused for) is never starved by a window full of them.
/// Bounded by the store's own live set (`MAX_LISTINGS`).
#[must_use]
pub fn lapsed_holds_where(
    w: &mush_core::World,
    limit: usize,
    mut keep: impl FnMut(&LapsedHold) -> bool,
) -> Vec<LapsedHold> {
    let Some(store) = w.module_state::<crate::store::MarketStore>() else {
        return Vec::new();
    };
    let now = w.now();
    store
        .lot_listings()
        .filter(|l| crate::projection::lapsed(l.expires.at, l.expires.clock, now))
        .filter_map(|l| {
            let (endpoint, commodity, grade) = l.source.lot()?;
            Some(LapsedHold {
                listing: l.id,
                endpoint,
                commodity: commodity.clone(),
                quantity: l.quantity,
                grade,
                name: store
                    .entry_of(&l.catalog)
                    .map_or_else(|| l.catalog.code.to_string(), |e| e.name.to_string()),
            })
        })
        .filter(|hold| keep(hold))
        .take(limit)
        .collect()
}

/// **Release a lapsed listing's hold, as its holder** (doc 18 cut IC8, lead rulings L7
/// and L20; D-18-26, Q-18-31: never by the GOD lapse sweep). The listing must exist, be
/// hold-backed and be lapsed, and `ctx.actor` must be able to take from its endpoint
/// (`mush_inventory::may_take`). It stages the one `release` leg through
/// `stage` **as `ctx.actor`**, keyed exactly as every release is
/// (`market.lot.release.{listing:x}.{ic second:x}`); the reservation it releases is the
/// derived [`listing_reservation`]. It **does not delete the document**: the market
/// sweep deletes it once the hold reads `Gone`, which is the releaser's pull
/// verification. A re-stage after the hold is gone is refused by `require_held` in the
/// prepare and applies nothing twice.
///
/// # Errors
///
/// [`crate::MarketError::NoStanding`] for no such listing (or one the actor cannot take
/// from), [`crate::MarketError::Bad`] for one that is not a lapsed hold-backed listing
/// or when staging failed.
pub fn release_lapsed_hold(
    ctx: &mut mush_core::Ctx<'_>,
    id: ListingId,
) -> Result<(), crate::api::MarketError> {
    let Some(listing) = ctx
        .w
        .module_state::<crate::store::MarketStore>()
        .and_then(|s| s.listing(id).cloned())
    else {
        return Err(crate::api::MarketError::NoStanding(
            crate::api::NO_SUCH_LISTING,
        ));
    };
    let Some((endpoint, commodity, grade)) = listing.source.lot() else {
        return Err(crate::api::MarketError::bad(
            "That listing holds no stock, so there is nothing to release.",
        ));
    };
    if !mush_inventory::may_take(ctx.w, ctx.actor, endpoint) {
        return Err(crate::api::MarketError::NoStanding(
            crate::api::NO_SUCH_LISTING,
        ));
    }
    if !crate::projection::lapsed(listing.expires.at, listing.expires.clock, ctx.w.now()) {
        return Err(crate::api::MarketError::bad(
            "That listing has not lapsed; withdraw it to give its stock back.",
        ));
    }
    let leg = LotLeg {
        op: LotOp::Release,
        listing: id,
        endpoint,
        source: endpoint,
        commodity: commodity.clone(),
        quantity: listing.quantity,
        grade,
        destination: None,
        contract: None,
    };
    if !stage(ctx, &leg) {
        return Err(crate::api::MarketError::bad(
            "This server cannot move lot stock right now, so nothing was released.",
        ));
    }
    Ok(())
}
