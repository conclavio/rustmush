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

use std::collections::BTreeMap;

use mush_core::{Ctx, ModuleOp, ModuleWrite, ObjId, Outbox};
use mush_economy::{CadenceId, ContractId, InstrumentId};
use smol_str::SmolStr;

use crate::economy::config::SrEconomyConfig;
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
    pub(crate) next_run: u64,
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
    /// The listing that mirrors `contract`, if any.
    #[must_use]
    pub fn run_of_contract(&self, contract: ContractId) -> Option<&RunTemplate> {
        self.by_contract.get(&contract).and_then(|id| self.run(*id))
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
        self.runs
            .values()
            .filter(|r| !r.state.is_closed() && r.principal == *principal)
            .count()
    }
    /// How many listings are open at all (§9's `BOARD_CAP`).
    #[must_use]
    pub fn open_runs(&self) -> usize {
        self.runs.values().filter(|r| !r.state.is_closed()).count()
    }
    /// Whether `party` already holds a live run (§11 q5: at most one).
    #[must_use]
    pub fn party_has_live_run(&self, party: u64) -> bool {
        self.runs
            .values()
            .any(|r| !r.state.is_closed() && r.party == Some(party))
    }

    /// Mint the next listing id, or `None` at the ceiling (E4b1's B2 rule: an
    /// exhausted allocator refuses one operation loudly, it never wraps a live id).
    pub fn alloc_run(&mut self) -> Option<RunId> {
        let id = RunId(self.next_run);
        self.next_run = self.next_run.checked_add(1)?;
        Some(id)
    }

    /// The allocator document.
    #[must_use]
    pub fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({ "next_run_id": self.next_run })
    }

    /// Re-derive the contract index from the listings — the one place it is built,
    /// so it cannot drift.
    pub(crate) fn reindex(&mut self) {
        self.by_contract = self
            .runs
            .values()
            .map(|r| (r.contract, r.id))
            .collect::<BTreeMap<_, _>>();
    }
}

/// Queue one module write.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Install a profile into RAM and queue its container doc — the single write path.
pub(crate) fn put_profile(ctx: &mut Ctx<'_>, p: SrProfile) {
    if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
        store.profiles.insert(p.obj, p.clone());
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

/// Install a listing into RAM and queue its container doc — the single write path.
pub(crate) fn put_run(ctx: &mut Ctx<'_>, r: RunTemplate) {
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
    for (key, value) in kv {
        let key = key.as_str();
        if let Some(rest) = key.strip_prefix("sr:profile:") {
            load_profile(&mut store, key, rest, value);
        } else if let Some(rest) = key.strip_prefix("sr:run:") {
            load_run(&mut store, key, rest, value);
        } else if key == META_KEY {
            store.next_run = value
                .get("next_run_id")
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
    store.next_run = store.next_run.max(floor);
    store.reindex();
    store
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
