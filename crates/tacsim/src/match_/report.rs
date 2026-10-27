//! **The debrief** (16b cut V.3; 16c N2 and Q37; doc 16 §4 "Ledger events",
//! "The result record", §5 the observer rule).
//!
//! The black box records everything a debrief needs, and until this cut no verb
//! read it. This module is the reading, in three parts:
//!
//! 1. **[`Fold`]**, a pure projection of the tape onto one reader. For a pilot
//!    ([`Reader::Pilot`]) it is the pilot's own shots and what became of each,
//!    the hits on their own tank, what they destroyed and the public end — and
//!    nothing of the other side's ammunition, movement, detections or positions:
//!    **the fog does not lift after the match**. For staff ([`Reader::Staff`]) it
//!    is one line an event, every field present.
//! 2. **The fold runs as the tape passes.** One pilot fold per human pilot lives
//!    on the match's record ([`crate::match_::MatchRecord::folds`]), built at
//!    check-out with the match's own rules, fed every chunk's events by
//!    `crate::arena_bridge`'s `on_chunk` as the world appends the chunk, and
//!    finished by [`crate::match_::finish`] into **one
//!    `tacsim:report:<match>:<unit>` document per pilot**, written in the same
//!    action as the result. A whole tape at the owner's scale is tens of
//!    megabytes and the store's read lane is not ordered against its write lane,
//!    so the report is never folded from a read of the store afterwards.
//! 3. **Two readers of the store**: `SIM REPORT [<match>]` ([`request`]) reads
//!    exactly one report document, and `@arena/report <match> [<chunk>]`
//!    ([`request_page`]) pages the tape itself for staff, one page of
//!    [`mush_core::MAX_STORE_READ_KEYS`] chunks a request. Both complete through
//!    [`ReportRead`], the tacsim module's first
//!    [`mush_core::StoreReadCompletion`].
//!
//! # The report says only what the cockpit said
//!
//! A pilot's shot line carries **no range** — `Fired.range_cm` is the target's
//! true pose at acceptance, deliberately not what the cockpit showed (the `Shot`
//! doc in `crate::arena::weapons`) — and **no damage figure**: the in-match hit
//! notice carries a section and nothing of what the round did (`step.rs`'s
//! impact arm: *the shooter sees the round land, and nothing about what it
//! did*). The name and the section word on a shot line are resolved **when the
//! shot's outcome is folded**, exactly where `step.rs` resolves them for the
//! in-match notice (`named_target` and `target_classified` at the hit), so a
//! target classified when the shot was fired and lost before it landed reads
//! `cannon at a contact: hit.` — which is what the pilot was told. The draws and
//! the seed (16b cut M.3) are staff-only; no pilot line ever prints a draw, a
//! seed, a contact id or an object number.
//!
//! # The pilot's own mind (17b cut M5.6)
//!
//! A pilot whose slot chose a mind, or whose tank the mind flew, reads a
//! `Your mind` block ([`MindBlock`]) after the report's header and before its
//! timeline: the mind and its profiles as sealed and the card changes on their
//! hull by the card's own name, whether and from when it flew the tank (never a
//! wreck: a span ends at the hull's destruction, and one the world opened after
//! it is none), its shots, and a drop to the ROM by its cause. The check-out
//! hands the fold what the tape cannot say ([`OwnMind`],
//! [`Fold::with_own_mind`]), and the world opens and closes each
//! span of its flying ([`Fold::released`], [`Fold::reclaimed`]) at the arena's
//! own clock as it last heard it — the one fact of a report not told from the
//! tape or the check-out (a declared departure). The block is beside the
//! pilot's own counts, never in them: the rounds, the kills, the `You fired …`
//! line and the timeline keep the hull's every shot. Another hull's mind is
//! never in it, and the staff page is unchanged.
//!
//! # Held to the arena library's standard
//!
//! The fold is under `match_/` and not `arena/`, so
//! `crates/mush-contract-tests/tests/arena_library.rs` does not scan it, but it
//! keeps the same rules: no I/O, no world, no clock, no float, and every map it
//! iterates to produce a line is a `BTreeMap` (17b cut M5.6's spans are handed
//! their moments by the world; the fold reads no clock of its own). The
//! request and completion halves below the fold are the world-side code, and
//! are the only part of the file that touches a [`Ctx`].

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CmdError, Ctx, ObjId, StoreReadCompletion, StoreReadResult};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::core::{CoreClass, fitted_with_core, read_core};
use crate::arena::damage::DESTROYED;
use crate::arena::ledger::{EventKind, LedgerEvent};
use crate::arena::resolution::{Consequence, UnitProfile};
use crate::arena::state::{ContactId, CustodyUnit, EndReason, Team};
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::arena_task::MatchId;
use crate::deliberation::order::CharacterRef;
use crate::match_::MatchBook;
use crate::match_::cockpit::{consequence_words, miss_words};
use crate::match_::lobby;
use crate::match_::record::{
    HISTORY_DEPTH, HistoryRow, MatchOutcome, MatchRecord, UnitOutcome, arena_label, civil_utc,
    duration_words, end_words, outcome_words, reason_word, report_key,
};

/// The most timeline lines a report keeps before `… and N more.`.
///
/// It bounds the fold's memory through a whole match whatever the pilot does:
/// once this many lines are held, a further completed line is counted in
/// [`Report::truncated`] and dropped. The tail lines are never counted
/// against it ([`Report::tail`]).
pub const REPORT_MAX_LINES: usize = 200;

/// How many lines one `Sys` event carries, joined by `'\n'`.
///
/// The per-connection output queues are 256 events (SSH's `OUTPUT_BUFFER`, the
/// web's `send_queue`), and a queue that overflows detaches the session — so a
/// two-hundred-line report goes out as eight events and not two hundred.
/// `mush-ansi` splits a `Sys` on `'\n'`, the web's `RichLine` is `pre-wrap`, and
/// the TUI's transcript takes the same path.
pub const REPORT_LINES_PER_EVENT: usize = 25;

/// The record word a report document carries.
pub const REPORT_RECORD: &str = "tacsim.report";

// ---------------------------------------------------------------------------
// The roster, the reader and the rules
// ---------------------------------------------------------------------------

/// **One unit as a report names it** (16b cut V.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterUnit {
    /// Its side, or `None` for a unit on no roster (a restart abort's).
    pub team: Option<Team>,
    /// Its callsign as the arena resolved it (`RA`), or empty.
    pub callsign: SmolStr,
    /// The first pilot seated on it, or `None` for a machine.
    pub pilot: Option<ObjId>,
    /// That pilot's display name, when the staff request read it off the world
    /// ([`Roster::name_pilots`]). The fold never reads it.
    pub pilot_name: Option<String>,
}

/// **Who flew what under which callsign** (16b cut V.3): unit → (team,
/// callsign, pilot), in unit-id order.
///
/// Built from the match's own record at check-out ([`Roster::from_record`]) for
/// the pilot folds, and from a remembered result ([`Roster::from_row`]) for a
/// staff page. The one source of every callsign is the arena's own
/// (`crate::arena::state::Arena::callsigns`, carried by 16b cut V.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    units: BTreeMap<ObjId, RosterUnit>,
}

impl Roster {
    /// An empty roster — a staff page of a match the centre no longer
    /// remembers, whose units then print as `#n`.
    #[must_use]
    pub fn new() -> Roster {
        Roster::default()
    }

    /// Add or replace one unit.
    pub fn insert(
        &mut self,
        unit: ObjId,
        team: Option<Team>,
        callsign: SmolStr,
        pilot: Option<ObjId>,
    ) {
        self.units.insert(
            unit,
            RosterUnit {
                team,
                callsign,
                pilot,
                pilot_name: None,
            },
        );
    }

    /// **The roster of a match the world is tracking**: every unit on its
    /// rosters, its callsign off [`MatchRecord::callsigns`], and the first pilot
    /// the roster seated on it.
    #[must_use]
    pub fn from_record(record: &MatchRecord) -> Roster {
        let mut roster = Roster::new();
        for side in &record.teams {
            for unit in &side.units {
                let pilot = side
                    .pilots
                    .iter()
                    .find(|(_, seated)| seated == unit)
                    .map(|(pilot, _)| *pilot);
                roster.insert(*unit, Some(side.team), record.callsign_word(*unit), pilot);
            }
        }
        roster
    }

    /// **The roster of a remembered match**, off its result document's rows.
    #[must_use]
    pub fn from_row(row: &HistoryRow) -> Roster {
        let mut roster = Roster::new();
        for unit in &row.units {
            roster.insert(unit.unit, unit.team, unit.callsign.clone(), unit.pilot);
        }
        roster
    }

    /// Put each pilot's display name on the roster (the staff request does, at
    /// request time, off the world).
    pub fn name_pilots(&mut self, name: impl Fn(ObjId) -> String) {
        for unit in self.units.values_mut() {
            unit.pilot_name = unit.pilot.map(&name);
        }
    }

    /// One unit.
    #[must_use]
    pub fn get(&self, unit: ObjId) -> Option<&RosterUnit> {
        self.units.get(&unit)
    }

    /// One unit's callsign, when it has a non-empty one.
    #[must_use]
    pub fn callsign(&self, unit: ObjId) -> Option<&str> {
        self.units
            .get(&unit)
            .map(|held| held.callsign.as_str())
            .filter(|callsign| !callsign.is_empty())
    }

    /// Every unit, in unit-id order.
    pub fn iter(&self) -> impl Iterator<Item = (ObjId, &RosterUnit)> {
        self.units.iter().map(|(unit, held)| (*unit, held))
    }

    /// Every `(pilot, unit)` pair, in unit-id order.
    pub fn pilots(&self) -> impl Iterator<Item = (ObjId, ObjId)> + '_ {
        self.units
            .iter()
            .filter_map(|(unit, held)| held.pilot.map(|pilot| (pilot, *unit)))
    }
}

/// **Whose report it is** (16b cut V.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reader {
    /// One pilot, reading their own match through their own platform's fog.
    Pilot {
        /// The unit they flew.
        unit: ObjId,
    },
    /// Staff, reading the tape itself.
    Staff,
}

/// **The match's own rules the projection needs** (16b cut V.3), held on the
/// fold from check-out rather than re-read later, so a config reload during a
/// match cannot change how that match is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportRules {
    /// `[tacsim.sensors].designate_on_detection`: a contact is named by its
    /// target's callsign the moment it is held, rather than once it is told
    /// apart (`crate::arena::perception`'s `designator_for`).
    pub designate_on_detection: bool,
}

/// **The pilot's own tank, as far as the words for a lost section need it** (16b
/// cut V.3).
///
/// A `section_destroyed` line carries the section's **whole authored** list
/// (`crate::arena::damage::Arena::fired_consequences` says why), and one
/// consequence on it can be written down and do nothing:
/// `disable_motion_when_all_of` on the first of a line tank's two tracks or a
/// biped's two legs. The cockpit's `SectionLost` line drops it until every
/// section it names is destroyed (16b cuts F.2 and X.2), because *a pilot told
/// their tank cannot move while it is still walking would act on it* — and the
/// debrief says only what the cockpit said. So the fold keeps the one question
/// `fired_consequences` asks: each of the tank's sections' motion groups, in
/// authored order, and which of its sections are destroyed — at check-out, then
/// off its own `section_destroyed` lines. Built from the custody at check-out
/// ([`OwnTank::from_custody`]); a fold with none (a fixture's) can prove no
/// group complete, so it never says a tank cannot move on that consequence's
/// word.
///
/// **17b cut M3.6**: and the core's two words, which `fired_consequences`
/// puts through a question of its own (a `destroy_core` on a hull never
/// fitted with a core, a `degrade_core` that left the core no lower than it
/// was sealed at, say nothing): the fold keeps the core as [`OwnCore`],
/// read off the custody at check-out and moved by its own
/// `section_destroyed` lines the way `Arena::core_ops` moves the cell. A
/// fold that does not know its tank's core never says a core word.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnTank {
    /// Section → the section list of each `disable_motion_when_all_of` it
    /// authors, in authored order.
    motion_groups: BTreeMap<SmolStr, Vec<Vec<SmolStr>>>,
    /// Its sections destroyed so far.
    destroyed: BTreeSet<SmolStr>,
    /// Its core (17b cut M3.6): `None` for a tank checked out with no core
    /// fitted, or a fold that does not know.
    #[serde(default)]
    core: Option<OwnCore>,
}

/// **The pilot's own core, as the core words need it** (17b cut M3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnCore {
    /// The class it was sealed at (`MindSeal.core_class`).
    sealed: CoreClass,
    /// The class its cell reads now.
    now: CoreClass,
    /// Whether the capability is still held (`destroy_core` takes it).
    held: bool,
}

impl OwnCore {
    /// **A core sealed at `sealed`, reading `now`, held or not.**
    #[must_use]
    pub const fn new(sealed: CoreClass, now: CoreClass, held: bool) -> OwnCore {
        OwnCore { sealed, now, held }
    }
}

impl OwnTank {
    /// **From a damage profile and the sections already destroyed.**
    #[must_use]
    pub fn new(profile: &UnitProfile, destroyed: impl IntoIterator<Item = SmolStr>) -> OwnTank {
        let motion_groups = profile
            .sections
            .iter()
            .filter_map(|section| {
                let groups: Vec<Vec<SmolStr>> = section
                    .consequences
                    .iter()
                    .filter_map(|consequence| match consequence {
                        Consequence::DisableMotionWhenAllOf { sections } => Some(sections.clone()),
                        _ => None,
                    })
                    .collect();
                (!groups.is_empty()).then(|| (section.id.clone(), groups))
            })
            .collect();
        OwnTank {
            motion_groups,
            destroyed: destroyed.into_iter().collect(),
            core: None,
        }
    }

    /// **The same tank with its core** (17b cut M3.6).
    #[must_use]
    pub fn with_core(mut self, core: OwnCore) -> OwnTank {
        self.core = Some(core);
        self
    }

    /// **The tank as it is handed over at check-out**: its profile, and every
    /// section its M32 truth already reads `destroyed` — the condition
    /// `fired_consequences`' own `all_destroyed` reads.
    ///
    /// **17b cut M3.6**: and its core — fitted ([`fitted_with_core`]), sealed
    /// at the seal's `core_class`, reading [`read_core`]'s class now and held
    /// while the capability is.
    #[must_use]
    pub fn from_custody(unit: &CustodyUnit) -> OwnTank {
        let own = OwnTank::new(
            &unit.profile,
            unit.unit
                .sections()
                .filter(|(_, state)| state.condition.as_str() == DESTROYED)
                .map(|(id, _)| SmolStr::new(id.as_str())),
        );
        if !fitted_with_core(&unit.unit) {
            return own;
        }
        let live = read_core(&unit.unit);
        own.with_core(OwnCore::new(
            unit.seal.core_class,
            live.as_ref().map_or(CoreClass::Zero, |core| core.class),
            live.is_some(),
        ))
    }

    /// **`section` is destroyed: the consequences that fired**, from the ledger
    /// line's authored list — every one but a `disable_motion_when_all_of`
    /// whose sections are not all destroyed yet (or whose group this tank does
    /// not know).
    ///
    /// **17b cut M3.6**: the core words move the core first, every one in
    /// authored order as `Arena::core_ops` builds its ops off the core held
    /// BEFORE the plan (each `degrade_core` one class while held and above
    /// Class 0, a `destroy_core` taking it), and are then told off the core
    /// as it now stands, as `fired_consequences` tells them: `destroy_core`
    /// for a fitted core no longer held, `degrade_core` for a held core below
    /// its sealed class.
    fn fired(&mut self, section: &SmolStr, consequences: &[SmolStr]) -> Vec<SmolStr> {
        self.destroyed.insert(section.clone());
        if let Some(core) = &mut self.core {
            let held = core.held;
            for label in consequences {
                match label.as_str() {
                    DEGRADE_CORE if held && core.now > CoreClass::Zero => {
                        core.now = core.now.lower();
                    }
                    DESTROY_CORE => core.held = false,
                    _ => {}
                }
            }
        }
        let core = self.core;
        let groups = self.motion_groups.get(section);
        let mut nth = 0_usize;
        consequences
            .iter()
            .filter(|label| {
                match label.as_str() {
                    DESTROY_CORE => return core.is_some_and(|core| !core.held),
                    DEGRADE_CORE => {
                        return core.is_some_and(|core| core.held && core.now < core.sealed);
                    }
                    MOTION_GROUP => {}
                    _ => return true,
                }
                let group = groups.and_then(|groups| groups.get(nth));
                nth = nth.saturating_add(1);
                group.is_some_and(|group| {
                    !group.is_empty() && group.iter().all(|held| self.destroyed.contains(held))
                })
            })
            .cloned()
            .collect()
    }
}

/// The ledger word of `Consequence::DisableMotionWhenAllOf`.
const MOTION_GROUP: &str = "disable_motion_when_all_of";

/// The ledger word of `Consequence::DestroyCore` (17b cut M3.6).
const DESTROY_CORE: &str = "destroy_core";

/// The ledger word of `Consequence::DegradeCore` (17b cut M3.6).
const DEGRADE_CORE: &str = "degrade_core";

// ---------------------------------------------------------------------------
// The pilot's own mind (17b cut M5.6)
// ---------------------------------------------------------------------------

/// **The pilot's own mind, as the check-out sealed it** (17b cut M5.6; the
/// owner's OQ-9 (ii); the design note's M5.6 item 1, F12) — what the `Your
/// mind` block names that the tape cannot say: whether the slot chose a mind
/// at all (a chosen `SIM MIND Standard` with no `PROFILE` seals byte for byte
/// as an unchosen slot does, so no tape line tells the two apart), the mind
/// and its profile names as sealed (empty is Line alone), and whether
/// Q-UM-15 substituted Standard Doctrine for the slot's choice.
///
/// Seeded by `crate::match_::start` beside [`Fold::with_own_tank`], through
/// [`Fold::with_own_mind`], from `crate::match_::Side::minds` and the seal it
/// took — [`OwnTank`]'s precedent of a check-out fact the fold is handed,
/// not a departure from 16b cut V.3's "told from the tape".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnMind {
    /// Whether the slot chose a mind (`SealChoice::chose`).
    pub chose: bool,
    /// The mind as the seal names it (`MindSeal::mind`).
    pub mind: SmolStr,
    /// The profile names as the seal names them, the first the one that
    /// flies at the release; empty is Line alone (`MindSeal::profiles`).
    pub profiles: Vec<SmolStr>,
    /// Whether Q-UM-15 substituted Standard Doctrine for the slot's choice
    /// (`SealChoice::substituted`; fixture-only throughout 17b's M5, the
    /// owner's OQ-5: no production seal is other than Standard Doctrine).
    pub substituted: bool,
}

/// **How the mind came to fly the pilot's tank** (17b cut M5.6): what
/// `crate::match_::lobby::slot_left` reads off the slot as it marks it out of
/// its cockpit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandOver {
    /// An `EXIT`: the pilot climbed out, for good.
    ClimbedOut,
    /// A drop — the linkdead grace running out, a `QUIT`, or the boot's
    /// resume of the match, each of which stamps `Slot::dropped_at_ms`
    /// before its `Released` marks the slot: the pilot's line was down, and
    /// a `BOARD` may take the tank back.
    Dropped,
}

/// **One stretch of the match the mind flew the pilot's tank** (17b cut
/// M5.6), in arena milliseconds on the world's copy of the arena's own clock
/// (`MatchMirror::now_ms`, the orchestrator's ruling at 17b cut M5.4's close):
/// opened at the release, closed when the pilot takes the tank back, `None`
/// while it runs to the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Span {
    from_ms: i64,
    to_ms: Option<i64>,
    by: HandOver,
}

impl Span {
    /// Whether a line stamped `t_ms` falls inside it — from its opening, up
    /// to and not including its close.
    fn holds(&self, t_ms: i64) -> bool {
        self.from_ms <= t_ms && self.to_ms.is_none_or(|to_ms| t_ms < to_ms)
    }

    /// `from 0:42 to 1:10`, or `from 0:42 to the end`.
    fn stretch(&self) -> String {
        match self.to_ms {
            Some(to_ms) => format!("from {} to {}", stamp(self.from_ms), stamp(to_ms)),
            None => format!("from {} to the end", stamp(self.from_ms)),
        }
    }
}

/// One weapon's shots inside a span, and how many of them hit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tally {
    shots: u32,
    hits: u32,
}

/// A tape line's moment and its one word — a card's name, a drop's cause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Said {
    t_ms: i64,
    word: SmolStr,
}

/// **What the fold keeps of the pilot's own mind** (17b cut M5.6): the
/// check-out's [`OwnMind`], the spans the world opened and closed, and off
/// the tape — on the pilot's OWN hull alone — the hull's shots inside a span
/// with their hits and kills, its card changes (`mind_card`: a seat's `MIND
/// PROFILE`, or a staff `@mind/live`), and its drop to the ROM
/// (`mind_dropped`). Bounded whatever the match does: one tally per weapon,
/// the last card change and a count, one drop (the arena journals one a
/// sortie), and one span per release — each needing `reclaim_min_ms` after
/// the drop that opened the one before.
///
/// **The mind flies no wreck** (17b cut M5.6's review, R1/R23): the tape's
/// `unit_destroyed` of the hull is kept ([`MindTrack::wrecked_ms`]), and the
/// block reads the spans cut there ([`MindTrack::flown`]) — a destroyed pilot
/// told to `EXIT` (or whose line drops) after it opens a span the world
/// cannot tell from any other, and it is no span. Cut when the block is
/// built, never at [`Fold::released`]: the tape reaches the fold after the
/// world has marked the slot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MindTrack {
    own: Option<OwnMind>,
    spans: Vec<Span>,
    /// The hull's shots fired inside a span whose outcome is not on the tape
    /// yet, by `shot_seq`.
    in_flight: BTreeSet<u64>,
    shots: BTreeMap<SmolStr, Tally>,
    kills: u32,
    /// The target of the hull's last completed shot, when it was one of the
    /// mind's and hit — what a `unit_destroyed` crediting the hull is the
    /// mind's kill by (the line follows the `hit` that did it in the same
    /// substep, as [`Fold::append_clause`]'s rule reads it).
    last_hit: Option<ObjId>,
    switches: u32,
    last_switch: Option<Said>,
    dropped: Option<Said>,
    /// When the hull was destroyed (its own `unit_destroyed`), if it was.
    wrecked_ms: Option<i64>,
}

impl MindTrack {
    /// Nothing seeded and nothing seen: what a fold built before this cut,
    /// or one never seeded, holds — skipped on the live record, so its bytes
    /// are the bytes it wrote before.
    fn is_unset(&self) -> bool {
        *self == MindTrack::default()
    }

