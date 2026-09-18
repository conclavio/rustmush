//! **The match: its states, its identity, its custody and its result** (16b cut
//! T.3; doc 16 §3.1 "Custody", §3.8 "Lifecycle", §4 "The result record").
//!
//! The arena task (cut T.2) runs one match. This is everything *around* it that
//! the world owns: the state machine of doc 16 §3.8, the counter that mints match
//! ids, the RAM record of each match while it runs, the two custody transitions,
//! and the one document each match leaves behind.
//!
//! # Why the module is called `match_`
//!
//! `match` is a keyword. The underscore is the whole of the compromise; every
//! name inside it is spelled properly.
//!
//! # The state machine, and which half of it this cut builds
//!
//! ```text
//! Idle → Selection → Boarding → Running → Ended → Idle
//!                  ↘        ↘        ↘
//!                   Aborted ← Aborted ← Aborted | Void → Idle
//! ```
//!
//! `Selection` and `Boarding` are slots: cut K fills them with `SIM TEAM`, `SIM
//! START` and `BOARD`. What this cut builds is the machine itself and the one
//! transition that has to be right before any of that exists — **`Boarding →
//! Running`, the check-out**, as a function ([`start`]) cut K calls — and the
//! three ends, [`finish`], which is check-in.
//!
//! Staff drive both ends today through `@arena/start` and `@arena/abort`, so the
//! custody transition is exercised, reviewed and tested without waiting for the
//! cockpit.
//!
//! # Identity across boots
//!
//! A match id names a durable ledger log (`log:match.<id>`) and a durable result
//! (`tacsim:match:<id>`). A counter that restarted at zero every boot would write
//! the second match of the second boot over the second match of the first, so the
//! counter is **persisted** (`tacsim:match:seq`), read at boot with the rest of
//! the tacsim store, and advanced past the highest id any surviving result
//! already claims. Allocation is one document write on the same lane as
//! everything else the module persists.
//!
//! # What "one authoritative action" means here
//!
//! Check-out writes a marker on every unit's record and sends one `CustodyOut`.
//! Check-in applies each unit's returned cells, re-places it at the arena's final
//! pose and clears its marker. Both run inside **one** command handler on the
//! single world task, so every document they queue rides one outbox batch and
//! lands or does not land together — which is what the journal and the T6 gate
//! both read. There is no second lane and no second writer.

mod arena_def;
pub mod centre;
/// 16b cut K.2: **the cockpit** — `BOARD`, `EXIT`, doc 16 §3.7's nine selectors,
/// the gate that makes them contextual, and doc 16 §5's frame.
pub mod cockpit;
pub mod lobby;
mod record;
pub mod sim;
mod verbs;

pub use arena_def::{
    ARENA_CELL_CM, ARENA_CELLS, PROVING_GROUND, STAND_IN, definition_for, resolution_for,
    stock_definition, stock_resolution, tracked_rules, unit_profile,
};
pub(crate) use centre::record_roster;
pub use centre::{
    ArenaRoster, BLUE_ROOM_ENTRY, CENTRE_ARENA, CENTRE_PACK, CentreRoom, LOBBY_ENTRY, POD_ENTRIES,
    PODS_PER_SIDE, RED_ROOM_ENTRY, ROSTER_PREFIX, centre_object, centre_rooms, pod_object,
    pod_slot, room_role, roster_from_document, roster_id_of_key, roster_key, slot_id, slot_object,
    tank_for_slot, tank_of, where_in_centre,
};
pub use cockpit::{
    BOARD_VERB, COCKPIT_VERBS, CockpitError, CockpitGate, EXIT_VERB, NO_LOCK, NO_SUCH_CONTACT,
    NOT_IN_A_POD, OUTSIDE_COCKPIT, OUTSIDE_TEAM_ROOM, POWER_NOT_ONLINE, STATION,
    board as cockpit_board, exit as cockpit_exit, flying,
};
pub use lobby::{
    Centre, OUTSIDE_CENTRE, SimError, Slot, abort as sim_abort, board_deadline,
    centre as centre_of, claim, leave, pilot_of, pod_of, reset as reset_centre, slot_seated,
    slot_vacated, slots_per_side, start as sim_start, status_lines,
};
pub use record::{
    MatchOutcome, MatchRecord, MatchState, PoseNumbers, TeamRoster, UnitOutcome, UnitResult,
    match_id_of_key, result_key, write_restart_abort,
};
pub use sim::{SIM_VERB, Selector, SimGate, sim_command};
pub use verbs::{ArenaVerbError, arena_command, register_switches, state_word};

use std::collections::BTreeMap;

use mush_core::{Ctx, ObjId};
use smol_str::SmolStr;

use crate::arena::damage::UnitReturn;
use crate::arena::state::{CustodyOut, CustodyUnit, EndReason, MatchEnd, Pilot, StopReason, Team};
use crate::arena_bridge::{ArenaRegistry, OpenMatchError, match_log_id};
use crate::arena_task::MatchId;
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};

/// The persisted key of the match counter, inside the tacsim namespace.
pub const SEQ_KEY: &str = "tacsim:match:seq";

