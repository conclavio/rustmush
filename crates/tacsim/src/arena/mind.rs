//! **The mind seam** (17b cut M1a.5; doc 17 §3 "The view", §4 "Intents", §6
//! "When a mind thinks", §7 "The runner"; D-UM-1, D-UM-2, D-UM-4, D-UM-8,
//! D-UM-9, D-UM-13, D-UM-20, D-UM-21, D-UM-23).
//!
//! A [`Controller::Mind`] hull is flown by a **unit mind**: something that is
//! handed the hull's own view, its own memory and the map, and answers a short
//! list of [`IntentKind`]s and a new memory. This file is everything the arena
//! does around that call, and nothing of what answers it:
//!
//! * **The state** a mind carries between thinks ([`MindState`], one row per
//!   hull on [`Arena::minds`]): its think counter, the think in flight, its
//!   memory's bytes, its notice buffer, its range memory, the standing order a
//!   squad gave it and its strikes. Hashed (`crate::arena::hash`, item 3c) and
//!   carried by a snapshot, because every one of them decides a later intent.
//! * **The view** ([`Arena::mind_view`], [`MindView`]): the hull's keyframe and
//!   nothing finer (D-UM-4). It is built by the one builder a pilot's frame is
//!   built by (`Arena::frame_for`) and projected through the one row formatter
//!   a pilot's `CONTACTS` reads ([`crate::arena::view::frame_contacts_held`]),
//!   so a mind can never read what the keyframe does not carry.
//! * **The think** ([`DueKind::MindThink`]): a due event on the stagger grid,
//!   every [`STOCK_THINK_MS`] of arena time, resolved in the substep its
//!   timestamp falls in **after that millisecond's other due items** (D-UM-20;
//!   `Arena::substep`'s due loop holds the mind kinds back), against the state
//!   then. The runner is called **synchronously** here; a think of more than
//!   [`MIND_INTENTS_MAX`] intents is a fault and lands nothing (D-UM-9).
//! * **The landing** ([`DueKind::MindLand`]): one substep later, the think's
//!   intents go through the pilot's own door (`Arena::apply_machine_intents`,
//!   D-UM-12) at the epochs the think recorded (D-UM-13) under
//!   [`IntentSource::Mind`], so a seat that changed hands between the two
//!   refuses the stale thought `Epoch`, and each refusal is appended to the
//!   mind's own notice buffer (D-UM-8: *a refusal is never silent*).
//!
//! # What runs a think
//!
//! [`MindRunner`] is a trait and this library implements it once, with
//! [`NoMind`], which thinks nothing. The real runners live outside the library
//! (doc 17 §7, D-UM-1): 17b cut M1a.6's native stock mind, and later the
//! interpreter the sim task drives — the Rhai engine of doc 17 §2 — which this
//! file must never name in code: the contract test
//! `crates/mush-contract-tests/tests/arena_library.rs` refuses the lowercase
//! crate name anywhere under `crates/tacsim/src/arena/`. The arena calls the
//! runner and reads its answer; it never times it (there is no wall clock
//! here: the twin lane times a view and a think from the test), and it never
//! decodes the memory it hands back (D-UM-30: canonical bytes, opaque).
//!
//! # Production
//!
//! No lobby, check-out or control sets [`Controller::Mind`] in this cut, so no
//! production match pops a mind kind, folds a mind row or calls a runner: the
//! two pins of 17b cut M1a.1 (`tests/reflex.rs`,
//! `m1a1_the_moved_motor_folds_the_same_bytes`; `tests/motor.rs`,
//! `m1a1_the_shipped_duel_hashes_as_before`) are re-asserted unchanged by
//! `tests/minds.rs`. 17b cut M4 is the flip.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::intents::{IntentKind, IntentSource};
use crate::arena::ledger::EventKind;
use crate::arena::perception::{Aspect, FrameCondition, FrameNotice, FrameOrder, Receiver};
use crate::arena::reflex::{Landed, ReflexCounters};
use crate::arena::state::{
    Arena, ArenaDefinition, Channel, Classification, ContactId, Controller, Designator, DueKind,
    Power, Team,
};
use crate::arena::step::{FrameHeat, FrameSection};
use crate::arena::view::ContactRow;
use crate::arena::weapons::FrameWeapon;
use crate::config::ArenaConfig;

/// **How often a stock mind thinks**, arena milliseconds (17b cut M1a.5; doc
/// 17 §9, D-UM-35's class 0/I cadence). A constant until 17b cut M3's
/// `CoreProfile.think_ms` replaces it, read in one place: the think arm
/// schedules the next think this far after its own lattice millisecond.
pub const STOCK_THINK_MS: i64 = 1_000;

/// **How many notices a mind's buffer holds** (D-UM-8): the newest
/// `MIND_NOTICES`, the oldest dropped first.
pub const MIND_NOTICES: usize = 16;

/// **The most intents one think may answer** (D-UM-9). A think that answers
/// more is a fault: it lands **nothing** and keeps its old memory — never a
/// truncation, which would land a prefix of a plan its author never wrote.
pub const MIND_INTENTS_MAX: usize = 16;

/// **How many contacts a view carries** (doc 17 §3; D-UM-35's class I
/// `contact_rows`): the nearest twelve by the held range, then by
/// [`ContactId`]. A constant until 17b cut M3's `CoreProfile` replaces it.
pub const STOCK_CONTACT_ROWS: usize = 12;

/// **The think in flight** (D-UM-23's two rows): which think and the lattice
/// millisecond it was taken at. `Some` from the think to its landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingThink {
    /// The think's sequence number.
    pub think_seq: u64,
    /// The arena millisecond it was taken at — its lattice millisecond.
    pub t_ms: i64,
}

/// **What one unit mind holds between thinks** (17b cut M1a.5; doc 17 §3, §6,
/// §8), one row per [`Controller::Mind`] hull on [`Arena::minds`].
///
/// Every field decides a later intent, so every field is hashed
/// (`crate::arena::hash`, item 3c) and carried by a snapshot — the argument
/// the reflex machine's own row was folded on (16b cut N.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindState {
    /// **The next think's sequence number**: a [`DueKind::MindThink`] whose
    /// `think_seq` is not this one is stale and does nothing.
    pub think_seq: u64,
    /// **The think waiting to land** (D-UM-23), or `None`.
    pub pending: Option<PendingThink>,
    /// **The mind's memory** (D-UM-30): canonical bytes the runner writes and
    /// the arena never decodes. Written at the landing, so a fault keeps the
    /// old bytes.
    pub memory: Vec<u8>,
    /// **The notice buffer** (D-UM-8): every notice staged on the hull since
    /// the last think — the pilot's own notices, and the mind's own refusals
    /// — at most [`MIND_NOTICES`], the oldest dropped; drained into the next
    /// view.
    pub notices: Vec<FrameNotice>,
    /// **The range memory** (doc 17 §3; 16b cut B.4's `T-60` hysteresis read
    /// for a mind): the figure each contact's `range_m` was last given as,
    /// settled at every think so a plot near a 50 m edge does not flip.
    pub ranges: BTreeMap<ContactId, u64>,
    /// **The standing order** (D-UM-15 as decided): a squad's order re-keyed
    /// at write time to this hull's own HELD contacts, or `None`.
    pub order: Option<FrameOrder>,
    /// **The faults counted against the mind** (D-UM-31): always `0` in this
    /// cut; 17b cut M3 counts them.
    pub strikes: u8,
}