    /// Whether the mind was flying the hull at `t_ms`.
    fn flying_at(&self, t_ms: i64) -> bool {
        self.spans.iter().any(|span| span.holds(t_ms))
    }

    /// The hull fired: one of the mind's shots when it flew the hull then.
    fn fired(&mut self, t_ms: i64, shot_seq: u64, weapon: &SmolStr) {
        if self.flying_at(t_ms) {
            self.in_flight.insert(shot_seq);
            let tally = self.shots.entry(weapon.clone()).or_default();
            tally.shots = tally.shots.saturating_add(1);
        }
    }

    /// One of the hull's shots landed, `hit` or not.
    fn landed(&mut self, shot_seq: u64, weapon: &SmolStr, target: ObjId, hit: bool) {
        let ours = self.in_flight.remove(&shot_seq);
        if ours
            && hit
            && let Some(tally) = self.shots.get_mut(weapon)
        {
            tally.hits = tally.hits.saturating_add(1);
        }
        self.last_hit = (ours && hit).then_some(target);
    }

    /// **The spans as the block reads them** (17b cut M5.6's review): cut at
    /// the hull's destruction — a span still open then, or closed after it,
    /// ends there — and one the world opened at or after it dropped, since
    /// the mind flies no wreck. Every span as the world told it, when the
    /// hull lived to the end.
    fn flown(&self) -> Vec<Span> {
        self.spans
            .iter()
            .filter_map(|span| match self.wrecked_ms {
                None => Some(*span),
                Some(wrecked_ms) if span.from_ms >= wrecked_ms => None,
                Some(wrecked_ms) => Some(Span {
                    to_ms: Some(span.to_ms.map_or(wrecked_ms, |to_ms| to_ms.min(wrecked_ms))),
                    ..*span
                }),
            })
            .collect()
    }

    /// **The `Your mind` block**, or `None` when it is not printed: the
    /// owner's OQ-9 (ii) prints it only for a slot that chose a mind or a
    /// hull the mind flew ([`MindTrack::flown`]: a span opened on a wreck is
    /// none), and a fold with no check-out facts has nothing to name.
    fn block(&self) -> Option<MindBlock> {
        let own = self.own.as_ref()?;
        let spans = self.flown();
        if !own.chose && spans.is_empty() {
            return None;
        }
        let mut lines = vec![self.head(own)];
        if self.switches > 1
            && let Some(last) = &self.last_switch
        {
            lines.push(format!(
                "  Its last switch was to {} at {}.",
                last.word,
                stamp(last.t_ms)
            ));
        }
        if own.substituted {
            lines.push(SUBSTITUTED_LINE.to_owned());
        }
        lines.push(span_line(&spans));
        if !spans.is_empty() {
            lines.push(self.shots_line());
        }
        if let Some(dropped) = &self.dropped {
            lines.push(drop_line(dropped));
        }
        Some(MindBlock { lines })
    }

    /// `Your mind: Standard Doctrine (Sniper, Line); switched to Line at
    /// 1:05.` — the mind and its profiles as sealed, and its card changes by
    /// the card's own name: one by its name and moment, more as a count here
    /// and the last on a line of its own under it (`  Its last switch was to
    /// Line at 1:30.`), so no count and no edited card's name takes the head
    /// past 120 columns.
    fn head(&self, own: &OwnMind) -> String {
        let mut text = format!("Your mind: {}", lobby::mind_words(&own.mind, &own.profiles));
        match &self.last_switch {
            Some(_) if self.switches > 1 => {
                text.push_str(&format!("; switched {} times", self.switches));
            }
            Some(last) => {
                text.push_str(&format!(
                    "; switched to {} at {}",
                    last.word,
                    stamp(last.t_ms)
                ));
            }
            None => {}
        }
        text.push('.');
        text
    }

    /// `  Its shots: cannon 4, 2 of them hits; it destroyed nothing.` — the
    /// hull's shots inside a span, by weapon id as authored and in
    /// `BTreeMap` order, the hits among them and the units they destroyed;
    /// `  It fired nothing.` when it fired none.
    fn shots_line(&self) -> String {
        let fired: Vec<(&SmolStr, Tally)> = self
            .shots
            .iter()
            .filter(|(_, tally)| tally.shots > 0)
            .map(|(weapon, tally)| (weapon, *tally))
            .collect();
        let shots = fired
            .iter()
            .fold(0_u32, |sum, (_, tally)| sum.saturating_add(tally.shots));
        let hits = fired
            .iter()
            .fold(0_u32, |sum, (_, tally)| sum.saturating_add(tally.hits));
        let each: Vec<String> = fired
            .iter()
            .map(|(weapon, tally)| format!("{weapon} {}", tally.shots))
            .collect();
        let weapons = match each.split_last() {
            None => return "  It fired nothing.".to_owned(),
            Some((only, [])) => only.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        };
        let outcome = match (shots, hits) {
            (1, 0) => "no hit".to_owned(),
            (1, _) => "a hit".to_owned(),
            (_, 0) => "none of them hits".to_owned(),
            (_, 1) => "1 of them a hit".to_owned(),
            (_, hits) => format!("{hits} of them hits"),
        };
        let destroyed = match self.kills {
            0 => "nothing".to_owned(),
            1 => "1 unit".to_owned(),
            kills => format!("{kills} units"),
        };
        format!("  Its shots: {weapons}, {outcome}; it destroyed {destroyed}.")
    }
}

/// The `Your mind` block's line for a slot whose choice Q-UM-15 substituted
/// (17b cut M5.6): the seal line's own clause (`crate::match_::lobby::seal_line`).
const SUBSTITUTED_LINE: &str = "  Sealed in place of your choice, for humans on the other side.";

/// **When it flew** (17b cut M5.6; the design note's M5.6 item 1), bounded
/// whatever the count: `  It never flew: you flew the tank the whole match.`;
/// one span — `  It flew your tank from 0:42, when you climbed out, to the
/// end.` (`… to 3:12.` when the hull was destroyed then, the one way an
/// `EXIT`'s span closes), `  It flew your tank from 0:42 to 1:10, while your
/// line was down.` (`… to the end, …` when never taken back); two — `  It flew your
/// tank from 0:42 to 1:10 and from 2:05 to the end, while your line was
/// down.` (the cause clause only when both were drops); three or more — `
/// It flew your tank 5 times: first from 0:42 to 1:10, last from 12:05 to the
/// end.`, with no cause clause, since the spans' causes may differ.
fn span_line(spans: &[Span]) -> String {
    const LEAD: &str = "  It flew your tank";
    const LINE_DOWN: &str = ", while your line was down";
    match spans {
        [] => "  It never flew: you flew the tank the whole match.".to_owned(),
        [only] => match (only.by, only.to_ms) {
            (HandOver::ClimbedOut, None) => format!(
                "{LEAD} from {}, when you climbed out, to the end.",
                stamp(only.from_ms)
            ),
            (HandOver::ClimbedOut, Some(to_ms)) => format!(
                "{LEAD} from {}, when you climbed out, to {}.",
                stamp(only.from_ms),
                stamp(to_ms)
            ),
            (HandOver::Dropped, _) => format!("{LEAD} {}{LINE_DOWN}.", only.stretch()),
        },
        [first, second] => {
            let cause = if first.by == HandOver::Dropped && second.by == HandOver::Dropped {
                LINE_DOWN
            } else {
                ""
            };
            format!(
                "{LEAD} {} and {}{cause}.",
                first.stretch(),
                second.stretch()
            )
        }
        [first, .., last] => format!(
            "{LEAD} {} times: first {}, last {}.",
            spans.len(),
            first.stretch(),
            last.stretch()
        ),
    }
}

/// `  It dropped to the ROM at 1:30: its core was destroyed.` — the drop's
/// cause word alone (D17: `MindDropped.fault`'s ruling is still open, so the
/// last fault's word and the op budget are staff's), put in words; a cause
/// word this build does not know reads the moment alone.
fn drop_line(dropped: &Said) -> String {
    use crate::arena::mind::DropCause;
    let why = match DropCause::parse(&dropped.word) {
        Some(DropCause::Strikes) => ": three faults running",
        Some(DropCause::Core) => ": its core was destroyed",
        Some(DropCause::Abi) => ": its core could not run its mind on this engine",
        None => "",
    };
    format!("  It dropped to the ROM at {}{why}.", stamp(dropped.t_ms))
}

/// **The `Your mind` block** (17b cut M5.6; the owner's OQ-9 (ii); doc 17
/// §22's deliverable 8): the pilot's own mind and its profiles as sealed,
/// its card changes by the card's own name, whether and from when it flew
/// the tank, its shots, and a drop to the ROM by its cause — each line under
/// 120 columns, said only to the pilot whose report it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MindBlock {
    /// The block, its head first.
    pub lines: Vec<String>,
}

/// **The one page of the tape a staff header describes** (16b cut V.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Page {
    /// The first chunk asked for.
    pub first: u64,
    /// The last chunk the page returned (the last asked for, when none was).
    pub last: u64,
    /// How many tape lines parsed.
    pub lines: usize,
    /// How many tape lines did not parse.
    pub unreadable: usize,
}

// ---------------------------------------------------------------------------
// The fold
// ---------------------------------------------------------------------------

/// **One contact the pilot's platform holds** (16b cut V.3): the target it is
/// held on, whether the cockpit named it, and whether it told it apart.
///
/// The two bits are `perception.rs::designator_for`'s two conditions, kept
/// apart because under `designate_on_detection` a blip is named at once and
/// still not told apart — and a section id is a chassis vocabulary (16b cut
/// Q.2), earned only by telling the target apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Known {
    target: ObjId,
    designated: bool,
    classified: bool,
}

/// A shot the pilot fired whose outcome is not on the tape yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeldShot {
    /// The `Fired` event's own stamp and seq — the shot line's.
    t_ms: i64,
    seq: u64,
    weapon: SmolStr,
    target: ObjId,
    /// `BA's turret destroyed` clauses that arrived while it was held.
    clauses: Vec<String>,
}

/// One timeline line and the stamp it sorts by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    t_ms: i64,
    seq: u64,
    text: String,
}

/// **The accumulator** (16b cut V.3): one reader's projection of a tape, fed
/// one event at a time in seq order.
///
/// `Clone + Debug + PartialEq` because it lives on the
/// [`crate::match_::MatchRecord`], which is all three.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fold {
    roster: Roster,
    reader: Reader,
    rules: ReportRules,
    /// The pilot's own tank: which of its lost sections' consequences fired.
    own: OwnTank,
    /// The pilot's knowledge model: every contact their platform holds now.
    known: BTreeMap<ContactId, Known>,
    /// The pilot's shots in flight, by `shot_seq`.
    held: BTreeMap<u64, HeldShot>,
    lines: Vec<Line>,
    truncated: usize,
    tail: Vec<String>,
    /// The seq the next event should carry; `None` until the first, for staff
    /// (a page begins wherever it begins).
    expected: Option<u64>,
    gap: bool,
    rounds: BTreeMap<SmolStr, u32>,
    kills: u32,
    last_ms: i64,
    /// The most recently completed shot line: its target and, when it was
    /// kept, its index in `lines`.
    last_completed: Option<(ObjId, Option<usize>)>,
    /// **The pilot's last `ordered` line said** (16b cut Y.1), as `(t_ms,
    /// squad)`: the tape carries one line per unit the order landed on, and the
    /// pilot gave one order, so a second line of the same moment and squad says
    /// nothing.
    last_order: Option<(i64, SmolStr)>,
    /// **The pilot's last `order_released` line said** (16b cut Z.2), as
    /// `(t_ms, squad)`, on [`Fold::last_order`]'s rule: one line per unit the
    /// release dropped, one release given.
    last_release: Option<(i64, SmolStr)>,
    /// **The pilot's own mind** (17b cut M5.6): what the `Your mind` block
    /// is folded from ([`MindTrack`]).
    ///
    /// `#[serde(default)]` under the `LIVE_FORMAT` policy
    /// (`crate::match_::record`'s head): an added defaulted field, so no
    /// bump — a live record written before this cut holds no mind facts,
    /// which is true of it — and skipped while unset, so a fold that has
    /// none writes the bytes it wrote before.
    #[serde(default, skip_serializing_if = "MindTrack::is_unset")]
    mind: MindTrack,
}

/// **A folded report** (16b cut V.3): what one reader is told of one match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The timeline, sorted by `(t_ms, seq)` of each line's own event, at most
    /// [`REPORT_MAX_LINES`].
    pub lines: Vec<String>,
    /// Timeline lines not kept.
    pub truncated: usize,
    /// The pilot's own end (disabled, destroyed) and the match's end, in event
    /// order and never truncated.
    pub tail: Vec<String>,
    /// Part of the tape was missing: a seq gap, or a chunk that was not UTF-8.
    pub gap: bool,
    /// The pilot's own rounds fired, per weapon id as authored.
    pub rounds: BTreeMap<SmolStr, u32>,
    /// Units the pilot's rounds destroyed.
    pub kills: u32,
    /// The last event's `t_ms` folded. Not on the report document.
    #[serde(default)]
    pub last_ms: i64,
    /// **The `Your mind` block** (17b cut M5.6; the owner's OQ-9 (ii)), or
    /// `None` when it is not printed — a slot that chose no mind and a hull
    /// the mind never flew. Beside the pilot's own counts, never in them:
    /// [`Report::rounds`], [`Report::kills`], the `You fired …` line and the
    /// timeline count the hull's every shot exactly as before this cut.
    /// Skipped on the report document when `None`, so every report without
    /// one is the document it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mind: Option<MindBlock>,
}

impl Fold {
    /// An empty fold for `reader`, naming units off `roster`, under `rules`.
    #[must_use]
    pub fn new(roster: Roster, reader: Reader, rules: ReportRules) -> Fold {
        Fold {
            roster,
            reader,
            rules,
            own: OwnTank::default(),
            known: BTreeMap::new(),
            held: BTreeMap::new(),
            lines: Vec::new(),
            truncated: 0,
            tail: Vec::new(),
            // A pilot's fold reads a whole tape, which starts at 1; a staff page
            // starts wherever the page does.
            expected: match reader {
                Reader::Pilot { .. } => Some(1),
                Reader::Staff => None,
            },
            gap: false,
            rounds: BTreeMap::new(),
            kills: 0,
            last_ms: 0,
            last_completed: None,
            last_order: None,
            last_release: None,
            mind: MindTrack::default(),
        }
    }

    /// **The pilot's own tank** ([`OwnTank`]), which the words for its lost
    /// sections are put through. `match_::start` hands every pilot fold its
    /// tank off the custody; a fold without one never says a
    /// `disable_motion_when_all_of` fired.
    #[must_use]
    pub fn with_own_tank(mut self, own: OwnTank) -> Fold {
        self.own = own;
        self
    }

    /// **The pilot's own mind as the check-out sealed it** ([`OwnMind`];
    /// 17b cut M5.6), which the `Your mind` block names. `match_::start`
    /// hands every pilot fold its mind beside its tank; a fold without one
    /// prints no block.
    #[must_use]
    pub fn with_own_mind(mut self, own: OwnMind) -> Fold {
        self.mind.own = Some(own);
        self
    }

    /// **The pilot this fold is for**: the roster's pilot of its unit, or
    /// `None` for a staff fold or a unit the roster seats nobody in.
    #[must_use]
    pub fn pilot(&self) -> Option<ObjId> {
        match self.reader {
            Reader::Pilot { unit } => self.roster.get(unit).and_then(|held| held.pilot),
            Reader::Staff => None,
        }
    }

    /// **The mind took the pilot's tank at `at_ms`** (17b cut M5.6): a span
    /// of the `Your mind` block opens, `by` the route the release took.
    /// Called by `crate::match_::lobby::slot_left` — the funnel every
    /// `Released` that names a pilot passes — with the arena's own clock as
    /// the world last heard it (`MatchMirror::now_ms`), the one fact of the
    /// block neither the tape nor the check-out carries (a declared
    /// departure, the design note's §7 3): a shot in the pulse of the
    /// hand-over may be counted on either side of it. A span already open is
    /// left as it is. One opened on a wreck is kept here and read as none
    /// when the block is built (`MindTrack::flown`): the hull's
    /// `unit_destroyed` may reach the fold after this call.
    pub fn released(&mut self, at_ms: i64, by: HandOver) {
        if self
            .mind
            .spans
            .last()
            .is_some_and(|span| span.to_ms.is_none())
        {
            return;
        }
        self.mind.spans.push(Span {
            from_ms: at_ms,
            to_ms: None,
            by,
        });
    }

    /// **The pilot took the tank back at `at_ms`** (17b cut M5.6): the open
    /// span closes there (never before it opened). Called by
    /// `crate::match_::lobby::slot_reclaimed` on the same clock as
    /// [`Fold::released`]; nothing when no span is open.
    pub fn reclaimed(&mut self, at_ms: i64) {
        if let Some(span) = self.mind.spans.last_mut()
            && span.to_ms.is_none()
        {
            span.to_ms = Some(at_ms.max(span.from_ms));
        }
    }

    /// Whose fold this is.
    #[must_use]
    pub const fn reader(&self) -> Reader {
        self.reader
    }

    /// Timeline lines not kept so far — the staff page reads it to find the
    /// chunk a page was cut short in.
    #[must_use]
    pub const fn truncated(&self) -> usize {
        self.truncated
    }

    /// **One event, in seq order.** A seq that is not the last seq plus one sets
    /// the gap (a pilot's first event's is 1).
    pub fn absorb(&mut self, event: &LedgerEvent) {
        if let Some(expected) = self.expected
            && event.seq != expected
        {
            self.gap = true;
        }
        self.expected = Some(event.seq.saturating_add(1));
        self.last_ms = event.t_ms;
        match self.reader {
            Reader::Pilot { unit } => self.absorb_pilot(unit, event),
            Reader::Staff => self.absorb_staff(event),
        }
    }

    /// **A chunk that could not be read at all** (not UTF-8): part of the tape
    /// is missing.
    pub fn lost_chunk(&mut self) {
        self.gap = true;
    }

    /// **One line said at this point of the tape, as it is** (16b cut AF.3): the
    /// staff pager's resume-marker line ([`fold_page`]). Placed after every
    /// event folded so far and before the next — keyed at the last event's time
    /// and the next event's seq, and pushed before that event's own line, which
    /// a stable sort keeps behind it — never stamped, never counted against
    /// [`REPORT_MAX_LINES`] and never truncated: it is the tape's structure, not
    /// one of its events. It moves nothing a fold reads (`expected`, the gap).
    pub(crate) fn mark(&mut self, text: String) {
        self.lines.push(Line {
            t_ms: self.last_ms,
            seq: self.expected.unwrap_or(0),
            text,
        });
    }

    /// **The report.** Every shot still held reads `no outcome on the tape`
    /// (named as at the end), and the timeline is sorted stably by each line's
    /// own `(t_ms, seq)` — a shot line by its `Fired`, because a shot completes
    /// after later events were written.
    #[must_use]
    pub fn finish(mut self) -> Report {
        let held = std::mem::take(&mut self.held);
        for shot in held.into_values() {
            let name = self.name_of(shot.target);
            let text = with_clauses(
                format!("{} at {name}: no outcome on the tape.", shot.weapon),
                &shot.clauses,
            );
            let _ = self.push_line(shot.t_ms, shot.seq, text);
        }
        let mut lines = std::mem::take(&mut self.lines);
        lines.sort_by_key(|line| (line.t_ms, line.seq));
        Report {
            lines: lines.into_iter().map(|line| line.text).collect(),
            truncated: self.truncated,
            tail: self.tail,
            gap: self.gap,
            rounds: self.rounds,
            kills: self.kills,
            last_ms: self.last_ms,
            // 17b cut M5.6: a span still open runs to the end.
            mind: self.mind.block(),
        }
    }

    // -- the pilot's projection ----------------------------------------------

