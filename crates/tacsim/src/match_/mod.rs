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
//! Running`, the check-out**, as a function ([`crate::match_::start`]) cut K calls — and the
//! three ends, [`crate::match_::finish`], which is check-in.
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
/// 16b cut Z.4 (16c `N8`): **the side nets** — `RedNet` and `BlueNet`, comsys
/// channels made at boot, joined at the check-out and left with the seat.
pub(crate) mod net;
// 16b cut I.4: the staff picture's payload, `arena.observer` — deliberately
// its own module and never the cockpit's, which builds only what a pilot is
// sent. Documented by its own module doc.
pub mod observer;
mod record;
// 16b cut AF.3 (16c N16): the boot's resume — phase A's named checks, phase
// B's re-open, the seats reconciled and the void by name. Documented by its
// own module doc.
pub mod resume;
// 17b cut M3.3 (doc 17 §12, §13; D-UM-45, D-UM-46): the seal, computed once
// from the finished custody. Documented by its own module doc.
pub mod seal;
// 16b cut I.1: the replay — the pure fold from the tape to one objective
// picture of both sides, the replay documents, `SIM REPLAY` and the two queries'
// reads. Documented by its own module doc, as `report` is.
pub mod replay;
// 16b cut V.3: the debrief — the pure fold from the tape to one reader's report,
// the report documents, `SIM REPORT`, `@arena/report` and the module's store-read
// completion. Documented by its own module doc (an outer doc here would move its
// links into this module's scope).
pub mod report;
/// 16b cut F.1: **the stock-tank reset** — doc 16 §3.3's *at every `Idle`
/// transition each is reset to its `UnitSpec` by one `UnitMutationPlan`*.
pub mod reset;
pub mod sim;
// 16b cut AH.1 (16c N18): the first-time hints — five fixed lines, each said
// once per ACCOUNT at its point of a first match, the `tacsim:tutorial:`
// record that remembers them, `@tutorial` and the lobby's `Moved` observer.
// Documented by its own module doc.
pub mod tutorial;
mod verbs;

pub use arena_def::{
    ARENA_CELL_CM, ARENA_CELLS, PROVING_GROUND, STAND_IN, custody_unit, definition_for,
    opening_cell_pose, opening_pose, resolution_for, sensor_height_cm, sensor_profile,
    stock_definition, stock_resolution, tracked_rules, unit_profile, weapon_summaries,
};
pub(crate) use centre::record_roster;
pub use centre::{
    ArenaRoster, BLUE_ROOM_ENTRY, CENTRE_ARENA, CENTRE_PACK, CentreRoom, LOBBY_ENTRY, POD_ENTRIES,
    PODS_PER_SIDE, RED_ROOM_ENTRY, ROSTER_PREFIX, Readiness as CentreReadiness, centre_object,
    centre_rooms, fieldable_per_side, pod_object, pod_slot, readiness as centre_readiness,
    room_role, roster_from_document, roster_id_of_key, roster_key, slot_id, slot_object,
    tank_for_slot, tank_of, where_in_centre,
};
pub use cockpit::{
    BOARD_VERB, COCKPIT_VERBS, CockpitError, CockpitGate, EXIT_VERB, EjectReport, NO_LOCK,
    NO_SUCH_CONTACT, NOT_IN_A_POD, NOT_STARTED_YET, OUTSIDE_COCKPIT, OUTSIDE_TEAM_ROOM,
    POWER_NOT_ONLINE, POWER_STARTING, STATION, Seat, board as cockpit_board, exit as cockpit_exit,
    flying,
};
pub use lobby::{
    BoardReader, Centre, HoldWords, OUTSIDE_CENTRE, ResetOutcome, SimError, Slot,
    abort as sim_abort, ai as sim_ai, ai_side_size, authored_unit_type, board_deadline,
    centre as centre_of, claim, clock_text, concede as sim_concede, hold_words, largest_side,
    leave, mode as sim_mode, pilot_of, pod_of, range as sim_range, range_sides,
    reset as reset_centre, slot_seated, slot_vacated, slots_per_side, start as sim_start,
    status_lines, unattended_deadline, unattended_line, unit_type as sim_unit_type,
};
pub use net::{BLUE_NET, RED_NET, net_name};
pub use record::{
    HISTORY_DEPTH, HistoryRow, HistoryUnit, LIVE_FORMAT, LIVE_RECORD, LiveRecord, LiveRecordError,
    MatchOutcome, MatchRecord, MatchState, MindSealRecord, NO_HISTORY, PoseNumbers, TeamRoster,
    UnitOutcome, UnitResult, arena_label, civil_utc, delete_live_and_snapshot, duration_words,
    end_words, history_lines, live_id_of_key, live_key, match_id_of_key, outcome_words, report_key,
    result_key, snapshot_id_of_key, snapshot_key, write_live, write_replay, write_report,
    write_restart_abort,
};
pub use reset::{RESET_CAUSE, ResetReport, reset_stock_tanks, spec_plan};
pub use resume::{
    PendingResume, RESUME_LINE, ResumeRefusal, doctrine_line, resume_pending, void_at_boot,
};
pub use seal::{
    NO_CORE_FALLBACK, SEALED_CHARTER, SEALED_LICENCE, SEALED_LINE, SEALED_MIND, SealError,
    SealTerms, seal_for,
};
pub use sim::{SIM_VERB, Selector, sim_command};
pub use verbs::{
    ArenaVerbError, MIND_SET_USAGE, arena_command, mind_set, register_switches, state_word,
};

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use mush_core::{Ctx, ObjId};
use smol_str::SmolStr;

use crate::arena::damage::UnitReturn;
use crate::arena::state::{
    CustodyOut, CustodyUnit, EndReason, MatchEnd, MatchMode, Pilot, StopReason, Team,
};
use crate::arena_bridge::{ArenaRegistry, OpenMatchError, match_log_id};
use crate::arena_task::MatchId;
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
use crate::deliberation::order::CharacterRef;

/// The persisted key of the match counter, inside the tacsim namespace.
pub const SEQ_KEY: &str = "tacsim:match:seq";

