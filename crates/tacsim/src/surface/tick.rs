//! **The surface tick, `tacsim.surface`** (doc 20 cut SM.3b; §8 "The clock", D-20-11; the
//! SM.3b brief's 3.5 and its lead rulings 2 and 4, findings F2 and F8).
//!
//! Coarse, 1 s, the journey tick's shape (`mush_sim`'s `advance_due`): each fire scans the
//! `tacsim.excursion` family, takes the rigs that are **due** (an act whose `due_ic` has come)
//! or **untold** (a finished act the operator has not heard), oldest first by `(due_ic or
//! last.at_ic, tv)`, visits at most [`MAX_EXCURSIONS_VISITED_PER_FIRE`] and works at most
//! [`MAX_EXCURSIONS_WORKED_PER_FIRE`]:
//!
//! - **tell** — a normal-lane write: the projection (`tacsim:tv:<tv>`, `told` moved on and one
//!   log line stamped at the act's own instant) and the completion line to the operator
//!   (`crate::port_tick::notify`); no swap, no submission;
//! - **due** — the world step, idempotent by state, in the tick's own outbox: a launch's
//!   `mush_sim::place` at the ramp pose, a recover's `mush_sim::place_interior` back into the
//!   bay, nothing for a dump or a probe; then the projection beside the placement; then the
//!   finish `tacsim_surface_done` **as the operator** through
//!   [`crate::port_tick::Fire::as_principal`](crate::port_tick) — one act per principal per
//!   fire, so an operator's two rigs due together finish on successive fires (§8).
//!
//! The tick runs as GOD and submits nothing as itself (`mush-campaign/src/tick.rs:28-37`);
//! every submission is inside an `as_principal` closure. **Each fire starts from an empty owed
//! map** (finding F8): the tick persists no debts; its fairness is the oldest-due order, which
//! a deferred act keeps because its `due_ic` does not move.
//!
//! **Nothing is RAM-only that matters**: the slot ([`SurfaceTickState`]) holds only retry
//! bookkeeping and once-a-boot pages; every fact is in the family or the projection, so a
//! restart resumes at the next fire and an overdue act applies at its own stamp (lead ruling
//! 2). The start's pull verification is by construction (the tick only sees acts that landed
//! in the family); the finish's is the tell pass.
//!
//! **The retry rule** (finding F2): only a `Submitted` finish that did not land counts as an
//! attempt; past [`MAX_DONE_ATTEMPTS`] the tick pages once a boot and passes the act over for
//! [`DONE_BACKOFF_FIRES`], then submits again — never skipped for the boot (the port tick's
//! `launch_backoff` shape).
//!
//! **Doc 20 cut SM.4a** adds, per fire: **the warm** — at the top, at most one wanted site's
//! ground is built (`crate::surface::warm_one`; lead ruling 6), never on an act's path; **the
//! reading acts' due step** — a scan or a sounding from the bay is placed at the ramp exactly as
//! a launch is; **the reading tell** — a finished scan's or sounding's readings are rendered
//! from the operator's own chart row restricted to the act's span (D17), the projection logging
//! the block's first line, and `READINGS_GONE` alone when the row is gone, unreadable or under
//! another rule (D26) — `READINGS_DARK` when the row stands but the site's ground cannot be read
//! at the tell; and **the heard sounding** (§5, Q-20-9; D7, D8, D24) — a loud sounding
//! inside its sounder's budget reaches every other party with a rig or a hull on the site,
//! connected and not quiet, as one line naming a bearing and a range band and nothing more,
//! **only once the projection write that advances `told` succeeded**, so the at-least-once tell
//! never repeats a heard line.
//!
//! **Doc 20 cut SM.4b** adds **the started leg** — a third candidate reason: a drive (or a far
//! recover) in flight, not yet due, whose leg the projection does not show yet is projected at
//! once, so clients interpolate the rig between its waypoints; **the drive's due step** — the rig
//! placed at its leg's end (from the bay straight there, the ramp half inside the act), a far
//! recover's rig placed back in the bay from its leg's start, its end or beside the ramp; **the
//! arrival** — `ARRIVED`, with the operator's own reading of the end pocket; and the projection
//! fitted under its 4 KiB by dropping the oldest log lines, never the leg.
//!
//! **Doc 20 cut SM.8** adds a fourth candidate reason, **queued** — an idle record holding the
//! unload a `Done` recover filed (`RigRecord::queued`): the tick tells the recover
//! (`RECOVERED_UNLOADING`), then submits the unload's start **as the operator**, under its own key
//! `tacsim.surface/{tv}/{ordinal}/queued` with `how: queued`, by the same attempt rule (attempts
//! keyed `(tv, ordinal, QUEUED_CYCLE)`, never a finish's retries); and the unload's tell —
//! `UNLOADED` (with what stayed), `UNLOADED_NONE` (nothing moved, something stays) or
//! `UNLOADED_NONE_EMPTY` (nothing moved, nothing was there). **Doc 20 cut SM.11** (the reviewer's
//! R1): a queued start the hopper's full marker ring would refuse is never submitted — the tick
//! reads the offer's own rule first (`markers_hold`), holds the queue on its back-off and tells
//! `WHY_MARKERS_QUEUED` once per queue under the notice budget.
//!
//! **Doc 20 cut SM.11** adds three candidate reasons (the brief's 3.8): **plan** — an idle record
//! whose fill plan is live for the rig's lessee: the tick runs the interpreter
//! (`crate::surface::plan::decide`) over its own reads and submits the next act's start **as the
//! author** with `by: plan` (keyed `tacsim.surface/{tv}/{ordinal}/plan/{step}/{word}`), or the
//! plan's end (`act: plan_end`, keyed `tacsim.surface/{tv}/plan/{no}/end`), or caches the wait in
//! RAM (a charging wait until its instant, the others for `PLAN_WAIT_IC_SECS`; a restart re-decides
//! from durable state); **plan untold** — the plan's notes the projection has not told
//! (`tell_plan`); **idle** — a rig on the ground with nothing in flight and no live plan for a day:
//! the tick recovers it as its operator (`by: idle`), or, when its charge cannot pay the way home,
//! reports it parked where it stands, once. **The notice budget** (lead ruling 7): every surface
//! line goes through `crate::port_tick::notify_budget` — an offline operator's queue holds the
//! trip-level lines only (an unload, a plan's parking or end, an idle return, a stranded rig, a
//! refused act), never a cycle, a reading, a drive, a launch or a recover.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{Ctx, ObjId, PerceivedRef, SceneId, World};
use serde_json::json;

use crate::port::families::PortHandles;
use crate::port_tick::{Fire, Step, SwapRefused};
use crate::store::{TacsimStore, put_tv_rec};
use crate::surface::act::{
    Where, cycle_key, done_key, queued_key, ramp_label, site_berth, site_name, where_is,
};
use crate::surface::address::{self, Octant};
use crate::surface::chart::{ChartKey, ChartRecord};
use crate::surface::excursion::{
    ActKind, ActResult, ActView, ExcursionId, FrozenLeg, LAUNCH_S, Mined, Outcome, PlanTold,
    PlanView, ReadSpan, RefusedCode, RigKey, RigRecord, RunEnd, RunView, TvPose, TvRec, TvRecState,
    TvState, cells_apart, site_cell,
};
use crate::surface::reading::{Footing, READING_RULE_VERSION};

/// The tick's tag.
pub const SURFACE_TICK: &str = "tacsim.surface";
/// How often it fires (Coarse, §8).
pub const SURFACE_TICK_PERIOD: Duration = Duration::from_secs(1);
/// Most rigs one fire visits (§8; the journey tick's figure).
pub const MAX_EXCURSIONS_VISITED_PER_FIRE: usize = 64;
/// Most rigs one fire works — tells and submissions (§8; the journey tick's figure).
pub const MAX_EXCURSIONS_WORKED_PER_FIRE: usize = 16;
/// Counted attempts before the finish pages and backs off (finding F2).
pub const MAX_DONE_ATTEMPTS: u8 = 8;
/// Fires an exhausted finish is passed over before it is submitted again.
pub const DONE_BACKOFF_FIRES: u64 = 60;
/// One scan page of the family.
pub const SCAN_PAGE: usize = 256;
/// The most rig records one fire reads (past it the scan pages once a boot and prunes
/// nothing).
pub const MAX_RIG_RECORDS: usize = 1_024;

/// `(tv.0, ordinal, cycle)`: one finish's key in the slot — an act's with cycle 0, a mining run's
/// cycle `k` under the run's ordinal (doc 20 cut SM.7; the brief's 3.8) — and since doc 20 cut
/// SM.8 a queued unload's START under the ordinal it will take, with [`QUEUED_CYCLE`].
type ActKey = (u32, u32, u8);

/// **The attempts key's cycle for a queued unload's start** (doc 20 cut SM.8; the brief's 3.9):
/// past [`crate::surface::excursion::MAX_RUN_CYCLES`], so it never meets a cycle's key (cycle 0
/// is an act's). Its attempts are a start's, never a finish's retries: [`finish_retried`] skips
/// them.
pub(crate) const QUEUED_CYCLE: u8 = u8::MAX;

/// **The attempts key's cycle for a plan's start or end** (doc 20 cut SM.11; the brief's 3.8): past
/// every run cycle and beside [`QUEUED_CYCLE`]; never a finish's retries.
pub(crate) const PLAN_CYCLE: u8 = u8::MAX - 1;

/// **The attempts key's cycle for the idle return's recover** (doc 20 cut SM.11).
pub(crate) const IDLE_CYCLE: u8 = u8::MAX - 2;

/// Whether `cycle` is a start's attempts key (a queued unload, a plan's step, the idle return),
/// never a finish's retry.
const fn start_cycle(cycle: u8) -> bool {
    cycle == QUEUED_CYCLE || cycle == PLAN_CYCLE || cycle == IDLE_CYCLE
}

/// **The tick's RAM slot** (the brief's 3.5): retry bookkeeping and once-a-boot pages only;
/// every fact is in the family or the projection, so a restart loses nothing but a retry
/// count. Its own slot, so `on_world_loaded`'s wholesale `TacsimStore` replacement never
/// touches it.
#[derive(Clone, Debug, Default)]
pub struct SurfaceTickState {
    /// Fires that found the layer on, this boot.
    fires: u64,
    /// The last fire's stats.
    last_fire: Option<SurfaceFireStats>,
    /// Acts whose finish the previous visit submitted (and has not yet been judged).
    submitted: BTreeSet<ActKey>,
    /// Counted attempts per act.
    attempts: BTreeMap<ActKey, u8>,
    /// Acts in their back-off: the fire it began on.
    backoff: BTreeMap<ActKey, u64>,
    /// Acts that have paged this boot (once each). A key whose ordinal is 0 is a record
    /// whose key disagreed with its value (ordinals start at 1): paged once and never
    /// pruned.
    paged: BTreeSet<ActKey>,
    /// Whether the scan's cap has paged this boot.
    scan_paged: bool,
    /// **Doc 20 cut SM.11** (the brief's 3.8, D16; the reviewer's F10): each waiting plan's cached
    /// wait — its `seq` and the IC instant before which the tick does not decide for it again;
    /// honoured only while the plan's `seq` is unchanged. Lost to a restart, and re-derived.
    plan_waits: BTreeMap<ObjId, (u32, i64)>,
    /// **Doc 20 cut SM.11** (lead ruling 6): a rig parked where it stands (or its operator
    /// ashore), retried no sooner than this IC instant.
    stranded: BTreeMap<ObjId, i64>,
    /// **Doc 20 cut SM.11**: the waits told to a connected author this boot, `(tv, seq, wait code)`.
    plan_told_waits: BTreeSet<(u32, u32, u8)>,
    /// **Doc 20 cut SM.11** (the reviewer's R1): the queued unloads held on a full marker ring
    /// whose `WHY_MARKERS_QUEUED` line was told (said, or queued under the notice budget) this
    /// boot, `(tv, the queue's Queued::after)` — once per queue; pruned once the queue is gone.
    markers_told: BTreeSet<(u32, u32)>,
}

/// **Test hook** (doc 20 cut SM.11; the brief's P1): the plan wait the tick has cached for `tv`,
/// as `(seq, until_ic)`, if any. Reads only.
#[doc(hidden)]
#[must_use]
pub fn plan_wait_for_tests(w: &World, tv: ObjId) -> Option<(u32, i64)> {
    w.module_state::<SurfaceTickState>()
        .and_then(|state| state.plan_waits.get(&tv).copied())
}

/// What one fire did, for tests and status renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceFireStats {
    /// Rigs visited.
    pub visited: u32,
    /// Rigs worked: tells and submissions.
    pub worked: u32,
    /// Finishes submitted (deferred to the engine's drain).
    pub submitted: u32,
    /// Finishes deferred: the operator already acted this fire, or has a submission in
    /// flight.
    pub deferred: u32,
    /// Acts passed over in their back-off.
    pub passed_over: u32,
    /// Completion lines told.
    pub told: u32,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): the fire's own wall time in
    /// microseconds — the tick's body from just after the layer check to the stats' store, the
    /// warm included. RAM only, never persisted, never shown to a player.
    pub elapsed_us: u64,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): what this fire's warm did
    /// (`crate::surface::warm_one`). RAM only, never persisted, never shown to a player.
    pub warmed: crate::surface::Warmed,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): the warm's wall time in
    /// microseconds, timed around the one `warm_one` call. RAM only, never persisted, never
    /// shown to a player.
    pub warm_us: u64,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): the drive legs this fire's
    /// own plan steps (and idle returns) searched — every `act::drive_plan` call their
    /// `plan::decide_walk` made, yard rigs included (summed from each call's
    /// `crate::surface::Peeks`). RAM only, never persisted, never shown to a player.
    pub searches: u32,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): those searches' summed wall
    /// time in microseconds. RAM only, never persisted, never shown to a player.
    pub search_us: u64,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): the non-building site peeks
    /// (`chart::peek_site` memo misses) this fire's plan steps paid. RAM only, never persisted,
    /// never shown to a player.
    pub peeks: u32,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): those peeks' summed wall time
    /// in microseconds. RAM only, never persisted, never shown to a player.
    pub peek_us: u64,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): **the hold decisions** the
    /// queued arm took this fire — each time `markers_hold` found a queued unload's hopper ring
    /// full and held its start. A held queue backs off [`DONE_BACKOFF_FIRES`] (60) fires before it
    /// is decided again, so **one held queue reads about one decision per 60 fires, not one per
    /// held fire**: held fires and held seconds come from the rig's own ring samples, never from
    /// this field. RAM only, never persisted, never shown to a player.
    pub markers_held: u32,
}

/// The last fire's stats, `None` before the first fire that found the layer on.
#[must_use]
pub fn last_surface_fire(w: &World) -> Option<SurfaceFireStats> {
    w.module_state::<SurfaceTickState>()
        .and_then(|state| state.last_fire)
}

/// **How many `tacsim.surface` fires found the layer on this boot** (doc 20 cut SM.12c, the cost
/// lane; `crate::port_tick::fires`' precedent): a status read for tests, which compare it across a
/// step to know whether the tick fired. RAM only, never persisted, never shown to a player.
#[must_use]
pub fn surface_fires(w: &World) -> u64 {
    w.module_state::<SurfaceTickState>()
        .map_or(0, |state| state.fires)
}

/// Whole microseconds since `started`, saturating (doc 20 cut SM.12c: no `as` cast).
fn micros_since(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// **Fold one call's peeks into the fire's stats** (doc 20 cut SM.12c, the cost lane): the legs it
/// searched and the site peeks it paid, with their wall times, saturating.
fn add_peeks(stats: &mut SurfaceFireStats, peeks: &crate::surface::Peeks) {
    stats.searches = stats.searches.saturating_add(peeks.legs);
    stats.search_us = stats.search_us.saturating_add(peeks.leg_us);
    stats.peeks = stats.peeks.saturating_add(peeks.site_misses);
    stats.peek_us = stats.peek_us.saturating_add(peeks.peek_us);
}

/// The most act ordinals of one rig [`finish_retried`] hands the finish's offer: the highest
/// ones, so the live act (a rig's ordinals only grow) is always among them.
pub(crate) const MAX_RETRIED_ORDINALS: usize = 8;

/// **The acts of `tv` whose finish was handed to the engine and did not land** — the act
/// ordinals the attempt rule has counted at least one attempt for this boot (the SM.3b fixer
/// round's item 4), ascending, at most [`MAX_RETRIED_ORDINALS`] (the highest). Keyed by the
/// act, not the rig (review finding F1): a stale attempt of an earlier act of the rig that a
/// capped scan never pruned, or a second live act of one rig, never marks this act. The
/// finish's offer fixes it so a dump whose transfer refused mid-bundle records `SpoilGone` on
/// its next finish instead of wedging. RAM only, like every retry count: after a restart the
/// first finish tries the transfer again. **Doc 20 cut SM.7**: `(ordinal, cycle)` pairs, cycle 0
/// an act's, so a mining cycle whose transfer refused records `Unworked` the same way. **Doc 20
/// cut SM.8** (the brief's reviewer finding F9): a queued unload's start attempts
/// ([`QUEUED_CYCLE`]) are never a finish's retries and never spend a slot.
#[must_use]
pub(crate) fn finish_retried(w: &World, tv: ObjId) -> Vec<(u32, u8)> {
    w.module_state::<SurfaceTickState>()
        .map(|state| retried_ordinals(state, tv))
        .unwrap_or_default()
}

/// [`finish_retried`], pure over the slot.
fn retried_ordinals(state: &SurfaceTickState, tv: ObjId) -> Vec<(u32, u8)> {
    let keys: Vec<(u32, u8)> = state
        .attempts
        .range((tv.0, 1, 0)..=(tv.0, u32::MAX, u8::MAX))
        .filter(|((_, _, cycle), _)| !start_cycle(*cycle))
        .map(|(&(_, ordinal, cycle), _)| (ordinal, cycle))
        .collect();
    let skip = keys.len().saturating_sub(MAX_RETRIED_ORDINALS);
    keys.get(skip..)
        .map(<[(u32, u8)]>::to_vec)
        .unwrap_or_default()
}

/// **Register the tick** (the brief's 3.5): its slot, the `+tv` selector's slot
/// (`crate::port::tv::RigPointer`) and the Coarse 1 s `tacsim.surface` tick. Called at the end
/// of `crate::port_tick::register`.
///
/// # Errors
///
/// Any registration the registrar refuses.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(SurfaceTickState::default());
    reg.state(crate::port::tv::RigPointer::default());
    // Doc 20 cut SM.4b (the brief's decision P2): the drive orders the start's offer fixes.
    reg.state(crate::surface::act::ActOrders::default());
    // Doc 20 cut SM.11: the plan's test counters (RAM; read by test hooks only).
    reg.state(crate::surface::plan::PlanCounters::default());
    reg.tick(
        SURFACE_TICK,
        TickCadence::Coarse(SURFACE_TICK_PERIOD),
        Arc::new(surface_tick),
    )?;
    Ok(())
}

/// What the retry rule says about one due visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Visit {
    /// Submit the finish (again).
    Submit,
    /// In its back-off: visited, not worked.
    PassOver,
    /// The attempts are exhausted this visit: page (once a boot) and pass over.
    Page,
}

/// **The attempt rule** (finding F2), pure over the slot: an attempt is counted only when
/// the visit finds the act still due, the previous visit's step for it was `Submitted`
/// (`submitted` holds it) and nothing is pending for the operator — a finish that was handed
/// to the engine and did not land. A deferral (`Pending`, `AlreadyActed`, `Owed`) never
/// counts and never clears `submitted`. At [`MAX_DONE_ATTEMPTS`] the act backs off
/// [`DONE_BACKOFF_FIRES`] (paging once a boot), then is submitted again; a further failure
/// restarts the back-off.
pub(crate) fn attempt_visit(state: &mut SurfaceTickState, key: ActKey, pending: bool) -> Visit {
    if let Some(began) = state.backoff.get(&key).copied() {
        if state.fires.saturating_sub(began) < DONE_BACKOFF_FIRES {
            return Visit::PassOver;
        }
        state.backoff.remove(&key);
        return Visit::Submit;
    }
    if !pending && state.submitted.remove(&key) {
        let attempts = state.attempts.entry(key).or_insert(0);
        *attempts = attempts.saturating_add(1);
        if *attempts >= MAX_DONE_ATTEMPTS {
            state.backoff.insert(key, state.fires);
            return if state.paged.insert(key) {
                Visit::Page
            } else {
                Visit::PassOver
            };
        }
    }
    Visit::Submit
}

/// After a due visit's step: a `Submitted` finish is remembered for the next visit's
/// judgement; nothing else touches the bookkeeping.
pub(crate) fn note_step(state: &mut SurfaceTickState, key: ActKey, step: Step) {
    if step == Step::Submitted {
        state.submitted.insert(key);
    }
}

/// The `tacsim.surface` fire.
fn surface_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    // The layer off visits nothing and leaves `last_fire` untouched (the port tick's rule).
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return;
    }
    // Doc 20 cut SM.12c (the cost lane): the fire's own clock, the warm included; one `Instant`
    // around the warm here rather than inside `warm_one` (one clock, no second seam through its
    // early returns).
    let started = std::time::Instant::now();
    // Doc 20 cut SM.4a (lead ruling 6): at most one wanted site's ground a fire, here and
    // never on an act's path; a dark geology layer returns at once.
    let warmed = crate::surface::warm_one(ctx);
    let warm_us = micros_since(started);
    let Some(slot) = ctx.w.module_state_mut::<SurfaceTickState>() else {
        return;
    };
    let mut state = std::mem::take(slot);
    state.fires = state.fires.saturating_add(1);
    let mut stats = walk(ctx, &mut state);
    stats.warmed = warmed;
    stats.warm_us = warm_us;
    stats.elapsed_us = micros_since(started);
    state.last_fire = Some(stats);
    if let Some(slot) = ctx.w.module_state_mut::<SurfaceTickState>() {
        *slot = state;
    }
}