    /// **The pilot's knowledge model and lines** — see the module doc and
    /// [`Known`]. Nothing about how another pilot's platform saw anything is
    /// ever consulted: every knowledge write below is gated on `viewer == me`.
    fn absorb_pilot(&mut self, me: ObjId, event: &LedgerEvent) {
        let (t_ms, seq) = (event.t_ms, event.seq);
        match &event.kind {
            // -- knowledge: never a line --------------------------------------
            EventKind::Detected {
                viewer,
                contact,
                target,
            } if *viewer == me => {
                self.known.insert(
                    *contact,
                    Known {
                        target: *target,
                        designated: self.rules.designate_on_detection,
                        classified: false,
                    },
                );
            }
            EventKind::Classified {
                viewer,
                contact,
                target,
                ..
            } if *viewer == me => {
                // A mesh arrival journals `detected` then `classified` in one
                // moment; either way the row ends up held, named and told apart.
                self.known.insert(
                    *contact,
                    Known {
                        target: *target,
                        designated: true,
                        classified: true,
                    },
                );
            }
            EventKind::Lost {
                viewer, contact, ..
            } if *viewer == me => {
                self.known.remove(contact);
            }
            // -- the pilot's own platform -------------------------------------
            EventKind::PowerOnline { unit } if *unit == me => {
                let _ = self.push_line(t_ms, seq, "power online.".to_owned());
            }
            // **16b cut O.2** (16c N12): the pilot's own pod's switch. Another
            // unit's switch is not this pilot's to read — the sights it reached
            // learned of it only through their own receivers.
            EventKind::Emcon { unit, on } if *unit == me => {
                let text = if *on {
                    "you switched EMCON on."
                } else {
                    "you switched EMCON off."
                };
                let _ = self.push_line(t_ms, seq, text.to_owned());
            }
            EventKind::Locked { unit, target, .. } if *unit == me => {
                let text = format!("locked {}.", self.name_of(*target));
                let _ = self.push_line(t_ms, seq, text);
            }
            EventKind::Fired {
                unit,
                weapon,
                shot_seq,
                target,
                rounds,
                ..
            } if *unit == me => {
                let total = self.rounds.entry(weapon.clone()).or_insert(0);
                *total = total.saturating_add(*rounds);
                // 17b cut M5.6: and the `Your mind` block's count, when the
                // mind flew the hull then — beside the pilot's own counts,
                // which keep every shot of the hull as they always did.
                self.mind.fired(t_ms, *shot_seq, weapon);
                self.held.insert(
                    *shot_seq,
                    HeldShot {
                        t_ms,
                        seq,
                        weapon: weapon.clone(),
                        target: *target,
                        clauses: Vec::new(),
                    },
                );
            }
            EventKind::Hit {
                shot_seq,
                target,
                section,
                facet,
                armor_section,
                armor_left,
                internals_left,
                ..
            } => {
                // The shooter's half: resolved NOW, where `step.rs` resolves the
                // in-match `Hit` notice — the name through `named_target`, the
                // section under `target_classified`.
                if let Some(shot) = self.held.remove(shot_seq) {
                    self.mind.landed(*shot_seq, &shot.weapon, shot.target, true);
                    let name = self.name_of(shot.target);
                    let outcome = if self.classified(shot.target) {
                        format!("hit, {section}.")
                    } else {
                        "hit.".to_owned()
                    };
                    self.complete(shot, &name, &outcome);
                }
                // The victim's half: their own physical knowledge, in the
                // vocabulary of `FrameNotice::Struck` — and never who fired it.
                if *target == me {
                    let place = if armor_section == section {
                        format!("the {section}'s {facet} facet")
                    } else {
                        format!("the {section}, through the {armor_section}'s {facet} facet")
                    };
                    let text = if *armor_left > 0 {
                        format!("hit on {place}: {armor_left} armour left.")
                    } else {
                        format!("hit on {place}: armour gone, {internals_left} inside.")
                    };
                    let _ = self.push_line(t_ms, seq, text);
                }
            }
            EventKind::Miss { shot_seq, reason } => {
                if let Some(shot) = self.held.remove(shot_seq) {
                    self.mind
                        .landed(*shot_seq, &shot.weapon, shot.target, false);
                    let name = self.name_of(shot.target);
                    // **16b cut AE.2**: the reason under the cockpit's own gate
                    // (`step.rs` stages `FrameNotice::Miss` with no reason for a
                    // target the shooter no longer holds). `moved`, `blocked`
                    // and `cover` are facts about where the target was; a pilot
                    // whose platform held no contact on it at the impact reads
                    // `miss.` and no more, here as in the cockpit.
                    let outcome = if self.held_on(shot.target) {
                        format!("miss: {}.", miss_words(reason))
                    } else {
                        "miss.".to_owned()
                    };
                    self.complete(shot, &name, &outcome);
                }
            }
            EventKind::SectionDestroyed {
                unit,
                section,
                by,
                consequences,
            } => {
                if *unit == me {
                    // The consequences that FIRED, as the cockpit's
                    // `SectionLost` line says them (`step.rs` puts the list
                    // through `fired_consequences`), not the authored list the
                    // ledger line carries: a line tank that loses one track is
                    // slowed and still driving.
                    let fired = self.own.fired(section, consequences);
                    let cost = consequence_words(&fired);
                    let text = if cost.is_empty() {
                        format!("your {section} destroyed.")
                    } else {
                        format!("your {section} destroyed: {cost}.")
                    };
                    let _ = self.push_line(t_ms, seq, text);
                } else if *by == Some(me) && self.classified(*unit) {
                    let clause = format!("{}'s {section} destroyed", self.name_of(*unit));
                    self.append_clause(*unit, clause);
                }
            }
            EventKind::UnitDestroyed { unit, by } => {
                if *unit == me {
                    self.tail
                        .push(stamped(t_ms, "your tank destroyed.".to_owned()));
                    // 17b cut M5.6's review: and the mind flies no wreck —
                    // the `Your mind` block's spans end here.
                    self.mind.wrecked_ms.get_or_insert(t_ms);
                    // The wreck's platform forgets every contact it held, with
                    // no `lost` line (`Arena::destroy_unit` clears the viewer
                    // silently). Its rounds still in flight land on their own
                    // merits, and the cockpit names and sections them through
                    // a hold that is gone — `Hit.`, `Target destroyed.` — so
                    // the fold forgets them here too.
                    self.known.clear();
                } else if *by == Some(me) {
                    self.kills = self.kills.saturating_add(1);
                    // 17b cut M5.6: the mind's, when the hit that did it was.
                    if self.mind.last_hit == Some(*unit) {
                        self.mind.kills = self.mind.kills.saturating_add(1);
                    }
                    let text = format!("{} destroyed by your round.", self.name_of(*unit));
                    let _ = self.push_line(t_ms, seq, text);
                }
            }
            // The tape does not say by whom: it credits nobody.
            EventKind::UnitDisabled { unit } if *unit == me => {
                self.tail
                    .push(stamped(t_ms, "your tank disabled.".to_owned()));
            }
            EventKind::MatchEnded { reason } => {
                let words = end_words(MatchOutcome::from(*reason), reason_word(*reason));
                self.tail
                    .push(stamped(t_ms, format!("the match ended: {words}.")));
            }
            EventKind::Aborted => {
                self.tail
                    .push(stamped(t_ms, "the match was aborted.".to_owned()));
            }
            EventKind::Void => {
                self.tail
                    .push(stamped(t_ms, "the match was voided.".to_owned()));
            }
            // **16b cut Y.1**: the pilot's own orders, once per order. Only the
            // issuer reads them — an order the machines' own commander gave, or
            // another pilot's, is not this pilot's to be told about.
            EventKind::Ordered {
                squad,
                issuer: Some(issuer),
                objective,
                target,
                spot,
                hold_fire,
                ..
            } if *issuer == me => {
                let moment = (t_ms, squad.clone());
                if self.last_order.as_ref() == Some(&moment) {
                    return;
                }
                self.last_order = Some(moment);
                let what = match objective.as_str() {
                    "advance" => match spot {
                        Some(spot) => format!("advance on {spot}"),
                        // The cell pair is never printed: it is a coordinate
                        // and not a word the pilot used.
                        None => "advance".to_owned(),
                    },
                    // The hint is the pilot's own contact id, so it is named
                    // through the pilot's own knowledge, exactly as a lock is.
                    "engage" => match target {
                        Some(contact) => format!("engage {}", self.contact_name(*contact)),
                        None => "engage".to_owned(),
                    },
                    other => other.to_owned(),
                };
                let holding = if *hold_fire { ", holding fire" } else { "" };
                let _ = self.push_line(t_ms, seq, format!("ordered {squad}{holding}: {what}."));
            }
            // **16b cut Z.2** (16c Q44): the pilot's own release, once per
            // release, on the `ordered` line's rule — only the tank that gave it
            // reads it. **17b cut M5.6** (the M5.3 close's D9 ruling): `to
            // doctrine.` until that cut — a released squad flies each hull's
            // sealed mind, one word for every mind, as the acknowledgement says.
            EventKind::OrderReleased {
                squad,
                issuer: Some(issuer),
                ..
            } if *issuer == me => {
                let moment = (t_ms, squad.clone());
                if self.last_release.as_ref() == Some(&moment) {
                    return;
                }
                self.last_release = Some(moment);
                let _ = self.push_line(t_ms, seq, format!("released {squad} to their minds."));
            }
            // **17b cut M5.6** (the owner's OQ-9 (ii); D14): the pilot's own
            // hull's card changes — a seat's `MIND PROFILE`, or a staff
            // `@mind/live` read by the card's own name (`Sniper, edited
            // (min_odds 70)`) — and its drop to the ROM, by its cause word
            // alone (D17), for the `Your mind` block: never a timeline line.
            // Another hull's are not this pilot's to read.
            EventKind::MindCard { unit, name, .. } if *unit == me => {
                self.mind.switches = self.mind.switches.saturating_add(1);
                self.mind.last_switch = Some(Said {
                    t_ms,
                    word: name.clone(),
                });
            }
            // The arena journals one a sortie; the first is kept all the same.
            EventKind::MindDropped { unit, cause, .. }
                if *unit == me && self.mind.dropped.is_none() =>
            {
                self.mind.dropped = Some(Said {
                    t_ms,
                    word: cause.clone(),
                });
            }
            // Everything else prints nothing: every other unit's detections,
            // classifications and losses, position summaries, tempo, blocked,
            // receiver, lag, startup, order expiry, impact, and every event of
            // any other unit — and (16b cut I.3) the staff levers' `tempo_floor`,
            // `placed` and `place_refused`, even about the pilot's own tank: a
            // place is journalled always and announced never (16c N10) — and
            // (17b cut M3.3) the seal, a think's fault and the sortie's close,
            // which are staff's.
            _ => {}
        }
    }

    /// A held shot's outcome arrived: one line at the **shot's** stamp and seq.
    fn complete(&mut self, shot: HeldShot, name: &str, outcome: &str) {
        let text = with_clauses(
            format!("{} at {name}: {outcome}", shot.weapon),
            &shot.clauses,
        );
        let kept = self.push_line(shot.t_ms, shot.seq, text);
        self.last_completed = Some((shot.target, kept));
    }

    /// **`; BA's turret destroyed.`** onto the shot line that did it: the shot
    /// completed last when it was at this target (a section destroyed follows the
    /// `hit` that finished it in the same substep), else the most recent shot at
    /// it still in flight, else nothing.
    fn append_clause(&mut self, target: ObjId, clause: String) {
        match self.last_completed {
            Some((at, Some(index))) if at == target => {
                if let Some(line) = self.lines.get_mut(index) {
                    line.text = with_clauses(std::mem::take(&mut line.text), &[clause]);
                }
            }
            // The line that did it was dropped at the cap; so is its clause.
            Some((at, None)) if at == target => {}
            _ => {
                if let Some(shot) = self
                    .held
                    .values_mut()
                    .rev()
                    .find(|shot| shot.target == target)
                {
                    shot.clauses.push(clause);
                }
            }
        }
    }

    /// **Whether this platform holds any contact on `target` at all** (16b cut
    /// AE.2), named or not and told apart or not — the fold's reading of
    /// `step.rs`'s `named_target(...).is_some()`, which is the gate on a miss's
    /// reason.
    fn held_on(&self, target: ObjId) -> bool {
        self.known.values().any(|known| known.target == target)
    }

    /// Whether some contact this platform holds is on `target` and named.
    fn named(&self, target: ObjId) -> bool {
        self.known
            .values()
            .any(|known| known.target == target && known.designated)
    }

    /// Whether some contact this platform holds is on `target` and told apart.
    fn classified(&self, target: ObjId) -> bool {
        self.known
            .values()
            .any(|known| known.target == target && known.classified)
    }

    /// **A contact id of the pilot's own, as they would read it** (16b cut Y.1):
    /// the callsign of its target while the platform holds it designated, and
    /// `a contact` otherwise — a hint the pilot typed at a still-unknown `X2`
    /// is not named after the fact.
    fn contact_name(&self, contact: ContactId) -> String {
        match self.known.get(&contact) {
            Some(known) if known.designated => self
                .roster
                .callsign(known.target)
                .map_or_else(|| "a contact".to_owned(), str::to_owned),
            _ => "a contact".to_owned(),
        }
    }

    /// `BA` while the platform holds `target` named, and `a contact` otherwise.
    fn name_of(&self, target: ObjId) -> String {
        if self.named(target)
            && let Some(callsign) = self.roster.callsign(target)
        {
            return callsign.to_owned();
        }
        "a contact".to_owned()
    }

    // -- the staff whole -----------------------------------------------------

    /// One line an event, except `position_summary`.
    fn absorb_staff(&mut self, event: &LedgerEvent) {
        if matches!(event.kind, EventKind::PositionSummary { .. }) {
            return;
        }
        let mut text = format!(
            "  {}  {}",
            stamp_fine(event.t_ms),
            self.staff_fields(&event.kind)
        );
        if !event.draws.is_empty() {
            let draws: Vec<String> = event
                .draws
                .iter()
                .map(|(label, value)| format!("{label}={value}"))
                .collect();
            text.push_str(&format!(" [{}]", draws.join(" ")));
        }
        if self.lines.len() >= REPORT_MAX_LINES {
            self.truncated = self.truncated.saturating_add(1);
        } else {
            self.lines.push(Line {
                t_ms: event.t_ms,
                seq: event.seq,
                text,
            });
        }
    }

    /// `RA(#101)`, or `#101` for a unit not on the roster.
    fn unit_word(&self, unit: ObjId) -> String {
        match self.roster.callsign(unit) {
            Some(callsign) => format!("{callsign}(#{})", unit.0),
            None => format!("#{}", unit.0),
        }
    }

    /// **A warrant principal as staff read it** (17b cut M3.3): a character by
    /// the unit word of the first roster unit it pilots (`RA(#101)` — the
    /// unit-word rule, never the character's own object number), `character`
    /// for one that pilots none; a sim persona by its name in quotes
    /// (`"red lance leader 1"`); nobody as `-`.
    fn principal_word(&self, principal: Option<&CharacterRef>) -> String {
        match principal {
            None => "-".to_owned(),
            Some(CharacterRef::Character { id }) => self
                .roster
                .units
                .iter()
                .find(|(_, unit)| unit.pilot == Some(*id))
                .map_or_else(|| "character".to_owned(), |(unit, _)| self.unit_word(*unit)),
            Some(CharacterRef::Persona { squad }) => {
                format!("\"{}\"", CharacterRef::persona_name(*squad))
            }
        }
    }

    /// **The staff spelling of one event**: its kind word, then every field it
    /// carries as `key=value` — units as [`Fold::unit_word`], contacts as their
    /// sixteen digits, teams as their word. The hash is never printed: it is the
    /// record's, not a reader's.
    fn staff_fields(&self, kind: &EventKind) -> String {
        let word = kind.as_str();
        let unit = |id: &ObjId| self.unit_word(*id);
        match kind {
            EventKind::MatchStarted | EventKind::Aborted | EventKind::Void => word.to_owned(),
            EventKind::Tempo { from, to } => {
                format!("{word} from={} to={}", from.as_str(), to.as_str())
            }
            EventKind::Startup { unit: id } | EventKind::PowerOnline { unit: id } => {
                format!("{word} unit={}", unit(id))
            }
            EventKind::PositionSummary { units } => format!("{word} units={}", units.len()),
            EventKind::Blocked {
                unit: id,
                x,
                y,
                cause,
            } => format!(
                "{word} unit={} cell={x},{y} cause={}",
                unit(id),
                cause.as_str()
            ),
            EventKind::Detected {
                viewer,
                contact,
                target,
            }
            | EventKind::Lost {
                viewer,
                contact,
                target,
            } => format!(
                "{word} viewer={} contact={contact} target={}",
                unit(viewer),
                unit(target)
            ),
            EventKind::Classified {
                viewer,
                contact,
                target,
                team,
            } => format!(
                "{word} viewer={} contact={contact} target={} team={}",
                unit(viewer),
                unit(target),
                team.as_str()
            ),
            EventKind::Receiver { unit: id, degraded } => {
                format!("{word} unit={} degraded={degraded}", unit(id))
            }
            // 16b cut O.2: the switch's position as a bare word.
            EventKind::Emcon { unit: id, on } => {
                format!(
                    "{word} unit={} {}",
                    unit(id),
                    if *on { "on" } else { "off" }
                )
            }
            EventKind::Locked {
                unit: id,
                contact,
                target,
            } => format!(
                "{word} unit={} contact={contact} target={}",
                unit(id),
                unit(target)
            ),
            EventKind::Fired {
                unit: id,
                weapon,
                shot_seq,
                target,
                range_cm,
                rounds,
                by,
                slot,
            } => {
                // **17b cut M1b.1** (D-UM-14): the attribution, appended only
                // when the line carries it — a seat's or a machine's shot reads
                // byte for byte as it always did.
                format!(
                    "{word} unit={} weapon={weapon} shot={shot_seq} target={} range_cm={range_cm} \
                     rounds={rounds}{}{}",
                    unit(id),
                    unit(target),
                    by.as_ref()
                        .map(|by| format!(" by={by}"))
                        .unwrap_or_default(),
                    slot.map(|slot| format!(" slot={slot}")).unwrap_or_default()
                )
            }
            EventKind::Impact {
                shot_seq,
                unit: id,
                target,
            } => format!(
                "{word} shot={shot_seq} unit={} target={}",
                unit(id),
                unit(target)
            ),
            EventKind::Hit {
                shot_seq,
                unit: id,
                target,
                section,
                facet,
                armor_section,
                rounds,
                damage,
                armor_left,
                internals_left,
                consequences,
                cause,
            } => format!(
                "{word} shot={shot_seq} unit={} target={} section={section} facet={facet} \
                 armor_section={armor_section} rounds={rounds} damage={damage} \
                 armor_left={armor_left} internals_left={internals_left} consequences={} \
                 cause={cause}",
                unit(id),
                unit(target),
                list_word(consequences)
            ),
            EventKind::Miss { shot_seq, reason } => {
                format!("{word} shot={shot_seq} reason={reason}")
            }
            EventKind::SectionDestroyed {
                unit: id,
                section,
                by,
                consequences,
            } => format!(
                "{word} unit={} section={section} by={} consequences={}",
                unit(id),
                by.as_ref().map_or_else(|| "-".to_owned(), unit),
                list_word(consequences)
            ),
            EventKind::UnitDisabled { unit: id } => format!("{word} unit={}", unit(id)),
            EventKind::UnitDestroyed { unit: id, by } => format!(
                "{word} unit={} by={}",
                unit(id),
                by.as_ref().map_or_else(|| "-".to_owned(), unit)
            ),
            EventKind::OrderExpired {
                unit: id,
                squad,
                objective,
            } => format!(
                "{word} unit={} squad={squad} objective={objective}",
                unit(id)
            ),
            EventKind::OrderReleased {
                unit: id,
                squad,
                issuer,
            } => format!(
                "{word} unit={} squad={squad} issuer={}",
                unit(id),
                issuer.as_ref().map_or_else(|| "-".to_owned(), unit)
            ),
            EventKind::Ordered {
                unit: id,
                squad,
                issuer,
                objective,
                target,
                spot,
                hold_fire,
                ttl_ms,
            } => format!(
                "{word} unit={} squad={squad} issuer={} objective={objective} target={} \
                 spot={} hold_fire={hold_fire} ttl_ms={ttl_ms}",
                unit(id),
                issuer.as_ref().map_or_else(|| "-".to_owned(), unit),
                target.map_or_else(|| "-".to_owned(), |contact| contact.to_string()),
                spot.as_ref().map_or("-", SmolStr::as_str)
            ),
            // **16b cut I.3** (16c N10): the two staff levers, as staff read
            // them. The pilot's fold prints none of the three.
            EventKind::TempoFloor {
                period_ms: Some(period_ms),
            } => format!("{word} period_ms={period_ms}"),
            EventKind::TempoFloor { period_ms: None } => format!("{word} lifted"),
            EventKind::Placed {
                unit: id,
                x,
                y,
                from_x,
                from_y,
            } => {
                let from = if (*from_x, *from_y) == (u16::MAX, u16::MAX) {
                    "off the window".to_owned()
                } else {
                    format!("({from_x}, {from_y})")
                };
                format!("{word} unit={} ({x}, {y}) from {from}", unit(id))
            }
            EventKind::PlaceRefused {
                unit: id,
                x,
                y,
                reason,
            } => format!("{word} unit={} ({x}, {y}): {reason}", unit(id)),
            // **17b cut M1b.5** (doc 17 §11): the staff card lever, as staff
            // read it — the card's name and the sliders it moved; the pilot's
            // timeline prints neither. (17b cut M5.6: the card's name on the
            // pilot's own hull reaches that pilot's `Your mind` block, never
            // the sliders — `absorb_pilot`'s `MindCard` arm.)
            EventKind::MindCard {
                unit: id,
                name,
                changed,
                ..
            } => format!(
                "{word} unit={} name=\"{name}\" changed={}",
                unit(id),
                list_word(changed)
            ),
            EventKind::MindCardRefused { unit: id, reason } => {
                format!("{word} unit={}: {reason}", unit(id))
            }
            // **17b cut M3.3** (doc 17 §13, §19): the four mind kinds, as staff
            // read them (defined here; journalled from 17b cut M3.4 and M3.6).
            // A principal is named as the unit-word rule names a unit — the
            // hull its character flies — never by an object number of its own.
            EventKind::MindSealed {
                unit: id,
                sortie,
                mind,
                class,
                governed,
                licence,
                principal,
                ..
            } => format!(
                "{word} unit={} sortie={} mind=\"{mind}\" class={} governed={governed} \
                 licence={licence} principal={}",
                unit(id),
                sortie,
                class.as_str(),
                self.principal_word(principal.as_ref())
            ),
            EventKind::SortieClosed {
                unit: id,
                sortie,
                word: closed,
            } => format!("{word} unit={} sortie={} word={closed}", unit(id), sortie),
            EventKind::MindFault {
                unit: id,
                think_seq,
                cause,
            } => format!(
                "{word} unit={} think_seq={think_seq} cause={cause}",
                unit(id)
            ),
            EventKind::MindDropped {
                unit: id,
                cause,
                fault,
                ops,
            } => {
                let mut text = format!("{word} unit={} cause={cause}", unit(id));
                if let Some(fault) = fault {
                    text.push_str(&format!(" fault={fault}"));
                }
                if let Some(ops) = ops {
                    text.push_str(&format!(" ops={ops}"));
                }
                text
            }
            EventKind::Lagged { dt_ms } => format!("{word} dt_ms={dt_ms}"),
            EventKind::MatchEnded { reason } => {
                let winner = match reason {
                    EndReason::Elimination { winner: Some(team) }
                    | EndReason::Conceded { winner: team }
                    // 16b cut O.1: a hold names its winner the same way.
                    | EndReason::Hold { winner: team } => team.as_str(),
                    _ => "-",
                };
                format!("{word} reason={} winner={winner}", reason_word(*reason))
            }
        }
    }

    // -- the timeline --------------------------------------------------------

    /// Keep one completed timeline line, or count it in `truncated` once
    /// [`REPORT_MAX_LINES`] are held. Answers its index when kept.
    fn push_line(&mut self, t_ms: i64, seq: u64, text: String) -> Option<usize> {
        if self.lines.len() >= REPORT_MAX_LINES {
            self.truncated = self.truncated.saturating_add(1);
            return None;
        }
        self.lines.push(Line {
            t_ms,
            seq,
            text: stamped(t_ms, text),
        });
        Some(self.lines.len().saturating_sub(1))
    }
}

/// `a,b` for a list of authored words, `-` for none.
fn list_word(words: &[SmolStr]) -> String {
    if words.is_empty() {
        "-".to_owned()
    } else {
        words
            .iter()
            .map(SmolStr::as_str)
            .collect::<Vec<&str>>()
            .join(",")
    }
}

/// `text` (ending in `.`) with each clause joined on by `; ` before the stop.
fn with_clauses(mut text: String, clauses: &[String]) -> String {
    if clauses.is_empty() {
        return text;
    }
    if text.ends_with('.') {
        text.pop();
    }
    for clause in clauses {
        text.push_str("; ");
        text.push_str(clause);
    }
    text.push('.');
    text
}

