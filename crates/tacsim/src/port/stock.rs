//! **Finite seeded stock and the lapsed-hold release** (doc 18 cut IC8; doc 18 §4.3
//! "Finite seeded stock", "The driver"; D-18-26, D-18-32, Q-18-31; lead rulings L4,
//! L5, L7, L19, L20).
//!
//! Three things live here, all driven by the `tacsim.port` tick
//! ([`crate::port_tick`]), which runs as GOD and submits nothing itself: every
//! authoritative act below is made inside `Fire::as_principal`, as the principal it
//! is for, at most one per principal per fire (D-18-32).
//!
//! - **`tacsim_port_stock`, tacsim's first production action provider**
//!   (`StockProvider`): one op — refuse when the stock issue's receipt exists,
//!   otherwise `mint_origin` the authored quantity into the storefront vault as a
//!   `Lineage::Origin` lot and insert the [`StockReceipt`] **in the same bundle**.
//!   The receipt is the durable replay guard (idempotency keys do not survive a
//!   reboot). The offer carries the clerk's stock rows as **server-authenticated
//!   fixed parameters**, derived from the port map at projection (lead ruling L4, the
//!   IC7 `paid` precedent): the submission names a row by its issue key only, and the
//!   prepare re-derives the issue id and refuses a receipted row.
//! - **The stock arm** (`stock_step`): one step per port clerk per fire — mint a
//!   row with no receipt, dissolve a minted lot into the storefront pool, and once
//!   every row is seeded list each ask as a Draw (`Supply::Stock` on the storefront)
//!   when no listing stands, no Draw on the pool is in flight and the pool holds
//!   stock (lead ruling L5's one rule). The drift check pages once per boot per pool
//!   (lead ruling L19). The clerk is a Thing: nobody is notified.
//! - **The lapsed-hold release arm** (`release_verify`, `release_agent`,
//!   `release_step`): a player's hold-backed lot listing that lapsed is released
//!   by the tick acting **as the vault's owner** through
//!   `mush_market::release_lapsed_hold` — never by the GOD market sweep (D-18-26,
//!   Q-18-31; lead ruling L7) — and the holder is told once, on a later fire, after
//!   the market sweep has deleted the listing (the pull verification: the sweep
//!   deletes it only once the hold reads `Gone`).
//!
//! **The typed-family key rule** (lead 2c-iii): the receipt family's value validator
//! cannot see the key, so a writer derives the key from the value (the prepare
//! re-derives the [`StockIssueId`]) and every reader re-checks it (`receipt_at`,
//! `receipt_in`); a mismatch reads as unreadable — fail closed, logged once.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalId, Ctx, DefinitionRef, GOD, IdempotencyKeyText, ObjId, PerceivedRef,
    ProjectedParameter, ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema,
    ProviderOfferDraft, RevisionSet, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::{Amount, CatalogRef, VaultId};
use mush_inventory::record::{EndpointRef, Grade, LotId};
use mush_inventory::{InventoryFamilies, InventoryVerb};
use mush_market::{ListingId, MarketStore, MarketVisibility, PaidKind, Supply};
use mush_org::PrincipalId;
use serde_json::{Map, Value, json};

use crate::definitions::content::LANE_PACK;
use crate::port::def::PortDef;
use crate::port::families::{PortHandles, StockIssueId, StockReceipt};
use crate::port::map::PortMap;
use crate::port::readiness::report_ports;
use crate::port_tick::{
    Fire, PortTickState, Step, SwapRefused, gone, notify, port_error, read_pool,
};
use crate::store::TacsimStore;

/// The mint action's id (doc 18 §4.3).
pub const STOCK_ACTION: &str = "tacsim_port_stock";
/// The mint's outcome id.
pub const STOCK_OUTCOME: &str = "tacsim_port_stock_result";
/// The most stock rows one clerk's offer carries (every vendor of its port).
pub const MAX_CLERK_ROWS: usize = 64;
/// How many times one stock row's mint is submitted with no receipt before the clerk
/// skips it for the boot (doc 18 cut IC14 row 29; lead ruling L24): a mint the prepare
/// always refuses (a tampered map row) never stalls the port's arm.
pub const MAX_STOCK_ATTEMPTS: u8 = 3;
/// A mint's idempotency-key prefix: `tacsim.port.stock/<issue key text>`.
pub const STOCK_KEY_PREFIX: &str = "tacsim.port.stock/";
/// A dissolve's idempotency-key prefix: `tacsim.port.dissolve/<issue key text>`.
pub const DISSOLVE_KEY_PREFIX: &str = "tacsim.port.dissolve/";
/// The `ExternalRef.kind` a minted lot's `Lineage::Origin` names.
pub const STOCK_SOURCE_KIND: &str = "port.stock";
/// The release arm's in-flight ceiling (RAM).
pub const MAX_RELEASING: usize = 64;
/// Fires a staged release is left before it may be re-staged (about two minutes at
/// the 5 s cadence: two market sweeps of grace). A refused release waits the same
/// grace before it is offered again (`PortTickState.release_backoff`).
pub(crate) const RELEASE_GRACE_FIRES: u64 = 24;
/// Fires an in-flight release is watched before it is dropped (about twenty minutes).
const RELEASE_WATCH_FIRES: u64 = 240;