/// **How long a boot's hatch line waits for its pilot's session** (16b cut U.4,
/// 16c `B-37`), in world seconds from the first boarding tick that holds it.
///
/// The boot's pod sweep owes each body it moved one line, said by the module's
/// tick. On the split server a pilot's SSH session survives a restart of the
/// server half, but the world only learns it is attached when the gateway's
/// re-announce reaches it — after the boot, and not ordered against the first
/// tick. So a line whose body is not yet connected is **held** rather than
/// said to nobody, and said on the first tick that finds the body connected.
/// This bounds the wait: a pilot who is not back by then logged off across the
/// restart, and a hatch line an hour later would be about nothing they saw.
pub const HATCH_HOLD_S: i64 = 60;

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
///
/// **The history ring** (16b cut V.2; 16c N2, the result half). The book also
/// remembers the centre's last [`HISTORY_DEPTH`] finished matches, newest first,
/// as one [`HistoryRow`] each ([`MatchBook::remember`]). The ring is **RAM**,
/// filled at boot from the result documents the module's boot data already
/// carries (`boot_matches` in the crate root, before its voids — 16b cut AF.3's phase A
/// — write anything) and kept current by every result write
/// (`record::write_result`, [`write_restart_abort`]), each of which remembers
/// the very document it pushes. It is what `SIM HISTORY` reads, and what 16b cut
/// V.3's `SIM REPORT` will read — **never the store**: the depth is past
/// [`mush_core::MAX_STORE_READ_KEYS`], and every result document is already a
/// boot document, so a store read would fetch what the book was handed at boot.
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
    /// **16b cut U.4 (16c `B-37`)** — the hatch lines the boot's pod sweep owes,
    /// as `(body, pod)`. See [`MatchBook::defer_hatch`].
    hatch_notices: Vec<(ObjId, ObjId)>,
    /// **16b cut U.4** — the world second after which a hatch line still owed
    /// is dropped unsaid, set by the first boarding tick that holds one. See
    /// [`HATCH_HOLD_S`].
    hatch_hold_until_s: Option<i64>,
    /// **16b cut U.4 (16c `T-52`)** — the opening cells staff chose with
    /// `@arena/pose`, by `(team, slot)`. See [`MatchBook::set_opening_cell`].
    ///
    /// Beside the centre and deliberately **not inside it**: every return to
    /// `Idle` and every staff `@arena/start` rebuilds the centre from
    /// [`lobby::Centre::default`], and a player's lone `SIM TEAM` on a terminal
    /// centre is one of those returns — an override kept on the centre would be
    /// wiped by the very claim that starts the match it was set for.
    opening_cells: BTreeMap<(Team, u8), OpeningCell>,
    /// **16b cut U.4** — the match whose check-out used [`Self::opening_cells`],
    /// so that match's end, and no other, clears them.
    opening_consumed_by: Option<MatchId>,
    /// **16b cut V.2** — the last [`HISTORY_DEPTH`] finished matches, in
    /// **descending id order** (newest first). See [`MatchBook::remember`].
    history: VecDeque<HistoryRow>,
    /// **16b cut V.3** — the store reads in flight, one per actor: a `SIM
    /// REPORT` or an `@arena/report` page waiting on
    /// [`report::ReportRead`]. RAM, like the rest of the book: a read is a view,
    /// and one lost to a restart is asked again.
    pending_reads: BTreeMap<ObjId, report::PendingRead>,
    /// **16b cut I.1 (16c N9, Q37)** — the replay of every remembered match
    /// that has one, by id: held by [`finish`] beside its document, filled at
    /// boot from the `tacsim:replay:<match>` documents whose match the ring
    /// remembers, and forgotten with the ring's row
    /// ([`MatchBook::forget_replay`]). At most [`HISTORY_DEPTH`] of at most
    /// [`replay::REPLAY_MAX_BYTES`] each.
    replays: BTreeMap<MatchId, Arc<replay::Replay>>,
    /// **16b cut I.1** — the ground each remembered replay was fought on,
    /// **keyed by arena digest**, so twenty lobby matches on one proving ground
    /// hold one copy. [`finish`] takes the registry's `Arc` before `untrack`
    /// drops it (`crate::arena_bridge::ground_of`); after a boot
    /// `replay::ground_for` builds one once per digest. A ground no remembered
    /// replay names is dropped with the last replay that did.
    grounds: BTreeMap<String, Arc<crate::arena::state::ArenaDefinition>>,
    /// **16b cut I.1** — the digests whose ground `replay::ground_for` built and
    /// found to be a different ground (a pack changed since the match), so it
    /// is built once and never again. Pruned with the grounds.
    ground_mismatch: BTreeSet<String>,
    /// **16b cut I.1** — each actor's replay cursor
    /// ([`replay::ReplayCursor`]): what `SIM REPLAY` writes and what the
    /// `arena.replay` and `arena.map` queries read for that actor. RAM, never
    /// persisted.
    replay_cursors: BTreeMap<ObjId, replay::ReplayCursor>,
    /// **16b cut AF.3 (16c N16)** — the matches the boot's phase A found
    /// resumable, in id order, waiting for phase B (the crate's
    /// `on_all_loaded`). Empty outside the boot.
    pending_resumes: Vec<resume::PendingResume>,
    /// **16b cut AF.3** — the two lines the resume owes each pilot whose seat
    /// it released, as `(body, tank)`. See [`MatchBook::defer_resume_lines`].
    resume_lines: Vec<(ObjId, ObjId)>,
    /// **16b cut AF.3** — the world second after which a resume line still owed
    /// is dropped unsaid, set by the first boarding tick that holds one (the
    /// hatch lines' [`HATCH_HOLD_S`], for the same reason).
    resume_hold_until_s: Option<i64>,
}

/// **One opening cell staff chose for a centre slot** (16b cut U.4, 16c `T-52`;
/// `@arena/pose`): an arena cell on the centre's arena and the heading the unit
/// opens at.
///
/// A cell and not a [`mush_sim::data::Pose`]: the pose carries the unit's own
/// opaque posture, which is the unit's at check-out and nobody's at set time, and
/// a cell is what rides [`Fielded`] (`Copy + Eq`, which a `Pose` is not). The
/// check-out turns it into the world pose at the cell's centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpeningCell {
    /// The arena cell's column.
    pub x: u16,
    /// The arena cell's row.
    pub y: u16,
    /// The opening heading, millidegrees clockwise from north, `0..360_000`.
    pub heading_mdeg: u32,
}

impl MatchBook {
    /// An empty book.
    #[must_use]
    pub fn new() -> MatchBook {
        MatchBook::default()
    }

    /// **Owe one body its hatch line** (16b cut U.4, 16c `B-37`).
    ///
    /// The boot's pod sweep ([`cockpit::eject_all_at_boot`], run from the
    /// module's `on_all_loaded`) moves bodies out of the pods and gives the
    /// cockpit billets back **inside the boot**, where every module's load has
    /// run and the content latch the pods resolve through is full. What it
    /// cannot do there is *tell* anybody: boot output is dropped — no session is
    /// attached yet — and on the split server, which is production, a pilot's
    /// SSH session survives a restart of the server half and is waiting to read
    /// why it is standing in the team room. So each `(body, pod)` is kept here,
    /// and the module's boarding tick ([`sim`]'s `board_tick`) takes the list
    /// and says the hatch line and the room to each body whose session is
    /// attached — holding the rest for at most [`HATCH_HOLD_S`], because on the
    /// split server the gateway's re-announce is not ordered against that tick.
    /// Only the boot adds to it, so a pilot who boards later in the same boot is
    /// never told a hatch opened.
    pub(crate) fn defer_hatch(&mut self, body: ObjId, pod: ObjId) {
        self.hatch_notices.push((body, pod));
    }

    /// The hatch lines still owed, as `(body, pod)`, in the order the sweep
    /// opened them. Empty once each has been said, or dropped at the hold.
    #[must_use]
    pub fn hatch_notices(&self) -> &[(ObjId, ObjId)] {
        &self.hatch_notices
    }

    /// Take every owed hatch line. A second caller gets nothing until the
    /// first hands back what it could not say yet
    /// (`MatchBook::hold_hatch_notices`).
    pub fn take_hatch_notices(&mut self) -> Vec<(ObjId, ObjId)> {
        std::mem::take(&mut self.hatch_notices)
    }

    /// The world second the owed hatch lines are held until, once a boarding
    /// tick has started holding them ([`HATCH_HOLD_S`]).
    #[must_use]
    pub const fn hatch_hold_until_s(&self) -> Option<i64> {
        self.hatch_hold_until_s
    }

    /// Hand back the hatch lines a tick could not say yet — their bodies have no
    /// session attached — to be held until `until_s`. An empty hand-back ends
    /// the hold.
    pub(crate) fn hold_hatch_notices(&mut self, held: Vec<(ObjId, ObjId)>, until_s: i64) {
        self.hatch_hold_until_s = (!held.is_empty()).then_some(until_s);
        self.hatch_notices = held;
    }