/// How many matches one server tracks in RAM before `@arena/status` stops
/// listing them. A finished match's record is dropped as its result is written,
/// so this only ever bounds a server with very many live matches.
pub const MAX_TRACKED_MATCHES: usize = 256;

// ---------------------------------------------------------------------------
// The book
// ---------------------------------------------------------------------------

/// **The world's book of matches** — the tacsim module's own state slot.
///
/// World-task-only, like [`ArenaRegistry`] beside it. It is a second slot rather
/// than three more fields on that one because the two have different lifetimes:
/// the registry's rows exist only while a task does, and a [`MatchRecord`] exists
/// from `Selection` (before any task) to the moment its result is written (after
/// the task is gone).
#[derive(Clone, Debug, Default)]
pub struct MatchBook {
    /// The next id to mint. `0` means "not loaded yet"; the first allocation
    /// reads the store's value through [`MatchBook::adopt_seq`].
    next_id: u64,
    /// Whether the persisted counter has been read.
    loaded: bool,
    /// Every match this server is tracking, by id.
    matches: BTreeMap<MatchId, MatchRecord>,
    /// **16b cut K.1** — the simulator centre: the lobby's slots, its pods and
    /// where it is in doc 16 §3.8's machine.
    ///
    /// One per server, on this slot rather than one of its own, because a centre
    /// and a match record are two halves of one story: the centre names the
    /// match it is holding and a check-in has to release both in one action.
    centre: lobby::Centre,
}

impl MatchBook {
    /// An empty book.
    #[must_use]
    pub fn new() -> MatchBook {
        MatchBook::default()
    }

    /// Adopt the persisted counter at boot. Never lowers: a result document that
    /// claims a higher id than the counter (a crash between the two writes) still
    /// cannot have its id re-used.
    pub fn adopt_seq(&mut self, seq: u64) {
        self.next_id = self.next_id.max(seq);
        self.loaded = true;
    }

    /// Whether the persisted counter has been read at least once.
    #[must_use]
    pub const fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// The next id this book would mint, without minting it.
    #[must_use]
    pub const fn peek_seq(&self) -> u64 {
        self.next_id.saturating_add(1)
    }

    /// One match's record.
    #[must_use]
    pub fn get(&self, match_id: MatchId) -> Option<&MatchRecord> {
        self.matches.get(&match_id)
    }

    /// One match's record, mutably.
    pub fn get_mut(&mut self, match_id: MatchId) -> Option<&mut MatchRecord> {
        self.matches.get_mut(&match_id)
    }

    /// Every tracked match, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (MatchId, &MatchRecord)> {
        self.matches.iter().map(|(id, record)| (*id, record))
    }

    /// How many matches are tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// Whether nothing is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Forget the oldest FINISHED record when the book is at `cap`. A live
    /// match's row is what its own check-in reads, so it is never the one that
    /// goes.
    fn prune_finished(&mut self, cap: usize) {
        if self.matches.len() < cap {
            return;
        }
        let stale = self
            .matches
            .iter()
            .find(|(_, record)| !record.state.is_live())
            .map(|(id, _)| *id);
        if let Some(stale) = stale {
            self.matches.remove(&stale);
        }
    }

    /// **The simulator centre** (16b cut K.1).
    #[must_use]
    pub fn centre(&self) -> &lobby::Centre {
        &self.centre
    }

    /// The centre, mutably — the lobby's own transitions and nothing else.
    pub(crate) fn centre_mut(&mut self) -> &mut lobby::Centre {
        &mut self.centre
    }

    /// Which match holds `unit`, as the book knows it (the registry's binding is
    /// the one the routing reads; this one survives the task).
    #[must_use]
    pub fn match_of_unit(&self, unit: ObjId) -> Option<MatchId> {
        self.matches
            .iter()
            .find(|(_, record)| record.state.is_live() && record.holds(unit))
            .map(|(id, _)| *id)
    }
}

/// Mint the next match id, persisting the counter in the same action.
///
/// # Errors
///
/// A message when the tacsim layer is not composed.
pub fn allocate_id(ctx: &mut Ctx<'_>) -> Result<MatchId, String> {
    let next = {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or("the arena is not available on this server")?;
        book.next_id = book.next_id.saturating_add(1);
        book.loaded = true;
        book.next_id
    };
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: SmolStr::new_static(SEQ_KEY),
            value: serde_json::json!({ "next_id": next }),
        },
    );
    Ok(MatchId(next))
}

/// Read the persisted counter out of the boot key-value pairs.
///
/// Total: an unreadable counter answers `0`, and the boot pass then raises the
/// book past every result document it can see, so a lost counter costs no id.
#[must_use]
pub fn seq_from_document(value: &serde_json::Value) -> u64 {
    value
        .get("next_id")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Check-out: the `Boarding -> Running` trigger
// ---------------------------------------------------------------------------

/// One side of a match, as [`start`] is given it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    /// Which side.
    pub team: Team,
    /// Its units, in the order they were named.
    pub units: Vec<ObjId>,
}

