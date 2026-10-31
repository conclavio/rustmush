//! **The motor** (17b cut M1a.1; doc 17 §4 "The motor", D-UM-3) — what drives
//! one hull, whatever is deciding where it should go.
//!
//! Until this cut the reflex controller (`crate::arena::reflex`, deleted in 17b
//! cut M4.6) did two jobs in one row. **Deciding** — the state machine, the
//! utility score, the goal precedence, the clocks — stayed in that module, and is
//! the stock mind's since 17b cut M1a.6. **Driving** — the route and
//! the search that refreshes it, the seven-step follower (consumption, arrival,
//! the wander, the search, the aim, the shoulder, the orders), the open-line
//! walks, the braking speed, the remembered stops and the per-substep
//! `hold_routeless` pre-pass — is the HELM station's, and it lives here: owned
//! per unit as a [`MotorRow`] in one [`MotorTable`] on [`Arena::motors`], so a
//! pilot's `DRIVE` (17b cut M1a.4), a mind's `goto` (M1a.5, M1a.6) and (until
//! 17b cut M4.6) the reflex controller's own goals all steer one hull through
//! one follower.
//!
//! Why it stays native, which doc 17 §4 argues in full: a path search in an
//! interpreter costs a thousand times its native price, and the motor is where
//! a field's searches are bounded. Every bound moved here unchanged:
//!
//! - one search expands at most `reflex.path_max_cells` nodes
//!   ([`crate::arena::path::path_with`]);
//! - one unit searches at most once per `reflex.repath_ms`;
//! - the whole field searches at most `reflex.max_path_queries` times per
//!   `reflex.decide_ms` **window of arena time** ([`MotorTable`]'s budget,
//!   which every row shares — a unit refused by it keeps the path it has);
//! - every scan of a route reads at most [`STEER_LOOKAHEAD_CELLS`] cells of it;
//! - and the wander rule keeps a route while the machine is inside its turning
//!   circle and a cell of it has an open line.
//!
//! The reflex module's head, "The bounds", was the history of each (the module
//! is gone since 17b cut M4.6; `git log` keeps it).
//!
//! # What the motor reads (doc 17 §4: "The motor's reads need an audit in M1a")
//!
//! The ground ([`Arena::definition`]), the hull's own pose, speed, mobility, slope
//! limit and cost row, its own [`MotorRow`] — the goal, the route, the
//! remembered stops — and the doctrine block's path bounds. **It reads no
//! contact, no other unit's pose and no spatial index**: the avoid list is the
//! one thing it knows about traffic, and that is a list of cells the hull was
//! refused, never what refused it (doc 16 §3.6). `crates/tacsim/tests/motor.rs`
//! holds each kernel to that, and this file's own test scans it for the
//! contact store and the fire line.
//!
//! # Every controller, one table
//!
//! A row exists for a hull something is steering — a pilot's `DRIVE`, a mind's
//! goal (the reflex controller adopted one with its machine and released it
//! with it until 17b cut M4.6). The fields are the nine that left the reflex's
//! `ReflexUnit` — byte-identical in the state hash, which read them back from
//! here in the reflex row's own order (item 3) until 17b cut M4.6 and folds
//! every row as its own section ([`crate::arena::hash::state_hash`], item 3b).

use std::collections::BTreeMap;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::arena::index::range_between;
use crate::arena::intents::{Intent, IntentKind, IntentSource, apply_intent};
use crate::arena::motion::BlockedCause;
use crate::arena::path::{
    PathScratch, Shoulder, ShoulderVerdict, path_with, shoulder_refuses, shoulder_state,
};
use crate::arena::perception::{DriveOutcome, FrameNotice, bearing_of};
use crate::arena::slope;
use crate::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, CostRow, DueKind, HoldObjective, PartyId, Power,
    SeenPose, UnitState,
};
use crate::config::ReflexConfig;

/// How near a goal cell counts as arrived, in cells.
///
/// One, because occupancy is one unit per cell (doc 16 §3.4 step 5): a controller
/// that insisted on standing *in* its goal would sit against a teammate for ever.
pub const ARRIVE_CELLS: u16 = 1;

/// The most cells the `Withdraw` goal search reads out of an entry zone before it
/// settles for the clamped corner.
///
/// An entry zone is sixteen cells deep and a window tall (the shipped arena's is
/// 16 by 256), and this search runs once per `repath_ms` per withdrawing unit, so
/// it is bounded for the same reason everything else here is.
pub const MAX_ENTRY_SCAN: usize = 1_024;

/// **Rows between two same-side hunters' crossing points** (16b cut AI.1, the
/// programme lead's ruling on `shipped_scale` e3; 16c `B-84`, `B-46`).
///
/// `Arena::far_entry_cell` clamps the machine's own cell into the far zone, so
/// two machines of one side that crossed on one road row were handed **one**
/// cell: with the B-84 orbit gone both arrive there, and the one that arrived
/// first turns back along the road into the other. A hunter's crossing cell is
/// therefore kept this far (Chebyshev) from every hunt cell its own side
/// already holds, and when its clamped corner is not, its row is fanned in
/// steps of this many rows until one is: `2 × ARRIVE_CELLS + 1`, the least
/// spacing at which two machines' arrival squares ([`ARRIVE_CELLS`] either way,
/// Chebyshev) do not overlap, so neither counts as arrived at the other's cell.
pub const HUNT_FAN_ROWS: u16 = 2 * ARRIVE_CELLS + 1;

/// **The most cells one unit remembers being stopped at** (16b cut Q.7), the
/// **least recently stopped at** dropped (16b cut E.3, `B-43`).
///
/// Eight, because the list is a memory of *this* stop and the two or three before
/// it — a tank that is being herded round a wreck needs the wreck's cell and the
/// cells it tried instead, and nothing older than that. It is also what bounds
/// the scan [`crate::arena::path::path_with`] pays per neighbour.
pub const REFLEX_AVOID_MAX: usize = 8;

/// **How long a remembered stop lasts**, arena milliseconds (16b cut Q.7).
///
/// Entries **age**; they are not cleared when the goal changes. `Approach`
/// recomputes its goal from the contact's plot at every tick, so a
/// clear-on-goal-change would wipe the list about once a second — in exactly the
/// case the owner hit, a tank chasing a moving target past a wreck. Thirty
/// seconds is long enough to get round an obstacle and short enough that a wreck
/// that has been salvaged, or a tank that has driven off, stops being a hole in
/// the map.
///
/// **A cell a machine is still held against does not age** (16b cut M.3):
/// `Arena::refresh_held_avoid` re-stamps it at every tick while
/// [`crate::arena::state::UnitState::held`] names it, so the thirty seconds run
/// from the last tick the unit was still pushing at it.
pub const REFLEX_AVOID_TTL_MS: i64 = 30_000;

/// **Remember `cell` on a machine's avoid list at `t_ms`** (16b cut Q.7, its
/// eviction 16b cut E.3's, shared since 16b cut M.3 by the stop's own edge and
/// the tick's held refresh): a listed cell has its stamp refreshed in place, and
/// a new one takes a slot, evicting the least recently stopped at when the list
/// is full.
fn remember_avoid(row: &mut MotorRow, cell: (u16, u16), t_ms: i64) {
    match row.avoid.iter_mut().find(|(at, _)| *at == cell) {
        // A listed cell refreshes its stamp rather than taking a second slot: a
        // tank held against one wreck for ten seconds must not push the other
        // seven cells it learned out of the list.
        Some(entry) => entry.1 = t_ms,
        None => {
            // **16b cut E.3 (16c `B-43`)**: the list evicts the **least recently
            // stopped at**, not the one that happens to sit at index zero.
            // Insertion order and recency are not the same order — a refresh
            // moves an entry's stamp in place — so the old rule dropped exactly
            // the cell the unit kept being stopped at, which is the one cell it
            // must not forget. Ties go to the lowest index, so the choice is a
            // function of the list and not of its scan.
            if row.avoid.len() >= REFLEX_AVOID_MAX {
                let oldest = row
                    .avoid
                    .iter()
                    .enumerate()
                    .min_by_key(|(index, (_, at_ms))| (*at_ms, *index))
                    .map(|(index, _)| index);
                if let Some(index) = oldest {
                    row.avoid.remove(index);
                }
            }
            row.avoid.push((cell, t_ms));
        }
    }
}

/// **How far short of `engage_cm` an `Approach` aims its stand-off cell**, in
/// cells (16b cut E.1).
///
/// Three, and the number is the arrive window plus the cell quantisation.
/// [`ARRIVE_CELLS`] is one cell Chebyshev — 14 m on a diagonal at the shipped
/// 10 m cell — and the goal a search is given is a cell, whose centre is up to
/// half a cell off the point `Arena::stand_off_cell` projected. So a goal
/// placed *at* `engage_cm` leaves a unit that has arrived a few metres outside
/// the band: stopped, in `Approach`, never firing. Three cells is enough slack
/// that an arrived unit is inside `engage_cm` and little enough that the machine
/// is still holding a band rather than closing onto the target.
pub const STAND_OFF_MARGIN_CELLS: u16 = 3;

/// **Millidegrees in a radian**, rounded: `180,000 / π` (16b cut AI.1; 16c
/// `B-84`) — the scale of a machine's turning circle.
///
/// A unit turning at `turn_rate_mdeg_s` while it drives at `v` cm/s runs on a
/// circle of radius `v × MDEG_PER_RADIAN / turn_rate_mdeg_s` centimetres (doc
/// 16 §3.4 step 1's turn, step 3's advance). The shipped chassis come to 1,273
/// to 2,865 cm at speed — the line tanks 1,500 cm/s at 30,000 mdeg/s, the drone
/// 2,000 at 90,000 — against a 1,000 cm cell, which is why the follower has to
/// know it: a route cell beside a machine at speed is inside that circle
/// whenever the machine's heading is ten to twenty-five degrees off it, and a
/// machine steered straight at such a cell circles it. Integer, like every
/// other figure in the arena (doc 16 §3.1's D-16-4).
pub const MDEG_PER_RADIAN: u64 = 57_296;

/// **How many cells of its route the follower reads** (16b cut AI.1; 16c
/// `B-84`; doc 16 §6): the bound on every scan of the route the steer makes —
/// the cells it has passed, the cell it steers at, and whether it has wandered
/// off.
///
/// Eight, so a tick costs at most eight open-line walks however long the route
/// is — sixteen on a tick whose route's next cell is not a neighbour, when the
/// wander rule walks up to eight before the aim walks its own — and eight cells — 80 m on the shipped ground — is more than the widest
/// turning circle's diameter (57 m at the line tank's full speed), so a machine
/// that is turning onto its route always has a cell of it ahead and outside the
/// circle to steer at.
pub const STEER_LOOKAHEAD_CELLS: usize = 8;

// ---------------------------------------------------------------------------
// The row
// ---------------------------------------------------------------------------

/// **What drives one hull** (17b cut M1a.1; doc 17 §4 "The motor", D-UM-3): the
/// cell it is making for, the route there and how it was searched, the cells it
/// was stopped at, and the waypoint walk it falls back on.
///
/// These are the nine fields that were the reflex's `ReflexUnit`'s driving
/// half until this cut, each with its doc moved verbatim. A row is a
/// fact about the hull and the ground and never about another unit: nothing
/// here names one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotorRow {
    /// The cell it is making for.
    pub goal: Option<(u16, u16)>,
    /// The principal's requested cell, before the formation fan. Retained with
    /// a terminal outcome until another request or disengagement replaces it.
    pub requested_goal: Option<(u16, u16)>,
    /// The last request's terminal outcome. `None` while driving; the active
    /// fanned `goal` is spent when an outcome is recorded.
    pub outcome: Option<DriveOutcome>,
    /// The private formation offset assigned in an opposing entry zone. It
    /// continues through goals within that same zone so a completed hunter
    /// does not immediately merge back onto another hunter's arrival lane.
    /// A goal outside the zone, disengagement, ownership change or PLACE
    /// clears it. Neither the assignment nor its offset is projected to minds.
    pub fan: Option<GoalFan>,
    /// The route to [`MotorRow::goal`], nearest cell first; consumed as the unit
    /// crosses each one.
    pub route: Vec<(u16, u16)>,
    /// The arena millisecond of its last path search, whatever the search found —
    /// a failure costs a search, or an unreachable goal would be rediscovered
    /// every tick.
    pub searched_ms: Option<i64>,
    /// **The cell the last search started from** (16b cut AI.1; 16c `B-84`),
    /// written with [`MotorRow::searched_ms`] by the tick that spends one, and
    /// never cleared by anything short of a new search (or the reset of a
    /// `Disabled` row).
    ///
    /// It is what [`Arena::note_blocked`] asks before it records a
    /// [`MotorRow::refused`]: a route broken at its own next cell is the
    /// route a search from the same cell would return again only when it was
    /// planned from the cell the machine still stands in. Folded into the state
    /// hash (item 3b; inside the reflex row's item 3 until 17b cut M4.6),
    /// because it gates a later search.
    pub searched_from: Option<(u16, u16)>,
    /// **The cells this unit was stopped at by something standing in them**, with
    /// the arena millisecond of the stop (16b cut Q.7; doc 16 §3.4 step 5).
    ///
    /// Written by [`Arena::note_blocked`] off the `Blocked` **ledger event** —
    /// the one place the cell and the cause are both in hand — and read by the
    /// path search, which treats every listed cell as impassable. At most
    /// [`REFLEX_AVOID_MAX`] entries, the **least recently stopped at** dropped
    /// (16b cut E.3, `B-43`: evicting by insertion order dropped the cell the
    /// unit kept being stopped at, because a refresh moves a stamp in place);
    /// a new stop at a listed cell refreshes its stamp, and so does every tick
    /// while the unit is still held against that cell (16b cut M.3: the `blocked`
    /// line fires once per edge now, so re-collisions no longer refresh it); an
    /// entry older than [`REFLEX_AVOID_TTL_MS`] is dropped at the top of a tick.
    ///
    /// It is **not** occupancy and it cannot be used as one: the unit knows it
    /// was refused a cell, and never what refused it. The occupant may be a
    /// platform it has not detected (doc 16 §3.6).
    pub avoid: Vec<((u16, u16), i64)>,
    /// **A stop this machine remembers, which refuses the same search by name**
    /// (16b cut AI.1; 16c `B-84`'s acceptance line), or `None`.
    ///
    /// Written by [`Arena::note_blocked`] for a stop of any cause but
    /// [`BlockedCause::Occupied`] **at the route's own next cell**, and only when
    /// the route was planned from the cell the machine still stands in
    /// ([`MotorRow::searched_from`]) and the machine has a goal: then a search
    /// from here for that goal would return the route that was just refused. It
    /// holds `until_ms`, the stop plus [`REFLEX_AVOID_TTL_MS`]; `Arena::follow`
    /// reads it where a search would be spent and, while the machine still
    /// stands at `at`, still makes for `goal` and it is still before
    /// `until_ms`, refuses the search by name, counts it in
    /// [`MotorCounters::path_refused_unchanged`] and holds the machine.
    ///
    /// **Narrow and bounded, on purpose.** It is keyed on the exact loop 16c
    /// `B-84` measured — the same route from the same cell to the same goal —
    /// and it is dropped by every change of any input to that search: a spent
    /// search, standing in another cell, another goal, a stop against another
    /// unit ([`BlockedCause::Occupied`], which puts a cell on the avoid list), an
    /// avoid entry ageing out at the top of a tick, and `forget_route` (the
    /// reflex's `adopt` and `drop_to_doctrine` too, until 17b cut M4.6 deleted
    /// them). **Nothing can hold a machine past
    /// [`REFLEX_AVOID_TTL_MS`] from its stop, and nothing refuses a search
    /// whose inputs changed.** A void memory is dropped where it is read.
    ///
    /// **At this cut's rules it is unreachable through `Arena::step`**, and it is
    /// kept as the named guard: a route planned from `here` has a first step the
    /// search accepted (the m4 property, `crate::arena::path`'s tests), a stop
    /// that is not `Occupied` needs an edge motion refuses, the terrain is
    /// immutable for the match and the avoid list grows only by `Occupied`. So
    /// its counter is expected to read nought on every lane, and a count is a
    /// controller defect to investigate — the harness's own convention for a
    /// motion-level `too_steep` stop — never a sign the guard is working.
    /// Folded into the state hash (item 3b; inside the reflex row's item 3
    /// until 17b cut M4.6).
    pub refused: Option<Refusal>,
    /// The waypoint node `Patrol` is walking toward.
    pub waypoint: Option<u16>,
    /// The node it came from, so a patrol does not turn round at every node.
    pub from_waypoint: Option<u16>,
    /// **When a Hold goal no route reaches may be tried again** (16b cut O.1;
    /// plan items 16, 34), arena milliseconds; `None` otherwise.
    ///
    /// Set by `Arena::follow`'s `unreachable` edge when the goal that failed
    /// is the Hold spot — E.4's give-up, `B-41`'s lesson applied once more — to
    /// `t_ms + max(hunt_after_ms, repath_ms)`, and read by the reflex's
    /// `Arena::hold_goal` until 17b cut M4.6 deleted it (which declined while
    /// it was in the future, so `Patrol` fell through to the waypoint walk,
    /// and cleared it once past); read by nothing but the hash since — the
    /// stock mind keeps its own retry clock in its memory — and written by the
    /// `Fallback::Waypoint` branch no production path takes. Folded into the state hash (item 3b;
    /// beside the reflex row's other clocks in item 3 until 17b cut M4.6).
    pub hold_retry_ms: Option<i64>,
}

/// A motor's formation assignment, retained after the active goal is spent.
/// The two cells identify both the offset and the immutable entry zone that
/// owns it; no other hull or private position is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalFan {
    /// The requested cell whose entry zone owns this assignment.
    pub origin: (u16, u16),
    /// The physical cell assigned for that request.
    pub assigned: (u16, u16),
}

impl GoalFan {
    /// Keep only actual offsets. A normal goal carries no formation memory.
    pub(crate) fn between(origin: (u16, u16), assigned: (u16, u16)) -> Option<Self> {
        (origin != assigned).then_some(Self { origin, assigned })
    }

    fn shifted(self, cell: (u16, u16)) -> Option<(u16, u16)> {
        let x = i32::from(cell.0) + i32::from(self.assigned.0) - i32::from(self.origin.0);
        let y = i32::from(cell.1) + i32::from(self.assigned.1) - i32::from(self.origin.1);
        Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?))
    }
}

/// **A stop the machine remembers** (16b cut AI.1; 16c `B-84`): where it
/// stood, the cell and cause it was refused, the goal it was making for, and
/// when it may search for that goal from that cell again.
///
/// See [`MotorRow::refused`] for when one is written, what it refuses and
/// everything that drops it. (17b cut M1a.1 moved it here with the row; the
/// reflex module re-exported it under its old path until 17b cut M4.6
/// deleted the module.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
    /// The cell the machine stood in when it was stopped — and the cell its
    /// refused route was planned from.
    pub at: (u16, u16),
    /// The cell it was refused: the route's own next cell.
    pub cell: (u16, u16),
    /// Why motion refused it (never [`BlockedCause::Occupied`]).
    pub cause: BlockedCause,
    /// The goal the refused route led to.
    pub goal: (u16, u16),
    /// The arena millisecond the memory lapses at: the stop plus
    /// [`REFLEX_AVOID_TTL_MS`].
    pub until_ms: i64,
}

impl MotorRow {
    /// **Age the remembered stops at `t_ms`** (17b cut M1a.1, lifted verbatim
    /// from the top of the reflex decision; D-UM-3), answering whether any
    /// entry aged out.
    ///
    /// **16b cut Q.7**: remembered stops age out, at the top of the tick and in
    /// arena time. They are deliberately NOT cleared on a goal change —
    /// `Approach` moves its goal every tick, and clearing there would empty the
    /// list in exactly the case it exists for. **16b cut M.3**: a cell the unit
    /// is still held against was re-stamped to this tick just before
    /// (`Arena::refresh_held_avoid`), so it survives this sweep however long
    /// the unit has been pinned there.
    ///
    /// **16b cut AI.1**: an entry that ages out changes what a search from here
    /// would find, so a remembered stop ([`MotorRow::refused`]) goes with it —
    /// nothing refuses a search whose inputs changed.
    ///
    /// The reflex decision called this exactly where the sweep stood, so a
    /// machine's row aged the bytes it aged before (the two HEAD pins of 17b
    /// cut M1a.1 proved it); 17b cut M1a.4's motor step calls it for a row no
    /// decision runs, so a pilot's or a mind's stops age too — its one caller
    /// since 17b cut M4.6 deleted the reflex decision.
    pub fn age_avoid(&mut self, t_ms: i64) -> bool {
        let listed = self.avoid.len();
        self.avoid
            .retain(|(_, at_ms)| t_ms.saturating_sub(*at_ms) < REFLEX_AVOID_TTL_MS);
        if self.avoid.len() != listed {
            self.refused = None;
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// **Every motor row of a match, the path budget they share, and the A\*
/// scratch** (17b cut M1a.1; doc 17 §4) — [`Arena::motors`].
///
/// The budget's window and its spend moved here from the reflex controller's
/// memory (`ReflexMemory`, deleted in 17b cut M4.6) with the scratch: they bound the
/// searches of every hull that drives, whatever decides its goal, so they are
/// the motor's and not one controller's. The scratch is one half-megabyte
/// buffer for the whole match rather than one per unit, and is never carried
/// by a snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotorTable {
    rows: BTreeMap<ObjId, MotorRow>,
    /// Which `decide_ms` window of arena time the budget below belongs to.
    window: i64,
    /// Searches already spent in that window.
    spent: u32,
    #[serde(skip)]
    scratch: PathScratch,
}

impl MotorTable {
    /// An empty table, its budget window not yet opened — what
    /// [`Arena::new`] lays down (beside the reflex memory until 17b cut M4.6
    /// deleted it).
    #[must_use]
    pub fn new() -> MotorTable {
        MotorTable {
            rows: BTreeMap::new(),
            window: i64::MIN,
            spent: 0,
            scratch: PathScratch::default(),
        }
    }

    /// One hull's row, if something is steering it.
    #[must_use]
    pub fn row(&self, unit: ObjId) -> Option<&MotorRow> {
        self.rows.get(&unit)
    }

    /// One hull's row, to write.
    pub fn row_mut(&mut self, unit: ObjId) -> Option<&mut MotorRow> {
        self.rows.get_mut(&unit)
    }

    /// One hull's row, made empty when it has none — the door a controller
    /// that takes a hull over steers it through.
    pub fn ensure(&mut self, unit: ObjId) -> &mut MotorRow {
        self.rows.entry(unit).or_default()
    }

    /// Forget a hull's row — nothing is steering it any more.
    pub fn release(&mut self, unit: ObjId) {
        self.rows.remove(&unit);
    }

    /// Every row, in unit-id order — the map's own, so there is no iteration
    /// order to diverge on.
    pub fn rows(&self) -> impl Iterator<Item = (ObjId, &MotorRow)> {
        self.rows.iter().map(|(unit, row)| (*unit, row))
    }

    /// **Take one hull's row out of the table** for the length of a decision
    /// (remove-or-default), so the decision can hold the row and the table's
    /// budget as two disjoint `&mut`s while the arena stays `&self`.
    pub(crate) fn take(&mut self, unit: ObjId) -> MotorRow {
        self.rows.remove(&unit).unwrap_or_default()
    }

    /// Put a row [`MotorTable::take`] took back.
    pub(crate) fn put(&mut self, unit: ObjId, row: MotorRow) {
        self.rows.insert(unit, row);
    }

    /// Whether this tick may spend a path search, against the per-window budget.
    ///
    /// **17b cut M1a.1**: moved from the reflex memory with the window and the
    /// spend, and handed the doctrine block it read off that memory — the same
    /// value, since [`Arena::new`] built the memory from `arena_cfg.reflex`
    /// (until 17b cut M4.6 deleted it) and nothing writes that block
    /// afterwards. The window is keyed on `decide_ms`, as it was.
    pub(crate) fn may_search(&mut self, t_ms: i64, cfg: &ReflexConfig) -> bool {
        let window_ms = i64::from(cfg.decide_ms.max(1));
        let window = t_ms.div_euclid(window_ms);
        if window != self.window {
            self.window = window;
            self.spent = 0;
        }
        if self.spent >= cfg.max_path_queries {
            return false;
        }
        self.spent = self.spent.saturating_add(1);
        true
    }

    /// Forget a path invalidated by staff PLACE, preserving the requested
    /// and active destinations. The next tick searches from the new cell at
    /// once; a former completion no longer proves arrival there. PLACE first
    /// reactivates a terminal request, because its active goal was spent.
    pub fn forget_route(&mut self, unit: ObjId) {
        if let Some(row) = self.rows.get_mut(&unit) {
            row.route.clear();
            row.searched_ms = None;
            row.searched_from = None;
            row.refused = None;
            row.outcome = None;
            row.fan = None;
        }
    }

    /// **Forget the goal an order or a decision handed a hull** (17b cut M1a.1):
    /// the goal, the route and the remembered stop, which is what the reflex
    /// row's `drop_to_doctrine` cleared of the driving fields until this cut —
    /// its callers called this beside it until 17b cut M4.6. The
    /// search stamp, the avoid list and the waypoint walk stay, as they did. A
    /// hull with no row is left alone.
    pub fn forget_goal(&mut self, unit: ObjId) {
        if let Some(row) = self.rows.get_mut(&unit) {
            row.goal = None;
            row.requested_goal = None;
            row.outcome = None;
            row.fan = None;
            row.route.clear();
            // 16b cut AI.1: a remembered stop belongs to the goal it was making
            // for.
            row.refused = None;
        }
    }
}

// ---------------------------------------------------------------------------
// What one drive answers
// ---------------------------------------------------------------------------

/// **The orders one drive decided on** (17b cut M1a.1): the heading and the
/// speed `Arena::follow` wrote onto the reflex plan until this cut. The
/// caller copies each it finds `Some` onto its own plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MotorPlan {
    /// The heading to order, millidegrees.
    pub heading_mdeg: Option<u32>,
    /// The speed to order, cm/s.
    pub speed_cms: Option<i32>,
}

/// **What one drive came to** (17b cut M1a.1): whether the hull is within
/// [`ARRIVE_CELLS`] of its goal, and the goal no route reached, if one did not.
///
/// `unreachable` is how the decision that handed the motor its goal learns what
/// `Arena::follow` used to do to the decision's own fields on the `unreachable`
/// edge — `B-41`'s `Advance` end, Z.2's `Support` end, E.4's two give-ups —
/// which the reflex decision performed from it, in the order the edge did,
/// until 17b cut M4.6 deleted it. Since then `Arena::motor_step` reads it for
/// the `drive_no_route` notice, and a mind gives up off that notice at its
/// next think.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MotorOutcome {
    /// The hull stands within [`ARRIVE_CELLS`] of its goal (step 2).
    pub arrived: bool,
    /// The goal a search failed for, at this drive's `unreachable` edge.
    pub unreachable: Option<(u16, u16)>,
}

