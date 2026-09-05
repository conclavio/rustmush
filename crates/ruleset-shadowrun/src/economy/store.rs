//! The E6a module state and its documents (namespace `shadowrun`).
//!
//! | doc | class | corrupt → |
//! |---|---|---|
//! | `sr:profile:{obj}` | container | **skipped loudly**; the character leaves the economic layer until `@sr/migrate commit` re-adopts them |
//! | `sr:run:{id}` | container | **skipped loudly**; the *contract* is untouched, so the escrow, the consent and the settlement all survive — only the board entry is lost |
//! | `sr:meta` | allocators | tolerated field by field, floors re-derived from what loaded |
//!
//! Neither container fails the boot, and the direction of each loss is deliberate.
//! A profile is re-derivable: the account is discoverable from the economy, the
//! lifestyle obligation self-identifies by its cause (DL-07), and `@sr/migrate` is
//! the reconciliation verb that adopts what boot found. A listing is a *projection*
//! of a contract that still exists: losing it costs a board row, and the payer can
//! still `@contract/cancel` and be refunded. Failing the boot instead would take the
//! whole world down for a row the board can rebuild.
//!
//! Both loaders apply the **identity rule** (DL-07): a document whose record
//! disagrees with its own key is dropped loudly, never trusted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_core::{Ctx, ModuleOp, ModuleWrite, ObjId, Outbox};
use mush_economy::{AccountId, CadenceId, ContractId, InstrumentId, ObligationId};
use smol_str::SmolStr;

use crate::economy::config::{MAX_QUOTES, RECENT_CLOSED_RING, SrEconomyConfig};
use crate::economy::fence_record::{FenceId, FenceQuote, QuoteId, SrFence};
use crate::economy::record::{RunId, RunTemplate, SrProfile};

/// The module namespace (the ruleset module's own name).
pub const NS: &str = "shadowrun";
/// The E6a allocator document.
pub const META_KEY: &str = "sr:meta";
/// The board sweep's tick tag.
pub const TICK_TAG: &str = "sr.economy";
/// The board sweep's wall period. A **latency** bound only: what is due is decided
/// by the contract's own deadline, never by this rate.
pub const TICK_PERIOD_SECS: u64 = 60;
/// The most listings one board-sweep fire may reconcile.
pub const MAX_RUNS_PER_FIRE: usize = 64;

/// `sr:profile:{obj}`.
#[must_use]
pub fn profile_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("sr:profile:{}", obj.0))
}

/// `sr:run:{id}`.
#[must_use]
pub fn run_key(id: RunId) -> SmolStr {
    SmolStr::new(format!("sr:run:{}", id.0))
}

/// `sr:fence:{id}` (E6b, B9).
#[must_use]
pub fn fence_key(id: FenceId) -> SmolStr {
    SmolStr::new(format!("sr:fence:{}", id.0))
}

/// `sr:quote:{id}` (E6b, B9).
#[must_use]
pub fn quote_key(id: QuoteId) -> SmolStr {
    SmolStr::new(format!("sr:quote:{}", id.0))
}

