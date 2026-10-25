//! **The mill** (doc 20 cut SM.12a; doc 20 §7.3 "The mill", §12.3, D-20-10; doc 18 §5's
//! post-loop list, P5's supply half).
//!
//! A house that bids for an ore mills it into the good it asks: the Freeport Exchange its
//! `metal_ore` into `metals` (metal concentrate), Occator its `ice_ore` into `volatiles`
//! (water ice), five tonnes of ore to two of product, 20 t a batch, at most 40 t of ore an
//! IC day (the lane's `mill` rows, [`MillDef`], whose doc carries the numbers' basis). It
//! is the Exchange's finite 4,000 t of concentrate's and Occator's 9,600 t of water's only
//! replenishing source, and it closes doc 20's loop: miner, Exchange, trader, Kasei.
//!
//! **Two writes, and why they are two** (D1). The batch is one authoritative bundle,
//! `tacsim_port_mill` (`MillProvider`); the restock of the house's drawn ask is a
//! second, normal-lane write on a later fire. A listing is a legacy slot written through
//! the outbox (`mush_market::MarketStore`'s `put_listing`), and an action's
//! `prepare_authoritative` gets an `ActionTxn` and no world (`mush-market/src/lot.rs`'s
//! module doc), so no bundle can raise a listing. Exactly-once comes from durable state:
//! what the clerk restocks is the mill's durable ledger total less the vendor row's
//! durable `mill_listed` counter (`crate::port::map::VendorRow::mill_listed`), re-read
//! every fire; the listing and the counter are written in one handler outbox (the
//! `bid_step` precedent: `post_wanted`, then `put_port_map`), the next map computed before
//! the listing is written; and `mush_market::restock_ask`'s own backing check refuses a
//! second restock of the same units.
//!
//! **Debit and credit, at one storefront, in one bundle** (D2). The batch debits
//! `batch_t` of ore from the storefront's ore pool and credits `batch_t / ratio.ore ×
//! ratio.product` of product into the same storefront's product pool, with the ledger,
//! in one `ActionTxn` — the extraction cycle's debit-then-make shape, `open_deposit`'s
//! credit under its own record and relay R9a's foundry credit under its receipt. It writes
//! no `inventory.transferred` marker, so the storefronts' marker rings are untouched.
//! **The lineage consequence** (12a-M1; O-7b's default, D-18-26 amended for converted
//! and opened matter): milled product enters by `credit` with no tranche, so it is
//! unrooted exactly as every tonne of mined ore is (block pools are seeded by
//! `open_deposit`'s credit). It moves by Draw and Deliver (`carry_lineage` carries only
//! what a root holds) and can never be lot-listed or crystallized past the pool's root
//! (the Exchange's 4,000 t). **The market never mints**: `restock_ask` reads the pool and
//! writes one listing.
//!
//! **The ledger** ([`MillLedger`], family `tacsim.port_mill`, keyed by [`MillId`]) is the
//! batch's receipt and replay guard: a batch names its ordinal, and the prepare refuses
//! any ordinal but the ledger's next. Its `into` and `ratio` are fixed for its life (D9,
//! O-5's default): the id names the ore, never the product or the ratio, so an authored
//! change of either is refused at the prepare and paged by the step, never folded into
//! its totals.
//!
//! **The cadence** (D6) and **one act a fire** (D7). The step (`mill_step`) runs on
//! the Ceres stock clerk's `Arm::Stock` turn, after the intake drip and before the
//! lapsed-bid re-post, and so behind the emergency bids and the ask list step; it makes
//! at most one act a fire — a restock first, so each batch's units are listed before the
//! next batch lands, then a batch. At most two batches and two restocks a mill an IC day.
//!
//! **The restart argument.** The predicate is the durable ledger total less the durable
//! counter, re-read every fire, so a restart never restocks twice or loses a batch. The
//! RAM back-offs (`PortTickState::mill_refused`, `mill_attempts`, `mill_warned`, the
//! drip's `drip_refused` precedent) only pass a mill over for an IC day or keep a warning
//! to once a boot, and a restart clears them: a mill passed over is tried again at once
//! after a reboot (D1 — the letter of the lead note's "nothing RAM-only" is not met, its
//! purpose, that a restart changes nothing durable, is).
//!
//! **The restock's wait** (the lead's ruling on SM.12a's conformance F2). While any Draw
//! quote on the house's pool is unsettled — accepted and not yet drawn, or drawn and not
//! yet mirrored by the market's sweep — the restock is deferred: a delay, never a loss.
//! The units stay owed (the durable predicate), the Listings line reads Ready with its
//! "milled and not yet listed" clause, nothing pages, and the first fire after the quote
//! settles or lapses (`mush_market`'s draw window) restocks them. The narrower form —
//! attempt the restock every fire and defer only on a backing refusal — was not taken: it
//! spends the shared clerk's one act every fire of the window and copies a market rule
//! (when a Draw's units leave the pool) into the clerk. A buyer who accepts and never
//! draws holds the restock for up to one draw window; the live listing still sells.
//!
//! **The staff remedy for a split restock** (D1; F7). Were the listing and the counter
//! ever to come apart (a store fault, never a code path), the restock is refused each
//! day, the drift check pages and the gated Listings line reads Flagged; `@market/vendor
//! withdraw <listing>` on that ask clears it, after which the stock arm's list step
//! re-lists the whole pool and sets every counter of that good's mills to its ledger
//! total.

use std::sync::atomic::{AtomicBool, Ordering};

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalHash, CanonicalId, Ctx, DefinitionRef, IdempotencyKeyText, ObjId, PerceivedRef,
    ProjectedParameter, ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema,
    ProviderOfferDraft, RevisionSet, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::VaultId;
use mush_inventory::InventoryFamilies;
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::port::def::{MAX_MILL_RATIO, MillDef, MillRatio, PortDef, VendorDef};
use crate::port::families::PortHandles;
use crate::port::map::{PortMap, VendorRow};
use crate::port::readiness::LineState;
use crate::port::stock::{IC_DAY, MAX_CLERK_ROWS, clerk_port, swapped};
use crate::port_tick::{Fire, PortTickState, Step, SwapRefused, port_error};

/// The batch's action id (doc 20 cut SM.12a).
pub const MILL_ACTION: &str = "tacsim_port_mill";
/// The batch's outcome id.
pub const MILL_OUTCOME: &str = "tacsim_port_mill_result";
/// The mill-ledger family (doc 20 cut SM.12a): one [`MillLedger`] per mill.
pub const MILL_FAMILY: &str = "tacsim.port_mill";
/// A batch's idempotency-key prefix: `tacsim.port.mill/<mill key text>/<ordinal>`.
pub const MILL_KEY_PREFIX: &str = "tacsim.port.mill/";
/// How many of its last batches a ledger keeps, its durable audit tail (D3).
pub const MAX_RECENT_BATCHES: usize = 16;
/// How many times one batch ordinal is submitted with the ledger not advancing before the
/// clerk passes that mill's batch over for the rest of the IC day (the stock rule's
/// `MAX_STOCK_ATTEMPTS`, lead ruling L24).
pub const MAX_MILL_ATTEMPTS: u8 = 3;
/// The mill identity's hash domain.
const MILL_ID_DOMAIN: &str = "tacsim.mill_id.v1";
/// The canonical-bytes ceiling of one mill identity's input.
const MILL_ID_MAX_BYTES: usize = 1024;

// ---- the pure arithmetic ------------------------------------------------------------

/// **One batch's figures** (doc 20 cut SM.12a): the ore it mills, the product it makes,
/// and the ore left in the pool after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Milled {
    /// Tonnes of ore debited: always a whole batch.
    pub ore_t: u64,
    /// Tonnes of product credited.
    pub product_t: u64,
    /// Tonnes of ore left unmilled in the pool.
    pub remainder_t: u64,
}