impl MindState {
    /// **Append one notice to the buffer** (D-UM-8), dropping the oldest past
    /// [`MIND_NOTICES`].
    pub fn push_notice(&mut self, notice: FrameNotice) {
        self.notices.push(notice);
        if self.notices.len() > MIND_NOTICES {
            self.notices.remove(0);
        }
    }
}

/// **One think, waiting to land** (D-UM-21, D-UM-13): what the runner answered
/// and the epochs of the hull's three channels at the think. Scratch on
/// [`Arena::mind_landings`]: neither hashed nor carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landing {
    /// The think it is the answer of.
    pub think_seq: u64,
    /// Each channel's epoch at the think, in [`Channel::ALL`]'s order — what
    /// the landing's intents are stamped with, so a seat that changed hands in
    /// between refuses them `Epoch`.
    pub epochs: [(Channel, u64); 3],
    /// The intents, in the order the runner emitted them.
    pub intents: Vec<IntentKind>,
    /// The memory the runner handed back.
    pub memory: Vec<u8>,
}

/// **What runs a think** (doc 17 §7, D-UM-1): the seam the sim task plugs a
/// runner into. Called synchronously inside the think's due event; it must be
/// a pure function of the job for the cross-tempo property to hold.
pub trait MindRunner: std::fmt::Debug + Send + Sync {
    /// Think once.
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult;
}

/// **One think's input** (doc 17 §6).
#[derive(Clone, Copy, Debug)]
pub struct ThinkJob<'a> {
    /// The hull thinking — its own object number, which is also the view's
    /// `unit` (doc 17 §3 reads "no kernel takes or returns an `ObjId`" as about
    /// other units: the M1a plan's departure (e)).
    pub unit: ObjId,
    /// The think's sequence number.
    pub think_seq: u64,
    /// **The think's lattice millisecond** — the [`DueKind::MindThink`]'s own
    /// timestamp. The view's `t_ms` is the end of the substep it fell in, the
    /// state's own clock (D-UM-20's *the view is the state then*).
    pub t_ms: i64,
    /// The hull's view.
    pub view: &'a MindView,
    /// The memory the last landed think left.
    pub memory: &'a [u8],
    /// The map: the window, its zones, its spots and its waypoint graph.
    pub map: &'a ArenaDefinition,
}

/// **One think's answer** (doc 17 §6): the intents in emission order and the
/// new memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThinkResult {
    /// What to do, at most [`MIND_INTENTS_MAX`].
    pub intents: Vec<IntentKind>,
    /// What to remember.
    pub memory: Vec<u8>,
}

/// **The runner that thinks nothing** (17b cut M1a.5): no intent, the memory
/// handed back as it came. What every arena runs until 17b cut M1a.6's native
/// stock mind replaces it in [`default_mind_runner`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoMind;

impl MindRunner for NoMind {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        ThinkResult {
            intents: Vec::new(),
            memory: job.memory.to_vec(),
        }
    }
}

/// **The runner an arena is opened with** (17b cut M1a.5): the one function
/// both [`Arena::new`] and the snapshot's decode call, so a resumed match
/// thinks with the runner a fresh one does. [`NoMind`] in this cut; 17b cut
/// M1a.6 changes the answer and nothing else. The configuration is taken now
/// so that change needs no new caller.
#[must_use]
pub fn default_mind_runner(_cfg: &ArenaConfig) -> Arc<dyn MindRunner> {
    Arc::new(NoMind)
}

/// **What a mind sees** (17b cut M1a.5; doc 17 §3, D-UM-4 read strictly — the
/// M1a plan's departure (h)): the hull's keyframe, field by field, at the
/// keyframe's own resolution and nothing finer.
///
/// Every field is a key of `crate::match_::cockpit::status_frame_held` or a
/// [`ContactRow`] rounding of one: `unit`, `callsign`, `cell`, `x_cm`, `y_cm`,
/// `z_cm`, `heading_mdeg` (the keyframe carries the own pose at those
/// resolutions), `speed_cms`, `power`, `held` (the keyframe's `blocked`),
/// `wreck`, `immobile`, `sight_m`, `ecm`, `receiver`, `heat`, `sections`,
/// `weapons` (each row's keyframe keys, `bands_cm` included since 17b cut
/// M1a.3), `lock`, `order`, `t_ms`, `notices` and `contacts`. `team` is the
/// keyframe's callsign letter read as a side. The only object number in it is
/// the hull's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MindView {
    /// The hull's own object number (the keyframe's `unit`).
    pub unit: u32,
    /// Its public name (`callsign`).
    pub callsign: Designator,
    /// Its side.
    pub team: Team,
    /// The cell it stands in (`cell`).
    pub cell: Option<(u16, u16)>,
    /// Its own pose, centimetres (`x_cm`, `y_cm`, `z_cm`).
    pub x_cm: i64,
    /// North.
    pub y_cm: i64,
    /// Height.
    pub z_cm: i64,
    /// Its heading, millidegrees (`heading_mdeg`).
    pub heading_mdeg: u32,
    /// Its speed, cm/s (`speed_cms`).
    pub speed_cms: i32,
    /// Its power (`power`).
    pub power: Power,
    /// Whether motion is holding it against something (`blocked`).
    pub held: bool,
    /// Whether it is a wreck (`wreck`).
    pub wreck: bool,
    /// Whether its motion is disabled (`immobile`).
    pub immobile: bool,
    /// Its own nominal sight, whole metres (`sight_m`).
    pub sight_m: u64,
    /// Its pod's switch, `None` without a pod (`ecm`).
    pub ecm: Option<bool>,
    /// Its own receiver (`receiver`).
    pub receiver: Option<Receiver>,
    /// Its heat (`heat`).
    pub heat: Option<FrameHeat>,
    /// Its own sections (`sections`).
    pub sections: Vec<FrameSection>,
    /// Its weapons (`weapons`), empty on a hull with no weapons station.
    pub weapons: Vec<FrameWeapon>,
    /// The contact it is locked on (`lock`).
    pub lock: Option<ContactId>,
    /// Its standing order, re-keyed to its own rows (`order`).
    pub order: Option<FrameOrder>,
    /// **The arena millisecond the view was built at** (`t_ms`): the end of
    /// the substep the think fell in, the clock every age above is read on.
    pub t_ms: i64,
    /// The notice buffer, drained into this view (`notices`).
    pub notices: Vec<FrameNotice>,
    /// The nearest [`STOCK_CONTACT_ROWS`] contacts (`contacts`).
    pub contacts: Vec<ViewContact>,
}