/// **What the motor does with a goal no route reaches** (17b cut M1a.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// **The reflex controller's rule, exactly as it was**: the failure is
    /// counted, a Hold spot sets [`MotorRow::hold_retry_ms`], the nearest
    /// waypoint (or the next along the graph) becomes the goal
    /// ([`Arena::fall_back_to_waypoint`]) and the steer carries on this tick.
    Waypoint,
    /// **17b cut M1a.4's**, for a goal a pilot or a mind named: the failure is
    /// counted, the goal and the route are dropped and the hull is ordered to
    /// stop — no waypoint is touched — and whoever named the goal is told. At
    /// arrival the goal is dropped too.
    Notice,
}

// ---------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------

/// **What the machines cost one pulse** — the motor's searches and refusals,
/// the intents and shots machines put through the pilot's door, and the minds'
/// thinks, landings and faults (17b cut M4.5: the reflex's counters of 16b cut
/// N.1, renamed and moved here so they outlive the reflex). The shape
/// [`crate::arena::perception::PerceptionCounters`] has and for the same
/// reason: the harness asserts a bound against the worst pulse of a run and
/// the report carries the totals; never hashed. The reflex's own `ticks`,
/// `ordered` and `in_state` retired with the harness's move onto minds (17b
/// cut M4.5's second commit): a mind's decisions are its
/// [`MotorCounters::thinks`] and [`MotorCounters::landings`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MotorCounters {
    /// Path searches run — what `harness.max_path_queries_per_pulse` bounds.
    pub path_queries: u32,
    /// Searches refused by the per-window budget or by `repath_ms`.
    pub path_deferred: u32,
    /// Nodes those searches expanded — **found or failed** since 16b cut AI.1
    /// (16c `B-84`): a `Budget` failure is `path_max_cells` expansions and a
    /// `NoRoute` the whole reachable region, the two most expensive searches
    /// there are, and until that cut they counted nothing here. What
    /// `harness.max_path_cells_per_pulse` bounds.
    pub path_cells: u64,
    /// Searches that found no route, ran out of budget or were off the ground —
    /// the `unreachable` edge.
    pub path_failures: u32,
    /// **Every intent put through `Arena::apply_machine_intents`, refused
    /// ones included** (counted before `apply_intent` answers): a mind's
    /// landing and the motor's heading and speed at its follow — plus
    /// `hold_routeless`'s stops (and the reflex's tick until 17b cut M4.6
    /// deleted it). Not a count of decisions: the motor alone moves it.
    ///
    /// (`traces`, the line-of-sight traces the reflex took through
    /// `Arena::reflex_has_line`, reported nowhere since 17b cut M4.5, was
    /// deleted with its one writer in 17b cut M4.6.)
    pub intents: u32,
    /// **`FIRE`s accepted**: a mind's cited `Fire` and a slot's own
    /// ([`MotorCounters::cond_fires`] ⊆ this) — and the reflex's uncited one
    /// until 17b cut M4.6.
    pub shots: u32,
    /// **Orders that ran out** (16b cut N.2), one per unit dropped to doctrine:
    /// a mind's at the landing that reaches its deadline
    /// (`Arena::expire_mind_order`) or that finds its `Support`'s charge gone
    /// (17b cut M4.6b, the owner's STOP E) — and the reflex's at its tick
    /// until 17b cut M4.6.
    pub orders_expired: u32,
    /// **Orders a pilot handed back** (16b cut Z.2; 16c Q44), one per unit a
    /// `ReleaseOrder` dropped to doctrine — counted at apply, in pulse phase 2.
    pub orders_released: u32,
    /// **Edges refused by the slope rule** (16b cut U.1; 16c `T-35`): every edge
    /// a search refused as too steep either way or by the shoulder rule
    /// ([`crate::arena::path::PathScratch::slope_refusals`]), plus every time the
    /// follower's one-cell test (`Arena::first_step_is_open`) found the cell in
    /// front open to the class and the avoid list and refused it by the same
    /// edge rule. What the harness asserts non-zero to prove the cohort's
    /// locomotion met the relief; a counter, not state, so it moves no hash.
    ///
    /// **16b cut AI.1**: a corner refused because no shoulder is open is not a
    /// slope refusal and is counted in [`MotorCounters::corner_refusals`]
    /// instead, so this figure keeps its meaning.
    pub slope_refusals: u32,
    /// **Searches refused by name** (16b cut AI.1; 16c `B-84`): ticks at which a
    /// due search was not spent because [`MotorRow::refused`] said it would
    /// return the route that had just been refused, and the machine held
    /// instead. **Expected nought on every lane**: the memory is unreachable
    /// through `Arena::step` at this cut's rules, and a count is a controller
    /// defect to investigate. A counter, never hashed.
    pub path_refused_unchanged: u32,
    /// **Diagonals refused as a corner closed on both shoulders** (16b cut
    /// AI.1; 16c `B-84`): every one a search refused
    /// ([`crate::arena::path::PathScratch::corner_refusals`]), plus every time
    /// the follower's one-cell test found the diagonal in front open to the
    /// class and the avoid list and refused it because neither of its shoulders
    /// is. A counter, never hashed.
    pub corner_refusals: u32,
    /// **Unit minds' thinks applied** (17b cut M1a.5; doc 17 §6): one per
    /// `MindThink` that called its runner. The minds lane's `thinks` (17b cut
    /// M4.2), the arena lanes' liveness assert (with `landings`) and their
    /// report-only `harness_arena_minds` line (17b cut M4.5); never hashed,
    /// and in no compared leaf of the arena report.
    pub thinks: u32,
    /// **Thinks that answered more than `MIND_INTENTS_MAX` intents** (17b cut
    /// M1a.5; D-UM-9) and so landed nothing; 17b cut M3 journals the fault.
    /// The minds lane's `overflows`; never hashed.
    pub mind_overflows: u32,
    /// **Thinks that faulted** (17b cut M2.2; D-UM-31): a runner's
    /// `ThinkResult::fault` or an overflow (each overflow is also one of
    /// [`MotorCounters::mind_overflows`]), every one landing nothing and
    /// counting a strike. The minds lane's `faults`; never hashed.
    pub mind_faults: u32,
    /// **Condition slots' own shots accepted** (17b cut M1b.1; doc 17 §5,
    /// D-UM-16): every `fired` line `by: cond`. The minds lane's `cond_fires`;
    /// a counter, never hashed.
    pub cond_fires: u32,
    /// **Slot evaluations that held for a named reason** (17b cut M1b.1): a
    /// warrant term failed with the target there. Never hashed; not reported.
    pub cond_held: u32,
    /// **Unit minds' answers landed** (17b cut M4.2; the M4 row's (ii)): one
    /// per `MindLand` that reached its apply in `Arena::mind_land` — the
    /// written answer landed, clean or refused intent by intent. A landing
    /// retracted by a fault never reaches the apply, so this and
    /// [`MotorCounters::mind_faults`] are disjoint. The minds lane's
    /// `landings`; never hashed.
    pub landings: u32,
}

impl MotorCounters {
    /// Add another pulse's counts — the harness's running total.
    pub fn absorb(&mut self, other: &MotorCounters) {
        self.path_queries = self.path_queries.saturating_add(other.path_queries);
        self.path_deferred = self.path_deferred.saturating_add(other.path_deferred);
        self.path_cells = self.path_cells.saturating_add(other.path_cells);
        self.path_failures = self.path_failures.saturating_add(other.path_failures);
        self.intents = self.intents.saturating_add(other.intents);
        self.shots = self.shots.saturating_add(other.shots);
        self.orders_expired = self.orders_expired.saturating_add(other.orders_expired);
        self.orders_released = self.orders_released.saturating_add(other.orders_released);
        self.slope_refusals = self.slope_refusals.saturating_add(other.slope_refusals);
        self.path_refused_unchanged = self
            .path_refused_unchanged
            .saturating_add(other.path_refused_unchanged);
        self.corner_refusals = self.corner_refusals.saturating_add(other.corner_refusals);
        self.thinks = self.thinks.saturating_add(other.thinks);
        self.mind_overflows = self.mind_overflows.saturating_add(other.mind_overflows);
        self.mind_faults = self.mind_faults.saturating_add(other.mind_faults);
        self.cond_fires = self.cond_fires.saturating_add(other.cond_fires);
        self.cond_held = self.cond_held.saturating_add(other.cond_held);
        self.landings = self.landings.saturating_add(other.landings);
    }
}

// ---------------------------------------------------------------------------
// Driving
// ---------------------------------------------------------------------------

impl Arena {
    /// **A routeless machine is asked every substep whether the cell in front of
    /// it is still open** (16b cut E.3; 16c `B-44`), before motion runs.
    ///
    /// 16b cut Q.7 put that question into `Arena::follow`: a machine steering at
    /// a goal with no route is steering at a *direction* nobody has checked the
    /// ground along, so the one cell it would cross first is tested with the
    /// crossing's own rule and a machine that would be refused it is ordered to
    /// stop. But `follow` runs at the decide tick — once a second of arena time —
    /// and motion runs eight times in that second at the fastest tempo, so for up
    /// to a whole tick the machine drove at a bluff, a wreck or the cell it was
    /// just stopped at with nothing between it and the refusal.
    ///
    /// So the question is asked at motion's own cadence. For every unit the
    /// controller is flying whose power is online, whose speed order is not
    /// already zero, and whose machine has a goal and an **empty route** — a route
    /// is a sequence of edges the search has already checked — the cell on the
    /// unit's **actual** `pose.heading_mdeg` (not the heading it was ordered onto;
    /// motion will cross the one the tank is pointing at) is put to
    /// `Arena::first_step_is_open` with the machine's own remembered stops.
    /// Where it answers `false` the unit is ordered `Speed { cms: 0 }` through
    /// [`apply_intent`], at its own current epoch, exactly the way
    /// `Arena::order` builds one: **the machine still does nothing a pilot could
    /// not.** The next decide tick orders a speed again as soon as the step on the
    /// heading it wants is open, which is Q.7's own rule unchanged.
    ///
    /// It draws nothing, reads the rows in unit-id order (the map's own), and
    /// collects the ids before it orders any of them, so no borrow is held across
    /// [`apply_intent`]. The four cheap tests come first and the map lookup last,
    /// because this runs once per substep over the whole field.
    ///
    /// # One cell was never enough: it looks as far as the unit needs to stop
    ///
    /// **16b cut M.2** (reopening E.3's `B-44` closure; the fix was the M.2
    /// review's). Asking only the one cell in front was a rule for a unit that
    /// can stop inside a cell, and a stock tank cannot: it needs `v² / 2·brake` —
    /// 56 m from 15 m/s at 2 m/s² — so the hold came when the seam was already
    /// inside its stopping distance and the tank was refused at the boundary
    /// anyway. E.3's own proof missed it only because its thirty-second window
    /// closed five hundred milliseconds before the first approach arrived; M.2's
    /// machine `STARTUP` at its tick moved the same approach one decide tick
    /// earlier and into the window.
    ///
    /// So a machine is held when **any** cell on its actual heading within
    /// `stopping_reach_cm`, beyond the one in front, would be refused by the same
    /// rule — the stopping
    /// distance from the fastest it can be going at the end of this substep, plus
    /// what it travels in the substep before the next pre-pass can ask again —
    /// walked by `Arena::line_is_open` along the pose's own heading (its M.2
    /// walk, `steer: false`, which was `way_ahead_is_open` until 16b cut AI.1
    /// generalised it for the follower's steer). The one-cell question is still
    /// asked first, so a unit at a standstill is exactly as free as it was —
    /// **with one change of 16b cut AI.1's**: both questions ask the search's
    /// edge rule as that cut left it, so under a slope limit a diagonal neither
    /// of whose shoulders is open is refused (`EdgeVerdict::Corner`, counted
    /// in [`MotorCounters::corner_refusals`]) where one passed before, and a
    /// shoulder on the machine's own avoid list is no shoulder. A limited
    /// routeless machine therefore holds at a corner pinched by two walls, or by
    /// a wall and a cell it was stopped at, until its search answers; an
    /// unlimited one is refused no shoulder, as before. Held early, the tank brakes on its own profile and stops short of the
    /// seam; the next decide tick orders it on only while the cell in front is
    /// open, and the pre-pass holds it again the substep it would outrun its
    /// brakes, so it closes on the seam slower and slower and never reaches it.
    /// A platform with no braking rate is asked the one cell alone: it cannot
    /// slow down, which is `braking_speed_cms`'s answer too.
    ///
    /// # Whatever is steering it (17b cut M1a.1)
    ///
    /// The pre-pass reads [`Arena::motors`] and not the reflex rows, and it no
    /// longer asks who the controller is: **a motor row exists only for a hull
    /// something is steering** (the reflex's adoption laid one down with a
    /// machine and its release took it up, until 17b cut M4.6), so a row
    /// with a goal and no route is a hull driven at a direction nobody has
    /// checked, whether a machine, a mind or a pilot's `DRIVE` named the goal. A
    /// seated pilot steering by `HEADING` and `SPEED` has no row and is never
    /// touched. Until this cut the controller test was what kept the pre-pass
    /// off a pilot's tank; at this cut's rules the two readings are one set of
    /// hulls because [`Arena::set_controller`]'s `Pilot` arm releases the row —
    /// pinned on the production path by `tests/reflex.rs`' reclaim (a `Live`
    /// helm step), `catchup.rs`' `apply_control` reclaim and this module's E.3
    /// test, each asserting the row is gone.
    pub fn hold_routeless(&mut self, counters: &mut MotorCounters) {
        // The full substep, which is what the unit travels before the next
        // pre-pass: the same length `Arena::step` cuts the pulse into, read off
        // the one method both use (16b cut U.1).
        let substep = self.substep_len_ms();
        let mut holding: Vec<ObjId> = Vec::new();
        for (id, row) in self.motors.rows() {
            if !row.route.is_empty() {
                continue;
            }
            if row.goal.is_none() {
                continue;
            }
            let Some(unit) = self.unit(id) else {
                continue;
            };
            if unit.power != Power::Online || unit.desired_speed_cms == 0 {
                continue;
            }
            let Some(here) = self.definition.terrain.cell_of(&unit.pose) else {
                continue;
            };
            // 16b cut AI.1: the plain cell list the edge rule reads, built once
            // for the machine's two questions.
            let avoid: Vec<(u16, u16)> = row.avoid.iter().map(|(cell, _)| *cell).collect();
            if self.first_step_is_open(unit, here, unit.pose.heading_mdeg, &avoid, counters)
                && self.line_is_open(
                    unit,
                    here,
                    unit.pose.heading_mdeg,
                    stopping_reach_cm(unit, substep),
                    &avoid,
                    false,
                )
            {
                continue;
            }
            holding.push(id);
        }
        for id in holding {
            let kind = IntentKind::Speed { cms: 0 };
            let channel = kind.channel_required();
            let intent = Intent {
                unit: id,
                channel,
                epoch: self.epochs.current(id, channel),
                kind,
                seq: 1,
            };
            counters.intents = counters.intents.saturating_add(1);
            // A speed is a **continuous** order: `apply_intent` writes it straight
            // onto the unit and never queues it, so there is nothing to take back
            // out of the pending queue the way `Arena::order` does for a discrete
            // action.
            let _ = apply_intent(self, &intent, IntentSource::Machine);
        }
    }

    /// **Note that `unit` was stopped at `cell`** (16b cut Q.7; doc 16 §3.4 step
    /// 4), which is the `blocked` edge read off the **event** instead of off the
    /// flag.
    ///
    /// [`crate::arena::state::UnitState::blocked`] is cleared at the top of every
    /// substep, so a tick saw a stop only when the blocking substep happened to be
    /// the last one before it — a fact about wall-clock pacing, and therefore
    /// exactly the kind of thing that must not reach hashed state. The `Blocked`
    /// *ledger* event is journalled once per stop, at its own arena timestamp,
    /// with the cell and the cause in hand; this is called from there, and it is
    /// the only way the controller learns of a stop now.
    ///
    /// **16b cut M.3** (16c `T-04`): "once per stop" is now exact — the line and
    /// this call fire on [`crate::arena::state::UnitState::held`]'s new edge and
    /// nothing else, so a unit re-colliding with one wreck substep after substep
    /// calls this once. The stamp refresh the repeats used to give a listed cell
    /// is the tick's now (`Arena::refresh_held_avoid`), for as long as the unit is
    /// still held there.
    ///
    /// **A stop against another unit** ([`BlockedCause::Occupied`]) throws the
    /// route away, remembers the cell (see [`MotorRow::avoid`]) and clears
    /// `searched_ms`, so the next tick searches at once rather than waiting out
    /// `repath_ms` against an obstacle that is not going to move — and, since 16b
    /// cut AI.1, clears [`MotorRow::refused`]: the avoid list is a new input to
    /// the search. The re-search is still under `MotorTable::may_search`'s
    /// per-window budget, so a field that all stops at once cannot spend more
    /// searches than a field that does not.
    ///
    /// **A stop by the ground** — any other cause — **no longer throws every
    /// route away** (16b cut AI.1; 16c `B-84`). Until that cut it did, and left
    /// `searched_ms` standing, so a tank stopped on a corner's shoulder searched
    /// the identical route again after `repath_ms`, drove it into the same
    /// shoulder and did so for the rest of the match. What it does now depends on
    /// where the stop was, against the cell the machine stands in (read before
    /// the row is taken; off the window, the route is cleared and nothing
    /// recorded):
    ///
    /// - **at the route's own next cell**: the route is cleared (`searched_ms`
    ///   untouched, the rule as it was), and when the route was planned from
    ///   this very cell ([`MotorRow::searched_from`]) and the machine has a
    ///   goal, the stop is remembered as a [`Refusal`] until
    ///   [`REFLEX_AVOID_TTL_MS`] from now — a search from here for that goal
    ///   would return it again. Unreachable at this cut's rules; the guard.
    /// - **beside the route** — the route's next cell is still a neighbour and
    ///   the stop was somewhere else: the unit clipped a shoulder, or its heading
    ///   lagged the order. **The route is kept** and nothing is recorded; motion
    ///   reports the cell entered, so this is where 16c `B-84`'s corner stop
    ///   lands, and the follower's shoulder step (`Arena::follow`) is what now
    ///   keeps it from happening in the first place.
    /// - **anywhere else** — no route, or a next cell that is not a neighbour:
    ///   the route is cleared as it always was, and **no memory is recorded for
    ///   a routeless stop**: the follower's one-cell test and
    ///   [`Arena::hold_routeless`] own that case.
    ///
    /// **17b cut M1a.1**: it writes the hull's [`MotorRow`], and it is gated on
    /// that row's presence and no longer on the controller — a hull something
    /// is steering learns of its stops, whatever is steering it, and a seated
    /// pilot's tank, which has no row, learns nothing (the
    /// [`Arena::hold_routeless`] argument).
    pub fn note_blocked(&mut self, unit: ObjId, cell: (u16, u16), cause: BlockedCause, t_ms: i64) {
        let Some(state) = self.unit(unit) else {
            return;
        };
        let here = self.definition.terrain.cell_of(&state.pose);
        let Some(row) = self.motors.row_mut(unit) else {
            return;
        };
        if cause == BlockedCause::Occupied {
            row.route.clear();
            row.searched_ms = None;
            row.refused = None;
            remember_avoid(row, cell, t_ms);
            return;
        }
        let Some(here) = here else {
            row.route.clear();
            return;
        };
        match row.route.first().copied() {
            Some(next) if next == cell => {
                row.route.clear();
                if row.searched_from == Some(here)
                    && let Some(goal) = row.goal
                {
                    row.refused = Some(Refusal {
                        at: here,
                        cell,
                        cause,
                        goal,
                        until_ms: t_ms.saturating_add(REFLEX_AVOID_TTL_MS),
                    });
                }
            }
            Some(next) if adjacent(here, next) => {}
            _ => row.route.clear(),
        }
    }

    /// **A machine still held on an occupied cell keeps it on its avoid list**
    /// (16b cut M.3; 16c `T-04` with `B-43`'s guarantee), run at the top of
    /// `Arena::motor_step` (and of the reflex's tick until 17b cut M4.6),
    /// before the [`REFLEX_AVOID_TTL_MS`] sweep.
    ///
    /// Until the one-edge rule, every re-collision with the wreck was a `blocked`
    /// line and a [`Arena::note_blocked`] that refreshed the cell's stamp, so a
    /// machine pinned against it never aged it out. The line now fires once per
    /// edge, and a machine cornered against a wreck for longer than the TTL would
    /// forget the one cell it is still pushing at — so the tick refreshes it off
    /// the condition instead: a steered unit whose
    /// [`crate::arena::state::UnitState::held`] names an
    /// [`BlockedCause::Occupied`] cell has that cell's entry stamped `t_ms`, or
    /// inserted under the same least-recently-stopped eviction a new stop uses.
    /// Rate-independent: the tick is a due event at its own arena millisecond, and
    /// `held` is hashed. Nothing else about the stop is re-applied — the route and
    /// `searched_ms` were the edge's to clear, once.
    ///
    /// **17b cut M1a.1**: gated on the hull's [`MotorRow`] and not on the
    /// controller, as [`Arena::note_blocked`] is; its one caller was still the
    /// reflex tick at that cut, so it refreshed exactly the rows it had. 17b
    /// cut M1a.4's `Arena::motor_step` calls it too — its one caller since
    /// 17b cut M4.6 deleted the reflex tick.
    pub(crate) fn refresh_held_avoid(&mut self, unit: ObjId, t_ms: i64) {
        let Some(held) = self.unit(unit).and_then(|state| state.held) else {
            return;
        };
        if held.cause != BlockedCause::Occupied {
            return;
        }
        if let Some(row) = self.motors.row_mut(unit) {
            remember_avoid(row, (held.x, held.y), t_ms);
        }
    }