    /// **Owe one pilot the resume's two lines** (16b cut AF.3; 16c N16,
    /// `Q-AF4`): [`RESUME_LINE`], then Y.3's [`doctrine_line`] for `tank`.
    ///
    /// The boot's resume releases every seat to doctrine, and it cannot say so:
    /// boot output reaches no session, and on the split server, which is
    /// production, the pilot's session survived the restart of the server half
    /// and still shows the abandoned timeline's last cockpit frame. So each
    /// `(body, tank)` is kept here, and the boarding tick ([`sim`]'s
    /// `board_tick`) says both lines — and clears the panel — to a body whose
    /// session is attached while its slot is still dropped, holding the rest
    /// for at most [`HATCH_HOLD_S`]. Only the resume adds to it.
    pub(crate) fn defer_resume_lines(&mut self, body: ObjId, tank: ObjId) {
        self.resume_lines.push((body, tank));
    }

    /// Whether the book still owes `actor` the resume's lines — what
    /// `SeatFacts::connected` asks, so a pilot reads them once whichever path
    /// reaches them first.
    #[must_use]
    pub fn resume_lines_deferred(&self, actor: ObjId) -> bool {
        self.resume_lines.iter().any(|(body, _)| *body == actor)
    }

    /// Take every owed resume line, as `(body, tank)`. A second caller gets
    /// nothing until the first hands back what it could not say yet.
    pub fn take_resume_lines(&mut self) -> Vec<(ObjId, ObjId)> {
        std::mem::take(&mut self.resume_lines)
    }

    /// The world second the owed resume lines are held until, once a boarding
    /// tick has started holding them.
    #[must_use]
    pub const fn resume_hold_until_s(&self) -> Option<i64> {
        self.resume_hold_until_s
    }

    /// Hand back the resume lines a tick could not say yet, held until
    /// `until_s`. An empty hand-back ends the hold.
    pub(crate) fn hold_resume_lines(&mut self, held: Vec<(ObjId, ObjId)>, until_s: i64) {
        self.resume_hold_until_s = (!held.is_empty()).then_some(until_s);
        self.resume_lines = held;
    }

    /// **The matches waiting for the boot's phase B** (16b cut AF.3), in id
    /// order.
    #[must_use]
    pub fn pending_resumes(&self) -> &[resume::PendingResume] {
        &self.pending_resumes
    }

    /// **The opening cells overridden for the next match** (16b cut U.4), by
    /// `(team, slot)`, in team-then-slot order.
    #[must_use]
    pub fn opening_cells(&self) -> &BTreeMap<(Team, u8), OpeningCell> {
        &self.opening_cells
    }

    /// One slot's overridden opening cell, if staff set one.
    #[must_use]
    pub fn opening_cell(&self, team: Team, slot: u8) -> Option<OpeningCell> {
        self.opening_cells.get(&(team, slot)).copied()
    }

    /// The match whose check-out used the overrides, if one has since the last
    /// time they were cleared.
    #[must_use]
    pub const fn opening_consumed_by(&self) -> Option<MatchId> {
        self.opening_consumed_by
    }

    /// Set one slot's opening cell (`@arena/pose <callsign>=<x> <y>`), replacing
    /// any it had. The caller has validated it.
    pub(crate) fn set_opening_cell(&mut self, team: Team, slot: u8, cell: OpeningCell) {
        self.opening_cells.insert((team, slot), cell);
    }

    /// Give one slot its authored cell back (`@arena/pose <callsign>=off`).
    pub(crate) fn clear_opening_cell(&mut self, team: Team, slot: u8) {
        self.opening_cells.remove(&(team, slot));
    }

    /// Clear every override (`@arena/reset`, and the end of the match that used
    /// them).
    pub(crate) fn clear_opening_cells(&mut self) {
        self.opening_cells.clear();
        self.opening_consumed_by = None;
    }

    /// Record that `match_id`'s check-out used the overrides (the lobby's
    /// `Boarding → Running`), so its end clears them.
    pub(crate) fn consume_openings(&mut self, match_id: MatchId) {
        self.opening_consumed_by = Some(match_id);
    }