/// **One contact as a mind sees it** (17b cut M1a.5; doc 17 §3): a keyframe
/// contact's keys at the keyframe's resolution — degrees and the held metres,
/// never millidegrees or centimetres.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewContact {
    /// The row key (`id`).
    pub id: ContactId,
    /// Its short name (`designator`).
    pub designator: Designator,
    /// The number behind a renamed row (`alias`).
    pub alias: Option<u16>,
    /// What the platform knows of it (`class`, the word's source).
    pub class: Classification,
    /// Whether it is on the hull's own side (`friendly`).
    pub friendly: bool,
    /// The bearing to its plot, whole degrees (`bearing_deg`).
    pub bearing_deg: u32,
    /// The same off the hull's heading, whole degrees (`relative_deg`).
    pub relative_deg: u32,
    /// The held range to its plot, whole metres (`range_m`).
    pub range_m: u64,
    /// The band word (`band`).
    pub band: SmolStr,
    /// How old the plot is (`age_ms`).
    pub age_ms: i64,
    /// The plot's cell (`cell`).
    pub cell: Option<(u16, u16)>,
    /// The held reading (`condition`).
    pub condition: Option<FrameCondition>,
    /// The chassis's fatal section words beside a held reading (`fatal`).
    pub fatal: Option<Vec<SmolStr>>,
    /// The facet its plot was showing (`aspect`).
    pub aspect: Aspect,
    /// The odds per weapon, default weapon first (`odds`), as
    /// `(weapon, p_permille)`; empty for a row outside the odds rows and on a
    /// hull with no weapons station.
    pub odds: Vec<(SmolStr, u32)>,
    /// Whether it reads dead (`reads_dead`), `None` on a hull with no weapons
    /// station — the keyframe carries no key there.
    pub reads_dead: Option<bool>,
}

impl Arena {
    /// **The first [`DueKind::MindThink`] of every unit the check-out handed a
    /// mind** (17b cut M1a.5; doc 17 §6), in unit-id order, each with a fresh
    /// [`MindState`] — the shape `Arena::schedule_first_reflex_ticks` has. Only
    /// a fixture fields one in this cut.
    pub fn schedule_first_mind_thinks(&mut self) {
        let minds: Vec<ObjId> = self
            .units
            .iter()
            .filter(|unit| unit.controller == Controller::Mind)
            .map(|unit| unit.id)
            .collect();
        for id in minds {
            self.adopt_mind(id);
        }
    }

    /// **Hand a hull to a mind** (17b cut M1a.5): a [`MindState`] row if it
    /// has none, and its first think at its next slot of the stagger grid
    /// ([`STOCK_THINK_MS`] apart) if the heap holds none for it.
    pub(crate) fn adopt_mind(&mut self, unit: ObjId) {
        let think_seq = self.minds.entry(unit).or_default().think_seq;
        if self
            .due
            .iter()
            .any(|event| matches!(event.kind, DueKind::MindThink { unit: on, .. } if on == unit))
        {
            return;
        }
        let at = self.grid_at(unit, STOCK_THINK_MS);
        self.due
            .schedule(at, DueKind::MindThink { unit, think_seq });
    }

    /// **Take a hull back from its mind** (17b cut M1a.5): the row, the think
    /// waiting to land and every think and landing on the heap.
    pub(crate) fn release_mind(&mut self, unit: ObjId) {
        self.minds.remove(&unit);
        self.mind_landings.remove(&unit);
        self.due.retain(|kind| match kind {
            DueKind::MindThink { unit: on, .. } | DueKind::MindLand { unit: on, .. } => *on != unit,
            _ => true,
        });
    }

    /// **One hull's view, as its next think would read it** (17b cut M1a.5;
    /// doc 17 §3, D-UM-4), or `None` for a unit this arena does not hold or one
    /// with no mind row.
    ///
    /// Built by `Arena::frame_for` — the builder a pilot's keyframe is built by,
    /// which leaves the pilots' notices where they are — and projected through
    /// [`crate::arena::view::frame_contacts_held`] against the mind's own range
    /// memory, which it then settles ([`crate::arena::view::settle_ranges`]):
    /// the rows nearest-first by the held `range_m` then by [`ContactId`],
    /// the first [`STOCK_CONTACT_ROWS`] of them. It **drains** the mind's
    /// notice buffer into the view. Two writes, both hashed state, which is why
    /// it takes `&mut self` and why only the think arm calls it in a match.
    pub fn mind_view(&mut self, unit: ObjId) -> Option<MindView> {
        let frame = self.frame_for(unit, self.clock.tempo())?;
        let lock = crate::arena::view::frame_lock(&frame);
        let state = self.minds.get_mut(&unit)?;
        let mut rows: Vec<ContactRow> =
            crate::arena::view::frame_contacts_held(&frame, &state.ranges);
        state.ranges = crate::arena::view::settle_ranges(&frame, &state.ranges);
        let notices = std::mem::take(&mut state.notices);
        rows.sort_by_key(|row| (row.range_m, row.id));
        rows.truncate(STOCK_CONTACT_ROWS);
        let contacts = rows
            .into_iter()
            .filter_map(|row| {
                let held = frame.contacts.iter().find(|held| held.id == row.id)?;
                Some(ViewContact {
                    id: row.id,
                    designator: row.designator,
                    alias: row.alias,
                    class: held.class,
                    friendly: row.friendly,
                    bearing_deg: row.bearing_deg,
                    relative_deg: row.relative_deg,
                    range_m: row.range_m,
                    band: row.band,
                    age_ms: row.age_ms,
                    cell: row.cell,
                    condition: held.condition,
                    fatal: held.fatal.clone(),
                    aspect: held.aspect,
                    odds: frame
                        .odds
                        .iter()
                        .filter(|odds| odds.contact == row.id)
                        .map(|odds| (odds.weapon.clone(), odds.p_permille))
                        .collect(),
                    reads_dead: held.reads_dead,
                })
            })
            .collect();
        Some(MindView {
            unit: frame.unit.0,
            callsign: frame.callsign,
            team: frame.team,
            cell: frame.cell,
            x_cm: frame.pose.x_cm,
            y_cm: frame.pose.y_cm,
            z_cm: i64::from(frame.pose.z_cm),
            heading_mdeg: frame.pose.heading_mdeg,
            speed_cms: frame.speed_cms,
            power: frame.power,
            held: frame.held,
            wreck: frame.wreck,
            immobile: frame.immobile,
            sight_m: u64::from(frame.sight_cm / 100),
            ecm: frame.ecm,
            receiver: frame.receiver,
            heat: frame.heat,
            sections: frame.sections,
            lock,
            weapons: frame.weapons,
            order: frame.order,
            t_ms: frame.t_ms,
            notices,
            contacts,
        })
    }