/// The in-RAM E6a slot — the single runtime source of truth.
#[derive(Clone, Debug, Default)]
pub struct SrEconomyStore {
    /// Frozen at wiring.
    pub(crate) cfg: SrEconomyConfig,
    pub(crate) profiles: BTreeMap<ObjId, SrProfile>,
    pub(crate) runs: BTreeMap<RunId, RunTemplate>,
    /// **contract → listing**, re-derived from the runs on every write, so the board
    /// observer answers a `ContractChanged` without scanning the board.
    pub(crate) by_contract: BTreeMap<ContractId, RunId>,
    /// The ids of every listing that has **not** closed — the whole working set of
    /// the caps, the board page and the per-party rule (E6a verify fold, D1).
    ///
    /// The `mush-party` `live` shape: derived from the state on every write and
    /// re-derived at boot, never authority (the state on the record is). A closed
    /// listing leaves it *and* leaves the store — see [`del_run`] — so a
    /// post/withdraw loop mints no permanent documents.
    pub(crate) live: BTreeSet<RunId>,
    /// **upkeep obligation → character** (E6a verify fold, D7). The lifestyle
    /// observer resolves a subject per event, one event per profiled character per
    /// period, and a `profiles().find(..)` there is quadratic in the population.
    pub(crate) by_obligation: BTreeMap<ObligationId, ObjId>,
    /// **profile account → character** (E6a verify fold, D7). The other half of the
    /// same lookup: a record self-identifies by its debit account (DL-07).
    pub(crate) by_account: BTreeMap<AccountId, ObjId>,
    /// The last [`RECENT_CLOSED_RING`] listing ids to close, newest last
    /// (E6a verify fold, D1).
    ///
    /// A closed listing's document is deleted, so `+run/show` on one would otherwise
    /// answer "No such run." the instant it settled. This ring is the grace period,
    /// and it costs an id rather than a record. It is **not** persisted: after a
    /// reboot a closed run is simply gone, which is the same answer a week later.
    pub(crate) recent_closed: VecDeque<RunId>,
    pub(crate) next_run: u64,
    // ---- E6b: B9 -- the fence's two families ----
    /// `sr:fence:{id}` -- the registered vendor organizations.
    pub(crate) fences: BTreeMap<FenceId, SrFence>,
    /// **organization to fence**, re-derived on every write: one fence per org.
    pub(crate) by_org: BTreeMap<mush_org::OrgId, FenceId>,
    /// `sr:quote:{id}` -- the open offers. A quote whose deal ended is `Del`'d, the
    /// same trade `del_run` makes for a closed listing.
    pub(crate) quotes: BTreeMap<QuoteId, FenceQuote>,
    /// **contract to quote**, re-derived on every write, so the board observer
    /// answers a `ContractChanged` without scanning the quotes.
    pub(crate) by_quote_contract: BTreeMap<ContractId, QuoteId>,
    pub(crate) next_fence: u64,
    pub(crate) next_quote: u64,
    /// The registered instrument, once boot has found or made it.
    pub(crate) instrument: Option<InstrumentId>,
    /// The resolved `sr.month` cadence, once boot has found it.
    pub(crate) cadence: Option<CadenceId>,
}