/// One staged lapsed-hold release, awaiting its pull verification (RAM only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Releasing {
    /// The vault's owner the release was staged as.
    pub holder: ObjId,
    /// `PortTickState.fires` when it was staged.
    pub staged_fire: u64,
    /// The quantity the hold reserved.
    pub quantity: u64,
    /// The catalogue entry's name.
    pub name: String,
}

// ---- the typed-family key rule ---------------------------------------------------

/// Set once the first key mismatch has been logged (the rule logs once per process).
static KEY_MISMATCH_LOGGED: AtomicBool = AtomicBool::new(false);

fn checked(found: Option<&StockReceipt>, id: &StockIssueId) -> Result<Option<StockReceipt>, ()> {
    match found {
        None => Ok(None),
        Some(receipt) if receipt.issue_id().ok().as_ref() == Some(id) => Ok(Some(receipt.clone())),
        Some(_) => {
            if !KEY_MISMATCH_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::error!(
                    key = %id.key_text(),
                    "tacsim: a stock receipt is stored under another issue's key; it reads as \
                     unreadable (fail closed)"
                );
            }
            Err(())
        }
    }
}

/// **One stock receipt, read through this module's own typed-state lens**, with the key
/// re-checked (`receipt.issue_id() == key`; lead 2c-iii). `Err` when the lens refuses or
/// the receipt names another issue — never "absent".
pub(crate) fn receipt_at(
    ctx: &Ctx<'_>,
    handle: &StateFamilyHandle<StockIssueId, StockReceipt>,
    id: &StockIssueId,
) -> Result<Option<StockReceipt>, ()> {
    let found = ctx.action_state(handle, id).map_err(|_| ())?;
    checked(found, id)
}

/// [`receipt_at`] from a hook (the boards).
pub(crate) fn receipt_in(
    hook: &mush_core::HookReadContext<'_>,
    handle: &StateFamilyHandle<StockIssueId, StockReceipt>,
    id: &StockIssueId,
) -> Result<Option<StockReceipt>, ()> {
    let found = hook.action_state(handle, id).map_err(|_| ())?;
    checked(found, id)
}

// ---- the clerk's port ------------------------------------------------------------

/// **Whether a lane port entry is latched now** (doc 18 cut IC14 row 31; lead ruling
/// L25): the lane's content latch is applied, not retired, and its entries hold
/// `entry`. A port entry a later lane version dropped keeps its port map (retire keeps
/// the map so a later apply reuses every row), but its clerk idles. Read per entry from
/// the content latch only — never `TacsimStore::applied_in`'s legacy-seed fallback,
/// which is for routes, not for a port's clerk.
#[must_use]
pub(crate) fn entry_latched(w: &World, entry: &CanonicalId) -> bool {
    let Ok(lane) = CanonicalId::new(LANE_PACK) else {
        return false;
    };
    w.module_state::<mush_content::ContentState>()
        .and_then(|content| content.latches.applied(&lane))
        .is_some_and(|applied| !applied.is_retired() && applied.entries().contains_key(entry))
}

/// The lane port whose map records `clerk` as its stock clerk, with its entry (cloned,
/// so the caller may hold `&mut Ctx`). `None` when no present lane map names it, or
/// (doc 18 cut IC14 row 31) when the map's entry is no longer latched.
pub(crate) fn clerk_port(w: &World, clerk: ObjId) -> Option<(PortMap, PortDef)> {
    let store = w.module_state::<TacsimStore>()?;
    let map = store.port_maps().find(|map| {
        map.pack.as_str() == LANE_PACK && map.clerk.as_ref().is_some_and(|c| c.thing == clerk.0)
    })?;
    if !entry_latched(w, &map.entry) {
        return None;
    }
    let (_, ports) = report_ports(store, w);
    let def = ports
        .into_iter()
        .find(|(entry, _)| *entry == map.entry)
        .map(|(_, def)| def.clone())?;
    Some((map.clone(), def))
}