/// `  0:42  <text>` — a pilot line's own indentation and stamp.
fn stamped(t_ms: i64, text: String) -> String {
    format!("  {}  {text}", stamp(t_ms))
}

/// **An arena time as a pilot reads it** (16b cut V.3): `m:ss`, the seconds two
/// digits, the minutes as many as they need (`64:05`). A negative time reads
/// `0:00`.
#[must_use]
pub fn stamp(t_ms: i64) -> String {
    let secs = t_ms.max(0) / 1_000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// **An arena time as staff read it** (16b cut V.3): `m:ss.mmm`.
#[must_use]
pub fn stamp_fine(t_ms: i64) -> String {
    let t_ms = t_ms.max(0);
    format!("{}.{:03}", stamp(t_ms), t_ms % 1_000)
}

/// **Fold a whole event list for one reader** (16b cut V.3): [`Fold::new`], an
/// [`Fold::absorb`] per event in order, and [`Fold::finish`].
#[must_use]
pub fn fold(events: &[LedgerEvent], roster: &Roster, reader: Reader, rules: ReportRules) -> Report {
    let mut fold = Fold::new(roster.clone(), reader, rules);
    for event in events {
        fold.absorb(event);
    }
    fold.finish()
}

// ---------------------------------------------------------------------------
// The header: the public part
// ---------------------------------------------------------------------------

/// A unit's outcome word, as a header prints it.
const fn outcome_word(outcome: UnitOutcome) -> &'static str {
    match outcome {
        UnitOutcome::Intact => "intact",
        UnitOutcome::Disabled => "disabled",
        UnitOutcome::Destroyed => "destroyed",
        UnitOutcome::CustodyLost => "custody lost",
    }
}

/// `1 shot` / `14 shots`.
fn counted(n: u32, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// **The pilot's hit rate** (16b cut O.3; 16c N2), the bracket after `You flew
/// …`'s counters: ` (42%)` for 6 hits in 14 shots — `hits × 100 / shots`,
/// rounded down, in `u64` so no counter can overflow it — and nothing at all
/// when no shot was fired, where a rate would be a division by nought dressed
/// as `0%`. The counters are [`UnitReturn`](crate::arena::damage::UnitReturn)'s,
/// so a hit is a round that took something off something.
fn hit_rate(shots: u32, hits: u32) -> String {
    u64::from(hits)
        .saturating_mul(100)
        .checked_div(u64::from(shots))
        .map(|per_cent| format!(" ({per_cent}%)"))
        .unwrap_or_default()
}

/// **A report's header** (16b cut V.3): the public part, from the remembered
/// result and the arena's label, and then the reader's own.
///
/// ```text
/// Match 12 on The Proving Ground, 2026-09-24 14:02 UTC: blue won (red eliminated), 4 m 32 s.
/// Red lost 1 of 2; blue lost 0 of 2.
/// ```
///
/// **16b cut O.1**: a match fought under a mode other than the skirmish carries
/// its tag after the outcome words (`… red won (red held the key spot) [hold],
/// 2 m 0 s.`), as the ring's line does.
///
/// For [`Reader::Pilot`]: `You flew RA: destroyed; 14 shots, 6 hits (42%).` (the
/// row's own outcome word and counters, and **16b cut O.3**'s hit rate — 16c N2,
/// the range's reason to exist — `hits × 100 / shots` rounded down, printed only
/// when a shot was fired, so `0 shots, 0 hits.` carries no bracket and `3 shots,
/// 0 hits (0%).` does) and, when `facts` are given, `You fired 14
/// rounds of cannon and 60 of machine_gun.` (weapon ids as authored, in
/// `BTreeMap` order; `You fired nothing.`). For [`Reader::Staff`], with `staff`
/// given: `seed …; tape match.12, digest ab12…ef, 42 chunks; this page: chunks 0
/// to 15, 412 lines, 0 unreadable.` and one line per result row, `  red RA #101
/// (Ada #5): destroyed, 14 shots, 6 hits.` — the pilots' display names from the
/// roster, `(machine)` for none. The seed is staff-only and never reaches a
/// pilot's header.
#[must_use]
pub fn header_lines(
    row: &HistoryRow,
    label: &str,
    reader: Reader,
    facts: Option<&Report>,
    staff: Option<(&Page, &Roster)>,
) -> Vec<String> {
    let place = if label.is_empty() {
        String::new()
    } else {
        format!(" on {label}")
    };
    let mut lines = vec![
        format!(
            "Match {}{place}, {}: {}{}, {}.",
            row.id.0,
            civil_utc(row.ended_ms / 1_000),
            outcome_words(row.outcome, &row.reason),
            // 16b cut O.1: ` [hold]` for a match fought under another rule.
            row.mode_tag(),
            duration_words(row.arena_ms),
        ),
        {
            let (red_lost, red_of) = row.losses(Team::Red);
            let (blue_lost, blue_of) = row.losses(Team::Blue);
            format!("Red lost {red_lost} of {red_of}; blue lost {blue_lost} of {blue_of}.")
        },
    ];
    match reader {
        Reader::Pilot { unit } => {
            if let Some(flown) = row.units.iter().find(|held| held.unit == unit) {
                lines.push(format!(
                    "You flew {}: {}; {}, {}{}.",
                    flown.callsign,
                    outcome_word(flown.outcome),
                    counted(flown.shots, "shot", "shots"),
                    counted(flown.hits, "hit", "hits"),
                    hit_rate(flown.shots, flown.hits),
                ));
            }
            if let Some(report) = facts {
                lines.push(fired_line(&report.rounds));
            }
        }
        Reader::Staff => {
            if let Some((page, roster)) = staff {
                lines.push(format!(
                    "seed {}; tape {}, digest {}, {}; {}",
                    row.seed,
                    row.log,
                    short_digest(&row.digest),
                    counted_u64(row.chunks, "chunk", "chunks"),
                    page_words(page),
                ));
                for held in &row.units {
                    let team = held.team.map_or("none", |team| team.as_str());
                    let name = if held.callsign.is_empty() {
                        format!("#{}", held.unit.0)
                    } else {
                        format!("{} #{}", held.callsign, held.unit.0)
                    };
                    let pilot = match held.pilot {
                        Some(pilot) => {
                            let called = roster
                                .get(held.unit)
                                .and_then(|entry| entry.pilot_name.clone())
                                .unwrap_or_default();
                            if called.is_empty() || called == format!("#{}", pilot.0) {
                                format!("(#{})", pilot.0)
                            } else {
                                format!("({called} #{})", pilot.0)
                            }
                        }
                        None => "(machine)".to_owned(),
                    };
                    lines.push(format!(
                        "  {team} {name} {pilot}: {}, {}, {}.",
                        outcome_word(held.outcome),
                        counted(held.shots, "shot", "shots"),
                        counted(held.hits, "hit", "hits"),
                    ));
                }
            }
        }
    }
    lines
}

/// `42 chunks`.
fn counted_u64(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// `this page: chunks 0 to 15, 412 lines, 0 unreadable.`
fn page_words(page: &Page) -> String {
    format!(
        "this page: chunks {} to {}, {}, {} unreadable.",
        page.first,
        page.last,
        counted_u64(
            u64::try_from(page.lines).unwrap_or(u64::MAX),
            "line",
            "lines"
        ),
        page.unreadable
    )
}

/// `ab12…ef`: a digest's first four and last two digits, `none` for none.
fn short_digest(digest: &str) -> String {
    if digest.is_empty() {
        return "none".to_owned();
    }
    let chars: Vec<char> = digest.chars().collect();
    if chars.len() <= 6 {
        return digest.to_owned();
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars.iter().skip(chars.len().saturating_sub(2)).collect();
    format!("{head}…{tail}")
}

/// `You fired 14 rounds of cannon and 60 of machine_gun.` / `You fired
/// nothing.`
fn fired_line(rounds: &BTreeMap<SmolStr, u32>) -> String {
    let fired: Vec<(&SmolStr, u32)> = rounds
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(weapon, count)| (weapon, *count))
        .collect();
    let parts: Vec<String> = fired
        .iter()
        .enumerate()
        .map(|(n, (weapon, count))| {
            if n == 0 {
                format!("{} of {weapon}", counted(*count, "round", "rounds"))
            } else {
                format!("{count} of {weapon}")
            }
        })
        .collect();
    match parts.split_last() {
        None => "You fired nothing.".to_owned(),
        Some((only, [])) => format!("You fired {only}."),
        Some((last, rest)) => format!("You fired {} and {last}.", rest.join(", ")),
    }
}

/// **The header of a tape the centre does not remember** (16b cut V.3): a
/// match still running, or older than the ring. There is no result to read the
/// public part from, so it names the match and the page and nothing else.
#[must_use]
pub fn untracked_header(match_id: MatchId, running: bool, page: &Page) -> Vec<String> {
    let head = if running {
        format!("Match {} is still running.", match_id.0)
    } else {
        format!(
            "Match {} is not among the centre's last {HISTORY_DEPTH} matches.",
            match_id.0
        )
    };
    vec![
        head,
        format!(
            "tape {}; {}",
            crate::arena_bridge::match_log_id(match_id),
            page_words(page)
        ),
    ]
}

// ---------------------------------------------------------------------------
// The report document
// ---------------------------------------------------------------------------

/// **One report document** (16b cut V.3), as [`crate::match_::finish`] writes it
/// at [`report_key`]:
///
/// ```json
/// { "record": "tacsim.report", "record_version": 1, "match": 12, "unit": 101, "pilot": 5,
///   "callsign": "RA", "lines": [ "  0:04  power online." ], "truncated": 0,
///   "tail": [ "  4:32  the match ended: red eliminated." ], "gap": false,
///   "rounds": { "cannon": 14 }, "kills": 1 }
/// ```
///
/// **17b cut M5.6**: and `"mind": { "lines": [ "Your mind: Standard Doctrine
/// (Line).", … ] }` after them when the report carries its `Your mind` block
/// ([`Report::mind`]), and no such key when it does not.
#[must_use]
pub fn report_document(
    match_id: MatchId,
    unit: ObjId,
    pilot: Option<ObjId>,
    callsign: &str,
    report: &Report,
) -> serde_json::Value {
    let mut document = serde_json::json!({
        "record": REPORT_RECORD,
        "record_version": 1,
        "match": match_id.0,
        "unit": unit.0,
        "pilot": pilot.map(|pilot| pilot.0),
        "callsign": callsign,
        "lines": report.lines,
        "truncated": report.truncated,
        "tail": report.tail,
        "gap": report.gap,
        "rounds": report.rounds,
        "kills": report.kills,
    });
    // 17b cut M5.6: the `Your mind` block, only when it is printed, so every
    // report without one is the document it was.
    if let (Some(mind), Some(fields)) = (&report.mind, document.as_object_mut()) {
        fields.insert("mind".to_owned(), serde_json::json!(mind));
    }
    document
}

/// **Read one report document back** (16b cut V.3): `None` unless its `record`
/// is `tacsim.report` and the rest decodes.
#[must_use]
pub fn report_from_document(value: &serde_json::Value) -> Option<Report> {
    if value.get("record").and_then(serde_json::Value::as_str) != Some(REPORT_RECORD) {
        return None;
    }
    serde_json::from_value::<Report>(value.clone()).ok()
}

// ---------------------------------------------------------------------------
// The two requests and the one completion
// ---------------------------------------------------------------------------

/// **One store read in flight for one actor** (16b cut V.3), on
/// [`MatchBook`]'s `pending_reads` — one per actor, superseded past
/// [`mush_core::STORE_READ_STALE_SECS`], as the scenes precedent does.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingRead {
    /// `SIM REPORT`: one report document.
    Pilot {
        /// What [`Ctx::read_store`] returned.
        correlation: u64,
        /// World seconds.
        requested_at: i64,
        /// The match.
        match_id: MatchId,
        /// The unit the actor flew in it.
        unit: ObjId,
        /// The arena's authored name, read at request time.
        label: String,
    },
    /// `@arena/report`: one page of the tape.
    Staff {
        /// What [`Ctx::read_store`] returned.
        correlation: u64,
        /// World seconds.
        requested_at: i64,
        /// The match.
        match_id: MatchId,
        /// The first chunk asked for.
        first_chunk: u64,
        /// The arena's authored name, read at request time.
        label: String,
        /// Who flew what, with the pilots' display names read at request time.
        roster: Roster,
        /// The remembered result, when the ring holds the match (boxed: it is
        /// most of the row, and a pilot's pending read carries none).
        row: Option<Box<HistoryRow>>,
        /// Whether the match was live at request time.
        running: bool,
    },
}

impl PendingRead {
    /// The correlation this row waits on.
    #[must_use]
    pub const fn correlation(&self) -> u64 {
        match self {
            PendingRead::Pilot { correlation, .. } | PendingRead::Staff { correlation, .. } => {
                *correlation
            }
        }
    }

    /// When it was asked, in world seconds.
    #[must_use]
    pub const fn requested_at(&self) -> i64 {
        match self {
            PendingRead::Pilot { requested_at, .. } | PendingRead::Staff { requested_at, .. } => {
                *requested_at
            }
        }
    }
}

/// The one line both requests refuse a second read with.
pub const READ_IN_FLIGHT: &str = "A report is already being read for you; wait for it.";

/// `#12` or `12` as a match id.
fn match_word(word: &str) -> Option<MatchId> {
    let digits = word.strip_prefix('#').unwrap_or(word);
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| digits.parse::<u64>().ok())
        .flatten()
        .map(MatchId)
}

/// Whether `actor` already has a read younger than the stale window.
fn read_in_flight(ctx: &Ctx<'_>, actor: ObjId) -> bool {
    let now = ctx.w.now();
    ctx.w
        .module_state::<MatchBook>()
        .and_then(|book| book.pending_reads.get(&actor))
        .is_some_and(|pending| {
            now.saturating_sub(pending.requested_at()) < mush_core::STORE_READ_STALE_SECS
        })
}

/// **`SIM REPORT [<match>]`** (16b cut V.3; 16c N2, Q37): read the actor's own
/// report of one match — **one** `tacsim:report:<match>:<unit>` document,
/// through [`Ctx::read_store`] and [`ReportRead`].
///
/// An empty word is the newest remembered match the actor piloted. Refusals,
/// in this order: the usage line; `You have not flown a match the centre
/// remembers.`; `Match 12 is still running.` (the book's live match, checked
/// before the ring, which never holds one); `The centre remembers its last 20
/// matches; match 12 is not one of them.`; `The server restarted during match
/// 3; there is no debrief of it.` (before the pilot check, because a restart
/// row has no pilot rows); `You did not fly in match 12.`; `Match 3 was
/// recorded before reports existed; there is nothing to read of it.`; and
/// [`READ_IN_FLIGHT`]. The centre gate is `sim_command`'s, like every
/// selector's.
///
/// # Errors
///
/// Every refusal above, as the line the player sees.
pub fn request(ctx: &mut Ctx<'_>, word: &str) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let word = word.trim();
    let named = if word.is_empty() {
        None
    } else {
        Some(match_word(word).ok_or_else(|| CmdError::msg("Usage: SIM REPORT [<match>]"))?)
    };
    let (row, live) = {
        let Some(book) = ctx.w.module_state::<MatchBook>() else {
            return Err(CmdError::msg(
                "You have not flown a match the centre remembers.",
            ));
        };
        match named {
            None => {
                let row = book
                    .history()
                    .find(|row| row.pilot_unit(actor).is_some())
                    .cloned()
                    .ok_or_else(|| {
                        CmdError::msg("You have not flown a match the centre remembers.")
                    })?;
                (row, false)
            }
            Some(id) => {
                let live = book.get(id).is_some_and(|record| record.state.is_live());
                (
                    book.remembered(id)
                        .cloned()
                        .unwrap_or_else(|| empty_row(id)),
                    live,
                )
            }
        }
    };
    let id = row.id;
    if live {
        return Err(CmdError::msg(format!("Match {} is still running.", id.0)));
    }
    let remembered = ctx
        .w
        .module_state::<MatchBook>()
        .is_some_and(|book| book.remembered(id).is_some());
    if !remembered {
        return Err(CmdError::msg(format!(
            "The centre remembers its last {HISTORY_DEPTH} matches; match {} is not one of them.",
            id.0
        )));
    }
    if row.reason == "restart" {
        return Err(CmdError::msg(format!(
            "The server restarted during match {}; there is no debrief of it.",
            id.0
        )));
    }
    let Some(unit) = row.pilot_unit(actor).map(|flown| flown.unit) else {
        return Err(CmdError::msg(format!("You did not fly in match {}.", id.0)));
    };
    // A row whose result names no report at all was written before this cut (a
    // row written since names every pilot's unit, so an empty list and "not in
    // it" are the same test).
    if row.reports.is_empty() {
        return Err(CmdError::msg(format!(
            "Match {} was recorded before reports existed; there is nothing to read of it.",
            id.0
        )));
    }
    if read_in_flight(ctx, actor) {
        return Err(CmdError::msg(READ_IN_FLIGHT));
    }
    let label = arena_label(ctx.w, &row.arena);
    let correlation = match ctx.read_store(vec![report_key(id, unit)]) {
        Ok(correlation) => correlation,
        Err(error) => {
            return Err(CmdError::msg(format!(
                "Your report of match {} cannot be requested: {error}.",
                id.0
            )));
        }
    };
    let requested_at = ctx.w.now();
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.pending_reads.insert(
            actor,
            PendingRead::Pilot {
                correlation,
                requested_at,
                match_id: id,
                unit,
                label,
            },
        );
    }
    ctx.out.sys_to_actor(
        actor,
        NOTICE_CATEGORY,
        format!("Reading your report of match {} …", id.0),
    );
    Ok(())
}

/// A row that stands for a match id the ring does not hold, so the request's
/// checks read in one order whichever arm found the id.
fn empty_row(id: MatchId) -> HistoryRow {
    HistoryRow {
        id,
        outcome: MatchOutcome::Abort,
        reason: SmolStr::default(),
        arena: SmolStr::default(),
        started_ms: 0,
        ended_ms: 0,
        arena_ms: 0,
        log: String::new(),
        digest: String::new(),
        chunks: 0,
        seed: 0,
        units: Vec::new(),
        named: false,
        reports: Vec::new(),
        replay: false,
        mode: None,
    }
}

/// **`@arena/report [<match>] [<chunk>]`** (16b cut V.3): one page of a match's
/// tape for staff — [`mush_core::MAX_STORE_READ_KEYS`] chunks of the module's own
/// `log:match.<id>` from the chunk named (0 by default), one line an event.
///
/// An empty match word is the newest remembered match (`The centre remembers no
/// match.` when there is none); a match the ring does not hold is still read,
/// because the tape may outlive the ring, and its units then print as `#n`.
/// **No live, restart or named gate**: a running or crashed match's tape is
/// exactly what an operator reads. The verb's own staff gate applies; no centre
/// gate does.
///
/// # Errors
///
/// The usage line, `The centre remembers no match.` and [`READ_IN_FLIGHT`].
pub fn request_page(ctx: &mut Ctx<'_>, args: &str) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @arena/report [<match>] [<chunk>]";
    let actor = ctx.actor;
    let words: Vec<&str> = args.split_whitespace().collect();
    let (named, first_chunk) = match words.as_slice() {
        [] => (None, 0),
        [id] => (Some(match_word(id).ok_or_else(|| CmdError::msg(USAGE))?), 0),
        [id, chunk] => (
            Some(match_word(id).ok_or_else(|| CmdError::msg(USAGE))?),
            chunk
                .parse::<u64>()
                .ok()
                .filter(|_| chunk.chars().all(|c| c.is_ascii_digit()))
                .ok_or_else(|| CmdError::msg(USAGE))?,
        ),
        _ => return Err(CmdError::msg(USAGE)),
    };
    let (match_id, row, roster, running, arena) = {
        let book = ctx.w.module_state::<MatchBook>();
        let match_id = match named {
            Some(id) => id,
            None => book
                .and_then(|book| book.history().next().map(|row| row.id))
                .ok_or_else(|| CmdError::msg("The centre remembers no match."))?,
        };
        let row = book.and_then(|book| book.remembered(match_id).cloned().map(Box::new));
        let record = book.and_then(|book| book.get(match_id));
        let running = record.is_some_and(|record| record.state.is_live());
        // The live record names a running match's units; the remembered result
        // names a finished one's; a match neither holds prints `#n`.
        let (roster, arena) = match (&row, record) {
            (Some(row), _) => (Roster::from_row(row.as_ref()), row.arena.clone()),
            (None, Some(record)) => (Roster::from_record(record), record.arena.clone()),
            (None, None) => (Roster::new(), SmolStr::default()),
        };
        (match_id, row, roster, running, arena)
    };
    if read_in_flight(ctx, actor) {
        return Err(CmdError::msg(READ_IN_FLIGHT));
    }
    let mut roster = roster;
    roster.name_pilots(|pilot| lobby::display_name(ctx.w, pilot));
    let label = arena_label(ctx.w, &arena);
    let log_id = crate::arena_bridge::match_log_id(match_id);
    let last_chunk =
        first_chunk.saturating_add(u64::try_from(mush_core::MAX_STORE_READ_KEYS).unwrap_or(16) - 1);
    let keys: Vec<SmolStr> = (first_chunk..=last_chunk)
        .map(|n| mush_core::chunk_key(&log_id, n))
        .collect();
    let correlation = match ctx.read_store(keys) {
        Ok(correlation) => correlation,
        Err(error) => {
            return Err(CmdError::msg(format!(
                "The tape of match {} cannot be requested: {error}.",
                match_id.0
            )));
        }
    };
    let requested_at = ctx.w.now();
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.pending_reads.insert(
            actor,
            PendingRead::Staff {
                correlation,
                requested_at,
                match_id,
                first_chunk,
                label,
                roster,
                row,
                running,
            },
        );
    }
    ctx.out.sys_to_actor(
        actor,
        NOTICE_CATEGORY,
        format!(
            "Reading chunks {first_chunk} to {last_chunk} of match {} …",
            match_id.0
        ),
    );
    Ok(())
}

/// Say `lines` to `actor` in blocks of [`REPORT_LINES_PER_EVENT`], each block one
/// `Sys` event joined by `'\n'`.
fn say_blocks(ctx: &mut Ctx<'_>, actor: ObjId, lines: &[String]) {
    for block in lines.chunks(REPORT_LINES_PER_EVENT) {
        ctx.out
            .sys_to_actor(actor, NOTICE_CATEGORY, block.join("\n"));
    }
}