impl SrEconomyStore {
    /// The frozen configuration.
    #[must_use]
    pub fn config(&self) -> &SrEconomyConfig {
        &self.cfg
    }
    /// One profile.
    #[must_use]
    pub fn profile(&self, obj: ObjId) -> Option<&SrProfile> {
        self.profiles.get(&obj)
    }
    /// Every profile, in object-id order.
    pub fn profiles(&self) -> impl Iterator<Item = &SrProfile> {
        self.profiles.values()
    }
    /// One listing.
    #[must_use]
    pub fn run(&self, id: RunId) -> Option<&RunTemplate> {
        self.runs.get(&id)
    }
    /// Every listing, in id order.
    pub fn runs(&self) -> impl Iterator<Item = &RunTemplate> {
        self.runs.values()
    }
    /// Every listing that has not closed, in id order — the board's whole working set
    /// (D1).
    pub fn live_runs(&self) -> impl Iterator<Item = &RunTemplate> {
        self.live.iter().filter_map(|id| self.runs.get(id))
    }
    /// The ids of every listing that has not closed, in id order.
    pub fn live_ids(&self) -> impl Iterator<Item = RunId> + '_ {
        self.live.iter().copied()
    }
    /// The same, newest first — the board page's walk order (D1).
    pub fn live_ids_desc(&self) -> impl Iterator<Item = RunId> + '_ {
        self.live.iter().rev().copied()
    }
    /// Whether `id` closed recently enough that `+run/show` still answers "closed"
    /// rather than hiding it (D1).
    #[must_use]
    pub fn recently_closed(&self, id: RunId) -> bool {
        self.recent_closed.contains(&id)
    }
    /// The listing that mirrors `contract`, if any.
    #[must_use]
    pub fn run_of_contract(&self, contract: ContractId) -> Option<&RunTemplate> {
        self.by_contract.get(&contract).and_then(|id| self.run(*id))
    }
    /// The character whose profile names `obligation` as its upkeep, or whose profile
    /// account is `account` — the lifestyle observer's O(log n) subject lookup (D7).
    #[must_use]
    pub fn owner_of_record(&self, obligation: ObligationId, account: AccountId) -> Option<ObjId> {
        self.by_obligation
            .get(&obligation)
            .or_else(|| self.by_account.get(&account))
            .copied()
    }
    /// The registered instrument, once boot has resolved it.
    #[must_use]
    pub fn instrument(&self) -> Option<&InstrumentId> {
        self.instrument.as_ref()
    }
    /// The resolved `sr.month` cadence, once boot has found it.
    #[must_use]
    pub fn cadence(&self) -> Option<CadenceId> {
        self.cadence
    }
    /// How many listings `principal` currently holds open (§9's per-principal cap).
    #[must_use]
    pub fn open_runs_of(&self, principal: &mush_org::PrincipalId) -> usize {
        self.live_runs()
            .filter(|r| r.principal == *principal)
            .count()
    }
    /// How many listings are open at all (§9's `BOARD_CAP`).
    #[must_use]
    pub fn open_runs(&self) -> usize {
        self.live.len()
    }
    /// Whether `party` already holds a live run (§11 q5: at most one).
    #[must_use]
    pub fn party_has_live_run(&self, party: u64) -> bool {
        self.live_runs().any(|r| r.party == Some(party))
    }
    /// Whether every derived index agrees with the records (E6a verify fold, D7/D1).
    ///
    /// A test-and-`debug_assert` predicate, not a runtime gate: the indexes are
    /// maintained in exactly two write paths and re-derived at boot, and this is what
    /// says so out loud.
    #[must_use]
    pub fn indexes_agree(&self) -> bool {
        let live_ok = self
            .runs
            .values()
            .all(|r| self.live.contains(&r.id) != r.state.is_closed());
        let contract_ok = self
            .runs
            .values()
            .all(|r| self.by_contract.get(&r.contract) == Some(&r.id))
            && self.by_contract.len() == self.runs.len();
        let obl_ok = self.profiles.values().all(|p| {
            p.upkeep
                .is_none_or(|o| self.by_obligation.get(&o) == Some(&p.obj))
        }) && self.by_obligation.len()
            == self
                .profiles
                .values()
                .filter(|p| p.upkeep.is_some())
                .count();
        let acct_ok = self.profiles.values().all(|p| {
            p.account
                .is_none_or(|a| self.by_account.get(&a) == Some(&p.obj))
        }) && self.by_account.len()
            == self
                .profiles
                .values()
                .filter(|p| p.account.is_some())
                .count();
        live_ok && contract_ok && obl_ok && acct_ok
    }

    /// Mint the next listing id, or `None` at the ceiling (E4b1's B2 rule: an
    /// exhausted allocator refuses one operation loudly, it never wraps a live id).
    pub fn alloc_run(&mut self) -> Option<RunId> {
        let id = RunId(self.next_run);
        self.next_run = self.next_run.checked_add(1)?;
        Some(id)
    }

    // ---- E6b: B9 -- the fence ----

    /// One registered fence.
    #[must_use]
    pub fn fence(&self, id: FenceId) -> Option<&SrFence> {
        self.fences.get(&id)
    }
    /// Every registered fence, in id order.
    pub fn fences(&self) -> impl Iterator<Item = &SrFence> {
        self.fences.values()
    }
    /// The fence an organization is, if it is one.
    #[must_use]
    pub fn fence_of_org(&self, org: mush_org::OrgId) -> Option<&SrFence> {
        self.by_org.get(&org).and_then(|id| self.fences.get(id))
    }
    /// One open quote.
    #[must_use]
    pub fn quote(&self, id: QuoteId) -> Option<&FenceQuote> {
        self.quotes.get(&id)
    }
    /// Every open quote, in id order (double-ended, so a caller can walk the newest
    /// first under a visit bound).
    pub fn quotes(&self) -> impl DoubleEndedIterator<Item = &FenceQuote> {
        self.quotes.values()
    }
    /// The quote a contract belongs to, if any (the observer's O(log n) lookup).
    #[must_use]
    pub fn quote_of_contract(&self, contract: ContractId) -> Option<QuoteId> {
        self.by_quote_contract.get(&contract).copied()
    }
    /// Mint the next fence id, or `None` at the ceiling.
    pub fn alloc_fence(&mut self) -> Option<FenceId> {
        let id = FenceId(self.next_fence);
        self.next_fence = self.next_fence.checked_add(1)?;
        Some(id)
    }
    /// The next fence id **without** minting it — the allocator floor, which
    /// E6b's verify fold (R8) derives from every document's parsed key rather than
    /// from what loaded.
    #[must_use]
    pub fn next_fence_id(&self) -> u64 {
        self.next_fence
    }
    /// Mint the next quote id, or `None` at the ceiling.
    pub fn alloc_quote(&mut self) -> Option<QuoteId> {
        let id = QuoteId(self.next_quote);
        self.next_quote = self.next_quote.checked_add(1)?;
        Some(id)
    }
    /// Re-derive the fence indexes -- the one place they are built.
    pub(crate) fn reindex_fences(&mut self) {
        self.by_org = self.fences.values().map(|f| (f.org, f.id)).collect();
        self.by_quote_contract = self
            .quotes
            .values()
            .filter_map(|q| q.contract.map(|c| (c, q.id)))
            .collect();
    }

    /// The allocator document.
    #[must_use]
    pub fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_run_id": self.next_run,
            "next_fence_id": self.next_fence,
            "next_quote_id": self.next_quote,
        })
    }

    /// Re-derive every listing index from the listings — the one place they are
    /// built, so they cannot drift.
    pub(crate) fn reindex(&mut self) {
        self.by_contract = self
            .runs
            .values()
            .map(|r| (r.contract, r.id))
            .collect::<BTreeMap<_, _>>();
        self.live = self
            .runs
            .values()
            .filter(|r| !r.state.is_closed())
            .map(|r| r.id)
            .collect();
    }

    /// Re-derive the profile indexes (D7) — same rule, other family.
    pub(crate) fn reindex_profiles(&mut self) {
        self.by_obligation = self
            .profiles
            .values()
            .filter_map(|p| p.upkeep.map(|o| (o, p.obj)))
            .collect();
        self.by_account = self
            .profiles
            .values()
            .filter_map(|p| p.account.map(|a| (a, p.obj)))
            .collect();
    }

    /// Note a listing as just-closed, dropping the oldest when the ring is full.
    pub(crate) fn note_closed(&mut self, id: RunId) {
        if self.recent_closed.contains(&id) {
            return;
        }
        while self.recent_closed.len() >= RECENT_CLOSED_RING {
            self.recent_closed.pop_front();
        }
        self.recent_closed.push_back(id);
    }
}