/// The family, read whole up to [`MAX_RIG_RECORDS`]: the records and whether the scan was
/// complete. A record whose key disagrees with its value is skipped and paged once.
fn scan(
    ctx: &Ctx<'_>,
    handles: &PortHandles,
    state: &mut SurfaceTickState,
) -> (Vec<RigRecord>, bool) {
    let mut records: Vec<RigRecord> = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = match ctx.scan_action_state(
            &handles.families.excursions,
            after.as_deref(),
            SCAN_PAGE,
        ) {
            Ok(page) => page,
            Err(error) => {
                if !state.scan_paged {
                    state.scan_paged = true;
                    tracing::error!(%error, "tacsim: the rig excursion family does not read; the surface tick waits");
                }
                return (records, false);
            }
        };
        for entry in page.entries() {
            if records.len() >= MAX_RIG_RECORDS {
                if !state.scan_paged {
                    state.scan_paged = true;
                    tracing::error!(
                        max = MAX_RIG_RECORDS,
                        "tacsim: the rig excursion family holds more records than one surface \
                         fire reads; the rest wait"
                    );
                }
                return (records, false);
            }
            let record = entry.value();
            if RigKey::parse(entry.encoded_key()) != Some(record.key()) {
                if state.paged.insert((record.tv.0, 0, 0)) {
                    tracing::error!(
                        key = entry.encoded_key(),
                        tv = record.tv.0,
                        "tacsim: a rig excursion record disagrees with its own key; it is \
                         skipped until staff repair it"
                    );
                }
                continue;
            }
            records.push(record.clone());
        }
        match page.next_after() {
            Some(next) => after = Some(next.to_string()),
            None => return (records, true),
        }
    }
}

/// One candidate of a fire.
pub(crate) struct Candidate {
    record: RigRecord,
    due: bool,
    /// **Doc 20 cut SM.7**: the due item is the run's cycle `k` (else the act's).
    cycle: Option<u8>,
    untold: bool,
    /// **Doc 20 cut SM.4b** (the brief's decision P8): an act in flight, not yet due, whose
    /// params carry a leg the projection does not show yet — projected at once so clients
    /// interpolate the rig while it moves. **Doc 20 cut SM.7** (the lead's pre-commit round, item
    /// 4): or a run in flight, its cycle not yet due, that the projection does not show yet.
    started: bool,
    /// **Doc 20 cut SM.8** (the brief's 3.9): an idle record holding a queued unload, outside its
    /// start's back-off — the tick submits the start as the operator.
    queued: bool,
    /// **Doc 20 cut SM.11** (the brief's 3.8): an idle record whose plan is live for the rig's
    /// lessee, outside its back-off and its cached wait — the tick decides and submits.
    plan: bool,
    /// **Doc 20 cut SM.11**: the plan's notes or view the projection has not told.
    plan_untold: bool,
    /// **Doc 20 cut SM.11** (lead ruling 6): a rig a day idle on the ground — the idle return.
    idle: bool,
    order: (i64, u32),
}

/// **What the candidates read of a rig's plan** (doc 20 cut SM.11): the rig's live lessee and the
/// projection's told plan and view.
pub(crate) struct PlanReads<'a> {
    /// The rig's live lessee (`TacsimStore::tv_lease_on`; since doc 20 cut SM.12b through
    /// `crate::port::yard::rig_lease`, a yard rig's dispatcher).
    pub lessee: &'a dyn Fn(ObjId) -> Option<ObjId>,
    /// The projection's `plan_told` and `plan`.
    pub shown: &'a dyn Fn(ObjId) -> (Option<PlanTold>, Option<PlanView>),
    /// The hull whose helm a pilot holds (`crate::surface::plan::helm_hull`; since doc 20 cut
    /// SM.12b the hull a pilot operates — a dispatcher's yard hull): an orphaned queue's reading
    /// (the SM.11 fixer round's correctness/F3).
    pub helm: &'a dyn Fn(ObjId) -> Option<ObjId>,
}

/// No lessee.
#[cfg(test)]
fn no_lessee(_: ObjId) -> Option<ObjId> {
    None
}

/// No helm.
#[cfg(test)]
fn no_helm(_: ObjId) -> Option<ObjId> {
    None
}

/// Nothing shown.
#[cfg(test)]
fn nothing_shown(_: ObjId) -> (Option<PlanTold>, Option<PlanView>) {
    (None, None)
}

#[cfg(test)]
impl PlanReads<'static> {
    /// No plan reads: no lessee, nothing shown (the earlier cuts' candidate tests).
    #[must_use]
    pub(crate) fn none() -> PlanReads<'static> {
        PlanReads {
            lessee: &no_lessee,
            shown: &nothing_shown,
            helm: &no_helm,
        }
    }
}

/// **A plan's attempts key** (doc 20 cut SM.11): `(tv, the ordinal its start will take,
/// PLAN_CYCLE)`.
fn plan_attempt(record: &RigRecord) -> ActKey {
    (record.tv.0, record.ordinal.saturating_add(1), PLAN_CYCLE)
}

/// **The idle return's attempts key** (doc 20 cut SM.11).
fn idle_attempt(record: &RigRecord) -> ActKey {
    (record.tv.0, record.ordinal.saturating_add(1), IDLE_CYCLE)
}

/// **The plan view a rig's projection carries** (the SM.11 fixer round's conformance/F1): the
/// record's plan only when the rig's live lessee filed it (`plan_view_for`), so the next lessee's
/// projection never carries a previous lessee's plan.
fn plan_view_of(w: &World, record: &RigRecord) -> Option<PlanView> {
    // Doc 20 cut SM.12b: the one lessee reader (`yard::rig_lease`) — a yard rig's is its
    // dispatcher, else `plan_untold` would compare its view with an always-empty projection.
    let lessee = crate::port::yard::rig_lease(w, record.tv).map(|lease| lease.pilot);
    crate::surface::plan::plan_view_for(record.plan.as_ref(), lessee)
}

/// **Whether a plan's projection is untold** (doc 20 cut SM.11), pure: the record's plan number and
/// `seq` differ from the projection's told pair, or the view the rig's `lessee` may see
/// ([`plan_view_of`]'s rule) from the projection's view.
fn plan_untold(
    record: &RigRecord,
    lessee: Option<ObjId>,
    shown: &(Option<PlanTold>, Option<PlanView>),
) -> bool {
    let view = crate::surface::plan::plan_view_for(record.plan.as_ref(), lessee);
    let told = record.plan.as_ref().map(|plan| PlanTold {
        no: plan.no,
        seq: plan.seq,
    });
    (told.is_some() && told != shown.0) || view != shown.1
}

/// **An outcome's told pair** (doc 20 cut SM.7; decision R3): its ordinal and, for a mining
/// cycle, its cycle (0 for an act).
#[must_use]
pub(crate) fn told_pair(last: &Outcome) -> (u32, u8) {
    (last.ordinal, last.mined.as_ref().map_or(0, |mined| mined.k))
}

/// **A queued unload's attempts key** (doc 20 cut SM.8): `(tv, the ordinal its start will take,
/// QUEUED_CYCLE)`.
fn queued_attempt(record: &RigRecord) -> ActKey {
    (record.tv.0, record.ordinal.saturating_add(1), QUEUED_CYCLE)
}

/// Whether `key`'s finish is inside its back-off this fire (the attempt rule's own test,
/// read without touching the slot).
fn backing_off(state: &SurfaceTickState, key: ActKey) -> bool {
    state
        .backoff
        .get(&key)
        .is_some_and(|began| state.fires.saturating_sub(*began) < DONE_BACKOFF_FIRES)
}

/// **The fire's candidates, in visit order, capped** (§8; the SM.3b fixer round's item 5):
/// every record whose act is due (`due_ic <= now`) or whose last outcome is untold (`told`
/// reads the projection), ordered oldest first by `(due_ic or last.at_ic, tv)` — and **an act
/// inside its back-off with nothing to tell is left out before the cap**, so 64 or more
/// finishes that cannot land never fill the window of [`MAX_EXCURSIONS_VISITED_PER_FIRE`] and
/// starve newer acts world-wide. Once the back-off has run its [`DONE_BACKOFF_FIRES`] the act
/// is a candidate again (its old `due_ic` puts it first). Deterministic: a total order over
/// `(instant, tv)`. Returns the candidates and how many backed-off acts were left out.
///
/// **Doc 20 cut SM.4b** adds a third reason, **started** (the brief's decision P8): an act in
/// flight, not yet due, whose params carry a leg and whose projection's act (`shown`, the
/// projection's act ordinal, read like `told`) is not that act — ordered by `(started_ic, tv)`
/// among the others, so a drive's leg reaches `tacsim:tv:<tv>` on the first fire after its start.
/// **Doc 20 cut SM.7** (the SM.7 lead's pre-commit round, item 4) makes a run started the same way:
/// its cycle not yet due and the projection's run (`shown_run`, its ordinal and cycle) not this
/// run's — ordered by the cycle's own start (`due_ic − secs`), so the run reaches the projection
/// on the first fire after the start lands, never only at its first cycle's due.
///
/// **Doc 20 cut SM.7** (the brief's 3.8, decisions R1 and R3): a record's due instant is the
/// earlier of its act's and its run's cycle's — each outside its own back-off — a tie going to the
/// run (its cycle began first); and an outcome is **untold** iff `(last.ordinal, its cycle)`
/// differs from the projection's told pair (`told`, read like `shown`) — equality, never `>`, since
/// a run's cycles share its ordinal and may finish after a later scan's.
///
/// **Doc 20 cut SM.11** (the brief's 3.8) adds three reasons: **plan** — a record whose plan is
/// live for the rig's lessee, with no act, run or queued unload, outside its back-off and its
/// cached wait of the same `seq` (`Busy` is never cached), ordered by `(last.at_ic or the filing,
/// tv)`; **plan untold** — [`plan_untold`]; **idle** — an open excursion, nothing in flight, no
/// live plan for the lessee, `idle_since + IDLE_PARK_IC_SECS ≤ now`, outside its back-off and the
/// stranded retry, ordered by its idle instant.
#[allow(clippy::too_many_lines)] // one reason after another, then the order and the cap
pub(crate) fn candidates(
    records: Vec<RigRecord>,
    now: i64,
    told: impl Fn(ObjId) -> (u32, u8),
    shown: impl Fn(ObjId) -> Option<u32>,
    shown_run: impl Fn(ObjId) -> Option<(u32, u8)>,
    state: &SurfaceTickState,
    plans: &PlanReads<'_>,
) -> (Vec<Candidate>, u32) {
    let mut passed_over = 0_u32;
    let mut candidates: Vec<Candidate> = records
        .into_iter()
        .filter_map(|record| {
            let told = told(record.tv);
            let act_started = record
                .act
                .as_ref()
                .filter(|act| act.due_ic > now && act.params.leg().is_some())
                .filter(|act| shown(record.tv) != Some(act.ordinal))
                .map(|act| act.started_ic);
            let run_started = record
                .run
                .as_ref()
                .filter(|run| run.due_ic > now)
                .filter(|run| shown_run(record.tv) != Some((run.ordinal, run.k)))
                .map(|run| run.due_ic.saturating_sub(i64::from(run.secs)));
            let started_at = match (act_started, run_started) {
                (Some(act), Some(run)) => Some(act.min(run)),
                (act, run) => act.or(run),
            };
            let mut rest = |key: ActKey| {
                let resting = backing_off(state, key);
                if resting {
                    passed_over = passed_over.saturating_add(1);
                }
                !resting
            };
            let act_due = record
                .act
                .as_ref()
                .filter(|act| act.due_ic <= now)
                .filter(|act| rest((record.tv.0, act.ordinal, 0)))
                .map(|act| act.due_ic);
            let run_due = record
                .run
                .as_ref()
                .filter(|run| run.due_ic <= now)
                .filter(|run| rest((record.tv.0, run.ordinal, run.k)))
                .map(|run| (run.due_ic, run.k));
            let (due_at, cycle) = match (act_due, run_due) {
                (Some(act), Some((run, k))) if run <= act => (Some(run), Some(k)),
                (Some(act), _) => (Some(act), None),
                (None, Some((run, k))) => (Some(run), Some(k)),
                (None, None) => (None, None),
            };
            let untold_at = record
                .last
                .as_ref()
                .filter(|last| told_pair(last) != told)
                .map(|last| last.at_ic);
            // Doc 20 cut SM.11 (the brief's 3.8): the plan, its untold notes, the idle return.
            let lessee = (plans.lessee)(record.tv);
            let live = lessee.and_then(|lessee| record.live_plan_for(lessee));
            // The SM.8 handoff (the SM.11 fixer round's correctness/F3): the author's own queued
            // unload, the author off the plan's hull's helm, is the plan's to read (its helm wait,
            // told once, and one page), never SM.8's start, which would be refused at every try.
            let orphaned = live
                .zip(record.queued.as_ref())
                .is_some_and(|(plan, queued)| {
                    crate::surface::plan::queue_orphaned(queued, plan, (plans.helm)(plan.author))
                });
            // Doc 20 cut SM.8 (the brief's 3.9): a queued unload on an idle rig, outside its
            // start's back-off.
            let queued_at = record
                .queued
                .as_ref()
                .filter(|_| record.act.is_none() && record.run.is_none() && !orphaned)
                .filter(|_| rest(queued_attempt(&record)))
                .map(|queued| queued.at_ic);
            let quiet = record.act.is_none()
                && record.run.is_none()
                && (record.queued.is_none() || orphaned);
            // The lead's pre-commit round, item 3: a plan past its bound is decided whatever it
            // waits on — a standing queue or a cached wait — so its `Bound` end is submitted (the
            // interpreter answers it from any wait; only an act or a run in flight holds it).
            let idle = record.act.is_none() && record.run.is_none();
            let plan_at = live
                .filter(|plan| quiet || (idle && plan.deadline_ic <= now))
                .filter(|plan| {
                    plan.deadline_ic <= now
                        || state
                            .plan_waits
                            .get(&record.tv)
                            .is_none_or(|(seq, until)| *seq != plan.seq || *until <= now)
                })
                .filter(|_| rest(plan_attempt(&record)))
                .map(|plan| {
                    record
                        .last
                        .as_ref()
                        .map_or(plan.filed_ic, |last| last.at_ic)
                });
            let untold_plan_at =
                plan_untold(&record, lessee, &(plans.shown)(record.tv)).then(|| {
                    record.plan.as_ref().map_or(0, |plan| {
                        plan.notes.last().map_or(plan.filed_ic, |note| note.at_ic)
                    })
                });
            let idle_since = record.idle_since();
            let idle_at = idle_since
                .filter(|_| quiet && live.is_none() && record.excursion.is_some())
                .and_then(|since| since.checked_add(crate::surface::plan::IDLE_PARK_IC_SECS))
                .filter(|due| *due <= now)
                .filter(|_| {
                    state
                        .stranded
                        .get(&record.tv)
                        .is_none_or(|retry| *retry <= now)
                })
                .filter(|_| rest(idle_attempt(&record)));
            let at = due_at
                .or(untold_at)
                .or(started_at)
                .or(queued_at)
                .or(untold_plan_at)
                .or(plan_at)
                .or(idle_at)?;
            Some(Candidate {
                order: (at, record.tv.0),
                due: due_at.is_some(),
                cycle,
                untold: untold_at.is_some(),
                started: started_at.is_some(),
                queued: queued_at.is_some(),
                plan: plan_at.is_some(),
                plan_untold: untold_plan_at.is_some(),
                idle: idle_at.is_some(),
                record,
            })
        })
        .collect();
    candidates.sort_by_key(|candidate| candidate.order);
    candidates.truncate(MAX_EXCURSIONS_VISITED_PER_FIRE);
    (candidates, passed_over)
}

/// Collect, order and walk the candidates; the fire's stats.
#[allow(clippy::too_many_lines)] // one walk: tell, then due, then the bookkeeping
fn walk(ctx: &mut Ctx<'_>, state: &mut SurfaceTickState) -> SurfaceFireStats {
    let mut stats = SurfaceFireStats::default();
    let Some(handles) = ctx.w.module_state::<PortHandles>().cloned() else {
        return stats;
    };
    let now = ctx.w.world_ic_now().at;
    let (records, complete) = scan(ctx, &handles, state);
    // Durable staff recovery shares this fire's work ceiling.
    crate::surface::recovery::tick(ctx, &handles, &records, &mut stats);
    // Doc 20 cut SM.7 (the lead's pre-commit round, item 5): a mining order whose start landed (or
    // was overtaken) leaves the RAM slot, read against the whole family — never a capped scan.
    if complete {
        crate::surface::act::prune_mine_orders(ctx.w, &records);
    }
    // Every act still due over the whole scan, taken **before** the visit cut: an act past
    // the cut is still due, so its retry bookkeeping must survive the prune (the brief's
    // 3.5 step 4; review finding conformance/F2).
    let still_due = still_due(&records, now);
    // Doc 20 cut SM.4a (§3.9): every open excursion's operator and rig by site, from the whole
    // scan and before the records move into the candidates — the heard sounding's listeners.
    let on_site = on_site(&records);
    // Doc 20 cut SM.11 (the reviewer's R1): every queue standing, `(tv, Queued::after)` — the
    // `WHY_MARKERS_QUEUED` told set keeps only these.
    let queues: BTreeSet<(u32, u32)> = records
        .iter()
        .filter_map(|record| {
            record
                .queued
                .as_ref()
                .map(|queued| (record.tv.0, queued.after))
        })
        .collect();
    // Doc 20 cut SM.11 (the brief's 3.4 "the caller gathers"): the open excursions by operator
    // and site, from the whole scan — a plan's launch's caps (a capped scan only under-counts,
    // which the start's own cap check then refuses).
    let open: Vec<(ObjId, mush_core::CanonicalId)> = records
        .iter()
        .filter_map(|record| {
            record
                .excursion
                .as_ref()
                .map(|excursion| (excursion.operator, excursion.site.clone()))
        })
        .collect();
    let (candidates, resting) = {
        let w: &World = ctx.w;
        // Doc 20 cut SM.12b: the one lessee reader (`yard::rig_lease`).
        let lessee = |tv: ObjId| crate::port::yard::rig_lease(w, tv).map(|lease| lease.pilot);
        let shown_plan = |tv: ObjId| plan_shown_of(w, tv);
        let helm = |pilot: ObjId| crate::surface::plan::helm_hull(w, pilot);
        candidates(
            records,
            now,
            |tv| told_of(w, tv),
            |tv| shown_of(w, tv),
            |tv| shown_run_of(w, tv),
            state,
            &PlanReads {
                lessee: &lessee,
                shown: &shown_plan,
                helm: &helm,
            },
        )
    };
    stats.passed_over = resting;
    let mut fire = Fire::new(BTreeMap::new());
    for candidate in candidates {
        let worked = usize::try_from(stats.worked).unwrap_or(usize::MAX);
        let submitted = usize::try_from(stats.submitted).unwrap_or(usize::MAX);
        if worked >= MAX_EXCURSIONS_WORKED_PER_FIRE || submitted >= MAX_EXCURSIONS_WORKED_PER_FIRE {
            break;
        }
        stats.visited = stats.visited.saturating_add(1);
        let record = &candidate.record;
        let tv = record.tv;
        // Tell before due, in the same visit.
        if candidate.untold
            && let Some(last) = &record.last
        {
            tell(ctx, &handles, record, last, &on_site);
            stats.told = stats.told.saturating_add(1);
            stats.worked = stats.worked.saturating_add(1);
        }
        // Doc 20 cut SM.11 (the brief's 3.8): the plan's untold notes and view.
        if candidate.plan_untold {
            tell_plan(ctx, record);
            stats.told = stats.told.saturating_add(1);
            stats.worked = stats.worked.saturating_add(1);
        }
        // Doc 20 cut SM.4b (decision P8): a started drive's leg, projected at once — and, doc 20
        // cut SM.7 (the lead's pre-commit round, item 4), a started run with it (`project_start`
        // writes the run beside the act; a run with no act in flight through `project_run`).
        if candidate.started && !candidate.due {
            if record.act.is_some() {
                project_start(ctx, record);
            } else {
                project_run(ctx, record);
            }
            stats.worked = stats.worked.saturating_add(1);
        }
        // The due item: the act's (cycle 0), or — doc 20 cut SM.7 — the run's cycle `k`.
        let due_item: Option<(u32, u8, ObjId, &'static str)> = if candidate.due {
            match (candidate.cycle, &record.act, &record.run) {
                (Some(k), _, Some(run)) => Some((run.ordinal, k, run.operator, "mine")),
                (None, Some(act), _) => Some((act.ordinal, 0, act.operator, act.kind.word())),
                (Some(_), _, None) | (None, None, _) => None,
            }
        } else {
            None
        };
        let step = match (due_item, handles.surface_done.as_ref()) {
            (Some((ordinal, cycle, operator, word)), Some(done)) => {
                let key = (tv.0, ordinal, cycle);
                let pending = ctx.authoritative_pending(operator);
                match attempt_visit(state, key, pending) {
                    Visit::PassOver => {
                        stats.passed_over = stats.passed_over.saturating_add(1);
                        Step::Held
                    }
                    Visit::Page => {
                        tracing::error!(
                            tv = tv.0,
                            act = ordinal,
                            cycle,
                            kind = word,
                            "tacsim: a surface act's finish has not landed after 8 tries \
                             (tacsim.excursion {}, act {}, cycle {}, {}); the rig and its hull \
                             stay held (TV_BUSY) until it lands; the tick retries every 60 s; \
                             staff read the finish's refusal in the engine log; a finish that \
                             lands clears it",
                            tv.0,
                            ordinal,
                            cycle,
                            word
                        );
                        stats.passed_over = stats.passed_over.saturating_add(1);
                        Step::Held
                    }
                    Visit::Submit => {
                        if cycle == 0 {
                            world_step(ctx, record);
                            project_due(ctx, record);
                        } else {
                            // A run never moves its rig: no placement, the projection only.
                            project_run(ctx, record);
                        }
                        let step = submit_done(
                            ctx,
                            &mut fire,
                            &mut state.paged,
                            done,
                            (tv, ordinal, cycle),
                            operator,
                        );
                        note_step(state, key, step);
                        match step {
                            Step::Submitted => {
                                stats.submitted = stats.submitted.saturating_add(1);
                                stats.worked = stats.worked.saturating_add(1);
                            }
                            Step::Deferred => stats.deferred = stats.deferred.saturating_add(1),
                            Step::Idle | Step::Worked | Step::Refused | Step::Held => {}
                        }
                        step
                    }
                }
            }
            (Some((ordinal, cycle, ..)), None) => {
                if state.paged.insert((tv.0, ordinal, cycle)) {
                    tracing::error!(
                        tv = tv.0,
                        "tacsim: a rig's act is due but the finish is not registered on this \
                         roster; it waits"
                    );
                }
                Step::Refused
            }
            // Doc 20 cut SM.8 (lead ruling 1; the brief's 3.9): a recover's queued unload, started
            // as the operator under its own key, by the attempt rule (a start that does not land
            // and leaves the queue standing counts an attempt).
            (None, _)
                if candidate.queued
                    && !crate::detachment::sortie::surface::unload_ready(ctx, record) =>
            {
                Step::Held
            }
            (None, _) if candidate.queued => {
                let queued = record.queued.as_ref();
                match (queued, handles.surface_act.as_ref()) {
                    (Some(queued), Some(act)) => {
                        let key = queued_attempt(record);
                        let pending = ctx.authoritative_pending(queued.operator);
                        match attempt_visit(state, key, pending) {
                            Visit::PassOver => {
                                stats.passed_over = stats.passed_over.saturating_add(1);
                                Step::Held
                            }
                            Visit::Page => {
                                tracing::error!(
                                    tv = tv.0,
                                    slot = queued.slot,
                                    "tacsim: rig {} (tacsim.excursion {}): its queued unload has \
                                     not started after 8 tries; the pilot's own +tv/{} unload or \
                                     dump supersedes it; the tick retries every 60 s",
                                    queued.slot,
                                    tv.0,
                                    queued.slot
                                );
                                stats.passed_over = stats.passed_over.saturating_add(1);
                                Step::Held
                            }
                            // Doc 20 cut SM.11 (the reviewer's R1): a start known to fail is
                            // never submitted — the hopper's ring read by the offer's own rule;
                            // while it is full the queue holds on its back-off (re-read every 60
                            // fires) and `WHY_MARKERS_QUEUED` is told once per queue.
                            Visit::Submit if markers_hold(ctx, state, record, queued) => {
                                stats.passed_over = stats.passed_over.saturating_add(1);
                                // Doc 20 cut SM.12c (the cost lane): one hold decision
                                // (`markers_hold`'s only caller, so this counts its true
                                // branch).
                                stats.markers_held = stats.markers_held.saturating_add(1);
                                Step::Held
                            }
                            Visit::Submit => {
                                let step = submit_queued(
                                    ctx,
                                    &mut fire,
                                    &mut state.paged,
                                    act,
                                    (tv, key.1),
                                    queued.operator,
                                );
                                note_step(state, key, step);
                                match step {
                                    Step::Submitted => {
                                        stats.submitted = stats.submitted.saturating_add(1);
                                        stats.worked = stats.worked.saturating_add(1);
                                    }
                                    Step::Deferred => {
                                        stats.deferred = stats.deferred.saturating_add(1);
                                    }
                                    Step::Idle | Step::Worked | Step::Refused | Step::Held => {}
                                }
                                step
                            }
                        }
                    }
                    (Some(_), None) => {
                        if state.paged.insert(queued_attempt(record)) {
                            tracing::error!(
                                tv = tv.0,
                                "tacsim: a rig's unload is queued but the start is not registered \
                                 on this roster; it waits"
                            );
                        }
                        Step::Refused
                    }
                    (None, _) => Step::Worked,
                }
            }
            // Doc 20 cut SM.11 (the brief's 3.8): the plan's next move, as its author.
            (None, _) if candidate.plan => {
                plan_step(ctx, &mut fire, state, &handles, record, &open, &mut stats)
            }
            // Doc 20 cut SM.11 (lead ruling 6): the idle return, as the operator.
            (None, _) if candidate.idle => {
                idle_step(ctx, &mut fire, state, &handles, record, &mut stats)
            }
            (None, _) => Step::Worked,
        };
        fire.settle(tv, step);
    }
    // The bookkeeping holds only acts still due; a capped scan prunes nothing.
    if complete {
        state.submitted.retain(|key| still_due.contains(key));
        state.attempts.retain(|key, _| still_due.contains(key));
        state.backoff.retain(|key, _| still_due.contains(key));
        state
            .paged
            .retain(|key| key.1 == 0 || still_due.contains(key));
        state.markers_told.retain(|key| queues.contains(key));
    }
    stats
}