    /// Keep the route fresh and turn it into a heading and a speed.
    ///
    /// # The steer (16b cut AI.1; 16c `B-84`; doc 16 §3.4 steps 1 to 4, §6)
    ///
    /// Until 16b cut AI.1 the follower steered at the **centre of its route's
    /// first cell** at full speed whatever its heading error, and a machine
    /// turns on a circle of radius `v × MDEG_PER_RADIAN / turn_rate` — 1,273 to
    /// 2,865 cm on the shipped chassis against a 1,000 cm cell — so a route cell
    /// beside it was inside its circle whenever its heading was ten to
    /// twenty-five degrees off: it circled the cell, drifted more than a cell
    /// off it, and 16b cut Q.7's wandered-off branch bought a full search on
    /// every tick of the orbit. It now takes seven steps, in this order:
    ///
    /// 1. **Consumption**: of the first `STEER_LOOKAHEAD_CELLS + 1` route
    ///    cells, the last that is the cell it stands in, or a neighbour it may
    ///    drive to in one step (`Arena::step_is_drivable`), marks what is
    ///    behind it; the cells before it leave the route (and it too, when it is
    ///    the cell stood in).
    /// 2. **Arrival** within [`ARRIVE_CELLS`], as before.
    /// 3. **The wander**: a route whose next cell is not a neighbour is kept
    ///    while the nearest of its first [`STEER_LOOKAHEAD_CELLS`] cells is
    ///    within `2 r + cell_cm` and one of them has an open line from the pose;
    ///    otherwise it is thrown away with its search stamp, as Q.7 did.
    /// 4. **The search**, as before, with the refusal memory
    ///    ([`MotorRow::refused`]) read before the budget and every expansion
    ///    counted.
    /// 5. **The aim**: the first lookahead cell outside the turning circle with
    ///    an open line and ahead of the beam; else the first such cell (a
    ///    U-turn); else the first lookahead cell with an open line; else the
    ///    route's first cell — re-planned by a limited unit and held by an
    ///    unlimited one when it is not a step the unit may drive.
    /// 6. **The shoulder**: a drivable diagonal first cell with one open
    ///    shoulder is aimed through that shoulder's centre.
    /// 7. **The orders**: the heading at the aim, the braking speed for the goal,
    ///    and — when the point aimed at is inside the circle — no faster than the
    ///    speed at which it would not be, so the machine converges instead of
    ///    orbiting.
    ///
    /// The circle, the beam, the lookahead, the open line, the drivable step, the
    /// shoulder and the turn speed are **pure functions of hashed state and the
    /// definition** — the pose, the speed, the mobility, the route, the goal, the
    /// avoid list and the ground — and add no state: nothing here is new to the
    /// hash but the two fields the refusal memory writes. Everything is decided
    /// at the tick's own `t_ms` (D-16-2), in integers — the steer's open line
    /// walks the offset to its point and never a bearing — and the one float
    /// read is `bearing_of`, where it was before this cut: the heading order
    /// itself, and the routeless steer's direction (16b cut Q.7's).
    ///
    /// # The motor's door (17b cut M1a.1; doc 17 §4, D-UM-3)
    ///
    /// It drives one [`MotorRow`] — `row`, **taken out of** `motors` by the
    /// caller for the length of the drive — and spends `motors`' budget and
    /// scratch, so the row and the table are two disjoint borrows while the arena
    /// stays `&self`. Its doctrine is `self.arena_cfg.reflex`, the very value the
    /// reflex memory was built from at check-out and which nothing writes
    /// afterwards. It writes its orders onto `plan` and answers what it came to.
    ///
    /// **It no longer takes the order**, which it held until this cut for one
    /// thing: 16b cut E.2's `B-41` rule, that a failed search for the order's own
    /// `Advance` cell ends the order for this unit — and, since 16b cut Z.2, a
    /// failed search for a supporter's station — with E.4's two give-ups (the
    /// search's plot, the hunt's cell) beside them. Those are the decision's
    /// fields, so the `unreachable` edge answers the goal that failed
    /// ([`MotorOutcome::unreachable`]) and the reflex decision performed them
    /// from it, in the order this edge did, until 17b cut M4.6 deleted it; the
    /// steer itself is unchanged.
    ///
    /// `policy` is what the edge does to the goal: [`Fallback::Waypoint`] is the
    /// rule this function has always applied — the O.1 retry clock and the
    /// fallback waypoint, then the steer on to steps 5 to 7 — and
    /// [`Fallback::Notice`] (17b cut M1a.4's) drops the goal and orders a stop
    /// instead. Step 2's arrival answers [`MotorOutcome::arrived`] under both,
    /// and under `Notice` drops the goal too.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn follow(
        &self,
        id: ObjId,
        t_ms: i64,
        motors: &mut MotorTable,
        row: &mut MotorRow,
        here: Option<(u16, u16)>,
        policy: Fallback,
        counters: &mut MotorCounters,
        plan: &mut MotorPlan,
    ) -> MotorOutcome {
        let mut outcome = MotorOutcome::default();
        let Some(unit) = self.unit(id) else {
            return outcome;
        };
        let cfg = &self.arena_cfg.reflex;
        let (Some(here), Some(goal)) = (here, row.goal) else {
            plan.speed_cms = Some(0);
            return outcome;
        };
        // **16b cut AI.1**: the cells this unit has been stopped at, as the plain
        // list the search, the edge rule and the steer all read — at most
        // `REFLEX_AVOID_MAX` of them, built once per tick.
        let avoid: Vec<(u16, u16)> = row.avoid.iter().map(|(cell, _)| *cell).collect();
        let cell_cm = i64::from(self.definition.terrain.cell_cm().max(1));

        // ---- 1. consumption ----
        //
        // Cells already behind the unit leave the route. Before 16b cut AI.1 that
        // was only the cells it stood in, one at a time; a machine that passed
        // *beside* its route's first cells — turning onto it, or round a corner
        // through a shoulder — kept them, and steered back at a cell it had
        // passed. So: of the first `STEER_LOOKAHEAD_CELLS + 1` cells, the **last**
        // that is the cell it stands in, or a neighbour it may drive to in one
        // step, marks where it is along the route; everything before it goes,
        // and so does that cell when it is the one stood in.
        //
        // **Only across a drivable step.** A route that comes back beside its
        // start across an edge the search refused — a two-wall pinch, a bluff
        // with a ramp — is not shortened **to the cell it comes back to** when
        // that edge is one no motion takes: A\* detours to an adjacent cell
        // exactly when the direct edge is refused, so draining to it across a
        // corner closed on both shoulders, or up the bluff's own face, would be
        // the loop this cut removes (the follower aims across the closed corner,
        // is turned back and re-plans), or a `too_steep` stop at the route's own
        // next cell below the bluff.
        //
        // **One refusal of the search's is drained, by design**: a diagonal with
        // one `Open` and one `Steep` shoulder, which the search refuses
        // (`ShoulderVerdict::TooSteep`, 16b cut M.4's safe direction for a route)
        // and `Arena::step_is_drivable` accepts — motion tests each orthogonal
        // leg, and both legs of the open shoulder pass. At the foot of a ramp the
        // route's detour through the open shoulder is drained to the ramp's own
        // diagonal cell. What keeps that safe is the rest of the steer, not this
        // rule: the steer's open line needs **both** shoulders open on a diagonal
        // change (`Arena::line_is_open`, `steer: true`), so no lookahead cell
        // across it is a candidate, and the fourth arm's shoulder aim
        // (`Arena::first_step_aim`) drives it through the open shoulder's centre.
        // Loosen either and the steep shoulder is back in the machine's path
        // (`ai1_consumption_drains_only_across_a_drivable_step`'s second case).
        let mut behind: Option<usize> = None;
        for (at, cell) in row
            .route
            .iter()
            .take(STEER_LOOKAHEAD_CELLS.saturating_add(1))
            .enumerate()
        {
            if *cell == here
                || (adjacent(here, *cell) && self.step_is_drivable(unit, here, *cell, &avoid))
            {
                behind = Some(at);
            }
        }
        if let Some(at) = behind {
            if row.route.get(at) == Some(&here) {
                row.route.drain(..=at);
            } else {
                row.route.drain(..at);
            }
        }

        // ---- 2. arrival ----
        //
        // **17b cut M1a.1**: answered to the caller, whatever the policy; under
        // `Notice` the goal a pilot or a mind named is spent, so it goes too.
        // Under `Waypoint` the goal stays: the reflex controller's goals were
        // its decision's to drop, until 17b cut M4.6 deleted it.
        if here.0.abs_diff(goal.0).max(here.1.abs_diff(goal.1)) <= ARRIVE_CELLS {
            row.route.clear();
            plan.speed_cms = Some(0);
            outcome.arrived = true;
            if policy == Fallback::Notice {
                row.goal = None;
            }
            return outcome;
        }

        // ---- 3. the wander ----
        //
        // A route whose next cell is not a neighbour of this one is a route the
        // unit may have wandered off (it was stopped, or pushed round an
        // obstacle, or it is turning onto it). **16b cut AI.1**: a wander inside
        // the turning circle is an **orbit**, and keeps its route — the nearest
        // of the first `STEER_LOOKAHEAD_CELLS` cells within the circle's diameter
        // plus a cell, and one of them with an open line from the pose, is a
        // machine turning back onto its route, and step 7 slows it until it can.
        // A shove, or a place, beyond that is a route that starts somewhere the
        // unit no longer is.
        let radius_cm = turning_radius_cm(unit, i64::from(cfg.decide_ms));
        if row.route.first().is_some_and(|next| !adjacent(here, *next))
            && !self.route_is_within_reach(unit, here, &row.route, radius_cm, &avoid)
        {
            row.route.clear();
            // **16b cut Q.7**: and the search that route came from is spent. A
            // unit that has been pushed off its route is standing somewhere the
            // route was never computed from, and making it wait out `repath_ms`
            // before it may say so is what left a tank crawling sideways past an
            // obstacle for four seconds at a time.
            row.searched_ms = None;
        }

        // ---- 4. the search ----
        if row.route.is_empty() {
            let fresh_enough = row
                .searched_ms
                .is_some_and(|at| t_ms.saturating_sub(at) < i64::from(cfg.repath_ms));
            if !fresh_enough && let Some(refusal) = row.refused {
                if refusal.at == here && Some(refusal.goal) == row.goal && t_ms < refusal.until_ms {
                    // **16b cut AI.1 (16c `B-84`): refused by name.** The route a
                    // search from here for this goal would return is the one
                    // that was just refused at its own next cell — so the search
                    // is not spent, the refusal is counted, and the machine holds
                    // until the memory lapses or an input changes (see
                    // [`MotorRow::refused`]). Expected never to run.
                    counters.path_refused_unchanged =
                        counters.path_refused_unchanged.saturating_add(1);
                    plan.speed_cms = Some(0);
                    return outcome;
                }
                // A memory that no longer matches — another cell, another goal,
                // or past its time — is dropped where it is read.
                row.refused = None;
            }
            if fresh_enough {
                counters.path_deferred = counters.path_deferred.saturating_add(1);
            } else if motors.may_search(t_ms, cfg) {
                row.searched_ms = Some(t_ms);
                // **16b cut AI.1**: where it was spent, and a spent search is new
                // information whatever it finds.
                row.searched_from = Some(here);
                row.refused = None;
                counters.path_queries = counters.path_queries.saturating_add(1);
                let budget = cfg.path_max_cells;
                // **16b cut AE.1**: the heuristic's scale is this unit's own
                // row's cheapest cell. A scale above the row's true minimum makes
                // the heuristic inadmissible (`crate::arena::path::path`'s own
                // doc): a car's search scaled on the tracked 100 over an 80 road
                // would plan routes that are not its cheapest.
                let scale = self
                    .min_cost
                    .get(unit.cost_row.index())
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                // **16b cut Q.7**: the cells this unit has been stopped at — the
                // list built at the top of the tick (16b cut AI.1).
                //
                // **16b cut X.6**: the unit's own slope limit goes into the
                // search, so the route the controller follows is a route the
                // crossing will accept. A route over an edge motion then refuses
                // is a tank driving at a bluff and re-planning at it every
                // `repath_ms` for the rest of the match.
                let searched = path_with(
                    &self.definition.terrain,
                    &mut motors.scratch,
                    here,
                    goal,
                    budget,
                    scale,
                    unit.slope_limit_pct,
                    unit.cost_row,
                    &avoid,
                );
                // 16b cut U.1: what the search refused by slope, won or lost —
                // and, since 16b cut AI.1, as a closed corner.
                counters.slope_refusals = counters
                    .slope_refusals
                    .saturating_add(motors.scratch.slope_refusals());
                counters.corner_refusals = counters
                    .corner_refusals
                    .saturating_add(motors.scratch.corner_refusals());
                // **16b cut AI.1 (16c `B-84`)**: every expansion is counted, found
                // or failed, from the one source — a `Budget` failure is
                // `path_max_cells` of them and a `NoRoute` the whole reachable
                // region, and `harness.max_path_cells_per_pulse` bounds this.
                counters.path_cells = counters
                    .path_cells
                    .saturating_add(u64::from(motors.scratch.last_expanded()));
                match searched {
                    Ok(found) => {
                        row.route = found.cells;
                    }
                    Err(why) => {
                        // **The `unreachable` edge** (doc 16 §6): over budget, no
                        // route, or ground that is not ground. The goal is given up
                        // and the nearest waypoint takes its place; a patrol whose
                        // own node is what failed steps onward along the graph
                        // instead, so the fallback cannot be the same refusal twice.
                        counters.path_failures = counters.path_failures.saturating_add(1);
                        let _ = why;
                        // **17b cut M1a.1**: the goal that failed is answered to
                        // the caller, which is what let the reflex decision
                        // perform `B-41`'s, Z.2's and E.4's give-ups on its own
                        // fields, in the order this edge did them (until 17b
                        // cut M4.6; the motor step reads it for its
                        // `drive_no_route` notice since); the retry
                        // clock and the fallback below touch none of those
                        // fields, so performing them after the drive moves no
                        // byte.
                        outcome.unreachable = Some(goal);
                        if policy == Fallback::Notice {
                            // **17b cut M1a.4's policy**: a goal a pilot or a
                            // mind named is not swapped for a waypoint behind
                            // its back. It is dropped, the hull stops, and the
                            // one who named it hears of it.
                            row.goal = None;
                            row.route.clear();
                            plan.speed_cms = Some(0);
                            return outcome;
                        }
                        // **16b cut O.1 (16c N11; plan items 16, 34; `B-41`'s
                        // lesson applied once more)**: a Hold spot no route
                        // reaches — a spot sealed off to this hull's class — is
                        // given up here too, or `hold_goal` would re-assert it on
                        // the very next tick and the fallback waypoint would never
                        // hold. Given up for `max(hunt_after_ms, repath_ms)`: one
                        // failed search per retry, and **floored at `repath_ms`
                        // because `hunt_after_ms` 0 is the documented "off"
                        // value**. The floor does not change how often the spot
                        // is searched — `repath_ms` already throttles a search
                        // above (`fresh_enough`) — but what the machine does in
                        // between: unfloored at 0, `hold_goal` would re-assert the
                        // spot on the very next tick and the machine would steer
                        // straight at the sealed pocket between two failures,
                        // which is `B-41`'s own defect; floored, the fallback
                        // waypoint holds until the retry.
                        if self.hold.is_some_and(|hold| row.goal == Some(hold.spot)) {
                            row.hold_retry_ms =
                                Some(t_ms.saturating_add(i64::from(
                                    cfg.hunt_after_ms.max(cfg.repath_ms),
                                )));
                        }
                        self.fall_back_to_waypoint(id, row, here);
                    }
                }
            } else {
                counters.path_deferred = counters.path_deferred.saturating_add(1);
            }
        }
        let decide_ms = i64::from(cfg.decide_ms);

        // ---- 5. and 6. the aim, and the shoulder ----
        let Some(first) = row.route.first().copied() else {
            self.steer_routeless(unit, here, goal, &avoid, decide_ms, counters, plan);
            return outcome;
        };
        let aim = match self.steer_aim(unit, here, &row.route, radius_cm, &avoid) {
            Some(point) => point,
            // **The route's first cell**, which step 3 guarantees is a neighbour
            // of `here` — a wander is kept only while a lookahead cell has an
            // open line, and then one of the three arms above answered.
            None if !self.step_is_drivable(unit, here, first, &avoid) => {
                if unit.slope_limit_pct.is_some() {
                    // **A limited unit re-plans.** It meets an undrivable first
                    // step only from a cell its route was not planned from —
                    // consumption drains only across drivable steps, and the
                    // first step of a route planned here is one the search
                    // accepted — so the one search from here (the next tick's)
                    // finds the way round the corner or the refused edge, or
                    // fails once through the `unreachable` edge; it cannot return
                    // this route. Until then it steers routeless, as a wander does.
                    row.route.clear();
                    row.searched_ms = None;
                    self.steer_routeless(unit, here, goal, &avoid, decide_ms, counters, plan);
                } else {
                    // **An unlimited unit holds**, with its route and without a
                    // search: its search reads no shoulder and would return the
                    // same route. A fixture's ground says hold; an avoided
                    // shoulder ages out after `REFLEX_AVOID_TTL_MS`.
                    plan.speed_cms = Some(0);
                }
                return outcome;
            }
            None => self.first_step_aim(unit, here, first, &avoid, cell_cm),
        };

        // ---- 7. the orders ----
        //
        // The heading at the aim, as before; the braking speed measured to the
        // **goal**, as before — and, when the point aimed at is inside the
        // turning circle, no faster than the speed at which it is not. Whatever
        // is aimed at: a lookahead cell in the third arm, the route's first cell,
        // or the open shoulder's centre (a candidate is never inside by
        // construction). The machine slows until the point leaves its circle and
        // converges on it, instead of circling it at full speed.
        plan.heading_mdeg = Some(bearing_of(
            aim.0.saturating_sub(unit.pose.x_cm),
            aim.1.saturating_sub(unit.pose.y_cm),
        ));
        let mut speed = braking_speed_cms(unit, goal, cell_cm, decide_ms);
        if inside_turning_circle(&unit.pose, radius_cm, aim) {
            let turn = turn_speed_cms(&unit.pose, unit.mobility.turn_rate_mdeg_s, aim);
            speed = speed.min(i32::try_from(turn).unwrap_or(i32::MAX));
        }
        plan.speed_cms = Some(speed);
        outcome
    }

    /// **The routeless steer** (16b cut Q.7; factored by 16b cut AI.1 so the
    /// empty route and a limited unit's re-plan are one steer): the goal's
    /// direction, at the braking speed for the goal.
    ///
    /// **16b cut Q.7** (closes 16b cut X.6's deviation 13): with no route, the
    /// goal is a *direction* and nobody has checked the ground between here and
    /// it. The one cell the unit would actually cross first is checked with the
    /// crossing's own rule, and a unit that would be refused it waits for its
    /// search rather than driving at a cliff or back into the cell it was just
    /// stopped at. With a route the question is already answered — the search
    /// asked it of every edge it planned.
    #[allow(clippy::too_many_arguments)]
    fn steer_routeless(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        goal: (u16, u16),
        avoid: &[(u16, u16)],
        decide_ms: i64,
        counters: &mut MotorCounters,
        plan: &mut MotorPlan,
    ) {
        let cell_cm = i64::from(self.definition.terrain.cell_cm().max(1));
        let (to_x, to_y) = cell_centre(goal, cell_cm);
        let heading = bearing_of(
            to_x.saturating_sub(unit.pose.x_cm),
            to_y.saturating_sub(unit.pose.y_cm),
        );
        if !self.first_step_is_open(unit, here, heading, avoid, counters) {
            plan.speed_cms = Some(0);
            return;
        }
        plan.heading_mdeg = Some(heading);
        plan.speed_cms = Some(braking_speed_cms(unit, goal, cell_cm, decide_ms));
    }

    /// **Where a follower steers, among its route's lookahead cells** (16b cut
    /// AI.1; 16c `B-84`; `Arena::follow`'s step 5), or `None` when none of them
    /// has an open line from the pose.
    ///
    /// The *candidates* are the first [`STEER_LOOKAHEAD_CELLS`] route cells that
    /// are **not inside** the unit's turning circle of `radius_cm` and have an
    /// **open line** from the pose (`Arena::line_is_open`, the steer's walk).
    /// The answer is, in this order:
    ///
    /// 1. **the first candidate ahead of the beam** — a cell the machine can
    ///    turn onto without turning its back on the way it is going;
    /// 2. else **the first candidate** — a U-turn, when nothing ahead
    ///    qualifies: a cell straight behind is never inside the circle, and a
    ///    machine whose goal is behind it turns round;
    /// 3. else **the first lookahead cell with an open line** — inside the
    ///    circle, which the orders then slow for.
    ///
    /// Preferring a cell ahead is what keeps a machine that has passed beside its
    /// route from U-turning to a cell behind it; consumption drains the cells it
    /// has passed. The cheap tests (the circle, the beam) are asked first and a
    /// line is walked only for a cell that could still be the answer, so a
    /// machine straight on its route pays the one walk to its first cell.
    /// Answers the centre of the chosen cell, centimetres.
    fn steer_aim(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        route: &[(u16, u16)],
        radius_cm: u64,
        avoid: &[(u16, u16)],
    ) -> Option<(i64, i64)> {
        let cell_cm = i64::from(self.definition.terrain.cell_cm().max(1));
        let mut first_candidate: Option<(i64, i64)> = None;
        let mut first_open: Option<(i64, i64)> = None;
        for cell in route.iter().take(STEER_LOOKAHEAD_CELLS) {
            let point = cell_centre(*cell, cell_cm);
            let inside = inside_turning_circle(&unit.pose, radius_cm, point);
            let ahead = ahead_of_beam(&unit.pose, point);
            let wanted = (!inside && (ahead || first_candidate.is_none())) || first_open.is_none();
            if !wanted || !self.line_is_open_to(unit, here, point, avoid) {
                continue;
            }
            first_open.get_or_insert(point);
            if inside {
                continue;
            }
            first_candidate.get_or_insert(point);
            if ahead {
                return Some(point);
            }
        }
        first_candidate.or(first_open)
    }

    /// **The aim at the route's first cell** (16b cut AI.1; `Arena::follow`'s
    /// steps 5 and 6), for a step `Arena::step_is_drivable` has already
    /// answered yes to: its centre, or — for a diagonal with exactly one open
    /// shoulder — **that shoulder's centre**.
    ///
    /// Motion drives a diagonal through whichever shoulder its line crosses (a
    /// unit crosses one boundary a substep), so:
    ///
    /// - **both shoulders open**: the diagonal cell itself, as before — either
    ///   shoulder takes the unit;
    /// - **exactly one open**: its centre, a one-cell lateral step. The route is
    ///   kept, and from the shoulder the diagonal cell is an orthogonal
    ///   neighbour, so consumption carries on. Aimed at the diagonal instead,
    ///   the line could cross the other shoulder — a wall, a cliff or a cell the
    ///   unit was stopped at — which is 16c `B-84`'s corner stop, taken before it
    ///   happens rather than learned after;
    /// - **none open**: not a drivable step, and answered before this is asked
    ///   (a limited unit re-plans, an unlimited one holds).
    ///
    /// An orthogonal first cell's chord crosses only the edge the two cells
    /// share, so it is aimed at directly.
    fn first_step_aim(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        first: (u16, u16),
        avoid: &[(u16, u16)],
        cell_cm: i64,
    ) -> (i64, i64) {
        let terrain = &self.definition.terrain;
        if slope::step_kind(here, first) != Some(mush_atlas::physics::StepKind::Diagonal) {
            return cell_centre(first, cell_cm);
        }
        let (Some(here_cell), Some(to_cell)) =
            (terrain.get(here.0, here.1), terrain.get(first.0, first.1))
        else {
            return cell_centre(first, cell_cm);
        };
        let shoulders = shoulder_state(
            terrain,
            unit.slope_limit_pct,
            unit.cost_row,
            here_cell,
            to_cell,
            here,
            first,
            avoid,
        );
        match shoulders {
            [Shoulder::Open, Shoulder::Open] => cell_centre(first, cell_cm),
            [Shoulder::Open, _] => cell_centre((first.0, here.1), cell_cm),
            [_, Shoulder::Open] => cell_centre((here.0, first.1), cell_cm),
            _ => cell_centre(first, cell_cm),
        }
    }

    /// **Whether this unit may drive `here → to` in one step** (16b cut AI.1;
    /// 16c `B-84`) — the follower's own question, for an 8-neighbour `to`.
    ///
    /// Four tests: `to` is priced passable to the unit's row; it is not on the
    /// `avoid` list; the edge's own steepness is within the unit's slope limit
    /// either way, exactly as `edge_verdict` tests it (an unlimited unit is
    /// refused by no slope); and, on a diagonal, **at least one** shoulder is
    /// `Shoulder::Open` (`shoulder_state`) — motion drives the corner
    /// through a shoulder, so a corner with none is a stop. A `to` that is not a
    /// neighbour (or is `here`) is not a step.
    ///
    /// **It is looser than the search's verdict on one point only**: a diagonal
    /// with one `Open` and one `Steep` shoulder is drivable here, through the
    /// open shoulder — motion tests each orthogonal leg as it crosses it, and
    /// both legs of an open shoulder pass — where the search refuses it
    /// (`ShoulderVerdict::TooSteep`) as 16b cut M.4 designed it to, the safe
    /// direction for a route. The follower's shoulder aim (`Arena::follow`'s
    /// step 6) is what drives it through the open one. And it is stricter than
    /// the search for an **unlimited** unit, whose search reads no shoulder at
    /// all: that is the follower's own rule covering the class gap the search
    /// leaves open without a limit.
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it reads the ground, the unit's own row, slope limit and the
    /// `avoid` list it is handed, and nothing else — `crates/tacsim/tests/motor.rs`
    /// holds it to that, and 17b cut M1a.6 lifts it into a free function.
    pub fn step_is_drivable(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        to: (u16, u16),
        avoid: &[(u16, u16)],
    ) -> bool {
        let terrain = &self.definition.terrain;
        let Some(kind) = slope::step_kind(here, to) else {
            return false;
        };
        let (Some(here_cell), Some(to_cell)) =
            (terrain.get(here.0, here.1), terrain.get(to.0, to.1))
        else {
            return false;
        };
        if terrain.price(unit.cost_row, to_cell) == 0 || avoid.contains(&to) {
            return false;
        }
        if slope::too_steep(
            unit.slope_limit_pct,
            slope::steepness_pct(here_cell, to_cell, kind, u32::from(terrain.cell_cm())),
        ) {
            return false;
        }
        kind != mush_atlas::physics::StepKind::Diagonal
            || shoulder_state(
                terrain,
                unit.slope_limit_pct,
                unit.cost_row,
                here_cell,
                to_cell,
                here,
                to,
                avoid,
            )
            .contains(&Shoulder::Open)
    }

    /// **Whether a follower off its route's next cell is still on its route**
    /// (16b cut AI.1; `Arena::follow`'s step 3, the wander rule): the nearest of
    /// the route's first [`STEER_LOOKAHEAD_CELLS`] cells is within `2 ×
    /// radius_cm + cell_cm` of the pose (planar, at the unit's own height), and
    /// at least one of them has an open line from the pose.
    ///
    /// Twice the radius is the circle's diameter: a machine turning onto its
    /// route is never further off it than that, and a cell more for the cell it
    /// is turning onto. A shove, or a place, beyond it — or a route none of whose
    /// next cells can be reached in a straight line — is a route that starts
    /// somewhere the unit no longer is.
    fn route_is_within_reach(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        route: &[(u16, u16)],
        radius_cm: u64,
        avoid: &[(u16, u16)],
    ) -> bool {
        let cell_cm = i64::from(self.definition.terrain.cell_cm().max(1));
        let reach = radius_cm
            .saturating_mul(2)
            .saturating_add(u64::try_from(cell_cm).unwrap_or(0));
        let look = || {
            route
                .iter()
                .take(STEER_LOOKAHEAD_CELLS)
                .map(|cell| cell_centre(*cell, cell_cm))
        };
        let nearest = look()
            .map(|(x, y)| {
                range_between(
                    unit.pose.x_cm,
                    unit.pose.y_cm,
                    unit.pose.z_cm,
                    x,
                    y,
                    unit.pose.z_cm,
                )
            })
            .min();
        nearest.is_some_and(|off| off <= reach)
            && look().any(|point| self.line_is_open_to(unit, here, point, avoid))
    }

    /// **The steer's open line to a point** (16b cut AI.1): `Arena::line_is_open`'s
    /// walk with `steer: true`, straight at the point and out to its planar
    /// range from the pose.
    ///
    /// The direction is the offset itself over the range (`Arena::walk_is_open`),
    /// **not** a bearing: no float and no [`bearing_of`] reaches this decision
    /// (doc 16 §3.1's D-16-4, and 16b cut AI.1's lead decision). The heading
    /// order the machine is then given is `bearing_of` to the same point, which
    /// differs from this line by at most the millidegree it is rounded to.
    fn line_is_open_to(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        point: (i64, i64),
        avoid: &[(u16, u16)],
    ) -> bool {
        let (dx, dy) = (
            point.0.saturating_sub(unit.pose.x_cm),
            point.1.saturating_sub(unit.pose.y_cm),
        );
        let reach = i64::try_from(range_between(
            unit.pose.x_cm,
            unit.pose.y_cm,
            unit.pose.z_cm,
            point.0,
            point.1,
            unit.pose.z_cm,
        ))
        .unwrap_or(i64::MAX);
        self.walk_is_open(unit, here, (dx, dy, reach), reach, avoid, true)
    }

    /// **Whether the one cell on `heading` is a cell this unit would plan
    /// through** (16b cut Q.7).
    ///
    /// The neighbour is the octant of the bearing — the eight compass directions,
    /// each owning 45 degrees — because that is the cell
    /// [`crate::arena::motion`]'s step 4 will test first. Then four questions: it
    /// is on the window, its class is passable, the edge onto it is within this
    /// unit's slope limit **either way**, and on a diagonal its shoulders are not
    /// a cliff (`shoulder_refuses`) — and, since 16b cut AI.1, not **both
    /// closed** (off the window, impassable to the row, or on the avoid list):
    /// under a limit such a corner is refused here where it passed before. And
    /// one of Q.7's own: it is not a cell the unit has been stopped at
    /// ([`MotorRow::avoid`]).
    ///
    /// # It is the search's rule, not the crossing's — deliberately stricter
    ///
    /// **16b cut E.4 (16c `T-42`)** corrects what this paragraph used to claim.
    /// It said the four questions were asked "exactly as `motion`'s step 4 will
    /// ask" them, "in the crossing's own order", shoulder rule included. That is
    /// not true and cannot be: `crate::arena::motion::refusal` applies **no**
    /// shoulder rule at all — its own doc says a diagonal crossing tests the
    /// diagonal neighbour and nothing else — and the crossing knows nothing of a
    /// machine's remembered stops.
    ///
    /// What this really is: **the search's own edge rule
    /// ([`crate::arena::path`]'s, shoulders and all) plus the remembered stops**,
    /// which is strictly stricter than the crossing. That is the right way round
    /// and is the reason the function exists: a routeless machine is steering at
    /// a *direction* no search has checked, and the one thing it must never do is
    /// take a step its own search would refuse to plan — it would be stopped or
    /// turned aside, and then re-plan into the same step next tick. A cell motion
    /// would have let it through and the search would not costs it one held
    /// substep and a route a tick later; the other way round costs it the rest of
    /// the match.
    ///
    /// A unit with no slope limit at all is refused by no slope, exactly as
    /// `path` and `motion` refuse it by none.
    ///
    /// **16b cut U.1**: a refusal by the edge rule — the slope either way or the
    /// shoulders, after the class and the avoid list let the cell through — is
    /// counted in [`MotorCounters::slope_refusals`]. **16b cut AI.1**: by
    /// `edge_verdict`, so a slope refusal (`EdgeVerdict::TooSteep`) is
    /// counted there and a corner closed on both shoulders
    /// (`EdgeVerdict::Corner`) in [`MotorCounters::corner_refusals`] — the
    /// harness's proof that the cohort met the relief keeps its meaning. It takes
    /// the plain cell list the tick built once.
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it reads the ground, the unit's own pose, row and slope
    /// limit and the `avoid` list it is handed, and never the spatial index —
    /// `crates/tacsim/tests/motor.rs` holds it to that, and 17b cut M1a.6 lifts
    /// it into a free function.
    pub fn first_step_is_open(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        heading_mdeg: u32,
        avoid: &[(u16, u16)],
        counters: &mut MotorCounters,
    ) -> bool {
        let terrain = &self.definition.terrain;
        let (dx, dy) = octant_step(heading_mdeg);
        let (Some(nx), Some(ny)) = (
            u16::try_from(i32::from(here.0).saturating_add(dx)).ok(),
            u16::try_from(i32::from(here.1).saturating_add(dy)).ok(),
        ) else {
            return false;
        };
        if (nx, ny) == here {
            return true;
        }
        let (Some(here_cell), Some(to_cell)) = (terrain.get(here.0, here.1), terrain.get(nx, ny))
        else {
            return false;
        };
        // 16b cut AE.1: passable to this unit's own row, as the crossing asks.
        if terrain.price(unit.cost_row, to_cell) == 0 {
            return false;
        }
        if avoid.contains(&(nx, ny)) {
            return false;
        }
        match edge_verdict(terrain, unit, here_cell, to_cell, here, (nx, ny), avoid) {
            EdgeVerdict::Open => true,
            EdgeVerdict::TooSteep => {
                counters.slope_refusals = counters.slope_refusals.saturating_add(1);
                false
            }
            EdgeVerdict::Corner => {
                counters.corner_refusals = counters.corner_refusals.saturating_add(1);
                false
            }
        }
    }

    /// **Whether every cell on a line from the pose within `reach_cm` is one the
    /// unit would plan through** (16b cut M.2's walk for
    /// [`Arena::hold_routeless`]; generalised by 16b cut AI.1 for the follower's
    /// steer, and named for what it answers).
    ///
    /// [`Arena::first_step_is_open`]'s four questions and Q.7's remembered stops,
    /// asked of each cell a line of travel enters: the pose is walked along
    /// `bearing_mdeg` in steps of half a cell, with [`crate::arena::motion`]'s
    /// own fixed-point sine and cosine (no float: doc 16 §3.1's D-16-4), and
    /// every change of cell is one edge put to the edge rule: the cell priced
    /// passable to the row, not on `avoid`, and the edge open. Half a cell is
    /// fine enough that the walk enters every cell the line does except where it
    /// clips a corner, and the corner it clips is the one the diagonal edge it
    /// then tests shoulders onto. A step off the window is a refusal, as the
    /// crossing's is. A `reach_cm` of nought walks nothing and answers `true`.
    ///
    /// # `steer: false` — the M.2 walk, `hold_routeless`'s question
    ///
    /// The line is the pose's own heading and the diagonal edge is asked
    /// `edge_is_open` — the search's rule, one open shoulder enough. Since 16b
    /// cut AI.1 that is the search's rule as that cut left it: under a limit a
    /// corner with **no** open shoulder is refused, and a shoulder on `avoid`
    /// is no shoulder (it read neither before); an unlimited unit is refused no
    /// shoulder at all. **The
    /// first cell the walk enters is not asked here**: the one cell in front is
    /// `first_step_is_open`'s question, asked exactly as Q.7 asked it (the octant
    /// the crossing tests first), and the walk only reaches the cells beyond it
    /// that the one-cell rule never looked at. This is measured, not tidiness:
    /// asking the walk's own first cell holds a unit that a stop has just
    /// clamped at a boundary — any reach at all, from there, is into the cell it
    /// was refused — so it can never turn away under power, re-plans from the
    /// same centimetre every `repath_ms` and is refused the same cell again; on
    /// the harness field that took one line tank to eight `blocked` lines at one
    /// cell and the field from 341 to 439.
    ///
    /// # `steer: true` — the follower's walks (16b cut AI.1)
    ///
    /// The line is the one to a route cell the follower may steer at, and there
    /// is no one-cell question in front of it, so **the first cell is asked
    /// too**. And on a **diagonal** cell change **both** shoulders must be
    /// `Shoulder::Open` (`shoulder_state`), where the search's rule is
    /// content with one: the half-cell walk cannot tell which shoulder the
    /// segment clips, motion enters whichever it is, and a line through a closed
    /// shoulder is not open. The edge's own slope is asked as the search asks it.
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it walks the ground from the unit's own pose against
    /// its row, its slope limit and the `avoid` list it is handed, and never the
    /// spatial index — `crates/tacsim/tests/motor.rs` holds it to that,
    /// and 17b cut M1a.6 lifts it into a free function.
    pub fn line_is_open(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        bearing_mdeg: u32,
        reach_cm: i64,
        avoid: &[(u16, u16)],
        steer: bool,
    ) -> bool {
        use crate::arena::motion::{SIN_ONE, cos_fx, sin_fx};
        self.walk_is_open(
            unit,
            here,
            (sin_fx(bearing_mdeg), cos_fx(bearing_mdeg), SIN_ONE),
            reach_cm,
            avoid,
            steer,
        )
    }

    /// **The walk under [`Arena::line_is_open`] and `Arena::line_is_open_to`**
    /// (16b cut AI.1): the half-cell walk from the pose along the direction
    /// `way = (east, north, scale)` — the point `walked` centimetres out is the
    /// pose plus `walked × east / scale` east and `walked × north / scale`
    /// north — out to `reach_cm`. A bearing's walk is `(sin, cos, SIN_ONE)` in
    /// [`crate::arena::motion`]'s fixed point; a walk **to a point** is the
    /// offset to it over its own range, so it ends on the point exactly and no
    /// angle is ever computed for it. Integers throughout.
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it reads the ground, the unit's own pose, row and slope
    /// limit and the `avoid` list it is handed, and never the spatial index —
    /// `crates/tacsim/tests/motor.rs` holds it to that, and 17b cut M1a.6 lifts
    /// it into a free function.
    pub fn walk_is_open(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        way: (i64, i64, i64),
        reach_cm: i64,
        avoid: &[(u16, u16)],
        steer: bool,
    ) -> bool {
        let terrain = &self.definition.terrain;
        let cell_cm = u32::from(terrain.cell_cm());
        let stride = (i64::from(terrain.cell_cm()) / 2).max(1);
        let (east, north, scale) = (way.0, way.1, way.2.max(1));
        let mut from = here;
        let mut walked: i64 = 0;
        while walked < reach_cm {
            walked = walked.saturating_add(stride).min(reach_cm);
            let x = unit
                .pose
                .x_cm
                .saturating_add(walked.saturating_mul(east) / scale);
            let y = unit
                .pose
                .y_cm
                .saturating_add(walked.saturating_mul(north) / scale);
            let Some(to) = terrain.cell_at(x, y) else {
                return false;
            };
            if to == from {
                continue;
            }
            if !steer && from == here {
                from = to;
                continue;
            }
            let (Some(from_cell), Some(to_cell)) =
                (terrain.get(from.0, from.1), terrain.get(to.0, to.1))
            else {
                return false;
            };
            if terrain.price(unit.cost_row, to_cell) == 0 || avoid.contains(&to) {
                return false;
            }
            let open = if steer {
                match slope::step_kind(from, to) {
                    None => false,
                    Some(kind) => {
                        !slope::too_steep(
                            unit.slope_limit_pct,
                            slope::steepness_pct(from_cell, to_cell, kind, cell_cm),
                        ) && (kind != mush_atlas::physics::StepKind::Diagonal
                            || shoulder_state(
                                terrain,
                                unit.slope_limit_pct,
                                unit.cost_row,
                                from_cell,
                                to_cell,
                                from,
                                to,
                                avoid,
                            ) == [Shoulder::Open, Shoulder::Open])
                    }
                }
            } else {
                edge_is_open(terrain, unit, from_cell, to_cell, from, to, avoid)
            };
            if !open {
                return false;
            }
            from = to;
        }
        true
    }

    /// The `unreachable` edge's landing: the nearest waypoint, or the next one
    /// along the graph when the nearest is what could not be reached.
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it reads the arena definition's waypoint graph, the unit's
    /// own object number (the `id % neighbours` step, departure (e) of the M1a
    /// plan) and its own [`MotorRow`], and writes only the row's goal, route
    /// and waypoint pair. 17b cut M1a.6's stock mind performs the same landing
    /// off a `drive_no_route` notice and lifts it into a free function.
    ///
    /// **17b cut M1a.6**: the arithmetic is the free [`fallback_waypoint`],
    /// which the stock mind calls too; this is its thin wrapper over the row.
    pub fn fall_back_to_waypoint(&self, id: ObjId, row: &mut MotorRow, here: (u16, u16)) {
        row.route.clear();
        let (waypoint, from_waypoint, goal) = fallback_waypoint(
            &self.definition,
            here,
            row.goal,
            row.waypoint,
            row.from_waypoint,
            id.0,
        );
        row.waypoint = waypoint;
        row.from_waypoint = from_waypoint;
        row.goal = goal;
    }

    /// `Patrol`'s goal: the node it is walking to, advanced when it arrives.
    ///
    /// **17b cut M1a.1** moved it here as `patrol_step` (it was the reflex
    /// controller's `patrol_goal`), body unchanged: it writes only the row's
    /// `waypoint`, `from_waypoint`, `goal` and `route`, which are all the
    /// motor's. **Public**, as one of the motor's audited kernels (doc 17 §4): it
    /// reads the waypoint graph, the unit's own object number (`id %
    /// choices.len()`, the M1a plan's departure (e)) and its own row, and no
    /// other unit — `crates/tacsim/tests/motor.rs` holds it to that.
    ///
    /// **17b cut M1a.6**: the walk is the free [`patrol_step`], which the stock
    /// mind calls too; this is its thin wrapper over the row — a test and
    /// oracle surface, with no production caller since 17b cut M4.6c deleted
    /// the reflex controller, which called it.
    pub fn patrol_step(&self, id: ObjId, row: &mut MotorRow, here: Option<(u16, u16)>) {
        let step = patrol_step(
            &self.definition,
            id.0,
            here,
            row.outcome == Some(DriveOutcome::Arrived),
            Walk {
                waypoint: row.waypoint,
                from_waypoint: row.from_waypoint,
                goal: row.requested_goal.or(row.goal),
            },
        );
        row.waypoint = step.walk.waypoint;
        row.from_waypoint = step.walk.from_waypoint;
        row.goal = step.walk.goal;
        if step.clear_route {
            row.route.clear();
        }
    }

    /// **Whether `unit`, standing in `here`, has arrived at the Hold spot**
    /// (16b cut O.1's arrival rule; plan items 2, 16, 34): a hull that
    /// [`UnitState::cannot_close`], or one whose cell is within `radius_cells −
    /// 1` of the spot (Chebyshev) — anywhere inside and not the spot's own cell,
    /// because presence is all a hold needs and one cell with [`ARRIVE_CELLS`]
    /// of one cannot seat a side; one cell short of the edge, so a machine
    /// braking onto the square does not stop astride it. `false` on a match with
    /// no Hold objective.
    ///
    /// **17b cut M1a.1** lifted it out of the reflex controller's `hold_goal`,
    /// which kept the retry clock and called this for the arithmetic until 17b
    /// cut M4.6 deleted it. **Public**,
    /// as one of the motor's audited kernels (doc 17 §4): it reads the arena's
    /// Hold objective and the unit's own mobility and cell, and no other unit.
    ///
    /// **17b cut M1a.6**: the arithmetic is the free [`hold_arrived`]; this is
    /// its wrapper over the arena's objective and the hull's mobility — a test
    /// and oracle surface, with no production caller since 17b cut M4.6c (the
    /// stock mind's `hold_goal` calls the free one).
    #[must_use]
    pub fn hold_arrived(&self, unit: ObjId, here: Option<(u16, u16)>) -> bool {
        let Some(hold) = self.hold.as_ref() else {
            return false;
        };
        hold_arrived(
            &hold.objective(),
            self.unit(unit).is_some_and(UnitState::cannot_close),
            here,
        )
    }

    /// The cell to stand in to hold `want_cm` of range on `at` — the goal
    /// `Approach` drives to.
    ///
    /// The point on the line from the plot toward *here*, at the holding range.
    /// A cell off the window or impassable falls back to the plot's own cell,
    /// and the path search's `unreachable` edge is what catches that.
    ///
    /// **16b cut F.3** made `at` the contact's plot rather than the target's
    /// object: an approach is toward the last known position, and a unit that
    /// arrives there and finds nothing is a unit that has lost its contact.
    ///
    /// **16b cut E.1**: `want_cm` came from the reflex module's
    /// `stand_off_want` (`crate::arena::doctrine::stand_off_want` since 17b
    /// cut M4.6a), which is what
    /// guarantees `want_cm ≤ range` — and that is what guarantees the **point**
    /// this projects is never further from the plot than the unit already
    /// stands, so the goal is never behind the machine (16c `B-05`). The *cell*
    /// it is quantised to may be up to seven tenths of a cell further out in the
    /// degenerate case where the floor of one cell is the answer, and is two
    /// cells nearer or better at every shipped distance.
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it reads the plot it is handed, the unit's own pose and cost
    /// row and the ground — never the target's live pose (16b cut F.3's rule;
    /// `crates/tacsim/tests/motor.rs` moves the target and finds the answer
    /// unchanged) — and 17b cut M1a.6 lifts it into a free function.
    ///
    /// **17b cut M1a.6**: the arithmetic is the free [`stand_off_cell`]; this
    /// is its wrapper over the hull's own pose and cost row and the plot.
    pub fn stand_off_cell(&self, id: ObjId, at: &SeenPose, want_cm: u64) -> Option<(u16, u16)> {
        let from = self.unit(id)?;
        stand_off_cell(
            &self.definition,
            from.cost_row,
            (from.pose.x_cm, from.pose.y_cm, from.pose.z_cm),
            (at.x_cm, at.y_cm, at.z_cm),
            want_cm,
        )
    }

    /// `Withdraw`'s goal: the nearest passable cell of this unit's **own** entry
    /// zone (doc 16 §6: *toward the entry zone*).
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it reads the arena definition's entry zones and ground and
    /// the unit's own team, pose and cost row, and no other unit —
    /// `crates/tacsim/tests/motor.rs` holds it to that, and 17b cut M1a.6 lifts
    /// it into a free function.
    ///
    /// **17b cut M1a.6**: the scan is the free [`entry_cell`]; this is its
    /// wrapper over the hull's own team, cell and cost row — a test and oracle
    /// surface, with no production caller since 17b cut M4.6c.
    pub fn entry_cell(&self, id: ObjId) -> Option<(u16, u16)> {
        let unit = self.unit(id)?;
        let here = self.definition.terrain.cell_of(&unit.pose)?;
        entry_cell(&self.definition, unit.team, unit.cost_row, here)
    }

    /// **The hunt's goal: a cell of some OTHER side's entry zone** (16b cut E.4;
    /// 16c `B-04`; fanned by 16b cut AI.1), or `None`.
    ///
    /// [`Arena::entry_cell`] is deliberately **not** touched by this cut — not
    /// its zone choice, not its metric, not its fallback — so `Withdraw`'s goal
    /// is exactly what it was. This is a second function with its own rule, and
    /// the whole of that rule is here:
    ///
    /// * over the entry zones whose team is **not** this unit's, **in zone
    ///   order**;
    /// * each zone's candidate is its **clamped corner** — the unit's own cell
    ///   clamped into the zone — when that cell is passable, else the **nearest
    ///   passable cell of the zone by Manhattan distance**, which is the metric
    ///   [`Arena::entry_cell`]'s scan already ranks by, so the file has one
    ///   ranking and not two; a candidate within [`HUNT_FAN_ROWS`] of a cell in
    ///   `taken` is **fanned** (below);
    /// * the scan reads the zone's rows **nearest the row it starts from
    ///   first** — that row, then one either side, then two — because
    ///   [`MAX_ENTRY_SCAN`] is 1,024 cells and a shipped zone is 16 by 256, so a
    ///   row-major scan from `y0` would hand a machine standing on row 200 a
    ///   cell on row 60;
    /// * one budget of [`MAX_ENTRY_SCAN`] cells is shared across every zone and
    ///   every fanned row;
    /// * the answer is the candidate **nearest the unit** by that same distance,
    ///   ties to the **lower zone index**.
    ///
    /// **The fan** (16b cut AI.1, the programme lead's ruling on `shipped_scale`
    /// e3; 16c `B-84`, `B-46`). Two machines of one side that clamp to one
    /// corner — two tanks in column on one road row — were handed one cell, and
    /// with the orbit gone both reach it and the first there turns back into
    /// the second. `taken` is every hunt cell the rest of the unit's side holds
    /// (`Arena::hunt_goal`, the reflex's, read it off the side's own reflex
    /// rows at the tick until 17b cut M4.6 deleted both; the stock mind's
    /// `hunt_goal` hands the free [`far_entry_cell`] none, and the motor fans
    /// the goal at its landing, `Arena::fan_goal`), and a candidate is
    /// **clear** when its Chebyshev distance to every
    /// one of them is at least [`HUNT_FAN_ROWS`], so no two arrival squares
    /// overlap. A clear candidate is the answer exactly as E.4 gave it: a lone
    /// hunter, and the first of every side, crosses where E.4 sent it, and so
    /// does one whose own row is already three rows from its neighbour's. One
    /// that is not clear walks the fan — the zone's rows `cy + s`, `cy − s`,
    /// `cy + 2s`, `cy − 2s`, … from its clamped row `cy`, with `s` =
    /// [`HUNT_FAN_ROWS`] and rows off the zone skipped (`fan_rows`) — taking
    /// at each row that row's corner when it is passable, else the zone's
    /// nearest passable cell to the unit's column on that row, and answering
    /// the first that is clear. So **every cell this answers is clear of every
    /// hunt its side holds** while the zone has room: whatever rows the hunters
    /// stand on and in whatever order their ticks fall. **When the fan runs out**
    /// — every fanned row's cell taken, or the scan budget spent — the E.4
    /// candidate is answered after all: a zone with more hunters than its rows
    /// have room for, or ground refusing the rows between, is the zone's own
    /// limit and not the rule's. It reads the side's own goals and no unit's
    /// pose, so it adds no state: the cell it answers becomes
    /// the hunt (the reflex row's `hunt` until 17b cut M4.6) and the goal,
    /// hashed as every goal is.
    ///
    /// **It never answers a cell the passability test rejected.**
    /// [`Arena::entry_cell`] ends `.or(Some(clamped))`, which can hand back the
    /// impassable corner it has just refused; for a `Withdraw` that is a goal the
    /// search fails on and a fallback waypoint catches, but for a hunt it would
    /// be an unreachable cell re-asserted every `hunt_after_ms` for the rest of
    /// the match. When the budget runs out with nothing found, or no other side
    /// has a zone at all, this answers `None` and the hunt does nothing — which
    /// is the honest answer, and the hunt restarts its clock on it so the scan
    /// is paid once per `hunt_after_ms` (the stock mind's `hunt_goal`;
    /// `Arena::hunt_goal`, the reflex's, until 17b cut M4.6).
    ///
    /// **Public since 17b cut M1a.1**, as one of the motor's audited kernels
    /// (doc 17 §4): it reads the arena definition's entry zones and ground, the
    /// unit's own team, pose and cost row, and the `taken` cells the caller
    /// hands it — the reflex decision read those off its own side's machines,
    /// which is the decision's knowledge and not the motor's, until 17b cut
    /// M4.6 — and no other unit. 17b cut M1a.6 lifts it into a free function.
    ///
    /// **17b cut M1a.6**: the scan and the fan are the free [`far_entry_cell`];
    /// this is its wrapper over the hull's own team, cell and cost row — a
    /// test and oracle surface, with no production caller since 17b cut M4.6c
    /// deleted its caller, the reflex's `Arena::hunt_goal`.
    pub fn far_entry_cell(&self, id: ObjId, taken: &[(u16, u16)]) -> Option<(u16, u16)> {
        let unit = self.unit(id)?;
        let here = self.definition.terrain.cell_of(&unit.pose)?;
        far_entry_cell(&self.definition, unit.team, unit.cost_row, here, taken)
    }
}