/// Queue one module write.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Install a profile into RAM and queue its container doc — the single write path,
/// and therefore the one place the profile indexes are maintained (D7).
pub(crate) fn put_profile(ctx: &mut Ctx<'_>, p: SrProfile) {
    if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
        if let Some(prev) = store.profiles.insert(p.obj, p.clone()) {
            // A profile's obligation and account both move (a downgrade replaces the
            // record, R4 re-resolves a closed account), so the old keys go first.
            if let Some(o) = prev.upkeep {
                store.by_obligation.remove(&o);
            }
            if let Some(a) = prev.account {
                store.by_account.remove(&a);
            }
        }
        if let Some(o) = p.upkeep {
            store.by_obligation.insert(o, p.obj);
        }
        if let Some(a) = p.account {
            store.by_account.insert(a, p.obj);
        }
    }
    let Ok(value) = serde_json::to_value(&p) else {
        tracing::error!(obj = p.obj.0, "sr.economy: a profile did not serialize");
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: profile_key(p.obj),
            value,
        },
    );
}

/// Install a **live** listing into RAM and queue its container doc — the single write
/// path.
///
/// A listing whose state is `Closed` is not written: it is [`del_run`]'d instead
/// (E6a verify fold, D1). Nothing ever reads a closed listing except `+run/show`'s
/// grace ring, and keeping the document made post→withdraw a free loop that minted
/// permanent records and lengthened every board scan for the life of the world.
pub(crate) fn put_run(ctx: &mut Ctx<'_>, r: RunTemplate) {
    if r.state.is_closed() {
        del_run(ctx, r.id);
        return;
    }
    if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
        store.runs.insert(r.id, r.clone());
        store.reindex();
    }
    let Ok(value) = serde_json::to_value(&r) else {
        tracing::error!(run = r.id.0, "sr.economy: a listing did not serialize");
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: run_key(r.id),
            value,
        },
    );
}

