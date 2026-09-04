//! The E5c module state and its documents (namespace `wod`).
//!
//! | doc | class | corrupt → |
//! |---|---|---|
//! | `profile:{obj}` | leaf | **skipped loudly and re-derived** at the next evaluation |
//! | `eval:{obj}` | leaf | skipped loudly, re-seeded from the clock |
//! | `residence:{obj}` | leaf | skipped loudly (a chargen pick, not a home) |
//! | `econ:meta` | allocators | tolerated field by field |
//!
//! **Why a corrupt profile does not fail the boot**, unlike housing's two container
//! docs: a `SpendingProfile` is *derived*. Resources comes off the sheet, the level
//! comes off Resources, and the plan it submits is restated every period — so
//! re-deriving one costs the character their strain counter and their open prompt,
//! and costs their **home** nothing at all. Failing the boot instead would take the
//! whole world down for a record the next tick rebuilds. The direction of the loss is
//! deliberate: strain resets to zero and a prompt disappears, both of which are the
//! *lenient* half of every rule they take part in.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Ctx, ModuleOp, ModuleWrite, ObjId, Outbox};
use mush_faction::FactionId;
use smol_str::SmolStr;

use crate::economy::config::WodEconomyConfig;
use crate::economy::record::{SpendingProfile, open_pool};

/// The module namespace (the ruleset module's own name).
pub const NS: &str = "wod";
/// The E5c allocator/cursor document.
pub const META_KEY: &str = "econ:meta";
/// The evaluation tick's tag.
pub const TICK_TAG: &str = "wod.economy";
/// The evaluation tick's wall period. A **latency** bound only: the evaluation
/// compares cadence positions, so this rate never affects what is evaluated
/// (E5b's deviation 6, same rule).
pub const TICK_PERIOD_SECS: u64 = 30;
/// The most profiles one fire may evaluate (housing's `MAX_ASSESSMENTS_PER_FIRE`,
/// restated for this tick so both bounds are visible where they apply).
pub const MAX_EVALUATIONS_PER_FIRE: usize = 128;

/// `profile:{obj}`.
#[must_use]
pub fn profile_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("profile:{}", obj.0))
}
/// `eval:{obj}` — the per-character pull cursor.
#[must_use]
pub fn eval_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("eval:{}", obj.0))
}
/// `residence:{obj}` — a chargen Residence pick, consumed at approval.
#[must_use]
pub fn residence_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("residence:{}", obj.0))
}

/// What the evaluation remembers about one character between fires.
///
/// Module state rather than profile state, for E5a deviation 3's reason: the period
/// ordinal is *how this world has been running the clock*, not a fact about the
/// character, and a profile that carried it would be rewritten on every silent
/// period.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EvalState {
    /// The cadence position last evaluated at.
    #[serde(default)]
    pub position: i64,
    /// The monotonic period ordinal. Prompt expiry and the `Coast` rate limit are
    /// measured in this, never in `position`, so a re-pointed clock changes nothing.
    #[serde(default)]
    pub period: u64,
    /// Whether `position` has ever been set. A profile created between two fires must
    /// not read as "last evaluated at position 0" on a clock sitting at 900.
    #[serde(default)]
    pub seeded: bool,
}

/// The in-RAM E5c slot.
#[derive(Clone, Debug, Default)]
pub struct WodEconomyStore {
    /// Frozen at wiring.
    pub(crate) cfg: WodEconomyConfig,
    pub(crate) profiles: BTreeMap<ObjId, SpendingProfile>,
    pub(crate) eval: BTreeMap<ObjId, EvalState>,
    /// Chargen Residence picks awaiting approval.
    pub(crate) residence: BTreeMap<ObjId, mush_housing::PropertyId>,
    /// The rotation cursor the evaluation resumes at (E4b2's B4).
    pub(crate) cursor: u32,
    /// The prompt-id allocator.
    pub(crate) next_prompt: u64,
    /// The resolved cadence id, once the named clock has been found or created.
    pub(crate) cadence: Option<mush_economy::CadenceId>,
    /// Whether this boot created the cadence clock (what `@wod/economy` reports).
    pub(crate) cadence_created: bool,
    /// The `wod.usd` instrument id, in exact mode.
    pub(crate) instrument: Option<mush_economy::InstrumentId>,
}