// ---------------------------------------------------------------------------
// The pickers, as free functions (17b cut M1a.6; doc 17 §3 "The map is public",
// §7's kernel list)
// ---------------------------------------------------------------------------

/// **Where a waypoint walk stands** (17b cut M1a.6): the node it is walking
/// to, the node it came from, and the goal — the three fields of a
/// [`MotorRow`] [`patrol_step`] reads and writes, and the three keys of the
/// stock mind's memory it keeps them in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Walk {
    /// The node it is walking to.
    pub waypoint: Option<u16>,
    /// The node it came from.
    pub from_waypoint: Option<u16>,
    /// The cell it is driving at.
    pub goal: Option<(u16, u16)>,
}

/// **One step of the walk** (17b cut M1a.6): the walk after it, and whether
/// the route that served the old goal is spent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalkStep {
    /// The walk after the step.
    pub walk: Walk,
    /// Whether the step threw the route away (a new node, or no graph).
    pub clear_route: bool,
}

/// **`Patrol`'s waypoint walk** (16b cut N.1's `patrol_goal`; the motor's
/// `Arena::patrol_step` since 17b cut M1a.1; lifted here by 17b cut M1a.6):
/// the node the walk holds, advanced only when `arrived` names completion of
/// the requested waypoint. Merely passing within the arrival radius does not
/// advance the walk before the motor has completed its physical destination.
/// The next node is `choices[unit % choices.len()]` over the neighbours other
/// than the one it came from (all of them when that leaves none).
///
/// It reads the definition's waypoint graph, the hull's own number and cell
/// and the walk, and nothing else: no graph authored is a goal of `None` and
/// a spent route (a patrol with nowhere to go holds where it is), a hull off
/// the window a goal of `None` with the route kept.
#[must_use]
pub fn patrol_step(
    def: &ArenaDefinition,
    unit: u32,
    here: Option<(u16, u16)>,
    arrived: bool,
    walk: Walk,
) -> WalkStep {
    let mut step = WalkStep {
        walk,
        clear_route: false,
    };
    let graph = &def.waypoints;
    if graph.is_empty() {
        // No graph authored: a patrol with nowhere to go holds where it is,
        // which is the honest answer and not a random walk.
        step.walk.goal = None;
        step.clear_route = true;
        return step;
    }
    let Some((x, y)) = here else {
        step.walk.goal = None;
        return step;
    };
    let mut node = match step.walk.waypoint {
        Some(node) if graph.node(node).is_some() => node,
        _ => {
            let Some(node) = graph.nearest(x, y) else {
                return step;
            };
            step.walk.from_waypoint = None;
            node
        }
    };
    let arrived = graph
        .node(node)
        .is_some_and(|at| arrived && step.walk.goal == Some((at.x, at.y)));
    if arrived {
        let neighbours = graph.neighbours(node);
        let onward: Vec<u16> = neighbours
            .iter()
            .copied()
            .filter(|next| Some(*next) != step.walk.from_waypoint)
            .collect();
        let choices: &[u16] = if onward.is_empty() {
            neighbours
        } else {
            &onward
        };
        if let Some(next) = choices
            .get(usize::try_from(unit).unwrap_or(0) % choices.len().max(1))
            .copied()
        {
            step.walk.from_waypoint = Some(node);
            node = next;
            step.walk.goal = None;
            step.clear_route = true;
        }
    }
    step.walk.waypoint = Some(node);
    let goal = graph.node(node).map(|at| (at.x, at.y));
    if step.walk.goal != goal {
        step.walk.goal = goal;
        step.clear_route = true;
    }
    step
}

/// **The `unreachable` edge's landing** (`Arena::fall_back_to_waypoint`'s
/// arithmetic, lifted by 17b cut M1a.6): with the hull in `here` and `goal`
/// the goal no route reached, the walk after the fallback — `(waypoint,
/// from_waypoint, goal)`. The nearest node of the graph; or, when the nearest
/// is the goal that failed, the next one along the graph,
/// `neighbours[unit % neighbours.len()]` (departure (e)). The node walked from
/// becomes `from_waypoint`. No graph to fall back on: the goal is given up
/// (`None`) and the walk kept — a hull that cannot get there and has nowhere
/// else to be does not drive at a wall.
///
/// The reflex controller's motor called it in-tick (`Fallback::Waypoint`)
/// until 17b cut M4.6; the stock mind calls it one think later, off a
/// `drive_no_route` notice (the M1a plan's departure (i)).
#[must_use]
pub fn fallback_waypoint(
    def: &ArenaDefinition,
    here: (u16, u16),
    goal: Option<(u16, u16)>,
    waypoint: Option<u16>,
    from_waypoint: Option<u16>,
    unit: u32,
) -> (Option<u16>, Option<u16>, Option<(u16, u16)>) {
    let graph = &def.waypoints;
    let Some(node) = graph.nearest(here.0, here.1) else {
        return (waypoint, from_waypoint, None);
    };
    let at = graph.node(node).map(|node| (node.x, node.y));
    let node = if at == goal {
        let neighbours = graph.neighbours(node);
        neighbours
            .get(usize::try_from(unit).unwrap_or(0) % neighbours.len().max(1))
            .copied()
            .unwrap_or(node)
    } else {
        node
    };
    (
        Some(node),
        waypoint,
        graph.node(node).map(|node| (node.x, node.y)),
    )
}

/// **Whether a hull standing in `here` has arrived at the Hold spot** (16b cut
/// O.1's rule; `Arena::hold_arrived`'s arithmetic, lifted by 17b cut M1a.6): a
/// hull that `cannot_close`, or one whose cell is within `radius_cells − 1` of
/// the spot (Chebyshev).
#[must_use]
pub fn hold_arrived(hold: &HoldObjective, cannot_close: bool, here: Option<(u16, u16)>) -> bool {
    cannot_close
        || here.is_some_and(|cell| {
            cell.0
                .abs_diff(hold.spot.0)
                .max(cell.1.abs_diff(hold.spot.1))
                <= hold.radius_cells.saturating_sub(1)
        })
}

/// **The cell to stand in to hold `want_cm` of range on a point** (16b cut
/// F.3, E.1; `Arena::stand_off_cell`'s arithmetic, lifted by 17b cut M1a.6):
/// the point on the line from `at` toward `from` at the holding range, as a
/// cell passable to `cost_row`; a cell off the window or impassable falls back
/// to `at`'s own cell. `from` and `at` are `(x_cm, y_cm, z_cm)`.
///
/// The reflex controller handed it the hull's pose and its plot of the
/// target, until 17b cut M4.6; the stock mind hands it the view's own pose and the **centre of the plot's
/// cell** at the ground's height there (the view carries the cell, not the
/// pose: the divergence `plot_cell`).
#[must_use]
pub fn stand_off_cell(
    def: &ArenaDefinition,
    cost_row: CostRow,
    from: (i64, i64, i32),
    at: (i64, i64, i32),
    want_cm: u64,
) -> Option<(u16, u16)> {
    let terrain = &def.terrain;
    let range = range_between(from.0, from.1, from.2, at.0, at.1, at.2);
    let want = i64::try_from(want_cm.max(1)).unwrap_or(i64::MAX);
    if range == 0 {
        return terrain.cell_at(from.0, from.1);
    }
    let range = i64::try_from(range).unwrap_or(i64::MAX).max(1);
    let dx = from.0.saturating_sub(at.0);
    let dy = from.1.saturating_sub(at.1);
    let x = at.0.saturating_add(dx.saturating_mul(want) / range);
    let y = at.1.saturating_add(dy.saturating_mul(want) / range);
    let cell = i64::from(terrain.cell_cm().max(1));
    let (Ok(cx), Ok(cy)) = (
        u16::try_from(x.div_euclid(cell)),
        u16::try_from(y.div_euclid(cell)),
    ) else {
        return terrain.cell_at(at.0, at.1);
    };
    // 16b cut AE.1: passable to this unit's own row — a walker's stand-off
    // cell may be in the thick woods, a car's may not be on rubble.
    if terrain.passable_for(cost_row, cx, cy) {
        Some((cx, cy))
    } else {
        terrain.cell_at(at.0, at.1)
    }
}