/// Why a match could not start. Every variant names the subject.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// The tacsim layer did not compose.
    #[error("the arena is not available on this server")]
    Unavailable,
    /// The match already exists and is not in `Boarding`.
    #[error("match {match_id} is {state}, not boarding")]
    WrongState {
        /// Which match.
        match_id: u64,
        /// What state it is in.
        state: &'static str,
    },
    /// A side had no units.
    #[error("the {0} side has no units")]
    EmptySide(&'static str),
    /// A unit was named twice, on one side or on both.
    #[error("{0} was named twice")]
    DuplicateUnit(ObjId),
    /// One side is over `match.max_slots_per_side`.
    #[error("the {team} side has {units} units; a side takes at most {max}")]
    TooManyUnits {
        /// Which side.
        team: &'static str,
        /// How many were named.
        units: usize,
        /// The configured cap.
        max: u32,
    },
    /// A named object is not an active tactical unit.
    #[error("{0} is not a tactical unit")]
    NotAUnit(ObjId),
    /// A named unit has no world placement, so it has no pose to freeze.
    #[error("{0} is not placed in a domain; place it before it can deploy")]
    NotPlaced(ObjId),
    /// A named unit is already in a match.
    #[error("{0}")]
    Deployed(#[from] mush_sim::deployed::UnitDeployed),
    /// The arena definition is unknown.
    #[error("there is no arena definition called `{0}`")]
    NoArena(SmolStr),
    /// **16b cut C.2** — the arena is authored, and its ground could not be
    /// built over this world's atlas records.
    #[error("{0}")]
    NoGround(String),
    /// The window has no room for that many units in a side's entry zone.
    #[error("the {0} entry zone has no free cell for every unit on that side")]
    NoEntryCell(&'static str),
    /// The check-out itself refused.
    #[error("{unit} could not be checked out: {detail}")]
    Custody {
        /// The unit.
        unit: ObjId,
        /// What the tactics layer said.
        detail: String,
    },
    /// The arena refused the check-out, or could not be started.
    #[error("{0}")]
    Open(#[from] OpenMatchError),
}

/// **`Boarding → Running`** (doc 16 §3.8): check every unit out and open the
/// match, as one authoritative action.
///
/// In order, and the order is the whole of the safety argument:
///
/// 1. **Every refusal first.** Sides non-empty and inside
///    `match.max_slots_per_side`, no duplicate, every unit a live tactical unit,
///    placed, and not already deployed; the arena definition resolved; an entry
///    cell for every unit. Nothing is written while any of this can still fail.
/// 2. **Check out.** One marker per unit, one write each, the world placement
///    left where it is.
/// 3. **Open.** The task is spawned, the registry tracks it, every unit and pilot
///    is bound and the epoch mirror is seeded — all of it [`open_match`]'s.
/// 4. **Roll back on a refusal at step 3.** The markers written in step 2 are
///    checked straight back in (as a void, so no arena state can have reached
///    them — there was no arena), and the error is the caller's. A half-deployed
///    unit is the one state this function must not be able to leave behind.
///
/// [`open_match`]: crate::arena_bridge::open_match
///
/// # Errors
///
/// [`StartError`], naming the unit or the side. On any of them, nothing is
/// deployed.
pub fn start(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    arena: &SmolStr,
    sides: &[Side],
    arena_cfg: &ArenaConfig,
    match_cfg: &MatchConfig,
    sensors_cfg: &SensorsConfig,
) -> Result<MatchId, StartError> {
    // 16b cut C.2: the authored arena when a loaded pack declares one, and the
    // stand-in window of T.3 otherwise.
    let definition = arena_def::definition_for(ctx.w, arena)
        .map_err(StartError::NoGround)?
        .ok_or_else(|| StartError::NoArena(arena.clone()))?;

    // (1) Every refusal, before the first write.
    let mut seen: Vec<ObjId> = Vec::new();
    for side in sides {
        if side.units.is_empty() {
            return Err(StartError::EmptySide(side.team.as_str()));
        }
        if side.units.len() > match_cfg.max_slots_per_side as usize {
            return Err(StartError::TooManyUnits {
                team: side.team.as_str(),
                units: side.units.len(),
                max: match_cfg.max_slots_per_side,
            });
        }
        for unit in &side.units {
            if seen.contains(unit) {
                return Err(StartError::DuplicateUnit(*unit));
            }
            seen.push(*unit);
            mush_sim::deployed::refuse_if_deployed(ctx.w, *unit)?;
            // "Is it a unit at all" before "is it placed": an object that is
            // neither wants the first answer, not the second.
            if mush_tactics::module::unit_of(ctx.w, *unit).is_none() {
                return Err(StartError::NotAUnit(*unit));
            }
            if mush_sim::pose_of(ctx.w, *unit).is_none() {
                return Err(StartError::NotPlaced(*unit));
            }
        }
    }

    // The opening poses, and the roster the record keeps.
    let mut custody_units: Vec<CustodyUnit> = Vec::new();
    let mut pilots: Vec<Pilot> = Vec::new();
    let mut rosters: Vec<TeamRoster> = Vec::new();
    let mut taken: Vec<(u16, u16)> = Vec::new();
    // 16b cut S.1: the loaded content, read once for the whole check-out. The arena
    // decodes nothing and holds no registry (D-16-1), so every unit's sensor row,
    // weapon rows and eye height are decoded here and carried across the seam. A
    // world with no content loaded gets an empty catalog, and every unit then goes
    // over with no sensor and no weapon — which runs a match with no perception,
    // exactly as cut A's did.
    let catalog = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .map(|slot| slot.definitions().clone())
        .unwrap_or_default();
    for side in sides {
        let mut roster = TeamRoster {
            team: side.team,
            units: side.units.clone(),
            pilots: Vec::new(),
        };
        for unit in &side.units {
            let cells = mush_tactics::module::unit_of(ctx.w, *unit)
                .ok_or(StartError::NotAUnit(*unit))?
                .clone();
            let world_pose = mush_sim::pose_of(ctx.w, *unit).map(|(_, pose)| pose);
            let pose = arena_def::opening_pose(&definition, side.team, world_pose, &mut taken)
                .ok_or(StartError::NoEntryCell(side.team.as_str()))?;
            custody_units.push(CustodyUnit {
                id: *unit,
                team: side.team,
                pose,
                mobility: tracked_rules(&cells),
                sensor_height_cm: arena_def::sensor_height_cm(&catalog, &cells),
                sensor: arena_def::sensor_profile(&catalog, &cells),
                weapons: arena_def::weapon_summaries(&catalog, &cells),
                // **16b cut W.2**: what the unit presents and what a hit may
                // strike, and the gunnery the `PilotProfile` seam gives its shots.
                profile: arena_def::unit_profile(&catalog, &cells),
                gunnery: gunnery_of(ctx, *unit),
                unit: cells,
            });
            for (pilot, station) in seated_pilots(ctx, *unit) {
                roster.pilots.push((pilot, *unit));
                pilots.push(Pilot {
                    unit: *unit,
                    pilot,
                    station,
                    epoch: 1,
                });
            }
        }
        rosters.push(roster);
    }

    // (2) The record, in `Boarding`. The machine is walked, not jumped: this is
    // the state cut K's `SIM START` leaves the match in, and the transition at
    // the end of this function is the one named trigger of doc 16 section 3.8. A
    // caller that already has a record - cut K - is refused unless that record
    // is in `Boarding`.
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let seed = seed_for(match_id, ctx.w.now());
    let boarding = MatchRecord {
        id: match_id,
        state: MatchState::Boarding,
        arena: arena.clone(),
        arena_definition_digest: crate::arena_bridge::digest_text(&definition.digest()),
        teams: rosters,
        started_ms: now_ms,
        ended: None,
        ledger_log_id: match_log_id(match_id),
        ledger_digest: String::new(),
        seed,
        last_pose: BTreeMap::new(),
    };
    {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(StartError::Unavailable)?;
        match book.matches.get(&match_id) {
            Some(record) if !record.state.may_start() => {
                return Err(StartError::WrongState {
                    match_id: match_id.0,
                    state: record.state.as_str(),
                });
            }
            Some(_) => {}
            None => {
                book.prune_finished(MAX_TRACKED_MATCHES);
                book.matches.insert(match_id, boarding);
            }
        }
    }

    // (3) Check out. One marker per unit, one write each.
    let mark = mush_sim::deployed::DeployedMark::new(match_id.0, arena.clone());
    let mut checked_out: Vec<ObjId> = Vec::new();
    for unit in &seen {
        match mush_tactics::custody::check_out(ctx, *unit, mark.clone()) {
            Ok(_) => checked_out.push(*unit),
            Err(error) => {
                roll_back(ctx, match_id, &checked_out);
                return Err(StartError::Custody {
                    unit: *unit,
                    detail: error.to_string(),
                });
            }
        }
    }

    // (4) Open. (5) And undo (3) if it refuses.
    let resolution = arena_def::resolution_for(&catalog);
    let custody = CustodyOut {
        definition,
        units: custody_units,
        pilots,
        resolution,
        seed,
    };
    if let Err(error) =
        crate::arena_bridge::open_match(ctx, match_id, custody, arena_cfg, match_cfg, sensors_cfg)
    {
        roll_back(ctx, match_id, &checked_out);
        return Err(StartError::Open(error));
    }

    // **`Boarding` -> `Running`** - the trigger itself, and the last thing that
    // happens, so a refusal above never leaves a match claiming to run.
    if let Some(record) = ctx
        .w
        .module_state_mut::<MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        record.state = MatchState::Running;
    }
    Ok(match_id)
}

/// Undo a partial check-out and leave the match `Aborted`.
///
/// Every unit goes back the way a void does — there was no arena, so there is no
/// arena state to commit — and no result document is written: doc 16 §3.8 is
/// explicit that an abort from `Boarding` releases slots and pods only, because
/// there was no match to have a result.
fn roll_back(ctx: &mut Ctx<'_>, match_id: MatchId, units: &[ObjId]) {
    if let Some(record) = ctx
        .w
        .module_state_mut::<MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        record.state = MatchState::Aborted;
    }
    for unit in units {
        if let Err(error) = mush_tactics::custody::check_in(ctx, *unit, None) {
            tracing::error!(
                %unit,
                %error,
                "tacsim: a rolled-back check-out could not be undone; the unit keeps its marker \
                 and staff must clear it"
            );
        }
    }
}

/// **The gunnery rating one unit's shots are resolved at** (16b cut W.2; doc 16
/// §3.5 step 2's `PilotProfile` seam), 1 to 5.
///
/// The **best** of the ratings its seated pilots carry, off their character sheets
/// through [`crate::sheet::PilotProfile`] — a tank with a gunner and a driver aboard
/// shoots with the gunner's hand, and asking which seat is nominally the gun would
/// be a rule about stations that doc 16 does not make (§3.7 puts every station on
/// the tank and lets one pilot hold all three channels).
///
/// [`crate::arena::resolution::STOCK_GUNNERY`] for an empty tank, for a pilot with
/// no sheet, and for a world with no sheet module composed: doc 16 §3.5 says *every
/// stock rating is 1*, and a resolution that stopped to ask staff for a sheet would
/// be a pulse that could not run.
fn gunnery_of(ctx: &Ctx<'_>, unit: ObjId) -> u8 {
    seated_pilots(ctx, unit)
        .into_iter()
        .map(|(pilot, _)| crate::sheet::profile_of(ctx.w, pilot).gunnery)
        .max()
        .unwrap_or(crate::arena::resolution::STOCK_GUNNERY)
}

/// Every actor seated on a station of `unit`, with the station's id.
fn seated_pilots(ctx: &Ctx<'_>, unit: ObjId) -> Vec<(ObjId, SmolStr)> {
    ctx.w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.assignments.get(&unit))
        .map(|host| {
            host.seats
                .iter()
                .map(|(station, seat)| (seat.actor, SmolStr::new(station.as_str())))
                .collect()
        })
        .unwrap_or_default()
}