    /// The end of `match_id`: clears the overrides when that match is the one
    /// that used them, and leaves them alone for any other.
    pub(crate) fn release_openings(&mut self, match_id: MatchId) {
        if self.opening_consumed_by == Some(match_id) {
            self.clear_opening_cells();
        }
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

    /// **Remember one finished match** (16b cut V.2): replaces a row of the same
    /// id, inserts in **descending id order** whatever the order of the calls,
    /// truncates to [`HISTORY_DEPTH`] and answers the row that fell off, if one
    /// did.
    ///
    /// The order is the ids' and never the calls': the boot remembers the result
    /// documents in whatever order its key-value pairs arrive and then writes its
    /// restart aborts, and a result written later in the boot can name a lower id
    /// than one the boot already read. The row that falls off is the lowest id,
    /// which can be `row` itself when the ring is full of newer ones.
    pub fn remember(&mut self, row: HistoryRow) -> Option<HistoryRow> {
        self.history.retain(|held| held.id != row.id);
        let at = self
            .history
            .iter()
            .position(|held| held.id < row.id)
            .unwrap_or(self.history.len());
        self.history.insert(at, row);
        if self.history.len() > HISTORY_DEPTH {
            self.history.pop_back()
        } else {
            None
        }
    }

    /// The remembered matches, newest first (16b cut V.2).
    pub fn history(&self) -> impl Iterator<Item = &HistoryRow> {
        self.history.iter()
    }

    /// **The store read in flight for `actor`** (16b cut V.3), if any.
    #[must_use]
    pub fn pending_read(&self, actor: ObjId) -> Option<&report::PendingRead> {
        self.pending_reads.get(&actor)
    }

    /// One remembered match by id, if the ring still holds it (16b cut V.2).
    #[must_use]
    pub fn remembered(&self, id: MatchId) -> Option<&HistoryRow> {
        self.history.iter().find(|row| row.id == id)
    }

    /// **The replay of a remembered match** (16b cut I.1), if the ring holds
    /// one for it.
    #[must_use]
    pub fn replay(&self, id: MatchId) -> Option<Arc<replay::Replay>> {
        self.replays.get(&id).cloned()
    }

    /// **The ground held under an arena digest** (16b cut I.1), if any. A
    /// read: a ground not yet built after a boot is built by `SIM REPLAY`
    /// (`replay::ground_for`), never here.
    #[must_use]
    pub fn ground(&self, digest: &str) -> Option<Arc<crate::arena::state::ArenaDefinition>> {
        self.grounds.get(digest).cloned()
    }

    /// How many grounds the book holds (16b cut I.1): one per distinct arena
    /// digest a remembered replay names.
    #[must_use]
    pub fn ground_count(&self) -> usize {
        self.grounds.len()
    }

    /// Whether a digest's ground was built and found not to match (16b cut
    /// I.1), so it will not be built again.
    #[must_use]
    pub fn ground_mismatched(&self, digest: &str) -> bool {
        self.ground_mismatch.contains(digest)
    }

    /// **One actor's replay cursor** (16b cut I.1), if they have a replay
    /// open.
    #[must_use]
    pub fn replay_cursor(&self, actor: ObjId) -> Option<replay::ReplayCursor> {
        self.replay_cursors.get(&actor).copied()
    }

    /// **Drop one actor's replay cursor** (16b cut I.4): a watch closes the
    /// actor's replay, one picture at a time. Answers whether one went.
    pub fn drop_replay_cursor(&mut self, actor: ObjId) -> bool {
        self.replay_cursors.remove(&actor).is_some()
    }

    /// **Hold one match's replay on the ring** (16b cut I.1), replacing any
    /// held under that id — what [`finish`] and the boot do. `pub` so an
    /// integration test can hand-edit one, as [`MatchBook::remember`] is.
    pub fn hold_replay(&mut self, id: MatchId, replay: Arc<replay::Replay>) {
        self.replays.insert(id, replay);
    }

    /// **Hold a ground under its digest** (16b cut I.1), keeping the first one
    /// held: twenty matches on one ground share one copy.
    pub fn hold_ground(
        &mut self,
        digest: String,
        ground: Arc<crate::arena::state::ArenaDefinition>,
    ) {
        self.grounds.entry(digest).or_insert(ground);
    }

    /// **Drop the ground held under a digest** (16b cut I.1) — for a test that
    /// stands in for a boot, after which no ground is held until `SIM REPLAY`
    /// asks for one.
    pub fn drop_ground(&mut self, digest: &str) {
        self.grounds.remove(digest);
    }

    /// **Mark a digest's ground as known not to match** (16b cut I.1), as
    /// `replay::ground_for` does after the one build that found a different
    /// ground — for a test that proves a marked digest is never built again:
    /// marked under a replay's REAL digest, whose build would succeed, only the
    /// mark can refuse it.
    pub fn mark_ground_mismatch(&mut self, digest: &str) {
        self.ground_mismatch.insert(digest.to_owned());
    }

    /// **Forget one match's replay** (16b cut I.1): the entry, every cursor on
    /// it, and any ground (or mismatch mark) no remembered replay names any
    /// more. Called by the ring's eviction, in the same action that deletes the
    /// replay document.
    pub fn forget_replay(&mut self, id: MatchId) {
        self.replays.remove(&id);
        self.replay_cursors
            .retain(|_, cursor| cursor.match_id != id);
        let named: BTreeSet<String> = self
            .replays
            .values()
            .map(|replay| replay.arena_digest.clone())
            .collect();
        self.grounds.retain(|digest, _| named.contains(digest));
        self.ground_mismatch.retain(|digest| named.contains(digest));
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

/// **One unit a side fields, and the pod it came out of** (16b cut Q.1).
///
/// The slot is what [`crate::arena::state::Designator::Slot`] renders, so the
/// `BA` a pilot locks is the pod `SIM STATUS` prints as `1` on blue. `0` is *no
/// pod*: a staff `@arena/start`, a fixture, the harness's field of three
/// hundred — and `Arena::new` then gives the unit the lowest free slot of its
/// team in unit-id order, so every unit in every match has a stable callsign
/// whether a lobby fielded it or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fielded {
    /// The unit.
    pub unit: ObjId,
    /// Its 1-based centre slot, or `0`.
    pub slot: u8,
    /// **16b cut U.4 (16c `T-52`)** — the opening cell staff chose for this
    /// slot with `@arena/pose`, or `None` for the unit's own world pose and the
    /// entry zone behind it. [`start`] offers it to [`opening_pose`] as the
    /// world pose, so it is held to exactly what a staff placement is held to.
    pub opening: Option<OpeningCell>,
}

impl Fielded {
    /// A unit no lobby placed.
    #[must_use]
    pub const fn unslotted(unit: ObjId) -> Fielded {
        Fielded {
            unit,
            slot: 0,
            opening: None,
        }
    }
}

/// One side of a match, as [`start`] is given it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    /// Which side.
    pub team: Team,
    /// Its units, in the order they were named, each with the centre slot it was
    /// fielded from (16b cut Q.1).
    pub units: Vec<Fielded>,
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
    /// **16b cut O.1** — the ground does not declare the mode the match was
    /// asked for ([`crate::arena::state::ArenaDefinition::supports`]). Refused
    /// right after the definition is resolved, before anything is written. No
    /// production composition reaches it from the lobby — the catalog is
    /// compiled in and the centre's arena is always the proving ground, which
    /// declares `hold` — so it is the direct caller's refusal.
    #[error("arena {arena} does not support {mode}")]
    ModeUnsupported {
        /// The mode's wire word.
        mode: &'static str,
        /// The arena's id.
        arena: SmolStr,
    },
    /// **17b cut M3.3** (doc 17 §13, D-UM-17): a unit's seal refused its card
    /// against the Compact floor — refused, never clamped. Reachable from an
    /// operator floor above the Line card's 10 per cent
    /// (`[tacsim.minds].odds_floor_permille` above 100) until 17b cut M5 lets
    /// a principal choose a card. Refused before the first write.
    #[error(
        "{callsign}'s card asks for odds under the Compact floor of {}%; the match cannot start.",
        .floor / 10
    )]
    CardUnlawful {
        /// The unit.
        unit: ObjId,
        /// How the unit is named before the arena resolves a callsign: its
        /// pod's designator (`RA`) when the lobby fielded it from a slot, else
        /// the unit's world name (a staff `@arena/start`).
        callsign: String,
        /// The floor, per mille.
        floor: u32,
    },
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
///
/// **16b cut O.1**: `mode` is the rule the match is fought under (the lobby's
/// `SIM MODE`; `@arena/start` passes [`MatchMode::Skirmish`]), refused with
/// [`StartError::ModeUnsupported`] when the ground does not declare it, and
/// carried on the [`CustodyOut`] and the [`MatchRecord`].
///
/// **17b cut M1b.5**: `fill` is the controller every hull with no seated pilot
/// is checked out under — `[tacsim.minds].fill_controller`
/// ([`crate::config::MindsConfig::fill_controller`], read by the callers
/// through `lobby::minds_config`): [`Controller::Reflex`] in production,
/// [`Controller::Mind`] on the lab's knob, when `Arena::new` schedules each
/// mind's first think and `Controller::is_machine` deals it into a squad as a
/// reflex hull is (17b cut L.2). A seated hull is `Pilot` whatever `fill` says.
///
/// [`Controller::Reflex`]: crate::arena::state::Controller::Reflex
/// [`Controller::Mind`]: crate::arena::state::Controller::Mind
#[allow(clippy::too_many_arguments)]
pub fn start(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    arena: &SmolStr,
    sides: &[Side],
    arena_cfg: &ArenaConfig,
    match_cfg: &MatchConfig,
    sensors_cfg: &SensorsConfig,
    mode: MatchMode,
    fill: crate::arena::state::Controller,
) -> Result<MatchId, StartError> {
    // 16b cut C.2: the authored arena when a loaded pack declares one, and the
    // stand-in window of T.3 otherwise.
    let definition = arena_def::definition_for(ctx.w, arena)
        .map_err(StartError::NoGround)?
        .ok_or_else(|| StartError::NoArena(arena.clone()))?;
    // **16b cut O.1** (16c N11): a mode the ground does not declare is refused
    // here, before the first write, as the arena itself would refuse it
    // (`ArenaError::ModeUnsupported`).
    if !definition.supports(mode) {
        return Err(StartError::ModeUnsupported {
            mode: mode.as_str(),
            arena: arena.clone(),
        });
    }

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
        for fielded in &side.units {
            let unit = fielded.unit;
            if seen.contains(&unit) {
                return Err(StartError::DuplicateUnit(unit));
            }
            seen.push(unit);
            mush_sim::deployed::refuse_if_deployed(ctx.w, unit)?;
            // "Is it a unit at all" before "is it placed": an object that is
            // neither wants the first answer, not the second.
            if mush_tactics::module::unit_of(ctx.w, unit).is_none() {
                return Err(StartError::NotAUnit(unit));
            }
            if mush_sim::pose_of(ctx.w, unit).is_none() {
                return Err(StartError::NotPlaced(unit));
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
    // **17b cut M3.3** (D-UM-17): the Compact floor every seal's card is
    // validated against — the same `[tacsim.minds]` read `open_match` hands
    // the arena, so the seal and the arena hold one floor.
    let floor = lobby::minds_config(ctx.w)
        .unwrap_or_default()
        .odds_floor_permille;
    for side in sides {
        let mut roster = TeamRoster {
            team: side.team,
            units: side.units.iter().map(|fielded| fielded.unit).collect(),
            pilots: Vec::new(),
        };
        for fielded in &side.units {
            let unit = fielded.unit;
            let cells = mush_tactics::module::unit_of(ctx.w, unit)
                .ok_or(StartError::NotAUnit(unit))?
                .clone();
            let world_pose = mush_sim::pose_of(ctx.w, unit).map(|(_, pose)| pose);
            // **16b cut U.4 (16c `T-52`)**: an `@arena/pose` override stands in
            // for the world pose — the cell's centre at its heading, on the
            // unit's own posture — so it is validated exactly as a staff
            // placement is (on the window, a way out, not taken) and falls back
            // to the entry zone the same way.
            let offered = match fielded.opening {
                Some(cell) => world_pose.as_ref().map(|pose| {
                    arena_def::opening_cell_pose(&definition, cell, pose.posture.clone())
                }),
                None => world_pose,
            };
            // **16b cut AE.1** (16c `B-58`): the cell is judged on the unit's own
            // row, read off the same record `Arena::new` will read it from, so a
            // car left on rubble opens in its zone and is not refused there.
            let row = crate::arena::state::CostRow::read(&cells);
            let pose =
                arena_def::opening_pose(&definition, side.team, offered.clone(), &mut taken, row)
                    .ok_or(StartError::NoEntryCell(side.team.as_str()))?;
            if let Some(cell) = fielded.opening
                && offered.as_ref() != Some(&pose)
            {
                tracing::warn!(
                    team = side.team.as_str(),
                    slot = fielded.slot,
                    x = cell.x,
                    y = cell.y,
                    "tacsim: an @arena/pose opening cell could not be used (off the window, no \
                     way out, or already taken); the unit opens in its entry zone instead"
                );
            }
            // **16b cut E.1** lifted the assembly into `arena_def::custody_unit`
            // (16b §0.4), so a shipped-scale test rig builds the same unit this
            // loop does instead of a second copy of it.
            // 17b cut M1b.5: an unseated hull is the fill's.
            let seated = seated_pilots(ctx, unit);
            let controller = if seated.is_empty() {
                fill
            } else {
                crate::arena::state::Controller::Pilot
            };
            // **17b cut M3.3** (doc 17 §13, §19; D-UM-45): the seal's terms.
            // The warrant principal is the character who chose the mind — the
            // hull's first seated pilot, else the side lead (a fill hull; the
            // lead set what the fill flies), else nobody: a staff
            // `@arena/start` match, or a side with no lead, tells nobody.
            let principal = match seated.first() {
                Some((pilot, _)) => Some(CharacterRef::Character { id: *pilot }),
                None => lobby::lead_character_of(ctx.w, side.team)
                    .map(|id| CharacterRef::Character { id }),
            };
            let terms = seal::SealTerms {
                match_id,
                principal,
                card: crate::arena::card::line(&arena_cfg.reflex),
                odds_floor_permille: floor,
            };
            let custody = arena_def::custody_unit(
                &catalog,
                cells,
                unit,
                side.team,
                pose,
                gunnery_of(ctx, unit),
                controller,
                fielded.slot,
                &terms,
            )
            .map_err(|error| match error {
                seal::SealError::Unlawful { floor } => StartError::CardUnlawful {
                    unit,
                    callsign: unsealed_callsign(ctx.w, unit, side.team, fielded.slot),
                    floor,
                },
                // The Line card over a validated doctrine always pairs; a
                // card that does not is the check-out's own refusal, named.
                card @ seal::SealError::Card(_) => StartError::Custody {
                    unit,
                    detail: card.to_string(),
                },
            })?;
            custody_units.push(custody);
            for (pilot, station) in seated {
                roster.pilots.push((pilot, unit));
                pilots.push(Pilot {
                    unit,
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
    // 16b cut N.2: the same grouping `Arena::new` derives, from the same
    // triples and the same `[tacsim.arena].squad_size` (16b cut Z.1) — a
    // side's machine units (reflex or, since 17b cut L.2, mind:
    // `Controller::is_machine`) dealt into squads, lowest id commanding. One
    // function of one data — the triples come from the one builder
    // `Arena::new` calls too (`CustodyUnit::squad_input`) — so the record and
    // the arena cannot disagree.
    let squads = crate::deliberation::squads_of(
        &crate::arena::state::CustodyUnit::squad_input(&custody_units),
        arena_cfg.squad_size,
    );
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
        // 16b cut V.2: filled from the arena's own answer once it is open.
        callsigns: BTreeMap::new(),
        // 16b cut V.3: built beside the callsigns, which they name units by.
        folds: BTreeMap::new(),
        unparsed: 0,
        // 16b cut I.1: built beside the folds, once the callsigns are known.
        replay: None,
        // 16b cut O.1: the rule it is fought under, for the result and the
        // replay.
        mode,
        // 17b cut L.1 (D-UM-53): one sim persona per squad, minted beside them.
        leaders: crate::deliberation::fill_leaders(&squads),
        squads,
        // 17b cut M3.3 (doc 17 §13, D-UM-46): one seal record per unit, off
        // the custody's own seal — written here, before the record is
        // inserted, so a refused check-out leaves no half record.
        minds: custody_units
            .iter()
            .map(|unit| {
                (
                    unit.id,
                    MindSealRecord {
                        sortie: unit.seal.sortie,
                        seal: unit.seal.clone(),
                    },
                )
            })
            .collect(),
    };
    {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(StartError::Unavailable)?;
        // **16b cut M.1** (`T-12`): both arms insert the fresh `boarding`. A
        // record already in `Boarding` used to be kept and this one dropped, so
        // its roster, definition digest, `started_ms` and seed were not the
        // custody's — the `CustodyOut` below always takes the fresh
        // `seed_for(..)`, which never repeats. Replacing it makes every field of
        // the record agree with the custody by construction; the `match` only
        // gates the state.
        match book.matches.get(&match_id) {
            Some(record) if !record.state.may_start() => {
                return Err(StartError::WrongState {
                    match_id: match_id.0,
                    state: record.state.as_str(),
                });
            }
            Some(_) => {
                book.matches.insert(match_id, boarding);
            }
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

    // **16b cut V.3**: each piloted tank's damage profile and the sections it is
    // checked out with already destroyed, read off the custody before it goes
    // over — what the pilot's debrief needs to say a lost section in the words
    // the cockpit used (`report::OwnTank`).
    let own_tanks: BTreeMap<ObjId, report::OwnTank> = custody_units
        .iter()
        .filter(|unit| pilots.iter().any(|seated| seated.unit == unit.id))
        .map(|unit| (unit.id, report::OwnTank::from_custody(unit)))
        .collect();

    // **16b cut I.1 (16c N9)**: the replay's keyframe 0 — every unit's
    // check-out pose on the ground's cell grid — read off the custody before it
    // goes over, for the same reason `own_tanks` is: `CustodyOut` takes both the
    // units and the definition below.
    let opening = ReplayOpening {
        poses: custody_units
            .iter()
            .map(|unit| (unit.id, unit.pose.clone()))
            .collect(),
        cell_cm: definition.terrain.cell_cm(),
        width: definition.terrain.width(),
        height: definition.terrain.height(),
    };

    // (4) Open. (5) And undo (3) if it refuses.
    let resolution = arena_def::resolution_for(&catalog);
    let custody = CustodyOut {
        definition,
        units: custody_units,
        pilots,
        resolution,
        seed,
        mode,
    };
    let callsigns = match crate::arena_bridge::open_match(
        ctx,
        match_id,
        custody,
        arena_cfg,
        match_cfg,
        sensors_cfg,
    ) {
        Ok(callsigns) => callsigns,
        Err(error) => {
            roll_back(ctx, match_id, &checked_out);
            return Err(StartError::Open(error));
        }
    };

    // **`Boarding` -> `Running`** - the trigger itself, and the last thing that
    // happens, so a refusal above never leaves a match claiming to run.
    //
    // **16b cut V.2**: and the callsigns beside it — the arena's own read of the
    // slot rule it applied (`Arena::callsigns`), kept for the result record.
    if let Some(record) = ctx
        .w
        .module_state_mut::<MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        record.state = MatchState::Running;
        record.callsigns = callsigns.into_iter().collect();
        // **16b cut V.3 (16c N2, Q37)**: one debrief fold per human pilot's
        // unit, named by the callsigns just stored and told under this match's
        // own `designate_on_detection` — held on the fold, so a later config
        // reload cannot change how this match is told. A unit with no pilot
        // (a machine) gets none; a unit with two seated pilots gets one. Each
        // carries its own tank as checked out, so a lost section reads what
        // the cockpit said of it.
        let roster = report::Roster::from_record(record);
        let rules = report::ReportRules {
            designate_on_detection: sensors_cfg.designate_on_detection,
        };
        record.folds = roster
            .pilots()
            .map(|(_, unit)| {
                let fold = report::Fold::new(roster.clone(), report::Reader::Pilot { unit }, rules)
                    .with_own_tank(own_tanks.get(&unit).cloned().unwrap_or_default());
                (unit, fold)
            })
            .collect();
        // **16b cut I.1 (16c N9, Q37)**: one replay fold for the match — both
        // sides, every unit the arena named, in unit-id order — seeded with the
        // opening captured above and fed the tape by `on_chunk` beside the
        // debrief folds. A staff `@arena/start` match gets one too: it is this
        // same function.
        let roster: Vec<(ObjId, crate::arena::state::Designator, Team)> = record
            .callsigns
            .iter()
            .filter_map(|(unit, designator)| {
                record.team_of(*unit).map(|team| (*unit, *designator, team))
            })
            .collect();
        record.replay = Some(replay::ReplayFold::new(
            match_id,
            &record.arena,
            &record.arena_definition_digest,
            opening.cell_cm,
            opening.width,
            opening.height,
            arena_cfg.ledger.summary_ms,
            &roster,
            &opening.poses,
        ));
    }
    // **16b cut AF.2 (16c N16)**: the live record, in this same authoritative
    // action as the markers, once the record carries its folds and its replay
    // fold. The opening pulse snapshots at once and rewrites it; this one write
    // is kept anyway, because it is cheap and a crash before the first snapshot
    // then finds a live record with no snapshot beside it — a void 16b cut AF.3
    // names `no snapshot document` rather than `no live record`. A lobby
    // centre is still `Boarding` at this line (the lobby moves it to `Running`
    // when this returns, in the same action); the snapshot's rewrite carries it
    // `Running`.
    record::write_live(ctx, match_id);
    Ok(match_id)
}

/// **The replay's opening** (16b cut I.1): every unit's check-out pose and the
/// ground's cell grid, captured in [`start`] before the custody crosses the
/// seam.
struct ReplayOpening {
    /// `(unit, pose)` in check-out order.
    poses: Vec<(ObjId, mush_sim::data::Pose)>,
    /// The side of one cell, centimetres.
    cell_cm: u16,
    /// The window's columns.
    width: u16,
    /// The window's rows.
    height: u16,
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
                // 16b cut D.3 (`T-49`): `@arena/release <unit>` is the verb that
                // clears a marker by hand. Before it, no verb did — `@arena`
                // registered status, start, intent, abort and reset and none of
                // them touched one — and the only advice was to restart.
                "tacsim: a rolled-back check-out could not be undone; the unit keeps its marker, \
                 and `@arena/release <unit>` or a restart of the server half is what clears one"
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

/// **How a unit is named before the arena resolves a callsign** (17b cut
/// M3.3; [`StartError::CardUnlawful`]): a lobby-fielded unit by its pod's
/// designator — the callsign 16b cut Q.1's first pass will give it, `RA` —
/// and a staff `@arena/start` unit (slot 0, no pod) by its world name, which
/// is what that staff console already calls it.
fn unsealed_callsign(w: &mush_core::World, unit: ObjId, team: Team, slot: u8) -> String {
    if slot == 0 {
        return mush_core::output::display_name(w, unit);
    }
    crate::arena::state::Designator::Slot {
        team,
        slot: u16::from(slot),
    }
    .to_string()
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
/// cleared (the same write as the cells), the seats and focus are released, the
/// match is untracked and the result document is written.
///
/// **The tank is not reset here** (16b cut F.1, a recorded deviation from doc 16
/// §3.3's *at check-in and at every `Idle` transition*). What a check-in commits
/// is the match's **evidence**: doc 16 §1's acceptance reads the loser's wreck
/// with `@unit/show` afterwards and the result document points at it, and the
/// terminal centre shows the outcome until somebody clears it. The reset happens
/// on the way **into** `Idle` and at boot — [`crate::match_::reset_stock_tanks`],
/// through [`lobby::reset`], which is the one funnel for that transition.
///
/// Idempotent: a second call on the same match finds no live record and answers
/// `false`, having written nothing.
///
/// On `Void` — a panic, a stall or a ledger overflow — [`custody_in`] returns
/// nothing at all, so every unit is restored from its check-out snapshot and
/// **no arena-derived cell is committed**. The pose is restored too, for the
/// same reason: the arena's last frame is a fact the faulted pulse could have
/// produced.
///
/// **16b cut V.3 (16c N2, Q37): the debriefs.** Each pilot's
/// [`report::Fold`] is taken off the record, finished, and written as one
/// `tacsim:report:<match>:<unit>` document ([`write_report`]) in this same
/// action and before the result, whose `"reports"` names them. A void still
/// writes them: what a report holds is what the tape said, and the tape says
/// `void`.
pub fn finish(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    end: MatchEnd,
    ledger_digest: String,
    returned: &[UnitReturn],
) -> bool {
    // **16b cut V.3**: the debrief folds come off the BOOK's record here, before
    // the clone, so each is finished exactly once and the copy below does not
    // carry them.
    let Some((record, folds, replay_fold)) = ctx
        .w
        .module_state_mut::<MatchBook>()
        .and_then(|book| book.get_mut(match_id))
        .filter(|record| record.state.is_live())
        .map(|record| {
            let folds = std::mem::take(&mut record.folds);
            // **16b cut I.1**: and the replay fold, beside them and for the
            // same reason.
            let replay_fold = record.replay.take();
            (record.clone(), folds, replay_fold)
        })
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
            results.push(UnitResult::lost(unit, record.callsign_word(unit)));
            continue;
        }
        // **16b cut W.2** fills `CustodyIn::cells`: the plan is the diff between the
        // unit the world still holds and the unit the arena hands back, and `None`
        // is either a void or a unit the match never marked.
        let plan = returned.cells.get(&unit).and_then(Option::as_ref);
        // **16b cut D.3 (`B-35`): a refused check-in does not strand the seat.**
        //
        // The old arm logged and `continue`d — before `release_seat` — so the
        // pilot kept the cockpit billet, the control lease and the account focus
        // on a tank that was no longer in a match, and the pod was unboardable
        // for the rest of the boot. Now the seat goes first, and then the return
        // is retried **as a void**: the plan the arena sent is what the strict
        // encoder refused, and the check-out snapshot is a shape the world wrote
        // itself, so the retry gives back a unit that is at least real. What is
        // lost is the match's cells for that one unit, which the `warn` says and
        // the result record records as `custody_lost`.
        //
        // One unit's failure never skips another's release: both arms below stay
        // inside this loop.
        let (checked_in, custody_lost) = match mush_tactics::custody::check_in(ctx, unit, plan) {
            Ok(checked_in) => (checked_in, false),
            Err(error) => {
                release_seat(ctx, unit);
                match mush_tactics::custody::check_in(ctx, unit, None) {
                    Ok(restored) => {
                        tracing::warn!(
                            %unit,
                            match_id = match_id.0,
                            %error,
                            "tacsim: a unit's returned cells were refused; its seat is released \
                             and it is restored from its check-out snapshot, so the arena's \
                             cells for it are dropped"
                        );
                        (restored, true)
                    }
                    Err(second) => {
                        tracing::error!(
                            %unit,
                            match_id = match_id.0,
                            %error,
                            %second,
                            // 16b cut D.3 (`T-49`): `@arena/release` is the verb
                            // that clears one now; the boot reconciliation is no
                            // longer the only thing that does.
                            "tacsim: a unit could not be checked in even as a void; its marker \
                             stays on the record, and `@arena/release <unit>` or a restart of \
                             the server half is what clears one"
                        );
                        results.push(UnitResult::lost(unit, record.callsign_word(unit)));
                        continue;
                    }
                }
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
        // Idempotent: on the `B-35` retry arm above the seat is already gone, and
        // this finds no station to hand back.
        release_seat(ctx, unit);
        // **16b cut D.3 (`B-35`)**: a unit the world could only take back as a
        // void is `custody_lost` on the record, whatever the arena said about it
        // — the counters and the outcome beside them describe cells that were
        // never committed.
        if custody_lost {
            results.push(UnitResult::lost(unit, record.callsign_word(unit)));
            continue;
        }
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
        // 16b cut V.2: named by the callsign the arena resolved at check-out.
        results.push(UnitResult {
            unit: unit.0,
            callsign: record.callsign_word(unit),
            outcome,
            shots,
            hits,
        });
    }

    // The registry's rows go last: `untrack` is what frees the epoch mirror and
    // the unit bindings the routing reads.
    //
    // **16b cut Q.3**: anybody still bound when it happens is owed a cleared
    // cockpit, so the list is read before the rows go. In the ordinary case
    // `release_seat` above has already unbound every pilot through
    // `send_control`, and each of them was cleared there; this is the rest — a
    // pilot whose station was already gone, and every pilot on a void.
    let orphans: Vec<ObjId> = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.pilots_of(match_id))
        .unwrap_or_default();
    // **16b cut I.4**: and every staffer watching it, read and unbound BEFORE
    // `untrack` — which has no `Ctx` and cannot send — and each sent the
    // cleared picture after it, exactly as the orphans are sent a cleared
    // cockpit. No `Watch { on: false }` is offered: the task is ending.
    let watchers: Vec<ObjId> = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .map(|registry| {
            let watchers = registry.watchers_of(match_id);
            for watcher in &watchers {
                registry.unbind_watcher(*watcher);
            }
            watchers
        })
        .unwrap_or_default();
    // **16b cut I.1**: the ground the replay was fought on, taken out of the
    // registry BEFORE `untrack` drops its `Arc` (16b cut K.2) and held on the
    // book under the arena's digest — so every lobby match on one proving
    // ground shares one copy, and a replay is drawn without rebuilding the
    // ground. Only a match that has a replay to draw keeps one.
    if replay_fold.is_some()
        && let Some(ground) = crate::arena_bridge::ground_of(ctx.w, match_id)
        && let Some(book) = ctx.w.module_state_mut::<MatchBook>()
    {
        book.hold_ground(record.arena_definition_digest.clone(), ground);
    }
    // **16b cut V.2**: the mirror `untrack` hands back is kept for its chunk
    // count, which the result document carries as `ledger.chunks`. By now the
    // world has appended the whole tape: the task sends every queued chunk
    // before its `Ended` (`drain_backlog_waiting`), and the egress is one
    // ordered queue.
    let chunks = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .and_then(|registry| registry.untrack(match_id))
        .map_or(0, |mirror| mirror.chunks_appended);
    for pilot in orphans {
        crate::arena_bridge::clear_cockpit(ctx, pilot);
    }
    for watcher in watchers {
        crate::arena_bridge::clear_observer(ctx, watcher);
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
    // **16b cut V.3 (16c N2, Q37)**: one report document per pilot fold, in this
    // same action and before the result, which names them. By now every chunk
    // has passed `on_chunk` — the task sends its whole backlog before `Ended` —
    // so each fold has seen the whole tape the world holds, its end line
    // included. A void end still writes them: the fold folded the `void`.
    let mut reports: Vec<ObjId> = Vec::with_capacity(folds.len());
    for (unit, fold) in folds {
        let pilot = record
            .teams
            .iter()
            .flat_map(|roster| roster.pilots.iter())
            .find(|(_, seated)| *seated == unit)
            .map(|(pilot, _)| *pilot);
        let report = fold.finish();
        record::write_report(
            ctx,
            match_id,
            unit,
            pilot,
            &record.callsign_word(unit),
            &report,
        );
        reports.push(unit);
    }
    // **16b cut I.1 (16c N9, Q37)**: the replay — one `tacsim:replay:<match>`
    // document in this same action, before the result, which says it exists —
    // and then the ring's copy in RAM. By now every chunk has passed `on_chunk`,
    // so the fold has seen the whole tape the world holds. A void end still
    // writes the replay it folded: the tape says `void`, and so does the replay.
    let replay_written = match replay_fold {
        Some(fold) => {
            // 16b cut O.1: the mode off the record, so the replay says the
            // rule the match was fought under (plan item 39).
            let mut replay = fold.finish(outcome, record::reason_word(end.reason));
            replay.mode = record.mode;
            record::write_replay(ctx, match_id, &replay);
            if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
                book.hold_replay(match_id, Arc::new(replay));
            }
            true
        }
        None => false,
    };
    record::write_result(
        ctx,
        match_id,
        outcome,
        end,
        &ledger_digest,
        &results,
        chunks,
        &reports,
        replay_written,
    );
    // **16b cut AF.2 (16c N16)**: the running match's two documents go with its
    // result, in this same action, on every end — a void included: a finished
    // match is never resumed, and nothing but the boot ever read them.
    record::delete_live_and_snapshot(ctx, match_id);
    // 16b cut K.1: and the lobby's own half of the release — the pods and the
    // slots — in this same authoritative action, with the centre left in its
    // terminal state carrying the outcome until somebody resets it.
    // 16b cut Q.7: and the arena's own reason beside the outcome, because two
    // different ends map to `draw` and the board is the surface that wants to
    // tell them apart.
    lobby::on_match_finished(ctx, match_id, outcome, end.reason);
    true
}

/// Whether the unit's current placement already **is** the pose a check-in
/// would write: the same domain, and the same three integers of doc 16 §3.2's
/// pose that the *world* decides.
///
/// Two fields are deliberately not compared. The opaque **posture** is not — the
/// arena never touches it, so a difference there is the world's own and must not
/// be overwritten. And, from **16b cut X.6**, neither is **`z_cm`**: inside a
/// match a unit's `z_cm` is a pure function of the cell it stands in (doc 16
/// §3.2, *`z_cm` equals the cell's `elev_cm`*), and `x_cm` and `y_cm` are what
/// decide the cell — so a difference in z alone is not a unit that moved, it is
/// the arena having told the truth about ground the world's flat placement did
/// not know about. Comparing it would make every tank standing on anything but
/// the datum re-place itself at check-in, which is exactly the second spatial
/// write doc 16 §3.1's *the custody markers are the only unit writes during a
/// match* exists to keep out of the record.
fn same_pose(
    current: Option<&(mush_sim::DomainId, mush_sim::data::Pose)>,
    domain: mush_sim::DomainId,
    pose: &mush_sim::data::Pose,
) -> bool {
    current.is_some_and(|(at, held)| {
        *at == domain
            && held.x_cm == pose.x_cm
            && held.y_cm == pose.y_cm
            && held.heading_mdeg == pose.heading_mdeg
    })
}

/// Release the cockpit seats of a unit the boot restored (16b T5; since 16b cut
/// AF.3 a unit of a match that could not resume, [`void_at_boot`]).
///
/// The same seam `release_seat` uses; a separate name because the boot caller
/// is a different story and deserves to read as one.
pub fn release_seats_after_restart(ctx: &mut Ctx<'_>, unit: ObjId) {
    release_seat(ctx, unit);
}

/// Release the seats, leases and focus of everyone stationed on `unit`.
///
/// Doc 16 §3.8: *seats, leases, focus and pods released atomically*. What is
/// released here is the station assignment, the control lease behind it and the
/// account focus pointed at it; the pods and the bodies in them are the lobby's
/// half (`cockpit::eject_all`, 16b cut F.1). The marker has already been cleared
/// by the check-in above, so the `deployed` admission site does not refuse it.
///
/// **16b cut D.3 (`T-49`)** made it `pub(crate)`: `@arena/release <unit>` is the
/// same repair by hand, and a staff verb that cleared a marker without giving the
/// seat back would leave exactly the half-released pod cut F.1 was paid to
/// remove.
pub(crate) fn release_seat(ctx: &mut Ctx<'_>, unit: ObjId) {
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
        // **16b cut F.1**: through [`cockpit::hand_seat_back`], which is the one
        // way this crate lets a cockpit seat go — the arena told first (so the
        // pilot leaves its frame roster and the registry's pilot binding), then
        // the stand-down, then the **billet**. Before this cut there were two
        // ways: `EXIT`'s, which told the arena, and this one, which did not, and
        // a pilot released by a check-in kept their binding until `untrack` a few
        // lines later.
        cockpit::hand_seat_back(ctx, unit, &station, actor, false);
    }
}

/// **What one stop through [`abort`] did** (16b cut M.1, `B-49`).
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortOutcome {
    /// The arena took the stop: the record is parked in `Stopping`, and the
    /// check-in follows when the arena reports its end.
    Stopping,
    /// There was no task to take it (or no layer, or no such match), so the
    /// world finished the match on the spot, with no units: every tank is
    /// restored from its check-out snapshot.
    Finished,
    /// **The arena could not take it this instant and the match is still the
    /// arena's**: a full inbox on a live task, or an `Ended` already on its way
    /// through the shared egress ([`crate::arena_bridge::ControlRefusal::Busy`]).
    /// Nothing was changed — no state write, no finish, no announcement — and
    /// the caller tells whoever asked to try again in a moment.
    Busy,
}

/// Stop one match, for whatever reason (`@arena/abort`, `SIM ABORT`, `SIM
/// CONCEDE`).
///
/// The stop is honoured at the arena's next pulse, and the check-in happens when
/// the arena reports its end — never here. A match whose task is already gone is
/// finished on the spot instead, so an abort always has an effect.
///
/// **16b cut M.1** (`B-49`): a match whose arena is only **busy** is left
/// exactly as it is ([`AbortOutcome::Busy`]). Before this cut every refused offer
/// fell to the no-task arm, so a `SIM ABORT` during a busy pulse — or in the
/// window after the task had sent its `Ended` and before the world had drained
/// it — finished the match with no units and restored every tank from its
/// snapshot, discarding the match's damage; the real `Ended` then found no live
/// record and was thrown away.
pub fn abort(ctx: &mut Ctx<'_>, match_id: MatchId, reason: StopReason) -> AbortOutcome {
    match crate::arena_bridge::send_stop(ctx, match_id, reason) {
        Ok(()) => {
            if let Some(book) = ctx.w.module_state_mut::<MatchBook>()
                && let Some(record) = book.get_mut(match_id)
            {
                record.state = MatchState::Stopping;
            }
            // **16b cut AF.2 (16c N16)**: the live record is rewritten the
            // moment the record parks, so a stop in flight at a crash is found
            // by the boot as `stopping` and voided — never undone by a resume
            // that would bring back a match somebody had ended (16b cut AF.3).
            record::write_live(ctx, match_id);
            return AbortOutcome::Stopping;
        }
        Err(crate::arena_bridge::ControlRefusal::Busy) => return AbortOutcome::Busy,
        Err(_) => {}
    }
    // No task: the world is the only thing left holding the match, so end it
    // here rather than leaving units deployed to nothing.
    // No units come back with it: there is no arena to return them, so every one
    // of them is restored from its check-out snapshot (doc 16 §3.1's fail-closed
    // read, the same one a void takes).
    let _ = finish(
        ctx,
        match_id,
        MatchEnd {
            reason: EndReason::Abort,
            t_ms: 0,
        },
        String::new(),
        &[],
    );
    AbortOutcome::Finished
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

    /// **16b cut X.6**: a z-only difference is not a move. The arena sets a
    /// unit's `z_cm` from the cell it stands on at check-out, so a tank the world
    /// placed flat on ground 300 cm up comes back with a z the placement never
    /// had — and re-placing it for that alone would be a spatial write for a unit
    /// that never drove. The other three are compared, and each of them alone is
    /// a move.
    #[test]
    fn a_z_only_difference_is_not_a_move_and_the_other_three_are() {
        let at = |x_cm: i64, y_cm: i64, z_cm: i32, heading_mdeg: u32| {
            let mut pose =
                mush_sim::data::Pose::at(x_cm, y_cm, z_cm, crate::arena::test_support::rules());
            pose.heading_mdeg = heading_mdeg;
            pose
        };
        let domain = mush_sim::DomainId(1);
        let other = mush_sim::DomainId(2);
        let held = at(6_500, 6_500, 0, 90_000);
        let current = (domain, held.clone());
        assert!(same_pose(Some(&current), domain, &held));
        // z alone: not a move.
        assert!(same_pose(
            Some(&current),
            domain,
            &at(6_500, 6_500, 300, 90_000)
        ));
        // Each of the other three, alone: a move.
        assert!(!same_pose(
            Some(&current),
            domain,
            &at(6_501, 6_500, 0, 90_000)
        ));
        assert!(!same_pose(
            Some(&current),
            domain,
            &at(6_500, 6_501, 0, 90_000)
        ));
        assert!(!same_pose(
            Some(&current),
            domain,
            &at(6_500, 6_500, 0, 90_001)
        ));
        // And the domain.
        assert!(!same_pose(Some(&current), other, &held));
        assert!(!same_pose(None, domain, &held));
    }
}