/// **One whole batch, or none** (doc 20 §7.3 "The mill"; cut SM.12a): `None` when the
/// batch is not a positive whole number of ratios, or the pool (`available_t`) or the IC
/// day's room (`room_today_t`) holds less than a batch; else `batch_t` of ore into
/// `batch_t / ratio.ore × ratio.product` of product, the rest of the pool kept as
/// unmilled ore. Never a partial batch. Pure and checked.
#[must_use]
pub fn batch(
    available_t: u64,
    batch_t: u64,
    ratio: MillRatio,
    room_today_t: u64,
) -> Option<Milled> {
    let per = u64::from(ratio.ore);
    if batch_t == 0
        || per == 0
        || batch_t.checked_rem(per)? != 0
        || available_t < batch_t
        || room_today_t < batch_t
    {
        return None;
    }
    let product_t = batch_t
        .checked_div(per)?
        .checked_mul(u64::from(ratio.product))?;
    Some(Milled {
        ore_t: batch_t,
        product_t,
        remainder_t: available_t.checked_sub(batch_t)?,
    })
}

/// **The IC day an IC instant falls in**: `at_ic.div_euclid(IC_DAY)` (the drip's day).
#[must_use]
pub fn ic_day(at_ic: i64) -> i64 {
    at_ic.div_euclid(IC_DAY)
}

/// **The ore a mill may still take today** (doc 20 cut SM.12a): `tally` is the ledger's
/// `(day, day_ore_t)`. No tally, or one of an earlier IC day, gives the whole rate;
/// today's gives what is left of it. A tally of a later day than `today` (a clock set
/// back) is read as today's, so a rewound clock never mills more than the rate. Pure.
#[must_use]
pub fn room_today(tally: Option<(i64, u64)>, today: i64, per_ic_day_t: u64) -> u64 {
    match tally {
        Some((day, ore_t)) if day >= today => per_ic_day_t.saturating_sub(ore_t),
        _ => per_ic_day_t,
    }
}

/// **Whether ore and product totals conserve under a ratio**: `ore_t × ratio.product ==
/// product_t × ratio.ore`, in `u128` so it never overflows. Pure.
#[must_use]
pub fn conserves(ore_t: u64, product_t: u64, ratio: MillRatio) -> bool {
    u128::from(ore_t) * u128::from(ratio.product) == u128::from(product_t) * u128::from(ratio.ore)
}

// ---- the identity -----------------------------------------------------------------

/// **A mill's identity** (doc 20 cut SM.12a): `hash(pack, entry, org, ore)` — the ore, never
/// the product or the ratio (D9, O-5's default), so one house's mill of one ore keeps one
/// ledger for its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MillId(CanonicalHash);

impl MillId {
    /// `CanonicalHash::for_canonical_value("tacsim.mill_id.v1", 1, {pack, entry, org, ore},
    /// 1024)` — the `StockIssueId` shape.
    ///
    /// # Errors
    ///
    /// The hash's own refusal, as text (it cannot refuse four canonical ids).
    pub fn derive(
        pack: &CanonicalId,
        entry: &CanonicalId,
        org: &CanonicalId,
        ore: &CanonicalId,
    ) -> Result<MillId, String> {
        CanonicalHash::for_canonical_value(
            MILL_ID_DOMAIN,
            1,
            &json!({
                "pack": pack.as_str(),
                "entry": entry.as_str(),
                "org": org.as_str(),
                "ore": ore.as_str(),
            }),
            MILL_ID_MAX_BYTES,
        )
        .map(MillId)
        .map_err(|error| error.to_string())
    }

    /// The key text: 64 lowercase hexadecimal digits.
    #[must_use]
    pub fn key_text(&self) -> String {
        self.0.to_hex()
    }

    /// The strict inverse of [`MillId::key_text`].
    #[must_use]
    pub fn parse(text: &str) -> Option<MillId> {
        CanonicalHash::from_hex(text).ok().map(MillId)
    }
}

// ---- the receipt ------------------------------------------------------------------

/// **One batch, as its ledger's audit tail keeps it** (doc 20 cut SM.12a).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MillBatch {
    /// The batch's ordinal, from 1.
    pub ordinal: u64,
    /// The IC instant it was milled at.
    pub at_ic: i64,
    /// Tonnes of ore it milled.
    pub ore_t: u64,
    /// Tonnes of product it made.
    pub product_t: u64,
}

/// **One mill's durable receipt** (doc 20 cut SM.12a; 12a-C4), replaced in every batch's
/// own bundle by `tacsim_port_mill`: the totals conservation is asserted over, the IC
/// day's tally the rate reads, and **the last sixteen batches as the durable audit tail**
/// (the Mill readiness line prints the last one's stamp; [`MillLedger::check`] reads the
/// tail); the ordinal check against `batches` is the replay guard; `into` and `ratio` are
/// fixed for the ledger's life — the [`MillId`] names the ore, not the product, so an
/// authored change of either is refused at the prepare and paged by the step, never
/// folded into these totals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MillLedger {
    /// The pack the mill row was authored in.
    pub pack: CanonicalId,
    /// The port entry.
    pub entry: CanonicalId,
    /// The org whose storefront mills.
    pub org: CanonicalId,
    /// The ore milled.
    pub ore: CanonicalId,
    /// The product made (fixed for the ledger's life).
    pub into: CanonicalId,
    /// The ratio milled at (fixed for the ledger's life).
    pub ratio: MillRatio,
    /// The storefront vault both pools stand in.
    pub vault: VaultId,
    /// The stock clerk the tick acted as.
    pub clerk: ObjId,
    /// Batches milled, `>= 1`.
    pub batches: u64,
    /// Tonnes of ore milled in all.
    pub ore_t: u64,
    /// Tonnes of product made in all.
    pub product_t: u64,
    /// The IC day of the last batch.
    pub day: i64,
    /// Tonnes of ore milled on `day`.
    pub day_ore_t: u64,
    /// The last batches, oldest first, at most [`MAX_RECENT_BATCHES`].
    pub recent: Vec<MillBatch>,
}

impl MillLedger {
    /// This ledger's mill id, re-derived from `(pack, entry, org, ore)`.
    ///
    /// # Errors
    ///
    /// As [`MillId::derive`].
    pub fn mill_id(&self) -> Result<MillId, String> {
        MillId::derive(&self.pack, &self.entry, &self.org, &self.ore)
    }

