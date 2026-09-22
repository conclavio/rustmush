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
use mush_economy::VaultId;
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFamilies, LotSale, MAX_GRADE, MAX_LOT_QUANTITY,
    pool_resource,
};
use serde_json::{Value, json};

use crate::ids::ListingId;

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

/// Which of the three the one action is doing.
///
/// One action, three ops, because the bundle each stages is the same shape and because
/// **one** market-owned `Authoritative` action is what §15.5 (c′) prescribes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LotOp {
    /// Place the listing's hold, at `@market/vendor stock`.
    Reserve,
    /// Settle it and hand the matter over, at `+market/accept`.
    Consume,
    /// Give it back, at `@market/vendor withdraw`.
    Release,
}

impl LotOp {
    /// The parameter word.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            LotOp::Reserve => "reserve",
            LotOp::Consume => "consume",
            LotOp::Release => "release",
        }
    }

    fn parse(raw: &str) -> Option<LotOp> {
        match raw {
            "reserve" => Some(LotOp::Reserve),
            "consume" => Some(LotOp::Consume),
            "release" => Some(LotOp::Release),
            _ => None,
        }
    }
}

/// The one market-owned action's provider.
pub(crate) struct MarketLotProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: InventoryFamilies,
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
        let parameters = vec![
            parameter(
                "op",
                true,
                ProjectedParameterKind::Choice {
                    choices: vec![
                        LotOp::Reserve.word().to_owned(),
                        LotOp::Consume.word().to_owned(),
                        LotOp::Release.word().to_owned(),
                    ],
                },
            )?,
            // `reserve` and `release` act on the SELLER's endpoint and `consume` on the
            // BUYER's, and in every case it is the acting principal's own — which is
            // why one `Choice` list serves all three and why the offer's regeneration
            // is the gate rather than a stored consent.
            parameter(
                "endpoint",
                true,
                ProjectedParameterKind::Choice {
                    choices: choices.clone(),
                },
            )?,
            // The seller's endpoint on a consume. It is NOT a `Choice` over the buyer's
            // own list: the seller's authority was frozen in the reservation at listing
            // time, and the reservation id is derived from `(listing, source, commodity)`
            // — so naming a source the listing did not use derives an id no book holds
            // and the settle refuses with `LOT_REFUSED`.
            parameter(
                "source",
                false,
                ProjectedParameterKind::Text { maximum_bytes: 32 },
            )?,
            parameter(
                "commodity",
                true,
                ProjectedParameterKind::Text { maximum_bytes: 64 },
            )?,
            parameter(
                "listing",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 1,
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
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "now": engine.world.world_ic_now() }),
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
                let grade = Grade::new(
                    u8::try_from(parameters.get("grade").and_then(Value::as_u64).unwrap_or(0))
                        .map_err(|_| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))?,
                )
                .map_err(|_| error(ActionErrorCode::InvalidRequest, LOT_REFUSED))?;
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
        },
    )?;
    Ok(())
}

/// What one command asks the lot action to do.
#[derive(Clone, Debug)]
pub struct LotLeg {
    /// Which of the three.
    pub op: LotOp,
    /// The listing the reservation is scoped to.
    pub listing: ListingId,
    /// The endpoint the ACTING principal owns — the seller's on a reserve or a
    /// release, the buyer's on a consume.
    pub endpoint: EndpointRef,
    /// The seller's endpoint on a consume, and the same as `endpoint` otherwise.
    pub source: EndpointRef,
    /// The commodity's pool at the source endpoint.
    pub commodity: CanonicalId,
    /// How much.
    pub quantity: u64,
    /// The grade the listing advertises.
    pub grade: u8,
}

/// **Stage the ONE `Ctx::submit_authoritative` a lot leg costs its command** (cut 2's
/// fold C4: exactly one per command).
///
/// The key is a pure function of the leg — `market.lot.{op}/{listing}` plus the IC
/// second — so a retyped command inside one second is a replay **HIT** that writes
/// nothing, while a deliberate retry in a later second is a new request. The handler
/// performs no mutation of its own and emits no success line: the mechanic is the
/// provider's and the confirmation is the receipt's, held behind the durable ack.
///
/// Returns `false` when this server composed no lot supply, so the caller can refuse
/// byte-identically to E6c rather than pretending something was staged.
pub(crate) fn stage(ctx: &mut mush_core::Ctx<'_>, leg: &LotLeg) -> bool {
    let Some(action) = action_ref(ctx.reg) else {
        return false;
    };
    let at = ctx.w.world_ic_now().at;
    let discriminator = format!(
        "market.lot.{}.{:x}.{:x}",
        leg.op.word(),
        leg.listing.0,
        at as u64
    );
    let Ok(key) = IdempotencyKeyText::parse_canonical(discriminator) else {
        return false;
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "op": leg.op.word(),
            "listing": leg.listing.0,
            "endpoint": leg.endpoint.vault().0.to_string(),
            "source": leg.source.vault().0.to_string(),
            "commodity": leg.commodity.as_str(),
            "quantity": leg.quantity,
            "grade": leg.grade,
        }),
        key,
    );
    true
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