impl WodEconomyStore {
    /// The frozen configuration.
    #[must_use]
    pub fn config(&self) -> &WodEconomyConfig {
        &self.cfg
    }
    /// One profile.
    #[must_use]
    pub fn profile(&self, obj: ObjId) -> Option<&SpendingProfile> {
        self.profiles.get(&obj)
    }
    /// Every profile, in id order.
    pub fn profiles(&self) -> impl Iterator<Item = &SpendingProfile> {
        self.profiles.values()
    }
    /// Every profiled character, in id order — the evaluation's working set.
    pub fn profiled(&self) -> Vec<ObjId> {
        self.profiles.keys().copied().collect()
    }
    /// The per-character evaluation cursor.
    #[must_use]
    pub fn eval_state(&self, obj: ObjId) -> EvalState {
        self.eval.get(&obj).copied().unwrap_or_default()
    }
    /// The cadence clock everything is assessed on, once resolved.
    #[must_use]
    pub fn cadence(&self) -> Option<mush_economy::CadenceId> {
        self.cadence
    }
    /// Whether this boot created that clock rather than finding it.
    #[must_use]
    pub fn cadence_created(&self) -> bool {
        self.cadence_created
    }
    /// The registered `wod.usd` instrument, in exact mode.
    #[must_use]
    pub fn instrument(&self) -> Option<&mush_economy::InstrumentId> {
        self.instrument.as_ref()
    }
    /// A chargen Residence pick awaiting approval.
    #[must_use]
    pub fn residence(&self, obj: ObjId) -> Option<mush_housing::PropertyId> {
        self.residence.get(&obj).copied()
    }

    /// **The per-faction pool of currently open front-attributed prompts** (§5.3).
    ///
    /// Derived on demand from the profiles rather than kept as a counter, so
    /// "decremented on resolution or expiry" is true by construction: a prompt that
    /// is gone is not counted, and there is no second place for the number to go
    /// stale.
    #[must_use]
    pub fn open_ops(&self) -> BTreeMap<FactionId, u32> {
        open_pool(&self.profiles)
    }

    /// How many operations are open on `faction` right now.
    #[must_use]
    pub fn open_ops_for(&self, faction: FactionId) -> u32 {
        self.open_ops().get(&faction).copied().unwrap_or(0)
    }

    /// Mint the next prompt id, or `None` at the ceiling (E4b1's B2 rule: refuse one
    /// operation loudly, never wrap a live id).
    pub fn alloc_prompt(&mut self) -> Option<u64> {
        let id = self.next_prompt;
        self.next_prompt = self.next_prompt.checked_add(1)?;
        Some(id)
    }

    /// The allocator resume document.
    #[must_use]
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_prompt_id": self.next_prompt,
            "eval_cursor": self.cursor,
        })
    }
}

// ---------------------------------------------------------------------------
// Persist helpers
// ---------------------------------------------------------------------------

fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp { ns: SmolStr::new_static(NS), op }));
}

/// Install a profile into RAM and queue its doc. Both always happen together.
pub(crate) fn put_profile(ctx: &mut Ctx<'_>, p: SpendingProfile) {
    if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
        store.profiles.insert(p.obj, p.clone());
    }
    push_op(ctx.out, ModuleWrite::JsonSet {
        key: profile_key(p.obj),
        value: serde_json::to_value(&p).expect("profile serializes"),
    });
}

/// Stamp a brand-new profile's evaluation cursor at the clock's **current** position
/// — housing's `seed_assessment` for the ruleset side, and for the same two reasons.
///
/// Without it the first tick fire would find an unseeded cursor at position 0 while
/// the campaign clock sits at 900, and the tick's own seeding step would then spend
/// the **first** cadence advance after creation doing nothing. That second half is
/// not cosmetic: housing assesses before the ruleset does (module order), so a
/// wasted first period is a period in which the tenancy has no plan at all, and two
/// of those in a row is exactly what `UNKNOWN_GRACE` fires on. Seeding at creation
/// makes the rule what it reads as: **the next period the clock advances is the
/// first one this character is evaluated on.**
pub(crate) fn seed_eval(ctx: &mut Ctx<'_>, obj: ObjId) {
    let position = ctx
        .w
        .module_state::<WodEconomyStore>()
        .and_then(WodEconomyStore::cadence)
        .and_then(|c| mush_housing::cadence_now(ctx.w, c))
        .unwrap_or(0);
    put_eval(ctx, obj, EvalState { position, period: 0, seeded: true });
}