    /// The ledger's invariants: at least one batch; a ratio in bounds; the totals
    /// conserving; a tail of 1 to [`MAX_RECENT_BATCHES`] batches whose ordinals ascend by
    /// one to `batches`, each milling ore and conserving, summing to no more than the
    /// total; the day the last batch's; the day's tally within the total and at least the
    /// last batch.
    ///
    /// # Errors
    ///
    /// A message naming the first invariant the ledger breaks.
    pub fn check(&self) -> Result<(), String> {
        if self.batches == 0 {
            return Err("a mill ledger records at least one batch".to_owned());
        }
        let ratio = self.ratio;
        if ratio.product == 0 || ratio.product > ratio.ore || ratio.ore > MAX_MILL_RATIO {
            return Err(format!(
                "a mill ledger's ratio is 1..={MAX_MILL_RATIO} tonnes of ore to 1..=that many \
                 of product"
            ));
        }
        if !conserves(self.ore_t, self.product_t, ratio) {
            return Err("a mill ledger's totals conserve under its ratio".to_owned());
        }
        let Some(last) = self.recent.last() else {
            return Err("a mill ledger keeps its last batch".to_owned());
        };
        if self.recent.len() > MAX_RECENT_BATCHES {
            return Err(format!(
                "a mill ledger keeps at most {MAX_RECENT_BATCHES} recent batches"
            ));
        }
        if last.ordinal != self.batches
            || self.recent.first().is_none_or(|first| first.ordinal == 0)
            || self.recent.windows(2).any(|pair| match pair {
                [a, b] => a.ordinal.checked_add(1) != Some(b.ordinal),
                _ => true,
            })
        {
            return Err(
                "a mill ledger's recent ordinals ascend by one to its batch count".to_owned(),
            );
        }
        let mut tail_ore: u64 = 0;
        for recent in &self.recent {
            if recent.ore_t == 0 || !conserves(recent.ore_t, recent.product_t, ratio) {
                return Err(format!(
                    "a mill ledger's batch {} mills ore and conserves under its ratio",
                    recent.ordinal
                ));
            }
            tail_ore = tail_ore
                .checked_add(recent.ore_t)
                .ok_or_else(|| "a mill ledger's recent batches overflow".to_owned())?;
        }
        if tail_ore > self.ore_t {
            return Err("a mill ledger's recent batches sum within its total".to_owned());
        }
        if self.day != ic_day(last.at_ic) {
            return Err("a mill ledger's day is its last batch's".to_owned());
        }
        if self.day_ore_t > self.ore_t || self.day_ore_t < last.ore_t {
            return Err(
                "a mill ledger's day tally is within its total and at least its last batch"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

// ---- the typed-family key rule ------------------------------------------------------

/// Set once the first key mismatch has been logged (the rule logs once per process).
static KEY_MISMATCH_LOGGED: AtomicBool = AtomicBool::new(false);

/// **One mill ledger, read through this module's own typed-state lens**, with the key
/// re-checked (`ledger.mill_id() == id`; the `crate::port::stock` key rule). `Err` when
/// the lens refuses or the ledger names another mill — fail closed, never "absent";
/// a mismatch is logged once a process.
pub(crate) fn ledger_at(
    ctx: &Ctx<'_>,
    handle: &StateFamilyHandle<MillId, MillLedger>,
    id: &MillId,
) -> Result<Option<MillLedger>, ()> {
    match ctx.action_state(handle, id).map_err(|_| ())? {
        None => Ok(None),
        Some(ledger) if ledger.mill_id().ok().as_ref() == Some(id) => Ok(Some(ledger.clone())),
        Some(_) => {
            if !KEY_MISMATCH_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::error!(
                    key = %id.key_text(),
                    "tacsim: a mill ledger is stored under another mill's key; it reads as \
                     unreadable (fail closed)"
                );
            }
            Err(())
        }
    }
}

// ---- the provider -------------------------------------------------------------------

/// **`tacsim_port_mill`** (doc 20 cut SM.12a; §7.3 "The mill"): one batch — the ore
/// debited and the product credited at the storefront, and the mill's ledger advanced
/// exactly once per ordinal, in one bundle.
pub(crate) struct MillProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub mills: StateFamilyHandle<MillId, MillLedger>,
    pub inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the mill is self-scoped, takes no targets and names one mill and its next batch",
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
/// naming one `mill` and an `ordinal` of at least 1.
fn parameters(
    invocation: &ActionInvocation,
) -> Result<&Map<String, Value>, mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    parameters
        .get("mill")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if parameters
        .get("ordinal")
        .and_then(Value::as_u64)
        .is_none_or(|ordinal| ordinal == 0)
    {
        return Err(invalid());
    }
    Ok(parameters)
}

/// **The clerk's mill rows as the offer's fixed parameters** (the stock offer's lead
/// ruling L4): for every vendor of the clerk's port with a map row, every authored mill
/// row, keyed by its [`MillId`] key text in ascending order, at most [`MAX_CLERK_ROWS`].
/// Derived from authoritative state at projection — never from the submitter.
fn mill_rows(w: &World, clerk: ObjId) -> serde_json::Map<String, Value> {
    let mut rows: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    let Some((map, def)) = clerk_port(w, clerk) else {
        return Map::new();
    };
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        for mill in &vendor.mill {
            let Ok(id) = MillId::derive(&map.pack, &map.entry, &vendor.org, &mill.ore) else {
                continue;
            };
            rows.insert(
                id.key_text(),
                json!({
                    "pack": map.pack.as_str(),
                    "entry": map.entry.as_str(),
                    "org": vendor.org.as_str(),
                    "ore": mill.ore.as_str(),
                    "into": mill.into.as_str(),
                    "ratio_ore": mill.ratio.ore,
                    "ratio_product": mill.ratio.product,
                    "batch_t": mill.batch_t,
                    "per_ic_day_t": mill.per_ic_day_t,
                    "vault": row.vault,
                    "clerk": clerk.0,
                }),
            );
        }
    }
    rows.into_iter().take(MAX_CLERK_ROWS).collect()
}

impl ActionProvider for MillProvider {
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
        let rows = mill_rows(&engine.world, query.actor());
        if rows.is_empty() {
            return Ok(None);
        }
        let choices: Vec<String> = rows.keys().cloned().collect();
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the mill's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the mill's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the mill's definition guard is unavailable"))?;
        let mill = ProjectedParameter {
            id: CanonicalId::new("mill")
                .map_err(|_| integrity("the mill's parameter id is invalid"))?,
            required: true,
            kind: ProjectedParameterKind::Choice { choices },
        };
        let ordinal = ProjectedParameter {
            id: CanonicalId::new("ordinal")
                .map_err(|_| integrity("the mill's parameter id is invalid"))?,
            required: true,
            kind: ProjectedParameterKind::Integer {
                minimum: 1,
                maximum: i64::from(u32::MAX),
            },
        };
        Ok(Some(ProviderOfferDraft {
            // The prepare has no world, so the IC instant rides the server-authenticated
            // offer beside the rows.
            fixed_parameters: json!({
                "rows": Value::Object(rows),
                "now": engine.world.world_ic_now().at,
            }),
            parameters: ProjectedParameterSchema::new(vec![mill, ordinal])
                .map_err(|_| integrity("the mill's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-port-mill",
                "Mill ore",
                "A port's stock clerk mills one batch of its house's ore into the good its house \
                 asks.",
            )
            .map_err(|_| integrity("the mill's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the mill's offer guards are invalid"))?,
        }))
    }