/// **The clerk's stock rows as the offer's fixed parameters** (lead ruling L4): every
/// stock row of every vendor the clerk's map records, keyed by issue key text in
/// ascending order, at most [`MAX_CLERK_ROWS`]. Derived from authoritative state at
/// projection — never from the submitter.
pub(crate) fn clerk_rows(w: &World, clerk: ObjId) -> BTreeMap<String, Value> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return BTreeMap::new();
    };
    rows_of(store.port_maps(), clerk)
}

/// [`clerk_rows`] over a set of port maps (pure).
fn rows_of<'a>(maps: impl Iterator<Item = &'a PortMap>, clerk: ObjId) -> BTreeMap<String, Value> {
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    for map in maps.filter(|map| {
        map.pack.as_str() == LANE_PACK && map.clerk.as_ref().is_some_and(|c| c.thing == clerk.0)
    }) {
        for (org, vendor) in &map.vendors {
            for row in &vendor.stock {
                rows.insert(
                    row.id.clone(),
                    json!({
                        "pack": map.pack.as_str(),
                        "entry": map.entry.as_str(),
                        "org": org.as_str(),
                        "good": row.good.as_str(),
                        "issue": row.issue,
                        "qty": row.qty,
                        "grade": row.grade,
                        "vault": vendor.vault,
                        "lot": row.lot,
                        "clerk": clerk.0,
                    }),
                );
            }
        }
    }
    rows.into_iter().take(MAX_CLERK_ROWS).collect()
}

// ---- the provider ------------------------------------------------------------------

/// **`tacsim_port_stock`** (doc 18 §4.3 "Finite seeded stock"; lead ruling L4).
pub(crate) struct StockProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub receipts: StateFamilyHandle<StockIssueId, StockReceipt>,
    pub inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the stock mint is self-scoped, takes no targets and names one stock issue",
    )
}

fn text<'p>(entry: &'p Map<String, Value>, key: &str) -> Result<&'p str, mush_core::ActionError> {
    entry.get(key).and_then(Value::as_str).ok_or_else(invalid)
}

fn number(entry: &Map<String, Value>, key: &str) -> Result<u64, mush_core::ActionError> {
    entry.get(key).and_then(Value::as_u64).ok_or_else(invalid)
}

fn canonical(entry: &Map<String, Value>, key: &str) -> Result<CanonicalId, mush_core::ActionError> {
    CanonicalId::new(text(entry, key)?).map_err(|_| invalid())
}

/// The invocation's parameters, checked for shape: self-scoped, no targets, an object
/// naming one `issue`.
fn parameters(
    invocation: &ActionInvocation,
) -> Result<&Map<String, Value>, mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    parameters
        .get("issue")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    Ok(parameters)
}