/// **`Withdraw`'s goal: the nearest passable cell of `team`'s own entry zone**
/// (doc 16 §6; `Arena::entry_cell`'s scan, lifted by 17b cut M1a.6) from the
/// hull's cell `here`, passable to `cost_row`: the clamped corner first, then
/// the nearest by Manhattan distance, the scan bounded by [`MAX_ENTRY_SCAN`]
/// (past it the best so far, else the clamped corner).
#[must_use]
pub fn entry_cell(
    def: &ArenaDefinition,
    team: PartyId,
    cost_row: CostRow,
    here: (u16, u16),
) -> Option<(u16, u16)> {
    let (here_x, here_y) = here;
    let mut best: Option<(u32, (u16, u16))> = None;
    let mut read = 0_usize;
    for zone in def.entry_zones.iter().filter(|zone| zone.team == team) {
        let (x0, x1) = (zone.x0.min(zone.x1), zone.x0.max(zone.x1));
        let (y0, y1) = (zone.y0.min(zone.y1), zone.y0.max(zone.y1));
        // The clamped corner first: it is the answer almost always, and it
        // costs one probe.
        let clamped = (here_x.clamp(x0, x1), here_y.clamp(y0, y1));
        // 16b cut AE.1: passable to this unit's own row, here and below.
        if def.terrain.passable_for(cost_row, clamped.0, clamped.1) {
            return Some(clamped);
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                read = read.saturating_add(1);
                if read > MAX_ENTRY_SCAN {
                    return best.map(|(_, cell)| cell).or(Some(clamped));
                }
                if !def.terrain.passable_for(cost_row, x, y) {
                    continue;
                }
                let distance = u32::from(x.abs_diff(here_x)) + u32::from(y.abs_diff(here_y));
                if best.is_none_or(|(far, _)| distance < far) {
                    best = Some((distance, (x, y)));
                }
            }
        }
    }
    best.map(|(_, cell)| cell)
}

/// An own-entry goal whose whole motor arrival square lies inside the zone.
///
/// Unlike [`entry_cell`], this is for a caller that needs physical entry,
/// rather than proximity to the nearest boundary. Each own zone is inset by
/// [`ARRIVE_CELLS`]; its clamped interior cell is tried first, then its nearest
/// passable interior cell by Manhattan distance. At most [`MAX_ENTRY_SCAN`]
/// terrain probes are shared across all zones. Thin, blocked or exhausted
/// zones supply no boundary fallback. Path clearance remains the motor's job.
#[must_use]
pub fn entry_interior_cell(
    def: &ArenaDefinition,
    team: PartyId,
    cost_row: CostRow,
    here: (u16, u16),
) -> Option<(u16, u16)> {
    let mut best: Option<(u32, (u16, u16))> = None;
    let mut read = 0_usize;
    for zone in def.entry_zones.iter().filter(|zone| zone.team == team) {
        let (Some(x0), Some(x1), Some(y0), Some(y1)) = (
            zone.x0.min(zone.x1).checked_add(ARRIVE_CELLS),
            zone.x0.max(zone.x1).checked_sub(ARRIVE_CELLS),
            zone.y0.min(zone.y1).checked_add(ARRIVE_CELLS),
            zone.y0.max(zone.y1).checked_sub(ARRIVE_CELLS),
        ) else {
            continue;
        };
        if x0 > x1 || y0 > y1 {
            continue;
        }
        let clamped = (here.0.clamp(x0, x1), here.1.clamp(y0, y1));
        if read == MAX_ENTRY_SCAN {
            return best.map(|(_, cell)| cell);
        }
        read += 1;
        if def.terrain.passable_for(cost_row, clamped.0, clamped.1) {
            return Some(clamped);
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                if (x, y) == clamped {
                    continue;
                }
                if read == MAX_ENTRY_SCAN {
                    return best.map(|(_, cell)| cell);
                }
                read += 1;
                if !def.terrain.passable_for(cost_row, x, y) {
                    continue;
                }
                let distance = u32::from(x.abs_diff(here.0)) + u32::from(y.abs_diff(here.1));
                if best.is_none_or(|(far, _)| distance < far) {
                    best = Some((distance, (x, y)));
                }
            }
        }
    }
    best.map(|(_, cell)| cell)
}

/// **The hunt's goal: a cell of some OTHER side's entry zone** (16b cut E.4,
/// fanned by AI.1; `Arena::far_entry_cell`'s scan, lifted by 17b cut M1a.6) —
/// see that method for the whole rule. `taken` is the hunt cells the rest of
/// the side holds: the reflex controller read them off its own machines until
/// 17b cut M4.6; the stock mind passes none, and the motor fans its landing
/// instead (the M1a plan's departure (d)).
#[must_use]
pub fn far_entry_cell(
    def: &ArenaDefinition,
    team: PartyId,
    cost_row: CostRow,
    here: (u16, u16),
    taken: &[(u16, u16)],
) -> Option<(u16, u16)> {
    let (here_x, here_y) = here;
    // 16b cut AE.1: passable to this unit's own row.
    let passable = |x: u16, y: u16| def.terrain.passable_for(cost_row, x, y);
    let reach =
        |cell: (u16, u16)| u32::from(cell.0.abs_diff(here_x)) + u32::from(cell.1.abs_diff(here_y));
    // 16b cut AI.1: clear of every hunt cell the side already holds.
    let clear = |cell: (u16, u16)| {
        taken
            .iter()
            .all(|other| other.0.abs_diff(cell.0).max(other.1.abs_diff(cell.1)) >= HUNT_FAN_ROWS)
    };
    let mut best: Option<(u32, (u16, u16))> = None;
    let mut read = 0_usize;
    for zone in def.entry_zones.iter().filter(|zone| zone.team != team) {
        let (x0, x1) = (zone.x0.min(zone.x1), zone.x0.max(zone.x1));
        let (y0, y1) = (zone.y0.min(zone.y1), zone.y0.max(zone.y1));
        let bounds = (x0, x1, y0, y1);
        let (cx, cy) = (here_x.clamp(x0, x1), here_y.clamp(y0, y1));
        // E.4's candidate: the corner, else the nearest passable cell to the
        // unit's own cell.
        let first = if passable(cx, cy) {
            Some((cx, cy))
        } else {
            nearest_passable_in_zone(&passable, bounds, cy, (here_x, here_y), &mut read)
        };
        let mut chosen = first;
        if first.is_some_and(|cell| !clear(cell)) {
            // The fan: the first fanned row whose cell is clear, the scan
            // budget shared; E.4's candidate when it runs out.
            for row in fan_rows(cy, y0, y1).skip(1) {
                if read > MAX_ENTRY_SCAN {
                    break;
                }
                let candidate = if passable(cx, row) {
                    Some((cx, row))
                } else {
                    nearest_passable_in_zone(&passable, bounds, row, (here_x, row), &mut read)
                };
                if let Some(cell) = candidate.filter(|cell| clear(*cell)) {
                    chosen = Some(cell);
                    break;
                }
            }
        }
        if let Some(cell) = chosen {
            let distance = reach(cell);
            if best.is_none_or(|(far, _)| distance < far) {
                best = Some((distance, cell));
            }
        }
        if read > MAX_ENTRY_SCAN {
            break;
        }
    }
    best.map(|(_, cell)| cell)
}

// ---------------------------------------------------------------------------
// A principal's goal: the fan, the cadence and the step (17b cut M1a.4)
// ---------------------------------------------------------------------------

impl Arena {
    /// **A goal fanned off a same-side hull's arrival square** (17b cut M1a.4;
    /// the M1a plan's departure (d); 16b cut AI.1's [`HUNT_FAN_ROWS`]), for the
    /// goals that land through `IntentKind::Goal`.
    ///
    /// [`Arena::far_entry_cell`]'s fan applied once more, at goal landing: when
    /// `cell` lies inside an entry zone of **another** side, and another hull
    /// of `unit`'s own side holds a motor goal within [`HUNT_FAN_ROWS`]
    /// (Chebyshev) of it, the answer is the first row of the zone's fan
    /// (`fan_rows` from `cell`'s own row, that row skipped) whose cell in
    /// `cell`'s column is passable to the hull's own cost row — else the zone's
    /// nearest passable cell on that row — and clear of every goal the side
    /// holds; when the fan runs out (or the scan budget [`MAX_ENTRY_SCAN`] is
    /// spent), `cell` itself. Anywhere else, `cell` unchanged.
    ///
    /// Two hulls cannot share an arrival square — physical knowledge the motor
    /// owns and a pilot's map or a mind's view does not carry — and two pilots
    /// of one side who both `DRIVE` the far zone's road corner would otherwise
    /// be handed one cell, and the first there would turn back into the second.
    /// The reflex controller's hunt kept its own fan in its decision half
    /// ([`Arena::far_entry_cell`] with the side's hunt cells), untouched, until
    /// 17b cut M4.6 deleted it; since then this is the one fan a hunt meets.
    ///
    /// It reads the ground, the entry zones, the hull's own team and cost row,
    /// and the **goals** of its own side's motor rows — never another hull's
    /// pose, and nothing of any other side.
    #[must_use]
    pub fn fan_goal(&self, unit: ObjId, cell: (u16, u16)) -> (u16, u16) {
        let Some(me) = self.unit(unit) else {
            return cell;
        };
        let Some(zone) = self.definition.entry_zones.iter().find(|zone| {
            zone.team != me.team
                && (zone.x0.min(zone.x1)..=zone.x0.max(zone.x1)).contains(&cell.0)
                && (zone.y0.min(zone.y1)..=zone.y0.max(zone.y1)).contains(&cell.1)
        }) else {
            return cell;
        };
        let in_zone = |at: (u16, u16)| {
            (zone.x0.min(zone.x1)..=zone.x0.max(zone.x1)).contains(&at.0)
                && (zone.y0.min(zone.y1)..=zone.y0.max(zone.y1)).contains(&at.1)
        };
        // Continue an already assigned lane within its original zone. This
        // survives terminal arrival; otherwise the next patrol waypoint would
        // put the leading hunter straight back in front of the following one.
        // Only own assignment and public ground are read. Invalid or out-of-
        // zone offsets fall back to the ordinary bounded fan below.
        let cell = self
            .motors
            .row(unit)
            .and_then(|row| row.fan)
            .filter(|fan| in_zone(fan.origin))
            .and_then(|fan| fan.shifted(cell))
            .filter(|at| in_zone(*at))
            .filter(|at| {
                self.definition
                    .terrain
                    .passable_for(me.cost_row, at.0, at.1)
            })
            .unwrap_or(cell);
        // Every goal the rest of the side holds, in unit-id order (the table's
        // own); a goal is a fact about ground, never a pose.
        let taken: Vec<(u16, u16)> = self
            .motors
            .rows()
            .filter(|(id, _)| *id != unit)
            .filter(|(id, _)| self.unit(*id).is_some_and(|other| other.team == me.team))
            .filter_map(|(_, row)| row.goal)
            .collect();
        let clear = |at: (u16, u16)| {
            taken
                .iter()
                .all(|other| other.0.abs_diff(at.0).max(other.1.abs_diff(at.1)) >= HUNT_FAN_ROWS)
        };
        if clear(cell) {
            return cell;
        }
        let passable = |x: u16, y: u16| self.definition.terrain.passable_for(me.cost_row, x, y);
        let (x0, x1) = (zone.x0.min(zone.x1), zone.x0.max(zone.x1));
        let (y0, y1) = (zone.y0.min(zone.y1), zone.y0.max(zone.y1));
        let mut read = 0_usize;
        for row in fan_rows(cell.1, y0, y1).skip(1) {
            if read > MAX_ENTRY_SCAN {
                break;
            }
            let candidate = if passable(cell.0, row) {
                Some((cell.0, row))
            } else {
                nearest_passable_in_zone(&passable, (x0, x1, y0, y1), row, (cell.0, row), &mut read)
            };
            if let Some(fanned) = candidate.filter(|at| clear(*at)) {
                return fanned;
            }
        }
        cell
    }

    /// **The stagger grid's next slot for `unit` at `cadence`** (16b cut U.1's
    /// formula, lifted by 17b cut M1a.4 so the reflex tick and the motor tick
    /// shared it; the motor tick and a mind's first think, `Arena::adopt_mind`,
    /// since 17b cut M4.6 deleted the reflex tick): the next multiple of `cadence` offset by
    /// [`crate::arena::rng::stagger_ms`]`(seed, unit, cadence)` that is at or
    /// after `now_ms`. A function of arena time, the seed and the unit id
    /// alone, so a hull put on a cadence mid-match lands where one put on it
    /// at check-out would, and the field stays spread.
    #[must_use]
    pub(crate) fn grid_at(&self, unit: ObjId, cadence: i64) -> i64 {
        let cadence = cadence.max(1);
        let offset = crate::arena::rng::stagger_ms(self.seed, unit, cadence);
        let mut at = self.now_ms - self.now_ms.rem_euclid(cadence) + offset;
        if at < self.now_ms {
            at = at.saturating_add(cadence);
        }
        at
    }

    /// **Put a hull on the motor's cadence** (17b cut M1a.4; D-UM-3): one
    /// [`DueKind::MotorTick`] at the hull's next slot of the stagger grid at
    /// `reflex.decide_ms` ([`Arena::grid_at`]), unless the heap already holds
    /// one for it — so a pilot who types `DRIVE` five times is driven by one
    /// tick, and a goal re-named while a tick is pending rides that tick.
    ///
    /// A wreck is put on nothing. (Nor was a reflex hull, whose drive ran
    /// inside its own reflex tick, until 17b cut M4.6 deleted the reflex
    /// controller and this guard's half for it.)
    pub(crate) fn engage_motor(&mut self, unit: ObjId) {
        if self.unit(unit).is_none_or(|state| state.limits.wreck) {
            return;
        }
        if self
            .due
            .iter()
            .any(|event| event.kind == DueKind::MotorTick { unit })
        {
            return;
        }
        let at = self.grid_at(unit, i64::from(self.arena_cfg.reflex.decide_ms));
        self.due.schedule(at, DueKind::MotorTick { unit });
    }

    /// **One step of a principal's goal** (17b cut M1a.4; doc 17 §4 D-UM-3,
    /// D-UM-10) — the body [`DueKind::MotorTick`] runs every `decide_ms`, and
    /// that a goal's landing runs once at the landing millisecond (*a new goal
    /// takes effect at N+1 and does not wait for the next motor tick*).
    ///
    /// For a hull that exists, is no wreck (and, until 17b cut M4.6 deleted
    /// it, was not the reflex controller's) and whose motor row holds a goal —
    /// and that can move: a hull that cannot
    /// ([`UnitState::cannot_close`]) has its goal dropped and is told
    /// [`crate::arena::perception::DriveOutcome::NoRoute`], once — in the
    /// order the reflex tick drove one:
    ///
    /// 1. `Arena::refresh_held_avoid` — a hull still held on an occupied
    ///    cell re-stamps it, before the sweep;
    /// 2. [`MotorRow::age_avoid`] — the TTL sweep a row no decision runs needs,
    ///    or a pilot's remembered stops would never expire;
    /// 3. `Arena::follow` under [`Fallback::Notice`], the row taken out of the
    ///    table for the drive as the reflex tick took it;
    /// 4. [`Arena::apply_motor_plan`] — the heading and the speed it decided,
    ///    through the pilot's own door at the hull's current epochs;
    /// 5. the notice: [`crate::arena::perception::DriveOutcome::Arrived`] when
    ///    the drive arrived and
    ///    [`crate::arena::perception::DriveOutcome::NoRoute`] when no route
    ///    reached the goal — once each, because under `Notice` both drop the
    ///    goal and the next tick finds none.
    ///
    /// Answers the drive's [`MotorOutcome`]; the default for a hull it does not
    /// drive. Draws nothing and journals nothing.
    pub(crate) fn motor_step(
        &mut self,
        unit: ObjId,
        t_ms: i64,
        counters: &mut MotorCounters,
    ) -> MotorOutcome {
        let Some(state) = self.unit(unit) else {
            return MotorOutcome::default();
        };
        if state.limits.wreck {
            return MotorOutcome::default();
        }
        let here = self.definition.terrain.cell_of(&state.pose);
        let cannot_close = state.cannot_close();
        let Some(goal) = self.motors.row(unit).and_then(|motor| motor.goal) else {
            return MotorOutcome::default();
        };
        // A completion names the principal's request, never the private fan.
        // Older direct motor fixtures have only an active goal; adopt it here.
        let requested = {
            let motor = self.motors.ensure(unit);
            *motor.requested_goal.get_or_insert(goal)
        };
        // A hull that cannot move — an engine killed under a standing goal
        // (`Consequence::DisableMotion` leaves no wreck), or one authored with
        // no speed — reaches no goal, and its pilot cannot let go of one: the
        // cockpit's and the arena's motion gates refuse `DRIVE OFF` and `DRIVE
        // HOLD` on it. So the motor lets go itself, once, and says so in the
        // no-route words — the tank holds where it is — instead of driving a
        // goal it never reaches every `decide_ms` for the rest of the match.
        if cannot_close {
            self.motors.forget_goal(unit);
            let motor = self.motors.ensure(unit);
            motor.requested_goal = Some(requested);
            motor.outcome = Some(DriveOutcome::NoRoute);
            self.stage_notice(
                unit,
                FrameNotice::Drive {
                    outcome: DriveOutcome::NoRoute,
                },
            );
            return MotorOutcome {
                arrived: false,
                unreachable: Some(goal),
            };
        }
        self.refresh_held_avoid(unit, t_ms);
        let mut motors = std::mem::take(&mut self.motors);
        let mut motor = motors.take(unit);
        let _ = motor.age_avoid(t_ms);
        let mut plan = MotorPlan::default();
        let outcome = self.follow(
            unit,
            t_ms,
            &mut motors,
            &mut motor,
            here,
            Fallback::Notice,
            counters,
            &mut plan,
        );
        if outcome.arrived {
            motor.outcome = Some(DriveOutcome::Arrived);
        } else if outcome.unreachable.is_some() {
            motor.outcome = Some(DriveOutcome::NoRoute);
        }
        motors.put(unit, motor);
        self.motors = motors;
        self.apply_motor_plan(unit, &plan, counters);
        if outcome.arrived {
            self.stage_notice(
                unit,
                FrameNotice::Drive {
                    outcome: DriveOutcome::Arrived,
                },
            );
        } else if outcome.unreachable.is_some() {
            self.stage_notice(
                unit,
                FrameNotice::Drive {
                    outcome: DriveOutcome::NoRoute,
                },
            );
        }
        outcome
    }

    /// **Put a drive's plan through the pilot's own door** (17b cut M1a.4):
    /// the heading, then the speed, each one an `Intent` on the channel that
    /// carries it at the hull's **current** epoch, applied through
    /// [`apply_intent`] as [`IntentSource::Machine`] — exactly the way
    /// `Arena::order` applied the reflex controller's two continuous kinds
    /// until 17b cut M4.6 deleted both, so
    /// the motor does nothing a pilot could not and a refused order (a cold or
    /// engine-killed hull) refuses here as it would there. A machine order
    /// never counts on the seat's rate and never takes the helm back from the
    /// goal it is driving. **17b cut M1a.5** folded the loop into
    /// `Arena::apply_machine_intents`: the two continuous kinds are handed over
    /// at the epochs the table holds now, and the answer is not read.
    pub(crate) fn apply_motor_plan(
        &mut self,
        unit: ObjId,
        plan: &MotorPlan,
        counters: &mut MotorCounters,
    ) {
        let mut orders: Vec<IntentKind> = Vec::new();
        if let Some(mdeg) = plan.heading_mdeg {
            orders.push(IntentKind::Heading { mdeg });
        }
        if let Some(cms) = plan.speed_cms {
            orders.push(IntentKind::Speed { cms });
        }
        let epochs = Channel::ALL.map(|channel| (channel, self.epochs.current(unit, channel)));
        let epoch_of = |channel: Channel| {
            epochs
                .iter()
                .find(|(on, _)| *on == channel)
                .map_or(0, |(_, epoch)| *epoch)
        };
        // Continuous: written straight onto the hull, never queued, so the
        // millisecond is read by nothing — the motor's own `now_ms` is handed
        // over for the shape's sake.
        let now_ms = self.now_ms;
        let _ = self.apply_machine_intents(
            unit,
            &orders,
            now_ms,
            &epoch_of,
            IntentSource::Machine,
            counters,
        );
    }
}

/// **The speed a machine orders while it is closing on its goal** (16b cut E.3;
/// 16c `B-42`), so that it arrives there instead of driving through it.
///
/// Before this cut [`Arena::follow`] ordered `max_speed_cms` flat until the unit
/// was inside the one-cell arrive window and then ordered zero. A stock tank
/// needs 56 m to stop from 15 m/s at 2 m/s², and the controller decides once a
/// second, so the machines coasted through their goals: through the firing stop
/// 16b cut E.1 had just priced, and — for a withdrawing tank — past its own entry
/// zone and out the far side, restarting its rally clock.
///
/// The profile is the classical one, in whole centimetres and `u64` throughout
/// ([`u64::isqrt`]; doc 16 §3.1's D-16-4 forbids a float in arena state and this
/// is arena arithmetic):
///
/// ```text
/// d      = max(0, range(pose, goal cell centre) − ARRIVE_CELLS × cell_cm)
/// d_eff  = max(0, d − |speed_cms| × decide_ms / 1000)
/// v_stop = isqrt(2 × brake_cms2 × d_eff)
/// order  = min(max_speed_cms, v_stop)
/// ```
///
/// `d` is measured to the **goal** and not to the route's next cell, because the
/// route is consumed cell by cell and its end *is* the goal: braking for the next
/// cell would crawl the whole way. `d_eff` is the one-tick lookahead — where the
/// unit will be when the controller next gets to think — without which a profile
/// sampled at 1 Hz orders a speed the tank has already outrun by the time it
/// obeys it. A unit with no braking rate at all orders `max_speed_cms`, which is
/// the pre-cut behaviour and the only honest answer for a platform that cannot
/// slow down.
///
/// The range is planar: the goal cell's own elevation is not read, so a goal up a
/// hill is not braked for a few metres early. That is the same simplification
/// [`Arena::stand_off_cell`] makes, and the arrive window is a cell wide.
fn braking_speed_cms(unit: &UnitState, goal: (u16, u16), cell_cm: i64, decide_ms: i64) -> i32 {
    let ceiling = u64::from(unit.mobility.max_speed_cms);
    let brake = u64::from(unit.mobility.brake_cms2);
    let capped = i32::try_from(ceiling).unwrap_or(i32::MAX);
    if brake == 0 {
        return capped;
    }
    let side = cell_cm.max(1);
    let to_x = i64::from(goal.0)
        .saturating_mul(side)
        .saturating_add(side / 2);
    let to_y = i64::from(goal.1)
        .saturating_mul(side)
        .saturating_add(side / 2);
    let range = range_between(
        unit.pose.x_cm,
        unit.pose.y_cm,
        unit.pose.z_cm,
        to_x,
        to_y,
        unit.pose.z_cm,
    );
    let window = u64::from(ARRIVE_CELLS).saturating_mul(u64::try_from(side).unwrap_or(1));
    let distance = range.saturating_sub(window);
    let ahead = u64::from(unit.speed_cms.unsigned_abs())
        .saturating_mul(u64::try_from(decide_ms.max(0)).unwrap_or(0))
        / 1_000;
    let effective = distance.saturating_sub(ahead);
    let stopping = brake.saturating_mul(2).saturating_mul(effective).isqrt();
    i32::try_from(ceiling.min(stopping)).unwrap_or(i32::MAX)
}

/// **How far ahead a routeless machine must be able to go** (16b cut M.2, for
/// [`Arena::hold_routeless`]), centimetres: the distance it needs to stop from
/// the fastest it can be going at the end of the next substep, plus what it
/// covers in that substep before the pre-pass asks again.
///
/// ```text
/// v      = max(0, speed_cms)
/// v_next = min(v + accel_cms2 × substep_ms / 1000, max(v, desired_speed_cms))
/// reach  = v_next² / (2 × brake_cms2) + v_next × substep_ms / 1000
/// ```
///
/// `v_next` bounds the speed from above: a unit ordered faster than it is going
/// gains at most one substep of acceleration, and one ordered slower is already
/// braking. The travel term is at the same bound, so it also covers what the
/// integration's per-substep speed update adds over the continuous stopping
/// distance. Integer arithmetic throughout (doc 16 §3.1's D-16-4). A platform
/// with no braking rate answers nought — it cannot stop in any distance, so the
/// look-ahead has nothing to buy it, which is [`braking_speed_cms`]'s rule — and
/// so does one moving backwards, whose way ahead is not on its heading.
fn stopping_reach_cm(unit: &UnitState, substep_ms: i64) -> i64 {
    let brake = i64::from(unit.mobility.brake_cms2);
    if brake == 0 {
        return 0;
    }
    let speed = i64::from(unit.speed_cms.max(0));
    let gained = speed.saturating_add(
        i64::from(unit.mobility.accel_cms2).saturating_mul(substep_ms.max(0)) / 1_000,
    );
    let next = gained.min(speed.max(i64::from(unit.desired_speed_cms)));
    next.saturating_mul(next) / brake.saturating_mul(2)
        + next.saturating_mul(substep_ms.max(0)) / 1_000
}

// ---------------------------------------------------------------------------
// The turning circle (16b cut AI.1; 16c `B-84`)
// ---------------------------------------------------------------------------
//
// A unit that turns at `turn_rate_mdeg_s` while it drives at `v` runs on a
// circle of radius `r = v × MDEG_PER_RADIAN / turn_rate_mdeg_s`, tangent to its
// heading at its pose (doc 16 §3.4 steps 1 and 3). A point at range `d` whose
// bearing is `β` off the heading lies on the circle on its own side when
// `d = 2 r sin β` — the chord of a circle through the tangent point — and
// **inside** it, where no turn at this speed reaches it, when `d < 2 r sin β`.
// Straight ahead (`β = 0`) nothing is inside, and straight behind (`β = 180°`)
// nothing is either: a machine can always drive on, or turn round.
//
// Written without an angle or a root: with `H` the heading (clockwise from
// north, `+y`), `(dx, dy)` the offset to the point and `s = sin H`,
// `c = cos H` in `crate::arena::motion`'s fixed point (`SIN_ONE = 2^20`), the
// cross product `|dx·c − dy·s|` is `d · sin β · SIN_ONE`, and multiplying the
// test through by `d` gives `d² · SIN_ONE < 2 r · |dx·c − dy·s|`. The dot product
// `dx·s + dy·c` is `d · cos β · SIN_ONE`, whose sign says whether the point is
// ahead of the beam. All `i128`: a window of 256 cells of 1,000 cm puts `d²`
// near `10^11` and `r · cross` near `10^16`, past what an `i64` product of the
// two would hold once `SIN_ONE` is in it. No float, no `bearing_of` and no
// `arc_between` anywhere below: these decide, and doc 16 §3.1's D-16-4 keeps a
// float out of every decision that can be made without one.