/// Forget a listing: out of RAM, out of the indexes, out of the store, and onto the
/// just-closed ring (E6a verify fold, D1).
///
/// The money was never here — it is the contract's, and the contract keeps its own
/// terminal record — so a deleted listing costs a board row and nothing else. That is
/// the same trade `store`'s header already makes for a listing that will not load.
pub(crate) fn del_run(ctx: &mut Ctx<'_>, id: RunId) {
    if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
        store.runs.remove(&id);
        store.reindex();
        store.note_closed(id);
    }
    push_op(ctx.out, ModuleWrite::Del { key: run_key(id) });
}

/// Install a fence record and queue its container doc -- the single write path
/// (E6b, B9), and therefore the one place the fence indexes are maintained.
pub(crate) fn put_fence(ctx: &mut Ctx<'_>, f: SrFence) {
    if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
        store.fences.insert(f.id, f.clone());
        store.reindex_fences();
    }
    let Ok(value) = serde_json::to_value(&f) else {
        tracing::error!(fence = f.id.0, "sr.economy: a fence did not serialize");
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: fence_key(f.id),
            value,
        },
    );
}

/// Install a quote and queue its container doc -- the single write path (E6b, B9).
pub(crate) fn put_quote(ctx: &mut Ctx<'_>, q: FenceQuote) {
    if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
        store.quotes.insert(q.id, q.clone());
        store.reindex_fences();
    }
    let Ok(value) = serde_json::to_value(&q) else {
        tracing::error!(quote = q.id.0, "sr.economy: a quote did not serialize");
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: quote_key(q.id),
            value,
        },
    );
}

/// Forget a quote: out of RAM, out of the indexes, out of the store (E6b, B9).
///
/// The same trade `del_run` makes for a closed listing -- the money was never here,
/// it is the contract's, and the contract keeps its own terminal record -- so a
/// quote/lapse loop mints no permanent documents.
pub(crate) fn del_quote(ctx: &mut Ctx<'_>, id: QuoteId) {
    if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
        store.quotes.remove(&id);
        store.reindex_fences();
    }
    push_op(ctx.out, ModuleWrite::Del { key: quote_key(id) });
}

/// Queue the allocator document.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(SrEconomyStore::meta_doc)
    else {
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: meta,
        },
    );
}