impl ActionProvider for StockProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let rows = clerk_rows(&engine.world, query.actor());
        if rows.is_empty() {
            return Ok(None);
        }
        let choices: Vec<String> = rows.keys().cloned().collect();
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the stock mint's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the stock mint's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the stock mint's definition guard is unavailable"))?;
        let issue = ProjectedParameter {
            id: CanonicalId::new("issue")
                .map_err(|_| integrity("the stock mint's parameter id is invalid"))?,
            required: true,
            kind: ProjectedParameterKind::Choice { choices },
        };
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "rows": Value::Object(rows.into_iter().collect()) }),
            parameters: ProjectedParameterSchema::new(vec![issue])
                .map_err(|_| integrity("the stock mint's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-port-stock",
                "Mint port stock",
                "A port stock clerk mints one authored stock issue into its storefront.",
            )
            .map_err(|_| integrity("the stock mint's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the stock mint's offer guards are invalid"))?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        let parameters = parameters(invocation)?;
        let key = text(parameters, "issue")?;
        // 1. The server-authenticated row (L4); an absent one is not this clerk's.
        let entry = parameters
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(key))
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let pack = canonical(entry, "pack")?;
        let port = canonical(entry, "entry")?;
        let org = canonical(entry, "org")?;
        let good = canonical(entry, "good")?;
        let issue = u32::try_from(number(entry, "issue")?).map_err(|_| invalid())?;
        let qty = number(entry, "qty")?;
        let grade = u8::try_from(number(entry, "grade")?).map_err(|_| invalid())?;
        let vault = number(entry, "vault")?;
        let lot = number(entry, "lot")?;
        let clerk = u32::try_from(number(entry, "clerk")?).map_err(|_| invalid())?;
        // 2. The row is the acting clerk's.
        let actor = invocation.actor();
        if clerk != actor.0 {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that stock issue is another clerk's",
            ));
        }
        // 3–4. The key, the lot and the grade re-derived from the row itself.
        let id = StockIssueId::derive(&pack, &port, &org, &good, issue).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the stock issue identity cannot be derived",
            )
        })?;
        if id.key_text() != key || id.lot_id().0 != lot {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "the stock row does not name its own issue",
            ));
        }
        let grade_value = Grade::new(grade).map_err(|_| invalid())?;
        // 5. The receipt is the replay guard.
        let prior = txn.get(&self.receipts, &id).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the stock receipts are unavailable",
            )
        })?;
        if prior.is_some() {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that stock issue is already minted",
            ));
        }
        // 6. The lot and the receipt, in this one bundle.
        let source = mush_economy::ExternalRef {
            provider: smol_str::SmolStr::new_static(crate::NS),
            kind: smol_str::SmolStr::new_static(STOCK_SOURCE_KIND),
            id: smol_str::SmolStr::new(format!("{pack}/{port}/{org}/{good}")),
            revision: Some(u64::from(issue)),
        };
        self.inventory
            .mint_origin(
                txn,
                EndpointRef(VaultId(vault)),
                &good,
                qty,
                grade_value,
                source,
                id.lot_id(),
            )
            .map_err(|_| {
                port_error(
                    ActionErrorCode::ProviderRejected,
                    "the stock lot could not be minted",
                )
            })?;
        let receipt = StockReceipt {
            pack,
            entry: port,
            org,
            good,
            issue,
            qty,
            grade,
            vault: VaultId(vault),
            lot,
            clerk: actor,
        };
        receipt.check().map_err(|_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the stock receipt is out of bounds",
            )
        })?;
        txn.insert(&self.receipts, &id, receipt).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the stock receipt did not stage",
            )
        })?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": actor.0, "issue": key, "lot": lot, "qty": qty, "vault": vault }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the stock mint's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ---- the stock arm -------------------------------------------------------------

/// What one ask's pool and listing read as (the drift check, the readiness report).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AskReading {
    /// The live listing on `(vault, good)` and its quantity.
    pub listing: Option<(ListingId, u64)>,
    /// Σ quantity over open Draw quotes on the pool not yet drawn.
    pub undrawn: u64,
    /// Whether any Draw quote on the pool is unsettled.
    pub in_flight: bool,
}

/// The listing and the open Draw quotes on one storefront pool.
pub(crate) fn ask_reading(store: &MarketStore, vault: u64, good: &CanonicalId) -> AskReading {
    let endpoint = EndpointRef(VaultId(vault));
    let listing = store
        .lot_listing_of(endpoint, good)
        .and_then(|id| store.listing(id).map(|l| (id, l.quantity)));
    let mut reading = AskReading {
        listing,
        ..AskReading::default()
    };
    for quote in store.quotes() {
        let Some(lot) = &quote.lot else {
            continue;
        };
        if lot.kind != PaidKind::Draw
            || lot.source.0 != endpoint
            || &lot.commodity.0 != good
            || quote.settled.is_some()
        {
            continue;
        }
        reading.in_flight = true;
        if quote.drawn_at.is_none() {
            reading.undrawn = reading.undrawn.saturating_add(quote.quantity);
        }
    }
    reading
}

/// Whether a reading drifts (doc 18 §4.3; lead ruling L19): fewer in the pool than
/// listed (an oversell risk), or a live listing with more in the pool than it and the
/// undrawn Draws account for (un-listed stock while a listing stands).
pub(crate) fn drifts(pool: u64, reading: &AskReading) -> bool {
    match reading.listing {
        Some((_, listed)) => pool < listed || pool > listed.saturating_add(reading.undrawn),
        None => false,
    }
}

/// `price_major` in the credit's minor units on this world.
fn ask_price(w: &World, price_major: u64) -> Option<Amount> {
    let scale = crate::port::apply::credit_scale(w);
    crate::port::apply::minor_of(price_major, scale).map(Amount::from_minor)
}