/// **The radius of the circle this unit will be turning on by its next tick**,
/// centimetres (16b cut AI.1).
///
/// ```text
/// v_look = min(max_speed_cms, |speed_cms| + accel_cms2 × decide_ms / 1000)
/// r_cm   = v_look × MDEG_PER_RADIAN / turn_rate_mdeg_s      (0 when the rate is 0)
/// ```
///
/// `v_look` is the one-tick look-ahead [`braking_speed_cms`] takes: the circle
/// the machine can be on by the time it next thinks, never smaller than the one
/// it is on now. A machine that cannot turn has no circle to orbit, and answers
/// nought. Unsigned and saturating throughout.
fn turning_radius_cm(unit: &UnitState, decide_ms: i64) -> u64 {
    let rate = u64::from(unit.mobility.turn_rate_mdeg_s);
    if rate == 0 {
        return 0;
    }
    let gained = u64::from(unit.mobility.accel_cms2)
        .saturating_mul(u64::try_from(decide_ms.max(0)).unwrap_or(0))
        / 1_000;
    let look = u64::from(unit.speed_cms.unsigned_abs())
        .saturating_add(gained)
        .min(u64::from(unit.mobility.max_speed_cms));
    look.saturating_mul(MDEG_PER_RADIAN) / rate
}

/// The offset from `pose` to `point`, and the heading's fixed-point sine and
/// cosine, all `i128` — the four numbers the three tests below are made of.
fn circle_terms(pose: &mush_sim::data::Pose, point: (i64, i64)) -> (i128, i128, i128, i128) {
    use crate::arena::motion::{cos_fx, sin_fx};
    (
        i128::from(point.0) - i128::from(pose.x_cm),
        i128::from(point.1) - i128::from(pose.y_cm),
        i128::from(sin_fx(pose.heading_mdeg)),
        i128::from(cos_fx(pose.heading_mdeg)),
    )
}

/// **Whether `point` is inside the turning circle of radius `radius_cm`** on
/// its own side of the heading (16b cut AI.1): `(dx² + dy²) × SIN_ONE < 2 ×
/// r × |dx·c − dy·s|` — `d < 2 r sin β` multiplied through by `d`. Nothing
/// straight ahead or straight behind is ever inside, and with a radius of
/// nought nothing is. The point is taken at the unit's own height.
fn inside_turning_circle(pose: &mush_sim::data::Pose, radius_cm: u64, point: (i64, i64)) -> bool {
    use crate::arena::motion::SIN_ONE;
    let (dx, dy, s, c) = circle_terms(pose, point);
    let cross = (dx * c - dy * s).abs();
    let square = (dx * dx + dy * dy).saturating_mul(i128::from(SIN_ONE));
    square
        < i128::from(radius_cm)
            .saturating_mul(2)
            .saturating_mul(cross)
}

/// **Whether `point` is ahead of the beam** (16b cut AI.1): `dx·s + dy·c ≥ 0`,
/// the point's bearing within ninety degrees of the heading either way. A point
/// exactly abeam is ahead.
fn ahead_of_beam(pose: &mush_sim::data::Pose, point: (i64, i64)) -> bool {
    let (dx, dy, s, c) = circle_terms(pose, point);
    dx * s + dy * c >= 0
}

/// **The speed at which `point` stops being inside the turning circle**,
/// cm/s (16b cut AI.1): the speed whose circle just reaches it —
///
/// ```text
/// v_turn = (dx² + dy²) × SIN_ONE × turn_rate_mdeg_s / (2 × MDEG_PER_RADIAN × |dx·c − dy·s|)
/// ```
///
/// — floored at 1 cm/s, so a machine told to converge is never told to stop.
/// Asked only of a point [`inside_turning_circle`] said is inside, which has a
/// cross product above nought; one that is not (straight ahead or behind)
/// answers `u64::MAX`, no cap at all. Floored in the division, so the circle of
/// the answer does not contain the point and the circle of one cm/s more does
/// (to within the radius's own rounding).
fn turn_speed_cms(pose: &mush_sim::data::Pose, turn_rate_mdeg_s: u32, point: (i64, i64)) -> u64 {
    use crate::arena::motion::SIN_ONE;
    let (dx, dy, s, c) = circle_terms(pose, point);
    let cross = (dx * c - dy * s).abs();
    if cross == 0 {
        return u64::MAX;
    }
    let numerator = (dx * dx + dy * dy)
        .saturating_mul(i128::from(SIN_ONE))
        .saturating_mul(i128::from(turn_rate_mdeg_s));
    let denominator = i128::from(MDEG_PER_RADIAN)
        .saturating_mul(2)
        .saturating_mul(cross);
    u64::try_from(numerator / denominator)
        .unwrap_or(u64::MAX)
        .max(1)
}

/// The centre of `cell`, centimetres, on a window of `cell_cm` cells.
fn cell_centre(cell: (u16, u16), cell_cm: i64) -> (i64, i64) {
    (
        i64::from(cell.0)
            .saturating_mul(cell_cm)
            .saturating_add(cell_cm / 2),
        i64::from(cell.1)
            .saturating_mul(cell_cm)
            .saturating_add(cell_cm / 2),
    )
}

/// Whether two cells are the same cell or 8-neighbours (Chebyshev one).
fn adjacent(a: (u16, u16), b: (u16, u16)) -> bool {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1)) <= 1
}

/// **The hunt's fanned rows** (16b cut AI.1; [`Arena::far_entry_cell`]'s fan):
/// `cy`, then `cy + s`, `cy − s`, `cy + 2s`, `cy − 2s`, … with `s` =
/// [`HUNT_FAN_ROWS`], every row outside `y0..=y1` skipped, and nothing once
/// both ends have left the zone. `cy` is the clamped row and inside the zone,
/// so it comes first; every row after it is a whole number of steps from it on
/// one side or the other, so the rows are distinct and any two of them at
/// least `s` apart. Integer only, and at most `span / s + 1` steps long.
pub(crate) fn fan_rows(cy: u16, y0: u16, y1: u16) -> impl Iterator<Item = u16> {
    let step = u32::from(HUNT_FAN_ROWS.max(1));
    let steps = u32::from(y1.saturating_sub(y0)) / step + 1;
    std::iter::once(cy).chain((1..=steps).flat_map(move |n| {
        let apart = n.saturating_mul(step);
        [
            u32::from(cy).checked_add(apart),
            u32::from(cy).checked_sub(apart),
        ]
        .into_iter()
        .filter_map(move |row| {
            row.and_then(|row| u16::try_from(row).ok())
                .filter(|row| (y0..=y1).contains(row))
        })
    }))
}

/// **The zone's nearest passable cell to `from`**, by Manhattan distance,
/// scanning the zone `(x0, x1, y0, y1)`'s rows nearest `row` first — `row`,
/// then one either side, then two — and every row's cells from `x0`
/// ([`Arena::far_entry_cell`]'s scan, 16b cut E.4, one copy since 16b cut
/// AI.1 reads it for each fanned row). `read` is the budget
/// [`MAX_ENTRY_SCAN`] shared across the call: past it the scan stops and the
/// best found so far is answered. Ties go to the cell scanned first.
fn nearest_passable_in_zone(
    passable: &impl Fn(u16, u16) -> bool,
    (x0, x1, y0, y1): (u16, u16, u16, u16),
    row: u16,
    from: (u16, u16),
    read: &mut usize,
) -> Option<(u16, u16)> {
    let reach = |x: u16, y: u16| u32::from(x.abs_diff(from.0)) + u32::from(y.abs_diff(from.1));
    let mut found: Option<(u32, (u16, u16))> = None;
    let span = u32::from(y1.saturating_sub(y0));
    'zone: for offset in 0..=span {
        for below in [true, false] {
            if !below && offset == 0 {
                // The starting row is read once, not twice.
                continue;
            }
            let at = if below {
                u32::from(row).checked_sub(offset)
            } else {
                u32::from(row).checked_add(offset)
            };
            let Some(Ok(y)) = at.map(u16::try_from) else {
                continue;
            };
            if y < y0 || y > y1 {
                continue;
            }
            for x in x0..=x1 {
                *read = read.saturating_add(1);
                if *read > MAX_ENTRY_SCAN {
                    break 'zone;
                }
                if !passable(x, y) {
                    continue;
                }
                let distance = reach(x, y);
                if found.is_none_or(|(far, _)| distance < far) {
                    found = Some((distance, (x, y)));
                }
            }
        }
    }
    found.map(|(_, cell)| cell)
}

/// **The 8-neighbour step nearest a bearing** (16b cut Q.7).
///
/// Bearing zero is north and [`bearing_of`] measures clockwise, so the eight
/// octants are 45 degrees wide and centred on the compass points: the octant is
/// `(mdeg + 22,500) / 45,000` modulo eight, and the table below is the step each
/// one names in cell coordinates (`+y` north, `+x` east).
fn octant_step(heading_mdeg: u32) -> (i32, i32) {
    const STEPS: [(i32, i32); 8] = [
        (0, 1),
        (1, 1),
        (1, 0),
        (1, -1),
        (0, -1),
        (-1, -1),
        (-1, 0),
        (-1, 1),
    ];
    let octant = ((heading_mdeg.saturating_add(22_500) / 45_000) % 8) as usize;
    STEPS.get(octant).copied().unwrap_or((0, 1))
}

/// **What the edge rule makes of one step** (16b cut AI.1): see
/// [`edge_verdict`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgeVerdict {
    /// Crossable.
    Open,
    /// Too steep: the edge's own slope either way, or a shoulder whose leg is a
    /// cliff ([`ShoulderVerdict::TooSteep`]) — a slope refusal.
    TooSteep,
    /// A diagonal neither of whose shoulders is open
    /// ([`ShoulderVerdict::Closed`]) — a corner refusal.
    Corner,
}

/// **Whether one edge is crossable for this unit, and if not why** (16b cut
/// Q.7's `edge_is_open`; the verdict 16b cut AI.1's), which is
/// [`crate::arena::motion`]'s step 4 and [`crate::arena::path::path_with`]'s edge
/// rule asked of a single step — with the unit's own row for the shoulders
/// since 16b cut AE.1, and its own avoid list since 16b cut AI.1, as the search
/// asks it.
///
/// A unit with no slope limit is refused nothing here, exactly as its search
/// refuses it nothing and reads no shoulder: [`EdgeVerdict::Open`]. Under a
/// limit, the edge's own steepness either way first ([`EdgeVerdict::TooSteep`]),
/// then, on a diagonal, [`shoulder_refuses`]' verdict: a steep shoulder is
/// [`EdgeVerdict::TooSteep`] and a corner with no open shoulder
/// [`EdgeVerdict::Corner`].
fn edge_verdict(
    terrain: &ArenaTerrain,
    unit: &UnitState,
    here_cell: &crate::arena::state::TerrainCell,
    to_cell: &crate::arena::state::TerrainCell,
    here: (u16, u16),
    to: (u16, u16),
    avoid: &[(u16, u16)],
) -> EdgeVerdict {
    let Some(limit) = unit.slope_limit_pct else {
        return EdgeVerdict::Open;
    };
    let cell_cm = u32::from(terrain.cell_cm());
    let Some(kind) = slope::step_kind(here, to) else {
        return EdgeVerdict::Open;
    };
    if slope::too_steep(
        Some(limit),
        slope::steepness_pct(here_cell, to_cell, kind, cell_cm),
    ) {
        return EdgeVerdict::TooSteep;
    }
    if kind != mush_atlas::physics::StepKind::Diagonal {
        return EdgeVerdict::Open;
    }
    match shoulder_refuses(
        terrain,
        Some(limit),
        unit.cost_row,
        here_cell,
        to_cell,
        here,
        to,
        avoid,
    ) {
        ShoulderVerdict::Passes => EdgeVerdict::Open,
        ShoulderVerdict::TooSteep => EdgeVerdict::TooSteep,
        ShoulderVerdict::Closed => EdgeVerdict::Corner,
    }
}

/// **Whether one edge is crossable for this unit** (16b cut Q.7): whether
/// [`edge_verdict`] answers [`EdgeVerdict::Open`].
fn edge_is_open(
    terrain: &ArenaTerrain,
    unit: &UnitState,
    here_cell: &crate::arena::state::TerrainCell,
    to_cell: &crate::arena::state::TerrainCell,
    here: (u16, u16),
    to: (u16, u16),
    avoid: &[(u16, u16)],
) -> bool {
    edge_verdict(terrain, unit, here_cell, to_cell, here, to, avoid) == EdgeVerdict::Open
}