/// Build the slot from the ruleset's boot documents.
///
/// Total: every document it cannot use is **skipped loudly** and the world boots.
/// The allocator floor is the larger of the persisted `next_run_id` and one past the
/// highest listing that loaded, saturating, so a lost meta document can never mint an
/// id a live listing already carries.
#[must_use]
pub fn load(kv: &[(SmolStr, serde_json::Value)], cfg: SrEconomyConfig) -> SrEconomyStore {
    let mut store = SrEconomyStore {
        cfg,
        ..SrEconomyStore::default()
    };
    // E6b verify fold (R8): the allocator floors are derived from every document's
    // **parsed key**, loaded or not. Deriving them from what loaded meant a corrupt
    // `sr:fence:3` plus a lost `sr:meta` re-minted id 3 and `JsonSet` straight over
    // the evidence — and any surviving quote naming fence 3 then named a different
    // organization. A key that parses is a claim on an id whether or not its body
    // does.
    let mut seen_run = 0u64;
    let mut seen_fence = 0u64;
    let mut seen_quote = 0u64;
    for (key, value) in kv {
        let key = key.as_str();
        if let Some(rest) = key.strip_prefix("sr:profile:") {
            load_profile(&mut store, key, rest, value);
        } else if let Some(rest) = key.strip_prefix("sr:run:") {
            if let Ok(n) = rest.parse::<u64>() {
                seen_run = seen_run.max(n.saturating_add(1));
            }
            load_run(&mut store, key, rest, value);
        } else if let Some(rest) = key.strip_prefix("sr:fence:") {
            if let Ok(n) = rest.parse::<u64>() {
                seen_fence = seen_fence.max(n.saturating_add(1));
            }
            load_fence(&mut store, key, rest, value);
        } else if let Some(rest) = key.strip_prefix("sr:quote:") {
            if let Ok(n) = rest.parse::<u64>() {
                seen_quote = seen_quote.max(n.saturating_add(1));
            }
            load_quote(&mut store, key, rest, value);
        } else if key == META_KEY {
            store.next_run = value
                .get("next_run_id")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            store.next_fence = value
                .get("next_fence_id")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            store.next_quote = value
                .get("next_quote_id")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
        }
        // Every other key belongs to another half of the ruleset (M17's `extended`)
        // or to a forward schema; tolerated in both directions.
    }
    let floor = store
        .runs
        .keys()
        .map(|id| id.0.saturating_add(1))
        .max()
        .unwrap_or(0);
    store.next_run = store.next_run.max(floor).max(seen_run);
    // E6b (B9): the same allocator floor for the two fence families, so a lost meta
    // document can never mint an id a live record already carries.
    let fence_floor = store
        .fences
        .keys()
        .map(|id| id.0.saturating_add(1))
        .max()
        .unwrap_or(0);
    store.next_fence = store.next_fence.max(fence_floor).max(seen_fence);
    let quote_floor = store
        .quotes
        .keys()
        .map(|id| id.0.saturating_add(1))
        .max()
        .unwrap_or(0);
    store.next_quote = store.next_quote.max(quote_floor).max(seen_quote);
    store.reindex();
    store.reindex_profiles();
    store.reindex_fences();
    store
}

/// Load one `sr:fence:{id}` document -- **total**: a corrupt one is skipped loudly
/// and the world boots, with DL-07's identity rule (`id == key`) applied.
fn load_fence(
    store: &mut SrEconomyStore,
    key: &str,
    rest: &str,
    value: &serde_json::Value,
) -> Option<()> {
    let Ok(n) = rest.parse::<u64>() else {
        tracing::warn!(%key, "sr.economy: a fence document has an unreadable key; skipped");
        return None;
    };
    let id = FenceId(n);
    // E6b verify fold (D6): the cap is enforced on **load** as well as on write, so
    // a store carrying more than `MAX_FENCES` boots with the cap and says so, rather
    // than quietly keeping whatever the documents held.
    if store.fences.len() >= crate::economy::config::MAX_FENCES {
        tracing::warn!(%key, "sr.economy: past the fence cap; skipped");
        return None;
    }
    match serde_json::from_value::<SrFence>(value.clone()) {
        Ok(f) if f.id == id => {
            store.fences.insert(id, f);
            Some(())
        }
        Ok(f) => {
            tracing::warn!(
                %key, record = f.id.0,
                "sr.economy: a fence document disagrees with its own key; skipped"
            );
            None
        }
        Err(error) => {
            tracing::warn!(%key, %error, "sr.economy: a fence document did not parse; skipped");
            None
        }
    }
}