/// One swapped submission's step: `Submitted`, or why not.
fn swapped(result: Result<(), SwapRefused>) -> Step {
    match result {
        Ok(()) => Step::Submitted,
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

/// **The stock arm** (doc 18 §4.3 "The driver"; D-18-26, D-18-32; lead ruling L5): one
/// step for the port's clerk this fire — the first that is due, in authored order: mint
/// a row with no receipt, dissolve a minted lot still standing at the storefront, then
/// (every row seeded) list an ask whose pool has stock, no live listing and no Draw in
/// flight, then (doc 18 cut IC10, lead 2h-ii) re-post a lapsed standing bid. The drift
/// check takes no step. Nothing is notified: the clerk is a Thing.
#[allow(clippy::too_many_lines)]
pub(crate) fn stock_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
) -> Step {
    if gone(ctx.w, clerk) {
        return Step::Refused;
    }
    let Some((map, def)) = clerk_port(ctx.w, clerk) else {
        return Step::Idle;
    };
    let Some(action) = state.stock_action.clone() else {
        return Step::Idle;
    };
    let Some(handles) = ctx.w.module_state::<PortHandles>().cloned() else {
        return Step::Refused;
    };
    let Some(inventory) = handles.inventory.clone() else {
        return Step::Refused;
    };
    // 1–3. Mint, then dissolve, row by row in authored order. Doc 18 cut IC14 row 29
    // (lead ruling L24): a row the clerk's offer does not carry, or whose mint was
    // refused `MAX_STOCK_ATTEMPTS` times, is skipped (warned once a boot) and the step
    // goes on to the next row — it never stalls the port's arm.
    let carried = clerk_rows(ctx.w, clerk);
    let mut uncarried = 0_usize;
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        for stock in &row.stock {
            if !carried.contains_key(&stock.id) {
                uncarried = uncarried.saturating_add(1);
                continue;
            }
            if state.stock_skipped.contains(&stock.id) {
                continue;
            }
            let Some(id) = StockIssueId::parse(&stock.id) else {
                tracing::error!(
                    entry = %map.entry, org = %vendor.org, good = %stock.good,
                    "tacsim: a recorded stock row names no issue id; repair the port map"
                );
                return Step::Refused;
            };
            let Ok(receipt) = receipt_at(ctx, &handles.families.port_stock, &id) else {
                return Step::Refused;
            };
            if receipt.is_some() {
                state.stock_attempts.remove(&stock.id);
            }
            match receipt {
                None => {
                    let attempts = state.stock_attempts.get(&stock.id).copied().unwrap_or(0);
                    if attempts >= MAX_STOCK_ATTEMPTS && !ctx.authoritative_pending(clerk) {
                        if state.stock_skipped.len() < MAX_CLERK_ROWS {
                            state.stock_skipped.insert(stock.id.clone());
                        }
                        state.stock_attempts.remove(&stock.id);
                        tracing::warn!(
                            entry = %map.entry, org = %vendor.org, good = %stock.good,
                            issue = %stock.id,
                            attempts,
                            "tacsim: a stock row's mint never lands; the clerk skips it this \
                             boot and stocks the rest (repair the port map row)"
                        );
                        continue;
                    }
                    let key = format!("{STOCK_KEY_PREFIX}{}", stock.id);
                    let Ok(key) = IdempotencyKeyText::parse_canonical(key) else {
                        return Step::Refused;
                    };
                    let parameters = json!({ "issue": stock.id });
                    let step = swapped(fire.as_principal(ctx, clerk, clerk, |ctx| {
                        ctx.submit_authoritative(
                            action.clone(),
                            PerceivedRef::SelfActor,
                            Vec::new(),
                            parameters,
                            key,
                        );
                    }));
                    if step == Step::Submitted
                        && (state.stock_attempts.contains_key(&stock.id)
                            || state.stock_attempts.len() < MAX_CLERK_ROWS)
                    {
                        state
                            .stock_attempts
                            .insert(stock.id.clone(), attempts.saturating_add(1));
                    }
                    return step;
                }
                Some(receipt) => {
                    let standing = ctx
                        .action_state(&inventory.lots, &LotId(receipt.lot))
                        .map(|lot| lot.is_some());
                    match standing {
                        Err(_) => return Step::Refused,
                        Ok(false) => {}
                        Ok(true) => {
                            let Some(dissolve) =
                                mush_inventory::action_ref(ctx.reg, InventoryVerb::Dissolve)
                            else {
                                return Step::Refused;
                            };
                            let key = format!("{DISSOLVE_KEY_PREFIX}{}", stock.id);
                            let Ok(key) = IdempotencyKeyText::parse_canonical(key) else {
                                return Step::Refused;
                            };
                            let parameters = json!({
                                "endpoint": receipt.vault.0.to_string(),
                                "lot": LotId(receipt.lot).to_string(),
                            });
                            return swapped(fire.as_principal(ctx, clerk, clerk, |ctx| {
                                ctx.submit_authoritative(
                                    dissolve,
                                    PerceivedRef::SelfActor,
                                    Vec::new(),
                                    parameters,
                                    key,
                                );
                            }));
                        }
                    }
                }
            }
        }
    }
    if uncarried > 0 && state.stock_uncarried.insert(map.entry.clone()) {
        tracing::warn!(
            entry = %map.entry,
            uncarried,
            max = MAX_CLERK_ROWS,
            "tacsim: a port records more stock rows than its clerk carries; the rows past \
             the clerk's bound are never minted (re-author the port under the bound)"
        );
    }
    // 4–5. Every row is seeded: the drift check, then the list step.
    let mut due: Option<(mush_market::VendorId, u64, CanonicalId, u64, u64)> = None;
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        for ask in &vendor.asks {
            let Ok(pool) = read_pool(ctx, EndpointRef(VaultId(row.vault)), &ask.good) else {
                return Step::Refused;
            };
            let reading = ctx
                .w
                .module_state::<MarketStore>()
                .map(|store| ask_reading(store, row.vault, &ask.good))
                .unwrap_or_default();
            if drifts(pool, &reading) && state.paged.insert((row.vault, ask.good.clone())) {
                tracing::error!(
                    vault = row.vault,
                    good = %ask.good,
                    listing = reading.listing.map(|(id, _)| id.0),
                    pool,
                    runbook = "@tacsim/port status; @inv/show <vault>",
                    "tacsim: a port storefront's stock and its listing disagree"
                );
            }
            if due.is_none()
                && reading.listing.is_none()
                && !reading.in_flight
                && pool > 0
                && let Some(vendor_id) = row.vendor
            {
                due = Some((
                    mush_market::VendorId(vendor_id),
                    row.vault,
                    ask.good.clone(),
                    ask.price_major,
                    pool,
                ));
            }
        }
    }
    let Some((vendor, vault, good, price_major, pool)) = due else {
        return bid_step(ctx, state, fire, clerk, &map, &def);
    };
    let Some(price) = ask_price(ctx.w, price_major) else {
        return Step::Refused;
    };
    let spec = mush_market::ListingSpec {
        vendor,
        catalog: CatalogRef {
            ruleset: smol_str::SmolStr::new_static(crate::NS),
            code: smol_str::SmolStr::new(good.as_str()),
        },
        price,
        quantity: pool.min(mush_market::MAX_QUANTITY),
        source: Supply::Stock {
            vault: VaultId(vault),
        },
        object: None,
        visibility: MarketVisibility::Public,
        ttl_secs: mush_market::MAX_LISTING_TTL_SECS,
    };
    match fire.as_principal(ctx, clerk, clerk, |ctx| mush_market::list_goods(ctx, spec)) {
        Ok(Ok(_)) => Step::Worked,
        Ok(Err(error)) => {
            tracing::warn!(
                vault, good = %good, ?error,
                "tacsim: the stock clerk could not list an ask"
            );
            Step::Refused
        }
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

/// **The clerk's bid re-post** (doc 18 cut IC10, lead 2h-ii; decisions 7 and 23): the
/// stock arm's last step kind. The **first** bid in authored order whose `BidRow`
/// exists, whose listing is gone from the market, whose recorded deadline has passed
/// and which the applier itself would post afresh (`posts_afresh`: the authored intake
/// unchanged) is posted again as the clerk — at its **full authored intake** and price,
/// built by the applier's own `wanted_spec` — and only that `BidRow`'s `listing` and
/// `expires` are rewritten. It submits nothing; a duplicate is refused by
/// `post_wanted`. The actor swap's refusals map exactly as the ask list step's do.
fn bid_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    def: &PortDef,
) -> Step {
    let now = ctx.w.now();
    let mut due: Option<(
        CanonicalId,
        crate::port::def::BidDef,
        VaultId,
        mush_market::VendorId,
    )> = None;
    'vendors: for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        let Some(vendor_id) = row.vendor else {
            continue;
        };
        for bid in &vendor.bids {
            let Some(recorded) = row.bids.get(&bid.good) else {
                continue;
            };
            let gone = ctx
                .w
                .module_state::<MarketStore>()
                .is_some_and(|s| s.listing(ListingId(recorded.listing)).is_none());
            if gone
                && now >= recorded.expires
                && crate::port::vendor::posts_afresh(ctx.w, Some(recorded), bid)
            {
                due = Some((
                    vendor.org.clone(),
                    bid.clone(),
                    VaultId(row.vault),
                    mush_market::VendorId(vendor_id),
                ));
                break 'vendors;
            }
        }
    }
    let Some((org, bid, vault, vendor)) = due else {
        return Step::Idle;
    };
    let Some(price) = ask_price(ctx.w, bid.price_major) else {
        return Step::Refused;
    };
    let spec = crate::port::vendor::wanted_spec(vendor, vault, &bid, price);
    match fire.as_principal(ctx, clerk, clerk, |ctx| mush_market::post_wanted(ctx, spec)) {
        Ok(Ok(listing)) => {
            let expires = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.listing(listing).map(|l| l.expires.at));
            let mut map = map.clone();
            if let (Some(expires), Some(row)) = (expires, map.vendors.get_mut(&org))
                && let Some(recorded) = row.bids.get_mut(&bid.good)
            {
                recorded.listing = listing.0;
                recorded.expires = expires;
                crate::store::put_port_map(ctx, &map);
            }
            Step::Worked
        }
        Ok(Err(error)) => {
            if state.bid_warned.insert((vault.0, bid.good.clone())) {
                tracing::warn!(
                    vault = vault.0, good = %bid.good, ?error,
                    "tacsim: the stock clerk could not post a lapsed bid again"
                );
            }
            Step::Refused
        }
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