/// Everything a unit-level test of the motor needs to reach.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::doctrine::MachineState;
    use crate::arena::state::{Controller, PartyId};
    use crate::arena::test_support::{arena_of, pose, tank};

    #[test]
    fn m5r_patrol_waits_for_the_requested_waypoints_completion() {
        let def = crate::arena::test_support::definition();
        let walk = Walk {
            waypoint: Some(0),
            from_waypoint: None,
            goal: Some((1, 4)),
        };
        let nearby = patrol_step(&def, 4, Some((1, 4)), false, walk);
        assert_eq!(nearby.walk.waypoint, Some(0));
        assert_eq!(nearby.walk.goal, Some((1, 4)));
        assert!(!nearby.clear_route);
        let complete = patrol_step(&def, 4, Some((1, 4)), true, walk);
        assert_eq!(complete.walk.waypoint, Some(1));
        assert_eq!(complete.walk.from_waypoint, Some(0));
        assert_eq!(complete.walk.goal, Some((6, 4)));
        assert!(complete.clear_route);
    }

    #[test]
    fn m5r_route_invalidation_keeps_the_principals_destination() {
        let unit = ObjId(4);
        let mut table = MotorTable::default();
        let row = table.ensure(unit);
        row.goal = Some((6, 5));
        row.requested_goal = Some((6, 4));
        row.route = vec![(3, 4), (4, 4)];
        row.searched_ms = Some(1_000);
        row.searched_from = Some((2, 4));
        table.forget_route(unit);
        let row = table.row(unit).expect("the same motor row");
        assert_eq!(row.goal, Some((6, 5)));
        assert_eq!(row.requested_goal, Some((6, 4)));
        assert!(row.route.is_empty());
        assert_eq!(row.searched_ms, None);
        assert_eq!(row.searched_from, None);
        table.forget_goal(unit);
        let row = table.row(unit).expect("disengagement keeps the row");
        assert_eq!(row.goal, None);
        assert_eq!(row.requested_goal, None);
        assert_eq!(row.outcome, None);
    }

    /// **The motor names no contact store and no fire line** (17b cut M1a.1;
    /// doc 17 §4's audit): this file's own text, comments included, holds none
    /// of the three readers a hull's driving must never consult — the contact
    /// store, the line a round would take, and the cover on it. The needles are
    /// built with `concat!` so this assertion's own source does not trip it.
    #[test]
    fn m1a1_motor_rs_names_no_contact_store_and_no_line_of_fire() {
        let text = include_str!("motor.rs");
        for needle in [
            concat!(".cont", "acts."),
            concat!("fire_", "line"),
            concat!("fire_", "cover"),
        ] {
            assert!(
                !text.contains(needle),
                "the motor reads {needle:?}, which no hull's driving may"
            );
        }
    }

    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn the_path_budget_is_a_window_of_arena_time_and_not_a_pulse() {
        let cfg = ReflexConfig {
            decide_ms: 1_000,
            max_path_queries: 2,
            ..ReflexConfig::default()
        };
        let mut motors = MotorTable::new();
        assert!(motors.may_search(0, &cfg));
        assert!(motors.may_search(400, &cfg));
        assert!(
            !motors.may_search(900, &cfg),
            "the window's budget is spent"
        );
        // The next window opens at 1,000 ms of arena time, whatever the tempo.
        assert!(motors.may_search(1_000, &cfg));
        assert!(motors.may_search(1_999, &cfg));
        assert!(!motors.may_search(1_999, &cfg));
    }

    /// **16b cut Q.7**: the remembered-stop list is bounded, refreshes rather
    /// than duplicating, and only an `Occupied` stop puts anything in it.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn the_avoided_cells_are_bounded_and_a_repeat_refreshes_its_stamp() {
        let mut arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(1_500, 1_500))]);
        // 17b cut M4.5a: a mind's hull, with the motor row a controller that
        // takes a hull over steers it through (`set_controller` lays none for a
        // mind: its first landing's would).
        arena.set_controller(ObjId(4), Controller::Mind, MachineState::Patrol);
        let _ = arena.motors.ensure(ObjId(4));
        // A search this tick already paid for, so the clearing below is a fact
        // about `note_blocked` and not about how a fresh row is laid.
        if let Some(row) = arena.motors.row_mut(ObjId(4)) {
            row.searched_ms = Some(0);
        }
        // Nine distinct cells into a list of eight: the oldest goes.
        for n in 0..9_u16 {
            arena.note_blocked(ObjId(4), (n, 1), BlockedCause::Occupied, i64::from(n) * 100);
        }
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.avoid.len(), REFLEX_AVOID_MAX);
        assert_eq!(row.avoid.first().map(|(cell, _)| *cell), Some((1, 1)));
        assert_eq!(row.avoid.last().map(|(cell, _)| *cell), Some((8, 1)));
        assert_eq!(row.searched_ms, None, "the next tick searches");

        // A repeat moves the stamp and takes no second slot.
        arena.note_blocked(ObjId(4), (1, 1), BlockedCause::Occupied, 9_000);
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.avoid.len(), REFLEX_AVOID_MAX);
        assert_eq!(row.avoid.first().copied(), Some(((1, 1), 9_000)));

        // **16b cut E.3 (16c `B-43`): the list evicts the LEAST RECENTLY STOPPED
        // AT and not the entry at index zero.** The two orders differ exactly
        // because a repeat refreshes a stamp in place: `(1, 1)` was the oldest
        // insert and is now the newest stop, so a list that evicted by insertion
        // order would drop the one cell the unit keeps being stopped at — the
        // wreck — and keep seven it drove past once. One more distinct cell
        // overflows the list: `(1, 1)` survives and `(2, 1)`, whose stamp is the
        // least, goes.
        arena.note_blocked(ObjId(4), (9, 1), BlockedCause::Occupied, 9_500);
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.avoid.len(), REFLEX_AVOID_MAX);
        let cells: Vec<(u16, u16)> = row.avoid.iter().map(|(cell, _)| *cell).collect();
        assert!(
            cells.contains(&(1, 1)),
            "the refreshed cell was evicted by its insertion order: {cells:?}"
        );
        assert!(
            !cells.contains(&(2, 1)),
            "the least recently stopped at survived: {cells:?}"
        );
        assert!(cells.contains(&(9, 1)), "the new stop is in: {cells:?}");

        // Every other cause remembers no cell: the ground is not going to move
        // out of the way, and the search already knows about it. **16b cut
        // AI.1** (16c `B-84`): what it does with the route depends on where the
        // stop was. The unit stands in (1, 1) and its route's next cell is (2, 2).
        for cause in [
            BlockedCause::Impassable,
            BlockedCause::Edge,
            BlockedCause::TooSteep,
        ] {
            // At the route's own next cell: the route is thrown away. No search
            // was ever spent from (1, 1), so no stop is remembered.
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.route = vec![(2, 2)];
                row.avoid.clear();
                row.searched_ms = Some(10_000);
                row.searched_from = None;
                row.refused = None;
                row.goal = Some((6, 4));
            }
            arena.note_blocked(ObjId(4), (2, 2), cause, 10_000);
            let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
            assert!(row.route.is_empty(), "{cause:?} throws the route away");
            assert!(row.avoid.is_empty(), "{cause:?} remembers nothing");
            assert_eq!(
                row.searched_ms,
                Some(10_000),
                "{cause:?} is ground, not traffic: the search it just paid for stands"
            );
            assert_eq!(
                row.refused, None,
                "{cause:?}: no search was spent from here"
            );

            // The same stop on a route planned from this very cell: remembered.
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.route = vec![(2, 2)];
                row.searched_from = Some((1, 1));
            }
            arena.note_blocked(ObjId(4), (2, 2), cause, 10_000);
            let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
            assert!(row.route.is_empty(), "{cause:?} throws the route away");
            assert!(row.avoid.is_empty(), "{cause:?} remembers no cell");
            assert_eq!(row.searched_ms, Some(10_000));
            assert_eq!(
                row.refused,
                Some(Refusal {
                    at: (1, 1),
                    cell: (2, 2),
                    cause,
                    goal: (6, 4),
                    until_ms: 10_000 + REFLEX_AVOID_TTL_MS,
                }),
                "{cause:?} at the next cell of a route planned from here"
            );

            // Beside the route — the next cell still a neighbour, the stop
            // somewhere else: the route is kept and nothing is remembered.
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.route = vec![(2, 2)];
                row.refused = None;
            }
            arena.note_blocked(ObjId(4), (3, 3), cause, 10_000);
            let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
            assert_eq!(
                row.route,
                vec![(2, 2)],
                "{cause:?} beside the route keeps it"
            );
            assert!(row.avoid.is_empty(), "{cause:?} remembers nothing");
            assert_eq!(row.searched_ms, Some(10_000));
            assert_eq!(row.refused, None, "{cause:?} beside the route");
        }

        // And a unit nothing is steering learns nothing at all. **17b cut
        // M1a.1**: the gate is the hull's motor row and no longer the
        // controller — a seat handed back to its pilot takes the row with it
        // (`set_controller` releases the motor row on every transition; 17b
        // cut M4.5a: from the mind as it did from the reflex), and a hull with
        // no row grows none here.
        arena.set_controller(ObjId(4), Controller::Pilot, MachineState::Withdraw);
        assert!(
            arena.motors.row(ObjId(4)).is_none(),
            "the seat handed back took its motor row with it"
        );
        arena.note_blocked(ObjId(4), (7, 7), BlockedCause::Occupied, 11_000);
        assert!(
            arena.motors.row(ObjId(4)).is_none(),
            "a hull nothing steers was given a row by its stop"
        );
    }

    /// **The pre-pass orders the one machine it is for, counts the order, and
    /// leaves everybody else alone** (16b cut E.3; 16c `B-44`).
    ///
    /// [`Arena::hold_routeless`] is five guards and one question, and every guard
    /// is load-bearing: the route test is what keeps a follower in a column out of
    /// it (16c `B-46` on the one ground the match uses), the row's presence (the
    /// controller test until 17b cut M1a.1) is what keeps it off a seated
    /// pilot's tank, and the power and speed tests are
    /// what keep it from ordering a stop onto a tank that is already standing
    /// still. None of them is reachable from an end-to-end fixture, because
    /// `Arena::follow`'s own tick-time check answers the same question a tick
    /// later and hides the difference; so each is asked here directly, of the
    /// function, with the count it is supposed to leave behind.
    ///
    /// The window is `test_support`'s eight by eight of passable ground, and the
    /// closed step is the machine's **own remembered stop** rather than a painted
    /// cliff: `first_step_is_open`'s `avoid` clause is the cheapest way to shut one
    /// cell without moving the ground under the other cases.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn e3_the_pre_pass_holds_a_routeless_machine_and_skips_everyone_else() {
        use crate::arena::state::Power;

        /// One case: build the field, let `edit` bend one thing about it, run the
        /// pre-pass once, and report what the unit was left ordering and what the
        /// pass counted.
        fn asked(edit: impl FnOnce(&mut Arena)) -> (i32, u32) {
            // Cell (3, 3) of a 1,000 cm window, facing north, which
            // `octant_step(0)` makes the step onto (3, 4).
            let mut arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(3_500, 3_500))]);
            if let Some(state) = arena.unit_mut(ObjId(4)) {
                state.power = Power::Online;
                state.desired_speed_cms = 800;
            }
            // 17b cut M4.5a: a mind's hull and the motor row it is steered
            // through (`set_controller` lays none for a mind).
            arena.set_controller(ObjId(4), Controller::Mind, MachineState::Patrol);
            let _ = arena.motors.ensure(ObjId(4));
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.goal = Some((3, 7));
                row.route.clear();
                row.avoid = vec![((3, 4), 0)];
            }
            edit(&mut arena);
            let mut counters = MotorCounters::default();
            arena.hold_routeless(&mut counters);
            (
                arena
                    .unit(ObjId(4))
                    .map_or(i32::MIN, |state| state.desired_speed_cms),
                counters.intents,
            )
        }

        // The case the pass exists for: a machine with a goal, no route, way on,
        // and the cell in front of it one it has just been stopped at.
        assert_eq!(
            asked(|_| {}),
            (0, 1),
            "a routeless machine pointed at a closed cell was not ordered to stand, or the order \
             was not counted"
        );
        // A machine whose route is not empty is a machine the search has already
        // checked the ground for — every follower in a column at the ford.
        assert_eq!(
            asked(|arena| {
                if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                    row.route = vec![(3, 4), (3, 5)];
                }
            }),
            (800, 0),
            "the pass took a machine that had a route"
        );
        // A machine with nowhere to be is not being driven anywhere.
        assert_eq!(
            asked(|arena| {
                if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                    row.goal = None;
                }
            }),
            (800, 0),
            "the pass took a machine with no goal"
        );
        // **A seated pilot's tank is theirs.** The controller does nothing a pilot
        // cannot, and it does nothing to a unit it is not flying. **17b cut
        // M1a.1**: what keeps the pass off it is that a seat handed back takes
        // its motor row with it (`set_controller` releases it on every
        // transition; 17b cut M4.5a: from the mind as it did from the reflex)
        // — the pass reads the rows and no longer the controller, so the seat
        // is handed back the way a reclaim hands it.
        assert_eq!(
            asked(|arena| {
                arena.set_controller(ObjId(4), Controller::Pilot, MachineState::Withdraw);
            }),
            (800, 0),
            "the pass ordered a seated pilot's tank to stop"
        );
        // A cold hull is not going anywhere and `apply_intent` would refuse the
        // order anyway; the pass does not pay for the map lookup to find out.
        assert_eq!(
            asked(|arena| {
                if let Some(state) = arena.unit_mut(ObjId(4)) {
                    state.power = Power::Starting;
                }
            }),
            (800, 0),
            "the pass took a hull whose power was not up"
        );
        // Already standing: nothing to order and nothing to count.
        assert_eq!(
            asked(|arena| {
                if let Some(state) = arena.unit_mut(ObjId(4)) {
                    state.desired_speed_cms = 0;
                }
            }),
            (0, 0),
            "the pass re-ordered a stop onto a machine that was already standing"
        );
        // And the question itself: with the cell in front open, the machine keeps
        // its way.
        assert_eq!(
            asked(|arena| {
                if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                    row.avoid.clear();
                }
            }),
            (800, 0),
            "the pass stopped a machine whose next cell was open"
        );
    }

    /// **16b cut U.1**: the follower's one-cell test counts a refusal by the
    /// **edge rule** in [`MotorCounters::slope_refusals`] — here an 8 m bluff
    /// in front of a unit with a 45 per cent limit, an 80 per cent edge — and a
    /// refusal by the machine's own remembered stop counts none: that cell is
    /// shut by the avoid list, not by the slope.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn u1_the_one_cell_test_counts_a_slope_refusal_and_not_a_remembered_stop() {
        use crate::arena::state::{ArenaTerrain, Power};
        use crate::arena::test_support::ground;

        /// Cell (3, 3) facing north onto (3, 4), as in the E.3 fixture; `bluff`
        /// raises (3, 4) by 800 cm and `remembered` puts it on the avoid list.
        fn asked(bluff: bool, remembered: bool) -> (i32, u32) {
            let mut arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(3_500, 3_500))]);
            if bluff {
                let mut cells = vec![ground(); 8 * 8];
                if let Some(slot) = cells.get_mut(4 * 8 + 3) {
                    slot.elev_cm = 800;
                }
                std::sync::Arc::make_mut(&mut arena.definition).terrain =
                    ArenaTerrain::new(8, 8, 1_000, cells).expect("the bluff window");
            }
            if let Some(state) = arena.unit_mut(ObjId(4)) {
                state.power = Power::Online;
                state.desired_speed_cms = 800;
                state.slope_limit_pct = Some(45);
            }
            // 17b cut M4.5a: a mind's hull and its motor row, as in E.3.
            arena.set_controller(ObjId(4), Controller::Mind, MachineState::Patrol);
            let _ = arena.motors.ensure(ObjId(4));
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.goal = Some((3, 7));
                row.route.clear();
                row.avoid = if remembered {
                    vec![((3, 4), 0)]
                } else {
                    Vec::new()
                };
            }
            let mut counters = MotorCounters::default();
            arena.hold_routeless(&mut counters);
            (
                arena
                    .unit(ObjId(4))
                    .map_or(i32::MIN, |state| state.desired_speed_cms),
                counters.slope_refusals,
            )
        }

        assert_eq!(
            asked(true, false),
            (0, 1),
            "the bluff in front did not hold the machine, or was not counted as a slope refusal"
        );
        assert_eq!(
            asked(false, true),
            (0, 0),
            "a remembered stop was counted as a slope refusal"
        );
        assert_eq!(
            asked(false, false),
            (800, 0),
            "level ground refused an edge"
        );
    }

    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn the_octant_of_a_bearing_is_the_cell_the_crossing_will_test() {
        for (mdeg, step) in [
            (0_u32, (0_i32, 1_i32)),
            (22_499, (0, 1)),
            (22_500, (1, 1)),
            (45_000, (1, 1)),
            (90_000, (1, 0)),
            (135_000, (1, -1)),
            (180_000, (0, -1)),
            (225_000, (-1, -1)),
            (270_000, (-1, 0)),
            (315_000, (-1, 1)),
            (359_999, (0, 1)),
        ] {
            assert_eq!(octant_step(mdeg), step, "{mdeg} mdeg");
        }
    }

    /// **16b cut I.3, restored by M5r**: PLACE invalidates the path without
    /// cancelling the principal's destination. The route, search stamp and old
    /// completion go; active/requested goals, decision, waypoints and avoid stay.
    ///
    /// **17b cut M4.5a**: the machine is a mind. Its decision is its mind row —
    /// the stock memory's `state` (`Approach`) and `target` (its own row's
    /// `ContactId`), the cached doctrine state, and the order it holds
    /// (`Advance (6, 6)`, re-keyed as the order's Mind arm writes it) — and
    /// the whole row is unchanged by `forget_route`, which stages nothing to a
    /// mind (the positive form of STOP F's premise: a placed mind is told
    /// nothing today). The path half is the hull's motor row, as it was.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn i3_forget_route_keeps_the_decision_and_drops_the_path() {
        use crate::arena::mind::{Memory, decode_memory, doctrine_state_of, encode_memory};
        use crate::arena::stock_mind::StockMemory;
        use crate::deliberation::order::{Constraints, Objective, Order, StandingOrder};
        let unit = ObjId(4);
        let mut arena = arena_of(vec![tank(unit, PartyId::RED, pose(1_500, 1_500))]);
        arena.set_controller(unit, Controller::Mind, MachineState::Patrol);
        let standing = StandingOrder::accepted(
            Order {
                squad: crate::deliberation::squad::SquadId {
                    team: PartyId::RED,
                    index: 1,
                },
                objective: Objective::Advance { to: (6, 6) },
                target: None,
                horizon_ms: 600_000,
                constraints: Constraints::default(),
                ttl_ms: 600_000,
                issued_ms: 0,
                issuer: None,
                by: crate::deliberation::order::CharacterRef::Persona {
                    squad: crate::deliberation::squad::SquadId {
                        team: crate::arena::state::PartyId::RED,
                        index: 1,
                    },
                },
                spot: None,
            },
            0,
        );
        let held = arena.rekey_order(unit, &standing.order, standing.expires_ms);
        let target = crate::arena::state::ContactId(0x0123_4567_89ab_cdef);
        // The decision on the mind row: the memory a think in `Approach` on
        // that target leaves, its cached state, and the order it holds.
        {
            let mind = arena.minds.get_mut(&unit).expect("the mind's row");
            mind.memory = encode_memory(&Memory::from(StockMemory {
                state: MachineState::Approach,
                target: Some(target),
                ..StockMemory::default()
            }));
            mind.doctrine_state = doctrine_state_of(&mind.memory);
            mind.order = Some(held);
        }
        // The path half on the hull's motor row (a mind's is its landings';
        // laid here for the fixture).
        {
            let row = arena.motors.ensure(unit);
            row.goal = Some((6, 6));
            row.requested_goal = Some((6, 6));
            row.outcome = Some(DriveOutcome::Arrived);
            row.fan = GoalFan::between((6, 3), (6, 6));
            row.route = vec![(2, 2), (3, 3)];
            row.searched_ms = Some(4_000);
            row.waypoint = Some(1);
            row.from_waypoint = Some(0);
            row.avoid = vec![((2, 1), 3_000)];
        }
        let mind_before = arena.minds.get(&unit).cloned().expect("the mind's row");
        arena.motors.forget_route(unit);
        let motor = arena.motors.row(unit).expect("the motor row");
        let mind = arena.minds.get(&unit).expect("the mind's row");
        assert_eq!(motor.goal, Some((6, 6)));
        assert_eq!(motor.requested_goal, Some((6, 6)));
        assert_eq!(motor.fan, None, "PLACE invalidates the old lane");
        assert_eq!(
            motor.outcome, None,
            "the old position's completion is invalid"
        );
        assert!(motor.route.is_empty());
        assert_eq!(motor.searched_ms, None, "the next tick searches at once");
        assert_eq!(mind, &mind_before, "the mind's row is untouched");
        assert_eq!(mind.doctrine_state, MachineState::Approach);
        assert_eq!(
            mind.order.as_ref().map(|order| order.objective),
            Some(crate::arena::perception::MindObjective::Advance { to: (6, 6) })
        );
        let memory = StockMemory::from(&decode_memory(&mind.memory));
        assert_eq!(memory.state, MachineState::Approach);
        assert_eq!((motor.waypoint, motor.from_waypoint), (Some(1), Some(0)));
        assert_eq!(memory.target, Some(target));
        assert_eq!(motor.avoid, vec![((2, 1), 3_000)]);
        // A unit with no machine is left alone.
        arena.motors.forget_route(ObjId(99));
        assert!(arena.motors.row(ObjId(99)).is_none());
    }

    /// **16b cut AE.1 (16c `B-58`): the three goal pickers read the unit's own
    /// row.** On a priced window, `entry_cell` (a `Withdraw`'s goal) and
    /// `far_entry_cell` (a hunt's) hand a car no rubble cell where they hand a tank
    /// the rubble corner its own row crosses, and `stand_off_cell` (an
    /// `Approach`'s) hands a walker the thick-woods cell at its holding range where
    /// it hands a tank the plot's own cell.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn ae1_the_goal_pickers_read_the_units_own_row() {
        use crate::arena::state::CostRow;
        use crate::arena::test_support::{priced_terrain, tactical_unit_on};
        use mush_atlas::TerrainClass;

        let car = ObjId(4);
        let walker = ObjId(5);
        let hull = ObjId(6);
        let at = |x: u16, y: u16| pose(i64::from(x) * 1_000 + 500, i64::from(y) * 1_000 + 500);
        let mut units = vec![
            tank(car, PartyId::RED, at(4, 4)),
            tank(walker, PartyId::RED, at(1, 3)),
            tank(hull, PartyId::RED, at(4, 5)),
        ];
        if let Some(unit) = units.get_mut(0) {
            unit.unit = tactical_unit_on(car, crate::definitions::unit::WHEELED);
            unit.reseal(&crate::match_::SealTerms::fixture())
                .expect("the fixture terms seal");
        }
        if let Some(unit) = units.get_mut(1) {
            unit.unit = tactical_unit_on(walker, crate::definitions::unit::LEGGED);
            unit.reseal(&crate::match_::SealTerms::fixture())
                .expect("the fixture terms seal");
        }
        let mut arena = arena_of(units);
        // Both zones are rubble but for one grass cell each, and the stand-off
        // cell is thick woods.
        let mut paint: Vec<((u16, u16), TerrainClass)> = Vec::new();
        for y in 0..8 {
            for x in [0_u16, 1, 6, 7] {
                if (x, y) != (1, 7) && (x, y) != (6, 0) && (x, y) != (1, 3) {
                    paint.push(((x, y), TerrainClass::Rubble));
                }
            }
        }
        paint.push(((4, 3), TerrainClass::WoodsHeavy));
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            priced_terrain(8, 8, TerrainClass::Grass, &paint);
        for (id, row) in [
            (car, CostRow::Wheeled),
            (walker, CostRow::Legged),
            (hull, CostRow::Tracked),
        ] {
            if let Some(state) = arena.unit_mut(id) {
                state.cost_row = row;
            }
        }
        let terrain = arena.definition.terrain.clone();

        // `Withdraw`: the car's own zone, clamped to (1, 4) — rubble — so the
        // nearest grass by Manhattan distance, (1, 3); the tank takes the rubble
        // corner it can cross.
        assert!(!terrain.passable_for(CostRow::Wheeled, 1, 4));
        assert!(terrain.passable_for(CostRow::Tracked, 1, 4));
        assert_eq!(arena.entry_cell(car), Some((1, 3)));
        assert_eq!(arena.entry_cell(hull), Some((1, 5)), "the clamped corner");
        // The hunt: the far zone, likewise (no hunt held by the side, so no fan).
        assert_eq!(arena.far_entry_cell(car, &[]), Some((6, 0)));
        assert_eq!(
            arena.far_entry_cell(hull, &[]),
            Some((6, 5)),
            "the clamped corner"
        );
        // 16b cut AI.1: with that corner already a teammate's hunt the hull's
        // row is fanned (5, then 8 off the zone, then 2) and the fanned corner
        // is still read on its own row: (6, 2) is rubble a car cannot cross
        // and a tank can.
        assert!(!terrain.passable_for(CostRow::Wheeled, 6, 2));
        assert!(terrain.passable_for(CostRow::Tracked, 6, 2));
        assert_eq!(
            arena.far_entry_cell(hull, &[(6, 5)]),
            Some((6, 2)),
            "the fanned corner"
        );

        // `Approach`: a plot at (7, 3) held at 3 cells from the walker at (1, 3)
        // projects to (4, 3), which is thick woods.
        let plot = SeenPose {
            x_cm: 7_500,
            y_cm: 3_500,
            z_cm: 0,
            heading_mdeg: 0,
            speed_cms: 0,
        };
        assert_eq!(
            arena.stand_off_cell(walker, &plot, 3_000),
            Some((4, 3)),
            "a walker stands off in the thick woods"
        );
        if let Some(state) = arena.unit_mut(walker) {
            state.cost_row = CostRow::Tracked;
        }
        assert_eq!(
            arena.stand_off_cell(walker, &plot, 3_000),
            Some((7, 3)),
            "a tank falls back to the plot's own cell"
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut AI.1 (16c `B-84`)
    // -----------------------------------------------------------------------

    /// The window's west waypoint, which the AI.1 fixtures' machines make for.
    const AI1_WEST: (u16, u16) = (1, 4);

    /// **A machine at cell (3, 3), online, in `Patrol` toward the west node**,
    /// on the controller's cadence — the fixture of the refusal memory's tests.
    ///
    /// 17b cut M4.5a: the machine is a mind on [`crate::arena::mind::NoMind`]
    /// (a stock mind's first landing would re-name its goal and drop the
    /// planted refusal memory with the route — `land_goal`), its goal the west
    /// node on the hull's motor row (laid here: `set_controller` lays none for
    /// a mind), and the hull on the motor's cadence (`Arena::engage_motor`,
    /// one `MotorTick` on the `decide_ms` grid) — the drive the refusal memory
    /// is the follower's of, under either policy.
    fn ai1_machine() -> Arena {
        let mut arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(3_500, 3_500))]);
        arena.mind_runner = std::sync::Arc::new(crate::arena::mind::NoMind);
        arena.set_controller(ObjId(4), Controller::Mind, MachineState::Patrol);
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.power = crate::arena::state::Power::Online;
        }
        let row = arena.motors.ensure(ObjId(4));
        row.goal = Some(AI1_WEST);
        row.waypoint = Some(0);
        arena.engage_motor(ObjId(4));
        arena
    }

    /// The memory a stop at the route's next cell `(2, 3)` from `(3, 3)` leaves,
    /// lapsing at `until_ms`.
    fn ai1_refusal(until_ms: i64) -> Refusal {
        Refusal {
            at: (3, 3),
            cell: (2, 3),
            cause: BlockedCause::Impassable,
            goal: AI1_WEST,
            until_ms,
        }
    }

    /// One tick of machine #4 at `t_ms`, and what it counted (17b cut M4.5a:
    /// the motor's own step, `Arena::motor_step` — the refusal by name and the
    /// TTL sweep are the follower's and the row's, not the decision's).
    fn ai1_tick(arena: &mut Arena, t_ms: i64) -> MotorCounters {
        let mut counters = MotorCounters::default();
        let _ = arena.motor_step(ObjId(4), t_ms, &mut counters);
        counters
    }

    /// **16b cut AI.1 (16c `B-84`'s acceptance line): a search whose inputs have
    /// not changed since its route was refused at its own next cell is refused
    /// by name.** The machine stood at (3, 3), its route planned from there, and
    /// was stopped at the route's first cell: the stop is remembered until
    /// `REFLEX_AVOID_TTL_MS` from the stop, the due search is refused, counted
    /// and held on — and the memory lapses at its time, and is dropped by
    /// standing elsewhere, by an `Occupied` stop and by an avoid entry ageing
    /// out.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn ai1_a_refusal_that_has_not_changed_refuses_the_search_by_name() {
        let t = 10_000_i64;
        let mut arena = ai1_machine();
        if let Some(row) = arena.motors.row_mut(ObjId(4)) {
            row.searched_from = Some((3, 3));
            row.searched_ms = None;
            row.route = vec![(2, 3), (1, 4)];
        }
        arena.note_blocked(ObjId(4), (2, 3), BlockedCause::Impassable, t);
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert!(row.route.is_empty(), "the route is thrown away");
        assert_eq!(row.refused, Some(ai1_refusal(t + REFLEX_AVOID_TTL_MS)));

        // Refused by name, counted, held.
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.desired_speed_cms = 700;
        }
        let counted = ai1_tick(&mut arena, t + 500);
        assert_eq!(counted.path_refused_unchanged, 1, "{counted:?}");
        assert_eq!(counted.path_queries, 0, "{counted:?}");
        assert_eq!(
            arena.unit(ObjId(4)).map(|state| state.desired_speed_cms),
            Some(0),
            "the machine holds"
        );
        assert!(
            arena
                .motors
                .row(ObjId(4))
                .is_some_and(|row| row.refused.is_some())
        );

        // At its time the memory lapses, and the search is spent.
        let counted = ai1_tick(&mut arena, t + REFLEX_AVOID_TTL_MS);
        assert_eq!(counted.path_refused_unchanged, 0, "{counted:?}");
        assert_eq!(counted.path_queries, 1, "{counted:?}");
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.refused, None, "a spent search clears the memory");
        assert_eq!(row.searched_from, Some((3, 3)));
        assert_eq!(row.searched_ms, Some(t + REFLEX_AVOID_TTL_MS));

        // Standing in another cell: the next tick searches and clears it.
        let set = |arena: &mut Arena| {
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.refused = Some(ai1_refusal(t + REFLEX_AVOID_TTL_MS));
                row.route.clear();
                row.searched_ms = None;
            }
        };
        let mut arena = ai1_machine();
        set(&mut arena);
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.pose.x_cm = 4_500;
        }
        let counted = ai1_tick(&mut arena, t + 500);
        assert_eq!(
            (counted.path_refused_unchanged, counted.path_queries),
            (0, 1),
            "{counted:?}"
        );
        assert_eq!(arena.motors.row(ObjId(4)).and_then(|row| row.refused), None);

        // An `Occupied` stop clears it.
        let mut arena = ai1_machine();
        set(&mut arena);
        arena.note_blocked(ObjId(4), (2, 3), BlockedCause::Occupied, t);
        assert_eq!(arena.motors.row(ObjId(4)).and_then(|row| row.refused), None);

        // An avoid entry ageing out at the top of a tick clears it — here with a
        // search fresh enough that nothing else is spent or read.
        let mut arena = ai1_machine();
        set(&mut arena);
        if let Some(row) = arena.motors.row_mut(ObjId(4)) {
            row.searched_ms = Some(t);
            row.avoid = vec![((5, 5), t - REFLEX_AVOID_TTL_MS)];
        }
        let counted = ai1_tick(&mut arena, t + 500);
        assert_eq!(
            (counted.path_refused_unchanged, counted.path_queries),
            (0, 0),
            "{counted:?}"
        );
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert!(row.avoid.is_empty(), "the entry aged out");
        assert_eq!(row.refused, None, "and took the memory with it");
    }

    /// **16b cut AI.1**: a stop by the ground **beside** the route — the route's
    /// next cell still a neighbour, the stop somewhere else — keeps the route and
    /// remembers nothing; a stop at the route's next cell on a route that was not
    /// planned from the cell the machine stands in throws the route away and
    /// remembers nothing either.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn ai1_a_stop_beside_the_route_keeps_it_and_a_stop_with_no_memory_of_its_start_records_nothing()
    {
        let mut arena = ai1_machine();
        if let Some(row) = arena.motors.row_mut(ObjId(4)) {
            row.route = vec![(2, 3), (1, 4)];
            row.searched_from = Some((3, 3));
        }
        arena.note_blocked(ObjId(4), (3, 4), BlockedCause::Impassable, 10_000);
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.route, vec![(2, 3), (1, 4)], "beside the route: kept");
        assert_eq!(row.refused, None);

        if let Some(row) = arena.motors.row_mut(ObjId(4)) {
            row.searched_from = Some((4, 3));
        }
        arena.note_blocked(ObjId(4), (2, 3), BlockedCause::TooSteep, 10_000);
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert!(row.route.is_empty(), "at the next cell: thrown away");
        assert_eq!(
            row.refused, None,
            "planned from elsewhere, so a search from here is a new search"
        );

        // And a stop with no route at all records nothing — even with every
        // other condition of a memory met: the last search started from the
        // very cell the machine stands in, and it has a goal.
        if let Some(row) = arena.motors.row_mut(ObjId(4)) {
            row.route.clear();
            row.searched_from = Some((3, 3));
            row.refused = None;
        }
        assert_eq!(
            arena.motors.row(ObjId(4)).and_then(|row| row.goal),
            Some(AI1_WEST),
            "the premise: a goal"
        );
        arena.note_blocked(ObjId(4), (3, 4), BlockedCause::Edge, 10_000);
        let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(
            row.refused, None,
            "a routeless stop is the one-cell test's, and no memory's"
        );
        assert!(row.route.is_empty());
    }

    #[test]
    fn conduct_interior_entry_contains_the_whole_arrival_square() {
        use crate::arena::state::EntryZone;
        let mut def = crate::arena::test_support::definition();
        for zone in [
            EntryZone {
                team: PartyId::RED,
                x0: 0,
                x1: 3,
                y0: 0,
                y1: 7,
            },
            EntryZone {
                team: PartyId::BLUE,
                x0: 7,
                x1: 4,
                y0: 7,
                y1: 0,
            },
        ] {
            def.entry_zones = vec![zone];
            for here in [(0, 0), (7, 0), (0, 7), (7, 7), (4, 4)] {
                let goal = entry_interior_cell(&def, zone.team, CostRow::Tracked, here)
                    .expect("passable interior");
                for x in goal.0 - ARRIVE_CELLS..=goal.0 + ARRIVE_CELLS {
                    for y in goal.1 - ARRIVE_CELLS..=goal.1 + ARRIVE_CELLS {
                        assert!((zone.x0.min(zone.x1)..=zone.x0.max(zone.x1)).contains(&x));
                        assert!((zone.y0.min(zone.y1)..=zone.y0.max(zone.y1)).contains(&y));
                    }
                }
            }
        }
        def.entry_zones = vec![EntryZone {
            team: PartyId::RED,
            x0: 0,
            x1: 3,
            y0: 0,
            y1: 7,
        }];
        assert_eq!(
            entry_cell(&def, PartyId::RED, CostRow::Tracked, (7, 4)),
            Some((3, 4))
        );
        assert_eq!(
            entry_interior_cell(&def, PartyId::RED, CostRow::Tracked, (7, 4)),
            Some((2, 4))
        );
        assert_eq!(
            entry_interior_cell(&def, PartyId::BLUE, CostRow::Tracked, (7, 4)),
            None
        );
    }

    #[test]
    fn conduct_interior_entry_uses_the_cost_row_without_boundary_fallback() {
        use crate::arena::state::EntryZone;
        use crate::arena::test_support::priced_terrain;
        use mush_atlas::TerrainClass;
        let mut def = crate::arena::test_support::definition();
        // The unchanged two-cell own zone has no complete arrival square.
        assert_eq!(
            entry_interior_cell(&def, PartyId::RED, CostRow::Tracked, (7, 4)),
            None
        );
        def.entry_zones = vec![EntryZone {
            team: PartyId::RED,
            x0: 0,
            x1: 3,
            y0: 0,
            y1: 7,
        }];
        def.terrain = priced_terrain(8, 8, TerrainClass::Rubble, &[((1, 4), TerrainClass::Grass)]);
        assert_eq!(
            entry_interior_cell(&def, PartyId::RED, CostRow::Tracked, (7, 4)),
            Some((2, 4))
        );
        assert_eq!(
            entry_interior_cell(&def, PartyId::RED, CostRow::Wheeled, (7, 4)),
            Some((1, 4))
        );
        let interior = (1..=6).flat_map(|y| [(1, y), (2, y)]).collect::<Vec<_>>();
        def.terrain = ai1_walled(&interior);
        assert_eq!(
            entry_cell(&def, PartyId::RED, CostRow::Tracked, (7, 4)),
            Some((3, 4))
        );
        assert_eq!(
            entry_interior_cell(&def, PartyId::RED, CostRow::Tracked, (7, 4)),
            None
        );
        for coordinate in [0, u16::MAX] {
            def.entry_zones = vec![EntryZone {
                team: PartyId::RED,
                x0: coordinate,
                x1: coordinate,
                y0: coordinate,
                y1: coordinate,
            }];
            assert_eq!(
                entry_interior_cell(&def, PartyId::RED, CostRow::Tracked, (7, 4)),
                None
            );
        }
    }

    #[test]
    fn conduct_interior_entry_stops_at_the_shared_probe_budget() {
        use crate::arena::state::{ArenaTerrain, EntryZone};
        let mut def = crate::arena::test_support::definition();
        let mut wall = crate::arena::test_support::ground();
        wall.move_cost = 0;
        // A passable interior exists, but lies after the bounded scan. A
        // second zone cannot restart the same exhausted terrain budget.
        let mut cells = vec![wall; 64 * 64];
        *cells.get_mut(62 * 64 + 62).expect("late interior") = crate::arena::test_support::ground();
        def.terrain = ArenaTerrain::new(64, 64, 1_000, cells).expect("bounded map");
        def.entry_zones = vec![
            EntryZone {
                team: PartyId::RED,
                x0: 0,
                x1: 63,
                y0: 0,
                y1: 63,
            },
            EntryZone {
                team: PartyId::RED,
                x0: 61,
                x1: 63,
                y0: 61,
                y1: 63,
            },
        ];
        assert_eq!(
            entry_interior_cell(&def, PartyId::RED, CostRow::Tracked, (0, 0)),
            None
        );
        def.entry_zones.remove(0);
        assert_eq!(
            entry_interior_cell(&def, PartyId::RED, CostRow::Tracked, (0, 0)),
            Some((62, 62))
        );
    }

    #[test]
    fn conduct_interior_entry_keeps_actual_motor_arrival_inside() {
        let mut arena = ai1_machine(); // actual online hull at (3, 3)
        let def = std::sync::Arc::make_mut(&mut arena.definition);
        def.entry_zones
            .iter_mut()
            .find(|zone| zone.team == PartyId::RED)
            .expect("own zone")
            .x1 = 2;
        let old = entry_cell(def, PartyId::RED, CostRow::Tracked, (3, 3)).expect("boundary");
        let interior =
            entry_interior_cell(def, PartyId::RED, CostRow::Tracked, (3, 3)).expect("interior");
        assert_eq!((old, interior), ((2, 3), (1, 3)));
        let row = |goal| MotorRow {
            goal: Some(goal),
            ..MotorRow::default()
        };
        let (_, boundary, _, _) = followed_under(&mut arena, row(old), Fallback::Notice);
        assert!(
            boundary.arrived,
            "existing one-cell boundary arrival is unchanged"
        );
        let (_, outside, _, _) = followed_under(&mut arena, row(interior), Fallback::Notice);
        assert!(
            !outside.arrived,
            "an outside cell cannot satisfy the inset goal"
        );
        arena.unit_mut(ObjId(4)).expect("actual hull").pose = pose(2_500, 3_500);
        let (_, inside, plan, _) = followed_under(&mut arena, row(interior), Fallback::Notice);
        assert!(inside.arrived);
        assert_eq!(plan.speed_cms, Some(0));
        let here = arena
            .definition
            .terrain
            .cell_of(&arena.unit(ObjId(4)).expect("hull").pose)
            .expect("cell");
        assert!(
            arena
                .definition
                .entry_zones
                .iter()
                .any(|zone| zone.team == PartyId::RED
                    && (zone.x0..=zone.x1).contains(&here.0)
                    && (zone.y0..=zone.y1).contains(&here.1))
        );
    }

    /// The 8 by 8 test window with walls at `walls`.
    fn ai1_walled(walls: &[(u16, u16)]) -> crate::arena::state::ArenaTerrain {
        let mut cells = vec![crate::arena::test_support::ground(); 8 * 8];
        for (x, y) in walls {
            if let Some(slot) = cells.get_mut(usize::from(*y) * 8 + usize::from(*x)) {
                slot.move_cost = 0;
            }
        }
        crate::arena::state::ArenaTerrain::new(8, 8, 1_000, cells).expect("the walled window")
    }

    /// **16b cut AI.1**: a limited machine whose route's first step is a
    /// diagonal across a two-wall pinch — a corner neither of whose shoulders is
    /// open, which motion would stop it on — re-plans once: the tick throws the
    /// route and the search stamp away, the next searches (and its search, under
    /// a limit, refuses the corner), and ten more search nothing. The unlimited
    /// twin, whose search reads no shoulder and would return the same route,
    /// holds with the route kept and searches nothing.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn ai1_a_limited_unit_at_a_corner_closed_on_both_shoulders_re_plans_once() {
        for limit in [Some(45_u32), None] {
            let mut arena = ai1_machine();
            std::sync::Arc::make_mut(&mut arena.definition).terrain = ai1_walled(&[(4, 3), (3, 4)]);
            if let Some(state) = arena.unit_mut(ObjId(4)) {
                state.slope_limit_pct = limit;
            }
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.goal = Some((6, 4));
                row.waypoint = Some(1);
                row.route = vec![(4, 4), (5, 4), (6, 4)];
                row.searched_ms = Some(9_000);
                row.searched_from = Some((2, 2));
            }
            let first = ai1_tick(&mut arena, 10_000);
            let row = arena.motors.row(ObjId(4)).cloned().unwrap_or_default();
            if limit.is_some() {
                assert_eq!(first.path_queries, 0, "{first:?}");
                assert!(row.route.is_empty(), "the corner route is thrown away");
                assert_eq!(row.searched_ms, None, "and its search is spent");
                let second = ai1_tick(&mut arena, 10_500);
                assert_eq!(second.path_queries, 1, "{second:?}");
                assert!(
                    second.corner_refusals > 0,
                    "the search refused the corner: {second:?}"
                );
                let route = arena
                    .motors
                    .row(ObjId(4))
                    .map(|row| row.route.clone())
                    .unwrap_or_default();
                assert!(
                    !route.is_empty() && !route.contains(&(4, 4)),
                    "the way round: {route:?}"
                );
            } else {
                assert_eq!(first.path_queries, 0, "{first:?}");
                assert_eq!(
                    row.route,
                    vec![(4, 4), (5, 4), (6, 4)],
                    "held with its route"
                );
                assert_eq!(
                    arena.unit(ObjId(4)).map(|state| state.desired_speed_cms),
                    Some(0)
                );
            }
            let mut later = MotorCounters::default();
            for n in 0..10_i64 {
                later.absorb(&ai1_tick(&mut arena, 11_000 + n * 500));
            }
            assert_eq!(later.path_queries, 0, "limit {limit:?}: {later:?}");
            assert_eq!(
                later.path_refused_unchanged, 0,
                "limit {limit:?}: {later:?}"
            );
        }
    }

    /// **16b cut AI.1's cross-tempo proof of the memory** (16b §11): the refusal
    /// of `ai1_a_refusal_that_has_not_changed_refuses_the_search_by_name`, set
    /// on three arenas before their first pulse and stepped at 5,000, 1,000 and
    /// 500 ms past its lapse and two `repath_ms` more, is one per-substep hash
    /// sequence, one count of refusals and one search spent at one arena
    /// millisecond.
    ///
    /// 17b cut M4.5a: the machine is a mind on `NoMind` (`ai1_machine`),
    /// stepped with `step_settled`, driven by the motor's `MotorTick` on the
    /// `decide_ms` grid where the reflex ticked on its own stagger — the claim
    /// (the first search at the first tick at or after the lapse) is relative
    /// to the tick grid and keeps. Under `Fallback::Notice` the arrival after
    /// that search drops the goal (the reflex walked on to its next node); no
    /// claim reads past the first search but the cross-cadence equality.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn ai1_the_refusal_memory_is_one_log_at_three_cadences() {
        let until_ms = REFLEX_AVOID_TTL_MS;
        // Past the lapse and two `repath_ms` more (38,000 ms), rounded up to a
        // multiple of all three cadences so every run ends at one millisecond.
        let end_ms = (until_ms + 2 * i64::from(ReflexConfig::default().repath_ms))
            .div_euclid(5_000)
            .saturating_add(1)
            .saturating_mul(5_000);
        // Per cadence: the per-substep hashes, the refusals counted, and the
        // search stamp read at every pulse boundary.
        type Run = (Vec<u64>, u32, BTreeMap<i64, Option<i64>>);
        let mut runs: Vec<Run> = Vec::new();
        for cadence in [5_000_i64, 1_000, 500] {
            let mut arena = ai1_machine();
            if let Some(row) = arena.motors.row_mut(ObjId(4)) {
                row.searched_from = Some((3, 3));
                row.refused = Some(ai1_refusal(until_ms));
            }
            arena
                .clock
                .pin_tempo_for_tests(crate::arena::clock::Tempo::Quiet);
            let mut hashes: Vec<u64> = Vec::new();
            let mut refused = 0_u32;
            let mut stamps: BTreeMap<i64, Option<i64>> = BTreeMap::new();
            let mut wall = 0_i64;
            loop {
                // 17b cut M4.5a: a mind's hull, so its thinks settled.
                let out = arena.step_settled(wall, &[], &[]);
                hashes.extend(arena.substep_hashes.iter().copied());
                refused = refused.saturating_add(out.motor.path_refused_unchanged);
                stamps.insert(
                    arena.now_ms,
                    arena.motors.row(ObjId(4)).and_then(|row| row.searched_ms),
                );
                if arena.now_ms >= end_ms {
                    break;
                }
                wall += cadence;
            }
            runs.push((hashes, refused, stamps));
        }
        let first = runs.first().cloned().unwrap_or_default();
        assert!(first.1 > 0, "nothing was refused");
        // The 500 ms run sees every tick's stamp: the first search is spent at
        // the first tick at or after the lapse, and not a tick before it.
        let fine = runs.last().cloned().unwrap_or_default();
        let spent = fine.2.values().flatten().copied().min();
        let decide = i64::from(ReflexConfig::default().decide_ms);
        assert!(
            spent.is_some_and(|at| at >= until_ms && at < until_ms + decide),
            "the first search was spent at {spent:?}, not at the first tick past {until_ms}"
        );
        for run in runs.iter().skip(1) {
            assert_eq!(run.0, first.0, "the hash sequence diverged");
            assert_eq!(run.1, first.1, "the refusals counted diverged");
            // At every boundary all three cadences share, the same stamp.
            for (at, stamp) in &first.2 {
                if let Some(theirs) = run.2.get(at) {
                    assert_eq!(theirs, stamp, "the search stamp at {at} ms diverged");
                }
            }
        }
    }

    /// **16b cut AI.1: the turning circle, the beam and the turn speed are
    /// integer geometry.** A unit facing north at the origin, a circle of
    /// radius `r`: a point abeam at `d` is inside it exactly while `d < 2 r`;
    /// nothing straight ahead or straight behind ever is; the beam is the
    /// east–west line; and the turn speed of a point inside is the speed whose
    /// circle stops holding it — one cm/s more and it holds it again. The same
    /// again heading due east and north-east, where the sine is not nought and a
    /// sign slip in either product would show.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn ai1_the_circle_the_beam_and_the_turn_speed_are_integer_geometry() {
        let at = pose(0, 0);
        let r = 1_000_u64;
        let d = |n: u64| i64::try_from(n).unwrap_or(0);
        for (point, inside) in [
            ((d(r), 0), true),
            ((-d(r), 0), true),
            ((d(2 * r - 1), 0), true),
            ((-d(2 * r - 1), 0), true),
            ((d(2 * r), 0), false),
            ((d(2 * r + 1), 0), false),
            ((-d(2 * r + 1), 0), false),
            ((0, d(r)), false),
            ((0, d(2 * r - 1)), false),
            ((0, -d(r)), false),
            ((0, -d(2 * r - 1)), false),
        ] {
            assert_eq!(
                inside_turning_circle(&at, r, point),
                inside,
                "{point:?} at r = {r}"
            );
        }
        assert!(
            !inside_turning_circle(&at, 0, (d(r), 0)),
            "no circle, no inside"
        );
        for (point, ahead) in [
            ((0, 500), true),
            ((500, 0), true),
            ((-500, 0), true),
            ((0, -500), false),
        ] {
            assert_eq!(ahead_of_beam(&at, point), ahead, "{point:?}");
        }
        // At a turn rate of a radian a second the radius in centimetres is the
        // speed in cm/s, and at half that it is twice the speed.
        for rate in [
            u32::try_from(MDEG_PER_RADIAN).unwrap_or(0),
            u32::try_from(MDEG_PER_RADIAN / 2).unwrap_or(0),
        ] {
            let radius = |speed: u64| speed * MDEG_PER_RADIAN / u64::from(rate);
            for point in [(1_500_i64, 0_i64), (1_000, 700), (-1_200, 300)] {
                assert!(
                    inside_turning_circle(&at, radius(1_500), point),
                    "{point:?} is inside at 1,500 cm/s"
                );
                let v = turn_speed_cms(&at, rate, point);
                assert!(
                    !inside_turning_circle(&at, radius(v), point),
                    "{point:?} is still inside at its turn speed {v}"
                );
                assert!(
                    inside_turning_circle(&at, radius(v + 1), point),
                    "{point:?} is outside one cm/s above its turn speed {v}"
                );
            }
        }
        // **Off the north axis**, where the sine is not nought and a sign slip in
        // the cross or the dot product would show (16b cut AI.1's review): at
        // heading 0 the cross reduces to `|dx·c|` and the dot to `dy·c`, so the
        // table above cannot tell `dx·c − dy·s` from `dx·c + dy·s`.
        //
        // Due east (90,000 mdeg), exactly: abeam is now north and south, and the
        // compass points are exact in the fixed point.
        let mut east = pose(0, 0);
        east.heading_mdeg = 90_000;
        for (point, inside) in [
            ((0, -d(r)), true),
            ((0, d(r)), true),
            ((0, -d(2 * r - 1)), true),
            ((0, d(2 * r - 1)), true),
            ((0, -d(2 * r)), false),
            ((0, d(2 * r + 1)), false),
            ((d(r), 0), false),
            ((d(2 * r - 1), 0), false),
            ((-d(r), 0), false),
            ((-d(2 * r - 1), 0), false),
        ] {
            assert_eq!(
                inside_turning_circle(&east, r, point),
                inside,
                "{point:?} at r = {r}, heading east"
            );
        }
        for (point, ahead) in [
            ((500, 0), true),
            ((0, 500), true),
            ((0, -500), true),
            ((-500, 0), false),
        ] {
            assert_eq!(
                ahead_of_beam(&east, point),
                ahead,
                "{point:?}, heading east"
            );
        }
        // North-east (45,000 mdeg). A point `k` along a rotated axis is
        // `(k·(x + y), k·(y − x)) / √2` for `x` to the right and `y` ahead, rounded
        // to whole centimetres — so abeam is asked at `2 r ± 20`, which the
        // rounding (under a centimetre) cannot move across the circle.
        let mut north_east = pose(0, 0);
        north_east.heading_mdeg = 45_000;
        // `k` centimetres along the frame axis `(right, ahead)`, in the world.
        let turned = |right: i64, ahead: i64| -> (i64, i64) {
            // 1 / √2 as 70,711 / 100,000, to the nearest centimetre.
            let scale = |v: i64| (v * 70_711 + 50_000).div_euclid(100_000);
            (scale(right + ahead), scale(ahead - right))
        };
        for (frame, inside) in [
            // Dead ahead, near and far: never inside.
            ((0, 500), false),
            ((0, d(r)), false),
            ((0, d(3 * r)), false),
            // Dead astern: never inside.
            ((0, -d(r)), false),
            // Abeam to starboard and to port, at `r` and across `2 r`.
            ((d(r), 0), true),
            ((-d(r), 0), true),
            ((d(2 * r) - 20, 0), true),
            ((-d(2 * r) + 20, 0), true),
            ((d(2 * r) + 20, 0), false),
            ((-d(2 * r) - 20, 0), false),
        ] {
            let point = turned(frame.0, frame.1);
            assert_eq!(
                inside_turning_circle(&north_east, r, point),
                inside,
                "{frame:?} in the frame, {point:?} in the world, at r = {r}, heading north-east"
            );
        }
        for (frame, ahead) in [
            ((0, 500), true),
            ((500, 10), true),
            ((-500, 10), true),
            ((500, -10), false),
            ((-500, -10), false),
            ((0, -500), false),
        ] {
            let point = turned(frame.0, frame.1);
            assert_eq!(
                ahead_of_beam(&north_east, point),
                ahead,
                "{frame:?} in the frame, {point:?} in the world, heading north-east"
            );
        }
        // And the turn speed's round trip, off the axis.
        for rate in [
            u32::try_from(MDEG_PER_RADIAN).unwrap_or(0),
            u32::try_from(MDEG_PER_RADIAN / 2).unwrap_or(0),
        ] {
            let radius = |speed: u64| speed * MDEG_PER_RADIAN / u64::from(rate);
            for frame in [(1_500_i64, 0_i64), (1_000, 700), (-1_200, 300)] {
                let point = turned(frame.0, frame.1);
                assert!(
                    inside_turning_circle(&north_east, radius(1_500), point),
                    "{point:?} is inside at 1,500 cm/s, heading north-east"
                );
                let v = turn_speed_cms(&north_east, rate, point);
                assert!(
                    !inside_turning_circle(&north_east, radius(v), point),
                    "{point:?} is still inside at its turn speed {v}, heading north-east"
                );
                assert!(
                    inside_turning_circle(&north_east, radius(v + 1), point),
                    "{point:?} is outside one cm/s above its turn speed {v}, heading north-east"
                );
            }
        }

        // And the circle a unit is on by its next tick.
        let arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(1_500, 1_500))]);
        let mut unit = arena.units.first().cloned().expect("the tank");
        unit.speed_cms = 600;
        // min(1,000, 600 + 200 × 1,000 / 1,000) = 800 cm/s at 30,000 mdeg/s.
        assert_eq!(
            turning_radius_cm(&unit, 1_000),
            800 * MDEG_PER_RADIAN / 30_000
        );
        unit.speed_cms = 1_000;
        assert_eq!(
            turning_radius_cm(&unit, 1_000),
            1_000 * MDEG_PER_RADIAN / 30_000
        );
        unit.mobility.turn_rate_mdeg_s = 0;
        assert_eq!(turning_radius_cm(&unit, 1_000), 0);
    }

    /// `Arena::follow` once, for machine #4 of `arena`, on `row` (17b cut
    /// M1a.1: a motor row, the table taken out beside it as the tick takes it,
    /// under the reflex controller's own policy).
    fn ai1_followed(arena: &mut Arena, row: MotorRow) -> MotorRow {
        followed_under(arena, row, Fallback::Waypoint).0
    }

    /// `Arena::follow` once at 10,000 ms, for machine #4 of `arena`, on `row`
    /// under `policy` (17b cut M1a.1): the row it left, what it answered, the
    /// orders it planned and what it counted.
    fn followed_under(
        arena: &mut Arena,
        mut row: MotorRow,
        policy: Fallback,
    ) -> (MotorRow, MotorOutcome, MotorPlan, MotorCounters) {
        let here = arena
            .unit(ObjId(4))
            .and_then(|state| arena.definition.terrain.cell_of(&state.pose));
        let mut motors = std::mem::take(&mut arena.motors);
        let mut counters = MotorCounters::default();
        let mut plan = MotorPlan::default();
        let outcome = arena.follow(
            ObjId(4),
            10_000,
            &mut motors,
            &mut row,
            here,
            policy,
            &mut counters,
            &mut plan,
        );
        arena.motors = motors;
        (row, outcome, plan, counters)
    }

    /// **17b cut M1a.1: the two fallback policies, at the `unreachable` edge
    /// and at arrival** (doc 17 §4, D-UM-3; the contract 17b cut M1a.4's
    /// `DRIVE` builds on).
    ///
    /// Machine #4 stands at (3, 3) on the test window with its goal (6, 6)
    /// walled in on all eight sides, so the one search it may spend fails.
    /// Under [`Fallback::Waypoint`] — the reflex controller's rule (no
    /// production caller since 17b cut M4.6) —
    /// the failure is counted, the goal that failed is answered, and the goal
    /// becomes the nearest waypoint (`west`, node 0, at (1, 4)) with the
    /// waypoint pair stepped. Under [`Fallback::Notice`] the failure is counted
    /// and answered the same, but the goal and the route are dropped, the hull
    /// is ordered to stand, and no waypoint, no departure node and no Hold
    /// retry clock is touched: a goal a pilot or a mind named is never swapped
    /// for a waypoint behind its back. And with the goal one cell off, both
    /// policies answer `arrived`, clear the route and order a stand; only
    /// `Notice` spends the goal.
    #[test]
    fn m1a1_the_fallback_policies_at_the_unreachable_edge_and_at_arrival() {
        const SEALED: (u16, u16) = (6, 6);
        let sealed_row = MotorRow {
            goal: Some(SEALED),
            route: Vec::new(),
            waypoint: Some(1),
            from_waypoint: Some(0),
            hold_retry_ms: Some(77_000),
            ..MotorRow::default()
        };
        let sealed_arena = || {
            let mut arena = ai1_machine();
            std::sync::Arc::make_mut(&mut arena.definition).terrain = ai1_walled(&[
                (5, 5),
                (6, 5),
                (7, 5),
                (5, 6),
                (7, 6),
                (5, 7),
                (6, 7),
                (7, 7),
            ]);
            arena
        };

        // The reflex controller's policy: the nearest waypoint takes the goal.
        let (row, outcome, _, counters) =
            followed_under(&mut sealed_arena(), sealed_row.clone(), Fallback::Waypoint);
        assert_eq!(
            (counters.path_queries, counters.path_failures),
            (1, 1),
            "one search, and it failed: {counters:?}"
        );
        assert_eq!(
            outcome,
            MotorOutcome {
                arrived: false,
                unreachable: Some(SEALED),
            },
            "the goal that failed is answered"
        );
        assert_eq!(
            row.goal,
            Some(AI1_WEST),
            "the nearest waypoint took its place"
        );
        assert_eq!(
            (row.waypoint, row.from_waypoint),
            (Some(0), Some(1)),
            "and the waypoint pair stepped"
        );
        assert_eq!(row.hold_retry_ms, Some(77_000), "no Hold, no retry clock");

        // M1a.4's policy: dropped, stood, and nothing swapped in.
        let (row, outcome, plan, counters) =
            followed_under(&mut sealed_arena(), sealed_row.clone(), Fallback::Notice);
        assert_eq!(
            (counters.path_queries, counters.path_failures),
            (1, 1),
            "one search, and it failed: {counters:?}"
        );
        assert_eq!(
            outcome,
            MotorOutcome {
                arrived: false,
                unreachable: Some(SEALED),
            },
            "the goal that failed is answered"
        );
        assert_eq!(row.goal, None, "the named goal is dropped");
        assert!(row.route.is_empty(), "with any route");
        assert_eq!(
            plan,
            MotorPlan {
                heading_mdeg: None,
                speed_cms: Some(0),
            },
            "and the hull is ordered to stand"
        );
        assert_eq!(
            (row.waypoint, row.from_waypoint, row.hold_retry_ms),
            (Some(1), Some(0), Some(77_000)),
            "no waypoint, departure node or retry clock was touched"
        );
        assert_eq!(row.searched_from, Some((3, 3)), "the search was spent here");

        // Arrival, one cell off the goal, under both.
        for policy in [Fallback::Waypoint, Fallback::Notice] {
            let near = MotorRow {
                goal: Some((4, 4)),
                route: vec![(4, 4)],
                ..sealed_row.clone()
            };
            let (row, outcome, plan, counters) = followed_under(&mut ai1_machine(), near, policy);
            assert_eq!(
                outcome,
                MotorOutcome {
                    arrived: true,
                    unreachable: None,
                },
                "{policy:?}: arrived"
            );
            assert!(row.route.is_empty(), "{policy:?}: the route is spent");
            assert_eq!(plan.speed_cms, Some(0), "{policy:?}: and the hull stands");
            assert_eq!(counters.path_queries, 0, "{policy:?}: nothing searched");
            let kept = if policy == Fallback::Notice {
                None
            } else {
                Some((4, 4))
            };
            assert_eq!(
                row.goal, kept,
                "{policy:?}: only a named goal is spent at arrival"
            );
            assert_eq!(
                (row.waypoint, row.from_waypoint),
                (Some(1), Some(0)),
                "{policy:?}: arrival steps no waypoint"
            );
        }
    }

    /// **16b cut AI.1: consumption drains only across a drivable step.** From
    /// (0, 0), a route that goes east, north and back west to (0, 1) is drained
    /// to (0, 1) in one tick on level ground: every cell before it is behind a
    /// machine that can step there. With (0, 1) 800 cm up and a 45 per cent
    /// limit the step to it is refused, so it is not drained to — the drain
    /// stops at (1, 1), whose one open shoulder makes it a step the machine may
    /// drive. And with (1, 1) the last route cell and both its shoulders walls,
    /// nothing is drained at all.
    #[test]
    // Moved from reflex.rs by 17b cut M1a.1; proves what it proved.
    fn ai1_consumption_drains_only_across_a_drivable_step() {
        let route = vec![
            (1, 0),
            (2, 0),
            (2, 1),
            (1, 1),
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 4),
        ];
        let row = MotorRow {
            goal: Some((0, 5)),
            route: route.clone(),
            searched_ms: Some(10_000),
            ..MotorRow::default()
        };
        // Level.
        let mut arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(500, 500))]);
        let followed = ai1_followed(&mut arena, row.clone());
        assert_eq!(
            followed.route.first().copied(),
            Some((0, 1)),
            "{:?}",
            followed.route
        );
        assert_eq!(followed.route.len(), 4);

        // (0, 1) up a bluff, for a limited unit.
        let mut arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(500, 500))]);
        let mut cells = vec![crate::arena::test_support::ground(); 8 * 8];
        if let Some(slot) = cells.get_mut(8) {
            slot.elev_cm = 800;
        }
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            crate::arena::state::ArenaTerrain::new(8, 8, 1_000, cells).expect("the bluff");
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.slope_limit_pct = Some(45);
        }
        let followed = ai1_followed(&mut arena, row.clone());
        assert_eq!(
            followed.route.first().copied(),
            Some((1, 1)),
            "never across the refused edge to (0, 1): {:?}",
            followed.route
        );

        // Both shoulders of (1, 1) walls, the route ending there: nothing.
        let mut arena = arena_of(vec![tank(ObjId(4), PartyId::RED, pose(500, 500))]);
        std::sync::Arc::make_mut(&mut arena.definition).terrain = ai1_walled(&[(1, 0), (0, 1)]);
        let short = MotorRow {
            route: vec![(1, 0), (2, 0), (2, 1), (1, 1)],
            ..row
        };
        let followed = ai1_followed(&mut arena, short);
        assert_eq!(
            followed.route,
            vec![(1, 0), (2, 0), (2, 1), (1, 1)],
            "nothing drained (and an unlimited unit holds with it)"
        );
    }

    /// **16b cut AI.1, the programme lead's ruling on `shipped_scale` e3: the
    /// hunters of one side are handed cells clear of each other's, wherever
    /// they stand.**
    ///
    /// On the test window (Blue's zone is columns 6 and 7, every row), three Red
    /// tanks on row 3 all clamp to Blue's corner (6, 3). With no hunt held by
    /// the side each is handed that corner, E.4's cell; handed one after
    /// another, each against the cells handed before it, they get rows 3, 6 and
    /// 0 — and from one cell exactly, since nothing here reads a pose but the
    /// unit's own. A unit whose own row is already [`HUNT_FAN_ROWS`] from the
    /// held cell keeps its own row (the cross-row case a fan by rank alone got
    /// wrong); one two rows from it is fanned; a zone with no row left clear
    /// answers E.4's cell, the named residual. [`fan_rows`] is pinned row for
    /// row from every row of the window.
    ///
    /// 17b cut M4.6a: part (a) of the reflex's
    /// `ai1_the_hunters_of_one_side_are_handed_clear_cells_at_three_cadences`,
    /// moved here with its assertions word for word — the only pin of
    /// [`fan_rows`] row for row; its part (b), three reflex hunters at three
    /// cadences, stayed there and dies with the reflex in 17b cut M4.6c.
    #[test]
    fn ai1_the_hunters_of_one_side_are_handed_clear_cells() {
        let red = |id: u32, x: u16, y: u16| {
            tank(
                ObjId(id),
                PartyId::RED,
                pose(i64::from(x) * 1_000 + 500, i64::from(y) * 1_000 + 500),
            )
        };
        // (a) The answer itself.
        let mut arena = arena_of(vec![
            tank(ObjId(3), PartyId::BLUE, pose(7_500, 7_500)),
            red(4, 2, 3),
            red(5, 3, 3),
            red(6, 4, 3),
        ]);
        assert_eq!(
            [4, 5, 6].map(|id| arena.far_entry_cell(ObjId(id), &[])),
            [Some((6, 3)); 3],
            "no hunt held: every machine is handed E.4's clamped corner"
        );
        let first = arena.far_entry_cell(ObjId(4), &[]);
        assert_eq!(first, Some((6, 3)));
        let second = arena.far_entry_cell(ObjId(5), &[(6, 3)]);
        assert_eq!(second, Some((6, 6)), "the fan's first row");
        let third = arena.far_entry_cell(ObjId(6), &[(6, 3), (6, 6)]);
        assert_eq!(third, Some((6, 0)), "the fan's second row");
        // From one cell exactly: the held cells, not the poses, keep them apart.
        let one = arena.unit(ObjId(4)).map(|unit| unit.pose.clone());
        for id in [5, 6] {
            if let (Some(state), Some(at)) = (arena.unit_mut(ObjId(id)), one.clone()) {
                state.pose = at;
            }
        }
        assert_eq!(
            [
                arena.far_entry_cell(ObjId(4), &[]),
                arena.far_entry_cell(ObjId(5), &[(6, 3)]),
                arena.far_entry_cell(ObjId(6), &[(6, 3), (6, 6)]),
            ],
            [Some((6, 3)), Some((6, 6)), Some((6, 0))],
            "three machines in one cell, three hunt cells"
        );
        // Across rows: a held cell three rows off leaves the unit its own row;
        // one two rows off fans it; a zone with every fanned row's cell held
        // answers E.4's cell after all.
        assert_eq!(
            arena.far_entry_cell(ObjId(4), &[(6, 6)]),
            Some((6, 3)),
            "three rows apart is clear"
        );
        assert_eq!(
            arena.far_entry_cell(ObjId(4), &[(6, 5)]),
            Some((6, 0)),
            "two rows apart is not: row 6 is one from it, row 0 is clear"
        );
        assert_eq!(
            arena.far_entry_cell(ObjId(4), &[(6, 0), (6, 3), (6, 6)]),
            Some((6, 3)),
            "the fan ran out, so E.4's cell"
        );
        // Another side's hunts are no business of Blue's: `taken` is the
        // caller's, and Blue's own is empty.
        assert_eq!(arena.far_entry_cell(ObjId(3), &[]), Some((1, 7)));
        // The fan's rows, from every row of an eight-row zone: the clamped row
        // first, then alternately above and below in steps of three, rows off
        // the zone skipped and nothing repeated.
        let expected: [&[u16]; 8] = [
            &[0, 3, 6],
            &[1, 4, 7],
            &[2, 5],
            &[3, 6, 0],
            &[4, 7, 1],
            &[5, 2],
            &[6, 3, 0],
            &[7, 4, 1],
        ];
        for (cy, want) in (0..=7_u16).zip(expected) {
            let rows: Vec<u16> = fan_rows(cy, 0, 7).collect();
            assert_eq!(rows, want, "the fan from row {cy}");
            for (i, a) in rows.iter().enumerate() {
                for b in rows.iter().skip(i + 1) {
                    assert!(
                        a.abs_diff(*b) >= HUNT_FAN_ROWS,
                        "rows {a} and {b} from row {cy}"
                    );
                }
            }
        }
        assert_eq!(fan_rows(4, 4, 4).collect::<Vec<_>>(), vec![4], "one row");
    }
}