    #[allow(clippy::too_many_lines)]
    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        // 1. The parameters as validated.
        let parameters = parameters(invocation)?;
        let key = text(parameters, "mill")?;
        let ordinal = number(parameters, "ordinal")?;
        let now = parameters
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(invalid)?;
        // 2. The server-authenticated row; an absent one is not this clerk's.
        let row = parameters
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(key))
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let pack = canonical(row, "pack")?;
        let port = canonical(row, "entry")?;
        let org = canonical(row, "org")?;
        let ore = canonical(row, "ore")?;
        let into = canonical(row, "into")?;
        let ratio = MillRatio {
            ore: u32::try_from(number(row, "ratio_ore")?).map_err(|_| invalid())?,
            product: u32::try_from(number(row, "ratio_product")?).map_err(|_| invalid())?,
        };
        let batch_t = number(row, "batch_t")?;
        let per_ic_day_t = number(row, "per_ic_day_t")?;
        let vault = number(row, "vault")?;
        let clerk = u32::try_from(number(row, "clerk")?).map_err(|_| invalid())?;
        // 3. The row is the acting clerk's.
        let actor = invocation.actor();
        if clerk != actor.0 {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that mill is another clerk's",
            ));
        }
        // 4. The key re-derived from the row itself.
        let id = MillId::derive(&pack, &port, &org, &ore).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the mill identity cannot be derived",
            )
        })?;
        if id.key_text() != key {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "the mill row does not name its own mill",
            ));
        }
        // 5. The ledger, its key re-checked.
        let prior = txn
            .get(&self.mills, &id)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the mill ledgers are unavailable",
                )
            })?
            .cloned();
        if let Some(ledger) = &prior
            && ledger.mill_id().ok().as_ref() != Some(&id)
        {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "a mill ledger is stored under another mill's key",
            ));
        }
        // 6. The replay guard: only the ledger's next ordinal.
        let next = prior.as_ref().map_or(Some(1), |l| l.batches.checked_add(1));
        if next != Some(ordinal) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that batch is already milled",
            ));
        }
        // 7. D9: a mill's ratio and product are fixed for its ledger's life.
        if let Some(ledger) = &prior
            && (ledger.ratio != ratio || ledger.into != into)
        {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the mill's ratio or product no longer matches its ledger",
            ));
        }
        // 8. Today's room.
        let today = ic_day(now);
        let room = room_today(
            prior.as_ref().map(|l| (l.day, l.day_ore_t)),
            today,
            per_ic_day_t,
        );
        // 9. The ore the storefront holds, less its holds.
        let endpoint = EndpointRef(VaultId(vault));
        let available = self.inventory.available(txn, endpoint, &ore).map_err(|_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the mill's ore cannot be read",
            )
        })?;
        // 10. One whole batch, or nothing is due.
        let milled = batch(available, batch_t, ratio, room)
            .ok_or_else(|| port_error(ActionErrorCode::Conflict, "nothing is due at that mill"))?;
        // 11. The ore debited, then the product credited, at the one storefront.
        let refused = |_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the mill's ore could not be milled",
            )
        };
        self.inventory
            .debit(txn, endpoint, &ore, milled.ore_t)
            .map_err(refused)?;
        self.inventory
            .credit(txn, endpoint, &into, milled.product_t)
            .map_err(refused)?;
        // 12. The next ledger.
        let overflow = || {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the mill ledger's totals overflow",
            )
        };
        let (ore_t, product_t, day_ore_t, mut recent) = match &prior {
            Some(ledger) => (
                ledger
                    .ore_t
                    .checked_add(milled.ore_t)
                    .ok_or_else(overflow)?,
                ledger
                    .product_t
                    .checked_add(milled.product_t)
                    .ok_or_else(overflow)?,
                if ledger.day == today {
                    ledger
                        .day_ore_t
                        .checked_add(milled.ore_t)
                        .ok_or_else(overflow)?
                } else {
                    milled.ore_t
                },
                ledger.recent.clone(),
            ),
            None => (milled.ore_t, milled.product_t, milled.ore_t, Vec::new()),
        };
        recent.push(MillBatch {
            ordinal,
            at_ic: now,
            ore_t: milled.ore_t,
            product_t: milled.product_t,
        });
        // The audit tail keeps the last sixteen: the oldest drops first.
        let excess = recent.len().saturating_sub(MAX_RECENT_BATCHES);
        recent.drain(..excess);
        let ledger = MillLedger {
            pack,
            entry: port,
            org,
            ore,
            into,
            ratio,
            vault: VaultId(vault),
            clerk: actor,
            batches: ordinal,
            ore_t,
            product_t,
            day: today,
            day_ore_t,
            recent,
        };
        ledger.check().map_err(|_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the mill ledger is out of bounds",
            )
        })?;
        let staged = if prior.is_some() {
            txn.replace(&self.mills, &id, ledger)
        } else {
            txn.insert(&self.mills, &id, ledger)
        };
        staged
            .map_err(|_| port_error(ActionErrorCode::Integrity, "the mill ledger did not stage"))?;
        // 13. The outcome, witnessed by the principal.
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": actor.0,
                "mill": key,
                "ordinal": ordinal,
                "ore_t": milled.ore_t,
                "product_t": milled.product_t,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the mill's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ---- milled and not yet listed ----------------------------------------------------

/// **Why a mill's unlisted units cannot be read** (doc 20 cut SM.12a; T7, D27): each
/// error names the mill it is about by its ore — the key of every warning — and a
/// counter fault also the good it blocks, so a caller warns per mill without a second
/// walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MillRead {
    /// The mill's ledger cannot be read (the lens refused, or it names another mill).
    Unreadable {
        /// The mill's ore.
        ore: CanonicalId,
    },
    /// The vendor row's counter is past the mill's ledger total (or there is a counter and
    /// no ledger, or the sum of a good's mills overflows).
    Counter {
        /// The mill's ore.
        ore: CanonicalId,
        /// The good whose ask it blocks.
        good: CanonicalId,
    },
}

impl MillRead {
    /// The ore of the mill the error is about.
    pub(crate) fn ore(&self) -> &CanonicalId {
        match self {
            MillRead::Unreadable { ore } | MillRead::Counter { ore, .. } => ore,
        }
    }

    /// The warning kind it raises.
    pub(crate) fn warn(&self) -> MillWarn {
        match self {
            MillRead::Unreadable { .. } => MillWarn::Unreadable,
            MillRead::Counter { .. } => MillWarn::Counter,
        }
    }
}

/// **One mill's milled-not-yet-listed units, and the good they are** (doc 20 cut SM.12a):
/// the mill's ledger total less the vendor row's `mill_listed` counter for its ore. `None`
/// when the mill has no ledger (nothing milled yet). **The good is the ledger's `into`,
/// never the row's** (D9). A counter with no ledger, or past the ledger's total, is
/// `Err(Counter)` — never saturated silently; an unreadable ledger is `Err(Unreadable)`.
pub(crate) fn owed(
    ctx: &Ctx<'_>,
    mills: &StateFamilyHandle<MillId, MillLedger>,
    map: &PortMap,
    vendor: &VendorDef,
    row: &VendorRow,
    mill: &MillDef,
) -> Result<Option<(CanonicalId, u64)>, MillRead> {
    let unreadable = || MillRead::Unreadable {
        ore: mill.ore.clone(),
    };
    let id =
        MillId::derive(&map.pack, &map.entry, &vendor.org, &mill.ore).map_err(|_| unreadable())?;
    let ledger = ledger_at(ctx, mills, &id).map_err(|()| unreadable())?;
    let counter = row.mill_listed.get(&mill.ore).copied().unwrap_or(0);
    match ledger {
        None if counter > 0 => Err(MillRead::Counter {
            ore: mill.ore.clone(),
            good: mill.into.clone(),
        }),
        None => Ok(None),
        Some(ledger) => match ledger.product_t.checked_sub(counter) {
            Some(units) => Ok(Some((ledger.into, units))),
            None => Err(MillRead::Counter {
                ore: mill.ore.clone(),
                good: ledger.into,
            }),
        },
    }
}

/// **The units of `good` this vendor's mills made and its ask does not yet carry** (doc 20
/// cut SM.12a): Σ [`owed`] over **this vendor's authored mill rows** whose owed good is
/// `good`; `Ok(0)` for a good no mill makes. A removed mill row drops out with its counter
/// (never summed by leftover `mill_listed` keys, 12a-C6). **An error blocks only the good
/// it is about** (T7, D27): a counter fault only when its good is `good`; an unreadable
/// ledger — which has no `into` to read — only when that mill row's **authored** `into` is
/// `good` (the one place the authored product stands in for the ledger's). The first
/// matching error is returned; an overflowing sum is `Err(Counter)` naming the mill whose
/// addition overflowed.
pub(crate) fn unlisted(
    ctx: &Ctx<'_>,
    mills: &StateFamilyHandle<MillId, MillLedger>,
    map: &PortMap,
    vendor: &VendorDef,
    row: &VendorRow,
    good: &CanonicalId,
) -> Result<u64, MillRead> {
    let mut total: u64 = 0;
    for mill in &vendor.mill {
        match owed(ctx, mills, map, vendor, row, mill) {
            Ok(Some((owed_good, units))) if &owed_good == good => {
                total = total.checked_add(units).ok_or_else(|| MillRead::Counter {
                    ore: mill.ore.clone(),
                    good: good.clone(),
                })?;
            }
            Ok(_) => {}
            Err(MillRead::Counter { ore, good: blocked }) if &blocked == good => {
                return Err(MillRead::Counter { ore, good: blocked });
            }
            Err(MillRead::Counter { .. }) => {}
            Err(MillRead::Unreadable { ore }) if &mill.into == good => {
                return Err(MillRead::Unreadable { ore });
            }
            Err(MillRead::Unreadable { .. }) => {}
        }
    }
    Ok(total)
}

// ---- the step -------------------------------------------------------------------------

/// **Which mill act** a refusal passes over for the day (doc 20 cut SM.12a, D8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MillAct {
    /// The restock of the house's drawn ask.
    Restock,
    /// The batch.
    Batch,
}

/// **Why the mill warned** (doc 20 cut SM.12a; O-2b's staff texts): each kind is its own
/// key, so a first warning never hides another; each is logged once a boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MillWarn {
    /// The mill's ledger cannot be read.
    Unreadable,
    /// The vendor row's counter is past the mill's ledger.
    Counter,
    /// A storefront holds less than its mills count as not yet listed (keyed by the ask's
    /// good: no single mill owns it, D21).
    Underflow,
    /// The authored ratio or product no longer matches the ledger (D9).
    Terms,
    /// The storefront's ore pool cannot be read (balance less holds) for a batch: keyed
    /// apart from [`MillWarn::Unreadable`], so a pool fault never hides a later ledger
    /// fault, and worded apart, so staff never repair a healthy ledger.
    Storefront,
    /// The market refused the restock.
    Refused,
    /// A batch's submission never lands.
    Stalled,
}