    /// **The think arm** (17b cut M1a.5; doc 17 §6, D-UM-20, D-UM-9): one
    /// [`DueKind::MindThink`], at its lattice millisecond `t_ms`, with every
    /// other kind due at that millisecond already applied.
    ///
    /// In order: nothing after the match ended; nothing for a unit that is
    /// gone, a wreck or not a mind's, or for a stale `think_seq` — and none of
    /// those reschedules (an adoption schedules its own); the view
    /// ([`Arena::mind_view`]); the three epochs recorded; the runner called
    /// synchronously; `think_seq` advanced. Then **D-UM-9, atomic**: more than
    /// [`MIND_INTENTS_MAX`] intents lands nothing, keeps the old memory, puts
    /// back what the view drained and settled (the notices in front of the
    /// buffer, oldest dropped past [`MIND_NOTICES`]; the range memory as it
    /// was), clears `pending` (counted `mind_overflows`; 17b cut M3 journals
    /// the fault) and schedules **no** landing, because there is nothing to land;
    /// otherwise the [`Landing`] is put on the scratch, `pending` set, and a
    /// [`DueKind::MindLand`] scheduled one substep later. Either way the next
    /// think is scheduled [`STOCK_THINK_MS`] after this one's own millisecond.
    pub(crate) fn mind_think(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        t_ms: i64,
        counters: &mut ReflexCounters,
    ) {
        if self.ended.is_some()
            || !self
                .unit(unit)
                .is_some_and(|state| state.controller == Controller::Mind && !state.limits.wreck)
            || !self
                .minds
                .get(&unit)
                .is_some_and(|state| state.think_seq == think_seq)
        {
            return;
        }
        // D-UM-9: the two writes `mind_view` makes — the range memory settled
        // and the notice buffer drained — are undone if the think faults, so the
        // range memory as it stood is kept to put back.
        let ranges = self
            .minds
            .get(&unit)
            .map(|state| state.ranges.clone())
            .unwrap_or_default();
        let Some(view) = self.mind_view(unit) else {
            return;
        };
        let epochs = Channel::ALL.map(|channel| (channel, self.epochs.current(unit, channel)));
        counters.thinks = counters.thinks.saturating_add(1);
        let runner = Arc::clone(&self.mind_runner);
        let Some(state) = self.minds.get(&unit) else {
            return;
        };
        let result = runner.think(&ThinkJob {
            unit,
            think_seq,
            t_ms,
            view: &view,
            memory: &state.memory,
            map: &self.definition,
        });
        let land_at = t_ms.saturating_add(self.substep_len_ms());
        let Some(state) = self.minds.get_mut(&unit) else {
            return;
        };
        state.think_seq = think_seq.saturating_add(1);
        if result.intents.len() > MIND_INTENTS_MAX {
            // D-UM-9: a fault lands nothing — not a prefix — and the memory the
            // last landed think left is the memory the next think is handed.
            // The think is atomic for the mind's own row too: what the view
            // drained goes back in front of the buffer and the range memory is
            // the one it was, so the notices the faulted think was handed —
            // its own refusals among them (D-UM-8, *a refusal is never
            // silent*) — reach the next think, whose memory never saw them.
            counters.mind_overflows = counters.mind_overflows.saturating_add(1);
            state.pending = None;
            state.ranges = ranges;
            let mut notices = view.notices;
            notices.append(&mut state.notices);
            let excess = notices.len().saturating_sub(MIND_NOTICES);
            notices.drain(..excess);
            state.notices = notices;
            self.mind_landings.remove(&unit);
        } else {
            state.pending = Some(PendingThink { think_seq, t_ms });
            self.mind_landings.insert(
                unit,
                Landing {
                    think_seq,
                    epochs,
                    intents: result.intents,
                    memory: result.memory,
                },
            );
            self.due
                .schedule(land_at, DueKind::MindLand { unit, think_seq });
        }
        self.due.schedule(
            t_ms.saturating_add(STOCK_THINK_MS),
            DueKind::MindThink {
                unit,
                think_seq: think_seq.saturating_add(1),
            },
        );
    }

    /// **The landing arm** (17b cut M1a.5; doc 17 §6, D-UM-21, D-UM-13): one
    /// [`DueKind::MindLand`], one substep after its think, at `t_ms`.
    ///
    /// In order:
    ///
    /// 1. the scratch for this think is taken. **None** — a match resumed from
    ///    a snapshot, which never carries the scratch — clears the hull's
    ///    `pending` row if it names this think and lands nothing: the residual
    ///    17b cut M3's recompute closes;
    /// 2. nothing after the match ended, nor for a unit gone, a wreck or no
    ///    longer a mind's (a reclaimed seat: `Arena::set_controller` took the
    ///    scratch already);
    /// 3. the memory is written and `pending` cleared;
    /// 4. a standing order whose deadline has come is cleared, with one
    ///    `order_expired` line journalled by the arena and the squad marked for
    ///    its next signal, as the reflex controller's expiry does;
    /// 5. the intents go through `Arena::apply_machine_intents` under
    ///    [`IntentSource::Mind`] at the epochs the think recorded — a channel
    ///    raised since refuses its stale thoughts `Epoch` with no code here —
    ///    and every refusal is appended to the notice buffer as a
    ///    [`FrameNotice::Refused`] in `stage_refusal`'s shape, with no
    ///    `phase_two_tells` filter: *a refusal is never silent* (D-UM-8).
    pub(crate) fn mind_land(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        t_ms: i64,
        counters: &mut ReflexCounters,
    ) {
        let landing = if self
            .mind_landings
            .get(&unit)
            .is_some_and(|landing| landing.think_seq == think_seq)
        {
            self.mind_landings.remove(&unit)
        } else {
            None
        };
        let Some(landing) = landing else {
            if let Some(state) = self.minds.get_mut(&unit)
                && state
                    .pending
                    .is_some_and(|pending| pending.think_seq == think_seq)
            {
                state.pending = None;
            }
            return;
        };
        if self.ended.is_some()
            || !self
                .unit(unit)
                .is_some_and(|state| state.controller == Controller::Mind && !state.limits.wreck)
        {
            return;
        }
        let Landing {
            epochs,
            intents,
            memory,
            ..
        } = landing;
        let Some(state) = self.minds.get_mut(&unit) else {
            return;
        };
        state.memory = memory;
        state.pending = None;
        self.expire_mind_order(unit, t_ms, counters);
        let epoch_of = |channel: Channel| {
            epochs
                .iter()
                .find(|(on, _)| *on == channel)
                .map_or(0, |(_, epoch)| *epoch)
        };
        let landed = self.apply_machine_intents(
            unit,
            &intents,
            t_ms,
            &epoch_of,
            IntentSource::Mind,
            counters,
        );
        for (at, outcome) in landed.into_iter().enumerate() {
            let Landed::Refused(refusal) = outcome else {
                continue;
            };
            let Some(kind) = intents.get(at) else {
                continue;
            };
            let weapon = match kind {
                IntentKind::Fire { weapon } => Some(weapon.clone()),
                _ => None,
            };
            let notice = self.refusal_notice(unit, kind.as_str(), refusal, weapon, t_ms);
            if let Some(state) = self.minds.get_mut(&unit) {
                state.push_notice(notice);
            }
        }
    }