/// A salt that makes two matches with the same id and the same world second —
/// two servers in one process, which is every integration test in the tree —
/// draw from different streams.
static SEED_SALT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The match seed: the id, the world's second and a process-local salt, folded.
///
/// Doc 16 §3.1 derives every draw from `(match_seed, label)`, so what a seed owes
/// is that a *replay* of one match's ledger re-derives that match's draws — which
/// it does, because the seed is recorded on the match. It does **not** owe
/// reproducibility from the id, and pretending it did would mean two servers
/// started in the same second shared a stream.
///
/// Never zero: the arena's fault-injection hooks are armed **by seed** and a seed
/// of `0` disarms them, so a match with seed `0` would be a match no fault could
/// be injected into — and never equal to another live match's, for the same
/// reason.
#[must_use]
pub fn seed_for(match_id: MatchId, world_now: i64) -> u64 {
    let salt = SEED_SALT
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .wrapping_add(0x2545_F491_4F6C_DD1D);
    let mixed = match_id
        .0
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(17)
        ^ (world_now as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ salt.wrapping_mul(0x94D0_49BB_1331_11EB).rotate_left(31);
    if mixed == 0 { 1 } else { mixed }
}

// ---------------------------------------------------------------------------
// Check-in: `Ended | Aborted | Void`
// ---------------------------------------------------------------------------

/// **What the arena returned, per unit** — doc 16 §3.1's seventh message, and
/// the seam **16b cut W.2** filled.
///
/// Doc 16 §3.1's seam carries `CustodyIn { units[{cells, final_pose, outcome}],
/// result, ledger_digest, reason }`. Cut T.2 could carry only the last three: there
/// was no damage model, so no cell inside a running arena could have changed, and
/// the one per-unit fact the arena produced was a pose — which it read back off the
/// ledger's `position_summary` events and the cockpit frames
/// ([`MatchRecord::last_pose`]).
///
/// Cut W.2 widened [`crate::arena_task::ArenaOut::Ended`] to carry
/// [`UnitReturn`] per unit, and this is the decode of that: the cells become one
/// [`mush_tactics::UnitMutationPlan`] against the check-out snapshot, the pose
/// becomes the four integers, and the outcome and the two counters become the result
/// record's per-unit row (doc 16 §4). `last_pose` is still the fallback for a unit
/// the arena's own message did not name.
#[derive(Clone, Debug, Default)]
pub struct CustodyIn {
    /// Per unit: the plan to apply, or `None` to restore the check-out snapshot
    /// (a `Void`, or a unit whose cells the match never moved).
    pub cells: BTreeMap<ObjId, Option<mush_tactics::UnitMutationPlan>>,
    /// Per unit: where the arena left it, as the four integers of doc 16 §3.2.
    /// Absent for a unit the arena never reported, and for every unit on a
    /// `Void`, which commits nothing the arena derived.
    pub poses: BTreeMap<ObjId, PoseNumbers>,
    /// **Per unit: what became of it, and its two counters** (16b cut W.2; doc 16
    /// §4's per-unit row). Absent on a `Void`, where nothing the arena derived may
    /// be committed — including a claim that a tank was destroyed.
    pub outcomes: BTreeMap<ObjId, (UnitOutcome, u32, u32)>,
}

/// Build the check-in message for one ended match (see [`CustodyIn`]).
///
/// `returned` is what the arena handed over with its end; `void` is doc 16 §3.1's
/// fail-closed case — a panic, a stall or a ledger overflow returns **nothing**, no
/// cell, no pose and no outcome, and every unit goes back to its check-out snapshot.
///
/// The plan is a **diff**, [`returned_plan`], between the unit the world still holds
/// and the unit the arena hands back. That is the honest shape: the world's unit was
/// frozen for the length of the match (the `deployed` marker refuses every ordinary
/// mutation), so the two differ in exactly what the arena changed, and a diff cannot
/// carry a cell the arena never touched.
#[must_use]
pub fn custody_in(
    ctx: &Ctx<'_>,
    record: &MatchRecord,
    returned: &[UnitReturn],
    void: bool,
) -> CustodyIn {
    let mut message = CustodyIn::default();
    for unit in record.units() {
        message.cells.insert(unit, None);
        if void {
            continue;
        }
        match returned.iter().find(|back| back.unit == unit) {
            Some(back) => {
                if let Some(held) = mush_tactics::module::unit_of(ctx.w, unit)
                    && let Some(plan) = returned_plan(held, &back.cells)
                {
                    message.cells.insert(unit, Some(plan));
                }
                message.poses.insert(unit, PoseNumbers::of(&back.pose));
                let outcome = if back.wreck {
                    UnitOutcome::Destroyed
                } else if back.disabled {
                    UnitOutcome::Disabled
                } else {
                    UnitOutcome::Intact
                };
                message
                    .outcomes
                    .insert(unit, (outcome, back.shots, back.hits));
            }
            // A unit the arena's message did not name — an `Ended` from a build
            // that carried none, or a match voided before it opened. The poses the
            // world saw on the feed are still the arena's own numbers.
            None => {
                if let Some(numbers) = record.last_pose.get(&unit) {
                    message.poses.insert(unit, *numbers);
                }
            }
        }
    }
    message
}

/// **The plan that turns `held` into `arena`** (16b cut W.2), or `None` when the two
/// already agree.
///
/// Doc 16 §3.1's check-in applies *the returned cells as one `UnitMutationPlan`
/// through `apply`*, and this is how a whole returned unit becomes one: every scalar
/// cell whose value moved is a `SetCell`, every section whose condition moved is a
/// `SetSectionCondition`, and every capability the arena revoked is a
/// `RevokeCapability`. Absolute `SetCell` rather than `AdjustCell` on purpose — the
/// arena's value is the truth and a delta would be one more thing that could be
/// applied twice.
///
/// **What it deliberately cannot carry**: a cell or a section the returned unit has
/// and the held one does not, a capability the arena *granted*, or any structural
/// change. Nothing in the arena produces one — damage moves cells, conditions and
/// revocations and nothing else (doc 16 §3.5) — and a check-in that could restructure
/// a persisted unit is a check-in that could restructure it wrongly.
#[must_use]
pub fn returned_plan(
    held: &mush_tactics::TacticalUnit,
    arena: &mush_tactics::TacticalUnit,
) -> Option<mush_tactics::UnitMutationPlan> {
    let mut ops: Vec<mush_tactics::UnitMutationOp> = Vec::new();
    for (addr, cell) in arena.cells() {
        if held
            .cell(addr)
            .is_some_and(|before| before.current != cell.current)
        {
            ops.push(mush_tactics::UnitMutationOp::SetCell {
                addr: addr.clone(),
                value: cell.current,
            });
        }
    }
    for (id, section) in arena.sections() {
        if held
            .section(id)
            .is_some_and(|before| before.condition != section.condition)
        {
            ops.push(mush_tactics::UnitMutationOp::SetSectionCondition {
                section: id.clone(),
                condition: section.condition.clone(),
            });
        }
    }
    for key in held.capabilities() {
        if arena.capability(key).is_none() {
            ops.push(mush_tactics::UnitMutationOp::RevokeCapability { key: key.clone() });
        }
    }
    if ops.is_empty() {
        return None;
    }
    match mush_tactics::UnitMutationPlan::new(ops, None, Some(SmolStr::new_static("arena:checkin")))
    {
        Ok(plan) => Some(plan),
        Err(error) => {
            // The ceiling is M32's `MAX_UNIT_MUTATION_OPERATIONS` and the stock tank
            // is nowhere near it; a unit with enough cells to reach it comes back
            // restored from its snapshot rather than half-updated, which is the same
            // fail-closed answer a `Void` gets.
            tracing::error!(
                %error,
                "tacsim: the returned cells would not fit one plan; the unit is restored instead"
            );
            None
        }
    }
}

/// **Check one match in, once** (doc 16 §3.1 "Custody", check-in).
///
/// Per unit and fallibly: the returned cells as one `UnitMutationPlan` through
/// `mush_tactics::apply`, the final pose through `mush_sim::place`, both in this
/// same authoritative action; a unit that no longer exists is journalled
/// `custody_lost` and skipped, **never failing the others**. Then the marker is
/// cleared (the same write as the cells), the tank is reset, the seats and focus
/// are released, the match is untracked and the result document is written.
///
/// Idempotent: a second call on the same match finds no live record and answers
/// `false`, having written nothing.
///
/// On `Void` — a panic, a stall or a ledger overflow — [`custody_in`] returns
/// nothing at all, so every unit is restored from its check-out snapshot and
/// **no arena-derived cell is committed**. The pose is restored too, for the
/// same reason: the arena's last frame is a fact the faulted pulse could have
/// produced.
pub fn finish(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    end: MatchEnd,
    ledger_digest: String,
    returned: &[UnitReturn],
) -> bool {
    let Some(record) = ctx
        .w
        .module_state::<MatchBook>()
        .and_then(|book| book.get(match_id))
        .filter(|record| record.state.is_live())
        .cloned()
    else {
        return false;
    };
    // Doc 16 §3.1 "Custody": *on `Void` (panic or ledger overflow) the world
    // restores each unit from the check-out snapshot and commits no
    // arena-derived cell*. The arena names the two apart in its own ledger; here
    // they are one thing.
    let void = matches!(end.reason, EndReason::Void | EndReason::LedgerOverflow);
    // Doc 16 §3.1's seventh message, built once and consumed below.
    let returned = custody_in(ctx, &record, returned, void);
    let mut results: Vec<UnitResult> = Vec::new();
    for unit in record.units() {
        // Treaty ruling 3: re-validate. An object destroyed while the match ran
        // is `custody_lost` — journalled, skipped, and never the reason another
        // unit stays deployed.
        if ctx.w.get(unit).is_none() {
            tracing::error!(
                %unit,
                match_id = match_id.0,
                "tacsim: a deployed unit no longer exists at check-in; its custody is lost"
            );
            mush_sim::deployed::clear(ctx.w, unit);
            results.push(UnitResult::lost(unit));
            continue;
        }
        // **16b cut W.2** fills `CustodyIn::cells`: the plan is the diff between the
        // unit the world still holds and the unit the arena hands back, and `None`
        // is either a void or a unit the match never marked.
        let plan = returned.cells.get(&unit).and_then(Option::as_ref);
        let checked_in = match mush_tactics::custody::check_in(ctx, unit, plan) {
            Ok(checked_in) => checked_in,
            Err(error) => {
                tracing::error!(
                    %unit,
                    match_id = match_id.0,
                    %error,
                    "tacsim: a unit could not be checked in; its marker is left for a staff repair"
                );
                results.push(UnitResult::lost(unit));
                continue;
            }
        };
        // The pose: the arena's own last numbers on an ordinary end, the
        // check-out pose on a void (doc 16 §3.1 — a void commits nothing the
        // arena derived). The posture is always the check-out snapshot's: the
        // arena has no posture and must not be read as having invented one.
        let pose = match (returned.poses.get(&unit), &checked_in.marker.pose) {
            (Some(numbers), Some(held)) => Some(numbers.onto(held.posture.clone())),
            (_, held) => held.clone(),
        };
        // Only when it MOVED. A unit the arena never drove is already at the
        // pose the world froze — its placement was left in place on purpose —
        // and re-placing it at the pose it is already at would be a second
        // spatial write for no change, which is exactly the noise doc 16 §3.1's
        // "the custody markers are the only unit writes during a match" exists
        // to keep out of the record.
        if let (Some(domain), Some(pose)) = (checked_in.marker.domain, pose)
            && !same_pose(mush_sim::pose_of(ctx.w, unit).as_ref(), domain, &pose)
            && let Err(error) = mush_sim::place(ctx, unit, domain, pose)
        {
            tracing::error!(
                %unit,
                match_id = match_id.0,
                %error,
                "tacsim: a checked-in unit could not be re-placed; it keeps its frozen placement"
            );
        }
        release_seat(ctx, unit);
        // **16b cut W.2**: doc 16 §4's per-unit row — what became of it and its two
        // counters, off the arena's own message. A unit the message did not name, and
        // every unit on a void, comes back `intact` with no counters, because that is
        // what the world is entitled to say about it.
        let (outcome, shots, hits) =
            returned
                .outcomes
                .get(&unit)
                .copied()
                .unwrap_or((UnitOutcome::Intact, 0, 0));
        results.push(UnitResult {
            unit: unit.0,
            outcome,
            shots,
            hits,
        });
    }

    // The registry's rows go last: `untrack` is what frees the epoch mirror and
    // the unit bindings the routing reads.
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.untrack(match_id);
    }
    if let Some(tasks) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        && let Some((handle, join)) = tasks.remove(match_id)
    {
        handle.signal_stop();
        join.abort();
    }

    let state = MatchState::from(end.reason);
    let outcome = MatchOutcome::from(end.reason);
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>()
        && let Some(record) = book.get_mut(match_id)
    {
        record.state = state;
        record.ended = Some(end);
        record.ledger_digest.clone_from(&ledger_digest);
    }
    record::write_result(ctx, match_id, outcome, end, &ledger_digest, &results);
    // 16b cut K.1: and the lobby's own half of the release — the pods and the
    // slots — in this same authoritative action, with the centre left in its
    // terminal state carrying the outcome until somebody resets it.
    lobby::on_match_finished(ctx, match_id, outcome);
    true
}