/// The restock warning's runbook (O-2b).
const RESTOCK_RUNBOOK: &str =
    "@tacsim/port status; @market/vendor withdraw <listing> re-lists the whole pool";

/// **Warn once a boot** under `(vault, key, kind)` — `key` the mill's ore, or for
/// [`MillWarn::Underflow`] the ask's good — at most [`MAX_CLERK_ROWS`] keys a boot.
/// `detail` is the market's refusal for [`MillWarn::Refused`].
pub(crate) fn mill_warn(
    state: &mut PortTickState,
    vault: u64,
    key: &CanonicalId,
    kind: MillWarn,
    detail: &str,
) {
    let entry = (vault, key.clone(), kind);
    if state.mill_warned.contains(&entry) || state.mill_warned.len() >= MAX_CLERK_ROWS {
        return;
    }
    state.mill_warned.insert(entry);
    match kind {
        MillWarn::Unreadable => tracing::warn!(
            vault, ore = %key,
            "tacsim: a mill's ledger cannot be read; the clerk passes that mill and its good's ask \
             over (repair the ledger)"
        ),
        MillWarn::Counter => tracing::warn!(
            vault, ore = %key,
            "tacsim: a mill's counter is past its ledger; the clerk passes that mill and its good's \
             ask over (@market/vendor withdraw <listing> re-lists the whole pool)"
        ),
        MillWarn::Underflow => tracing::warn!(
            vault, good = %key,
            "tacsim: a storefront holds less than its mills count as not yet listed; the clerk \
             passes that ask over (@market/vendor withdraw <listing> re-lists the whole pool)"
        ),
        MillWarn::Terms => tracing::warn!(
            vault, ore = %key,
            "tacsim: a mill's authored ratio or product no longer matches its ledger; author them \
             back (a mill's ratio and product are fixed for its ledger's life)"
        ),
        MillWarn::Storefront => tracing::warn!(
            vault, ore = %key, runbook = "@tacsim/port status; @inv/show <vault>",
            "tacsim: a mill's storefront ore cannot be read; the clerk tries that mill's batch \
             again next fire"
        ),
        MillWarn::Refused => tracing::warn!(
            vault, ore = %key, error = detail, runbook = RESTOCK_RUNBOOK,
            "tacsim: the clerk's restock of a milled ask was refused"
        ),
        MillWarn::Stalled => tracing::warn!(
            vault, ore = %key,
            "tacsim: a mill's batch never lands; the clerk passes it over for the rest of the IC \
             day (@tacsim/port status)"
        ),
    }
}

/// Whether a mill's act was refused today.
fn refused_today(
    state: &PortTickState,
    vault: u64,
    ore: &CanonicalId,
    act: MillAct,
    today: i64,
) -> bool {
    state
        .mill_refused
        .get(&(vault, ore.clone(), act))
        .is_some_and(|day| *day == today)
}

/// Pass a mill's act over for the rest of today (stale days pruned first, so the bound
/// holds the live refusals).
fn refuse_today(
    state: &mut PortTickState,
    vault: u64,
    ore: &CanonicalId,
    act: MillAct,
    today: i64,
) {
    state.mill_refused.retain(|_, day| *day == today);
    let key = (vault, ore.clone(), act);
    if state.mill_refused.contains_key(&key) || state.mill_refused.len() < MAX_CLERK_ROWS {
        state.mill_refused.insert(key, today);
    }
}

/// **The mill's turn of the stock arm** (doc 20 cut SM.12a; §7.3 "The mill"; D4, D7, D8):
/// after the intake drip and before the lapsed-bid re-post, so behind the emergency bids
/// and the ask list step too. Two passes, in authored order (vendors, then their mill
/// rows), and **the first due act acts — one act a fire**:
///
/// - **(a) a restock first**, so each batch's units are listed before the next lands: a
///   mill with units milled and not yet listed ([`owed`]) whose good has a live drawn ask
///   raises that ask by exactly those units through `mush_market::restock_ask` as the
///   clerk, and the vendor row's `mill_listed` counter rises by the same units in the same
///   handler outbox (`put_port_map`; the next map computed before the listing write). No
///   live listing (sold out, or lapsed and not yet swept): nothing — the list step lists
///   the whole pool and sets the counters. A Draw in flight on the pool: the restock waits
///   for it to settle or lapse, as the list step does — a delay, never a loss (the lead's
///   ruling on SM.12a's conformance F2; the module doc's "The restock's wait").
/// - **(b) then a batch**: a mill whose storefront ore pool (less holds) holds a batch and
///   whose IC day has room submits `tacsim_port_mill` as the clerk for the ledger's next
///   ordinal.
///
/// A refused act passes its mill's act over for the rest of the IC day
/// (`PortTickState::mill_refused`), warned once a boot; a batch submitted
/// [`MAX_MILL_ATTEMPTS`] times with its ledger not advancing is passed over likewise. No
/// refusal returns from the stock arm early (D8). `None` when nothing is due — the stock
/// arm goes on to the bids.
#[allow(clippy::too_many_lines)]
pub(crate) fn mill_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    def: &PortDef,
) -> Option<Step> {
    if def.vendors.iter().all(|vendor| vendor.mill.is_empty()) {
        return None;
    }
    let mills = ctx
        .w
        .module_state::<PortHandles>()
        .map(|handles| handles.families.mills.clone())?;
    let now = ctx.w.now();
    let today = ic_day(ctx.w.world_ic_now().at);
    // (a) A restock.
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        if row.vendor.is_none() {
            continue;
        }
        for mill in &vendor.mill {
            if refused_today(state, row.vault, &mill.ore, MillAct::Restock, today) {
                continue;
            }
            let (good, units) = match owed(ctx, &mills, map, vendor, row, mill) {
                Err(error) => {
                    mill_warn(state, row.vault, error.ore(), error.warn(), "");
                    refuse_today(state, row.vault, &mill.ore, MillAct::Restock, today);
                    continue;
                }
                Ok(None | Some((_, 0))) => continue,
                Ok(Some(owed)) => owed,
            };
            // A lapsed listing the sweep has not deleted yet reads as absent (12a-M6):
            // the sweep and the list step heal it. And **while any Draw quote on the pool
            // is unsettled the restock is deferred — a delay, never a loss** (the lead's
            // ruling on SM.12a's conformance F2, the builder's form): a Draw's goods leave
            // the pool when its leg lands, but its quote reads drawn only once the market's
            // sweep mirrors the leg, so in between the backing check would count those
            // units twice and refuse a restock the pool does back. No warning, no refused
            // mark: the units wait as "milled and not yet listed" on a Ready Listings line,
            // and the first fire after the quote settles or lapses restocks them.
            let endpoint = EndpointRef(VaultId(row.vault));
            let listing = ctx.w.module_state::<MarketStore>().and_then(|market| {
                if market.draw_reading(endpoint, &good).1 {
                    return None;
                }
                market.lot_listing_of(endpoint, &good).filter(|id| {
                    market
                        .listing(*id)
                        .is_some_and(|listing| !mush_market::listing_lapsed(listing, now))
                })
            });
            let Some(listing) = listing else {
                continue;
            };
            // 12a-M7: the next map before any write, so a counter overflow writes nothing.
            let mut next = map.clone();
            let counted = next.vendors.get_mut(&vendor.org).and_then(|next_row| {
                let counter = next_row.mill_listed.entry(mill.ore.clone()).or_insert(0);
                *counter = counter.checked_add(units)?;
                Some(())
            });
            if counted.is_none() {
                mill_warn(state, row.vault, &mill.ore, MillWarn::Counter, "");
                refuse_today(state, row.vault, &mill.ore, MillAct::Restock, today);
                continue;
            }
            let spec = mush_market::RestockSpec {
                listing,
                good,
                units,
            };
            return Some(
                match fire
                    .as_principal(ctx, clerk, clerk, |ctx| mush_market::restock_ask(ctx, spec))
                {
                    Ok(Ok(_)) => {
                        // One outbox (D1): the listing above, the counter here.
                        crate::store::put_port_map(ctx, &next);
                        state
                            .mill_refused
                            .remove(&(row.vault, mill.ore.clone(), MillAct::Restock));
                        Step::Worked
                    }
                    Ok(Err(error)) => {
                        let detail = format!("{error:?}");
                        mill_warn(state, row.vault, &mill.ore, MillWarn::Refused, &detail);
                        refuse_today(state, row.vault, &mill.ore, MillAct::Restock, today);
                        Step::Refused
                    }
                    Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
                        Step::Deferred
                    }
                    Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
                },
            );
        }
    }
    // (b) A batch.
    let action = state.mill_action.clone()?;
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        let endpoint = EndpointRef(VaultId(row.vault));
        for mill in &vendor.mill {
            if refused_today(state, row.vault, &mill.ore, MillAct::Batch, today) {
                continue;
            }
            let Ok(id) = MillId::derive(&map.pack, &map.entry, &vendor.org, &mill.ore) else {
                mill_warn(state, row.vault, &mill.ore, MillWarn::Unreadable, "");
                refuse_today(state, row.vault, &mill.ore, MillAct::Batch, today);
                continue;
            };
            let ledger = match ledger_at(ctx, &mills, &id) {
                Ok(ledger) => ledger,
                Err(()) => {
                    mill_warn(state, row.vault, &mill.ore, MillWarn::Unreadable, "");
                    refuse_today(state, row.vault, &mill.ore, MillAct::Batch, today);
                    continue;
                }
            };
            // D9: a mill's ratio and product are fixed for its ledger's life.
            if ledger
                .as_ref()
                .is_some_and(|ledger| ledger.ratio != mill.ratio || ledger.into != mill.into)
            {
                mill_warn(state, row.vault, &mill.ore, MillWarn::Terms, "");
                refuse_today(state, row.vault, &mill.ore, MillAct::Batch, today);
                continue;
            }
            // The pool, not the ledger: its own warning (keyed and worded apart), and no
            // refused mark, so the batch is tried again next fire.
            let Ok(available) = crate::port_tick::read_available(ctx, endpoint, &mill.ore) else {
                mill_warn(state, row.vault, &mill.ore, MillWarn::Storefront, "");
                continue;
            };
            let room = room_today(
                ledger.as_ref().map(|ledger| (ledger.day, ledger.day_ore_t)),
                today,
                mill.per_ic_day_t,
            );
            if batch(available, mill.batch_t, mill.ratio, room).is_none() {
                continue;
            }
            let Some(ordinal) = ledger
                .as_ref()
                .map_or(Some(1), |ledger| ledger.batches.checked_add(1))
            else {
                continue;
            };
            // The attempt rule (lead ruling L24's, the stock mint's shape): an advanced
            // ledger clears the entry; the same ordinal submitted `MAX_MILL_ATTEMPTS` times
            // with nothing in flight passes the mill's batch over for the rest of the day.
            let attempt_key = (row.vault, mill.ore.clone());
            let attempts = match state.mill_attempts.get(&attempt_key) {
                Some((recorded, attempts)) if *recorded == ordinal => *attempts,
                Some(_) => {
                    state.mill_attempts.remove(&attempt_key);
                    0
                }
                None => 0,
            };
            if attempts >= MAX_MILL_ATTEMPTS && !ctx.authoritative_pending(clerk) {
                state.mill_attempts.remove(&attempt_key);
                refuse_today(state, row.vault, &mill.ore, MillAct::Batch, today);
                mill_warn(state, row.vault, &mill.ore, MillWarn::Stalled, "");
                continue;
            }
            let key_text = id.key_text();
            let Ok(key) = IdempotencyKeyText::parse_canonical(format!(
                "{MILL_KEY_PREFIX}{key_text}/{ordinal}"
            )) else {
                return Some(Step::Refused);
            };
            let parameters = json!({ "mill": key_text, "ordinal": ordinal });
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
                && (state.mill_attempts.contains_key(&attempt_key)
                    || state.mill_attempts.len() < MAX_CLERK_ROWS)
            {
                state
                    .mill_attempts
                    .insert(attempt_key, (ordinal, attempts.saturating_add(1)));
            }
            return Some(step);
        }
    }
    None
}