/// Say each line as its own event (the header's few lines).
fn say_lines(ctx: &mut Ctx<'_>, actor: ObjId, lines: Vec<String>) {
    for line in lines {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
}

/// **The tacsim module's store-read completion** (16b cut V.3) — the re-entry of
/// both [`request`] and [`request_page`], registered once by
/// `crate::arena_bridge::register`.
///
/// It takes the actor's pending row whose correlation matches (a stale or
/// unknown correlation is dropped with a warning, as the precedents do),
/// re-validates against the world as it now stands — the actor still exists, a
/// pilot is still in the centre, staff are still staff — and renders. A read is
/// a view: nothing here writes a document or mutates the world beyond the
/// pending row it consumed.
pub struct ReportRead;

impl StoreReadCompletion for ReportRead {
    fn on_read(&self, ctx: &mut Ctx<'_>, correlation: u64, result: StoreReadResult) {
        let actor = ctx.actor;
        let pending = {
            let Some(book) = ctx.w.module_state_mut::<MatchBook>() else {
                return;
            };
            match book.pending_reads.get(&actor) {
                Some(pending) if pending.correlation() == correlation => {
                    book.pending_reads.remove(&actor)
                }
                Some(pending) => {
                    tracing::warn!(
                        %actor,
                        correlation,
                        pending = pending.correlation(),
                        "tacsim: a stale report read completion was dropped"
                    );
                    return;
                }
                None => {
                    tracing::warn!(
                        %actor,
                        correlation,
                        "tacsim: a report read completion with no pending read was dropped"
                    );
                    return;
                }
            }
        };
        let Some(pending) = pending else {
            return;
        };
        if ctx.w.get(actor).is_none() {
            return;
        }
        match pending {
            PendingRead::Pilot {
                match_id,
                unit,
                label,
                ..
            } => pilot_read(ctx, actor, match_id, unit, &label, result),
            PendingRead::Staff {
                match_id,
                first_chunk,
                label,
                roster,
                row,
                running,
                ..
            } => {
                if !lobby::is_staff(ctx.w, actor) {
                    ctx.out
                        .sys_to_actor(actor, NOTICE_CATEGORY, "Permission denied.");
                    return;
                }
                staff_read(
                    ctx,
                    actor,
                    StaffPage {
                        match_id,
                        first_chunk,
                        label: &label,
                        roster: &roster,
                        row: row.as_deref(),
                        running,
                    },
                    result,
                );
            }
        }
    }
}

/// `SIM REPORT`'s half of the completion.
fn pilot_read(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    match_id: MatchId,
    unit: ObjId,
    label: &str,
    result: StoreReadResult,
) {
    // Re-validate: still in the centre, like every selector.
    if let Err(refusal) = lobby::outside_centre(ctx.w, actor) {
        ctx.out
            .sys_to_actor(actor, NOTICE_CATEGORY, refusal.to_string());
        return;
    }
    let id = match_id.0;
    let docs = match result {
        StoreReadResult::Failed(detail) => {
            ctx.out.sys_to_actor(
                actor,
                NOTICE_CATEGORY,
                format!("The report of match {id} could not be read ({detail}); ask again."),
            );
            return;
        }
        StoreReadResult::Oversized { .. } => {
            ctx.out.sys_to_actor(
                actor,
                NOTICE_CATEGORY,
                format!("The report of match {id} is too large to read at once."),
            );
            return;
        }
        StoreReadResult::Docs { docs, .. } => docs,
    };
    let Some(row) = ctx
        .w
        .module_state::<MatchBook>()
        .and_then(|book| book.remembered(match_id).cloned())
    else {
        // The ring let it go between the request and now, and the eviction
        // deleted the document with it.
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!(
                "The centre remembers its last {HISTORY_DEPTH} matches; match {id} is not one of \
                 them."
            ),
        );
        return;
    };
    let key = report_key(match_id, unit);
    let Some((_, value)) = docs.iter().find(|(held, _)| *held == key) else {
        // The document rides the same batch as the result that put the row in
        // the ring, so an absent key is a read that outran the commit.
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("Your report of match {id} is still being written; ask again in a moment."),
        );
        return;
    };
    let reader = Reader::Pilot { unit };
    let Some(report) = report_from_document(value) else {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("Your report of match {id} cannot be read; only its result remains."),
        );
        say_lines(ctx, actor, header_lines(&row, label, reader, None, None));
        return;
    };
    say_lines(
        ctx,
        actor,
        header_lines(&row, label, reader, Some(&report), None),
    );
    // **17b cut M5.6**: the `Your mind` block after the header's lines and
    // before the gap notice and the timeline, when it is printed — to this
    // pilot alone, whose report it is.
    if let Some(mind) = &report.mind {
        say_lines(ctx, actor, mind.lines.clone());
    }
    if report.gap {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            "Part of the tape is missing; this is what remains.",
        );
    }
    say_blocks(ctx, actor, &report.lines);
    if report.truncated > 0 {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("  … and {} more.", report.truncated),
        );
    }
    say_blocks(ctx, actor, &report.tail);
}

/// What a staff page's completion carries over from its request.
struct StaffPage<'a> {
    match_id: MatchId,
    first_chunk: u64,
    label: &'a str,
    roster: &'a Roster,
    row: Option<&'a HistoryRow>,
    running: bool,
}

impl StaffPage<'_> {
    /// The header, for this page's numbers.
    fn header(&self, page: &Page) -> Vec<String> {
        match self.row {
            Some(row) => header_lines(
                row,
                self.label,
                Reader::Staff,
                None,
                Some((page, self.roster)),
            ),
            None => untracked_header(self.match_id, self.running, page),
        }
    }
}

/// `@arena/report`'s half of the completion.
fn staff_read(ctx: &mut Ctx<'_>, actor: ObjId, at: StaffPage<'_>, result: StoreReadResult) {
    let id = at.match_id.0;
    let chunk = at.first_chunk;
    let keys = u64::try_from(mush_core::MAX_STORE_READ_KEYS).unwrap_or(16);
    let empty = Page {
        first: chunk,
        last: chunk.saturating_add(keys - 1),
        lines: 0,
        unreadable: 0,
    };
    let (docs, absent) = match result {
        StoreReadResult::Failed(detail) => {
            ctx.out.sys_to_actor(
                actor,
                NOTICE_CATEGORY,
                format!("The tape of match {id} could not be read ({detail}); ask again."),
            );
            return;
        }
        StoreReadResult::Oversized { .. } => {
            ctx.out.sys_to_actor(
                actor,
                NOTICE_CATEGORY,
                format!("The tape of match {id} is too large to read at once."),
            );
            return;
        }
        StoreReadResult::Docs { docs, absent } => (docs, absent),
    };
    if docs.is_empty() {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("Match {id} has no tape at chunk {chunk} on this server."),
        );
        let header = at.header(&empty);
        say_lines(ctx, actor, header);
        return;
    }
    let log_id = crate::arena_bridge::match_log_id(at.match_id);
    let folded = match fold_page(&docs, &log_id, chunk, at.roster) {
        Ok(folded) => folded,
        Err(error) => {
            ctx.out.sys_to_actor(
                actor,
                NOTICE_CATEGORY,
                format!("The tape of match {id} cannot be read at chunk {chunk} ({error})."),
            );
            let header = at.header(&empty);
            say_lines(ctx, actor, header);
            return;
        }
    };
    let header = at.header(&folded.page);
    say_lines(ctx, actor, header);
    let report = folded.report;
    if report.gap {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            "Part of the tape is missing; this is what remains.",
        );
    }
    say_blocks(ctx, actor, &report.lines);
    if report.truncated > 0 {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("  … and {} more of this page.", report.truncated),
        );
    }
    if let Some(next) = next_page(chunk, folded.cut_at, absent.is_empty()) {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("Next: @arena/report {id} {next}"),
        );
    }
}

/// **One staff page, folded** (16b cut V.3): what [`fold_page`] answers.
#[derive(Clone, Debug, PartialEq)]
pub struct PageFold {
    /// The page's lines, at most [`REPORT_MAX_LINES`].
    pub report: Report,
    /// The page's numbers, for the header.
    pub page: Page,
    /// The chunk the first line the cap dropped came from, or `None` when the
    /// page kept every line — where `Next:` must go on from, so no part of the
    /// tape is stepped over by following it.
    pub cut_at: Option<u64>,
}

/// **Fold one page of a tape for staff** (16b cut V.3): the chunk documents a
/// store read brought back (absent keys already dropped) validated by
/// [`mush_core::read_cold_range`], every tape line of every entry parsed and
/// absorbed under [`Reader::Staff`] in `seq` order (a line that does not parse
/// is counted as unreadable), and the chunk each line came from kept so a page
/// cut short at [`REPORT_MAX_LINES`] can say where the rest of it starts.
///
/// **16b cut AF.3: the page's entries go through
/// [`crate::arena_bridge::tape_in_order`] first**, so a resumed match's
/// abandoned tail — the chunks the old server wrote after the snapshot it
/// resumed from — is not folded, and each resume marker prints its line
/// ([`crate::arena_bridge::resume_marker_line`]) at its place: after the last
/// chunk it keeps and before the first resumed one. **Nothing on the fold is
/// reset** at a marker: `expected` counts the tape's events, and the truncated
/// tape is contiguous by construction (chunk N's last event is followed by the
/// resumed chunk N + 1's first, which the snapshot's open buffer began where
/// chunk N ended), so a gap there would be a real one. **The limit of a paged
/// reader**: an abandoned chunk whose marker sits on the NEXT page is shown on
/// this one as it is, and the next page's marker line says which chunks it
/// retired.
///
/// # Errors
///
/// [`mush_core::read_cold_range`]'s refusal of the page.
pub fn fold_page(
    docs: &[(SmolStr, serde_json::Value)],
    log_id: &str,
    first_chunk: u64,
    roster: &Roster,
) -> Result<PageFold, mush_core::LedgerReadError> {
    let read = mush_core::read_cold_range(docs, log_id)?;
    // 16b cut AF.3: the entries in the order the tape counts, each kept chunk
    // with the ledger seq of the entry it is (the last one written for its
    // arena chunk, which is the one `tape_in_order` keeps), and the markers.
    let values: Vec<serde_json::Value> = read.iter().map(|entry| entry.value.clone()).collect();
    let (kept, markers) = crate::arena_bridge::tape_in_order(&values);
    let mut seq_of_chunk: BTreeMap<u64, u64> = BTreeMap::new();
    for entry in &read {
        if let Some(chunk) = entry.value.get("chunk").and_then(serde_json::Value::as_u64) {
            seq_of_chunk.insert(chunk, entry.seq);
        }
    }
    let entries: Vec<(u64, u64, serde_json::Value)> = kept
        .into_iter()
        .filter_map(|value| {
            let chunk = value.get("chunk").and_then(serde_json::Value::as_u64)?;
            Some((chunk, seq_of_chunk.get(&chunk).copied().unwrap_or(0), value))
        })
        .collect();
    let mut markers: std::collections::VecDeque<(u64, String)> = markers
        .iter()
        .filter_map(|marker| {
            let from = marker
                .get("resumed")
                .and_then(|resumed| resumed.get("from_chunk"))
                .and_then(serde_json::Value::as_u64)?;
            Some((from, crate::arena_bridge::resume_marker_line(marker)?))
        })
        .collect();
    // Which chunk holds each entry: `read_cold_range` has just validated every
    // document as a chunk of this log with contiguous, gap-free entries.
    let mut chunk_of: BTreeMap<u64, u64> = BTreeMap::new();
    let mut last = first_chunk;
    for (key, value) in docs {
        let Some((_, n)) = mush_core::parse_chunk_key(key) else {
            continue;
        };
        last = last.max(n);
        let held = value
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for entry in held {
            if let Some(seq) = entry.get("seq").and_then(serde_json::Value::as_u64) {
                chunk_of.insert(seq, n);
            }
        }
    }
    let mut fold = Fold::new(
        roster.clone(),
        Reader::Staff,
        ReportRules {
            designate_on_detection: false,
        },
    );
    let (mut parsed, mut unreadable) = (0_usize, 0_usize);
    let mut cut_at: Option<u64> = None;
    for (chunk, seq, value) in &entries {
        // Every marker that retired the chunks above its own, said before the
        // first chunk it did not retire.
        while markers.front().is_some_and(|(from, _)| *from < *chunk) {
            if let Some((_, line)) = markers.pop_front() {
                fold.mark(line);
            }
        }
        let text = value
            .get("lines")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        for line in text.split('\n').filter(|line| !line.trim().is_empty()) {
            match serde_json::from_str::<LedgerEvent>(line) {
                Ok(event) => {
                    parsed = parsed.saturating_add(1);
                    let before = fold.truncated();
                    fold.absorb(&event);
                    if cut_at.is_none() && fold.truncated() > before {
                        cut_at = Some(chunk_of.get(seq).copied().unwrap_or(first_chunk));
                    }
                }
                Err(_) => unreadable = unreadable.saturating_add(1),
            }
        }
    }
    // A marker with nothing after it on this page is said at the page's end.
    for (_, line) in markers {
        fold.mark(line);
    }
    Ok(PageFold {
        report: fold.finish(),
        page: Page {
            first: first_chunk,
            last,
            lines: parsed,
            unreadable,
        },
        cut_at,
    })
}