/// Release the seats, leases and focus of everyone stationed on `unit`.
///
/// Doc 16 §3.8: *seats, leases, focus and pods released atomically*. The pods are
/// cut C's (they do not exist yet); what exists is the station assignment, the
/// control lease behind it and the account focus pointed at it, and
/// `mush_sim::request_seat_activation(&SeatRequest::Revoke { .. })` is the one
/// seam that ends all three. The marker has already been cleared by the check-in
/// above, so the admission site this cut added does not refuse it.
/// Whether the unit's current placement already **is** the pose a check-in
/// would write: the same domain and the same four integers doc 16 §3.2 defines a
/// pose by. The opaque posture is deliberately not compared — the arena never
/// touches it, so a difference there is the world's own and must not be
/// overwritten.
fn same_pose(
    current: Option<&(mush_sim::DomainId, mush_sim::data::Pose)>,
    domain: mush_sim::DomainId,
    pose: &mush_sim::data::Pose,
) -> bool {
    current.is_some_and(|(at, held)| {
        *at == domain
            && held.x_cm == pose.x_cm
            && held.y_cm == pose.y_cm
            && held.z_cm == pose.z_cm
            && held.heading_mdeg == pose.heading_mdeg
    })
}

/// Release the cockpit seats of a unit the boot pass restored (16b T5).
///
/// The same seam [`release_seat`] uses; a separate name because the boot caller
/// is a different story and deserves to read as one.
pub fn release_seats_after_restart(ctx: &mut Ctx<'_>, unit: ObjId) {
    release_seat(ctx, unit);
}