/// One rig on a site's ground: its operator, the rig and its slot.
type OnSite = BTreeMap<mush_core::CanonicalId, Vec<(ObjId, ObjId, u8)>>;

/// **Every open excursion by site** (doc 20 cut SM.4a; §3.9): `(operator, tv, slot)` per rig
/// record with an excursion open, read from the whole scan.
fn on_site(records: &[RigRecord]) -> OnSite {
    let mut out: OnSite = BTreeMap::new();
    for record in records {
        if let Some(excursion) = &record.excursion {
            out.entry(excursion.site.clone()).or_default().push((
                excursion.operator,
                record.tv,
                excursion.slot,
            ));
        }
    }
    out
}

/// **Every act still due** among `records` at `now`: the keys the slot's retry bookkeeping
/// may keep. Read over the whole scan, never over the fire's visit cut (an act past
/// [`MAX_EXCURSIONS_VISITED_PER_FIRE`] is still due and keeps its attempts, its back-off and
/// its once-a-boot page; review finding conformance/F2).
fn still_due(records: &[RigRecord], now: i64) -> BTreeSet<ActKey> {
    let mut due = BTreeSet::new();
    for record in records {
        if let Some(act) = record.act.as_ref().filter(|act| act.due_ic <= now) {
            due.insert((record.tv.0, act.ordinal, 0));
        }
        // Doc 20 cut SM.7: a run's cycle in flight.
        if let Some(run) = record.run.as_ref().filter(|run| run.due_ic <= now) {
            due.insert((record.tv.0, run.ordinal, run.k));
        }
        // Doc 20 cut SM.8: a queued unload's start, so its attempts survive the prune.
        if record.queued.is_some() && record.act.is_none() && record.run.is_none() {
            due.insert(queued_attempt(record));
        }
        // Doc 20 cut SM.11: a live plan's next start (whatever its lessee: a capped visit must
        // not lose its bookkeeping) — past its bound with a queued unload standing too (the lead's
        // pre-commit round, item 3: its `Bound` end) — and an idle rig's return.
        let idle = record.act.is_none() && record.run.is_none();
        let quiet = idle && record.queued.is_none();
        if idle
            && record.plan.as_ref().is_some_and(|plan| {
                plan.live() && (record.queued.is_none() || plan.deadline_ic <= now)
            })
        {
            due.insert(plan_attempt(record));
        }
        if quiet && record.excursion.is_some() {
            due.insert(idle_attempt(record));
        }
    }
    due
}

/// The projection's told pair (`told`, `told_cycle`; doc 20 cut SM.7), `(0, 0)` when absent or
/// unreadable.
fn told_of(w: &World, tv: ObjId) -> (u32, u8) {
    match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    {
        Some(TvRecState::Present(rec)) => (rec.told, rec.told_cycle),
        Some(TvRecState::Unreadable) | None => (0, 0),
    }
}

/// The projection's act ordinal (doc 20 cut SM.4b), `None` when it shows no act or is absent or
/// unreadable.
fn shown_of(w: &World, tv: ObjId) -> Option<u32> {
    match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    {
        Some(TvRecState::Present(rec)) => rec.act.map(|act| act.ordinal),
        Some(TvRecState::Unreadable) | None => None,
    }
}

/// The projection's run, as its ordinal and cycle (doc 20 cut SM.7; the lead's pre-commit round,
/// item 4), `None` when it shows no run or is absent or unreadable.
fn shown_run_of(w: &World, tv: ObjId) -> Option<(u32, u8)> {
    match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    {
        Some(TvRecState::Present(rec)) => rec.run.as_ref().map(|run| (run.ordinal, run.k)),
        Some(TvRecState::Unreadable) | None => None,
    }
}

/// **The leg a record's act in flight carries, as the projection shows it** (doc 20 cut SM.4b;
/// §8): its waypoints from the leg's start — the act's own start, `LAUNCH_S` later when the
/// launch is implied. `None` for an act without a leg.
fn leg_view(record: &RigRecord) -> Option<FrozenLeg> {
    let act = record.act.as_ref()?;
    let (waypoints, _) = act.params.leg()?;
    let launch = if act.params.launches() {
        i64::from(LAUNCH_S)
    } else {
        0
    };
    Some(FrozenLeg {
        started_ic: act.started_ic.saturating_add(launch),
        waypoints: waypoints.to_vec(),
    })
}

/// **A started drive's projection** (doc 20 cut SM.4b; decision P8): the act view, its leg for
/// interpolation, and the state and pose as the placement reads now. Nothing else moves.
fn project_start(ctx: &mut Ctx<'_>, record: &RigRecord) {
    let Some(act) = &record.act else {
        return;
    };
    let excursion = record.excursion.as_ref().map(|excursion| excursion.id);
    let mut rec = projection_for(ctx.w, record.tv, act.slot, act.operator, excursion);
    rec.slot = act.slot;
    rec.operator = Some(act.operator);
    rec.hull = Some(act.hull);
    rec.excursion = excursion;
    rec.site = record
        .excursion
        .as_ref()
        .map(|excursion| excursion.site.clone());
    rec.state = projected_state(ctx.w, record, Some(act.hull));
    rec.pose = pose_of(ctx.w, record.tv);
    rec.act = Some(ActView {
        ordinal: act.ordinal,
        kind: act.kind,
        due_ic: act.due_ic,
    });
    rec.leg = leg_view(record);
    rec.run = record.run.as_ref().map(|run| Box::new(RunView::of(run)));
    rec.plan = plan_view_of(ctx.w, record);
    write_projection(ctx, &rec);
}

/// **A due cycle's projection** (doc 20 cut SM.7; the brief's 3.8): the run's view (and the act
/// in flight beside it, if any), the state and pose as the placement reads now — a run places
/// nothing, so `applied` does not move. Also a **started** run's (the SM.7 lead's pre-commit
/// round, item 4): written on the first fire after the start lands, before any cycle is due.
fn project_run(ctx: &mut Ctx<'_>, record: &RigRecord) {
    let Some(run) = &record.run else {
        return;
    };
    let excursion = record.excursion.as_ref().map(|excursion| excursion.id);
    let mut rec = projection_for(ctx.w, record.tv, run.slot, run.operator, excursion);
    rec.slot = run.slot;
    rec.operator = Some(run.operator);
    rec.hull = Some(run.hull);
    rec.excursion = excursion;
    rec.site = record
        .excursion
        .as_ref()
        .map(|excursion| excursion.site.clone());
    rec.state = projected_state(ctx.w, record, Some(run.hull));
    rec.pose = pose_of(ctx.w, record.tv);
    rec.act = record.act.as_ref().map(|act| ActView {
        ordinal: act.ordinal,
        kind: act.kind,
        due_ic: act.due_ic,
    });
    rec.leg = leg_view(record);
    rec.run = Some(Box::new(RunView::of(run)));
    rec.plan = plan_view_of(ctx.w, record);
    write_projection(ctx, &rec);
}

/// **The projection as stored for its current operator, else a new one** (doc 20 cut SM.9; the
/// brief's 3.4a, decision D16; doc 20 §5 and §16 — no surface shows another principal's
/// knowledge): `tacsim:tv:<tv>` is keyed by the TV and the counter's rigs are leased again and
/// again, so a stored record another operator wrote (its `operator` `Some(o)` with `o !=
/// operator`), or one written for another excursion (its `excursion` differs from `excursion`),
/// keeps **none of its log** — the last lessee's reading, arrival and cycle lines — and drops
/// its `leg` and `run` (every writer overwrites them anyway; clearing them makes the rule
/// plain). `applied`, `told`, `told_cycle`, `slot` and `tv` carry over: they are per-TV ordinals
/// the tick's idempotency reads (`told_of`, `shown_of`, `shown_run_of`), and clearing them would
/// re-tell or re-apply. A stored record no act ever wrote (`operator: None`) is taken as is.
pub(crate) fn projection_for(
    w: &World,
    tv: ObjId,
    slot: u8,
    operator: ObjId,
    excursion: Option<ExcursionId>,
) -> TvRec {
    match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    {
        Some(TvRecState::Present(rec)) => carried_for(rec.clone(), operator, excursion),
        Some(TvRecState::Unreadable) | None => TvRec::new(tv, slot),
    }
}

/// **[`projection_for`]'s rule over one stored record** (pure; doc 20 cut SM.9, D16): the log,
/// the leg and the run go when another operator or another excursion wrote it; the per-TV
/// ordinals stay.
fn carried_for(mut rec: TvRec, operator: ObjId, excursion: Option<ExcursionId>) -> TvRec {
    if let Some(was) = rec.operator
        && (was != operator || rec.excursion != excursion)
    {
        rec.log.clear();
        rec.leg = None;
        rec.run = None;
        rec.plan = None;
    }
    rec
}

/// The ground pose of `tv`, when it stands `Direct`.
pub(super) fn pose_of(w: &World, tv: ObjId) -> Option<TvPose> {
    match where_is(w, tv) {
        Where::Ground {
            x_cm,
            y_cm,
            heading_mdeg,
            ..
        } => Some(TvPose {
            x_cm,
            y_cm,
            heading_mdeg,
        }),
        Where::Bay { .. } | Where::Elsewhere => None,
    }
}

/// Write `rec` unless the store already holds it. **`true` when the store now holds it**
/// (already equal, or written); `false` on a `check` or serialise refusal (doc 20 cut SM.4a,
/// D24: the tell sends a heard line only on `true`).
pub(super) fn write_projection(ctx: &mut Ctx<'_>, rec: &TvRec) -> bool {
    // Doc 20 cut SM.4b (decision P7): a full leg and a full log do not fit 4 KiB together; the
    // oldest log lines go, never the leg.
    let mut rec = rec.clone();
    rec.fit();
    let same = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(rec.tv))
        .is_some_and(|state| *state == TvRecState::Present(rec.clone()));
    if same {
        return true;
    }
    if let Err(error) = rec.check() {
        tracing::error!(tv = rec.tv.0, %error, "tacsim: a rig projection is out of bounds; it is not written");
        return false;
    }
    // Doc 20 cut SM.9 (the brief's 3.4, decision D13): a projection that changed is one pushed
    // `tv.status` keyframe to its operator — the quarry screen's cue to pull the map again.
    // Nothing is pushed for an equal record (the early return above) or a refused one.
    let wrote = put_tv_rec(ctx, &rec);
    if wrote {
        crate::port::site_map::push_status(ctx, &rec);
    }
    wrote
}

