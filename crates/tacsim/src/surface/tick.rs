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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{Ctx, ObjId, PerceivedRef, SceneId, World};
use serde_json::json;

use crate::port::families::PortHandles;
use crate::port_tick::{Fire, Step, SwapRefused};
use crate::store::{TacsimStore, put_tv_rec};
use crate::surface::act::{Where, done_key, ramp_label, site_berth, site_name, where_is};
use crate::surface::excursion::{
    ActKind, ActResult, ActView, Outcome, RefusedCode, RigKey, RigRecord, TvPose, TvRec,
    TvRecState, TvState, cells_apart, site_cell,
};

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

/// `(tv.0, act ordinal)`: one act's key in the slot.
type ActKey = (u32, u32);

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
}

/// The last fire's stats, `None` before the first fire that found the layer on.
#[must_use]
pub fn last_surface_fire(w: &World) -> Option<SurfaceFireStats> {
    w.module_state::<SurfaceTickState>()
        .and_then(|state| state.last_fire)
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
/// first finish tries the transfer again.
#[must_use]
pub(crate) fn finish_retried(w: &World, tv: ObjId) -> Vec<u32> {
    w.module_state::<SurfaceTickState>()
        .map(|state| retried_ordinals(state, tv))
        .unwrap_or_default()
}

/// [`finish_retried`], pure over the slot.
fn retried_ordinals(state: &SurfaceTickState, tv: ObjId) -> Vec<u32> {
    let ordinals: Vec<u32> = state
        .attempts
        .range((tv.0, 1)..=(tv.0, u32::MAX))
        .map(|(&(_, ordinal), _)| ordinal)
        .collect();
    let skip = ordinals.len().saturating_sub(MAX_RETRIED_ORDINALS);
    ordinals[skip..].to_vec()
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
    let Some(slot) = ctx.w.module_state_mut::<SurfaceTickState>() else {
        return;
    };
    let mut state = std::mem::take(slot);
    state.fires = state.fires.saturating_add(1);
    let stats = walk(ctx, &mut state);
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
                if state.paged.insert((record.tv.0, 0)) {
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
    untold: bool,
    order: (i64, u32),
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
pub(crate) fn candidates(
    records: Vec<RigRecord>,
    now: i64,
    told: impl Fn(ObjId) -> u32,
    state: &SurfaceTickState,
) -> (Vec<Candidate>, u32) {
    let mut passed_over = 0_u32;
    let mut candidates: Vec<Candidate> = records
        .into_iter()
        .filter_map(|record| {
            let told = told(record.tv);
            let due_at = record
                .act
                .as_ref()
                .filter(|act| act.due_ic <= now)
                .filter(|act| {
                    let resting = backing_off(state, (record.tv.0, act.ordinal));
                    if resting {
                        passed_over = passed_over.saturating_add(1);
                    }
                    !resting
                })
                .map(|act| act.due_ic);
            let untold_at = record
                .last
                .as_ref()
                .filter(|last| last.ordinal > told)
                .map(|last| last.at_ic);
            let at = due_at.or(untold_at)?;
            Some(Candidate {
                order: (at, record.tv.0),
                due: due_at.is_some(),
                untold: untold_at.is_some(),
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
    // Every act still due over the whole scan, taken **before** the visit cut: an act past
    // the cut is still due, so its retry bookkeeping must survive the prune (the brief's
    // 3.5 step 4; review finding conformance/F2).
    let still_due = still_due(&records, now);
    let (candidates, resting) = {
        let w: &World = ctx.w;
        candidates(records, now, |tv| told_of(w, tv), state)
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
            tell(ctx, record, last);
            stats.told = stats.told.saturating_add(1);
            stats.worked = stats.worked.saturating_add(1);
        }
        let step = match (candidate.due, &record.act, handles.surface_done.as_ref()) {
            (true, Some(act), Some(done)) => {
                let key = (tv.0, act.ordinal);
                let pending = ctx.authoritative_pending(act.operator);
                match attempt_visit(state, key, pending) {
                    Visit::PassOver => {
                        stats.passed_over = stats.passed_over.saturating_add(1);
                        Step::Held
                    }
                    Visit::Page => {
                        tracing::error!(
                            tv = tv.0,
                            act = act.ordinal,
                            kind = act.kind.word(),
                            "tacsim: a surface act's finish has not landed after 8 tries \
                             (tacsim.excursion {}, act {}, {}); the rig and its hull stay held \
                             (TV_BUSY) until it lands; the tick retries every 60 s; staff read \
                             the finish's refusal in the engine log; a finish that lands clears \
                             it",
                            tv.0,
                            act.ordinal,
                            act.kind.word()
                        );
                        stats.passed_over = stats.passed_over.saturating_add(1);
                        Step::Held
                    }
                    Visit::Submit => {
                        world_step(ctx, record);
                        project_due(ctx, record);
                        let step = submit_done(
                            ctx,
                            &mut fire,
                            &mut state.paged,
                            done,
                            (tv, act.ordinal),
                            act.operator,
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
            (true, Some(act), None) => {
                if state.paged.insert((tv.0, act.ordinal)) {
                    tracing::error!(
                        tv = tv.0,
                        "tacsim: a rig's act is due but the finish is not registered on this \
                         roster; it waits"
                    );
                }
                Step::Refused
            }
            _ => Step::Worked,
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
    }
    stats
}

/// **Every act still due** among `records` at `now`: the keys the slot's retry bookkeeping
/// may keep. Read over the whole scan, never over the fire's visit cut (an act past
/// [`MAX_EXCURSIONS_VISITED_PER_FIRE`] is still due and keeps its attempts, its back-off and
/// its once-a-boot page; review finding conformance/F2).
fn still_due(records: &[RigRecord], now: i64) -> BTreeSet<ActKey> {
    records
        .iter()
        .filter_map(|record| {
            record
                .act
                .as_ref()
                .filter(|act| act.due_ic <= now)
                .map(|act| (record.tv.0, act.ordinal))
        })
        .collect()
}

/// The projection's `told`, 0 when absent or unreadable.
fn told_of(w: &World, tv: ObjId) -> u32 {
    match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    {
        Some(TvRecState::Present(rec)) => rec.told,
        Some(TvRecState::Unreadable) | None => 0,
    }
}

/// The projection as stored, else a new one.
fn projection_of(w: &World, tv: ObjId, slot: u8) -> TvRec {
    match w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    {
        Some(TvRecState::Present(rec)) => rec.clone(),
        Some(TvRecState::Unreadable) | None => TvRec::new(tv, slot),
    }
}

/// The ground pose of `tv`, when it stands `Direct`.
fn pose_of(w: &World, tv: ObjId) -> Option<TvPose> {
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

/// Write `rec` unless the store already holds it.
fn write_projection(ctx: &mut Ctx<'_>, rec: &TvRec) {
    let same = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_rec(rec.tv))
        .is_some_and(|state| *state == TvRecState::Present(rec.clone()));
    if same {
        return;
    }
    if let Err(error) = rec.check() {
        tracing::error!(tv = rec.tv.0, %error, "tacsim: a rig projection is out of bounds; it is not written");
        return;
    }
    put_tv_rec(ctx, rec);
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
    let tv = record.tv;
    let berth = hull_view_berth(ctx.w, act.hull);
    let Some((entry, domain, ramp, bay_room, bay_slot)) = berth else {
        return;
    };
    if entry != act.site {
        return;
    }
    match act.kind {
        ActKind::Launch => {
            if where_is(ctx.w, tv) == (Where::Bay { hull: act.hull })
                && let Err(error) = mush_sim::place(ctx, tv, domain, ramp)
            {
                tracing::warn!(tv = tv.0, %error, "tacsim: a launch's placement was refused; the finish records it");
            }
        }
        ActKind::Recover => {
            if let Where::Ground {
                domain: on,
                x_cm,
                y_cm,
                ..
            } = where_is(ctx.w, tv)
                && on == domain
                && cells_apart(site_cell(x_cm, y_cm), site_cell(ramp.x_cm, ramp.y_cm)) <= 1
                && let Err(error) = mush_sim::place_interior(ctx, tv, bay_room, Some(bay_slot))
            {
                tracing::warn!(tv = tv.0, %error, "tacsim: a recover's placement was refused; the finish records it");
            }
        }
        ActKind::Dump | ActKind::Probe => {}
    }
}

/// The hull's site berth, owned: `(entry, domain, ramp, bay room, bay slot)`.
#[allow(clippy::type_complexity)]
fn hull_view_berth(
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
    let mut rec = projection_of(ctx.w, record.tv, act.slot);
    rec.slot = act.slot;
    rec.operator = Some(act.operator);
    rec.hull = Some(act.hull);
    rec.excursion = record.excursion.as_ref().map(|excursion| excursion.id);
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
    rec.applied = act.ordinal;
    write_projection(ctx, &rec);
}

/// **The finish, as the operator** (the brief's 3.5): one closed call through
/// `Fire::as_principal`; `AlreadyActed`, `Owed` and `Pending` defer (one act per principal per
/// fire, A9); `God` and `Gone` refuse, **logged once a boot** under the act's key in `paged`
/// (review finding conformance/F1: the act stays due and is visited every 1 s fire, so an
/// unguarded line would repeat each second for as long as the record stands). The key is the
/// one the attempt rule pages under; an act whose operator is GOD or gone never submits, so
/// it never counts an attempt and the two pages cannot meet.
fn submit_done(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    paged: &mut BTreeSet<ActKey>,
    done: &mush_core::DefinitionRef,
    (tv, ordinal): (ObjId, u32),
    operator: ObjId,
) -> Step {
    let Some(key) = done_key(tv, ordinal) else {
        tracing::error!(tv = tv.0, "tacsim: a finish's key did not form");
        return Step::Refused;
    };
    let parameters = json!({ "tv": tv.0.to_string(), "ordinal": ordinal });
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
            if paged.insert((tv.0, ordinal)) {
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

/// **The tell** (the brief's 3.5): the projection (`told` moved to the last act, the state
/// and pose as they stand, one log line stamped at the act's own instant) and the completion
/// line to the operator. A probe's line is logged and never said (a test world's act).
fn tell(ctx: &mut Ctx<'_>, record: &RigRecord, last: &Outcome) {
    let w: &World = ctx.w;
    let mut rec = projection_of(w, record.tv, last.slot);
    let hull = record
        .excursion
        .as_ref()
        .map(|excursion| excursion.hull)
        .or(rec.hull)
        .or_else(|| {
            w.module_state::<TacsimStore>()
                .and_then(|store| store.tv_lease_on(record.tv))
                .map(|lease| lease.hull)
        });
    let line = completion_line(w, last, hull);
    rec.slot = last.slot;
    rec.operator = Some(last.operator);
    rec.hull = hull;
    rec.excursion = record.excursion.as_ref().map(|excursion| excursion.id);
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
    rec.told = last.ordinal;
    rec.push_log(last.at_ic, &line);
    write_projection(ctx, &rec);
    if last.kind != ActKind::Probe {
        crate::port_tick::notify(ctx, last.operator, &line);
    }
}

/// **The completion line** of a finished act (the brief's 3.6 texts).
fn completion_line(w: &World, last: &Outcome, hull: Option<ObjId>) -> String {
    use crate::port::tv;
    let n = last.slot.to_string();
    let ramp = hull.map_or_else(|| "ramp".to_owned(), |hull| ramp_label(w, hull));
    let name = hull.map_or_else(String::new, |hull| crate::port::coop::name_of(w, hull));
    let site = site_name(w, &last.site);
    match last.result {
        ActResult::Done => match last.kind {
            ActKind::Launch => tv::LAUNCHED
                .replace("{n}", &n)
                .replace("{ramp}", &ramp)
                .replace("{site}", &site),
            ActKind::Recover => tv::RECOVERED.replace("{n}", &n).replace("{name}", &name),
            ActKind::Dump => tv::DUMPED
                .replace("{n}", &n)
                .replace("{load}", &crate::surface::act::load_words(w, &last.moved))
                .replace("{site}", &site),
            ActKind::Probe => format!("Rig {n} ran a test."),
        },
        ActResult::Refused { code } => {
            let why = match code {
                RefusedCode::NeverLeft => tv::WHY_NEVER_LEFT.to_owned(),
                RefusedCode::NotBeside => tv::WHY_NOT_BESIDE.replace("{ramp}", &ramp),
                RefusedCode::HopperChanged => tv::WHY_HOPPER_CHANGED.to_owned(),
                RefusedCode::SpoilGone => tv::WHY_SPOIL_GONE.to_owned(),
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

    /// **The attempt rule** (finding F2): a deferral never counts; a `Submitted` step then a
    /// pending visit does not count; then a visit with nothing pending counts one; the eighth
    /// counted attempt pages once and passes the act over for `DONE_BACKOFF_FIRES − 1` fires,
    /// then submits again; a second exhaustion does not page twice in one boot.
    #[test]
    fn sm3b_the_attempt_rule_counts_only_finishes_that_did_not_land() {
        let mut state = SurfaceTickState::default();
        let key = (900, 1);
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
        state.attempts.insert((tv.0, 1), 2);
        state.attempts.insert((901, 2), 1);
        state.paged.insert((tv.0, 0));
        let failed = retried_ordinals(&state, tv);
        assert_eq!(failed, vec![1]);
        assert!(
            !failed.contains(&2),
            "a stale attempt of act 1 never marks act 2"
        );
        state.attempts.insert((tv.0, 2), 1);
        let failed = retried_ordinals(&state, tv);
        assert!(failed.contains(&2), "an attempt at (tv, 2) marks act 2");
        assert_eq!(failed, vec![1, 2]);
        assert!(retried_ordinals(&state, ObjId(902)).is_empty());
        // Bounded: the highest ordinals are kept.
        let cap = u32::try_from(MAX_RETRIED_ORDINALS).expect("fits");
        let over = cap + 5;
        for ordinal in 1..=over {
            state.attempts.insert((tv.0, ordinal), 1);
        }
        let failed = retried_ordinals(&state, tv);
        assert_eq!(failed.len(), MAX_RETRIED_ORDINALS);
        assert_eq!(failed.last(), Some(&over));
        assert_eq!(failed.first(), Some(&(over + 1 - cap)));
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
            }),
            excursion: None,
            last: None,
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
        assert!(due.contains(&(over, 3)));
        assert!(due.contains(&(1, 3)), "past the visit cut, still due");
        assert!(!due.contains(&(5_000, 3)), "not yet due");
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
            }),
            excursion: None,
            last: None,
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
            state.backoff.insert((tv, 2), 500);
        }
        let told = |_: ObjId| 0;
        // In the back-off: the fresh act is the one candidate.
        let (picked, resting) = candidates(records.clone(), now, told, &state);
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
        let (picked, _) = candidates(records.clone(), now, told, &state);
        assert_eq!(
            picked.len(),
            1,
            "still in the back-off one fire before its end"
        );
        // The back-off over: the 64 are candidates again, oldest first, and fill the window.
        state.fires = 500 + DONE_BACKOFF_FIRES;
        let (picked, resting) = candidates(records.clone(), now, told, &state);
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
        });
        let (picked, resting) = candidates(vec![untold], now, told, &state);
        assert_eq!(resting, 1);
        assert_eq!(picked.len(), 1);
        assert!(picked[0].untold && !picked[0].due, "told, not submitted");
    }
}