/// Install an evaluation cursor and queue its doc.
pub(crate) fn put_eval(ctx: &mut Ctx<'_>, obj: ObjId, state: EvalState) {
    if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
        store.eval.insert(obj, state);
    }
    push_op(ctx.out, ModuleWrite::JsonSet {
        key: eval_key(obj),
        value: serde_json::to_value(state).expect("eval state serializes"),
    });
}

/// Record (or clear) a chargen Residence pick.
pub(crate) fn put_residence(
    ctx: &mut Ctx<'_>,
    obj: ObjId,
    property: Option<mush_housing::PropertyId>,
) {
    if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
        match property {
            Some(p) => {
                store.residence.insert(obj, p);
            }
            None => {
                store.residence.remove(&obj);
            }
        }
    }
    match property {
        Some(p) => push_op(ctx.out, ModuleWrite::JsonSet {
            key: residence_key(obj),
            value: serde_json::json!({ "property": p.0 }),
        }),
        None => push_op(ctx.out, ModuleWrite::Del { key: residence_key(obj) }),
    }
}

/// Queue the allocator document.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx.w.module_state::<WodEconomyStore>().map(WodEconomyStore::meta_doc) else {
        return;
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key: SmolStr::new_static(META_KEY), value: meta });
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

/// Load the E5c documents out of the ruleset's own boot data.
///
/// The caller hands us the whole `wod` namespace, which also carries M17's
/// `conceal` / `pending` / `extended` documents; every key that is not one of ours is
/// simply not ours (forward schema, the pattern `on_world_loaded` already follows).
///
/// Allocators resume from `meta` **floored past every parsable key** and saturating
/// (E4b1's B1), so a hand-written id at the ceiling parks the allocator where minting
/// refuses instead of wrapping live ids back into use.
#[must_use]
pub fn load(kv: &[(SmolStr, serde_json::Value)], cfg: WodEconomyConfig) -> WodEconomyStore {
    let mut store = WodEconomyStore { cfg, ..WodEconomyStore::default() };
    let mut seen_prompts: BTreeSet<u64> = BTreeSet::new();
    for (key, value) in kv {
        let key = key.as_str();
        if let Some(rest) = key.strip_prefix("profile:") {
            let Some(obj) = rest.parse::<u32>().ok().map(ObjId) else {
                tracing::error!(%key, "wod.economy: unparsable profile key; skipped");
                continue;
            };
            match serde_json::from_value::<SpendingProfile>(value.clone()) {
                Ok(p) => {
                    if let Some(pr) = &p.prompt {
                        seen_prompts.insert(pr.id);
                    }
                    store.profiles.insert(obj, p);
                }
                Err(error) => {
                    tracing::error!(
                        %key, %error,
                        "wod.economy: a spending profile did not parse; skipped and re-derived \
                         at the next evaluation (strain and any open prompt are lost, which is \
                         the lenient direction; the lease is unaffected)"
                    );
                }
            }
        } else if let Some(rest) = key.strip_prefix("eval:") {
            let Some(obj) = rest.parse::<u32>().ok().map(ObjId) else {
                continue;
            };
            match serde_json::from_value::<EvalState>(value.clone()) {
                Ok(s) => {
                    store.eval.insert(obj, s);
                }
                Err(error) => {
                    tracing::error!(%key, %error, "wod.economy: eval cursor skipped; re-seeded");
                }
            }
        } else if let Some(rest) = key.strip_prefix("residence:") {
            let Some(obj) = rest.parse::<u32>().ok().map(ObjId) else {
                continue;
            };
            match value.get("property").and_then(serde_json::Value::as_u64) {
                Some(p) => {
                    store.residence.insert(obj, mush_housing::PropertyId(p));
                }
                None => {
                    tracing::error!(%key, "wod.economy: residence pick skipped");
                }
            }
        } else if key == META_KEY {
            if let Some(n) = value.get("next_prompt_id").and_then(serde_json::Value::as_u64) {
                store.next_prompt = store.next_prompt.max(n);
            }
            if let Some(n) = value.get("eval_cursor").and_then(serde_json::Value::as_u64) {
                store.cursor = u32::try_from(n).unwrap_or(u32::MAX);
            }
        }
    }
    // Floor the prompt allocator past every id a surviving profile actually holds,
    // never only from `meta`.
    for id in seen_prompts {
        store.next_prompt = store.next_prompt.max(id.saturating_add(1));
    }
    store
}