fn release_seat(ctx: &mut Ctx<'_>, unit: ObjId) {
    let seats: Vec<(mush_sim::StationId, ObjId)> = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.assignments.get(&unit))
        .map(|host| {
            host.seats
                .iter()
                .map(|(station, seat)| (station.clone(), seat.actor))
                .collect()
        })
        .unwrap_or_default();
    for (station, actor) in seats {
        if let Err(error) = mush_sim::request_seat_activation(
            ctx,
            &mush_sim::SeatRequest::Revoke {
                unit,
                station: station.clone(),
                npc: actor,
            },
        ) {
            tracing::debug!(
                %unit,
                station = station.as_str(),
                %error,
                "tacsim: a cockpit seat did not stand down at check-in"
            );
        }
        // **16b cut K.2.** A stand-down clears the narrowing and keeps the
        // billet, which is right for an NPC's own station and wrong for a pod
        // lent for one match: the billet IS the loan, and one left behind is the
        // next pilot's `StationHeld`. Doc 16 §3.8's "seats … released
        // atomically" is this line as much as the one above it.
        if let Err(error) = mush_sim::vacate_station(ctx, unit, &station, actor) {
            tracing::error!(
                %unit,
                station = station.as_str(),
                %error,
                "tacsim: a cockpit billet could not be given back at check-in"
            );
        }
    }
}