// ---- readiness: the Mill line --------------------------------------------------------

/// **What the readiness report read of one mill** (doc 20 cut SM.12a; the
/// `refresh_bid_line` shape, so each line is tested without a world).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MillLineRead {
    /// The vendor has no map row yet.
    Unseeded,
    /// The ledger cannot be read, or the counter is past it.
    Unreadable,
    /// The authored ratio no longer matches the ledger's.
    Ratio,
    /// The authored product no longer matches the ledger's.
    Product,
    /// The storefront's ore pool cannot be read.
    Storefront,
    /// Nothing milled yet.
    Nothing {
        /// Tonnes of ore in the storefront.
        waiting: u64,
    },
    /// The ledger's figures.
    Milled {
        /// Batches milled.
        batches: u64,
        /// Tonnes of ore milled.
        ore_t: u64,
        /// Tonnes of product made.
        product_t: u64,
        /// The product.
        into: CanonicalId,
        /// The last batch's IC instant.
        last_at_ic: i64,
        /// Tonnes of ore in the storefront.
        waiting: u64,
    },
}

/// **One mill's readiness line** (doc 20 cut SM.12a; O-2b's staff texts): reported, never
/// gated (`Binding::Mill`) — a mill's fault closes the desk only through the gated Listings
/// line of the good it makes (D15). Pure.
pub(crate) fn mill_line_text(
    entry: &CanonicalId,
    org: &CanonicalId,
    mill: &MillDef,
    read: &MillLineRead,
) -> (LineState, String) {
    let head = format!("{entry} mill {org} {}", mill.ore);
    let terms = format!(
        "{} to {}, {} t a batch, at most {} t an IC day",
        mill.ratio.ore, mill.ratio.product, mill.batch_t, mill.per_ic_day_t
    );
    match read {
        MillLineRead::Unseeded => (LineState::Pending, format!("{head}: not seeded yet.")),
        MillLineRead::Unreadable => (
            LineState::Flagged,
            format!("{head}: its ledger cannot be read."),
        ),
        MillLineRead::Ratio => (
            LineState::Flagged,
            format!("{head}: its ratio no longer matches its ledger."),
        ),
        MillLineRead::Product => (
            LineState::Flagged,
            format!("{head}: its product no longer matches its ledger."),
        ),
        MillLineRead::Storefront => (
            LineState::Flagged,
            format!("{head}: its storefront cannot be read."),
        ),
        MillLineRead::Nothing { waiting } => (
            LineState::Ready,
            format!("{head}: nothing milled yet; {waiting} t of ore waits; {terms}."),
        ),
        MillLineRead::Milled {
            batches,
            ore_t,
            product_t,
            into,
            last_at_ic,
            waiting,
        } => {
            let counted = crate::port::tv::count_words(*batches, "batch", "batches");
            let has = if *batches == 1 { "has" } else { "have" };
            let stamp = crate::port::licence::ic_stamp(*last_at_ic);
            (
                LineState::Ready,
                format!(
                    "{head}: {counted} {has} milled {ore_t} t of ore into {product_t} t of \
                     {into}, the last at {stamp}; {waiting} t of ore waits; {terms}."
                ),
            )
        }
    }
}

/// **One mill's readiness line, read** (doc 20 cut SM.12a): no map row → not seeded; the
/// ledger unreadable, or its counter past it → its ledger; a ratio, then a product, that
/// no longer matches the ledger; the storefront's ore pool unreadable (`read_pool`, any
/// error, `Unwired` too); no ledger → nothing milled yet; else the ledger's figures.
/// `waiting` is the storefront's ore pool (`read_pool`, the raw balance).
pub(crate) fn mill_line(
    ctx: &Ctx<'_>,
    entry: &CanonicalId,
    map: &PortMap,
    vendor: &VendorDef,
    mill: &MillDef,
) -> (LineState, String) {
    let read = mill_line_read(ctx, map, vendor, mill);
    mill_line_text(entry, &vendor.org, mill, &read)
}

