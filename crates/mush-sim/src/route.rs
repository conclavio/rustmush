//! # Journeys — the mobility / route-interception primitive (design 07 §9)
//!
//! A **plain object** (a WoD black car, a courier) follows an ordered path of
//! ordinary rooms over a `Coarse` sweep: it **never teleports**, it is
//! **legible** to anyone co-present as it passes, and a third party can
//! **intercept** it. This is an independent module clock that relocates plain
//! objects room-to-room — it is NOT `DomainClock` (still Passive-only) and is
//! NOT a placement-graph citizen (it never touches `SpatialState`).
//!
//! Time is in **seconds** (the only clock a tick can read is `world.now()`,
//! unix seconds). A leg's `travel_s` is both its travel time and the dwell in
//! the room it reaches — the window in which a co-present observer can waylay.
//!
//! The load-bearing invariant (doc 07 §9), scoped to LIVE advancement: while
//! the server runs at wall-clock pace a passage visits and narrates every room
//! in order (never a skip, never instantaneous relocation). A catch-up after
//! downtime is not a skipped *opportunity* — no live observer existed — so the
//! sweep advances the cursor to the object's correct current position in ONE
//! move and suppresses the stale intervening narration. That single-move design
//! also bounds a catch-up to one relocation per journey per sweep (no storm).

use std::collections::BTreeMap;

use mush_core::{Ctx, Deferred, Flags, ModuleOp, ModuleWrite, ObjId, WorldEvent, output::name_span};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The sweep cadence (seconds). A journey's own absolute due-times are the
/// authority; this only sets how often we check and the interception latency.
pub const SWEEP_SECS: u64 = 1;
/// The registered tick tag.
pub const SWEEP_TAG: &str = "route.sweep";
/// An arrival whose due-time is within this many seconds of now is LIVE
/// (narrated); older arrivals are catch-up (silent).
const LIVE_GRACE_S: i64 = 2;
/// After a resume, the journey is immune to re-interception this long
/// (anti-grief: the owner gets it moving before it can be re-waylaid).
const RESUME_GRACE_S: i64 = 5;
/// Default per-leg travel time when a route token gives no explicit seconds.
pub const DEFAULT_TRAVEL_S: u32 = 30;

/// One leg of a route: travel `travel_s` seconds, then arrive at room `to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leg {
    /// The room reached at the end of this leg.
    pub to: ObjId,
    /// Seconds to traverse it (>= 1; also the dwell/interception window).
    pub travel_s: u32,
}

/// A journey's lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JourneyStatus {
    /// Advancing on the sweep.
    Traveling,
    /// Held by an interception (or staff); resume shifts the schedule.
    Paused,
    /// Reached the last leg; the record is being retired.
    Arrived,
    /// Ended early (destroyed traveler/room, staff abort); being retired.
    Aborted,
}

/// A scheduled route a plain object is following (`journey:<traveler>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Journey {
    /// The moving object.
    pub traveler: ObjId,
    /// The ordered legs.
    pub legs: Vec<Leg>,
    /// Legs already reached (`0..=legs.len()`); the traveler is at
    /// `legs[cursor-1].to` (or its origin when 0).
    pub cursor: usize,
    /// Absolute start (unix seconds); `due(k) = started_at + sum(travel_s[0..=k])`.
    pub started_at: i64,
    /// Lifecycle.
    pub status: JourneyStatus,
    /// When it was paused (to shift the schedule on resume).
    pub paused_at: Option<i64>,
    /// No interception before this instant (unix seconds).
    pub grace_until: i64,
    /// The final destination (`legs.last().to`) — reserved for a typed
    /// `KnownPlaceRef` later; never revealed to observers in cut 1.
    pub destination: ObjId,
    /// Who authorized the route (staff).
    pub by: ObjId,
}

impl Journey {
    /// The absolute due-time to have reached `legs[k]`.
    #[must_use]
    fn due(&self, k: usize) -> i64 {
        self.started_at + self.legs[..=k].iter().map(|l| i64::from(l.travel_s)).sum::<i64>()
    }
    /// The absolute due-time of the next unreached leg, if any.
    #[must_use]
    pub fn next_due(&self) -> Option<i64> {
        (self.cursor < self.legs.len()).then(|| self.due(self.cursor))
    }
}

/// The module's journey slot. Never persisted as a whole (each journey is one
/// doc); rebuilt at boot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouteState {
    /// Journeys by traveler.
    pub journeys: BTreeMap<ObjId, Journey>,
}

impl RouteState {
    /// Whether `obj` is on a live (Traveling/Paused) journey.
    #[must_use]
    pub fn is_traveling(&self, obj: ObjId) -> bool {
        self.journeys
            .get(&obj)
            .is_some_and(|j| matches!(j.status, JourneyStatus::Traveling | JourneyStatus::Paused))
    }
}

// ---- persistence ------------------------------------------------------

/// The doc key of a journey.
#[must_use]
pub fn journey_key(traveler: ObjId) -> SmolStr {
    SmolStr::new(format!("journey:{}", traveler.0))
}

pub(crate) fn persist_journey(out: &mut mush_core::Outbox, j: &Journey) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static("sim"),
        op: ModuleWrite::JsonSet {
            key: journey_key(j.traveler),
            value: serde_json::to_value(j).expect("journey serializes"),
        },
    }));
}
pub(crate) fn persist_journey_del(out: &mut mush_core::Outbox, traveler: ObjId) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static("sim"),
        op: ModuleWrite::Del { key: journey_key(traveler) },
    }));
}