// ---- the release arm ------------------------------------------------------------

/// The notice a released holder reads (live, or queued for their next login).
#[must_use]
pub fn release_notice(listing: u64, quantity: u64, name: &str) -> String {
    format!(
        "Your market listing #{listing} lapsed; the {quantity} of {name} it held are free again."
    )
}

/// **The release arm's pull verification** (D-18-32 step 5), run at the top of every
/// fire that finds the layer on, whether or not any agent is offered: a staged release
/// whose listing is gone from the market — which the sweep does only once the hold
/// reads `Gone` — tells its holder and is forgotten; one watched past
/// [`RELEASE_WATCH_FIRES`] with its listing still standing is dropped and logged. It
/// submits nothing and needs no swap.
pub(crate) fn release_verify(ctx: &mut Ctx<'_>, state: &mut PortTickState) {
    let entries: Vec<(u64, Releasing)> = state
        .releasing
        .iter()
        .map(|(id, entry)| (*id, entry.clone()))
        .collect();
    for (listing, entry) in entries {
        let standing = ctx
            .w
            .module_state::<MarketStore>()
            .is_some_and(|store| store.listing(ListingId(listing)).is_some());
        if !standing {
            notify(
                ctx,
                entry.holder,
                &release_notice(listing, entry.quantity, &entry.name),
            );
            state.releasing.remove(&listing);
        } else if entry.staged_fire.saturating_add(RELEASE_WATCH_FIRES) < state.fires {
            tracing::warn!(
                listing,
                holder = entry.holder.0,
                "tacsim: a lapsed listing's release was not seen to land; it will be staged again"
            );
            state.releasing.remove(&listing);
        }
    }
}