/// Stop one match, for whatever reason (`@arena/abort`, `SIM ABORT`, a timeout).
///
/// The stop is honoured at the arena's next pulse, and the check-in happens when
/// the arena reports its end — never here. A match whose task is already gone is
/// finished on the spot instead, so an abort always has an effect.
pub fn abort(ctx: &mut Ctx<'_>, match_id: MatchId, reason: StopReason) -> bool {
    if crate::arena_bridge::send_stop(ctx, match_id, reason).is_ok() {
        if let Some(book) = ctx.w.module_state_mut::<MatchBook>()
            && let Some(record) = book.get_mut(match_id)
        {
            record.state = MatchState::Stopping;
        }
        return true;
    }
    // No task: the world is the only thing left holding the match, so end it
    // here rather than leaving units deployed to nothing.
    // No units come back with it: there is no arena to return them, so every one
    // of them is restored from its check-out snapshot (doc 16 §3.1's fail-closed
    // read, the same one a void takes).
    finish(
        ctx,
        match_id,
        MatchEnd {
            reason: EndReason::Abort,
            t_ms: 0,
        },
        String::new(),
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counter_never_lowers_and_never_repeats() {
        let mut book = MatchBook::new();
        assert!(!book.is_loaded());
        book.adopt_seq(9);
        assert_eq!(book.peek_seq(), 10);
        book.adopt_seq(4);
        assert_eq!(
            book.peek_seq(),
            10,
            "a lower persisted value never lowers it"
        );
        assert!(book.is_loaded());
    }

    #[test]
    fn a_match_seed_is_never_zero_and_never_repeats() {
        assert_ne!(seed_for(MatchId(1), 0), 0);
        assert_ne!(seed_for(MatchId(0), 0), 0);
        assert_ne!(seed_for(MatchId(1), 100), seed_for(MatchId(2), 100));
        // The same id in the same second, twice: two servers in one process.
        assert_ne!(seed_for(MatchId(1), 100), seed_for(MatchId(1), 100));
    }
}