/// **The due step's world write** (the brief's 3.5), idempotent by state: a launch places a
/// rig still in its hull's bay, with the hull still settled at the act's site, `Direct` on the
/// site's passive domain at the ramp pose; a recover places a rig standing within one cell of
/// the ramp back `Interior` in the bay's compartment; a re-run finds the placement already
/// there and writes nothing. A refusal is logged, never a panic; the finish then records
/// `Refused`.
fn world_step(ctx: &mut Ctx<'_>, record: &RigRecord) {
    let Some(act) = &record.act else {
        return;
    };
    if !crate::detachment::sortie::surface::working(
        ctx,
        record,
        matches!(
            act.kind,
            ActKind::Drive | ActKind::Recover | ActKind::Launch
        ),
    ) {
        return;
    }
    let tv = record.tv;
    let berth = hull_view_berth(ctx.w, act.hull);
    let Some((entry, domain, ramp, bay_room, bay_slot)) = berth else {
        return;
    };
    if entry != act.site {
        return;
    }
    // Doc 20 cut SM.4a (lead ruling 2): a scan or a sounding from the bay places its rig at the
    // ramp exactly as a launch does; one from the ground places nothing.
    let launches = act.kind == ActKind::Launch || act.params.launches();
    match act.kind {
        ActKind::Launch | ActKind::Scan | ActKind::Sound => {
            if launches
                && where_is(ctx.w, tv) == (Where::Bay { hull: act.hull })
                && let Err(error) = mush_sim::place(ctx, tv, domain, ramp)
            {
                tracing::warn!(tv = tv.0, %error, "tacsim: a launch's placement was refused; the finish records it");
            }
        }
        // Doc 20 cut SM.4b (§3.7 of the brief): the drive's one placement, at its due instant —
        // at the leg's end, from the bay straight there (the ramp half is inside the act), or
        // from the leg's start cell on the act's site domain. Anywhere else nothing (the finish
        // records `Moved`); at the end cell nothing is written again.
        ActKind::Drive => {
            let Some((waypoints, to)) = act.params.leg() else {
                return;
            };
            let start = waypoints
                .first()
                .map(|first| site_cell(first.x_cm, first.y_cm));
            let at_start = match where_is(ctx.w, tv) {
                Where::Bay { hull } => launches && hull == act.hull,
                Where::Ground {
                    domain: on,
                    x_cm,
                    y_cm,
                    ..
                } => on == domain && Some(site_cell(x_cm, y_cm)) == start,
                Where::Elsewhere => false,
            };
            let heading = match &act.params {
                crate::surface::excursion::ActParams::Drive { heading_mdeg, .. } => *heading_mdeg,
                _ => 0,
            };
            if at_start {
                match crate::surface::excursion::cell_pose(to, heading) {
                    Ok(pose) => {
                        if let Err(error) = mush_sim::place(ctx, tv, domain, pose) {
                            tracing::warn!(tv = tv.0, %error, "tacsim: a drive's placement was refused; the finish records it");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(tv = tv.0, %error, "tacsim: a drive's end does not read as a pose");
                    }
                }
            }
        }
        // Doc 20 cut SM.4b (§3.6 of the brief): a recover with a leg places the rig back in the
        // bay when it stands on the site's domain at its leg's start cell, at its end cell, or
        // within one cell of the ramp (SM.3b's rule) — idempotently.
        ActKind::Recover => {
            let leg = act.params.leg().map(|(waypoints, to)| {
                (
                    waypoints
                        .first()
                        .map(|first| site_cell(first.x_cm, first.y_cm)),
                    (to[0], to[1]),
                )
            });
            if let Where::Ground {
                domain: on,
                x_cm,
                y_cm,
                ..
            } = where_is(ctx.w, tv)
                && on == domain
            {
                let cell = site_cell(x_cm, y_cm);
                let beside = cells_apart(cell, site_cell(ramp.x_cm, ramp.y_cm)) <= 1;
                let on_leg = leg.is_some_and(|(start, end)| Some(cell) == start || cell == end);
                if (beside || on_leg)
                    && let Err(error) = mush_sim::place_interior(ctx, tv, bay_room, Some(bay_slot))
                {
                    tracing::warn!(tv = tv.0, %error, "tacsim: a recover's placement was refused; the finish records it");
                }
            }
        }
        // Doc 20 cut SM.7: a run never moves its rig (and never sits in the act slot); doc 20 cut
        // SM.8: an unload places nothing (the rig stays in the bay).
        ActKind::Dump | ActKind::Mine | ActKind::Unload => {}
    }
}

/// The hull's site berth, owned: `(entry, domain, ramp, bay room, bay slot)`. **Doc 23 cut
/// S.3** (A4): also how a staff home reads a second carrier (`crate::surface::recovery`).
#[allow(clippy::type_complexity)]
pub(crate) fn hull_view_berth(
    w: &World,
    hull: ObjId,
) -> Option<(
    mush_core::CanonicalId,
    mush_sim::DomainId,
    mush_sim::data::Pose,
    ObjId,
    mush_sim::SlotId,
)> {
    let view = crate::port::ship::hull_view(w, hull)?;
    let berth = site_berth(w, &view).ok()?;
    Some((
        berth.entry.clone(),
        berth.domain,
        berth.ramp.clone(),
        berth.bay_room,
        berth.bay_slot.clone(),
    ))
}

/// **The projected state, one rule for both writers** (the fixer round's item 6 and its
/// review's F7): `Bay` only with no excursion open and the rig aboard its hull's bay (`hull`
/// when known); a rig standing anywhere else, or one with an excursion open wherever it
/// stands, is `Out` whatever the placement says. Still a projection: it decides nothing. A
/// recovering rig therefore reads `Out` from the placement step until its finish lands and
/// is told, the record being the authority.
fn projected_state(w: &World, record: &RigRecord, hull: Option<ObjId>) -> TvState {
    match where_is(w, record.tv) {
        Where::Bay { hull: aboard }
            if record.excursion.is_none() && hull.is_none_or(|hull| hull == aboard) =>
        {
            TvState::Bay
        }
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => TvState::Out,
    }
}

/// **The projection beside the placement** (the brief's 3.5): `applied` is the act's
/// ordinal, the state ([`projected_state`]) and pose as the placement now reads, the act in
/// flight shown.
fn project_due(ctx: &mut Ctx<'_>, record: &RigRecord) {
    let Some(act) = &record.act else {
        return;
    };
    let excursion = record.excursion.as_ref().map(|excursion| excursion.id);
    let mut rec = projection_for(ctx.w, record.tv, act.slot, act.operator, excursion);
    rec.slot = act.slot;
    rec.operator = Some(act.operator);
    rec.hull = Some(act.hull);
    rec.excursion = excursion;
    rec.site = record
        .excursion
        .as_ref()
        .map(|excursion| excursion.site.clone());
    rec.state = projected_state(ctx.w, record, Some(act.hull));
    rec.pose = pose_of(ctx.w, record.tv);
    rec.act = Some(ActView {
        ordinal: act.ordinal,
        kind: act.kind,
        due_ic: act.due_ic,
    });
    // Doc 20 cut SM.4b: the placement just happened — the pose is the leg's end, and the leg
    // no longer moves anything.
    rec.leg = None;
    rec.run = record.run.as_ref().map(|run| Box::new(RunView::of(run)));
    rec.plan = plan_view_of(ctx.w, record);
    rec.applied = act.ordinal;
    write_projection(ctx, &rec);
}

/// **The finish, as the operator** (the brief's 3.5): one closed call through
/// `Fire::as_principal`; `AlreadyActed`, `Owed` and `Pending` defer (one act per principal per
/// fire, A9); `God` and `Gone` refuse, **logged once a boot** under the act's key in `paged`
/// (review finding conformance/F1: the act stays due and is visited every 1 s fire, so an
/// unguarded line would repeat each second for as long as the record stands). The key is the
/// one the attempt rule pages under; an act whose operator is GOD or gone never submits, so
/// it never counts an attempt and the two pages cannot meet. **Doc 20 cut SM.7** (the brief's
/// 3.8): a run's cycle `k` (`cycle > 0`) finishes under `tacsim.surface/{tv}/{ordinal}/cycle/{k}`
/// with the `cycle` parameter; an act's (cycle 0) as before.
fn submit_done(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    paged: &mut BTreeSet<ActKey>,
    done: &mush_core::DefinitionRef,
    (tv, ordinal, cycle): (ObjId, u32, u8),
    operator: ObjId,
) -> Step {
    let key = if cycle == 0 {
        done_key(tv, ordinal)
    } else {
        cycle_key(tv, ordinal, cycle)
    };
    let Some(key) = key else {
        tracing::error!(tv = tv.0, "tacsim: a finish's key did not form");
        return Step::Refused;
    };
    let parameters = if cycle == 0 {
        json!({ "tv": tv.0.to_string(), "ordinal": ordinal })
    } else {
        json!({ "tv": tv.0.to_string(), "ordinal": ordinal, "cycle": cycle })
    };
    match fire.as_principal(ctx, tv, operator, |ctx| {
        ctx.submit_authoritative(
            done.clone(),
            PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    }) {
        Ok(()) => Step::Submitted,
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => {
            if paged.insert((tv.0, ordinal, cycle)) {
                tracing::warn!(
                    tv = tv.0,
                    act = ordinal,
                    operator = operator.0,
                    "tacsim: a rig's operator is GOD or gone; its finish waits"
                );
            }
            Step::Refused
        }
    }
}

/// **Whether a queued unload is held on its hopper's full marker ring this visit** (doc 20 cut
/// SM.11, the reviewer's R1): `crate::surface::act::queued_ring_full` — the queued start's offer's
/// own rule, read through the fire's `Ctx` at the real second — says the start would be refused
/// `unload_markers`. Then nothing is submitted: the queue's attempts key enters its back-off (the
/// ring is read again after [`DONE_BACKOFF_FIRES`]; no attempt is counted, so no page), and the
/// operator is told `ACT_FAILED` with `WHY_MARKERS_QUEUED` **once per queue** (`markers_told`,
/// keyed `(tv, Queued::after)`), at trip level through the notice budget — said to a connected
/// pilot, queued for an offline one's reconnect. A queue the finish re-filed after its own
/// `MarkersFull` refusal (`queue_waits_on_markers`) was already told by that outcome's line and is
/// not told again.
fn markers_hold(
    ctx: &mut Ctx<'_>,
    state: &mut SurfaceTickState,
    record: &RigRecord,
    queued: &crate::surface::excursion::Queued,
) -> bool {
    let full = {
        let reads: &Ctx<'_> = ctx;
        crate::surface::act::queued_ring_full(reads.w, reads, record.tv, queued, reads.w.now())
    };
    if !full {
        return false;
    }
    state.backoff.insert(queued_attempt(record), state.fires);
    if state.markers_told.insert((record.tv.0, queued.after))
        && !crate::surface::plan::queue_waits_on_markers(record)
    {
        let line = crate::port::tv::ACT_FAILED
            .replace("{n}", &queued.slot.to_string())
            .replace("{act}", ActKind::Unload.word())
            .replace("{why}", crate::port::tv::WHY_MARKERS_QUEUED);
        crate::port_tick::notify_budget(
            ctx,
            queued.operator,
            &line,
            crate::port_tick::NoticeLevel::Trip,
        );
    }
    true
}

/// **A queued unload's start, as the operator** (doc 20 cut SM.8; lead ruling 1, decisions R1
/// and R15): one closed call through `Fire::as_principal` — `AlreadyActed`, `Owed` and `Pending`
/// defer (one submission an operator a fire, so the start lands on the fire after the recover is
/// told at the earliest); `God` and `Gone` refuse, logged once a boot under the queued key —
/// ONE `tacsim_surface_act` with `{ tv, act: unload, ordinal, how: queued }` keyed
/// `tacsim.surface/{tv}/{ordinal}/queued`, never `act_key` (a hand `go` for the same ordinal is a
/// distinct request; the prepare's ordinal check settles the race).
fn submit_queued(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    paged: &mut BTreeSet<ActKey>,
    act: &mush_core::DefinitionRef,
    (tv, ordinal): (ObjId, u32),
    operator: ObjId,
) -> Step {
    let Some(key) = queued_key(tv, ordinal) else {
        tracing::error!(tv = tv.0, "tacsim: a queued unload's key did not form");
        return Step::Refused;
    };
    let parameters = json!({
        "tv": tv.0.to_string(),
        "act": ActKind::Unload.word(),
        "ordinal": ordinal,
        "how": crate::surface::excursion::UnloadHow::Queued.word(),
    });
    match fire.as_principal(ctx, tv, operator, |ctx| {
        ctx.submit_authoritative(
            act.clone(),
            PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    }) {
        Ok(()) => Step::Submitted,
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => {
            if paged.insert((tv.0, ordinal, QUEUED_CYCLE)) {
                tracing::warn!(
                    tv = tv.0,
                    operator = operator.0,
                    "tacsim: a rig's operator is GOD or gone; its queued unload waits"
                );
            }
            Step::Refused
        }
    }
}

/// The projection's told plan and plan view (doc 20 cut SM.11), `(None, None)` when absent or
/// unreadable.
fn plan_shown_of(w: &World, tv: ObjId) -> (Option<PlanTold>, Option<PlanView>) {
    match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    {
        Some(TvRecState::Present(rec)) => (rec.plan_told, rec.plan.clone()),
        Some(TvRecState::Unreadable) | None => (None, None),
    }
}

/// **A start the tick submits as `operator`** (doc 20 cut SM.11; `submit_queued`'s shape): one
/// closed call through `Fire::as_principal` — `AlreadyActed`, `Owed` and `Pending` defer; `God`
/// and `Gone` refuse, logged once a boot under `paged_key`.
fn submit_start(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    paged: &mut BTreeSet<ActKey>,
    (action, tv, paged_key): (&mush_core::DefinitionRef, ObjId, ActKey),
    operator: ObjId,
    parameters: serde_json::Value,
    key: mush_core::IdempotencyKeyText,
) -> Step {
    match fire.as_principal(ctx, tv, operator, |ctx| {
        ctx.submit_authoritative(
            action.clone(),
            PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    }) {
        Ok(()) => Step::Submitted,
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => {
            if paged.insert(paged_key) {
                tracing::warn!(
                    tv = tv.0,
                    operator = operator.0,
                    "tacsim: a rig's plan author or operator is GOD or gone; its start waits"
                );
            }
            Step::Refused
        }
    }
}

/// Count a submission's step into the fire's stats.
fn count_step(stats: &mut SurfaceFireStats, step: Step) {
    match step {
        Step::Submitted => {
            stats.submitted = stats.submitted.saturating_add(1);
            stats.worked = stats.worked.saturating_add(1);
        }
        Step::Deferred => stats.deferred = stats.deferred.saturating_add(1),
        Step::Idle | Step::Worked | Step::Refused | Step::Held => {}
    }
}

/// **A plan's step at the tick** (doc 20 cut SM.11; the brief's 3.8): the rig as its lease names
/// it (never the author's helm list, the reviewer's F18), the caps' census from the scan, the
/// interpreter over the tick's own reads — then the start or the end submitted as the author under
/// SM.3b's attempt rule, or the wait cached (a charging wait until its instant, the others for
/// `PLAN_WAIT_IC_SECS`; a cold ground wanted) and told once a boot to a connected author.
#[allow(clippy::too_many_lines)] // one decision, then one submission or one wait
fn plan_step(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut SurfaceTickState,
    handles: &PortHandles,
    record: &RigRecord,
    open: &[(ObjId, mush_core::CanonicalId)],
    stats: &mut SurfaceFireStats,
) -> Step {
    use crate::surface::plan::{PLAN_WAIT_IC_SECS, PlanMove, PlanWait};
    let tv = record.tv;
    let now = ctx.w.world_ic_now().at;
    // Doc 20 cut SM.12b: the one lessee reader (`yard::rig_lease`): a yard rig's house view.
    let Some(lease) = crate::port::yard::rig_lease(ctx.w, tv) else {
        return Step::Worked;
    };
    let Some(plan) = record.live_plan_for(lease.pilot).cloned() else {
        return Step::Worked;
    };
    let rig = crate::surface::act::RigAt {
        crew: None,
        tv,
        slot: lease.slot,
        mark: lease.mark.clone(),
        hull: lease.hull,
        record: lease,
    };
    let mut counts = crate::surface::excursion::ExcursionCounts::default();
    for (operator, site) in open {
        counts.world = counts.world.saturating_add(1);
        if *site == plan.site {
            counts.site = counts.site.saturating_add(1);
        }
        if *operator == plan.author {
            counts.operator = counts.operator.saturating_add(1);
        }
    }
    // The SM.8 handoff (the fixer round's correctness/F3): a queued unload reaches the plan's arm
    // orphaned — the author's own, the author off the helm — and pages once a boot here; the plan
    // then reads its helm wait. (A plan past its bound reaches here with any queue standing — the
    // lead's pre-commit round, item 3 — and only an orphaned one pages.)
    if let Some(queued) = record.queued.as_ref()
        && crate::surface::plan::queue_orphaned(
            queued,
            &plan,
            crate::surface::plan::helm_hull(ctx.w, plan.author),
        )
        && state.paged.insert(queued_attempt(record))
    {
        tracing::error!(
            tv = tv.0,
            slot = queued.slot,
            plan = plan.no,
            "tacsim: rig {} (tacsim.excursion {}): its queued unload's operator #{} no longer \
             holds the hull's helm, so the unload is refused at every try and plan {} waits on \
             the helm; the queue clears when a start lands on the rig (the pilot back at the \
             helm, or the next lessee); staff reach a loaded hopper through the hull",
            queued.slot,
            tv.0,
            queued.operator.0,
            plan.no
        );
    }
    let tools = handles.surface_tools.clone().unwrap_or_default();
    let mut peeks = crate::surface::Peeks::default();
    let moved = {
        let w: &World = ctx.w;
        crate::surface::plan::decide(w, &*ctx, &rig, record, &counts, &tools, &mut peeks, now)
    };
    // Doc 20 cut SM.12c (the cost lane): the legs this step searched and the peeks it paid.
    add_peeks(stats, &peeks);
    let Some(moved) = moved else {
        return Step::Worked;
    };
    let Some(action) = handles.surface_act.clone() else {
        return Step::Refused;
    };
    let key = plan_attempt(record);
    let (parameters, idempotency) = match &moved {
        PlanMove::Start { word, step, .. } => {
            let ordinal = record.ordinal.saturating_add(1);
            (
                json!({
                    "tv": tv.0.to_string(),
                    "act": word,
                    "ordinal": ordinal,
                    "by": "plan",
                    "plan": plan.no,
                    "step": step,
                }),
                crate::surface::act::plan_step_key(tv, ordinal, *step, word),
            )
        }
        PlanMove::End { .. } => (
            json!({
                "tv": tv.0.to_string(),
                "act": "plan_end",
                "ordinal": record.ordinal.max(1),
                "by": "plan",
                "plan": plan.no,
                "step": plan.cursor,
            }),
            crate::surface::act::plan_end_key(tv, plan.no),
        ),
        PlanMove::Wait(wait) => {
            let until = match wait {
                PlanWait::Busy => return Step::Worked,
                PlanWait::Charging { until_ic } => *until_ic,
                PlanWait::Helm
                | PlanWait::Ashore
                | PlanWait::Cap
                | PlanWait::Cold
                | PlanWait::Dark
                | PlanWait::Markers => now.saturating_add(PLAN_WAIT_IC_SECS),
            };
            state.plan_waits.insert(tv, (plan.seq, until));
            if *wait == PlanWait::Cold {
                crate::surface::want_site_ground(ctx.w, &plan.site);
                if let Some((site, ramp)) = crate::surface::hull_home(ctx.w, plan.hull) {
                    crate::surface::want_home(ctx.w, &site, ramp);
                }
            }
            // Told once a boot per (rig, seq, reason), to a connected author only: nothing was
            // submitted, so nothing pages.
            if !matches!(wait, PlanWait::Charging { .. })
                && ctx.w.is_connected(plan.author)
                && state.plan_told_waits.insert((tv.0, plan.seq, wait.code()))
            {
                let words = crate::surface::act::PlanWords::of(ctx.w, &plan);
                let line = crate::port::tv::PLAN_WAITS
                    .replace("{n}", &words.n)
                    .replace(
                        "{why}",
                        &crate::surface::act::wait_why(ctx.w, *wait, &words),
                    );
                ctx.out.sys_to_actor(plan.author, "tacsim", &line);
            }
            return Step::Worked;
        }
    };
    let Some(idempotency) = idempotency else {
        tracing::error!(tv = tv.0, "tacsim: a plan step's key did not form");
        return Step::Refused;
    };
    let pending = ctx.authoritative_pending(plan.author);
    match attempt_visit(state, key, pending) {
        Visit::PassOver => {
            stats.passed_over = stats.passed_over.saturating_add(1);
            Step::Held
        }
        Visit::Page => {
            tracing::error!(
                tv = tv.0,
                plan = plan.no,
                "tacsim: rig {} (tacsim.excursion {}): plan {}'s next step has not started after \
                 8 tries; +tv/{} plan show says why; +tv/{} plan clear ends it; the tick retries \
                 every 60 s",
                plan.slot,
                tv.0,
                plan.no,
                plan.slot,
                plan.slot
            );
            stats.passed_over = stats.passed_over.saturating_add(1);
            Step::Held
        }
        Visit::Submit => {
            let step = submit_start(
                ctx,
                fire,
                &mut state.paged,
                (&action, tv, key),
                plan.author,
                parameters,
                idempotency,
            );
            note_step(state, key, step);
            count_step(stats, step);
            step
        }
    }
}

/// **The idle return at the tick** (doc 20 cut SM.11; lead ruling 6): the recover's world plan as
/// the open excursion's operator — its way home affordable: the recover submitted with `by: idle`
/// keyed `tacsim.surface/{tv}/{ordinal}/idle` under the attempt rule; its leg short of the charge:
/// **parked where it stands** — the projection reads `Parked` (no submission), the operator is told
/// `STRANDED` once (the projection's state is the de-dup) and the rig is tried no sooner than a day
/// later; the operator ashore: tried a day later, nothing told.
fn idle_step(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut SurfaceTickState,
    handles: &PortHandles,
    record: &RigRecord,
    stats: &mut SurfaceFireStats,
) -> Step {
    use crate::surface::act::Refusal;
    // Carrier teleoperation's idle recall is not an instruction to pilot a
    // physical detachment slot. Its operator (or actual NPC order driver)
    // chooses recovery through the normal admitted action.
    if record.field_crew.is_some() {
        return Step::Worked;
    }
    let tv = record.tv;
    let now = ctx.w.world_ic_now().at;
    let retry = now.saturating_add(crate::surface::plan::IDLE_PARK_IC_SECS);
    // Doc 20 cut SM.12b: the one lessee reader (`yard::rig_lease`), so a yard rig left idle on
    // the ground comes home.
    let (Some(lease), Some(excursion)) = (
        crate::port::yard::rig_lease(ctx.w, tv),
        record.excursion.clone(),
    ) else {
        return Step::Worked;
    };
    let operator = excursion.operator;
    let rig = crate::surface::act::RigAt {
        crew: None,
        tv,
        slot: lease.slot,
        mark: lease.mark.clone(),
        hull: lease.hull,
        record: lease,
    };
    let battery = handles.surface_tools.as_ref().and_then(|tools| {
        crate::surface::act::rig_battery_id(ctx.w, tv)
            .and_then(|id| crate::surface::act::battery_of(tools, &id).copied())
    });
    let rates = crate::surface::act::rig_rates(ctx.w, tv, battery.as_ref());
    let mut peeks = crate::surface::Peeks::default();
    let planned = {
        let w: &World = ctx.w;
        crate::surface::act::recover_plan(w, operator, &rig, &mut peeks, rates).and_then(|plan| {
            let ramp = ramp_label(w, rig.hull);
            crate::surface::act::leg_home(record, &plan, battery.as_ref(), &rig, &ramp)
                .map(|()| plan)
        })
    };
    // Doc 20 cut SM.12c (the cost lane): the idle return's peeks too (its recover plan searches
    // no leg through `decide_walk`, so `searches` gains nothing here; the site peeks count).
    add_peeks(stats, &peeks);
    match planned {
        Ok(_) => {}
        Err(Refusal::LegShort { .. } | Refusal::Far { .. }) => {
            parked_where_it_stands(ctx, record, &rig, &excursion);
            state.stranded.insert(tv, retry);
            return Step::Worked;
        }
        Err(Refusal::NotAboard { .. }) => {
            state.stranded.insert(tv, retry);
            return Step::Worked;
        }
        Err(Refusal::GroundCold { entry, .. }) => {
            crate::surface::want_site_ground(ctx.w, &entry);
            if let Some((site, ramp)) = crate::surface::hull_home(ctx.w, rig.hull) {
                crate::surface::want_home(ctx.w, &site, ramp);
            }
            return Step::Worked;
        }
        Err(_) => return Step::Worked,
    }
    let Some(action) = handles.surface_act.clone() else {
        return Step::Refused;
    };
    let ordinal = record.ordinal.saturating_add(1);
    let Some(idempotency) = crate::surface::act::idle_key(tv, ordinal) else {
        return Step::Refused;
    };
    let key = idle_attempt(record);
    let pending = ctx.authoritative_pending(operator);
    match attempt_visit(state, key, pending) {
        Visit::PassOver => {
            stats.passed_over = stats.passed_over.saturating_add(1);
            Step::Held
        }
        // The lead's pre-commit round, item 4 (the judge's correctness/F9): an idle return that
        // has not landed after 8 tries pages once a boot, as the plan's and the queue's arms do.
        Visit::Page => {
            tracing::error!(
                tv = tv.0,
                slot = rig.slot,
                operator = operator.0,
                "tacsim: rig {} (tacsim.excursion {}): its idle return has not started after 8 \
                 tries; the rig stays out on its site; the pilot's own +tv/{} recover brings it \
                 in; the tick retries every 60 s",
                rig.slot,
                tv.0,
                rig.slot
            );
            stats.passed_over = stats.passed_over.saturating_add(1);
            Step::Held
        }
        Visit::Submit => {
            let step = submit_start(
                ctx,
                fire,
                &mut state.paged,
                (&action, tv, key),
                operator,
                json!({
                    "tv": tv.0.to_string(),
                    "act": ActKind::Recover.word(),
                    "ordinal": ordinal,
                    "by": "idle",
                }),
                idempotency,
            );
            note_step(state, key, step);
            count_step(stats, step);
            step
        }
    }
}

/// **Parked where it stands** (doc 20 cut SM.11; lead ruling 6): the projection reads `Parked`;
/// the operator hears `STRANDED` once — only when the projection's state moved to `Parked` and the
/// write landed (the state is the de-dup, `told`'s order).
fn parked_where_it_stands(
    ctx: &mut Ctx<'_>,
    record: &RigRecord,
    rig: &crate::surface::act::RigAt,
    excursion: &crate::surface::excursion::Excursion,
) {
    let w: &World = ctx.w;
    let mut rec = projection_for(
        w,
        record.tv,
        rig.slot,
        excursion.operator,
        Some(excursion.id),
    );
    let already = rec.state == TvState::Parked;
    rec.slot = rig.slot;
    rec.operator = Some(excursion.operator);
    rec.hull = Some(rig.hull);
    rec.excursion = Some(excursion.id);
    rec.site = Some(excursion.site.clone());
    rec.state = TvState::Parked;
    rec.pose = pose_of(w, record.tv);
    rec.plan = plan_view_of(ctx.w, record);
    let line = crate::port::tv::STRANDED
        .replace("{n}", &rig.slot.to_string())
        .replace("{ramp}", &ramp_label(w, rig.hull))
        .replace("{site}", &site_name(w, &excursion.site));
    let written = write_projection(ctx, &rec);
    if written && !already {
        crate::port_tick::notify_budget(
            ctx,
            excursion.operator,
            &line,
            crate::port_tick::NoticeLevel::Trip,
        );
    }
}

/// **The hold's figures a plan's end names** (doc 20 cut SM.11; the brief's 3.8): what the hold
/// holds, whole tonnes by mass, and its unreserved room in whole tonnes of the plan's `family` of
/// ore in every measure — `crate::surface::plan::room_in_tonnes_of`, the compile's and the
/// interpreter's one rule (doc 20 cut SM.11r; it was the mass entry alone) — read at the tell
/// (`None` when either will not read).
fn hold_figures(
    ctx: &Ctx<'_>,
    hull: ObjId,
    family: crate::surface::geology::OreFamily,
) -> Option<(u64, u64)> {
    let w: &World = ctx.w;
    let view = crate::port::ship::hull_view(w, hull)?;
    let goods = w.module_state::<TacsimStore>()?.goods()?;
    let ore = mush_core::CanonicalId::new("metal_ore").ok()?;
    let tag = goods.good(&ore)?.stowage.clone();
    let mass = mush_core::CanonicalId::new(crate::definitions::logistics::MASS_KG).ok()?;
    let limit: u64 = crate::port::ship::compartments(&view)
        .into_iter()
        .filter(|(_, comp)| comp.accepts.contains(&tag))
        .filter_map(|(_, comp)| comp.limits.get(&mass).copied())
        .sum();
    let read = |endpoint: mush_inventory::record::EndpointRef, good: &mush_core::CanonicalId| {
        crate::port_tick::read_pool(ctx, endpoint, good)
    };
    let read_rig = |tv: ObjId| crate::port::site_map::SurfaceReads::rig(ctx, tv);
    let free = crate::port::ship::hold_unreserved_kg(w, &view, &[], &read, &read_rig)?;
    let rigs: Vec<ObjId> = crate::port::tv::live_on_hull(w, hull)
        .iter()
        .map(|lease| lease.tv)
        .collect();
    let room = crate::port::ship::hold_unreserved_with(goods, w, &view, &rigs, &read, &read_rig)?;
    let room_t = crate::surface::plan::room_in_tonnes_of(goods, &room, family)?;
    Some((limit.saturating_sub(free) / 1_000, room_t))
}

/// **A plan's note in words** (doc 20 cut SM.11; the brief's 3.8): a pass (`PLAN_PASSED`, live), a
/// parking (`PLAN_PARKING`, a trip) or the end with its summary (`PLAN_ENDED`, a trip).
fn plan_note_line(
    ctx: &Ctx<'_>,
    plan: &crate::surface::plan::FillPlan,
    note: &crate::surface::plan::PlanNoteKind,
) -> (String, crate::port_tick::NoticeLevel) {
    use crate::port::tv;
    use crate::port_tick::NoticeLevel;
    use crate::surface::plan::PlanNoteKind;
    let w: &World = ctx.w;
    let words = crate::surface::act::PlanWords::of(w, plan);
    match note {
        PlanNoteKind::Passed { at, field, why } => {
            let addr = if *field {
                address::field_name(at[0], at[1]).unwrap_or_default()
            } else {
                address::pocket_address(at[0], at[1]).unwrap_or_default()
            };
            (
                tv::PLAN_PASSED
                    .replace("{n}", &words.n)
                    .replace("{addr}", &addr)
                    .replace("{why}", &crate::surface::act::pass_why(*why, plan.family)),
                NoticeLevel::Live,
            )
        }
        PlanNoteKind::Parking { why } => (
            tv::PLAN_PARKING.replace("{n}", &words.n).replace(
                "{why}",
                &crate::surface::act::end_why(w, plan.author, *why, &words),
            ),
            NoticeLevel::Trip,
        ),
        PlanNoteKind::Ended { why } => {
            let (held, room) = hold_figures(ctx, plan.hull, plan.family).unwrap_or((0, 0));
            (
                tv::PLAN_ENDED
                    .replace("{n}", &words.n)
                    .replace("{site}", &words.site)
                    .replace(
                        "{why}",
                        &crate::surface::act::end_why(w, plan.author, *why, &words),
                    )
                    .replace("{t}", &plan.tally.tonnes.to_string())
                    .replace(
                        "{ore}",
                        &tv::ore_words(crate::surface::opening::ore_good(plan.family)),
                    )
                    .replace(
                        "{c}",
                        &tv::count_words(u64::from(plan.tally.cycles), "cycle", "cycles"),
                    )
                    .replace(
                        "{trips}",
                        &tv::count_words(u64::from(plan.tally.trips), "trip", "trips"),
                    )
                    .replace("{name}", &words.name)
                    .replace("{held}", &held.to_string())
                    .replace("{room}", &room.to_string()),
                NoticeLevel::Trip,
            )
        }
    }
}

/// **A plan's tell** (doc 20 cut SM.11; the brief's 3.8): the projection's plan view and its told
/// pair moved to the record's, then each note the projection had not told — in order — said to a
/// connected author, or queued under the notice budget for an offline one, **only once the
/// projection's write succeeded** (SM.4a's D24 order, so a retold note never doubles). A record
/// with no plan clears the projection's view.
fn tell_plan(ctx: &mut Ctx<'_>, record: &RigRecord) {
    let w: &World = ctx.w;
    let stored: Option<TvRec> = match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(record.tv))
    {
        Some(TvRecState::Present(rec)) => Some(rec.clone()),
        Some(TvRecState::Unreadable) | None => None,
    };
    let Some(stored) = stored else {
        // No projection yet: start one for the rig as its plan's author would see it.
        let Some(plan) = record.plan.as_ref() else {
            return;
        };
        let mut rec = TvRec::new(record.tv, plan.slot);
        rec.operator = Some(plan.author);
        rec.hull = Some(plan.hull);
        rec.state = state_for(w, record, Some(plan.hull), None);
        rec.pose = pose_of(w, record.tv);
        rec.plan = plan_view_of(w, record);
        rec.plan_told = Some(PlanTold {
            no: plan.no,
            seq: plan.seq,
        });
        let lines: Vec<(String, crate::port_tick::NoticeLevel)> = plan
            .notes
            .iter()
            .map(|note| plan_note_line(ctx, plan, &note.note))
            .collect();
        if write_projection(ctx, &rec) {
            for (line, level) in lines {
                crate::port_tick::notify_budget(ctx, plan.author, &line, level);
            }
        }
        return;
    };
    let mut rec = stored.clone();
    rec.plan = plan_view_of(ctx.w, record);
    let mut lines: Vec<(String, crate::port_tick::NoticeLevel)> = Vec::new();
    let mut author = None;
    if let Some(plan) = record.plan.as_ref() {
        let told = stored.plan_told.filter(|told| told.no == plan.no);
        for note in &plan.notes {
            if told.is_none_or(|told| note.seq > told.seq) {
                lines.push(plan_note_line(ctx, plan, &note.note));
            }
        }
        rec.plan_told = Some(PlanTold {
            no: plan.no,
            seq: plan.seq,
        });
        rec.state = state_for(w, record, rec.hull.or(Some(plan.hull)), Some(stored.state));
        author = Some(plan.author);
    }
    if write_projection(ctx, &rec)
        && let Some(author) = author
    {
        for (line, level) in lines {
            crate::port_tick::notify_budget(ctx, author, &line, level);
        }
    }
}

/// **The projected state with `Parked`** (doc 20 cut SM.11; lead ruling 6): [`projected_state`],
/// except a rig on the ground with nothing in flight reads `Parked` when its plan ended `Stranded`
/// with no act since, or when the stored projection already says `Parked` and no plan is live (an
/// idle rig the tick found short of its way home).
fn state_for(
    w: &World,
    record: &RigRecord,
    hull: Option<ObjId>,
    stored: Option<TvState>,
) -> TvState {
    use crate::surface::plan::{FillState, PlanEnd};
    let base = projected_state(w, record, hull);
    let quiet = record.act.is_none() && record.run.is_none() && record.queued.is_none();
    let on_ground = matches!(where_is(w, record.tv), Where::Ground { .. });
    if base != TvState::Out || !quiet || !on_ground {
        return base;
    }
    let stranded = record.plan.as_ref().is_some_and(|plan| match plan.state {
        FillState::Ended {
            why: PlanEnd::Stranded,
            at_ic,
        } => record.last.as_ref().is_none_or(|last| at_ic >= last.at_ic),
        FillState::Ended { .. } | FillState::Live | FillState::Parking { .. } => false,
    });
    let idle_parked =
        stored == Some(TvState::Parked) && record.plan.as_ref().is_none_or(|plan| !plan.live());
    if stranded || idle_parked {
        TvState::Parked
    } else {
        base
    }
}

/// **The tell** (the brief's 3.5): the projection (`told` moved to the last act, the state
/// and pose as they stand, one log line stamped at the act's own instant) and the completion
/// line to the operator.
///
/// **Doc 20 cut SM.4a**: a finished scan's or sounding's completion is a multi-line block
/// rendered from the operator's chart row restricted to the act's span (`reading_block`); the
/// projection logs its first line (cut to `MAX_TV_LOG_BYTES`) and the operator hears the whole
/// block. A loud sounding the site heard reaches its listeners (`heard_lines`) **only when the
/// projection write succeeded** (D24): a refused write leaves `told` where it was, so the next
/// fire tells the operator again (at-least-once, as before) but never re-broadcasts until the
/// write lands. The residual: a crash between the in-memory write and its persistence may
/// repeat one heard line once after a restart.
fn tell(
    ctx: &mut Ctx<'_>,
    handles: &PortHandles,
    record: &RigRecord,
    last: &Outcome,
    on_site: &OnSite,
) {
    let w: &World = ctx.w;
    let excursion = record.excursion.as_ref().map(|excursion| excursion.id);
    let mut rec = projection_for(w, record.tv, last.slot, last.operator, excursion);
    let stored_state = rec.state;
    let hull = record
        .excursion
        .as_ref()
        .map(|excursion| excursion.hull)
        .or(rec.hull)
        // Doc 20 cut SM.12b: the one lessee reader (`yard::rig_lease`).
        .or_else(|| crate::port::yard::rig_lease(w, record.tv).map(|lease| lease.hull));
    let line = match (&last.result, &last.span, last.arrived, &last.mined) {
        // Doc 20 cut SM.7 (§1 beat 8's line; the brief's 3.8): a mining cycle's line.
        (_, _, _, Some(mined)) => mine_line(ctx, handles, record.tv, last, mined),
        (ActResult::Done, Some(span), _, None) => reading_block(ctx, handles, record, last, span),
        // Doc 20 cut SM.4b (§3.7 of the brief): a drive's arrival, with the operator's own
        // reading of the pocket it ends in.
        (ActResult::Done, None, Some(cell), None) if last.kind == ActKind::Drive => {
            arrival_line(ctx, handles, last, cell)
        }
        _ => completion_line(ctx, record, last, hull),
    };
    // Doc 20 cut SM.4b: a finished drive asks for its site's ground and home once more (a no-op
    // when warm), so the next act from the new stand peeks them warm.
    let wants = (last.kind == ActKind::Drive && last.result == ActResult::Done)
        .then(|| hull.and_then(|hull| crate::surface::hull_home(w, hull)))
        .flatten();
    let log_line = line.lines().next().unwrap_or_default().to_owned();
    rec.slot = last.slot;
    rec.operator = Some(last.operator);
    rec.hull = hull;
    rec.excursion = excursion;
    rec.site = record
        .excursion
        .as_ref()
        .map(|excursion| excursion.site.clone());
    rec.state = projected_state(w, record, hull);
    rec.pose = pose_of(w, record.tv);
    rec.act = record.act.as_ref().map(|act| ActView {
        ordinal: act.ordinal,
        kind: act.kind,
        due_ic: act.due_ic,
    });
    // Doc 20 cut SM.4b: the leg of the act in flight now, if it carries one.
    rec.leg = leg_view(record);
    // Doc 20 cut SM.7: the run in flight now, and the told pair (decision R3).
    rec.run = record.run.as_ref().map(|run| Box::new(RunView::of(run)));
    (rec.told, rec.told_cycle) = told_pair(last);
    rec.push_log(last.at_ic, &log_line);
    // Doc 20 cut SM.11: the plan's view, and a rig whose plan ended stranded reads `Parked`.
    rec.plan = plan_view_of(ctx.w, record);
    rec.state = state_for(w, record, hull, Some(stored_state));
    let written = write_projection(ctx, &rec);
    // Doc 20 cut SM.11 (lead ruling 7): the notice budget — offline, only a trip-level line.
    crate::port_tick::notify_budget(ctx, last.operator, &line, level_of(last));
    if let Some((site, ramp)) = wants {
        crate::surface::want_home(ctx.w, &site, ramp);
    }
    if let Some(cell) = heard_from(written, last) {
        heard_lines(ctx, last, cell, on_site);
    }
    // Doc 20 cut SM.7 (§6; the brief's 3.10, decision R12): a cycle that moved ore is one
    // `tacsim.surface.mined` fact — only once the projection's write landed (SM.4a's D24 order),
    // so a retold cycle never doubles.
    if written && let Some(fact) = mined_fact(record.tv, last) {
        push_mined(ctx, &last.site, fact);
    }
    // The SM.7 lead's pre-commit round, item 3: a run the registry's dark read ended is paged
    // once — with the told cycle, only once the projection's write landed (as the ring's fact),
    // so a retold cycle never pages twice.
    if written
        && let Some(Mined {
            end: Some(RunEnd::Dark { why }),
            pocket,
            ..
        }) = &last.mined
    {
        tracing::error!(
            site = %last.site,
            tv = record.tv.0,
            pocket = %address::pocket_address(pocket[0], pocket[1]).unwrap_or_default(),
            block = %address::block_letter(pocket[0], pocket[1]).unwrap_or('?'),
            cause = why.words(),
            "tacsim: a mining run ended because its block's registry could not be read ({}); \
             the operator was told the registry is dark; staff repair the read before the \
             block is mined again",
            why.words()
        );
    }
}

/// **A completion line's notice level** (doc 20 cut SM.11; lead ruling 7, the brief's 3.8): a
/// trip-level line — an unload (`UNLOADED`, `UNLOADED_NONE`), a refused act or cycle
/// (`ACT_FAILED`, `CYCLE_FAILED`), a run ended by a right or the dark registry (`RUN_RIGHTS`,
/// `RUN_DARK`), the idle return (`IDLE_HOME`) — is queued for an offline operator; every other line
/// (a launch, a recover, a dump, a reading, an arrival, a cycle, a core) is said only.
#[must_use]
pub(crate) fn level_of(last: &Outcome) -> crate::port_tick::NoticeLevel {
    use crate::port_tick::NoticeLevel;
    use crate::surface::plan::ActBy;
    if let ActResult::Refused { .. } = last.result {
        return NoticeLevel::Trip;
    }
    if let Some(mined) = &last.mined {
        return match mined.end {
            Some(
                RunEnd::Licence { .. }
                | RunEnd::Claimed
                | RunEnd::Lease { .. }
                | RunEnd::Dark { .. },
            ) => NoticeLevel::Trip,
            Some(RunEnd::Done | RunEnd::WorkedOut | RunEnd::Stopped | RunEnd::Refused) | None => {
                NoticeLevel::Live
            }
        };
    }
    match (last.kind, last.by) {
        (ActKind::Unload, _) | (ActKind::Recover, Some(ActBy::Idle)) => NoticeLevel::Trip,
        (
            ActKind::Launch
            | ActKind::Recover
            | ActKind::Dump
            | ActKind::Scan
            | ActKind::Sound
            | ActKind::Drive
            | ActKind::Mine,
            _,
        ) => NoticeLevel::Live,
    }
}

/// **The fact a told cycle publishes** (doc 20 cut SM.7; decision R12): a `Done` cycle that moved
/// at least a tonne; `None` for everything else (a barren core never reaches the ring).
fn mined_fact(tv: ObjId, last: &Outcome) -> Option<crate::surface::mined::MinedFact> {
    let mined = last.mined.as_ref()?;
    if last.result != ActResult::Done || mined.tonnes == 0 {
        return None;
    }
    Some(crate::surface::mined::MinedFact {
        at_ic: last.at_ic,
        tv,
        operator: last.operator,
        pocket: address::pocket_address(mined.pocket[0], mined.pocket[1])?,
        good: mined.good.clone(),
        tonnes: mined.tonnes,
        ordinal: last.ordinal,
        k: mined.k,
    })
}

/// Push `fact` into `site`'s ring and write it (a fact already held, or older than a full ring,
/// writes nothing).
fn push_mined(
    ctx: &mut Ctx<'_>,
    site: &mush_core::CanonicalId,
    fact: crate::surface::mined::MinedFact,
) {
    use crate::surface::mined::{MinedRing, MinedState, push};
    let mut ring = match ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.mined(site))
    {
        Some(MinedState::Present(ring)) => ring.clone(),
        Some(MinedState::Unreadable) | None => MinedRing::new(site.clone()),
    };
    if push(&mut ring, fact) && ring.check().is_ok() {
        let _ = crate::store::put_mined(ctx, &ring);
    }
}

/// **A mining cycle's line** (doc 20 §1 beat 8; cut SM.7, the brief's 3.6): `CYCLE_DONE` with the
/// hopper read now and, at the run's end, its clause (`RUN_DONE`, `RUN_WORKED_OUT`,
/// `RUN_STOPPED`, `RUN_DARK`, or `RUN_RIGHTS` with the public line formatted here from the stored
/// standing);
/// a blind core `CORED` with the operator's own new reading (the address alone when their row is
/// gone) and no end clause; a cycle that did not happen `CYCLE_FAILED`.
fn mine_line(
    ctx: &Ctx<'_>,
    handles: &PortHandles,
    tv: ObjId,
    last: &Outcome,
    mined: &Mined,
) -> String {
    use crate::port::tv;
    let w: &World = ctx.w;
    let n = last.slot.to_string();
    let ore = tv::ore_words(mined.good.as_str());
    let pocket = (mined.pocket[0], mined.pocket[1]);
    let addr = address::pocket_address(pocket.0, pocket.1).unwrap_or_default();
    let hopper = match tv::hopper_load_of(ctx, tv) {
        Ok((kg, cap)) => tv::hopper_text(kg, cap),
        Err(error) => {
            tracing::warn!(tv = tv.0, %error, "tacsim: a cycle's tell could not read the hopper");
            "?".to_owned()
        }
    };
    if let ActResult::Refused { code } = last.result {
        let why = match code {
            RefusedCode::Unworked => tv::WHY_UNWORKED.to_owned(),
            RefusedCode::Moved => tv::WHY_MOVED.to_owned(),
            RefusedCode::NeverLeft => tv::WHY_NEVER_LEFT.to_owned(),
            RefusedCode::NotBeside => tv::WHY_NOT_BESIDE.replace("{ramp}", "ramp"),
            RefusedCode::HopperChanged => tv::WHY_HOPPER_CHANGED.to_owned(),
            RefusedCode::SpoilGone => tv::WHY_SPOIL_GONE.to_owned(),
            RefusedCode::LeftBay => tv::WHY_LEFT_BAY.to_owned(),
            RefusedCode::HoldRefused => tv::WHY_HOLD_REFUSED.to_owned(),
            RefusedCode::MarkersFull => tv::WHY_MARKERS_FULL.to_owned(),
        };
        return tv::CYCLE_FAILED
            .replace("{n}", &n)
            .replace("{k}", &mined.k.to_string())
            .replace("{cycles}", &mined.cycles.to_string())
            .replace("{addr}", &addr)
            .replace("{why}", &why);
    }
    if mined.blind {
        let key = ChartKey {
            principal: last.operator,
            site: last.site.clone(),
        };
        let row: Option<ChartRecord> = ctx
            .action_state(&handles.families.charts, &key)
            .ok()
            .flatten()
            .cloned()
            .filter(|row| row.key() == key);
        let reading = if crate::surface::chart::row_reads_pocket(row.as_ref(), pocket) {
            crate::surface::chart::pocket_reading(w, &last.site, row.as_ref(), pocket)
        } else {
            addr.clone()
        };
        let lift = if mined.tonnes == 0 {
            tv::NOTHING_LIFTED.to_owned()
        } else {
            format!("{} t {ore}", mined.tonnes)
        };
        return tv::CORED
            .replace("{n}", &n)
            .replace("{reading}", &reading)
            .replace("{lift}", &lift)
            .replace("{hopper}", &hopper);
    }
    let rights = |line: &str| {
        tv::RUN_RIGHTS
            .replace("{n}", &n)
            .replace("{addr}", &addr)
            .replace("{line}", line)
    };
    let end = match mined.end {
        None | Some(RunEnd::Refused) => String::new(),
        Some(RunEnd::Done) => tv::RUN_DONE.replace("{addr}", &addr),
        Some(RunEnd::WorkedOut) => tv::RUN_WORKED_OUT
            .replace("{addr}", &addr)
            .replace("{ore}", &ore),
        Some(RunEnd::Stopped) => tv::RUN_STOPPED.replace("{n}", &n),
        Some(RunEnd::Licence { why }) => rights(&crate::surface::act::licence_end_line(
            w,
            last.operator,
            why,
        )),
        Some(RunEnd::Dark { .. }) => tv::RUN_DARK.replace("{addr}", &addr),
        Some(RunEnd::Claimed) => rights(tv::MINE_CLAIMED),
        Some(RunEnd::Lease { forfeit }) => {
            let (counter, port) = tv::counter_words(w);
            rights(
                &if forfeit {
                    tv::MINE_FORFEIT
                } else {
                    tv::MINE_ENDED
                }
                .replace("{n}", &n)
                .replace("{counter}", &counter)
                .replace("{port}", &port),
            )
        }
    };
    let mut line = tv::CYCLE_DONE
        .replace("{n}", &n)
        .replace("{k}", &mined.k.to_string())
        .replace("{cycles}", &mined.cycles.to_string())
        .replace("{t}", &mined.tonnes.to_string())
        .replace("{ore}", &ore)
        .replace("{hopper}", &hopper);
    line.push_str(&end);
    line
}

/// **Whether the tell sends a heard line, and from where** (doc 20 cut SM.4a, D24; pure so it is
/// pinned on its own — the review's conformance/F3: since cut SM.4b fits the projection, no
/// world padding can refuse the write any more): the sounder's stand for a `Done` loud sounding
/// the site heard, **and only once the projection write landed** (`written`) — a refused write
/// holds the line back until the next fire's write lands, so a listener never hears it twice.
fn heard_from(written: bool, last: &Outcome) -> Option<[i64; 2]> {
    match &last.span {
        Some(ReadSpan::Sound {
            cell, heard: true, ..
        }) if written && last.result == ActResult::Done => Some(*cell),
        Some(ReadSpan::Sound { .. } | ReadSpan::Scan { .. }) | None => None,
    }
}

/// **A finished scan's or sounding's block** (doc 20 cut SM.4a; the brief's 3.8, D17, D26):
/// `SCANNED` (or `SCANNED_ICE`) with one row per field of the span, or `SOUNDED` (or
/// `SOUNDED_BARREN`) with one line per family with ore — rendered from the operator's chart row
/// for the site, restricted to this act's span, so a row the act sharpened reads at its
/// sharpest. The row absent (forgotten between the finish and the tell), unreadable or under
/// another rule → `READINGS_GONE` alone; the row present but the site's ground dark at the tell
/// → `READINGS_DARK` alone (the readings wait on the chart) — each with one `tracing::info!`
/// naming the rig and the act, and never a reading rendered from the geology without the row.
fn reading_block(
    ctx: &Ctx<'_>,
    handles: &PortHandles,
    record: &RigRecord,
    last: &Outcome,
    span: &ReadSpan,
) -> String {
    use crate::port::tv;
    let n = last.slot.to_string();
    let gone = || {
        tracing::info!(
            tv = record.tv.0,
            act = last.ordinal,
            "tacsim: a finished reading's chart row is gone, unreadable or under another rule; \
             the operator is told so"
        );
        tv::READINGS_GONE
            .replace("{n}", &n)
            .replace("{act}", last.kind.word())
    };
    let key = ChartKey {
        principal: last.operator,
        site: last.site.clone(),
    };
    let row: Option<ChartRecord> = match ctx.action_state(&handles.families.charts, &key) {
        Ok(found) => found.cloned(),
        Err(error) => {
            tracing::warn!(tv = record.tv.0, %error, "tacsim: a chart row does not read");
            None
        }
    };
    let Some(row) = row.filter(|row| row.rule == READING_RULE_VERSION) else {
        return gone();
    };
    // The row stands but the site's ground cannot be read now (the layer dark, no body table,
    // the spec unreadable): the readings are on the chart, so the operator is told that, never
    // `READINGS_GONE` (the SM.4a fixer round's finding F2; D26 names only a row that is gone).
    let Ok(site) = crate::surface::chart::site_context(ctx.w, &last.site) else {
        tracing::info!(
            tv = record.tv.0,
            act = last.ordinal,
            "tacsim: a finished reading's site ground cannot be read at the tell; the operator \
             is told its readings wait on the chart"
        );
        return tv::READINGS_DARK
            .replace("{n}", &n)
            .replace("{act}", last.kind.word());
    };
    match span {
        ReadSpan::Scan {
            footing,
            fields,
            hidden,
            ..
        } => {
            if *footing == Footing::Ice {
                return tv::SCANNED_ICE
                    .replace("{n}", &n)
                    .replace("{count}", &fields.len().to_string());
            }
            let rows = crate::surface::chart::scan_rows(&site, &row, fields);
            if rows.is_empty() {
                return gone();
            }
            let hidden_words = if hidden.is_empty() {
                String::new()
            } else {
                tv::HIDDEN_FIELDS.replace("{k}", &hidden.len().to_string())
            };
            let mut block = vec![
                tv::SCANNED
                    .replace("{n}", &n)
                    .replace("{count}", &fields.len().to_string())
                    .replace("{footing}", footing.name())
                    .replace("{hidden}", &hidden_words),
            ];
            block.extend(rows);
            block.join("\n")
        }
        ReadSpan::Sound {
            cell,
            radius,
            quiet,
            ..
        } => {
            let stand = (cell[0], cell[1]);
            let pockets = address::pockets_in_reach(stand, i64::from(*radius));
            if !pockets.iter().any(|pocket| row.pocket_sounded(*pocket)) {
                return gone();
            }
            let addr = address::pocket_of_cell(stand)
                .and_then(|(px, py)| address::pocket_address(px, py))
                .unwrap_or_default();
            let quietly = if *quiet { " quietly" } else { "" };
            let lines = crate::surface::chart::sound_lines(&site, &row, &pockets);
            let head = if lines.is_empty() {
                tv::SOUNDED_BARREN
            } else {
                tv::SOUNDED
            }
            .replace("{n}", &n)
            .replace("{count}", &pockets.len().to_string())
            .replace("{addr}", &addr)
            .replace("{quietly}", quietly);
            let mut block = vec![head];
            block.extend(lines);
            block.join("\n")
        }
    }
}

/// **The heard sounding** (doc 20 cut SM.4a; §5, Q-20-9; the brief's 3.9, D7, D8, D19): every
/// party with a rig or a hull on the sounding's site — the operators of every rig record with an
/// open excursion there (`on_site`) and the helm holder of every programme hull docked on its
/// pad — deduplicated, **minus the sounder**, minus anyone whose quiet preference is set, and
/// **connected now** (a heard line is a transient perception, never queued) hears exactly one
/// line: `HEARD` with the octant word of (the sounder's cell − the listener's position) and the
/// range band of the Chebyshev cells between them, or `HEARD_HERE` on the sounder's own cell.
/// The listener's position is their lowest-slot rig on that site's ground, else the pad centre
/// cell. Nothing else reaches a listener: no family, no tier, no name, no address, no chart or
/// projection write.
fn heard_lines(ctx: &mut Ctx<'_>, last: &Outcome, sounder: [i64; 2], on_site: &OnSite) {
    let w: &World = ctx.w;
    let site = &last.site;
    let mut listeners: BTreeMap<ObjId, (i64, i64)> = BTreeMap::new();
    let pad = {
        let centre = crate::surface::excursion::PAD_CENTRE_CM;
        site_cell(centre, centre)
    };
    // Rig operators on the site: the position of their lowest-slot rig standing on its ground.
    let domain = site_domain(w, site);
    let mut rigs: Vec<(ObjId, u8, ObjId)> = on_site
        .get(site)
        .map(|rows| {
            rows.iter()
                .map(|(operator, tv, slot)| (*operator, *slot, *tv))
                .collect()
        })
        .unwrap_or_default();
    rigs.sort_unstable();
    for (operator, _, tv) in &rigs {
        if let Where::Ground {
            domain: on,
            x_cm,
            y_cm,
            ..
        } = where_is(w, *tv)
            && Some(on) == domain
        {
            listeners
                .entry(*operator)
                .or_insert_with(|| site_cell(x_cm, y_cm));
        }
    }
    // An operator whose rigs on the site all stand off its ground hears from the pad.
    for (operator, _, _) in &rigs {
        listeners.entry(*operator).or_insert(pad);
    }
    // The helm holders of programme hulls docked on the site's pad.
    if let Some(port) = crate::port::coop::programme_port(w) {
        for row in &port.map.hulls {
            let hull = ObjId(row.hull);
            let docked_here = matches!(
                crate::port::ship::berth_of(w, hull),
                crate::port::ship::Berth::Docked { map, .. } if &map.entry == site
            );
            if docked_here && let Some(holder) = crate::port::coop::helm_holder(w, hull) {
                listeners.entry(holder).or_insert(pad);
            }
        }
    }
    let quiet = |who: ObjId| {
        w.module_state::<TacsimStore>()
            .is_some_and(|store| store.tv_quiet(who))
    };
    let lines: Vec<(ObjId, String)> = listeners
        .into_iter()
        .filter(|(who, _)| *who != last.operator && w.is_connected(*who) && !quiet(*who))
        .map(|(who, at)| (who, heard_line(sounder, at)))
        .collect();
    for (who, line) in lines {
        ctx.out.sys_to_actor(who, "tacsim", &line);
    }
}

/// The passive domain of a site's port map.
fn site_domain(w: &World, site: &mush_core::CanonicalId) -> Option<mush_sim::DomainId> {
    w.module_state::<TacsimStore>()?
        .port_maps()
        .find(|map| {
            &map.entry == site && map.pack.as_str() == crate::definitions::content::LANE_PACK
        })
        .and_then(|map| map.domain)
        .map(mush_sim::DomainId)
}

/// **One heard line** (D19): the octant word of the sounder's cell less the listener's, and the
/// band of the Chebyshev cells between them (< 50 `BAND_NEAR`, 50..=100 `BAND_MID`, > 100
/// `BAND_FAR`); `HEARD_HERE` on the sounder's own cell.
#[must_use]
pub(crate) fn heard_line(sounder: [i64; 2], listener: (i64, i64)) -> String {
    use crate::port::tv;
    let (dx, dy) = (
        sounder[0].saturating_sub(listener.0),
        sounder[1].saturating_sub(listener.1),
    );
    let Some(octant) = Octant::of(dx, dy) else {
        return tv::HEARD_HERE.to_owned();
    };
    let cells = cells_apart((sounder[0], sounder[1]), listener);
    let band = if cells < 50 {
        tv::BAND_NEAR
    } else if cells <= 100 {
        tv::BAND_MID
    } else {
        tv::BAND_FAR
    };
    tv::HEARD
        .replace("{bearing}", octant.word())
        .replace("{band}", band)
}

/// **A drive's arrival line** (doc 20 cut SM.4b; §3.7 of the brief): `ARRIVED` naming the pocket
/// address of the end cell, and — only where the operator's OWN chart row has sounded that pocket
/// — its reading in brackets (`chart::pocket_text` without its leading address: `fair metals,
/// lean volatiles`, or `barren`). Never from the ground: no row, a row under another rule, an
/// unsounded pocket or a site whose ground cannot be read now each print the address alone.
fn arrival_line(ctx: &Ctx<'_>, handles: &PortHandles, last: &Outcome, cell: [i64; 2]) -> String {
    use crate::port::tv;
    let pocket = address::pocket_of_cell((cell[0], cell[1]));
    let addr = pocket
        .and_then(|(px, py)| address::pocket_address(px, py))
        .unwrap_or_else(|| format!("{},{}", cell[0], cell[1]));
    let key = ChartKey {
        principal: last.operator,
        site: last.site.clone(),
    };
    let row: Option<ChartRecord> = ctx
        .action_state(&handles.families.charts, &key)
        .ok()
        .flatten()
        .cloned()
        .filter(|row| row.rule == READING_RULE_VERSION && row.key() == key);
    let reading = match (pocket, row.as_ref()) {
        (Some(pocket), Some(row)) if row.pocket_sounded(pocket) => {
            match crate::surface::chart::site_context(ctx.w, &last.site) {
                Ok(site) => {
                    let text = crate::surface::chart::pocket_text(&site, Some(row), pocket);
                    text.strip_prefix(&format!("{addr} "))
                        .map(|words| format!(" ({words})"))
                        .unwrap_or_default()
                }
                Err(_) => String::new(),
            }
        }
        _ => String::new(),
    };
    tv::ARRIVED
        .replace("{n}", &last.slot.to_string())
        .replace("{addr}", &addr)
        .replace("{reading}", &reading)
}

/// **The line of an unload that moved nothing** (doc 20 cut SM.8; the lead's pre-commit round,
/// item 3): `UNLOADED_NONE` naming what stays in the hopper (`stays`, `load_words`' list), or —
/// the hopper read empty at the start's offer (staff emptied it after the recover), so nothing
/// stays either — `UNLOADED_NONE_EMPTY`: never a line whose `{stays}` names nothing.
fn unloaded_none_line(n: &str, name: &str, stays: &str) -> String {
    use crate::port::tv;
    if stays.is_empty() {
        tv::UNLOADED_NONE_EMPTY
            .replace("{n}", n)
            .replace("{name}", name)
    } else {
        tv::UNLOADED_NONE
            .replace("{n}", n)
            .replace("{stays}", stays)
            .replace("{name}", name)
    }
}

/// **The completion line** of a finished act (the brief's 3.6 texts). Doc 20 cut SM.8: a `Done`
/// recover whose record queued its unload tells `RECOVERED_UNLOADING`; a `Done` unload tells
/// `UNLOADED` with the hopper read now (and `UNLOAD_STAYS` when something stayed), or, when it
/// moved nothing, [`unloaded_none_line`]'s.
fn completion_line(
    ctx: &Ctx<'_>,
    record: &RigRecord,
    last: &Outcome,
    hull: Option<ObjId>,
) -> String {
    use crate::port::tv;
    let w: &World = ctx.w;
    let n = last.slot.to_string();
    let ramp = hull.map_or_else(|| "ramp".to_owned(), |hull| ramp_label(w, hull));
    let name = hull.map_or_else(String::new, |hull| crate::port::coop::name_of(w, hull));
    let site = site_name(w, &last.site);
    match last.result {
        ActResult::Done => match last.kind {
            ActKind::Launch => {
                let launched = tv::LAUNCHED
                    .replace("{n}", &n)
                    .replace("{ramp}", &ramp)
                    .replace("{site}", &site);
                // Doc 17h cut M8e E4: a licence that fell the rig to doctrine at its dispatch is
                // said here, in the operator's own completion line and nowhere else.
                match record
                    .sortie
                    .as_ref()
                    .and_then(|open| open.seal.as_ref())
                    .and_then(|facts| crate::surface::dispatch::doctrine_clause(facts, &n))
                {
                    Some(clause) => format!("{launched} {clause}"),
                    None => launched,
                }
            }
            // Doc 20 cut SM.11 (lead ruling 6): the idle return, with its unload clause.
            ActKind::Recover if last.by == Some(crate::surface::plan::ActBy::Idle) => {
                let then = record
                    .queued
                    .as_ref()
                    .is_some_and(|queued| queued.after == last.ordinal);
                tv::IDLE_HOME
                    .replace("{n}", &n)
                    .replace("{site}", &site)
                    .replace("{name}", &name)
                    .replace("{then}", if then { tv::IDLE_THEN } else { "" })
            }
            ActKind::Recover
                if record
                    .queued
                    .as_ref()
                    .is_some_and(|queued| queued.after == last.ordinal) =>
            {
                tv::RECOVERED_UNLOADING
                    .replace("{n}", &n)
                    .replace("{name}", &name)
            }
            ActKind::Recover => tv::RECOVERED.replace("{n}", &n).replace("{name}", &name),
            // Doc 20 cut SM.8: the unload, with the hopper as it reads now.
            ActKind::Unload if last.moved.is_empty() => {
                unloaded_none_line(&n, &name, &crate::surface::act::load_words(w, &last.stays))
            }
            ActKind::Unload => {
                let hopper = match tv::hopper_load_of(ctx, record.tv) {
                    Ok((kg, cap)) => tv::hopper_text(kg, cap),
                    Err(error) => {
                        tracing::warn!(tv = record.tv.0, %error, "tacsim: an unload's tell could not read the hopper");
                        "?".to_owned()
                    }
                };
                let mut line = tv::UNLOADED
                    .replace("{n}", &n)
                    .replace("{load}", &crate::surface::act::load_words(w, &last.moved))
                    .replace("{name}", &name)
                    .replace("{hopper}", &hopper);
                if !last.stays.is_empty() {
                    line.push_str(
                        &tv::UNLOAD_STAYS
                            .replace("{stays}", &crate::surface::act::load_words(w, &last.stays)),
                    );
                }
                line
            }
            ActKind::Dump => tv::DUMPED
                .replace("{n}", &n)
                .replace("{load}", &crate::surface::act::load_words(w, &last.moved))
                .replace("{site}", &site),
            // A finished reading always carries its span (`reading_block`); without one its
            // readings cannot be named.
            ActKind::Scan | ActKind::Sound => tv::READINGS_GONE
                .replace("{n}", &n)
                .replace("{act}", last.kind.word()),
            // Doc 20 cut SM.4b: a finished drive is told by `arrival_line` (it reads the chart);
            // without its cell, the address of nothing is named.
            ActKind::Drive => tv::ARRIVED
                .replace("{n}", &n)
                .replace("{addr}", &site)
                .replace("{reading}", ""),
            // Doc 20 cut SM.7: a cycle's outcome always carries what it did (`mine_line`, which
            // the tell reads first); without it the run can only be said to be over.
            ActKind::Mine => tv::MINE_NOT_RUNNING.replace("{n}", &n),
        },
        ActResult::Refused { code } => {
            let why = match code {
                RefusedCode::NeverLeft => tv::WHY_NEVER_LEFT.to_owned(),
                RefusedCode::NotBeside => tv::WHY_NOT_BESIDE.replace("{ramp}", &ramp),
                RefusedCode::HopperChanged => tv::WHY_HOPPER_CHANGED.to_owned(),
                RefusedCode::SpoilGone => tv::WHY_SPOIL_GONE.to_owned(),
                RefusedCode::Moved => tv::WHY_MOVED.to_owned(),
                RefusedCode::Unworked => tv::WHY_UNWORKED.to_owned(),
                RefusedCode::LeftBay => tv::WHY_LEFT_BAY.to_owned(),
                RefusedCode::HoldRefused => tv::WHY_HOLD_REFUSED.to_owned(),
                // Doc 20 cut SM.11 (the lead's pre-commit round, item 1): a queued unload the
                // finish re-filed (its queue after this outcome) unloads by itself once the ring
                // frees; a hand unload's is "try again later".
                RefusedCode::MarkersFull
                    if record
                        .queued
                        .as_ref()
                        .is_some_and(|queued| queued.after == last.ordinal) =>
                {
                    tv::WHY_MARKERS_QUEUED.to_owned()
                }
                RefusedCode::MarkersFull => tv::WHY_MARKERS_FULL.to_owned(),
            };
            tv::ACT_FAILED
                .replace("{n}", &n)
                .replace("{act}", last.kind.word())
                .replace("{why}", &why)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A heard line waits for the projection** (doc 20 cut SM.4a, D24 — the review's
    /// conformance/F3 moved its witness here, since cut SM.4b's fitted projection leaves no
    /// world padding that refuses the write): a `Done` loud sounding the site heard is sent
    /// from its stand only once the write landed; a refused write, a quiet or unheard sounding,
    /// a refused act, a scan and an act with no span send nothing.
    #[test]
    fn sm4b_a_heard_line_waits_for_the_written_projection() {
        let sound = |quiet: bool, heard: bool| ReadSpan::Sound {
            cell: [170, 90],
            radius: 3,
            quiet,
            heard,
        };
        let outcome = |span: Option<ReadSpan>, result: ActResult| Outcome {
            ordinal: 3,
            kind: ActKind::Sound,
            operator: ObjId(7),
            slot: 1,
            site: mush_core::CanonicalId::new("ahuna_port").expect("an id"),
            at_ic: 1_000,
            result,
            moved: BTreeMap::new(),
            span,
            arrived: None,
            mined: None,
            stays: BTreeMap::new(),
            by: None,
        };
        let loud = outcome(Some(sound(false, true)), ActResult::Done);
        assert_eq!(heard_from(true, &loud), Some([170, 90]));
        assert_eq!(heard_from(false, &loud), None, "a refused write holds it");
        assert_eq!(
            heard_from(true, &outcome(Some(sound(true, false)), ActResult::Done)),
            None
        );
        assert_eq!(
            heard_from(true, &outcome(Some(sound(false, false)), ActResult::Done)),
            None
        );
        assert_eq!(
            heard_from(
                true,
                &outcome(
                    Some(sound(false, true)),
                    ActResult::Refused {
                        code: crate::surface::excursion::RefusedCode::NeverLeft
                    }
                )
            ),
            None
        );
        let scan = ReadSpan::Scan {
            cell: [170, 90],
            footing: crate::surface::reading::Footing::Rock,
            fields: vec![1],
            hidden: Vec::new(),
        };
        assert_eq!(
            heard_from(true, &outcome(Some(scan), ActResult::Done)),
            None
        );
        assert_eq!(heard_from(true, &outcome(None, ActResult::Done)), None);
    }

    /// **The attempt rule** (finding F2): a deferral never counts; a `Submitted` step then a
    /// pending visit does not count; then a visit with nothing pending counts one; the eighth
    /// counted attempt pages once and passes the act over for `DONE_BACKOFF_FIRES − 1` fires,
    /// then submits again; a second exhaustion does not page twice in one boot.
    #[test]
    fn sm3b_the_attempt_rule_counts_only_finishes_that_did_not_land() {
        let mut state = SurfaceTickState::default();
        let key = (900, 1, 0);
        // Sixty deferred visits count nothing.
        for _ in 0..60 {
            state.fires += 1;
            assert_eq!(attempt_visit(&mut state, key, false), Visit::Submit);
            note_step(&mut state, key, Step::Deferred);
        }
        assert_eq!(state.attempts.get(&key), None);
        // Submitted, then a visit with something pending: not counted, still remembered.
        state.fires += 1;
        assert_eq!(attempt_visit(&mut state, key, false), Visit::Submit);
        note_step(&mut state, key, Step::Submitted);
        state.fires += 1;
        assert_eq!(attempt_visit(&mut state, key, true), Visit::Submit);
        assert_eq!(state.attempts.get(&key), None);
        note_step(&mut state, key, Step::Deferred);
        assert!(state.submitted.contains(&key), "a deferral never clears it");
        // Then nothing pending: one counted attempt.
        state.fires += 1;
        assert_eq!(attempt_visit(&mut state, key, false), Visit::Submit);
        assert_eq!(state.attempts.get(&key), Some(&1));
        note_step(&mut state, key, Step::Submitted);
        // Six more failures: seven counted.
        for n in 2..MAX_DONE_ATTEMPTS {
            state.fires += 1;
            assert_eq!(attempt_visit(&mut state, key, false), Visit::Submit);
            assert_eq!(state.attempts.get(&key), Some(&n));
            note_step(&mut state, key, Step::Submitted);
        }
        // The eighth: a page, once.
        state.fires += 1;
        let began = state.fires;
        assert_eq!(attempt_visit(&mut state, key, false), Visit::Page);
        for _ in 1..DONE_BACKOFF_FIRES {
            state.fires += 1;
            assert_eq!(attempt_visit(&mut state, key, false), Visit::PassOver);
        }
        state.fires += 1;
        assert_eq!(state.fires - began, DONE_BACKOFF_FIRES);
        assert_eq!(
            attempt_visit(&mut state, key, false),
            Visit::Submit,
            "the retry"
        );
        note_step(&mut state, key, Step::Submitted);
        // A further failure restarts the back-off, without a second page this boot.
        state.fires += 1;
        assert_eq!(attempt_visit(&mut state, key, false), Visit::PassOver);
        assert_eq!(state.backoff.get(&key), Some(&state.fires));
        assert_eq!(state.paged.len(), 1);
    }

    /// **A retried finish is keyed by the act, not the rig** (review finding F1): a stale
    /// counted attempt of the rig's earlier act `(tv, 1)` — one a capped scan never pruned —
    /// leaves a fresh dump at ordinal 2 not-failed; an attempt at `(tv, 2)` marks it failed;
    /// another rig's attempts and a paged-only ordinal-0 key never count; and the set holds at
    /// most `MAX_RETRIED_ORDINALS`, the highest (the live act's among them).
    #[test]
    fn sm3b_a_retried_finish_is_keyed_by_the_act_not_the_rig() {
        let tv = ObjId(900);
        let mut state = SurfaceTickState::default();
        state.attempts.insert((tv.0, 1, 0), 2);
        state.attempts.insert((901, 2, 0), 1);
        state.paged.insert((tv.0, 0, 0));
        let failed = retried_ordinals(&state, tv);
        assert_eq!(failed, vec![(1, 0)]);
        assert!(
            !failed.contains(&(2, 0)),
            "a stale attempt of act 1 never marks act 2"
        );
        state.attempts.insert((tv.0, 2, 0), 1);
        let failed = retried_ordinals(&state, tv);
        assert!(
            failed.contains(&(2, 0)),
            "an attempt at (tv, 2) marks act 2"
        );
        assert_eq!(failed, vec![(1, 0), (2, 0)]);
        assert!(retried_ordinals(&state, ObjId(902)).is_empty());
        // Bounded: the highest ordinals are kept.
        let cap = u32::try_from(MAX_RETRIED_ORDINALS).expect("fits");
        let over = cap + 5;
        for ordinal in 1..=over {
            state.attempts.insert((tv.0, ordinal, 0), 1);
        }
        let failed = retried_ordinals(&state, tv);
        assert_eq!(failed.len(), MAX_RETRIED_ORDINALS);
        assert_eq!(failed.last(), Some(&(over, 0)));
        assert_eq!(failed.first(), Some(&(over + 1 - cap, 0)));
    }

    /// **The prune keeps every act still due, not only the visited ones** (review finding
    /// conformance/F2): more due rigs than one fire visits all count as still due, so an act
    /// in its back-off past the visit cut keeps its attempts, back-off and page; an act not
    /// yet due, a rig with no act, and a finished act do not.
    #[test]
    fn sm3b_still_due_reads_the_whole_scan_past_the_visit_cut() {
        use crate::surface::excursion::{Act, ActParams, Charge};
        let now = 10_000;
        let site = mush_core::CanonicalId::new("ahuna_port").expect("an id");
        let record = |tv: u32, act: Option<i64>| RigRecord {
            schema: crate::surface::excursion::EXCURSION_SCHEMA,
            queued: None,
            tv: ObjId(tv),
            ordinal: 3,
            charge: Charge {
                value_cu: 100_000,
                at_ic: 0,
                in_bay: true,
            },
            act: act.map(|due_ic| Act {
                ordinal: 3,
                kind: ActKind::Launch,
                operator: ObjId(7),
                hull: ObjId(8),
                slot: 1,
                site: site.clone(),
                started_ic: due_ic - 300,
                due_ic,
                reserved_cu: 0,
                params: ActParams::None {},
                by: None,
                seal: None,
            }),
            excursion: None,
            last: None,
            run: None,
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
        };
        let over = u32::try_from(MAX_EXCURSIONS_VISITED_PER_FIRE).expect("fits") + 6;
        let mut records: Vec<RigRecord> = (1..=over)
            .map(|tv| record(tv, Some(now - i64::from(tv))))
            .collect();
        records.push(record(5_000, Some(now + 1)));
        records.push(record(5_001, None));
        let due = still_due(&records, now);
        assert_eq!(due.len(), usize::try_from(over).expect("fits"));
        // Oldest due first: rig `over` is visited first and rig 1 (the newest due) falls
        // past the cut, still due.
        assert!(due.contains(&(over, 3, 0)));
        assert!(due.contains(&(1, 3, 0)), "past the visit cut, still due");
        assert!(!due.contains(&(5_000, 3, 0)), "not yet due");
        assert!(!due.iter().any(|key| key.0 == 5_001), "no act");
    }

    /// **Acts in their back-off never fill the visit window** (the SM.3b fixer round's item 5):
    /// 64 acts all due long ago and all inside their back-off, and one fresh act due now. The
    /// fire's candidates are the fresh act alone (the 64 counted as passed over), so it is
    /// visited this fire; once the back-off has run its `DONE_BACKOFF_FIRES`, the 64 are
    /// candidates again, oldest first, ahead of a newer act, and the window holds 64 of the 65.
    /// An act in its back-off with an outcome still untold stays a candidate, for the tell.
    #[test]
    fn sm3b_acts_in_their_back_off_never_starve_the_window() {
        use crate::surface::excursion::{Act, ActParams, Charge};
        let now = 100_000;
        let site = mush_core::CanonicalId::new("ahuna_port").expect("an id");
        let record = |tv: u32, due_ic: i64| RigRecord {
            schema: crate::surface::excursion::EXCURSION_SCHEMA,
            queued: None,
            tv: ObjId(tv),
            ordinal: 2,
            charge: Charge {
                value_cu: 100_000,
                at_ic: 0,
                in_bay: true,
            },
            act: Some(Act {
                ordinal: 2,
                kind: ActKind::Launch,
                operator: ObjId(7),
                hull: ObjId(8),
                slot: 1,
                site: site.clone(),
                started_ic: due_ic - 300,
                due_ic,
                reserved_cu: 0,
                params: ActParams::None {},
                by: None,
                seal: None,
            }),
            excursion: None,
            last: None,
            run: None,
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
        };
        let stuck = u32::try_from(MAX_EXCURSIONS_VISITED_PER_FIRE).expect("fits");
        let mut records: Vec<RigRecord> = (1..=stuck)
            .map(|tv| record(tv, 1_000 + i64::from(tv)))
            .collect();
        let fresh = 9_000;
        records.push(record(fresh, now));
        let mut state = SurfaceTickState {
            fires: 500,
            ..SurfaceTickState::default()
        };
        for tv in 1..=stuck {
            state.backoff.insert((tv, 2, 0), 500);
        }
        let told = |_: ObjId| (0, 0);
        // In the back-off: the fresh act is the one candidate.
        let (picked, resting) = candidates(
            records.clone(),
            now,
            told,
            |_| None,
            |_| None,
            &state,
            &PlanReads::none(),
        );
        assert_eq!(
            picked
                .iter()
                .map(|candidate| candidate.record.tv.0)
                .collect::<Vec<_>>(),
            vec![fresh]
        );
        assert!(picked[0].due);
        assert_eq!(resting, stuck, "every backed-off act is passed over");
        // One fire before the back-off ends: still the fresh act alone.
        state.fires = 500 + DONE_BACKOFF_FIRES - 1;
        let (picked, _) = candidates(
            records.clone(),
            now,
            told,
            |_| None,
            |_| None,
            &state,
            &PlanReads::none(),
        );
        assert_eq!(
            picked.len(),
            1,
            "still in the back-off one fire before its end"
        );
        // The back-off over: the 64 are candidates again, oldest first, and fill the window.
        state.fires = 500 + DONE_BACKOFF_FIRES;
        let (picked, resting) = candidates(
            records.clone(),
            now,
            told,
            |_| None,
            |_| None,
            &state,
            &PlanReads::none(),
        );
        assert_eq!(resting, 0);
        assert_eq!(picked.len(), MAX_EXCURSIONS_VISITED_PER_FIRE);
        assert_eq!(picked[0].record.tv, ObjId(1), "the oldest due first");
        assert!(picked.iter().all(|candidate| candidate.due));
        assert!(
            !picked
                .iter()
                .any(|candidate| candidate.record.tv.0 == fresh),
            "the newest waits its turn behind the old acts once they may retry"
        );
        // An act in its back-off with an untold outcome stays, for the tell alone.
        state.fires = 500;
        let mut untold = record(1, 1_001);
        untold.last = Some(Outcome {
            ordinal: 1,
            kind: ActKind::Launch,
            operator: ObjId(7),
            slot: 1,
            site: site.clone(),
            at_ic: 900,
            result: ActResult::Done,
            moved: BTreeMap::new(),
            span: None,
            arrived: None,
            mined: None,
            stays: BTreeMap::new(),
            by: None,
        });
        let (picked, resting) = candidates(
            vec![untold],
            now,
            told,
            |_| None,
            |_| None,
            &state,
            &PlanReads::none(),
        );
        assert_eq!(resting, 1);
        assert_eq!(picked.len(), 1);
        assert!(picked[0].untold && !picked[0].due, "told, not submitted");
    }

    /// **A started drive is a candidate once** (doc 20 cut SM.4b; decision P8): an act in
    /// flight, not yet due, whose params carry a leg is a candidate while the projection's act
    /// is not that ordinal, and never once it is; an act without a leg is not; a due drive is a
    /// due candidate, not a started one; ordered by `(started_ic, tv)` among the others.
    #[test]
    fn sm4b_a_started_drive_is_a_candidate_once() {
        use crate::surface::excursion::{Act, ActParams, Charge, LegWaypoint};
        let now = 10_000;
        let site = mush_core::CanonicalId::new("ahuna_port").expect("an id");
        let drive = ActParams::Drive {
            from_bay: false,
            target: "C117".to_owned(),
            to: [146, 226],
            waypoints: vec![
                LegWaypoint {
                    x_cm: 128_500,
                    y_cm: 124_500,
                    at_s: 0,
                },
                LegWaypoint {
                    x_cm: 146_500,
                    y_cm: 226_500,
                    at_s: 600,
                },
            ],
            secs: 600,
            cost: 18_000,
            metres: 1_020,
            heading_mdeg: 0,
        };
        let record = |tv: u32, started_ic: i64, due_ic: i64, params: ActParams| RigRecord {
            schema: crate::surface::excursion::EXCURSION_SCHEMA,
            queued: None,
            tv: ObjId(tv),
            ordinal: 4,
            charge: Charge {
                value_cu: 90_000,
                at_ic: 0,
                in_bay: false,
            },
            act: Some(Act {
                ordinal: 4,
                kind: if matches!(params, ActParams::Drive { .. }) {
                    ActKind::Drive
                } else {
                    ActKind::Scan
                },
                operator: ObjId(7),
                hull: ObjId(8),
                slot: 1,
                site: site.clone(),
                started_ic,
                due_ic,
                reserved_cu: 2_700,
                params,
                by: None,
                seal: None,
            }),
            excursion: None,
            last: None,
            run: None,
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
        };
        let state = SurfaceTickState::default();
        let records = vec![
            record(20, now - 50, now + 550, drive.clone()),
            record(21, now - 90, now + 510, drive.clone()),
            record(22, now - 10, now + 590, ActParams::None {}),
            record(23, now - 600, now, drive.clone()),
        ];
        let (picked, _) = candidates(
            records.clone(),
            now,
            |_| (0, 0),
            |_| None,
            |_| None,
            &state,
            &PlanReads::none(),
        );
        let order: Vec<(u32, bool, bool)> = picked
            .iter()
            .map(|c| (c.record.tv.0, c.started, c.due))
            .collect();
        assert_eq!(
            order,
            vec![(21, true, false), (20, true, false), (23, false, true)],
            "two started drives by their start, the due drive at its due instant, no leg no visit"
        );
        // Once the projection shows the act, a started drive is no longer a candidate.
        let (picked, _) = candidates(
            records,
            now,
            |_| (0, 0),
            |tv| (tv.0 == 20).then_some(4),
            |_| None,
            &state,
            &PlanReads::none(),
        );
        assert!(!picked.iter().any(|c| c.record.tv.0 == 20));
        assert!(picked.iter().any(|c| c.record.tv.0 == 21 && c.started));
    }

    /// A record with a run (ordinal 3, cycle `k`, due `run_due`) and, when `act_due` is given, a
    /// scan at ordinal 4 due then.
    fn drilling(tv: u32, run_due: i64, k: u8, act_due: Option<i64>) -> RigRecord {
        use crate::surface::excursion::{Act, ActParams, Charge, LockRow, Run};
        let site = mush_core::CanonicalId::new("ahuna_port").expect("an id");
        let stand = [146_i64, 226];
        let (px, py) = address::pocket_of_cell((stand[0], stand[1])).expect("a pocket");
        let number = address::pocket_number(px, py).expect("a number");
        RigRecord {
            schema: crate::surface::excursion::EXCURSION_SCHEMA,
            queued: None,
            tv: ObjId(tv),
            ordinal: if act_due.is_some() { 4 } else { 3 },
            charge: Charge {
                value_cu: 90_000,
                at_ic: 0,
                in_bay: false,
            },
            act: act_due.map(|due_ic| Act {
                ordinal: 4,
                kind: ActKind::Scan,
                operator: ObjId(7),
                hull: ObjId(8),
                slot: 1,
                site: site.clone(),
                started_ic: due_ic - 600,
                due_ic,
                reserved_cu: 1_000,
                params: ActParams::Scan {
                    from_bay: false,
                    body: mush_core::CanonicalId::new("ceres").expect("an id"),
                    cell: stand,
                    footing: Footing::Regolith,
                    fields: vec![27],
                    hidden: Vec::new(),
                },
                by: None,
                seal: None,
            }),
            excursion: None,
            last: None,
            run: Some(Run {
                ordinal: 3,
                operator: ObjId(7),
                hull: ObjId(8),
                slot: 1,
                site: site.clone(),
                body: mush_core::CanonicalId::new("ceres").expect("an id"),
                stand,
                pocket: [px, py],
                number,
                letter: address::block_letter(px, py).expect("a letter"),
                family: crate::surface::geology::OreFamily::Metals,
                deposit: mush_industry::OreDepositId(77),
                good: mush_core::CanonicalId::new("metal_ore").expect("an id"),
                source: Some(12),
                hopper: 13,
                cycles: 4,
                k,
                secs: 1_800,
                cost_cu: 2_000,
                reserve_t: vec![2, 2, 2, 1],
                class_t: 2,
                pocket_t: 7,
                blind: false,
                stop: false,
                lapses_ic: None,
                lock_rows: vec![LockRow {
                    deposit: mush_industry::OreDepositId(77),
                    pocket: number,
                }],
                due_ic: run_due,
                reserved_cu: 2_000 * u32::from(5 - k),
                by: None,
            }),
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
        }
    }

    /// **A run and an act take turns** (doc 20 cut SM.7; the brief's 3.8): the earlier due item
    /// first — the act when it falls due before the cycle, the cycle otherwise — a tie to the run
    /// (its cycle began first); each key's back-off independent (the act resting leaves the cycle
    /// due, and the reverse); `still_due` holds both keys.
    #[test]
    fn sm7_a_run_and_an_act_take_turns() {
        let now = 10_000;
        let state = SurfaceTickState::default();
        // The projection already shows each record's run (the started reason is the next test's).
        let pick = |record: RigRecord, state: &SurfaceTickState| {
            let shown = record.run.as_ref().map(|run| (run.ordinal, run.k));
            let (picked, _) = candidates(
                vec![record],
                now,
                |_| (0, 0),
                |_| None,
                |_| shown,
                state,
                &PlanReads::none(),
            );
            picked.first().map(|c| (c.due, c.cycle))
        };
        assert_eq!(
            pick(drilling(1, 9_000, 2, Some(8_000)), &state),
            Some((true, None))
        );
        assert_eq!(
            pick(drilling(1, 8_000, 2, Some(9_000)), &state),
            Some((true, Some(2)))
        );
        assert_eq!(
            pick(drilling(1, 9_000, 2, Some(9_000)), &state),
            Some((true, Some(2))),
            "a tie to the run"
        );
        assert_eq!(
            pick(drilling(1, 9_000, 3, None), &state),
            Some((true, Some(3)))
        );
        assert_eq!(
            pick(drilling(1, 11_000, 3, None), &state),
            None,
            "not yet due"
        );
        // The act resting: the cycle is still due; the cycle resting: the act still is.
        let mut resting = SurfaceTickState {
            fires: 100,
            ..SurfaceTickState::default()
        };
        resting.backoff.insert((1, 4, 0), 100);
        assert_eq!(
            pick(drilling(1, 9_000, 2, Some(8_000)), &resting),
            Some((true, Some(2)))
        );
        let mut resting = SurfaceTickState {
            fires: 100,
            ..SurfaceTickState::default()
        };
        resting.backoff.insert((1, 3, 2), 100);
        assert_eq!(
            pick(drilling(1, 8_000, 2, Some(9_000)), &resting),
            Some((true, None))
        );
        let due = still_due(&[drilling(1, 9_000, 2, Some(8_000))], now);
        assert!(due.contains(&(1, 4, 0)) && due.contains(&(1, 3, 2)));
    }

    /// **A started run is a candidate once** (doc 20 cut SM.7; the SM.7 lead's pre-commit round,
    /// item 4): a run whose cycle is not yet due is a started candidate (not a due one) while the
    /// projection shows no run, or another run's or another cycle's — ordered by its cycle's start
    /// (`due_ic − secs`) — and never once it shows this run's ordinal and cycle; a due cycle is a
    /// due candidate, not a started one.
    #[test]
    fn sm7_a_started_run_is_a_candidate_once() {
        let now = 10_000;
        let state = SurfaceTickState::default();
        let pick = |record: RigRecord, shown: Option<(u32, u8)>| {
            let (picked, _) = candidates(
                vec![record],
                now,
                |_| (0, 0),
                |_| None,
                |_| shown,
                &state,
                &PlanReads::none(),
            );
            picked.first().map(|c| (c.started, c.due, c.order))
        };
        assert_eq!(
            pick(drilling(1, 11_000, 1, None), None),
            Some((true, false, (11_000 - 1_800, 1))),
            "the start landed; the projection shows no run yet"
        );
        assert_eq!(
            pick(drilling(1, 11_000, 2, None), Some((3, 1))),
            Some((true, false, (11_000 - 1_800, 1))),
            "the projection shows the cycle before"
        );
        assert_eq!(
            pick(drilling(1, 11_000, 2, None), Some((2, 2))),
            Some((true, false, (11_000 - 1_800, 1))),
            "another run's"
        );
        assert_eq!(
            pick(drilling(1, 11_000, 2, None), Some((3, 2))),
            None,
            "shown: never again"
        );
        assert_eq!(
            pick(drilling(1, 9_000, 2, None), None).map(|(started, due, _)| (started, due)),
            Some((false, true)),
            "a due cycle is due, not started"
        );
    }

    /// **The told rule is equality** (doc 20 cut SM.7; decision R3): a cycle of the run at ordinal
    /// 3 finishing after a told scan at ordinal 4 is untold (never `>`); a told pair is told; the
    /// next cycle of the same run is untold.
    #[test]
    fn sm7_the_told_rule_is_equality() {
        use crate::surface::excursion::Mined;
        let now = 10_000;
        let state = SurfaceTickState::default();
        let mut record = drilling(1, 20_000, 3, None);
        record.ordinal = 4;
        record.last = Some(Outcome {
            ordinal: 3,
            kind: ActKind::Mine,
            operator: ObjId(7),
            slot: 1,
            site: mush_core::CanonicalId::new("ahuna_port").expect("an id"),
            at_ic: 9_000,
            result: ActResult::Done,
            moved: BTreeMap::new(),
            span: None,
            arrived: None,
            stays: BTreeMap::new(),
            mined: Some(Mined {
                k: 2,
                cycles: 4,
                pocket: [36, 56],
                good: mush_core::CanonicalId::new("metal_ore").expect("an id"),
                tonnes: 2,
                blind: false,
                end: None,
            }),
            by: None,
        });
        let untold = |told: (u32, u8)| {
            let (picked, _) = candidates(
                vec![record.clone()],
                now,
                |_| told,
                |_| None,
                |_| None,
                &state,
                &PlanReads::none(),
            );
            picked.first().is_some_and(|c| c.untold)
        };
        assert!(
            untold((4, 0)),
            "a later scan told; the cycle of ordinal 3 is not"
        );
        assert!(untold((3, 1)), "the cycle before");
        assert!(!untold((3, 2)), "told");
        assert_eq!(told_pair(record.last.as_ref().expect("an outcome")), (3, 2));
    }

    /// **A cycle's key forms** (doc 20 cut SM.7): `tacsim.surface/{tv}/{ordinal}/cycle/{k}`.
    #[test]
    fn sm7_a_cycle_key_forms() {
        assert_eq!(
            cycle_key(ObjId(900), 3, 4).map(|key| key.as_str().to_owned()),
            Some("tacsim.surface/900/3/cycle/4".to_owned())
        );
    }

    /// **The heard line names a bearing and a band and nothing more** (doc 20 cut SM.4a, D19):
    /// the octant of (sounder − listener) by the 2:5 rule, the band by Chebyshev cells (< 50
    /// under 500 m, 50..=100 500 m to 1 km, > 100 over 1 km), and right under you on the same
    /// cell.
    #[test]
    fn sm4a_the_heard_band_and_bearing_come_from_positions() {
        use crate::port::tv::{BAND_FAR, BAND_MID, BAND_NEAR, HEARD, HEARD_HERE};
        let line =
            |bearing: &str, band: &str| HEARD.replace("{bearing}", bearing).replace("{band}", band);
        assert_eq!(heard_line([128, 128], (128, 128)), HEARD_HERE);
        assert_eq!(heard_line([128, 177], (128, 128)), line("north", BAND_NEAR));
        assert_eq!(heard_line([128, 178], (128, 128)), line("north", BAND_MID));
        assert_eq!(heard_line([28, 128], (128, 128)), line("west", BAND_MID));
        assert_eq!(heard_line([27, 128], (128, 128)), line("west", BAND_FAR));
        assert_eq!(
            heard_line([200, 60], (128, 128)),
            line("southeast", BAND_MID)
        );
        assert_eq!(
            heard_line([10, 250], (128, 128)),
            line("northwest", BAND_FAR)
        );
        assert_eq!(
            heard_line([130, 125], (128, 128)),
            line("southeast", BAND_NEAR)
        );
        assert_eq!(heard_line([135, 126], (128, 128)), line("east", BAND_NEAR));
        let text = heard_line([200, 60], (128, 128));
        assert_eq!(
            text,
            "A sounding thumps through the ground from the southeast, 500 m to 1 km off."
        );
    }

    /// An idle record at ordinal 5 holding a queued unload filed by that recover at `at_ic`.
    fn queued(tv: u32, at_ic: i64) -> RigRecord {
        let mut record = RigRecord::fresh_unread(ObjId(tv));
        record.ordinal = 5;
        record.queued = Some(crate::surface::excursion::Queued {
            kind: ActKind::Unload,
            operator: ObjId(7),
            hull: ObjId(8),
            slot: 1,
            site: mush_core::CanonicalId::new("ahuna_port").expect("an id"),
            after: 5,
            at_ic,
            excursion: None,
        });
        record
    }

    /// **A queued unload is a candidate until a start lands** (doc 20 cut SM.8; the brief's 3.9):
    /// an idle record with a queue is a queued candidate (not due, not started), ordered by its
    /// `at_ic` among the others; with an act or a run beside it, not; inside its start's back-off,
    /// not (and counted passed over); `still_due` holds its attempts key.
    #[test]
    fn sm8_a_queued_unload_is_a_candidate_until_a_start_lands() {
        let now = 10_000;
        let state = SurfaceTickState::default();
        let pick = |records: Vec<RigRecord>, state: &SurfaceTickState| {
            let (picked, resting) = candidates(
                records,
                now,
                |_| (0, 0),
                |_| None,
                |_| None,
                state,
                &PlanReads::none(),
            );
            (
                picked
                    .iter()
                    .map(|c| (c.record.tv.0, c.queued, c.due, c.started, c.order))
                    .collect::<Vec<_>>(),
                resting,
            )
        };
        assert_eq!(
            pick(vec![queued(1, 9_000)], &state),
            (vec![(1, true, false, false, (9_000, 1))], 0)
        );
        assert_eq!(
            pick(vec![queued(1, 9_500), queued(2, 9_000)], &state).0,
            vec![
                (2, true, false, false, (9_000, 2)),
                (1, true, false, false, (9_500, 1))
            ],
            "ordered by the queue's instant"
        );
        let mut busy = queued(1, 9_000);
        busy.act = drilling(1, 9_000, 2, Some(12_000)).act;
        busy.ordinal = 4;
        assert!(pick(vec![busy], &state).0.is_empty(), "an act beside it");
        let mut drilling_too = drilling(1, 12_000, 2, None);
        drilling_too.queued = queued(1, 9_000).queued;
        if let Some(q) = drilling_too.queued.as_mut() {
            q.after = 3;
        }
        assert!(
            pick(vec![drilling_too], &state).0.iter().all(|c| !c.1),
            "never queued beside a run"
        );
        let mut resting = SurfaceTickState {
            fires: 100,
            ..SurfaceTickState::default()
        };
        resting.backoff.insert((1, 6, QUEUED_CYCLE), 100);
        assert_eq!(pick(vec![queued(1, 9_000)], &resting), (Vec::new(), 1));
        let due = still_due(&[queued(1, 9_000)], now);
        assert!(due.contains(&(1, 6, QUEUED_CYCLE)), "{due:?}");
    }

    /// **The queued key never meets a cycle** (doc 20 cut SM.8; the brief's reviewer finding F9):
    /// `QUEUED_CYCLE` lies past every run cycle; a slot holding a queued start's attempts and an
    /// act's finish attempts hands the finish's offer the act's ordinal only.
    #[test]
    fn sm8_the_queued_key_never_meets_a_cycle() {
        const { assert!(QUEUED_CYCLE > crate::surface::excursion::MAX_RUN_CYCLES) };
        let mut state = SurfaceTickState::default();
        state.attempts.insert((900, 6, QUEUED_CYCLE), 3);
        state.attempts.insert((900, 4, 0), 2);
        state.attempts.insert((900, 3, 2), 1);
        assert_eq!(retried_ordinals(&state, ObjId(900)), vec![(3, 2), (4, 0)]);
    }
    /// A record at ordinal 5 holding plan 1 by the lessee 7, live, filed at `filed_ic`.
    fn planned(tv: u32, filed_ic: i64) -> RigRecord {
        use crate::surface::plan::{FillPlan, FillState, PLAN_BOUND_IC_SECS, PlanStep, PlanTally};
        let mut record = RigRecord::fresh_unread(ObjId(tv));
        record.ordinal = 5;
        record.plans = 1;
        record.plan = Some(FillPlan {
            no: 1,
            author: ObjId(7),
            hull: ObjId(8),
            slot: 1,
            site: mush_core::CanonicalId::new("ahuna_port").expect("an id"),
            family: crate::surface::geology::OreFamily::Metals,
            filed_ic,
            deadline_ic: filed_ic + PLAN_BOUND_IC_SECS,
            list: vec!["C117".to_owned()],
            steps: vec![PlanStep::Block { letter: 'C' }],
            cursor: 0,
            passed: Vec::new(),
            state: FillState::Live,
            seq: 1,
            tally: PlanTally::default(),
            notes: Vec::new(),
        });
        record
    }

    /// An idle record at ordinal 5 with an open excursion launched at `launch_ic` and no plan.
    fn out_idle(tv: u32, launch_ic: i64) -> RigRecord {
        let mut record = RigRecord::fresh_unread(ObjId(tv));
        record.ordinal = 5;
        record.excursion = Some(crate::surface::excursion::Excursion {
            id: ExcursionId {
                tv: ObjId(tv),
                launch_ic,
            },
            operator: ObjId(7),
            hull: ObjId(8),
            slot: 1,
            site: mush_core::CanonicalId::new("ahuna_port").expect("an id"),
        });
        record
    }

    /// The lessee 7 on every rig.
    fn lessee_7(_: ObjId) -> Option<ObjId> {
        Some(ObjId(7))
    }

    /// The lessee 9 on every rig (plan 1's author, 7, no longer leases it).
    fn lessee_9(_: ObjId) -> Option<ObjId> {
        Some(ObjId(9))
    }

    /// The pilot 7 holds the helm of hull 8 (plan 1's hull).
    fn helm_8(_: ObjId) -> Option<ObjId> {
        Some(ObjId(8))
    }

    /// The projection a lessee other than plan 1's author holds: the told pair carried, no view
    /// (the SM.11 fixer round's conformance/F1).
    fn shown_told_only(_: ObjId) -> (Option<PlanTold>, Option<PlanView>) {
        (Some(PlanTold { no: 1, seq: 1 }), None)
    }

    /// The projection shows plan 1 at `seq` 1 with its view (nothing untold).
    fn shown_current(_: ObjId) -> (Option<PlanTold>, Option<PlanView>) {
        let record = planned(1, 1_000);
        (
            Some(PlanTold { no: 1, seq: 1 }),
            record.plan.as_ref().map(crate::surface::plan::plan_view),
        )
    }

    /// **The plan and idle candidates** (doc 20 cut SM.11; the brief's 3.8): pure over
    /// `candidates` — a live plan with nothing in flight is a plan candidate; with an act, a run or
    /// a queued unload beside it, not; an orphaned plan (its author no longer the lessee), not;
    /// inside its back-off, not; inside a cached wait of the same `seq`, not, and of an older
    /// `seq`, a candidate; an idle rig at 24 IC h less a minute, not, at 24 IC h, a candidate;
    /// a stranded rig inside its day, not; a plan untold, a candidate; the order.
    #[test]
    fn sm11_plan_and_idle_candidates() {
        use crate::surface::plan::IDLE_PARK_IC_SECS;
        let now = 100_000;
        let current = PlanReads {
            lessee: &lessee_7,
            shown: &shown_current,
            helm: &helm_8,
        };
        let orphaned = PlanReads {
            lessee: &lessee_9,
            shown: &shown_told_only,
            helm: &helm_8,
        };
        let pick = |records: Vec<RigRecord>, state: &SurfaceTickState, reads: &PlanReads<'_>| {
            let (picked, _) =
                candidates(records, now, |_| (0, 0), |_| None, |_| None, state, reads);
            picked
                .iter()
                .map(|c| (c.record.tv.0, c.plan, c.plan_untold, c.idle, c.order))
                .collect::<Vec<_>>()
        };
        let state = SurfaceTickState::default();
        assert_eq!(
            pick(vec![planned(1, 1_000)], &state, &current),
            vec![(1, true, false, false, (1_000, 1))],
            "a live plan with nothing in flight"
        );
        assert_eq!(
            pick(vec![planned(1, 2_000), planned(2, 1_000)], &state, &current)
                .iter()
                .map(|c| c.0)
                .collect::<Vec<_>>(),
            vec![2, 1],
            "ordered by the filing (no outcome yet), then the rig"
        );
        let mut busy = planned(1, 1_000);
        busy.act = drilling(1, now + 500, 2, Some(now + 500)).act;
        assert!(
            pick(vec![busy], &state, &current).iter().all(|c| !c.1),
            "an act beside it"
        );
        let mut running = drilling(1, now + 500, 2, None);
        running.plan = planned(1, 1_000).plan;
        running.plans = 1;
        assert!(
            pick(vec![running], &state, &current).iter().all(|c| !c.1),
            "a run beside it"
        );
        let mut waiting_unload = planned(1, 1_000);
        waiting_unload.queued = queued(1, 9_000).queued;
        assert!(
            pick(vec![waiting_unload], &state, &current)
                .iter()
                .all(|c| !c.1),
            "a queued unload beside it"
        );
        // The lead's pre-commit round, item 3: past its bound a plan is decided whatever it waits
        // on — a queued unload standing, or a cached wait — so its `Bound` end is submitted; an
        // act in flight still holds it.
        let past = now - crate::surface::plan::PLAN_BOUND_IC_SECS;
        let mut bound_unload = planned(1, past);
        bound_unload.queued = queued(1, 9_000).queued;
        assert!(
            pick(vec![bound_unload], &state, &current)
                .iter()
                .any(|c| c.1),
            "past its bound, beside a queued unload"
        );
        let mut cached = SurfaceTickState::default();
        cached.plan_waits.insert(ObjId(1), (1, now + 300));
        assert!(
            pick(vec![planned(1, past)], &cached, &current)
                .iter()
                .any(|c| c.1),
            "past its bound, inside a cached wait"
        );
        assert!(
            pick(vec![planned(1, past + 1)], &cached, &current)
                .iter()
                .all(|c| !c.1),
            "a second before its bound, the cached wait holds"
        );
        let mut bound_busy = planned(1, past);
        bound_busy.act = drilling(1, now + 500, 2, Some(now + 500)).act;
        assert!(
            pick(vec![bound_busy], &state, &current)
                .iter()
                .all(|c| !c.1),
            "past its bound, an act in flight holds it"
        );
        assert!(
            pick(vec![planned(1, 1_000)], &state, &orphaned).is_empty(),
            "an orphaned plan is inert"
        );
        // The fixer round's conformance/F1: a projection still showing the previous lessee's
        // plan is untold once (its view cleared for the new lessee), never a plan candidate.
        let stale = PlanReads {
            lessee: &lessee_9,
            shown: &shown_current,
            helm: &helm_8,
        };
        assert_eq!(
            pick(vec![planned(1, 1_000)], &state, &stale),
            vec![(1, false, true, false, (1_000, 1))],
            "the old view is cleared, the plan stays inert"
        );
        // The fixer round's correctness/F3: the author's own queued unload with the author off
        // the helm is orphaned — the plan's to read (its helm wait), not SM.8's start.
        let off_helm = PlanReads {
            lessee: &lessee_7,
            shown: &shown_current,
            helm: &no_helm,
        };
        let mut orphaned_unload = planned(1, 1_000);
        orphaned_unload.queued = queued(1, 9_000).queued;
        let (picked, _) = candidates(
            vec![orphaned_unload],
            now,
            |_| (0, 0),
            |_| None,
            |_| None,
            &state,
            &off_helm,
        );
        assert_eq!(
            picked
                .iter()
                .map(|c| (c.plan, c.queued))
                .collect::<Vec<_>>(),
            vec![(true, false)],
            "an orphaned queue goes to the plan's arm"
        );
        let mut resting = SurfaceTickState {
            fires: 100,
            ..SurfaceTickState::default()
        };
        resting.backoff.insert((1, 6, PLAN_CYCLE), 100);
        assert!(
            pick(vec![planned(1, 1_000)], &resting, &current).is_empty(),
            "in its back-off"
        );
        let mut cached = SurfaceTickState::default();
        cached.plan_waits.insert(ObjId(1), (1, now + 300));
        assert!(
            pick(vec![planned(1, 1_000)], &cached, &current).is_empty(),
            "inside a wait of the same seq"
        );
        cached.plan_waits.insert(ObjId(1), (0, now + 300));
        assert_eq!(
            pick(vec![planned(1, 1_000)], &cached, &current).len(),
            1,
            "a wait of an older seq is stale"
        );
        cached.plan_waits.insert(ObjId(1), (1, now));
        assert_eq!(
            pick(vec![planned(1, 1_000)], &cached, &current).len(),
            1,
            "a wait that has run out"
        );
        // The idle return (a rig with no plan shows none).
        let idle = PlanReads {
            lessee: &lessee_7,
            shown: &nothing_shown,
            helm: &helm_8,
        };
        let launched = now - IDLE_PARK_IC_SECS;
        assert!(
            pick(vec![out_idle(1, launched + 60)], &state, &idle).is_empty(),
            "24 IC h less a minute"
        );
        assert_eq!(
            pick(vec![out_idle(1, launched)], &state, &idle),
            vec![(1, false, false, true, (now, 1))],
            "24 IC h"
        );
        let mut stranded = SurfaceTickState::default();
        stranded.stranded.insert(ObjId(1), now + 1);
        assert!(
            pick(vec![out_idle(1, launched)], &stranded, &idle).is_empty(),
            "a stranded rig inside its day"
        );
        stranded.stranded.insert(ObjId(1), now);
        assert_eq!(
            pick(vec![out_idle(1, launched)], &stranded, &idle).len(),
            1,
            "a stranded rig's day has run"
        );
        let mut idle_resting = SurfaceTickState {
            fires: 100,
            ..SurfaceTickState::default()
        };
        idle_resting.backoff.insert((1, 6, IDLE_CYCLE), 100);
        assert!(pick(vec![out_idle(1, launched)], &idle_resting, &idle).is_empty());
        let mut planning_out = out_idle(1, launched);
        planning_out.plan = planned(1, 1_000).plan;
        planning_out.plans = 1;
        assert!(
            pick(vec![planning_out], &state, &current)
                .iter()
                .all(|c| !c.3),
            "a live plan is never idle"
        );
        // The plan untold.
        let mut untold = planned(1, 1_000);
        if let Some(plan) = untold.plan.as_mut() {
            plan.seq = 2;
        }
        let (picked, _) = candidates(
            vec![untold],
            now,
            |_| (0, 0),
            |_| None,
            |_| None,
            &resting,
            &current,
        );
        assert_eq!(
            picked
                .iter()
                .map(|c| (c.plan_untold, c.plan, c.order))
                .collect::<Vec<_>>(),
            vec![(true, false, (1_000, 1))],
            "untold even inside the step's back-off"
        );
    }

    /// **A live plan's and an idle rig's keys are still due** (doc 20 cut SM.11; the brief's
    /// 3.8): a planned rig's `PLAN_CYCLE` key and an idle rig's `IDLE_CYCLE` key both read in
    /// `still_due`. Its own test since doc 23 cut S.1 (`RigRecord` grew its open sortie, and
    /// the two records here were the last straw for the candidates test's stack frame under the
    /// M7 memory-size limits).
    #[test]
    fn sm11_the_plan_and_idle_keys_are_still_due() {
        use crate::surface::plan::IDLE_PARK_IC_SECS;
        let now = 100_000;
        let launched = now - IDLE_PARK_IC_SECS;
        let due = still_due(&[planned(1, 1_000), out_idle(2, launched)], now);
        assert!(due.contains(&(1, 6, PLAN_CYCLE)), "{due:?}");
        assert!(due.contains(&(2, 6, IDLE_CYCLE)), "{due:?}");
    }

    /// **The plan and idle keys never meet a cycle** (doc 20 cut SM.11; the brief's 3.8):
    /// `PLAN_CYCLE`, `IDLE_CYCLE` and SM.8's `QUEUED_CYCLE` are distinct and all past every run
    /// cycle, and `retried_ordinals` names none of them.
    #[test]
    fn sm11_the_plan_and_idle_keys_never_meet_a_cycle() {
        const {
            assert!(PLAN_CYCLE > crate::surface::excursion::MAX_RUN_CYCLES);
            assert!(IDLE_CYCLE > crate::surface::excursion::MAX_RUN_CYCLES);
            assert!(PLAN_CYCLE != QUEUED_CYCLE && IDLE_CYCLE != QUEUED_CYCLE);
            assert!(PLAN_CYCLE != IDLE_CYCLE);
        };
        let mut state = SurfaceTickState::default();
        state.attempts.insert((900, 6, QUEUED_CYCLE), 3);
        state.attempts.insert((900, 6, PLAN_CYCLE), 3);
        state.attempts.insert((900, 6, IDLE_CYCLE), 3);
        state.attempts.insert((900, 4, 0), 2);
        assert_eq!(retried_ordinals(&state, ObjId(900)), vec![(4, 0)]);
    }

    /// **A new operator or excursion starts a clean log** (doc 20 cut SM.9; the brief's 3.4a,
    /// decision D16): the counter's rigs are leased again and again, so a projection another
    /// operator wrote — or one of another excursion — keeps none of its log, leg or run, while
    /// the per-TV ordinals `applied`, `told` and `told_cycle` carry over (the tick's idempotency
    /// reads them); the same operator on the same excursion keeps every line.
    #[test]
    fn sm9_a_new_operator_or_excursion_starts_a_clean_log() {
        let a = ObjId(41);
        let b = ObjId(42);
        let first = Some(ExcursionId {
            tv: ObjId(900),
            launch_ic: 1_000,
        });
        let second = Some(ExcursionId {
            tv: ObjId(900),
            launch_ic: 9_000,
        });
        let mut stored = TvRec::new(ObjId(900), 1);
        stored.operator = Some(a);
        stored.excursion = first;
        stored.applied = 7;
        stored.told = 7;
        stored.told_cycle = 2;
        stored.leg = Some(FrozenLeg {
            started_ic: 1_100,
            waypoints: Vec::new(),
        });
        for (at, text) in [
            (1_100, "A's scan"),
            (1_200, "A's arrival"),
            (1_300, "A's cycle"),
        ] {
            stored.push_log(at, text);
        }
        let ordinals = |rec: &TvRec| (rec.applied, rec.told, rec.told_cycle, rec.slot, rec.tv);
        // Another operator: nothing of A's log, leg or run; the ordinals carry over.
        let for_b = carried_for(stored.clone(), b, first);
        assert!(for_b.log.is_empty() && for_b.leg.is_none() && for_b.run.is_none());
        assert_eq!(ordinals(&for_b), ordinals(&stored));
        // A again on a new excursion: a clean log too.
        let next = carried_for(stored.clone(), a, second);
        assert!(next.log.is_empty());
        assert_eq!(ordinals(&next), ordinals(&stored));
        // A on the same excursion: the three lines stand.
        let same = carried_for(stored.clone(), a, first);
        assert_eq!(same, stored);
        assert_eq!(same.log.len(), 3);
        // A record no act wrote is taken as is.
        let fresh = TvRec::new(ObjId(900), 1);
        assert_eq!(carried_for(fresh.clone(), b, second), fresh);
    }

    /// **The queued key is its own** (doc 20 cut SM.8; decision R15): `tacsim.surface/900/3/queued`,
    /// never the start's `act_key` nor the finish's `done_key`.
    #[test]
    fn sm8_the_queued_key_is_its_own() {
        let queued = queued_key(ObjId(900), 3).map(|k| k.as_str().to_owned());
        assert_eq!(queued.as_deref(), Some("tacsim.surface/900/3/queued"));
        assert_ne!(
            queued,
            crate::surface::act::act_key(ObjId(900), 3).map(|k| k.as_str().to_owned())
        );
        assert_ne!(
            queued,
            done_key(ObjId(900), 3).map(|k| k.as_str().to_owned())
        );
    }

    /// **An unload that moved nothing is told a whole line** (the lead's pre-commit round, item
    /// 3): with something staying, `UNLOADED_NONE` naming it; with the hopper read empty at the
    /// offer (nothing moved, nothing stays), `UNLOADED_NONE_EMPTY` — never a line with an empty
    /// `{stays}`.
    #[test]
    fn sm8_an_unload_that_moved_nothing_is_told_a_whole_line() {
        let stays = unloaded_none_line("1", "Salt Moth", "2 t of metal ore, run-of-mine");
        assert_eq!(
            stays,
            "Rig 1's hopper still holds 2 t of metal ore, run-of-mine: the Salt Moth's hold has no \
             room for it. +tv/1 dump empties it onto the spoil."
        );
        let empty = unloaded_none_line("1", "Salt Moth", "");
        assert_eq!(
            empty,
            "Rig 1 unloads nothing into the Salt Moth's hold: its hopper was already empty."
        );
        for line in [&stays, &empty] {
            assert!(!line.contains('{'), "{line}");
            assert!(!line.contains("holds :"), "{line}");
        }
    }
}