/// **The fire's one release agent** (doc 18 §5's IC8 row; D-18-26): the lowest-id
/// lapsed hold-backed listing whose endpoint vault a live player object owns, that is
/// not inside its re-stage grace and that was not refused within the last
/// [`RELEASE_GRACE_FIRES`]. An org-owned vault is not a player listing, and a vault
/// whose owner is GOD or gone has no holder the tick may act as (`Fire::turn` refuses
/// both): each is skipped and logged once. **Skipped listings are filtered before the
/// window is taken** (`mush_market::lapsed_holds_where`), so no number of them — the
/// sweep keeps every lapsed hold-backed listing until its hold is gone (lead ruling
/// L7) — and no listing that is always refused can starve a later player's release.
pub(crate) fn release_agent(w: &World, state: &mut PortTickState) -> Option<(ObjId, u64)> {
    let economy = w.module_state::<mush_economy::EconomyStore>()?;
    let fires = state.fires;
    state
        .release_backoff
        .retain(|_, refused| refused.saturating_add(RELEASE_GRACE_FIRES) > fires);
    let PortTickState {
        releasing,
        release_warned,
        release_backoff,
        ..
    } = state;
    let mut holder = None;
    let first = mush_market::lapsed_holds_where(w, 1, |hold| {
        let listing = hold.listing.0;
        if release_backoff.contains_key(&listing)
            || releasing
                .get(&listing)
                .is_some_and(|r| r.staged_fire.saturating_add(RELEASE_GRACE_FIRES) > fires)
        {
            return false;
        }
        let owner = economy
            .snapshot()
            .vault(hold.endpoint.vault())
            .map(|vault| vault.owner.clone());
        match owner {
            Some(PrincipalId::Object(owner)) if owner != GOD && !gone(w, owner) => {
                holder = Some(owner);
                true
            }
            _ => {
                if release_warned.insert(listing) {
                    tracing::info!(
                        listing,
                        "tacsim: a lapsed hold-backed listing's vault has no live player \
                         owner; it is left for its owner or staff to withdraw"
                    );
                }
                false
            }
        }
    });
    let listing = first.first()?.listing.0;
    holder.map(|holder| (holder, listing))
}