/// **Where `Next:` goes on from** (16b cut V.3), or `None` for no `Next:`.
///
/// A page cut short at [`REPORT_MAX_LINES`] goes on from the chunk its first
/// dropped line came from — whether or not the store returned every key, since
/// the lines it dropped are tape nobody has read — and a whole page that
/// returned every key it asked for goes on from the chunk after it. A page cut
/// short inside its own first chunk goes on from the next one, so following
/// `Next:` always moves; the omission line has said what that chunk held back.
#[must_use]
pub fn next_page(first_chunk: u64, cut_at: Option<u64>, every_key: bool) -> Option<u64> {
    let keys = u64::try_from(mush_core::MAX_STORE_READ_KEYS).unwrap_or(16);
    match cut_at {
        Some(cut) => Some(cut.max(first_chunk.saturating_add(1))),
        None if every_key => Some(first_chunk.saturating_add(keys)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::damage;

    const ME: ObjId = ObjId(101);
    const THEM: ObjId = ObjId(201);
    const SEEN: ContactId = ContactId(0x0123_4567_89ab_cdef);

    /// Red's `RA` (#101, pilot #5) against blue's `BA` (#201, pilot #6).
    fn roster() -> Roster {
        let mut roster = Roster::new();
        roster.insert(ME, Some(Team::Red), SmolStr::new("RA"), Some(ObjId(5)));
        roster.insert(THEM, Some(Team::Blue), SmolStr::new("BA"), Some(ObjId(6)));
        roster
    }

    const RULES: ReportRules = ReportRules {
        designate_on_detection: false,
    };

    /// A tape written by hand: each event numbered in order from 1.
    struct Tape(Vec<LedgerEvent>);

    impl Tape {
        fn new() -> Tape {
            Tape(Vec::new())
        }

        fn at(&mut self, t_ms: i64, kind: EventKind) -> &mut Tape {
            self.drawn(t_ms, kind, Vec::new())
        }

        fn drawn(&mut self, t_ms: i64, kind: EventKind, draws: Vec<(String, u64)>) -> &mut Tape {
            let seq = u64::try_from(self.0.len()).unwrap_or(0) + 1;
            self.0.push(LedgerEvent {
                seq,
                t_ms,
                kind,
                draws,
                hash: 0x0fed_cba9_8765_4321,
            });
            self
        }
    }

    fn fired(unit: ObjId, weapon: &str, shot_seq: u64, target: ObjId, rounds: u32) -> EventKind {
        EventKind::Fired {
            unit,
            weapon: SmolStr::new(weapon),
            shot_seq,
            target,
            range_cm: 43_000,
            rounds,
            by: None,
            slot: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn hit(
        shot_seq: u64,
        unit: ObjId,
        target: ObjId,
        section: &str,
        facet: &str,
        armor_section: &str,
        armor_left: u32,
        internals_left: u32,
    ) -> EventKind {
        EventKind::Hit {
            shot_seq,
            unit,
            target,
            section: SmolStr::new(section),
            facet: SmolStr::new(facet),
            armor_section: SmolStr::new(armor_section),
            rounds: 1,
            damage: 120,
            armor_left,
            internals_left,
            consequences: Vec::new(),
            cause: SmolStr::new(format!("shot:{shot_seq}")),
        }
    }

    /// **The brief's tape**: a detection, a miss at a blip, the classification,
    /// a lock, a hit that takes the turret off, a shot fired at a classified
    /// target that lands after the contact is lost, the kill, two rounds on the
    /// pilot's own tank, a lost track, the pilot's own wreck and the end.
    fn the_tape() -> Vec<LedgerEvent> {
        let draws = |label: &str| vec![(label.to_owned(), 17_u64)];
        let mut tape = Tape::new();
        tape.at(4_000, EventKind::PowerOnline { unit: ME })
            .at(
                10_000,
                EventKind::Detected {
                    viewer: ME,
                    contact: SEEN,
                    target: THEM,
                },
            )
            .at(20_000, fired(ME, "cannon", 1, THEM, 1))
            .drawn(
                21_000,
                EventKind::Miss {
                    shot_seq: 1,
                    reason: SmolStr::new(damage::DISPERSION),
                },
                draws("hit:1"),
            )
            .drawn(
                25_000,
                EventKind::Classified {
                    viewer: ME,
                    contact: SEEN,
                    target: THEM,
                    team: Team::Blue,
                },
                draws("classify:101:201:3"),
            )
            .at(
                30_000,
                EventKind::Locked {
                    unit: ME,
                    contact: SEEN,
                    target: THEM,
                },
            )
            .at(42_000, fired(ME, "cannon", 2, THEM, 1))
            .drawn(
                43_000,
                hit(2, ME, THEM, "turret", "front", "turret", 0, 0),
                vec![("hit:2".to_owned(), 5), ("facet:2".to_owned(), 9)],
            )
            .at(
                43_000,
                EventKind::SectionDestroyed {
                    unit: THEM,
                    section: SmolStr::new("turret"),
                    by: Some(ME),
                    consequences: vec![SmolStr::new("remove_weapons")],
                },
            )
            .at(50_000, fired(ME, "machine_gun", 3, THEM, 5))
            .at(
                50_500,
                EventKind::Lost {
                    viewer: ME,
                    contact: SEEN,
                    target: THEM,
                },
            )
            .at(51_000, hit(3, ME, THEM, "hull", "front", "hull", 180, 200))
            .at(
                60_000,
                EventKind::UnitDestroyed {
                    unit: THEM,
                    by: Some(ME),
                },
            )
            .at(70_000, hit(9, THEM, ME, "hull", "front", "hull", 33, 200))
            .at(71_000, hit(10, THEM, ME, "engine", "rear", "hull", 0, 40))
            .at(
                125_000,
                EventKind::SectionDestroyed {
                    unit: ME,
                    section: SmolStr::new("left_track"),
                    by: Some(THEM),
                    consequences: vec![SmolStr::new("slow")],
                },
            )
            .at(
                192_000,
                EventKind::UnitDestroyed {
                    unit: ME,
                    by: Some(THEM),
                },
            )
            .at(
                272_000,
                EventKind::MatchEnded {
                    reason: EndReason::Elimination {
                        winner: Some(Team::Blue),
                    },
                },
            );
        tape.0
    }

    /// **The no-leak grep** (16c N2): the first thing a pilot line may not
    /// carry, or `None`. A word of sixteen base-16 digits is a contact id.
    fn leak(line: &str) -> Option<&'static str> {
        let contact = line
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| {
                word.len() == 16
                    && word
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            });
        if contact {
            return Some("a contact id");
        }
        ["#", "seed", "hash", "draw", " m:", "damage"]
            .into_iter()
            .find(|word| line.contains(word))
    }

    fn every_line(report: &Report) -> Vec<&String> {
        report.lines.iter().chain(report.tail.iter()).collect()
    }

    /// **16b cut V.3**: the brief's tape, told to the pilot who fired — exactly
    /// the pinned lines, the fog as they lived it.
    #[test]
    fn v3_the_shooter_reads_their_own_shots_through_their_own_fog() {
        let report = fold(&the_tape(), &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            report.lines,
            vec![
                "  0:04  power online.",
                // Held as a blip: unnamed, and a miss in the cockpit's words.
                "  0:20  cannon at a contact: miss: the round went wide.",
                "  0:30  locked BA.",
                // Classified when it landed: named, the section, and the
                // section-destroyed clause on the shot that did it.
                "  0:42  cannon at BA: hit, turret; BA's turret destroyed.",
                // Classified when FIRED and lost before it landed: a contact
                // again, and no section — what the pilot was told.
                "  0:50  machine_gun at a contact: hit.",
                "  1:00  a contact destroyed by your round.",
                // The victim's own knowledge, and never who fired.
                "  1:10  hit on the hull's front facet: 33 armour left.",
                "  1:11  hit on the engine, through the hull's rear facet: armour gone, 40 inside.",
                "  2:05  your left_track destroyed: the tank is slowed.",
            ]
        );
        assert_eq!(
            report.tail,
            vec![
                "  3:12  your tank destroyed.",
                "  4:32  the match ended: red eliminated.",
            ]
        );
        assert_eq!(report.truncated, 0);
        assert!(!report.gap);
        assert_eq!(report.kills, 1);
        assert_eq!(report.last_ms, 272_000);
        assert_eq!(
            report.rounds,
            BTreeMap::from([
                (SmolStr::new("cannon"), 2),
                (SmolStr::new("machine_gun"), 5)
            ])
        );
        for line in every_line(&report) {
            assert_eq!(leak(line), None, "{line}");
            assert!(!line.contains("RA"), "{line}");
        }
    }

    /// **16b cut V.3**: under `designate_on_detection` the blip is named at
    /// once — and still earns no section before it is told apart.
    #[test]
    fn v3_designation_on_detection_names_at_once_and_still_withholds_the_section() {
        let rules = ReportRules {
            designate_on_detection: true,
        };
        let report = fold(&the_tape(), &roster(), Reader::Pilot { unit: ME }, rules);
        assert_eq!(
            report.lines.get(1).map(String::as_str),
            Some("  0:20  cannon at BA: miss: the round went wide.")
        );
        // The same shot as a hit, landing before the classification.
        let mut tape = the_tape();
        if let Some(event) = tape.get_mut(3) {
            event.kind = hit(1, ME, THEM, "turret", "front", "turret", 100, 150);
        }
        let report = fold(&tape, &roster(), Reader::Pilot { unit: ME }, rules);
        assert_eq!(
            report.lines.get(1).map(String::as_str),
            Some("  0:20  cannon at BA: hit.")
        );
        // Without the switch the same hit is a contact's.
        let report = fold(&tape, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            report.lines.get(1).map(String::as_str),
            Some("  0:20  cannon at a contact: hit.")
        );
    }

    /// **16b cut V.3**: the target reads the same tape as its own hits and its
    /// own end — no shot line, no shooter, no callsign of the other side.
    #[test]
    fn v3_the_target_reads_its_own_tank_and_nothing_of_the_shooter() {
        let report = fold(&the_tape(), &roster(), Reader::Pilot { unit: THEM }, RULES);
        assert_eq!(
            report.lines,
            vec![
                "  0:43  hit on the turret's front facet: armour gone, 0 inside.",
                "  0:43  your turret destroyed: its weapons are gone.",
                "  0:51  hit on the hull's front facet: 180 armour left.",
                // Its own round finished RA — which it never held, so it is a contact.
                "  3:12  a contact destroyed by your round.",
            ]
        );
        assert_eq!(
            report.tail,
            vec![
                "  1:00  your tank destroyed.",
                "  4:32  the match ended: red eliminated.",
            ]
        );
        assert_eq!(report.kills, 1);
        assert!(report.rounds.is_empty());
        for line in every_line(&report) {
            assert_eq!(leak(line), None, "{line}");
            assert!(!line.contains("cannon at"), "{line}");
            assert!(!line.contains(" at "), "{line}");
            assert!(!line.contains("RA"), "{line}");
        }
    }

    /// **16b cut V.3**: staff read the whole — every event but the position
    /// summaries, units by callsign and number, contacts by their digits, and
    /// the draws.
    #[test]
    fn v3_staff_read_every_field_and_the_draws() {
        let mut tape = the_tape();
        tape.insert(
            0,
            LedgerEvent {
                seq: 0,
                t_ms: 0,
                kind: EventKind::PositionSummary {
                    units: vec![(ME, 100, 200, 0, 0)],
                },
                draws: Vec::new(),
                hash: 1,
            },
        );
        let report = fold(&tape, &roster(), Reader::Staff, RULES);
        assert_eq!(report.lines.len(), the_tape().len(), "no position summary");
        let text = report.lines.join("\n");
        assert!(text.contains("RA(#101)"), "{text}");
        assert!(text.contains("BA(#201)"), "{text}");
        assert!(text.contains("0123456789abcdef"), "{text}");
        assert!(text.contains("[hit:2=5 facet:2=9]"), "{text}");
        assert!(text.contains("range_cm=43000"), "{text}");
        assert!(text.contains("damage=120"), "{text}");
        assert!(!text.contains("hash"), "the hash is the record's: {text}");
        assert!(!text.contains("position_summary"), "{text}");
        assert_eq!(
            report.lines.first().map(String::as_str),
            Some("  0:04.000  power_online unit=RA(#101)")
        );
        // The same line for the same event, every time.
        assert_eq!(report, fold(&tape, &roster(), Reader::Staff, RULES));
        // A unit off the roster is its number alone.
        let bare = fold(&tape, &Roster::new(), Reader::Staff, RULES);
        assert_eq!(
            bare.lines.first().map(String::as_str),
            Some("  0:04.000  power_online unit=#101")
        );
    }

    /// **17b cut M1b.1** (D-UM-14): a slot's shot is journalled `by: cond` with
    /// its slot, and the staff line spells the two after `rounds=` — while a
    /// seat's or a machine's bare `fired` line reads byte for byte as it
    /// always did.
    #[test]
    fn m1b1_the_staff_line_spells_a_slot_shots_attribution_and_a_bare_one_as_before() {
        let mut tape = Tape::new();
        tape.at(20_000, fired(ME, "cannon", 1, THEM, 1)).at(
            24_000,
            EventKind::Fired {
                unit: ME,
                weapon: SmolStr::new("cannon"),
                shot_seq: 2,
                target: THEM,
                range_cm: 43_000,
                rounds: 1,
                by: Some(SmolStr::new("cond")),
                slot: Some(1),
            },
        );
        let report = fold(&tape.0, &roster(), Reader::Staff, RULES);
        assert_eq!(
            report.lines,
            vec![
                "  0:20.000  fired unit=RA(#101) weapon=cannon shot=1 target=BA(#201) \
                 range_cm=43000 rounds=1"
                    .to_owned(),
                "  0:24.000  fired unit=RA(#101) weapon=cannon shot=2 target=BA(#201) \
                 range_cm=43000 rounds=1 by=cond slot=1"
                    .to_owned(),
            ]
        );
    }

    /// **16b cut I.3** (16c N10): the staff fold spells the two levers' three
    /// lines, and the pilot's fold prints none of them — not even a place of the
    /// pilot's own tank, which is journalled always and announced never.
    #[test]
    fn i3_the_staff_read_the_levers_and_the_pilot_reads_nothing_of_them() {
        let mut tape = Tape::new();
        tape.at(
            1_000,
            EventKind::TempoFloor {
                period_ms: Some(5_000),
            },
        )
        .at(
            2_000,
            EventKind::Placed {
                unit: ME,
                x: 120,
                y: 97,
                from_x: 118,
                from_y: 97,
            },
        )
        .at(
            3_000,
            EventKind::Placed {
                unit: ObjId(301),
                x: 4,
                y: 5,
                from_x: u16::MAX,
                from_y: u16::MAX,
            },
        )
        .at(
            4_000,
            EventKind::PlaceRefused {
                unit: ObjId(301),
                x: 120,
                y: 97,
                reason: SmolStr::new_static("in_flight"),
            },
        )
        .at(5_000, EventKind::TempoFloor { period_ms: None });
        let staff = fold(&tape.0, &roster(), Reader::Staff, RULES);
        assert_eq!(
            staff.lines,
            vec![
                "  0:01.000  tempo_floor period_ms=5000",
                "  0:02.000  placed unit=RA(#101) (120, 97) from (118, 97)",
                "  0:03.000  placed unit=#301 (4, 5) from off the window",
                "  0:04.000  place_refused unit=#301 (120, 97): in_flight",
                "  0:05.000  tempo_floor lifted",
            ]
        );
        let pilot = fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        for line in every_line(&pilot) {
            for word in ["tempo_floor", "placed", "place", "in_flight", "floor"] {
                assert!(!line.contains(word), "{word} reached the pilot: {line}");
            }
        }
        assert!(pilot.lines.is_empty(), "{:?}", pilot.lines);
    }

    fn stock_profile_key(
        card: &crate::arena::card::StockCard,
    ) -> crate::arena::runtime_card::ProfileKey {
        use crate::arena::runtime_card::{CardCatalog, ProfileKey, stock_definition};
        let definition = stock_definition(
            &[(SmolStr::new_static("fixture"), card.clone())],
            crate::arena::core::CoreClass::III,
        )
        .expect("the report fixture is a complete stock card");
        let mut catalog = CardCatalog::default();
        let definition = catalog
            .admit(definition)
            .expect("the report fixture admits");
        ProfileKey {
            definition,
            index: 0,
        }
    }

    /// **17b cut M1b.5** (doc 17 §11): the staff fold spells the card lever's
    /// two lines — the card's name and the sliders it moved, or the refusal's
    /// reason — and the pilot's fold prints neither: the change is another
    /// hull's, and the refusal, of the pilot's own tank, is staff's alone.
    ///
    /// **17b cut M5.6** (its review, R3): was "the pilot's fold prints
    /// neither, even of the pilot's own tank" — true of the timeline still,
    /// but a `mind_card` on the pilot's OWN hull now reaches that pilot's
    /// `Your mind` block by the card's name (D14;
    /// `m5_a_staff_edit_reads_by_its_card_name`). Here the fold is also seeded
    /// as `match_::start` seeds it, a slot that chose: its block names neither
    /// line — no switch, no slider, no refusal.
    #[test]
    fn m1b5_the_staff_read_the_card_lines_and_the_pilot_reads_nothing_of_them() {
        let mut card =
            crate::arena::card::preset("sniper", &crate::config::ReflexConfig::default())
                .expect("sniper");
        card.min_odds_permille = 700;
        let mut tape = Tape::new();
        tape.at(
            1_000,
            EventKind::MindCard {
                unit: ObjId(301),
                name: SmolStr::new("Sniper, edited (min_odds 70)"),
                changed: vec![SmolStr::new_static("min_odds")],
                card: stock_profile_key(&card),
                stock: Some(card),
            },
        )
        .at(
            2_000,
            EventKind::MindCardRefused {
                unit: ME,
                reason: SmolStr::new_static("not_mind"),
            },
        );
        let staff = fold(&tape.0, &roster(), Reader::Staff, RULES);
        assert_eq!(
            staff.lines,
            vec![
                "  0:01.000  mind_card unit=#301 name=\"Sniper, edited (min_odds 70)\" \
                 changed=min_odds",
                "  0:02.000  mind_card_refused unit=RA(#101): not_mind",
            ]
        );
        let pilot = fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert!(pilot.lines.is_empty(), "{:?}", pilot.lines);
        assert_eq!(pilot.mind, None);
        let seeded = mind_report(&tape.0, Some(own_mind(true, &[])), &[]);
        assert!(seeded.lines.is_empty(), "{:?}", seeded.lines);
        assert_eq!(
            block_of(&seeded),
            vec![
                "Your mind: Standard Doctrine (Line).",
                "  It never flew: you flew the tank the whole match.",
            ]
        );
    }

    /// **17b cut M3.3** (doc 17 §13, §19): the staff fold spells the four mind
    /// kinds — a principal by the unit its character flies (`RA(#101)`), a
    /// persona by its name and nobody as `-`, the drop's fault and budget
    /// only where the line carries them — and the pilot's timeline prints
    /// none of them, even of the pilot's own tank.
    ///
    /// **17b cut M5.6** (the owner's OQ-9 (ii); the design note's "What
    /// moves"): inverted for the pilot's own hull. Was `the pilot reads nothing
    /// of them` (`pilot.lines.is_empty()` on an unseeded fold, and the name
    /// `m3_the_staff_read_the_four_mind_lines_and_the_pilot_reads_nothing_of_them`)
    /// at 17b cut M3.3; the pilot's fold, seeded as `match_::start` seeds it,
    /// now reads its OWN hull's drop to the ROM in its `Your mind` block, by the
    /// cause word alone (D17) — never the last fault's word, the op budget,
    /// the seal, the fault or the sortie, and nothing of another hull's drop.
    /// The timeline still carries none of the four.
    #[test]
    fn m3_the_staff_read_the_four_mind_lines_and_the_pilot_reads_its_own_drop_alone() {
        use crate::arena::core::CoreClass;
        use crate::arena::mind::SortieId;
        use crate::deliberation::order::CharacterRef;
        let sealed = |unit: ObjId, principal: Option<CharacterRef>| EventKind::MindSealed {
            unit,
            sortie: SortieId::in_match(12, unit),
            mind: SmolStr::new_static("Standard Doctrine"),
            source: SmolStr::new_static("8b3ec78c9d957865"),
            card_digest: SmolStr::new("a".repeat(64)),
            class: CoreClass::I,
            governed: true,
            licence: SmolStr::new_static("doctrine"),
            principal,
        };
        let mut tape = Tape::new();
        tape.at(
            0,
            sealed(ME, Some(CharacterRef::Character { id: ObjId(5) })),
        )
        .at(
            0,
            sealed(
                ObjId(301),
                Some(CharacterRef::Persona {
                    squad: crate::deliberation::SquadId {
                        team: Team::Red,
                        index: 1,
                    },
                }),
            ),
        )
        .at(0, sealed(THEM, None))
        .at(
            1_000,
            EventKind::MindFault {
                unit: ME,
                think_seq: 3,
                cause: SmolStr::new_static("ops"),
            },
        )
        .at(
            2_000,
            EventKind::MindDropped {
                unit: ME,
                cause: SmolStr::new_static("strikes"),
                fault: Some(SmolStr::new_static("ops")),
                ops: Some(1_500),
            },
        )
        .at(
            3_000,
            EventKind::MindDropped {
                unit: THEM,
                cause: SmolStr::new_static("core"),
                fault: None,
                ops: None,
            },
        )
        .at(
            4_000,
            EventKind::SortieClosed {
                unit: ME,
                sortie: SortieId::in_match(12, ME),
                word: SmolStr::new_static("match_end"),
            },
        );
        let staff = fold(&tape.0, &roster(), Reader::Staff, RULES);
        assert_eq!(
            staff.lines,
            vec![
                "  0:00.000  mind_sealed unit=RA(#101) sortie=51539607653 mind=\"Standard \
                 Doctrine\" class=I governed=true licence=doctrine principal=RA(#101)",
                "  0:00.000  mind_sealed unit=#301 sortie=51539607853 mind=\"Standard Doctrine\" \
                 class=I governed=true licence=doctrine principal=\"red lance leader 1\"",
                "  0:00.000  mind_sealed unit=BA(#201) sortie=51539607753 mind=\"Standard \
                 Doctrine\" class=I governed=true licence=doctrine principal=-",
                "  0:01.000  mind_fault unit=RA(#101) think_seq=3 cause=ops",
                "  0:02.000  mind_dropped unit=RA(#101) cause=strikes fault=ops ops=1500",
                "  0:03.000  mind_dropped unit=BA(#201) cause=core",
                "  0:04.000  sortie_closed unit=RA(#101) sortie=51539607653 word=match_end",
            ]
        );
        for line in &staff.lines {
            assert!(!line.contains("#5)") && !line.contains("=#5"), "{line}");
        }
        assert_eq!(staff.mind, None, "the staff page carries no block");
        let pilot = fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert!(pilot.lines.is_empty(), "{:?}", pilot.lines);
        // 17b cut M5.6: seeded as `match_::start` seeds it, the pilot reads
        // their own hull's drop in the block, and nothing else of the four.
        let mut own =
            Fold::new(roster(), Reader::Pilot { unit: ME }, RULES).with_own_mind(OwnMind {
                chose: true,
                mind: SmolStr::new_static("Standard Doctrine"),
                profiles: Vec::new(),
                substituted: false,
            });
        for event in &tape.0 {
            own.absorb(event);
        }
        let pilot = own.finish();
        assert!(pilot.lines.is_empty(), "{:?}", pilot.lines);
        assert!(pilot.tail.is_empty(), "{:?}", pilot.tail);
        let block = pilot.mind.map(|block| block.lines).unwrap_or_default();
        assert_eq!(
            block,
            vec![
                "Your mind: Standard Doctrine (Line).",
                "  It never flew: you flew the tank the whole match.",
                "  It dropped to the ROM at 0:02: three faults running.",
            ]
        );
        for line in &block {
            for staff_only in ["ops", "1500", "1,500", "sortie", "principal", "core", "#"] {
                assert!(!line.contains(staff_only), "{staff_only:?} in {line}");
            }
        }
    }

    // -- 17b cut M5.6: the `Your mind` block -------------------------------

    /// The pilot's own mind as `match_::start` seeds it: Standard Doctrine at
    /// `profiles` (empty is Line alone).
    fn own_mind(chose: bool, profiles: &[&str]) -> OwnMind {
        OwnMind {
            chose,
            mind: SmolStr::new_static("Standard Doctrine"),
            profiles: profiles.iter().map(|name| SmolStr::new(*name)).collect(),
            substituted: false,
        }
    }

    /// One moment the world tells the fold of: a release (`Some(by)`) or a
    /// reclaim (`None`), at an arena millisecond.
    type Moment = (i64, Option<HandOver>);

    /// **A pilot fold over `tape` as the world feeds it** (17b cut M5.6):
    /// seeded with `own` when given, and each of `moments` told in order before
    /// the first event at or after it — the world opens a span as it marks the
    /// slot, ahead of the chunks that carry the tape after it — the rest after
    /// the last event.
    fn mind_report(tape: &[LedgerEvent], own: Option<OwnMind>, moments: &[Moment]) -> Report {
        let mut fold = Fold::new(roster(), Reader::Pilot { unit: ME }, RULES);
        if let Some(own) = own {
            fold = fold.with_own_mind(own);
        }
        let tell = |fold: &mut Fold, (at_ms, by): Moment| match by {
            Some(by) => fold.released(at_ms, by),
            None => fold.reclaimed(at_ms),
        };
        let mut moments = moments.iter().copied().peekable();
        for event in tape {
            while let Some(moment) = moments.next_if(|(at_ms, _)| *at_ms <= event.t_ms) {
                tell(&mut fold, moment);
            }
            fold.absorb(event);
        }
        for moment in moments {
            tell(&mut fold, moment);
        }
        fold.finish()
    }

    /// The block's lines, or none.
    fn block_of(report: &Report) -> Vec<String> {
        report
            .mind
            .as_ref()
            .map(|block| block.lines.clone())
            .unwrap_or_default()
    }

    /// **17b cut M5.6** (the owner's OQ-9 (ii)): the block is printed only for
    /// a slot that chose a mind or a hull the mind flew. A chosen `SIM MIND
    /// Standard` with no `PROFILE`, never released, prints it — it seals byte
    /// for byte as an unchosen slot does, so only the check-out's seed can say
    /// so; an unchosen hull never released prints none, and its report and
    /// its document are what they were before this cut; an unchosen hull the
    /// mind flew prints it; and a fold with no check-out facts names nothing.
    /// In every case the pilot's own lines are the brief tape's pinned ones.
    #[test]
    fn m5_the_block_is_printed_only_for_a_chosen_or_released_hull() {
        let tape = the_tape();
        let base = fold(&tape, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(base.mind, None);

        let chosen = mind_report(&tape, Some(own_mind(true, &[])), &[]);
        assert_eq!(
            block_of(&chosen),
            vec![
                "Your mind: Standard Doctrine (Line).",
                "  It never flew: you flew the tank the whole match.",
            ]
        );

        let unchosen = mind_report(&tape, Some(own_mind(false, &[])), &[]);
        assert_eq!(
            unchosen, base,
            "an unchosen hull never released prints none"
        );
        let document = report_document(MatchId(12), ME, Some(ObjId(5)), "RA", &unchosen);
        assert!(document.get("mind").is_none(), "{document}");
        assert_eq!(
            document,
            report_document(MatchId(12), ME, Some(ObjId(5)), "RA", &base)
        );

        // `EXIT` at 0:42, before its cannon's second shot: the mind flew it
        // from there, and the shots from there are the block's — until 3:12,
        // where the tape destroys the hull (the mind flies no wreck: 17b cut
        // M5.6's review, R1/R23; the span ran `to the end.` before it).
        let released = mind_report(
            &tape,
            Some(own_mind(false, &[])),
            &[(42_000, Some(HandOver::ClimbedOut))],
        );
        assert_eq!(
            block_of(&released),
            vec![
                "Your mind: Standard Doctrine (Line).",
                "  It flew your tank from 0:42, when you climbed out, to 3:12.",
                "  Its shots: cannon 1 and machine_gun 1, 2 of them hits; it destroyed 1 unit.",
            ]
        );
        let document = report_document(MatchId(12), ME, Some(ObjId(5)), "RA", &released);
        assert_eq!(
            document["mind"]["lines"][1],
            serde_json::json!("  It flew your tank from 0:42, when you climbed out, to 3:12.")
        );
        assert_eq!(
            report_from_document(&document).and_then(|report| report.mind),
            released.mind,
            "the block reads back off its document"
        );

        let unseeded = mind_report(&tape, None, &[(42_000, Some(HandOver::ClimbedOut))]);
        assert_eq!(
            unseeded, base,
            "a fold with no check-out facts names nothing"
        );

        // The pilot's own lines are untouched by any of it.
        for report in [&chosen, &unchosen, &released] {
            assert_eq!(
                Report {
                    mind: None,
                    ..(*report).clone()
                },
                base
            );
        }
    }

    /// **17b cut M5.6's review** (R1/R23; the owner's OQ-9 (ii): the block says
    /// truly whether and from when the mind flew): the mind flies no wreck. A
    /// pilot told `EXIT leaves the pod.` after their tank is destroyed, or
    /// whose line drops then, is marked out of the cockpit like any other, and
    /// the world opens a span it cannot tell apart — the fold, which reads the
    /// hull's own `unit_destroyed` off the tape (after the world's call: the
    /// tape lags it), drops it when the block is built: an unchosen pilot
    /// reads no block, a chosen one reads that it never flew. A span the
    /// destruction finds running ends there, whichever way it began and
    /// whether or not it was taken back after; one before it is untouched.
    #[test]
    fn m5_a_span_opened_on_a_wreck_is_no_span_and_one_running_ends_there() {
        let mut tape = Tape::new();
        tape.at(10_000, fired(ME, "cannon", 1, THEM, 1))
            .at(11_000, hit(1, ME, THEM, "hull", "front", "hull", 180, 200))
            .at(
                30_000,
                EventKind::UnitDestroyed {
                    unit: ME,
                    by: Some(THEM),
                },
            );
        let after = [(40_000, Some(HandOver::ClimbedOut))];
        let unchosen = mind_report(&tape.0, Some(own_mind(false, &[])), &after);
        assert_eq!(
            unchosen.mind, None,
            "an unchosen wreck's pilot reads no block"
        );
        assert_eq!(
            unchosen,
            fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES)
        );
        let chosen = mind_report(&tape.0, Some(own_mind(true, &["Sniper", "Line"])), &after);
        assert_eq!(
            block_of(&chosen),
            vec![
                "Your mind: Standard Doctrine (Sniper, Line).",
                "  It never flew: you flew the tank the whole match.",
            ]
        );
        // A drop after the wreck, and one at its very millisecond, are none.
        for at_ms in [30_000, 45_000] {
            let dropped = mind_report(
                &tape.0,
                Some(own_mind(false, &[])),
                &[(at_ms, Some(HandOver::Dropped))],
            );
            assert_eq!(dropped.mind, None, "a drop at {at_ms} ms");
        }
        // Running when the hull is destroyed: it ends there.
        for (moments, said) in [
            (
                vec![(5_000, Some(HandOver::ClimbedOut))],
                "  It flew your tank from 0:05, when you climbed out, to 0:30.",
            ),
            (
                vec![(5_000, Some(HandOver::Dropped))],
                "  It flew your tank from 0:05 to 0:30, while your line was down.",
            ),
            (
                vec![(5_000, Some(HandOver::Dropped)), (50_000, None)],
                "  It flew your tank from 0:05 to 0:30, while your line was down.",
            ),
            (
                vec![
                    (5_000, Some(HandOver::Dropped)),
                    (8_000, None),
                    (20_000, Some(HandOver::Dropped)),
                    (35_000, None),
                    (40_000, Some(HandOver::ClimbedOut)),
                ],
                "  It flew your tank from 0:05 to 0:08 and from 0:20 to 0:30, while your line \
                 was down.",
            ),
        ] {
            let report = mind_report(&tape.0, Some(own_mind(false, &[])), &moments);
            assert_eq!(block_of(&report).get(1).map(String::as_str), Some(said));
        }
        // The shot the mind fired before the wreck is still its own.
        let flown = mind_report(
            &tape.0,
            Some(own_mind(false, &[])),
            &[(5_000, Some(HandOver::ClimbedOut))],
        );
        assert_eq!(
            block_of(&flown).get(2).map(String::as_str),
            Some("  Its shots: cannon 1, a hit; it destroyed nothing.")
        );
    }

    /// **17b cut M5.6** (D14; doc 17 §11): the card changes on the pilot's own
    /// hull are read by the card's own name — a seat's `MIND PROFILE` by its
    /// preset, a staff `@mind/live` as `StockCard::name_of` names the edited
    /// card (`Sniper, edited (min_odds 70)`) — one by its moment, more as a
    /// count and the last on its own line; another hull's change is not this
    /// pilot's to read, and no change is ever a timeline line.
    #[test]
    fn m5_a_staff_edit_reads_by_its_card_name() {
        let cfg = crate::config::ReflexConfig::default();
        let mut edited = crate::arena::card::preset("sniper", &cfg).expect("sniper");
        edited.set("min_odds", "70").expect("a lawful edit");
        let name = edited.name_of(&cfg);
        assert_eq!(name, "Sniper, edited (min_odds 70)");
        let line = crate::arena::card::preset("line", &cfg).expect("line");
        let card =
            |unit: ObjId, name: &str, card: &crate::arena::card::StockCard| EventKind::MindCard {
                unit,
                name: SmolStr::new(name),
                changed: Vec::new(),
                card: stock_profile_key(card),
                stock: Some(card.clone()),
            };
        let sealed = own_mind(true, &["Sniper", "Line"]);

        // A seat's switch, the note's own line.
        let mut tape = Tape::new();
        tape.at(65_000, card(ME, "Line", &line))
            .at(66_000, card(THEM, "Hunter", &line));
        let switched = mind_report(&tape.0, Some(sealed.clone()), &[]);
        assert_eq!(
            block_of(&switched).first().map(String::as_str),
            Some("Your mind: Standard Doctrine (Sniper, Line); switched to Line at 1:05.")
        );
        assert!(switched.lines.is_empty(), "{:?}", switched.lines);

        // A staff edit while the mind flies the hull, by the card's name.
        let mut tape = Tape::new();
        tape.at(65_000, card(ME, &name, &edited))
            .at(66_000, card(THEM, "Hunter", &line));
        let staff_edit = mind_report(
            &tape.0,
            Some(sealed.clone()),
            &[(42_000, Some(HandOver::ClimbedOut))],
        );
        assert_eq!(
            block_of(&staff_edit),
            vec![
                "Your mind: Standard Doctrine (Sniper, Line); switched to Sniper, edited \
                 (min_odds 70) at 1:05.",
                "  It flew your tank from 0:42, when you climbed out, to the end.",
                "  It fired nothing.",
            ]
        );
        assert!(staff_edit.lines.is_empty(), "{:?}", staff_edit.lines);
        for line in block_of(&staff_edit) {
            assert!(!line.contains("Hunter"), "another hull's card: {line}");
        }

        // Two changes: a count and the last.
        let mut tape = Tape::new();
        tape.at(65_000, card(ME, &name, &edited))
            .at(90_000, card(ME, "Line", &line));
        let twice = mind_report(&tape.0, Some(sealed), &[]);
        assert_eq!(
            block_of(&twice),
            vec![
                "Your mind: Standard Doctrine (Sniper, Line); switched 2 times.",
                "  Its last switch was to Line at 1:30.",
                "  It never flew: you flew the tank the whole match.",
            ]
        );
    }

    /// **17b cut M5.6** (the design note's M5.6 item 1, the adversarial
    /// review's B20): the span line is bounded whatever the count — five
    /// drop-and-reclaim spans are one line, `5 times: first … last …`, with no
    /// cause clause, under 120 columns.
    #[test]
    fn m5_the_span_line_is_bounded_at_five_spans() {
        let dropped = Some(HandOver::Dropped);
        let moments: Vec<Moment> = vec![
            (42_000, dropped),
            (70_000, None),
            (130_000, dropped),
            (190_000, None),
            (300_000, dropped),
            (360_000, None),
            (500_000, dropped),
            (560_000, None),
            (725_000, dropped),
        ];
        let report = mind_report(&[], Some(own_mind(false, &[])), &moments);
        let block = block_of(&report);
        assert_eq!(
            block,
            vec![
                "Your mind: Standard Doctrine (Line).",
                "  It flew your tank 5 times: first from 0:42 to 1:10, last from 12:05 to the end.",
                "  It fired nothing.",
            ]
        );
        assert_eq!(
            block
                .iter()
                .filter(|line| line.starts_with("  It flew"))
                .count(),
            1
        );
        for line in &block {
            assert!(line.chars().count() < 120, "{line}");
            assert_eq!(leak(line), None, "{line}");
        }
    }

    /// **17b cut M5.6**: one span and two read as the design note writes them
    /// — the cause clause where every span had one cause (an `EXIT`'s span
    /// runs to the end, so it is always the last), none where the causes
    /// differ — and a chosen hull the mind never flew says so.
    #[test]
    fn m5_the_span_line_reads_one_and_two_spans_as_the_note_writes_them() {
        let span_line_of = |moments: &[Moment]| {
            block_of(&mind_report(&[], Some(own_mind(true, &[])), moments))
                .get(1)
                .cloned()
                .unwrap_or_default()
        };
        let (out, drop) = (Some(HandOver::ClimbedOut), Some(HandOver::Dropped));
        for (moments, said) in [
            (
                vec![],
                "  It never flew: you flew the tank the whole match.",
            ),
            (
                vec![(42_000, out)],
                "  It flew your tank from 0:42, when you climbed out, to the end.",
            ),
            (
                vec![(42_000, drop), (70_000, None)],
                "  It flew your tank from 0:42 to 1:10, while your line was down.",
            ),
            (
                vec![(42_000, drop)],
                "  It flew your tank from 0:42 to the end, while your line was down.",
            ),
            (
                vec![(42_000, drop), (70_000, None), (125_000, drop)],
                "  It flew your tank from 0:42 to 1:10 and from 2:05 to the end, while your \
                 line was down.",
            ),
            (
                vec![(42_000, drop), (70_000, None), (125_000, out)],
                "  It flew your tank from 0:42 to 1:10 and from 2:05 to the end.",
            ),
            (
                vec![
                    (42_000, drop),
                    (70_000, None),
                    (125_000, drop),
                    (130_000, None),
                    (245_000, drop),
                ],
                "  It flew your tank 3 times: first from 0:42 to 1:10, last from 4:05 to the end.",
            ),
        ] {
            assert_eq!(span_line_of(&moments), said);
        }
        // A second release while a span runs opens nothing, and a reclaim
        // with none open closes nothing.
        assert_eq!(
            span_line_of(&[
                (30_000, None),
                (42_000, drop),
                (50_000, out),
                (70_000, None)
            ]),
            "  It flew your tank from 0:42 to 1:10, while your line was down."
        );
    }

    /// **17b cut M5.6**: the drop is read by its cause word alone (D17) — the
    /// line's `fault` and `ops` are staff's — once, from the first drop the
    /// hull's sortie journals; another hull's drop is not this pilot's.
    #[test]
    fn m5_the_drop_reads_its_cause_word_alone() {
        let dropped = |unit: ObjId, cause: &'static str| EventKind::MindDropped {
            unit,
            cause: SmolStr::new_static(cause),
            fault: Some(SmolStr::new_static("ops")),
            ops: Some(1_500),
        };
        for (cause, said) in [
            (
                "strikes",
                "  It dropped to the ROM at 1:30: three faults running.",
            ),
            (
                "core",
                "  It dropped to the ROM at 1:30: its core was destroyed.",
            ),
            (
                "abi",
                "  It dropped to the ROM at 1:30: its core could not run its mind on this engine.",
            ),
            ("unheard_of", "  It dropped to the ROM at 1:30."),
        ] {
            let mut tape = Tape::new();
            tape.at(90_000, dropped(ME, cause))
                .at(95_000, dropped(ME, "core"))
                .at(96_000, dropped(THEM, "core"));
            let report = mind_report(&tape.0, Some(own_mind(true, &[])), &[]);
            assert_eq!(block_of(&report).last().map(String::as_str), Some(said));
            assert_eq!(block_of(&report).len(), 3, "{:?}", block_of(&report));
        }
    }

    /// **17b cut M5.6** (conventions §3: every line a pilot reads): every line
    /// the block can print, at its widest — the widest preset pair, the widest
    /// name `StockCard::name_of` gives an edited card over every preset and
    /// slider, three-digit counts, `59:59`, the three longest weapon ids at
    /// four-digit counts — is under 120 columns and carries nothing the
    /// report's no-leak grep refuses.
    #[test]
    fn m5_every_your_mind_line_is_under_120_columns_and_leaks_nothing() {
        let cfg = crate::config::ReflexConfig::default();
        let values = [
            "359",
            "300",
            "180",
            "120",
            "100",
            "95",
            "79.5",
            "17.5",
            "best_odds",
            "balanced",
            "advance",
            "nearest",
            "screen",
            "flank",
            "auto",
            "ccw",
            "off",
            "on",
            "50",
            "10",
        ];
        let mut widest = String::new();
        for preset in crate::arena::card::PRESETS {
            let base = crate::arena::card::preset(preset, &cfg).expect("a preset");
            for slider in [
                "stance",
                "band_pct",
                "close_m",
                "orbit_sense",
                "turn_into_hits",
                "turn_s",
                "screen_bearing",
                "screen_arc",
                "opening",
                "min_odds",
                "aspect",
                "reserve_pct",
                "reserve_odds",
                "prefer",
                "switch_m",
                "break_off_pct",
                "fight_home_pct",
                "rally_s",
                "search_s",
                "hunt_after_s",
                "stop_unseen",
            ] {
                for value in values {
                    let mut card = base.clone();
                    if card.set(slider, value).is_ok() {
                        let name = card.name_of(&cfg);
                        if name.chars().count() > widest.chars().count() {
                            widest = name;
                        }
                    }
                }
            }
        }
        assert!(widest.contains(", edited ("), "{widest}");
        let late = 3_599_000_i64;
        let spans = |count: usize, by: HandOver| -> Vec<Span> {
            (0..count)
                .map(|n| Span {
                    from_ms: late - 1_000,
                    to_ms: (n + 1 < count).then_some(late),
                    by,
                })
                .collect()
        };
        let mut lines: Vec<String> = Vec::new();
        for (switches, profiles) in [
            (1, vec!["Skirmisher", "Rearguard"]),
            (999, vec!["Skirmisher", "Rearguard"]),
            (1, vec![]),
        ] {
            let track = MindTrack {
                own: Some(OwnMind {
                    chose: true,
                    mind: SmolStr::new_static("Standard Doctrine"),
                    profiles: profiles.into_iter().map(SmolStr::new_static).collect(),
                    substituted: true,
                }),
                spans: spans(999, HandOver::Dropped),
                shots: [
                    ("machine_gun", 9_999),
                    ("autocannon", 9_999),
                    ("missiles", 9_999),
                ]
                .into_iter()
                .map(|(weapon, n)| (SmolStr::new_static(weapon), Tally { shots: n, hits: n }))
                .collect(),
                kills: 99,
                switches,
                last_switch: Some(Said {
                    t_ms: late,
                    word: SmolStr::new(&widest),
                }),
                dropped: Some(Said {
                    t_ms: late,
                    word: SmolStr::new_static("abi"),
                }),
                ..MindTrack::default()
            };
            lines.extend(track.block().map(|block| block.lines).unwrap_or_default());
        }
        for by in [HandOver::Dropped, HandOver::ClimbedOut] {
            for count in [1, 2] {
                lines.push(span_line(&spans(count, by)));
            }
        }
        for by in [HandOver::Dropped, HandOver::ClimbedOut] {
            let closed: Vec<Span> = spans(1, by)
                .into_iter()
                .map(|span| Span {
                    to_ms: Some(late),
                    ..span
                })
                .collect();
            lines.push(span_line(&closed));
        }
        for cause in ["strikes", "core", "abi"] {
            lines.push(drop_line(&Said {
                t_ms: late,
                word: SmolStr::new_static(cause),
            }));
        }
        assert!(lines.len() >= 20, "{lines:#?}");
        for line in &lines {
            assert!(
                line.chars().count() < 120,
                "{} columns: {line}",
                line.chars().count()
            );
            assert_eq!(leak(line), None, "{line}");
        }
    }

    /// **17b cut M5.6's review** (R19): `help minds` quotes the block as it
    /// is built — its example head, span and shots lines (one switch to Line
    /// at 1:05, an `EXIT` at 4:42, four cannon shots of which two hit), the
    /// bounded five-span line and a drop with its cause — each the fold's own
    /// output, so the page cannot drift from the block unseen.
    #[test]
    fn m5_help_minds_quotes_the_block_as_it_is_built() {
        let help = include_str!("../../../../docs/help/minds.md");
        let quoted = |said: &str| {
            help.lines()
                .any(|line| line.trim_start() == said.trim_start())
        };
        let track = MindTrack {
            own: Some(own_mind(true, &["Sniper", "Line"])),
            spans: vec![Span {
                from_ms: 282_000,
                to_ms: None,
                by: HandOver::ClimbedOut,
            }],
            shots: [(SmolStr::new_static("cannon"), Tally { shots: 4, hits: 2 })]
                .into_iter()
                .collect(),
            switches: 1,
            last_switch: Some(Said {
                t_ms: 65_000,
                word: SmolStr::new_static("Line"),
            }),
            ..MindTrack::default()
        };
        let block = track.block().map(|block| block.lines).unwrap_or_default();
        assert_eq!(
            block,
            vec![
                "Your mind: Standard Doctrine (Sniper, Line); switched to Line at 1:05.",
                "  It flew your tank from 4:42, when you climbed out, to the end.",
                "  Its shots: cannon 4, 2 of them hits; it destroyed nothing.",
            ]
        );
        let five: Vec<Span> = [
            (42_000, Some(70_000)),
            (130_000, Some(190_000)),
            (300_000, Some(360_000)),
            (500_000, Some(560_000)),
            (725_000, None),
        ]
        .into_iter()
        .map(|(from_ms, to_ms)| Span {
            from_ms,
            to_ms,
            by: HandOver::Dropped,
        })
        .collect();
        let dropped = drop_line(&Said {
            t_ms: 90_000,
            word: SmolStr::new_static("core"),
        });
        for line in block.iter().cloned().chain([span_line(&five), dropped]) {
            assert!(
                quoted(&line),
                "help minds does not quote the block's {line:?}"
            );
        }
    }

    /// **16b cut V.3**: a seq that is not the last plus one is a gap, and so is
    /// a pilot's tape that does not start at 1, and a chunk that was lost.
    #[test]
    fn v3_a_seq_gap_or_a_lost_chunk_sets_the_gap() {
        let mut tape = the_tape();
        tape.remove(4);
        let report = fold(&tape, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert!(report.gap);
        let whole = the_tape();
        let report = fold(
            whole.get(1..).unwrap_or_default(),
            &roster(),
            Reader::Pilot { unit: ME },
            RULES,
        );
        assert!(report.gap, "a pilot's tape starts at 1");
        // Staff pages start anywhere.
        let report = fold(
            whole.get(3..).unwrap_or_default(),
            &roster(),
            Reader::Staff,
            RULES,
        );
        assert!(!report.gap);
        let report = fold(&whole, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert!(!report.gap);
        let mut lost = Fold::new(roster(), Reader::Pilot { unit: ME }, RULES);
        lost.lost_chunk();
        assert!(lost.finish().gap);
    }

    /// **16b cut V.3**: 201 completed shots keep 200 lines and count one, and
    /// the tail is intact.
    #[test]
    fn v3_the_timeline_is_bounded_and_the_tail_is_not() {
        let mut tape = Tape::new();
        for n in 1..=201_u64 {
            let at = i64::try_from(n).unwrap_or(0) * 1_000;
            tape.at(at, fired(ME, "cannon", n, THEM, 1)).at(
                at + 500,
                EventKind::Miss {
                    shot_seq: n,
                    reason: SmolStr::new(damage::MOVED),
                },
            );
        }
        tape.at(300_000, EventKind::UnitDisabled { unit: ME })
            .at(301_000, EventKind::Aborted);
        let report = fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(report.lines.len(), REPORT_MAX_LINES);
        assert_eq!(report.truncated, 1);
        assert_eq!(
            report.tail,
            vec![
                "  5:00  your tank disabled.",
                "  5:01  the match was aborted."
            ]
        );
        assert_eq!(report.rounds.get("cannon"), Some(&201));
        // Held shots count too: a shot with no outcome at the end is a line.
        let mut open = Tape::new();
        open.at(1_000, fired(ME, "cannon", 1, THEM, 1))
            .at(2_000, EventKind::Void);
        let report = fold(&open.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            report.lines,
            vec!["  0:01  cannon at a contact: no outcome on the tape."]
        );
        assert_eq!(report.tail, vec!["  0:02  the match was voided."]);
    }

    /// **16b cut V.3**: a shot line sorts by its own `fired`, though it
    /// completes after later events were written.
    ///
    /// **16b cut AE.2**: the pilot never held the target, so the miss reads
    /// `miss.` with no reason — it read `miss: the line was blocked.` before,
    /// a fact about the ground round a target the platform had not found.
    #[test]
    fn v3_a_shot_sorts_by_its_own_stamp() {
        let mut tape = Tape::new();
        tape.at(1_000, fired(ME, "cannon", 1, THEM, 1))
            .at(2_000, EventKind::PowerOnline { unit: ME })
            .at(
                3_000,
                EventKind::Miss {
                    shot_seq: 1,
                    reason: SmolStr::new(damage::BLOCKED),
                },
            );
        let report = fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            report.lines,
            vec![
                "  0:01  cannon at a contact: miss.",
                "  0:02  power online.",
            ]
        );
    }

    /// **16b cut AE.2**: the miss's reason is the pilot's only while the
    /// platform holds the target — named, blip or told apart — and gone the
    /// moment it does not: the same shot at a target held as a blip, and at one
    /// whose contact was lost before the round landed.
    #[test]
    fn ae2_a_miss_carries_its_reason_only_on_a_target_the_platform_holds() {
        let miss = |seq: u64| EventKind::Miss {
            shot_seq: seq,
            reason: SmolStr::new(damage::MOVED),
        };
        let mut tape = Tape::new();
        tape.at(
            500,
            EventKind::Detected {
                viewer: ME,
                contact: ContactId(7),
                target: THEM,
            },
        )
        .at(1_000, fired(ME, "missiles", 1, THEM, 1))
        .at(3_400, miss(1))
        .at(
            4_000,
            EventKind::Lost {
                viewer: ME,
                contact: ContactId(7),
                target: THEM,
            },
        )
        .at(5_000, fired(ME, "missiles", 2, THEM, 1))
        .at(7_400, miss(2));
        let report = fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            report.lines,
            vec![
                "  0:01  missiles at a contact: miss: the target moved.",
                "  0:05  missiles at a contact: miss.",
            ]
        );
    }

    #[test]
    fn v3_stamps_are_pinned() {
        assert_eq!(stamp(0), "0:00");
        assert_eq!(stamp(42_999), "0:42");
        assert_eq!(stamp(272_000), "4:32");
        assert_eq!(stamp(3_845_000), "64:05");
        assert_eq!(stamp(-5), "0:00");
        assert_eq!(stamp_fine(43_007), "0:43.007");
    }

    /// A result document the way `write_result` shapes one, for the header.
    fn row() -> HistoryRow {
        let document = serde_json::json!({
            "record": "tacsim.match",
            "record_version": 1,
            "match": 12,
            "arena": "tacsim.arena.proving_ground",
            "outcome": "blue",
            "reason": "elimination",
            "teams": [
                { "team": "red", "units": [101, 102],
                  "pilots": [{ "pilot": 5, "unit": 101, "callsign": "RA" }] },
                { "team": "blue", "units": [201, 202],
                  "pilots": [{ "pilot": 6, "unit": 201, "callsign": "BA" }] },
            ],
            "units": [
                { "unit": 101, "callsign": "RA", "outcome": "destroyed", "shots": 14, "hits": 6 },
                { "unit": 102, "callsign": "RB", "outcome": "intact", "shots": 3, "hits": 1 },
                { "unit": 201, "callsign": "BA", "outcome": "intact", "shots": 1, "hits": 1 },
                { "unit": 202, "callsign": "BB", "outcome": "intact", "shots": 0, "hits": 0 },
            ],
            "ledger": { "log": "match.12", "digest": "ab12cd34ef", "chunks": 42 },
            "seed": 1_234_567,
            "started_ms": 1_790_258_000_000_i64,
            "ended_ms": 1_790_258_520_000_i64,
            "arena_ms": 272_000,
            "reports": [101, 201],
        });
        HistoryRow::from_document(&document).unwrap_or_else(|| empty_row(MatchId(0)))
    }

    /// **16b cut V.3**: the header, both readers, from a row V.2's reader built.
    #[test]
    fn v3_the_header_is_pinned_for_both_readers() {
        let row = row();
        assert_eq!(row.id, MatchId(12));
        assert_eq!(row.reports, vec![ME, THEM]);
        let facts = Report {
            rounds: BTreeMap::from([
                (SmolStr::new("cannon"), 14),
                (SmolStr::new("machine_gun"), 60),
            ]),
            ..Report::default()
        };
        let pilot = header_lines(
            &row,
            "The Proving Ground",
            Reader::Pilot { unit: ME },
            Some(&facts),
            None,
        );
        assert_eq!(
            pilot,
            vec![
                "Match 12 on The Proving Ground, 2026-09-24 14:02 UTC: blue won (red \
                 eliminated), 4 m 32 s.",
                "Red lost 1 of 2; blue lost 0 of 2.",
                "You flew RA: destroyed; 14 shots, 6 hits (42%).",
                "You fired 14 rounds of cannon and 60 of machine_gun.",
            ]
        );
        assert!(pilot.iter().all(|line| !line.contains("1234567")));
        // Blue's pilot, one shot one hit, and nothing fired on the facts.
        let blue = header_lines(
            &row,
            "The Proving Ground",
            Reader::Pilot { unit: THEM },
            Some(&Report::default()),
            None,
        );
        assert_eq!(
            blue.get(2..).unwrap_or_default(),
            [
                "You flew BA: intact; 1 shot, 1 hit (100%).",
                "You fired nothing."
            ]
        );
        assert_eq!(
            fired_line(&BTreeMap::from([(SmolStr::new("cannon"), 1)])),
            "You fired 1 round of cannon."
        );
        assert_eq!(
            fired_line(&BTreeMap::from([
                (SmolStr::new("a"), 1),
                (SmolStr::new("b"), 2),
                (SmolStr::new("c"), 3),
            ])),
            "You fired 1 round of a, 2 of b and 3 of c."
        );
        // Staff: the seed, the tape and the page, and every unit with its pilot.
        let mut names = Roster::from_row(&row);
        names.name_pilots(|pilot| match pilot.0 {
            5 => "Ada".to_owned(),
            other => format!("#{other}"),
        });
        let page = Page {
            first: 0,
            last: 15,
            lines: 412,
            unreadable: 0,
        };
        let staff = header_lines(
            &row,
            "The Proving Ground",
            Reader::Staff,
            None,
            Some((&page, &names)),
        );
        assert_eq!(staff.first(), pilot.first());
        assert_eq!(
            staff.get(2..).unwrap_or_default(),
            [
                "seed 1234567; tape match.12, digest ab12…ef, 42 chunks; this page: chunks 0 to \
                 15, 412 lines, 0 unreadable.",
                "  red RA #101 (Ada #5): destroyed, 14 shots, 6 hits.",
                "  red RB #102 (machine): intact, 3 shots, 1 hit.",
                "  blue BA #201 (#6): intact, 1 shot, 1 hit.",
                "  blue BB #202 (machine): intact, 0 shots, 0 hits.",
            ]
        );
        assert_eq!(
            untracked_header(MatchId(13), true, &page),
            [
                "Match 13 is still running.",
                "tape match.13; this page: chunks 0 to 15, 412 lines, 0 unreadable.",
            ]
        );
    }

    /// **16b cut V.3**: the report document carries the report and reads back;
    /// anything else reads as nothing.
    #[test]
    fn v3_the_report_document_round_trips() {
        let report = fold(&the_tape(), &roster(), Reader::Pilot { unit: ME }, RULES);
        let document = report_document(MatchId(12), ME, Some(ObjId(5)), "RA", &report);
        assert_eq!(document["record"], serde_json::json!("tacsim.report"));
        assert_eq!(document["record_version"], serde_json::json!(1));
        assert_eq!(document["match"], serde_json::json!(12));
        assert_eq!(document["unit"], serde_json::json!(101));
        assert_eq!(document["pilot"], serde_json::json!(5));
        assert_eq!(document["callsign"], serde_json::json!("RA"));
        assert_eq!(document["kills"], serde_json::json!(1));
        assert_eq!(
            document["rounds"],
            serde_json::json!({ "cannon": 2, "machine_gun": 5 })
        );
        assert!(document.get("last_ms").is_none());
        let back = report_from_document(&document).unwrap_or_default();
        assert_eq!(
            back,
            Report {
                last_ms: 0,
                ..report
            }
        );
        let mut foreign = document;
        foreign["record"] = serde_json::json!("tacsim.match");
        assert_eq!(report_from_document(&foreign), None);
    }

    /// A tape with its seqs renumbered from 1 after events were inserted.
    fn renumbered(mut tape: Vec<LedgerEvent>) -> Vec<LedgerEvent> {
        for (n, event) in tape.iter_mut().enumerate() {
            event.seq = u64::try_from(n).unwrap_or(0) + 1;
        }
        tape
    }

    fn event(t_ms: i64, kind: EventKind) -> LedgerEvent {
        LedgerEvent {
            seq: 0,
            t_ms,
            kind,
            draws: Vec::new(),
            hash: 0,
        }
    }

    /// **16b cut V.3 (fix)**: the fold's knowledge is the READER's platform's
    /// alone — a teammate's classification of the same target names nothing
    /// and earns no section — and the section-destroyed clause is under the
    /// same classified gate as the section word, so a round that took a
    /// section off a target lost before it landed adds no clause.
    #[test]
    fn v3_another_platforms_knowledge_and_an_unclassified_section_add_nothing() {
        const RB: ObjId = ObjId(102);
        const OTHER: ContactId = ContactId(0x0fed_cba9_8765_4321);
        let mut roster = roster();
        roster.insert(RB, Some(Team::Red), SmolStr::new("RB"), None);
        let mut tape = the_tape();
        // Before the seq-1 miss: RB tells BA apart on its own contact.
        tape.insert(
            3,
            event(
                20_500,
                EventKind::Classified {
                    viewer: RB,
                    contact: OTHER,
                    target: THEM,
                    team: Team::Blue,
                },
            ),
        );
        // Right after the seq-3 hit (which landed after ME lost BA): the round
        // took BA's hull off.
        let at = tape
            .iter()
            .position(|held| matches!(held.kind, EventKind::Hit { shot_seq: 3, .. }))
            .map_or(tape.len(), |n| n + 1);
        tape.insert(
            at,
            event(
                51_000,
                EventKind::SectionDestroyed {
                    unit: THEM,
                    section: SmolStr::new("hull"),
                    by: Some(ME),
                    consequences: Vec::new(),
                },
            ),
        );
        let report = fold(
            &renumbered(tape),
            &roster,
            Reader::Pilot { unit: ME },
            RULES,
        );
        assert_eq!(
            report.lines.get(1).map(String::as_str),
            Some("  0:20  cannon at a contact: miss: the round went wide."),
            "{:?}",
            report.lines
        );
        assert_eq!(
            report.lines.get(4).map(String::as_str),
            Some("  0:50  machine_gun at a contact: hit."),
            "{:?}",
            report.lines
        );
        for line in every_line(&report) {
            assert!(!line.contains("RB"), "{line}");
            assert!(!line.contains("hull destroyed"), "{line}");
        }
    }

    /// **16b cut V.3 (fix)**: a wreck's platform forgets what it held, with no
    /// `lost` line — so a round it fired that lands after it died is named and
    /// sectioned as the cockpit told it (`Hit.`, `Target destroyed.`).
    #[test]
    fn v3_a_wreck_forgets_every_contact_it_held() {
        let mut tape = Tape::new();
        tape.at(
            1_000,
            EventKind::Classified {
                viewer: ME,
                contact: SEEN,
                target: THEM,
                team: Team::Blue,
            },
        )
        .at(10_000, fired(ME, "cannon", 1, THEM, 1))
        .at(
            10_500,
            EventKind::UnitDestroyed {
                unit: ME,
                by: Some(THEM),
            },
        )
        .at(10_800, hit(1, ME, THEM, "turret", "front", "turret", 0, 0))
        .at(
            10_800,
            EventKind::SectionDestroyed {
                unit: THEM,
                section: SmolStr::new("turret"),
                by: Some(ME),
                consequences: vec![SmolStr::new("destroy_unit")],
            },
        )
        .at(
            10_800,
            EventKind::UnitDestroyed {
                unit: THEM,
                by: Some(ME),
            },
        );
        let report = fold(&tape.0, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            report.lines,
            vec![
                "  0:10  cannon at a contact: hit.",
                "  0:10  a contact destroyed by your round.",
            ]
        );
        assert_eq!(report.tail, vec!["  0:10  your tank destroyed."]);
        assert_eq!(report.kills, 1);
    }

    /// The stock line tank's two tracks as `content/units/line_tank.json`
    /// authors them: each slows, stops the turn that way, and names the pair
    /// that together stop the tank.
    fn line_tank() -> UnitProfile {
        use crate::arena::resolution::SectionProfile;
        use crate::arena::state::Side;
        let track = |id: &str, side: Side| SectionProfile {
            id: SmolStr::new(id),
            facets: Vec::new(),
            consequences: vec![
                Consequence::Slow { percent: 50 },
                Consequence::NoTurnToward { side },
                Consequence::DisableMotionWhenAllOf {
                    sections: vec![SmolStr::new("left_track"), SmolStr::new("right_track")],
                },
            ],
            struck_through: None,
        };
        UnitProfile {
            bands: Vec::new(),
            sections: vec![
                track("left_track", Side::Left),
                track("right_track", Side::Right),
            ],
        }
    }

    /// **16b cut V.3 (fix)**: a lost section of the pilot's own tank reads the
    /// consequences that FIRED, as the cockpit's `SectionLost` line did — the
    /// first track slows and stops the turn and does not stop the tank; the
    /// second does — though both ledger lines carry the whole authored list.
    #[test]
    fn v3_a_lost_track_says_what_fired_and_not_what_was_authored() {
        let authored = || {
            vec![
                SmolStr::new("slow"),
                SmolStr::new("no_turn_toward"),
                SmolStr::new(MOTION_GROUP),
            ]
        };
        let lost = |section: &str, t_ms: i64| {
            event(
                t_ms,
                EventKind::SectionDestroyed {
                    unit: ME,
                    section: SmolStr::new(section),
                    by: Some(THEM),
                    consequences: authored(),
                },
            )
        };
        let tape = renumbered(vec![
            lost("left_track", 125_000),
            lost("right_track", 140_000),
        ]);
        let read = |own: OwnTank| {
            let mut fold =
                Fold::new(roster(), Reader::Pilot { unit: ME }, RULES).with_own_tank(own);
            for held in &tape {
                fold.absorb(held);
            }
            fold.finish().lines
        };
        assert_eq!(
            read(OwnTank::new(&line_tank(), Vec::new())),
            vec![
                "  2:05  your left_track destroyed: the tank is slowed; it will not turn that way.",
                "  2:20  your right_track destroyed: the tank is slowed; it will not turn that way; \
                 the tank cannot move.",
            ]
        );
        // A tank checked out with its left track already gone stops at the
        // first track it loses in the match.
        assert_eq!(
            read(OwnTank::new(&line_tank(), [SmolStr::new("left_track")]))
                .get(1)
                .map(String::as_str),
            Some(
                "  2:20  your right_track destroyed: the tank is slowed; it will not turn that \
                 way; the tank cannot move."
            )
        );
        // A fold that does not know its tank can prove no group complete, so it
        // never says the tank cannot move on that consequence's word.
        for line in read(OwnTank::default()) {
            assert!(!line.contains("cannot move"), "{line}");
        }
    }

    /// **17b cut M3.6**: a lost turret of the pilot's own tank says the
    /// core words as the cockpit's `fired_consequences` does — `its core is
    /// gone` only for a tank fitted with a core, `its core is degraded` only
    /// while the held core reads below its sealed class (never at the floor
    /// of a core sealed at Class 0, never once the same list took it) — and a
    /// fold that does not know its tank's core says neither.
    #[test]
    fn m3_a_lost_turret_says_the_core_words_only_where_they_are_true() {
        let turret = |words: &[&str], t_ms: i64| {
            event(
                t_ms,
                EventKind::SectionDestroyed {
                    unit: ME,
                    section: SmolStr::new("turret"),
                    by: Some(THEM),
                    consequences: words.iter().map(|word| SmolStr::new(*word)).collect(),
                },
            )
        };
        let read = |own: OwnTank, words: &[&str]| {
            let tape = renumbered(vec![turret(words, 125_000)]);
            let mut fold =
                Fold::new(roster(), Reader::Pilot { unit: ME }, RULES).with_own_tank(own);
            for held in &tape {
                fold.absorb(held);
            }
            fold.finish().lines
        };
        let tank = || OwnTank::new(&line_tank(), Vec::new());
        let destroy = ["remove_weapons", DESTROY_CORE];
        let degrade = ["remove_weapons", DEGRADE_CORE];
        let line = |cost: &str| vec![format!("  2:05  your turret destroyed: {cost}.")];
        // A fitted core, destroyed.
        assert_eq!(
            read(
                tank().with_core(OwnCore::new(CoreClass::II, CoreClass::II, true)),
                &destroy
            ),
            line("its weapons are gone; its core is gone")
        );
        // No core fitted, or a fold that does not know: weapons alone.
        assert_eq!(read(tank(), &destroy), line("its weapons are gone"));
        assert_eq!(
            read(OwnTank::default(), &degrade),
            line("its weapons are gone")
        );
        // The heavy's Class III core, one class down.
        assert_eq!(
            read(
                tank().with_core(OwnCore::new(CoreClass::III, CoreClass::III, true)),
                &degrade
            ),
            line("its weapons are gone; its core is degraded")
        );
        // At the floor of a core sealed there: nothing degraded.
        assert_eq!(
            read(
                tank().with_core(OwnCore::new(CoreClass::Zero, CoreClass::Zero, true)),
                &degrade
            ),
            line("its weapons are gone")
        );
        // Destroyed and degraded in one list: gone, and only that.
        assert_eq!(
            read(
                tank().with_core(OwnCore::new(CoreClass::II, CoreClass::II, true)),
                &[DESTROY_CORE, DEGRADE_CORE]
            ),
            line("its core is gone")
        );
    }

    /// One chunk document of `match.12` holding one entry of `events` tape
    /// lines — the shape the world appends and the store hands back.
    fn chunk_doc(n: u64, sealed: bool, events: &[LedgerEvent]) -> (SmolStr, serde_json::Value) {
        let lines: Vec<String> = events
            .iter()
            .filter_map(|held| serde_json::to_string(held).ok())
            .collect();
        (
            mush_core::chunk_key("match.12", n),
            serde_json::json!({
                "chunk": n,
                "sealed": sealed,
                "entries": [{ "seq": n, "value": { "chunk": n, "lines": lines.join("\n") } }],
            }),
        )
    }

    /// **16b cut V.3 (fix)**: a page cut short at the cap says where the rest
    /// of it starts, and `Next:` goes on from there, so following it skips
    /// nothing; a whole page goes on from the chunk after it.
    #[test]
    fn v3_a_page_cut_short_goes_on_from_the_chunk_it_was_cut_in() {
        // Three chunks of ninety lines: the cap falls in the third.
        let run = |from: i64| -> Vec<LedgerEvent> {
            (0..90)
                .map(|n| {
                    let mut held = event(from + n, EventKind::PowerOnline { unit: ME });
                    held.seq = u64::try_from(from + n).unwrap_or(0) + 1;
                    held
                })
                .collect()
        };
        let docs = vec![
            chunk_doc(0, true, &run(0)),
            chunk_doc(1, true, &run(90)),
            chunk_doc(2, false, &run(180)),
        ];
        let folded = fold_page(&docs, "match.12", 0, &roster()).expect("a valid page");
        assert_eq!(folded.report.lines.len(), REPORT_MAX_LINES);
        assert_eq!(folded.report.truncated, 70);
        assert_eq!(folded.cut_at, Some(2));
        assert_eq!(
            folded.page,
            Page {
                first: 0,
                last: 2,
                lines: 270,
                unreadable: 0
            }
        );
        // Cut short: on from the chunk it was cut in, keys absent or not.
        assert_eq!(next_page(0, folded.cut_at, false), Some(2));
        assert_eq!(next_page(0, folded.cut_at, true), Some(2));
        // Cut short inside its own first chunk: on from the next, so it moves.
        assert_eq!(next_page(5, Some(5), true), Some(6));
        // Whole: the chunk after the page, and no `Next:` past the tape's end.
        let whole = fold_page(&docs[..2], "match.12", 0, &roster()).expect("a valid page");
        assert_eq!(whole.cut_at, None);
        assert_eq!(whole.report.truncated, 0);
        assert_eq!(next_page(0, whole.cut_at, true), Some(16));
        assert_eq!(next_page(0, whole.cut_at, false), None);
    }

    #[test]
    fn v3_match_words_are_numbers_with_an_optional_mark() {
        assert_eq!(match_word("12"), Some(MatchId(12)));
        assert_eq!(match_word("#12"), Some(MatchId(12)));
        assert_eq!(match_word("#"), None);
        assert_eq!(match_word("twelve"), None);
        assert_eq!(match_word("-1"), None);
        assert_eq!(short_digest(""), "none");
        assert_eq!(short_digest("abcdef"), "abcdef");
    }

    /// One `ordered` line, as the arena journals it per unit.
    #[allow(clippy::too_many_arguments)]
    fn ordered(
        unit: ObjId,
        issuer: Option<ObjId>,
        objective: &str,
        target: Option<ContactId>,
        spot: Option<&str>,
        hold_fire: bool,
    ) -> EventKind {
        EventKind::Ordered {
            unit,
            squad: SmolStr::new("red-1"),
            issuer,
            objective: SmolStr::new(objective),
            target,
            spot: spot.map(SmolStr::new),
            hold_fire,
            ttl_ms: 600_000,
        }
    }

    /// **16b cut Y.1**: the issuer reads their own orders — once per order,
    /// though the tape carries one line per unit of the squad — with the hint
    /// named through their own knowledge (`BA` for a contact they had told
    /// apart, `a contact` for one they had not), an advance by the spot's name
    /// and never by its cell, and `holding fire` where it was said. The other
    /// pilot reads none of it, an order the machines' own commander gave reads
    /// nothing to anybody, and staff read every field of every line.
    #[test]
    fn y1_the_issuer_reads_their_own_orders_once_and_nobody_else_does() {
        const WING: ObjId = ObjId(301);
        const WING2: ObjId = ObjId(302);
        const BLIP: ContactId = ContactId(0x0000_0000_0000_0042);
        let mut tape = Tape::new();
        tape.at(
            10_000,
            EventKind::Detected {
                viewer: ME,
                contact: SEEN,
                target: THEM,
            },
        )
        .at(
            11_000,
            EventKind::Detected {
                viewer: ME,
                contact: BLIP,
                target: ObjId(202),
            },
        )
        .at(
            25_000,
            EventKind::Classified {
                viewer: ME,
                contact: SEEN,
                target: THEM,
                team: Team::Blue,
            },
        )
        // One order to a squad of two: two lines, one moment.
        .at(
            62_000,
            ordered(WING, Some(ME), "engage", Some(SEEN), None, false),
        )
        .at(
            62_000,
            ordered(WING2, Some(ME), "engage", Some(SEEN), None, false),
        )
        .at(
            70_000,
            ordered(WING, Some(ME), "engage", Some(BLIP), None, true),
        )
        .at(
            71_000,
            ordered(WING, Some(ME), "advance", None, Some("The Ford"), false),
        )
        .at(
            72_000,
            ordered(WING, Some(ME), "advance", None, None, false),
        )
        .at(
            73_000,
            ordered(WING, Some(ME), "withdraw", None, None, true),
        )
        .at(74_000, ordered(WING, Some(ME), "engage", None, None, false))
        // Somebody else's order, and the machines' own commander's.
        .at(75_000, ordered(WING, Some(THEM), "hold", None, None, false))
        .at(76_000, ordered(WING, None, "hold", None, None, false));
        let events = tape.0.clone();

        let mine = fold(&events, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            mine.lines,
            vec![
                "  1:02  ordered red-1: engage BA.",
                "  1:10  ordered red-1, holding fire: engage a contact.",
                "  1:11  ordered red-1: advance on The Ford.",
                "  1:12  ordered red-1: advance.",
                "  1:13  ordered red-1, holding fire: withdraw.",
                "  1:14  ordered red-1: engage.",
            ]
        );

        let theirs = fold(&events, &roster(), Reader::Pilot { unit: THEM }, RULES);
        assert_eq!(
            theirs.lines,
            vec!["  1:15  ordered red-1: hold."],
            "the other pilot reads none of ME's orders, only their own"
        );

        let staff = fold(&events, &roster(), Reader::Staff, RULES);
        let lines: Vec<&String> = staff
            .lines
            .iter()
            .filter(|line| line.contains("ordered"))
            .collect();
        assert_eq!(lines.len(), 9, "staff read every line: {lines:?}");
        assert_eq!(
            lines[0].as_str(),
            "  1:02.000  ordered unit=#301 squad=red-1 issuer=RA(#101) objective=engage \
             target=0123456789abcdef spot=- hold_fire=false ttl_ms=600000"
        );
        assert_eq!(
            lines[3].as_str(),
            "  1:11.000  ordered unit=#301 squad=red-1 issuer=RA(#101) objective=advance \
             target=- spot=The Ford hold_fire=false ttl_ms=600000"
        );
        assert!(
            lines[8].contains("issuer=- objective=hold"),
            "the machines' commander is `-`: {}",
            lines[8]
        );
    }

    /// **16b cut Z.2** (16c N6, Q44): the two squad roles print their own words
    /// on the `ordered` line (the bearing and the charge do not ride the tape),
    /// and a release reads `released red-1 to their minds.` to the tank that gave
    /// it — once, though the tape carries one line per unit it dropped — nothing
    /// to the other pilot, and every field to staff, `issuer=-` for none.
    ///
    /// Was `released red-1 to doctrine.` until 17b cut M5.6 (D9: one word for
    /// every mind).
    #[test]
    fn z2_the_releaser_reads_the_release_once_and_the_roles_by_their_words() {
        const WING: ObjId = ObjId(301);
        const WING2: ObjId = ObjId(302);
        let released = |unit: ObjId, issuer: Option<ObjId>| EventKind::OrderReleased {
            unit,
            squad: SmolStr::new("red-1"),
            issuer,
        };
        let mut tape = Tape::new();
        tape.at(60_000, ordered(WING, Some(ME), "screen", None, None, false))
            .at(61_000, ordered(WING, Some(ME), "support", None, None, true))
            .at(62_000, released(WING, Some(ME)))
            .at(62_000, released(WING2, Some(ME)))
            .at(63_000, released(WING, Some(THEM)))
            .at(64_000, released(WING, None));
        let events = tape.0.clone();

        let mine = fold(&events, &roster(), Reader::Pilot { unit: ME }, RULES);
        assert_eq!(
            mine.lines,
            vec![
                "  1:00  ordered red-1: screen.",
                "  1:01  ordered red-1, holding fire: support.",
                "  1:02  released red-1 to their minds.",
            ]
        );

        let theirs = fold(&events, &roster(), Reader::Pilot { unit: THEM }, RULES);
        assert_eq!(
            theirs.lines,
            vec!["  1:03  released red-1 to their minds."],
            "the other pilot reads none of ME's release, only their own"
        );

        let staff = fold(&events, &roster(), Reader::Staff, RULES);
        let lines: Vec<&String> = staff
            .lines
            .iter()
            .filter(|line| line.contains("order_released"))
            .collect();
        assert_eq!(lines.len(), 4, "staff read every line: {lines:?}");
        assert_eq!(
            lines[0].as_str(),
            "  1:02.000  order_released unit=#301 squad=red-1 issuer=RA(#101)"
        );
        assert_eq!(
            lines[3].as_str(),
            "  1:04.000  order_released unit=#301 squad=red-1 issuer=-"
        );
    }
}