// ---- the gate reason --------------------------------------------------

/// Why a traveling object may not be moved/carried by ordinary means (it moves
/// only via the sweep, an interception, or staff `@route/abort`).
#[must_use]
pub fn traveling_reason(rs: &RouteState, w: &mush_core::World, obj: ObjId) -> Option<String> {
    rs.is_traveling(obj).then(|| {
        format!(
            "{} is en route; it moves only along its route (staff @route/abort to stop it).",
            mush_core::output::display_name(w, obj)
        )
    })
}

// ---- narrative helpers (narrative_emit is crate-private in core) -------

fn emit(text: RichText) -> OutputEvent {
    OutputEvent::Narrative { kind: NarrativeKind::Emit, actor: None, text }
}

// ---- the sweep --------------------------------------------------------

/// The `Coarse` tick: advance every traveling journey to its correct current
/// position, narrating only near-present (live) arrivals.
pub(crate) fn sweep(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    let now = ctx.w.now();
    let mut rs = std::mem::take(
        ctx.w.module_state_mut::<RouteState>().expect("route state slot installed at registration"),
    );
    let mut retire: Vec<ObjId> = Vec::new();
    let mut touched: Vec<ObjId> = Vec::new();

    for (&traveler, j) in rs.journeys.iter_mut() {
        if j.status != JourneyStatus::Traveling {
            continue;
        }
        match j.next_due() {
            None => {
                j.status = JourneyStatus::Arrived;
                retire.push(traveler);
                continue;
            }
            Some(nd) if nd > now => continue, // not due yet
            Some(_) => {}
        }
        // The traveler must still exist.
        let from = match ctx.w.get(traveler) {
            Some(o) if !o.flags.contains(Flags::GOING) => o.location,
            _ => {
                j.status = JourneyStatus::Aborted;
                retire.push(traveler);
                continue;
            }
        };
        // Advance the cursor to the correct current position (one relocation,
        // even across a downtime backlog — no per-leg storm).
        while j.cursor < j.legs.len() && j.due(j.cursor) <= now {
            j.cursor += 1;
        }
        let reached = j.cursor - 1;
        let dest = j.legs[reached].to;
        if ctx.w.move_to(traveler, dest).is_err() {
            // A leg room vanished/became unenterable: end the journey.
            j.status = JourneyStatus::Aborted;
            retire.push(traveler);
            touched.push(traveler);
            continue;
        }
        // Narrate only a LIVE arrival (near its due-time); a stale catch-up
        // relocation is silent (no live observer existed).
        if now - j.due(reached) <= LIVE_GRACE_S {
            if let Some(fr) = from {
                ctx.out.to_room_from(
                    fr,
                    vec![traveler],
                    traveler,
                    emit(RichText::new().span(name_span(ctx.w, traveler)).text(" departs.")),
                );
            }
            ctx.out.to_room_from(
                dest,
                vec![traveler],
                traveler,
                emit(RichText::new().span(name_span(ctx.w, traveler)).text(" arrives.")),
            );
        }
        ctx.emit_world_event(&WorldEvent::Moved { who: traveler, from, to: dest });
        if j.cursor == j.legs.len() {
            j.status = JourneyStatus::Arrived;
            retire.push(traveler);
        }
        touched.push(traveler);
    }

    // One persist per touched journey; delete terminal ones.
    for t in &touched {
        if let Some(j) = rs.journeys.get(t) {
            persist_journey(ctx.out, j);
        }
    }
    for t in retire {
        rs.journeys.remove(&t);
        persist_journey_del(ctx.out, t);
    }
    *ctx.w.module_state_mut::<RouteState>().expect("route state slot installed at registration") = rs;
}

// ---- lifecycle: a destroyed traveler drops its journey ----------------

pub(crate) fn on_traveler_destroyed(ctx: &mut Ctx<'_>, obj: ObjId) {
    let removed = ctx
        .w
        .module_state_mut::<RouteState>()
        .expect("route state slot installed at registration")
        .journeys
        .remove(&obj)
        .is_some();
    if removed {
        persist_journey_del(ctx.out, obj);
    }
}

// ---- interception / resume (called by commands) -----------------------

/// Pause `traveler`'s journey (an interception). Returns the room it was
/// stopped in on success.
pub(crate) fn pause(rs: &mut RouteState, traveler: ObjId, now: i64) -> Result<(), String> {
    let j = rs.journeys.get_mut(&traveler).ok_or("It is not going anywhere.")?;
    if j.status != JourneyStatus::Traveling {
        return Err("It is already stopped.".into());
    }
    if now < j.grace_until {
        return Err("It slips past before you can act — try again in a moment.".into());
    }
    j.status = JourneyStatus::Paused;
    j.paused_at = Some(now);
    Ok(())
}

/// Resume a paused journey, shifting its schedule forward by the pause length
/// so the remaining legs keep their relative timing, with a re-interception
/// grace.
pub(crate) fn resume(rs: &mut RouteState, traveler: ObjId, now: i64) -> Result<(), String> {
    let j = rs.journeys.get_mut(&traveler).ok_or("It is not going anywhere.")?;
    if j.status != JourneyStatus::Paused {
        return Err("It is not paused.".into());
    }
    if let Some(p) = j.paused_at.take() {
        j.started_at += now - p;
    }
    j.status = JourneyStatus::Traveling;
    j.grace_until = now + RESUME_GRACE_S;
    Ok(())
}