fn mill_line_read(
    ctx: &Ctx<'_>,
    map: &PortMap,
    vendor: &VendorDef,
    mill: &MillDef,
) -> MillLineRead {
    let Some(row) = map.vendors.get(&vendor.org) else {
        return MillLineRead::Unseeded;
    };
    let Some(mills) = ctx
        .w
        .module_state::<PortHandles>()
        .map(|handles| handles.families.mills.clone())
    else {
        return MillLineRead::Unreadable;
    };
    let Ok(id) = MillId::derive(&map.pack, &map.entry, &vendor.org, &mill.ore) else {
        return MillLineRead::Unreadable;
    };
    let Ok(ledger) = ledger_at(ctx, &mills, &id) else {
        return MillLineRead::Unreadable;
    };
    if matches!(
        owed(ctx, &mills, map, vendor, row, mill),
        Err(MillRead::Counter { .. })
    ) {
        return MillLineRead::Unreadable;
    }
    if let Some(ledger) = &ledger {
        if ledger.ratio != mill.ratio {
            return MillLineRead::Ratio;
        }
        if ledger.into != mill.into {
            return MillLineRead::Product;
        }
    }
    let Ok(waiting) = crate::port_tick::read_pool(ctx, EndpointRef(VaultId(row.vault)), &mill.ore)
    else {
        return MillLineRead::Storefront;
    };
    match ledger {
        None => MillLineRead::Nothing { waiting },
        Some(ledger) => MillLineRead::Milled {
            batches: ledger.batches,
            ore_t: ledger.ore_t,
            product_t: ledger.product_t,
            last_at_ic: ledger.recent.last().map_or(0, |last| last.at_ic),
            into: ledger.into,
            waiting,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap()
    }

    const FIVE_TWO: MillRatio = MillRatio { ore: 5, product: 2 };

    /// **Doc 20 cut SM.12a**: one whole batch or none, the remainder kept in the pool.
    #[test]
    fn sm12a_a_batch_is_whole_and_keeps_the_remainder() {
        assert_eq!(
            batch(47, 20, FIVE_TWO, 40),
            Some(Milled {
                ore_t: 20,
                product_t: 8,
                remainder_t: 27
            })
        );
        assert_eq!(
            batch(20, 20, FIVE_TWO, 20),
            Some(Milled {
                ore_t: 20,
                product_t: 8,
                remainder_t: 0
            })
        );
        assert_eq!(batch(19, 20, FIVE_TWO, 40), None, "less than a batch waits");
        assert_eq!(batch(47, 20, FIVE_TWO, 19), None, "the day's room binds");
        assert_eq!(batch(47, 21, FIVE_TWO, 40), None, "a batch is whole ratios");
        assert_eq!(batch(47, 0, FIVE_TWO, 40), None);
        assert_eq!(batch(47, 20, MillRatio { ore: 0, product: 0 }, 40), None);
        assert_eq!(
            batch(u64::MAX, 100, MillRatio { ore: 1, product: 1 }, u64::MAX),
            Some(Milled {
                ore_t: 100,
                product_t: 100,
                remainder_t: u64::MAX - 100
            })
        );
    }

    /// **Doc 20 cut SM.12a**: the IC day is the drip's, and the room resets at its boundary.
    #[test]
    fn sm12a_room_today_resets_at_the_ic_day_boundary() {
        assert_eq!(ic_day(-1), -1);
        assert_eq!(ic_day(0), 0);
        assert_eq!(ic_day(86_399), 0);
        assert_eq!(ic_day(86_400), 1);
        assert_eq!(room_today(None, 5, 40), 40);
        assert_eq!(room_today(Some((4, 40)), 5, 40), 40, "yesterday's tally");
        assert_eq!(room_today(Some((5, 20)), 5, 40), 20);
        assert_eq!(room_today(Some((5, 40)), 5, 40), 0);
        assert_eq!(room_today(Some((5, 60)), 5, 40), 0, "never below zero");
        assert_eq!(room_today(Some((6, 20)), 5, 40), 20, "a set-back clock");
    }

    /// **Doc 20 cut SM.12a**: each warning kind is its own key, so an unreadable storefront
    /// pool never hides a later ledger fault of the same mill (each logged once a boot),
    /// and the set stays within the clerk's bound.
    #[test]
    fn sm12a_each_mill_warning_is_keyed_apart() {
        let mut state = PortTickState::default();
        let ore = cid("metal_ore");
        mill_warn(&mut state, 7, &ore, MillWarn::Storefront, "");
        mill_warn(&mut state, 7, &ore, MillWarn::Unreadable, "");
        mill_warn(&mut state, 7, &ore, MillWarn::Unreadable, "");
        assert_eq!(
            state.mill_warned.iter().cloned().collect::<Vec<_>>(),
            vec![
                (7, ore.clone(), MillWarn::Unreadable),
                (7, ore.clone(), MillWarn::Storefront),
            ]
        );
        for vault in 0..u64::try_from(MAX_CLERK_ROWS).unwrap() * 2 {
            mill_warn(&mut state, vault + 100, &ore, MillWarn::Stalled, "");
        }
        assert_eq!(state.mill_warned.len(), MAX_CLERK_ROWS, "bounded");
    }

    /// **Doc 20 cut SM.12a**: the month as pure arithmetic — 30 IC days of a fixed inflow
    /// table, at most one batch a call, the rate per day; after every step the totals
    /// conserve, the inflow less the pool is the ore milled, and no day mills past 40 t.
    #[test]
    fn sm12a_a_simulated_month_conserves_as_pure_arithmetic() {
        const INFLOW: [u64; 30] = [
            0, 19, 1, 47, 60, 40, 12, 33, 55, 0, 20, 20, 41, 7, 58, 38, 0, 25, 44, 16, 60, 3, 39,
            52, 18, 0, 47, 31, 22, 40,
        ];
        let (batch_t, per_day) = (20, 40);
        let (mut pool, mut inflow, mut ore, mut product) = (0_u64, 0_u64, 0_u64, 0_u64);
        let mut tally: Option<(i64, u64)> = None;
        for (day, sold) in (0_i64..).zip(INFLOW) {
            pool += sold;
            inflow += sold;
            let mut today_ore = 0;
            // Many fires a day; each mills at most one batch.
            for _ in 0..720 {
                let room = room_today(tally, day, per_day);
                let Some(milled) = batch(pool, batch_t, FIVE_TWO, room) else {
                    continue;
                };
                pool = milled.remainder_t;
                ore += milled.ore_t;
                product += milled.product_t;
                today_ore += milled.ore_t;
                tally = Some(match tally {
                    Some((d, t)) if d == day => (d, t + milled.ore_t),
                    _ => (day, milled.ore_t),
                });
                assert!(conserves(ore, product, FIVE_TWO));
                assert_eq!(inflow - pool, ore);
            }
            assert!(today_ore <= per_day, "day {day} milled {today_ore}");
        }
        // The month's figures, pinned.
        assert_eq!(inflow, 848);
        assert_eq!(ore, 840);
        assert_eq!(product, 336);
        assert_eq!(pool, 8);
    }

    /// The golden mill id later cuts must not move (doc 20 cut SM.12a):
    /// `("tacsim.lane.ceres_phobos", "ceres_port", "freeport_exchange", "metal_ore")`.
    const GOLDEN_MILL: &str = "e84b53bacd9be0e7bf01d9d080afc719c84f1c03f0c91fd97fb6816516e0850e";

    /// **Doc 20 cut SM.12a**: the mill id is golden, each part moves it, and `parse` is
    /// strict.
    #[test]
    fn sm12a_the_mill_id_is_golden() {
        let parts = [
            "tacsim.lane.ceres_phobos",
            "ceres_port",
            "freeport_exchange",
            "metal_ore",
        ];
        let derive = |parts: [&str; 4]| {
            MillId::derive(
                &cid(parts[0]),
                &cid(parts[1]),
                &cid(parts[2]),
                &cid(parts[3]),
            )
            .unwrap()
        };
        let id = derive(parts);
        println!("    const GOLDEN_MILL: &str = \"{}\";", id.key_text());
        assert_eq!(id.key_text(), GOLDEN_MILL);
        assert_eq!(id.key_text().len(), 64);
        for slot in 0..4 {
            let mut moved = parts;
            moved[slot] = "other";
            assert_ne!(derive(moved), id, "part {slot} moves the id");
        }
        assert_eq!(MillId::parse(&id.key_text()), Some(id));
        assert_eq!(MillId::parse(&id.key_text().to_uppercase()), None);
        assert_eq!(MillId::parse(&id.key_text()[1..]), None);
        assert_eq!(MillId::parse("not hex"), None);
    }

    /// A one-batch Exchange ledger at IC instant `at`.
    fn ledger(at: i64) -> MillLedger {
        MillLedger {
            pack: cid("tacsim.lane.ceres_phobos"),
            entry: cid("ceres_port"),
            org: cid("freeport_exchange"),
            ore: cid("metal_ore"),
            into: cid("metals"),
            ratio: FIVE_TWO,
            vault: VaultId(7),
            clerk: ObjId(40),
            batches: 1,
            ore_t: 20,
            product_t: 8,
            day: ic_day(at),
            day_ore_t: 20,
            recent: vec![MillBatch {
                ordinal: 1,
                at_ic: at,
                ore_t: 20,
                product_t: 8,
            }],
        }
    }

    /// **Doc 20 cut SM.12a**: the ledger's wire round-trips, is strict at every level, and
    /// `check` refuses each broken invariant.
    #[test]
    fn sm12a_the_ledger_wire_is_strict_and_checks() {
        let good = ledger(100_000);
        assert_eq!(good.check(), Ok(()));
        let json = serde_json::to_value(&good).unwrap();
        assert_eq!(
            serde_json::from_value::<MillLedger>(json.clone()).unwrap(),
            good
        );
        let mut extra = json.clone();
        extra["x"] = json!(1);
        assert!(serde_json::from_value::<MillLedger>(extra).is_err());
        let mut extra = json.clone();
        extra["ratio"]["x"] = json!(1);
        assert!(serde_json::from_value::<MillLedger>(extra).is_err());
        let mut extra = json;
        extra["recent"][0]["x"] = json!(1);
        assert!(serde_json::from_value::<MillLedger>(extra).is_err());
        assert_eq!(
            good.mill_id(),
            MillId::derive(&good.pack, &good.entry, &good.org, &good.ore)
        );

        let broken = |edit: &dyn Fn(&mut MillLedger)| {
            let mut ledger = ledger(100_000);
            edit(&mut ledger);
            ledger
                .check()
                .err()
                .unwrap_or_else(|| "accepted".to_owned())
        };
        assert!(broken(&|l| l.batches = 0).contains("at least one batch"));
        assert!(broken(&|l| l.ratio = MillRatio { ore: 2, product: 5 }).contains("ratio"));
        assert!(
            broken(&|l| l.ratio = MillRatio {
                ore: 101,
                product: 2
            })
            .contains("ratio")
        );
        assert!(broken(&|l| l.product_t = 9).contains("totals conserve"));
        assert!(broken(&|l| l.recent.clear()).contains("keeps its last batch"));
        assert!(
            broken(&|l| {
                l.batches = 17;
                l.ore_t = 340;
                l.product_t = 136;
                l.recent = (1..=17)
                    .map(|ordinal| MillBatch {
                        ordinal,
                        at_ic: 100_000,
                        ore_t: 20,
                        product_t: 8,
                    })
                    .collect();
            })
            .contains("at most 16")
        );
        assert!(broken(&|l| l.recent[0].ordinal = 2).contains("ascend by one"));
        assert!(
            broken(&|l| {
                l.batches = 2;
                l.ore_t = 40;
                l.product_t = 16;
                l.recent.push(MillBatch {
                    ordinal: 3,
                    at_ic: 100_000,
                    ore_t: 20,
                    product_t: 8,
                });
            })
            .contains("ascend by one")
        );
        assert!(
            broken(&|l| {
                l.batches = 0;
                l.recent[0].ordinal = 0;
            })
            .contains("at least one batch")
        );
        assert!(
            broken(&|l| {
                l.recent[0].ore_t = 0;
                l.recent[0].product_t = 0;
            })
            .contains("mills ore")
        );
        assert!(broken(&|l| l.recent[0].product_t = 9).contains("conserves"));
        assert!(
            broken(&|l| {
                l.recent[0].ore_t = 25;
                l.recent[0].product_t = 10;
            })
            .contains("sum within its total")
        );
        assert!(broken(&|l| l.day += 1).contains("last batch's"));
        assert!(broken(&|l| l.day_ore_t = 25).contains("day tally"));
        assert!(broken(&|l| l.day_ore_t = 19).contains("day tally"));
        // A two-batch ledger across a day boundary: the tally is the last day's.
        let mut two = ledger(100_000);
        two.batches = 2;
        two.ore_t = 40;
        two.product_t = 16;
        two.day = ic_day(200_000);
        two.recent.push(MillBatch {
            ordinal: 2,
            at_ic: 200_000,
            ore_t: 20,
            product_t: 8,
        });
        assert_eq!(two.check(), Ok(()));
    }

    /// **Doc 20 cut SM.12a (O-2b)**: every Mill line, whole text and state.
    #[test]
    fn sm12a_each_mill_line_says_what_it_reads() {
        let mill = MillDef {
            ore: cid("metal_ore"),
            into: cid("metals"),
            ratio: FIVE_TWO,
            batch_t: 20,
            per_ic_day_t: 40,
        };
        let line = |read: MillLineRead| {
            mill_line_text(&cid("ceres_port"), &cid("freeport_exchange"), &mill, &read)
        };
        let head = "ceres_port mill freeport_exchange metal_ore";
        assert_eq!(
            line(MillLineRead::Unseeded),
            (LineState::Pending, format!("{head}: not seeded yet."))
        );
        assert_eq!(
            line(MillLineRead::Unreadable),
            (
                LineState::Flagged,
                format!("{head}: its ledger cannot be read.")
            )
        );
        assert_eq!(
            line(MillLineRead::Ratio),
            (
                LineState::Flagged,
                format!("{head}: its ratio no longer matches its ledger.")
            )
        );
        assert_eq!(
            line(MillLineRead::Product),
            (
                LineState::Flagged,
                format!("{head}: its product no longer matches its ledger.")
            )
        );
        assert_eq!(
            line(MillLineRead::Storefront),
            (
                LineState::Flagged,
                format!("{head}: its storefront cannot be read.")
            )
        );
        assert_eq!(
            line(MillLineRead::Nothing { waiting: 19 }),
            (
                LineState::Ready,
                format!(
                    "{head}: nothing milled yet; 19 t of ore waits; 5 to 2, 20 t a batch, at most \
                     40 t an IC day."
                )
            )
        );
        let at = 86_400 * 40 + 3_600 * 13 + 60 * 7;
        let stamp = crate::port::licence::ic_stamp(at);
        assert_eq!(
            line(MillLineRead::Milled {
                batches: 1,
                ore_t: 20,
                product_t: 8,
                into: cid("metals"),
                last_at_ic: at,
                waiting: 0,
            }),
            (
                LineState::Ready,
                format!(
                    "{head}: 1 batch has milled 20 t of ore into 8 t of metals, the last at \
                     {stamp}; 0 t of ore waits; 5 to 2, 20 t a batch, at most 40 t an IC day."
                )
            )
        );
        assert_eq!(
            line(MillLineRead::Milled {
                batches: 2,
                ore_t: 40,
                product_t: 16,
                into: cid("metals"),
                last_at_ic: at,
                waiting: 7,
            }),
            (
                LineState::Ready,
                format!(
                    "{head}: 2 batches have milled 40 t of ore into 16 t of metals, the last at \
                     {stamp}; 7 t of ore waits; 5 to 2, 20 t a batch, at most 40 t an IC day."
                )
            )
        );
    }
}