/// Load one `sr:quote:{id}` document -- the same rules, and the same loss: a quote
/// is a price anybody can ask for again.
fn load_quote(
    store: &mut SrEconomyStore,
    key: &str,
    rest: &str,
    value: &serde_json::Value,
) -> Option<()> {
    let Ok(n) = rest.parse::<u64>() else {
        tracing::warn!(%key, "sr.economy: a quote document has an unreadable key; skipped");
        return None;
    };
    let id = QuoteId(n);
    // E6b verify fold (D6): the same on the quote family, against the whole-store
    // ceiling rather than the per-seller one — a quote is a projection of a price
    // anyone can ask for again, so dropping the overflow costs a row and nothing.
    if store.quotes.len() >= MAX_QUOTES {
        tracing::warn!(%key, "sr.economy: past the quote cap; skipped");
        return None;
    }
    match serde_json::from_value::<FenceQuote>(value.clone()) {
        Ok(q) if q.id == id => {
            store.quotes.insert(id, q);
            Some(())
        }
        Ok(q) => {
            tracing::warn!(
                %key, record = q.id.0,
                "sr.economy: a quote document disagrees with its own key; skipped"
            );
            None
        }
        Err(error) => {
            tracing::warn!(%key, %error, "sr.economy: a quote document did not parse; skipped");
            None
        }
    }
}

fn load_profile(
    store: &mut SrEconomyStore,
    key: &str,
    rest: &str,
    value: &serde_json::Value,
) -> Option<()> {
    let obj = match rest.parse::<u32>() {
        Ok(n) if n != 0 => ObjId(n),
        _ => {
            tracing::warn!(%key, "sr.economy: a profile document has an unreadable key; skipped");
            return None;
        }
    };
    match serde_json::from_value::<SrProfile>(value.clone()) {
        // DL-07's identity rule: `obj == key`, loud skip.
        Ok(p) if p.obj == obj => {
            store.profiles.insert(obj, p);
            Some(())
        }
        Ok(p) => {
            tracing::warn!(
                %key, record = p.obj.0,
                "sr.economy: a profile document disagrees with its own key; skipped"
            );
            None
        }
        Err(error) => {
            tracing::warn!(%key, %error, "sr.economy: a profile document did not parse; skipped");
            None
        }
    }
}

fn load_run(
    store: &mut SrEconomyStore,
    key: &str,
    rest: &str,
    value: &serde_json::Value,
) -> Option<()> {
    let Ok(n) = rest.parse::<u64>() else {
        tracing::warn!(%key, "sr.economy: a listing document has an unreadable key; skipped");
        return None;
    };
    let id = RunId(n);
    match serde_json::from_value::<RunTemplate>(value.clone()) {
        Ok(r) if r.id == id => {
            store.runs.insert(id, r);
            Some(())
        }
        Ok(r) => {
            tracing::warn!(
                %key, record = r.id.0,
                "sr.economy: a listing document disagrees with its own key; skipped"
            );
            None
        }
        Err(error) => {
            tracing::warn!(%key, %error, "sr.economy: a listing document did not parse; skipped");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allocator_floor_never_reissues_a_live_id() {
        let kv = vec![
            (SmolStr::new(META_KEY), serde_json::json!({})),
            (
                SmolStr::new("sr:run:9"),
                serde_json::json!({
                    "id": 9, "contract": 1, "principal": { "Object": 5 }, "account": 1,
                    "notice": { "name": "Extraction", "window": 10, "kinds": ["attest"] },
                    "board": "public", "slots": 3, "outcomes": ["success"],
                    "deadline": { "clock": "RealUtc", "at": 10 }, "revision": 1
                }),
            ),
        ];
        let store = load(&kv, SrEconomyConfig::default());
        assert_eq!(store.runs.len(), 1);
        assert_eq!(store.next_run, 10);
    }

    #[test]
    fn a_document_that_disagrees_with_its_key_is_skipped() {
        let kv = vec![(
            SmolStr::new("sr:profile:4"),
            serde_json::json!({ "obj": 5 }),
        )];
        assert!(load(&kv, SrEconomyConfig::default()).profiles.is_empty());
    }

    #[test]
    fn a_system_payer_listing_never_loads() {
        let kv = vec![(
            SmolStr::new("sr:run:1"),
            serde_json::json!({
                "id": 1, "contract": 1, "principal": { "System": "sr.chargen" }, "account": 1,
                "notice": { "name": "Bad", "window": 10, "kinds": [] },
                "board": "public", "slots": 1, "outcomes": ["success"],
                "deadline": { "clock": "RealUtc", "at": 10 }, "revision": 1
            }),
        )];
        assert!(load(&kv, SrEconomyConfig::default()).runs.is_empty());
    }
}