    /// **A mind's standing order that has run out** (17b cut M1a.5; D-UM-15):
    /// cleared at the landing whose millisecond reaches its deadline, with the
    /// reflex controller's own expiry — one `order_expired` line under the
    /// substep's lazy hash, `orders_expired` counted, the squad marked
    /// (`Arena::note_order_expired`, the squad parsed back from the order's
    /// rendered word by `SquadId::parse`).
    fn expire_mind_order(&mut self, unit: ObjId, t_ms: i64, counters: &mut ReflexCounters) {
        let Some(order) = self
            .minds
            .get(&unit)
            .and_then(|state| state.order.as_ref())
            .filter(|order| order.expires_ms <= t_ms)
            .cloned()
        else {
            return;
        };
        let hash = self.event_hash();
        if let Some(state) = self.minds.get_mut(&unit) {
            state.order = None;
        }
        counters.orders_expired = counters.orders_expired.saturating_add(1);
        let _ = self.ledger.append(
            t_ms,
            EventKind::OrderExpired {
                unit,
                squad: order.squad.clone(),
                objective: SmolStr::new_static(order.objective.as_str()),
            },
            Vec::new(),
            hash,
        );
        if let Some(squad) = crate::deliberation::squad::SquadId::parse(&order.squad) {
            self.note_order_expired(squad);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::test_support::{arena_of, pose, tank};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    /// One mind (red) and one pilot (blue) on the eight-cell window.
    fn minded() -> Arena {
        let mut red = tank(RED, Team::Red, pose(1_500, 4_500));
        red.controller = Controller::Mind;
        arena_of(vec![red, tank(BLUE, Team::Blue, pose(6_500, 4_500))])
    }

    /// **`NoMind` thinks nothing** and hands the memory back as it came.
    #[test]
    fn m1a5_no_mind_answers_nothing_and_echoes_the_memory() {
        let mut arena = minded();
        let Some(view) = arena.mind_view(RED) else {
            panic!("a mind has a view");
        };
        let result = NoMind.think(&ThinkJob {
            unit: RED,
            think_seq: 0,
            t_ms: 0,
            view: &view,
            memory: &[1, 2, 3],
            map: &arena.definition,
        });
        assert_eq!(
            result,
            ThinkResult {
                intents: Vec::new(),
                memory: vec![1, 2, 3]
            }
        );
        assert!(
            format!("{:?}", arena.mind_runner).contains("NoMind"),
            "Arena::new opens with the default runner"
        );
    }

    /// **The check-out schedules one think per mind** on its grid, and lays a
    /// row down; a pilot's hull gets neither.
    #[test]
    fn m1a5_the_check_out_schedules_one_think_per_mind() {
        let arena = minded();
        let thinks: Vec<(i64, DueKind)> = arena
            .due
            .iter()
            .filter(|event| event.kind.is_mind())
            .map(|event| (event.t_ms, event.kind.clone()))
            .collect();
        assert_eq!(
            thinks,
            vec![(
                arena.grid_at(RED, STOCK_THINK_MS),
                DueKind::MindThink {
                    unit: RED,
                    think_seq: 0
                }
            )]
        );
        assert_eq!(arena.minds.keys().copied().collect::<Vec<_>>(), vec![RED]);
    }

    /// **The buffer is bounded**: seventeen notices keep the newest sixteen.
    #[test]
    fn m1a5_the_buffer_drops_the_oldest_past_sixteen() {
        let mut state = MindState::default();
        for radius_m in 0..17_u32 {
            state.push_notice(FrameNotice::Emcon { on: true, radius_m });
        }
        assert_eq!(state.notices.len(), MIND_NOTICES);
        assert_eq!(
            state.notices.first(),
            Some(&FrameNotice::Emcon {
                on: true,
                radius_m: 1
            })
        );
    }

    /// **The hash probe** (`crate::arena::hash`, item 3c): every field of a
    /// mind row moves the state hash on its own — `think_seq`, `pending` (its
    /// presence, its seq and its millisecond), the memory (its bytes at one
    /// length), the notices (one more, and two `Miss`es that differ only in
    /// their reason), the range memory (an id and a figure), the order (its
    /// presence, each objective's payload, `hold_fire`, the band, the deadline
    /// and the squad) and `strikes`.
    /// One change to a mind row.
    type Probe = Box<dyn Fn(&mut MindState)>;

    #[test]
    fn m1a5_every_mind_field_moves_the_state_hash() {
        use crate::arena::perception::MindObjective;
        use crate::deliberation::order::RangeBand;
        let base = minded();
        let before = crate::arena::hash::state_hash(&base);
        let order = FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Engage {
                target: Some(ContactId(7)),
            },
            hold_fire: false,
            max_range_band: None,
            expires_ms: 30_000,
        };
        let miss = |reason: &str| FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new(reason),
        };
        let probes: Vec<(&str, Probe)> = vec![
            ("think_seq", Box::new(|s| s.think_seq = 1)),
            (
                "pending",
                Box::new(|s| {
                    s.pending = Some(PendingThink {
                        think_seq: 0,
                        t_ms: 0,
                    });
                }),
            ),
            ("memory", Box::new(|s| s.memory = vec![1])),
            (
                "notices",
                Box::new(move |s| s.notices = vec![miss("moved")]),
            ),
            (
                "ranges",
                Box::new(|s| {
                    s.ranges.insert(ContactId(7), 450);
                }),
            ),
            ("order", {
                let order = order.clone();
                Box::new(move |s| s.order = Some(order.clone()))
            }),
            ("strikes", Box::new(|s| s.strikes = 1)),
        ];
        let mut seen = vec![before];
        for (field, probe) in &probes {
            let mut arena = base.clone();
            if let Some(state) = arena.minds.get_mut(&RED) {
                probe(state);
            }
            let after = crate::arena::hash::state_hash(&arena);
            assert!(!seen.contains(&after), "`{field}` did not move the hash");
            seen.push(after);
        }
        // Within a field: each part of it is folded, not only its presence.
        let with = |change: &dyn Fn(&mut MindState)| {
            let mut arena = base.clone();
            if let Some(state) = arena.minds.get_mut(&RED) {
                change(state);
            }
            crate::arena::hash::state_hash(&arena)
        };
        let pairs: Vec<(&str, u64, u64)> = vec![
            (
                "pending.t_ms",
                with(&|s| {
                    s.pending = Some(PendingThink {
                        think_seq: 0,
                        t_ms: 0,
                    });
                }),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        think_seq: 0,
                        t_ms: 250,
                    });
                }),
            ),
            (
                "pending.think_seq",
                with(&|s| {
                    s.pending = Some(PendingThink {
                        think_seq: 0,
                        t_ms: 0,
                    });
                }),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        think_seq: 1,
                        t_ms: 0,
                    });
                }),
            ),
            (
                "memory bytes",
                with(&|s| s.memory = vec![1]),
                with(&|s| s.memory = vec![2]),
            ),
            (
                "a miss's reason",
                with(&|s| s.notices = vec![miss("moved")]),
                with(&|s| s.notices = vec![miss("cover")]),
            ),
            (
                "one notice more",
                with(&|s| s.notices = vec![miss("moved")]),
                with(&|s| s.notices = vec![miss("moved"), miss("moved")]),
            ),
            (
                "a range figure",
                with(&|s| {
                    s.ranges.insert(ContactId(7), 450);
                }),
                with(&|s| {
                    s.ranges.insert(ContactId(7), 500);
                }),
            ),
            (
                "a range id",
                with(&|s| {
                    s.ranges.insert(ContactId(7), 450);
                }),
                with(&|s| {
                    s.ranges.insert(ContactId(8), 450);
                }),
            ),
        ];
        for (part, a, b) in pairs {
            assert_ne!(a, b, "{part} is not folded");
        }
        let ordered = |change: &dyn Fn(&mut FrameOrder)| {
            let mut order = order.clone();
            change(&mut order);
            with(&move |s| s.order = Some(order.clone()))
        };
        let stock = ordered(&|_| {});
        for (part, change) in [
            (
                "the engage target",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Engage {
                        target: Some(ContactId(8)),
                    };
                }) as Box<dyn Fn(&mut FrameOrder)>,
            ),
            (
                "no target",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Engage { target: None };
                }),
            ),
            (
                "an advance",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Advance { to: (3, 4) };
                }),
            ),
            (
                "a screen",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Screen {
                        facing_mdeg: 90_000,
                    };
                }),
            ),
            (
                "a support",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Support {
                        of: Some(ContactId(7)),
                    };
                }),
            ),
            (
                "hold_fire",
                Box::new(|o: &mut FrameOrder| o.hold_fire = true),
            ),
            (
                "the band",
                Box::new(|o: &mut FrameOrder| o.max_range_band = Some(RangeBand::Close)),
            ),
            (
                "the deadline",
                Box::new(|o: &mut FrameOrder| o.expires_ms = 31_000),
            ),
            (
                "the squad",
                Box::new(|o: &mut FrameOrder| o.squad = SmolStr::new("red-2")),
            ),
        ] {
            assert_ne!(ordered(&change), stock, "{part} is not folded");
        }
        let advance =
            |to: (u16, u16)| ordered(&move |o| o.objective = MindObjective::Advance { to });
        assert_ne!(advance((3, 4)), advance((4, 3)), "an advance's cell");
        let screen = |facing_mdeg: u32| {
            ordered(&move |o| o.objective = MindObjective::Screen { facing_mdeg })
        };
        assert_ne!(screen(90_000), screen(91_000), "a screen's facing");
        let support =
            |of: Option<ContactId>| ordered(&move |o| o.objective = MindObjective::Support { of });
        assert_ne!(
            support(Some(ContactId(7))),
            support(None),
            "a support's charge"
        );
    }

    /// **`stage_notice`'s mind branch** (D-UM-8): a notice staged on a hull a
    /// mind flies reaches its buffer as well as the platform's pulse queue; a
    /// pilot's hull's buffer does not exist, and nothing is written for it.
    #[test]
    fn m1a5_a_staged_notice_reaches_the_minds_buffer() {
        let mut arena = minded();
        arena.stage_notice(RED, FrameNotice::PowerOnline);
        arena.stage_notice(BLUE, FrameNotice::PowerOnline);
        assert_eq!(
            arena.minds.get(&RED).map(|state| state.notices.clone()),
            Some(vec![FrameNotice::PowerOnline])
        );
        assert_eq!(
            arena.pending_notices.get(&RED),
            Some(&vec![FrameNotice::PowerOnline]),
            "the pulse queue still gets it"
        );
        assert!(!arena.minds.contains_key(&BLUE));
        let Some(view) = arena.mind_view(RED) else {
            panic!("a mind has a view");
        };
        assert_eq!(view.notices, vec![FrameNotice::PowerOnline]);
        assert!(
            arena
                .minds
                .get(&RED)
                .is_some_and(|state| state.notices.is_empty()),
            "drained into the view"
        );
    }

    /// **The wide window** the view tests field on: 56 by 56 cells of ground,
    /// the test window's zones, spots and graph — room for a contact 500 m off,
    /// ten steps of `CONTACT_RANGE_STEP_M`, where the eight-cell window rounds
    /// every plot to the one figure of 50 m.
    fn wide(units: Vec<crate::arena::state::CustodyUnit>) -> Arena {
        use crate::arena::state::{ArenaTerrain, CustodyOut, MatchMode};
        use crate::arena::test_support::{definition, ground, resolution};
        use crate::config::{MatchConfig, SensorsConfig};
        Arena::new(
            CustodyOut {
                definition: ArenaDefinition {
                    terrain: ArenaTerrain::uniform(56, 56, 1_000, ground())
                        .unwrap_or_else(|error| panic!("the wide window: {error:?}")),
                    ..definition()
                },
                units,
                pilots: Vec::new(),
                resolution: resolution(),
                seed: 2_350,
                mode: MatchMode::Skirmish,
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap_or_else(|error| panic!("the wide arena: {error:?}"))
    }

    /// The row id the view fixtures hold contact `k` under: descending with
    /// the range, so the nearest row has the **highest** id and the frame's own
    /// row order (by id) is the reverse of nearest-first; and every id written
    /// with a letter in it, so a reader that took the keyframe's sixteen
    /// figures for decimal would read nothing.
    fn view_row_id(k: u16) -> ContactId {
        ContactId(0xAF0 - 0x10 * u64::from(k))
    }

    /// **The view is the keyframe** (17b cut M1a.5's observer-rule assertion
    /// for the new surface; doc 17 §3, D-UM-4 read strictly). A mind flying the
    /// laser tank — sections with facets, heat, a pod and a receiver — with a
    /// held standing order holds fourteen contacts, seven of each side, a few
    /// of them unclassified, one of them read (a held condition and the fatal
    /// words beside it), and a lock; its view is compared, field by field,
    /// with the keyframe `status_frame_held` writes off the same
    /// `Arena::frame_for` against the same range memory.
    ///
    /// The contacts sit at whole steps of 50 m from 50 to 500 m, two pairs and
    /// a third pair tied — **the third at the twelfth and thirteenth rows**, so
    /// the truncation is decided by the `ContactId` tie-break — and their ids
    /// fall as their range grows ([`view_row_id`]), so the keyframe's own row
    /// order is the reverse of nearest-first: a view that took the frame's
    /// first twelve rows, or sorted by id alone, would hold the farthest. The
    /// expected side sorts the keyframe's rows by `(range_m, id)` with the id
    /// read back as the sixteen hex figures it is written as.
    ///
    /// The own fields are the keyframe's keys, `ecm`, `receiver`, `heat`,
    /// `order` and every section's keys included; each contact is at the
    /// keyframe's resolution with its `aspect`, `odds`, `reads_dead`,
    /// `condition` and `fatal` the frame's. The view names no object but its
    /// own number, and a hidden unit's live move changes nothing in it until a
    /// pass.
    #[test]
    fn m1a5_the_view_is_the_keyframe() {
        use crate::arena::condition::{Condition, ConditionBand, Reading};
        use crate::arena::perception::{Jammer, MindObjective};
        use crate::arena::state::{SeenPose, ShadowContact};
        use crate::arena::test_support::{laser_tank, line_tank};
        use crate::deliberation::order::RangeBand;
        use crate::match_::cockpit::status_frame_held;
        // (dx, dy) from the mind in centimetres, nearest first: 50, 100, 100,
        // 150, 200, 200, 250, 300, 300, 350, 400, 450, 450 and 500 m.
        let offsets: [(i64, i64); 14] = [
            (0, 5_000),
            (0, 10_000),
            (10_000, 0),
            (0, 15_000),
            (0, 20_000),
            (20_000, 0),
            (15_000, 20_000),
            (0, 30_000),
            (30_000, 0),
            (0, 35_000),
            (0, 40_000),
            (0, 45_000),
            (45_000, 0),
            (0, 50_000),
        ];
        let (x0, y0) = (1_500_i64, 1_500_i64);
        let mut red = laser_tank(RED, Team::Red, pose(x0, y0));
        red.controller = Controller::Mind;
        let mut units = vec![red];
        let mut others: Vec<ObjId> = Vec::new();
        for (k, (dx, dy)) in (0_u16..).zip(offsets) {
            let id = ObjId(10 + u32::from(k));
            let team = if k % 2 == 0 { Team::Blue } else { Team::Red };
            units.push(line_tank(id, team, pose(x0 + dx, y0 + dy)));
            others.push(id);
        }
        let mut arena = wide(units);
        for (k, id) in (0_u16..).zip(&others) {
            let Some(target) = arena.unit(*id) else {
                panic!("a fielded unit");
            };
            let class = if k % 5 == 4 {
                Classification::Unknown
            } else {
                Classification::Classified { team: target.team }
            };
            let plot = SeenPose::of(target);
            arena.contacts.insert(
                RED,
                view_row_id(k),
                ShadowContact::sighted(*id, class, 0, plot),
            );
        }
        // Row 2 (100 m, a classified blue) read with its hull destroyed.
        if let Some(held) = arena.contacts.get_mut(RED, view_row_id(2)) {
            held.seen_condition = Some(Reading {
                at_ms: 0,
                condition: Condition::default().with_band(1, ConditionBand::Destroyed),
            });
        }
        let engaged = view_row_id(3);
        if let Some(state) = arena.unit_mut(RED) {
            state.power = Power::Online;
            state.lock = Some(view_row_id(0));
            state.jammer = Some(Jammer {
                radius_cm: 10_000,
                attenuation_cost: 25,
                signature_permille: 150,
            });
            state.receiver = Some(Receiver {
                budget: 30,
                budget_max: 40,
                reach_permille: 750,
            });
        }
        let order = FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Engage {
                target: Some(engaged),
            },
            hold_fire: true,
            max_range_band: Some(RangeBand::Close),
            expires_ms: 30_000,
        };
        if let Some(state) = arena.minds.get_mut(&RED) {
            state.order = Some(order.clone());
        }
        let ranges = arena
            .minds
            .get(&RED)
            .map(|state| state.ranges.clone())
            .unwrap_or_default();
        let Some(frame) = arena.frame_for(RED, arena.clock.tempo()) else {
            panic!("a frame");
        };
        let keyframe = status_frame_held(crate::arena_task::MatchId(1), &frame, &ranges);
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        let key = |name: &str| keyframe.get(name).cloned().unwrap_or_default();

        // The own fields.
        assert_eq!(key("unit"), serde_json::json!(view.unit));
        assert_eq!(
            key("callsign"),
            serde_json::json!(view.callsign.to_string())
        );
        assert_eq!(
            key("cell"),
            serde_json::json!(view.cell.map(|(x, y)| [x, y]))
        );
        assert_eq!(key("x_cm"), serde_json::json!(view.x_cm));
        assert_eq!(key("y_cm"), serde_json::json!(view.y_cm));
        assert_eq!(key("z_cm"), serde_json::json!(view.z_cm));
        assert_eq!(key("heading_mdeg"), serde_json::json!(view.heading_mdeg));
        assert_eq!(key("speed_cms"), serde_json::json!(view.speed_cms));
        assert_eq!(key("power"), serde_json::json!(view.power.as_str()));
        assert_eq!(key("blocked"), serde_json::json!(view.held));
        assert_eq!(key("wreck"), serde_json::json!(view.wreck));
        assert_eq!(key("immobile"), serde_json::json!(view.immobile));
        assert_eq!(key("sight_m"), serde_json::json!(view.sight_m));
        assert_eq!(key("t_ms"), serde_json::json!(view.t_ms));
        // The pod, the receiver and the heat: present, and the keyframe's.
        assert_eq!(view.ecm, Some(true), "the fixture's pod, switched on");
        assert_eq!(
            key("ecm"),
            serde_json::json!(view.ecm.map(|on| if on { "on" } else { "off" }))
        );
        let Some(receiver) = view.receiver else {
            panic!("the fixture's receiver");
        };
        assert_eq!(
            key("receiver"),
            serde_json::json!({
                "budget": receiver.budget,
                "budget_max": receiver.budget_max,
                "reach_percent": receiver.reach_permille / 10,
            })
        );
        let Some(heat) = view.heat else {
            panic!("the laser tank carries heat");
        };
        assert_eq!(
            key("heat"),
            serde_json::json!({
                "heat": heat.heat,
                "max": heat.max,
                "overheated": heat.overheated(),
            })
        );
        // The standing order: the mind's own, and the keyframe's object of it
        // — the target the designator of the mind's own row for it.
        assert_eq!(view.order, Some(order.clone()), "the mind's held order");
        let target = view
            .contacts
            .iter()
            .find(|contact| contact.id == engaged)
            .map(|contact| contact.designator.to_string());
        assert!(target.is_some(), "the engaged row is in the view");
        assert_eq!(
            key("order"),
            serde_json::json!({
                "squad": "red-1",
                "objective": order.objective.as_str(),
                "target": target,
                "to": null,
                "facing_mdeg": null,
                "of": null,
                "hold_fire": true,
                "max_range_band": "close",
                "expires_in_ms": order.expires_ms - view.t_ms,
            })
        );
        assert_eq!(
            key("lock"),
            serde_json::json!(view.lock.map(|id| id.to_string()))
        );
        assert_eq!(view.lock, Some(view_row_id(0)), "the fixture locks");
        // Every section, key by key.
        let sections = key("sections").as_array().cloned().unwrap_or_default();
        assert_eq!(sections.len(), view.sections.len());
        assert!(view.sections.len() > 1, "the laser tank's five sections");
        for (row, section) in sections.iter().zip(&view.sections) {
            assert_eq!(
                row,
                &serde_json::json!({
                    "id": section.id.as_str(),
                    "condition": section.condition.as_str(),
                    "internals": section.internals.map(|(current, _)| current),
                    "internals_max": section.internals.map(|(_, max)| max),
                    "facets": section.facets.iter().map(|facet| serde_json::json!({
                        "id": facet.id.as_str(),
                        "armor": facet.armor,
                        "armor_max": facet.armor_max,
                    })).collect::<Vec<serde_json::Value>>(),
                }),
                "section {}",
                section.id
            );
        }
        let weapons = key("weapons");
        let rows = weapons.as_array().cloned().unwrap_or_default();
        assert_eq!(rows.len(), view.weapons.len());
        assert!(!view.weapons.is_empty(), "the laser tank is armed");
        for (row, weapon) in rows.iter().zip(&view.weapons) {
            assert_eq!(row.get("id"), Some(&serde_json::json!(weapon.id.as_str())));
            assert_eq!(
                row.get("bands_cm"),
                Some(&serde_json::json!(weapon.bands_cm))
            );
            assert_eq!(row.get("ready"), Some(&serde_json::json!(weapon.ready)));
            assert_eq!(row.get("rounds"), Some(&serde_json::json!(weapon.rounds)));
        }

        // The contacts: the keyframe's fourteen, nearest first, the first twelve.
        let mut rows = key("contacts").as_array().cloned().unwrap_or_default();
        assert_eq!(rows.len(), 14, "the keyframe carries every held contact");
        let range = |row: &serde_json::Value| {
            row.get("range_m")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(u64::MAX)
        };
        // `ContactId`'s `Display` is sixteen hex figures.
        let id = |row: &serde_json::Value| {
            let text = row
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| panic!("a row id: {row}"));
            u64::from_str_radix(text, 16).unwrap_or_else(|_| panic!("a hex row id: {text}"))
        };
        assert_eq!(
            rows.iter().map(id).collect::<Vec<_>>(),
            (0_u16..14)
                .rev()
                .map(|k| view_row_id(k).0)
                .collect::<Vec<_>>(),
            "the precondition: the keyframe's rows come farthest first"
        );
        rows.sort_by_key(|row| (range(row), id(row)));
        assert_eq!(
            rows.iter().map(range).collect::<Vec<_>>(),
            vec![
                50, 100, 100, 150, 200, 200, 250, 300, 300, 350, 400, 450, 450, 500
            ],
            "the precondition: whole steps, a tie at the twelfth and thirteenth rows"
        );
        rows.truncate(STOCK_CONTACT_ROWS);
        assert_eq!(view.contacts.len(), STOCK_CONTACT_ROWS);
        // The tie-break kept the lower id of the pair at 450 m (row 12, due
        // east), dropped row 11 (due north) and the farthest.
        let held: Vec<ContactId> = view.contacts.iter().map(|contact| contact.id).collect();
        assert!(held.contains(&view_row_id(12)), "{held:?}");
        assert!(!held.contains(&view_row_id(11)), "{held:?}");
        assert!(!held.contains(&view_row_id(13)), "{held:?}");
        assert!(
            view.contacts.windows(2).all(|pair| match pair {
                [a, b] => (a.range_m, a.id) < (b.range_m, b.id),
                _ => true,
            }),
            "nearest first, then by id"
        );
        let mut read = 0_usize;
        for (row, contact) in rows.iter().zip(&view.contacts) {
            let field = |name: &str| row.get(name).cloned().unwrap_or_default();
            assert_eq!(field("id"), serde_json::json!(contact.id.to_string()));
            assert_eq!(
                field("designator"),
                serde_json::json!(contact.designator.to_string())
            );
            assert_eq!(
                field("alias"),
                serde_json::json!(
                    contact
                        .alias
                        .map(|n| Designator::Provisional(n).to_string())
                )
            );
            assert_eq!(
                field("class"),
                serde_json::json!(crate::arena::view::class_word(contact.class))
            );
            assert_eq!(field("bearing_deg"), serde_json::json!(contact.bearing_deg));
            assert_eq!(
                field("relative_deg"),
                serde_json::json!(contact.relative_deg)
            );
            assert_eq!(field("band"), serde_json::json!(contact.band.as_str()));
            assert_eq!(field("range_m"), serde_json::json!(contact.range_m));
            assert_eq!(field("age_ms"), serde_json::json!(contact.age_ms));
            assert_eq!(
                field("cell"),
                serde_json::json!(contact.cell.map(|(x, y)| [x, y]))
            );
            assert_eq!(field("friendly"), serde_json::json!(contact.friendly));
            assert_eq!(
                field("condition"),
                serde_json::json!(contact.condition.map(|reading| serde_json::json!({
                    "table": reading.table,
                    "bands": reading.bands.letters(),
                    "age_ms": reading.age_ms,
                })))
            );
            assert_eq!(field("aspect"), serde_json::json!(contact.aspect.as_str()));
            assert_eq!(field("fatal"), serde_json::json!(contact.fatal));
            assert_eq!(contact.fatal.is_some(), contact.condition.is_some());
            if contact.condition.is_some() {
                read += 1;
                assert_eq!(contact.id, view_row_id(2), "the one read row");
                assert_eq!(contact.reads_dead, Some(true), "its hull read destroyed");
            }
            assert_eq!(
                field("reads_dead"),
                serde_json::json!(contact.reads_dead.unwrap_or(false))
            );
            assert!(contact.reads_dead.is_some(), "an armed hull reads it");
            let odds: Vec<(String, u64)> = field("odds")
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|odds| {
                    (
                        odds.get("weapon")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        odds.get("p_permille")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or_default(),
                    )
                })
                .collect();
            let mine: Vec<(String, u64)> = contact
                .odds
                .iter()
                .map(|(weapon, p)| (weapon.to_string(), u64::from(*p)))
                .collect();
            assert_eq!(odds, mine, "the odds of {}", contact.id);
        }
        assert_eq!(read, 1, "the read row is in the view, with its words");
        assert!(
            view.contacts
                .iter()
                .filter(|contact| !contact.odds.is_empty())
                .count()
                >= 1,
            "the nearest classified rows carry odds"
        );
        // Nothing finer than the keyframe, and no object but its own.
        let text = format!("{view:?}");
        assert!(!text.contains("ObjId"), "the view names no object: {text}");
        let rows = format!("{:?}", view.contacts);
        assert!(
            !rows.contains("mdeg") && !rows.contains("_cm"),
            "a contact at a resolution finer than the keyframe's: {rows}"
        );

        // A hidden unit's live move changes nothing until a pass.
        let moved = others.get(3).copied().unwrap_or(ObjId(13));
        if let Some(state) = arena.unit_mut(moved) {
            state.pose.x_cm += 2_000;
            state.pose.y_cm += 2_000;
        }
        let Some(again) = arena.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(
            again.contacts, view.contacts,
            "a live pose leaked into the view"
        );
    }

    /// **The view holds its ranges as a pilot's rows do** (doc 17 §3; 16b cut
    /// B.4's `T-60` hysteresis on the mind's own memory). One contact due north
    /// of the mind: first plotted at 420 m, which rounds to 400 and settles
    /// the memory at 400; then at 428 m, which rounds to 450 with no memory and
    /// sits within five eighths of a step of the held 400 — the view keeps 400
    /// and so does the memory; then at 440 m, past the tolerance — the view
    /// reads 450 and the memory settles there. A view read against an empty
    /// memory, or one that never settled it, would read 450 at 428 m.
    #[test]
    fn m1a5_the_view_holds_its_ranges_against_the_minds_own_memory() {
        use crate::arena::state::{SeenPose, ShadowContact};
        use crate::arena::test_support::line_tank;
        use crate::arena::view::range_m_rounded;
        let mut red = crate::arena::test_support::scout(RED, Team::Red, pose(1_500, 1_500));
        red.controller = Controller::Mind;
        let mut arena = wide(vec![
            red,
            line_tank(BLUE, Team::Blue, pose(1_500, 1_500 + 42_000)),
        ]);
        let Some(target) = arena.unit(BLUE) else {
            panic!("blue");
        };
        let plot = SeenPose::of(target);
        let row = ContactId(0xB1);
        arena.contacts.insert(
            RED,
            row,
            ShadowContact::sighted(
                BLUE,
                Classification::Classified { team: Team::Blue },
                0,
                plot,
            ),
        );
        let replot = |arena: &mut Arena, north_cm: i64| {
            if let Some(held) = arena.contacts.get_mut(RED, row) {
                held.seen_at.y_cm = 1_500 + north_cm;
            }
        };
        let read = |arena: &mut Arena| {
            let Some(view) = arena.mind_view(RED) else {
                panic!("a view");
            };
            let shown = view
                .contacts
                .iter()
                .find(|contact| contact.id == row)
                .map(|contact| contact.range_m);
            let held = arena
                .minds
                .get(&RED)
                .and_then(|state| state.ranges.get(&row).copied());
            (shown, held)
        };
        assert_eq!(read(&mut arena), (Some(400), Some(400)), "420 m");
        replot(&mut arena, 42_800);
        assert_eq!(
            range_m_rounded(42_800, 50),
            450,
            "the precondition: 428 m rounds to 450 with no memory"
        );
        assert_eq!(read(&mut arena), (Some(400), Some(400)), "428 m, held");
        replot(&mut arena, 44_000);
        assert_eq!(read(&mut arena), (Some(450), Some(450)), "440 m, let go");
    }
}