/// **The release step** (lead rulings L7, L20): as the holder, stage the release of
/// the lapsed listing's hold through `mush_market::release_lapsed_hold`. `Submitted`
/// records it for [`release_verify`]; it never deletes a market document. Every
/// refusal puts the listing in `PortTickState.release_backoff`, so the next fire's
/// agent is the next listing in line; a deferral (the holder has already acted, or
/// has a submission queued) does not.
pub(crate) fn release_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    holder: ObjId,
    listing: u64,
) -> Step {
    let Some(hold) = mush_market::lapsed_holds_where(ctx.w, 1, |hold| hold.listing.0 == listing)
        .into_iter()
        .next()
    else {
        return Step::Idle;
    };
    if state.releasing.len() >= MAX_RELEASING && !state.releasing.contains_key(&listing) {
        if state.release_warned.insert(listing) {
            tracing::warn!(
                listing,
                "tacsim: the release arm is watching as many releases as it holds; this one waits"
            );
        }
        state.release_backoff.insert(listing, state.fires);
        return Step::Refused;
    }
    match fire.as_principal(ctx, holder, holder, |ctx| {
        mush_market::release_lapsed_hold(ctx, ListingId(listing))
    }) {
        Ok(Ok(())) => {
            state.releasing.insert(
                listing,
                Releasing {
                    holder,
                    staged_fire: state.fires,
                    quantity: hold.quantity,
                    name: hold.name,
                },
            );
            Step::Submitted
        }
        Ok(Err(error)) => {
            if state.release_warned.insert(listing) {
                tracing::warn!(
                    listing,
                    holder = holder.0,
                    ?error,
                    "tacsim: a lapsed listing's release could not be staged"
                );
            }
            state.release_backoff.insert(listing, state.fires);
            Step::Refused
        }
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => {
            state.release_backoff.insert(listing, state.fires);
            Step::Refused
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::map::{ClerkRow, StockRow, VendorRow};

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).expect("id")
    }

    /// **The offer rows are derived from the map** (lead ruling L4): every stock row of
    /// every recorded vendor of the clerk's lane map, keyed by its issue id, carrying
    /// the storefront vault and the clerk — and nothing for another object.
    #[test]
    fn ic8_the_offer_rows_are_derived_from_the_map() {
        let pack = cid(LANE_PACK);
        let entry = cid("ceres_port");
        let org = cid("freeport_exchange");
        let good = cid("metals");
        let id = StockIssueId::derive(&pack, &entry, &org, &good, 1).expect("derives");
        let mut map = PortMap::new(pack, entry);
        map.clerk = Some(ClerkRow {
            agent: cid("ceres_port.clerk"),
            thing: 40,
        });
        map.vendors.insert(
            org.clone(),
            VendorRow {
                storefront: 41,
                vault: 7,
                vendor: Some(2),
                signatory: 42,
                grant: Some(3),
                stock: vec![StockRow {
                    good: good.clone(),
                    issue: 1,
                    qty: 4_000,
                    grade: 50,
                    id: id.key_text(),
                    lot: id.lot_id().0,
                }],
                bids: BTreeMap::new(),
            },
        );
        let rows = rows_of(std::iter::once(&map), ObjId(40));
        assert_eq!(rows.len(), 1);
        let row = rows.get(&id.key_text()).expect("the row, by its issue key");
        assert_eq!(row["org"], "freeport_exchange");
        assert_eq!(row["good"], "metals");
        assert_eq!(row["qty"], 4_000);
        assert_eq!(row["grade"], 50);
        assert_eq!(row["vault"], 7);
        assert_eq!(row["lot"], id.lot_id().0);
        assert_eq!(row["clerk"], 40);
        assert!(
            rows_of(std::iter::once(&map), ObjId(41)).is_empty(),
            "another object gets nothing"
        );
    }
}
