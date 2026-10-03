//! **The pulse** (16b cut A.2; doc 16 §3.1 "Pulse phases", "Rate independence",
//! "Clock and lag" and "Determinism").
//!
//! One function, [`Arena::step`], and the five phases of doc 16 §3.1 in order,
//! *each reading the state the previous phase committed*. It is a pure function of
//! the arena's state and its two argument slices: no clock of its own (the wall
//! reading is an argument), no task, no I/O. Cut T's `arena_task` is the thin
//! wrapper that owns the timer, the bounded inbox, the bounded egress and the
//! unwind boundary.
//!
//! # The phases, and each one's snapshot rule
//!
//! 1. **Authority.** Every [`Control`] is applied to the epoch table, which purges
//!    queued orders below the new epoch — so a pilot who types `FIRE` then `EXIT`
//!    in one burst never fires, because the burst's `Control` is applied *before*
//!    the burst's `Intent`s. A `Stop` the world sent is honoured here and nowhere
//!    else: it never lands in the middle of a substep.
//! 2. **Orders.** Every [`Intent`] is authorised *at apply* against the table
//!    phase 1 just committed. Continuous orders set a desired value, latest wins;
//!    discrete ones queue for phase 4.
//! 3. **Time.** The wall reading becomes a clamped `dt`, and `dt` becomes a
//!    sequence of substeps. Each substep: (a) motion integrates every unit in
//!    unit-id order **from the substep-start snapshot**, with occupancy read from
//!    the index as the earlier movers left it; (b) the index is rebuilt; (c) arena
//!    time advances by the substep and every due event at or before it is applied
//!    in `(t_ms, seq)` order. The substep's ledger events are appended last,
//!    against the hash of the state at the substep's **end**.
//! 4. **Actions.** Each unit's queue is drained in unit-id order and resolved
//!    against the world phase 3 left. Nothing is re-queued: an action is resolved
//!    or refused in the pulse it was drained in.
//! 5. **The pulse's own record.** The tempo is evaluated, lag is journalled, the
//!    bounds are checked, the ledger's closed chunks are taken and a frame is
//!    built for every pilot whose view changed.
//!
//! # Why the hash is computed once per substep
//!
//! [`crate::arena::hash::state_hash`] folds the ledger's event sequence, so the
//! hash moves with every append. Every event of a substep therefore carries **one**
//! hash — the state at the substep's end, before any of that substep's events was
//! appended — which is what makes the value a function of the state rather than of
//! the order the events happened to be written in. The sequence of those hashes is
//! kept on [`crate::arena::state::Arena::substep_hashes`] for the pulse just run,
//! which is how the cross-tempo property test compares three cadences substep by
//! substep instead of only at the end.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use mush_core::ObjId;
use mush_sim::data::Pose;
use smol_str::SmolStr;

use crate::arena::clock::{Tempo, TempoSignals, substep_ms, substeps};
use crate::arena::hash::state_hash;
use crate::arena::intents::{
    Control, Intent, IntentKind, IntentOutcome, IntentSource, Refusal, apply_control, apply_intent,
    drain_pending, resolve_emcon, resolve_startup,
};
use crate::arena::ledger::{EventKind, LedgerChunk};
use crate::arena::motion::integrate;
use crate::arena::perception::{
    FrameCondition, FrameContact, FrameNotice, Named, Notice, NoticeKind, PerceptionCounters,
    RefusalDetail,
};
use crate::arena::reflex::ReflexCounters;
use crate::arena::state::{
    ActionHash, Arena, Classification, ContactId, DueKind, EndReason, MatchEnd, Power,
    SPENT_NOTICE_MS, StopReason, Team,
};
use crate::arena::weapons::FrameWeapon;

/// **One pilot's latest-wins view** (doc 16 §3.1's `Frames`).
///
/// The minimal slot cut A can honestly fill: where the tank is, how fast it is
/// going, whether its power is up, whether it is held against something, and the
/// tempo the arena is running at. Cut K builds the real cockpit keyframe and delta
/// on exactly this seam, and cut S is what makes a frame *per observer* rather
/// than per unit.
///
/// It is not `PartialEq`: [`Frame::same_as`] compares the fields that decide
/// whether a pilot is told anything, and deliberately not the pose's opaque
/// posture — a frame is a view, and comparing an opaque rules blob every pulse for
/// every pilot would be the most expensive thing in phase 5.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The tank the pilot is in.
    pub unit: ObjId,
    /// **What everybody who has told this tank apart calls it** (16b cut Q.1):
    /// its own team letter and centre slot. `STATUS` opens with it, so a pilot
    /// can answer "who is `RA`?" about themselves.
    pub callsign: crate::arena::state::Designator,
    /// **The arena cell the tank is standing in** (16b cut Q.1), or `None` for a
    /// pose off the window.
    ///
    /// The window's own resolution and no finer, exactly as a contact's `cell`
    /// is: it is what a panel's map draws the pilot's own marker at without
    /// having to divide a pose by a cell size it would have to be told.
    pub cell: Option<(u16, u16)>,
    /// **The side it is on** (16b cut S.2), so the cockpit can draw a classified
    /// contact as a friend or as a hostile without asking the world which team the
    /// pilot's slot was on. The platform's own side is never a secret from its
    /// pilot.
    pub team: Team,
    /// Where it is.
    pub pose: Pose,
    /// How fast, cm/s; negative is reverse.
    ///
    /// **16b cut E.3 decided what this reads while a tank is held against another
    /// unit** ([`crate::arena::state::UnitState::occupied_hold`]): the speed it
    /// had. A hold keeps `speed_cms` and does not set
    /// [`crate::arena::state::UnitState::held`], so for up to
    /// `[tacsim.arena].occupied_stop_ms` the pilot carries the speed they ordered,
    /// a pose that is not advancing, and no `HELD` — and then, at the stop, `0`
    /// and `HELD` together. That is deliberate. Reporting `0` while holding would
    /// make the speed of every tank following another in column flicker to zero
    /// for a substep at a time, and it would disclose nothing a pilot is not
    /// already told sooner: the stop's own `Held: something is in the way.` is a
    /// sentence only [`crate::arena::motion::BlockedCause::Occupied`] produces,
    /// and it arrives no later than it ever did plus the hold.
    pub speed_cms: i32,
    /// Whether its power is up.
    pub power: Power,
    /// Whether it is held against an obstacle, an occupied cell or the window's
    /// edge.
    ///
    /// **16b cut F.2** re-sourced it from [`crate::arena::state::UnitState::held`]
    /// — the *condition* — rather than from `blocked`, the flag for the substep
    /// just integrated: a tank ordered forward against a river bank covers no
    /// whole centimetre on some substeps and read `0 kph` with no `HELD` on those.
    ///
    /// **16b cut M.3** (16c `T-04`) renamed the Rust field from `blocked`, which
    /// named the flag it no longer read, to the condition it does read:
    /// `unit.held.is_some()`. Only the presence goes out — the held cell and its
    /// cause stay in the arena, the cause having already reached the pilot as the
    /// `Held:` sentence of the stop. The **wire key** `status_frame` writes is
    /// still `"blocked"`, because both panels read that key and renaming it is a
    /// client cut's (`crate::match_::cockpit::status_frame`).
    pub held: bool,
    /// **The tank is a wreck** (16b cut D.2, `B-03`; doc 16 §3.3 "A wreck": *no
    /// longer the subject of any order*).
    ///
    /// The pilot's own tank, which is physical knowledge and outside the observer
    /// rule entirely — the same kind of fact [`Frame::sections`] and
    /// [`Frame::heat`] are. It is what lets the cockpit refuse an order at once
    /// (`crate::match_::cockpit::CockpitError::Destroyed`) instead of a pulse
    /// later, which is the rule every other refusal on this surface follows.
    pub wreck: bool,
    /// **The tank cannot move** (16b cut D.2, `B-03`):
    /// [`crate::arena::state::MotionLimits::motion_disabled`], a destroyed engine or
    /// a biped's second leg.
    ///
    /// The pilot's own tank, for the reason [`Frame::wreck`] is.
    /// `crate::match_::cockpit::CockpitError::Immobile` reads it, and prints the
    /// sentence `crate::match_::cockpit::refusal_line` already says for
    /// [`crate::arena::intents::Refusal::MotionDisabled`].
    pub immobile: bool,
    /// The arena's tempo, so a cockpit can say how often it is being told things.
    pub tempo: Tempo,
    /// **How far the platform's own sight reaches**, centimetres (16b cut D.1;
    /// doc 16 §3.6), or `0` for a platform that carries no sensor at all.
    ///
    /// The **nominal** reach, and deliberately so: it is
    /// `SensorProfile::max_range_cells × cell_cm`, the same product
    /// `Arena::detections` calls `max_range_cm` *before* the target's signature,
    /// the thermal channel and any jamming shorten it. The reach a real detection
    /// gets is that figure scaled; this is the size of the scope and not what the
    /// scope can resolve this second, and it must not be scaled here — a jammed
    /// reach on the frame is its own piece of work.
    ///
    /// It is a fact about the pilot's own machine — the same kind of thing
    /// [`Frame::sections`] and [`Frame::heat`] are — so it is outside the observer
    /// rule entirely. [`crate::match_::cockpit::contact_band`] is its reader on
    /// this surface: the band word is thirds of the lesser of this and the fired
    /// weapon's last band. **16b cut E.1** made the product itself
    /// [`crate::arena::state::UnitState::sight_cm`], which the reflex
    /// controller's own fighting reach is priced off as well — one number, so a
    /// machine closes to a band its pilot's `CONTACTS` could name.
    pub sight_cm: u32,
    /// **What the platform holds** (16b cut S.1; doc 16 §3.6 "What a pilot sees"),
    /// in contact-id order — the same order [`crate::arena::state::ShadowContacts`]
    /// iterates in, so two pulses that hold the same set produce the same list.
    ///
    /// It is the platform's contacts and not the observer's: doc 16 §3.7 puts the
    /// station on the tank, so two pilots in one tank see one scope. Cut S.2
    /// renders it; nothing about an undetected unit is in it, which is the whole
    /// observer rule on this surface.
    pub contacts: Vec<FrameContact>,
    /// **The section vocabularies the readings on [`Frame::contacts`] are in**
    /// (16b cut Q.2; doc 16 §3.6), one entry per distinct section-id list, indexed
    /// by [`crate::arena::perception::FrameCondition::table`].
    ///
    /// Built from the contacts **whose `condition` is `Some`** and from no
    /// others. Not from the classified ones: a contact classified over the mesh
    /// that nobody has had a close look at carries no reading, and publishing its
    /// chassis would hand the pilot the one fact the close look is the price of.
    /// A frame whose scope holds no reading carries an empty table list.
    ///
    /// A target's section ids are read off the live unit as a **static fact about
    /// the machine** — the same declared exception the silhouette is — because the
    /// vocabulary is what the bands are in the order of and it does not move while
    /// a match runs.
    pub scan_tables: Vec<Vec<SmolStr>>,
    /// **What the pilot is told once** (16b cut S.2; doc 16 §3.6: *notices fire
    /// once per transition with hysteresis*), in the order the transitions
    /// happened.
    ///
    /// Deliberately outside [`Frame::same_as`]: a frame is sent when its numbers
    /// changed *or* when it carries a notice, and the copy remembered in
    /// `last_frames` has this emptied, so a notice is delivered exactly once and a
    /// later `CONTACTS` re-rendering the remembered frame does not repeat it.
    pub notices: Vec<FrameNotice>,
    /// **The contact the platform is locked on** (16b cut W.1; doc 16 §5's status
    /// block: *current lock*), by its row key, and always one of
    /// [`Frame::contacts`].
    ///
    /// **16b cut Q.1**: the key is not a pilot's handle any more. A pilot names a
    /// contact by the short designator its row carries
    /// ([`crate::arena::perception::FrameContact::designator`]), and a lock taken
    /// under a provisional number survives the rename precisely because what the
    /// arena holds is this id and not the word that was typed.
    pub lock: Option<ContactId>,
    /// **What the platform carries** (16b cut W.1; doc 16 §5: *ammunition and ready
    /// state per weapon*), in component order, with the lock's band where there is a
    /// lock. It is what `WEAPONSTATUS` prints while a match runs.
    pub weapons: Vec<FrameWeapon>,
    /// **What the tank has left** (16b cut F.2; doc 16 §5's status block: *armor
    /// and internals per section as current over max*), in **section-id order**:
    /// [`mush_tactics::TacticalUnit`] holds its sections and their facets in
    /// ordered maps, so a stock tank reads `engine, hull, left_track, right_track,
    /// turret` and not the order its pack authored them in.
    ///
    /// The whole of the recorded open point K7: a pilot shot to pieces had no
    /// surface at all on which to read it. It is a projection of the M32 cells the
    /// impact writes and never a second model — built per pilot per pulse, which is
    /// why a platform nobody is seated on pays nothing for it — and it names only
    /// the pilot's **own** tank, which is physical knowledge and outside the
    /// observer rule entirely.
    pub sections: Vec<FrameSection>,
    /// **What the platform is carrying, and what it may** (16b cut X.1; doc 16 §9
    /// item 1), or `None` for a platform whose content declares no heat — which is
    /// every stock tank, so nothing about a stock cockpit changes.
    ///
    /// It is what lets the cockpit refuse `FIRE` for heat at once instead of a
    /// pulse later, which is the rule every other `FIRE` refusal already follows
    /// (`crate::match_::cockpit`).
    pub heat: Option<FrameHeat>,
    /// **What the platform's own receiver is running under** (16b cut D.4,
    /// `B-22`; doc 16 §9 items 4 and 5), or `None` for a receiver that is clean —
    /// or off.
    ///
    /// [`crate::arena::state::UnitState::receiver`] as its last pass left it. It
    /// is a fact about the pilot's own machine, the same kind of thing
    /// [`Frame::sections`], [`Frame::heat`] and [`Frame::sight_cm`] are, so it is
    /// outside the observer rule entirely — and it carries no source, no bearing,
    /// no side and no object number, which is what keeps it there. See
    /// [`crate::arena::perception::Receiver`] for the two declared limits.
    pub receiver: Option<crate::arena::perception::Receiver>,
    /// **The platform's own jamming pod's switch** (16b cut O.2, 16c N12):
    /// `None` for a platform that carries no pod — every stock tank — and
    /// `Some(`[`crate::arena::state::UnitState::ecm_on`]`)` for one that does.
    ///
    /// The pilot's own machine, like [`Frame::receiver`] beside it, so it is
    /// outside the observer rule; it is what `STATUS`'s `emcon` line, the
    /// panels' scope-row suffix and the cockpit's local `EMCON` refusals read.
    pub ecm: Option<bool>,
    /// **What happened while the pilot could not see** (16b cut Y.4; doc 07 §1.3
    /// step 5), on the first frame after a reclaim and on no other — `None`
    /// everywhere else.
    ///
    /// One-shot, as [`Frame::notices`] is: `Arena::frames` takes it out of
    /// `Arena::pending_catchup` onto the frame and the copy remembered in
    /// `last_frames` has it cleared, and the world's mirror stores it cleared
    /// too, so `STATUS` and `VIEW` never re-say it. [`Frame::same_as`] answers
    /// `false` whenever either side carries one, so a frame that carries it is
    /// always sent. The pilot's own tank, plus the platform's own designators
    /// for the contacts it held or holds — the observer rule, as
    /// [`crate::arena::catchup`] states it.
    pub catchup: Option<crate::arena::catchup::Catchup>,
}

/// **One facet's armour, as the pilot's own status block carries it** (16b cut
/// F.2; doc 16 §5: *armor ... per section as current over max*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameFacet {
    /// The facet's id (`front`, `left`, `right`, `rear`, `top`).
    pub id: SmolStr,
    /// What it has left.
    pub armor: i64,
    /// What it holds when it is whole.
    pub armor_max: i64,
}

/// **One section of the pilot's own tank** (16b cut F.2; doc 16 §5's status
/// block), read off the M32 cells the damage pipeline writes.
///
/// `internals` is `None` for a section whose content declares no internals cell —
/// there is nothing to report rather than a nought that would read as destroyed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameSection {
    /// The section's id (`hull`, `turret`, `left_track`, …).
    pub id: SmolStr,
    /// Its opaque condition, as M32 holds it (`intact`, `destroyed`, `wreck`).
    pub condition: SmolStr,
    /// Its internals as current over max.
    pub internals: Option<(i64, i64)>,
    /// Its facets, in **facet-id order** (the unit's own ordered map's), so the
    /// hull reads `front, left, rear, right, top`.
    pub facets: Vec<FrameFacet>,
}

/// **The platform's heat, as its pilots' frame carries it** (16b cut X.1).
///
/// Two integers and nothing derived: the ceiling is the cell's own maximum, so
/// the cockpit's predicate and [`crate::arena::state::UnitState::overheated`] are
/// the same comparison over the same pair and cannot drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeat {
    /// What it is carrying.
    pub heat: u32,
    /// The ceiling it refuses to fire at.
    pub max: u32,
}

impl FrameHeat {
    /// Whether the platform is at its ceiling — doc 16 §9 item 1's refusal, read
    /// off the frame.
    #[must_use]
    pub const fn overheated(&self) -> bool {
        self.heat >= self.max
    }
}

impl Frame {
    /// Whether this frame says the same thing as `other`: the unit, the four pose
    /// integers, the speed, the power, the block, the tempo and the contact list.
    ///
    /// **The contacts are compared as a pilot reads them** (16b cut U.2, 16c
    /// `T-30`, Q27, Q28, `T-20`; doc 16 §3.1's latest-wins slot). A row's age
    /// moves every millisecond it is held, and it reaches a pilot through three
    /// surfaces: the **whole second** every panel and `CONTACTS` print, the
    /// **stale clause** the read verbs add once the plot is older than
    /// `pass_ms`, and on the locked row the **lock's range step**, which moves
    /// below a second. A frame is sent when any of them would print differently
    /// ([`crate::arena::perception::FrameContact::same_as`], through
    /// [`crate::arena::perception::age_key`] and
    /// [`crate::arena::perception::lock_range_step_m`]) and not on every pulse
    /// the milliseconds moved. The keyframe still carries `age_ms` to the
    /// millisecond, so what a pilot **may read** is unchanged on every surface;
    /// only what they are **sent** is. Until this cut the rows were compared in
    /// full, and every pilot holding any contact was sent a whole keyframe every
    /// wake.
    ///
    /// `pass_ms` is the arena's own `[tacsim.sensors].pass_ms`
    /// (`Arena::sensors_cfg`), the same number the world's `stale_clause` reads
    /// off its configuration.
    ///
    /// The notices are **not** compared: see [`Frame::notices`]. Nor is a weapon's
    /// `ready_in_ms` countdown: see [`FrameWeapon::same_as`].
    ///
    /// **16b cut X.1** put the heat in: a platform that is cooling changes a
    /// number every substep and its pilot is told, because a heat gauge that
    /// lagged a tempo behind would be a lie about the one thing standing between
    /// them and the next shot. A platform with no heat compares `None` to `None`
    /// and costs nothing.
    ///
    /// **16b cut Y.4**: a frame carrying a catch-up report is never the same as
    /// anything ([`Frame::catchup`]) — a re-seated pilot has no remembered frame
    /// anyway, and this pins that the report is sent.
    #[must_use]
    pub fn same_as(&self, other: &Frame, pass_ms: i64) -> bool {
        self.catchup.is_none()
            && other.catchup.is_none()
            && self.unit == other.unit
            && self.team == other.team
            // 16b cut Q.1: both are surfaces a pilot reads — the callsign on
            // `STATUS`'s first line, the cell on a panel's map — so both decide
            // whether a frame is sent. The callsign never moves in a match and
            // the cell moves once a cell, so neither costs a frame anybody would
            // not already have been sent.
            && self.callsign == other.callsign
            && self.cell == other.cell
            && self.pose.x_cm == other.pose.x_cm
            && self.pose.y_cm == other.pose.y_cm
            && self.pose.z_cm == other.pose.z_cm
            && self.pose.heading_mdeg == other.pose.heading_mdeg
            && self.speed_cms == other.speed_cms
            && self.power == other.power
            && self.held == other.held
            // **16b cut D.2**: both decide what the cockpit refuses locally, and
            // both are one-way flips that happen at most once in a match — so
            // comparing them costs no frame anybody would not already have been
            // sent, and a pilot whose tank has just died is sent one at once.
            && self.wreck == other.wreck
            && self.immobile == other.immobile
            && self.tempo == other.tempo
            // **16b cut D.1**: the pilot's band word is read against it, so a
            // frame that carried a different sight would print different words.
            // It never moves inside a match — the sensor row and the cell size
            // are both fixed at check-out — so comparing it costs no frame
            // anybody would not already have been sent.
            && self.sight_cm == other.sight_cm
            // **16b cut U.2** (16c `T-30`): row by row, at the resolutions a
            // surface prints the age at; the locked row at the lock's step too.
            && self.contacts.len() == other.contacts.len()
            && self
                .contacts
                .iter()
                .zip(other.contacts.iter())
                .all(|(mine, theirs)| mine.same_as(theirs, pass_ms, self.lock == Some(mine.id)))
            // **16b cut Q.2**: the readings ride the contacts, which are already
            // compared row by row, and the tables are what those readings are in the
            // order of — so a table list that moved with no contact moving would
            // be a panel drawing the wrong section names. Two `Vec<Vec<SmolStr>>`
            // of at most one entry per distinct chassis in the scope.
            && self.scan_tables == other.scan_tables
            && self.lock == other.lock
            && self.heat == other.heat
            // **16b cut D.4**: `STATUS` prints it and a panel draws it, so a
            // receiver that changed degree inside one bubble — which journals
            // nothing and says nothing — still sends the pilot the numbers.
            && self.receiver == other.receiver
            // **16b cut O.2**: `STATUS` prints it and both panels draw it, and
            // it moves only on a pilot's own flip, so comparing it costs no
            // frame anybody would not already have been sent.
            && self.ecm == other.ecm
            // **16b cut F.2**: the damage surface is compared, so a round that
            // came off the armour sends a frame. It is the cheapest comparison on
            // this list after the integers — a handful of small structs per pilot
            // — and a status block that lagged a pulse behind the tank would be
            // the same lie the heat gauge would be.
            && self.sections == other.sections
            && self.weapons.len() == other.weapons.len()
            && self
                .weapons
                .iter()
                .zip(other.weapons.iter())
                .all(|(mine, theirs)| mine.same_as(theirs))
    }
}

/// **What one pulse produced** (doc 16 §3.1: `StepOutput { ledger_chunks, frames,
/// tempo, next_due_ms }`, plus what cut T's task needs to schedule and to stop).
#[derive(Debug)]
pub struct StepOutput {
    /// Ledger chunks that closed and have not been handed over before. The world
    /// half appends them to `log:match.<id>` on its own writer lane and
    /// acknowledges them with `Ledger::mark_flushed`.
    pub ledger_chunks: Vec<LedgerChunk>,
    /// One frame per **pilot** whose view changed this pulse, in pilot-id order.
    pub frames: Vec<(ObjId, Frame)>,
    /// The tempo the arena is now in — the period cut T's timer uses.
    pub tempo: Tempo,
    /// When the task should step again, in the **wall** milliseconds
    /// `wall_now_ms` was given in: the earlier of the next multiple of the period
    /// and the first substep boundary at or after the next due event, both in
    /// absolute arena time (16b cut U.1; `Arena::next_due_ms` has the rule and
    /// why), so a production wake is on the substep lattice and never at a due
    /// event's own millisecond.
    pub next_due_ms: i64,
    /// The pulse absorbed more wall time than the clamp allows, or the egress is
    /// backing up.
    pub lagged: bool,
    /// Set once, on the pulse the match ends.
    pub ended: Option<MatchEnd>,
    /// How many substeps this pulse ran, the phase-4 drain included; the opening
    /// pulse runs none, because it only establishes the wall reference.
    ///
    /// **16b cut M.2** (16c `T-10`): it is counted as the substeps run, so a pulse
    /// whose second of four substeps ended the match reports `2` — it reported
    /// the whole partition — and a pulse whose phase 4 scheduled a zero-flight
    /// event reports one more than its partition (`Arena::step`'s phase-4 drain).
    pub substeps: u32,
    /// **What perception cost this pulse** (16b cut S.1; doc 16 §2 W4 and W6). The
    /// harness asserts `harness.max_traces_per_pulse` and
    /// `harness.max_share_ops_per_pulse` against it.
    pub perception: PerceptionCounters,
    /// **What the reflex controller cost this pulse** (16b cut N.1; doc 16 §1 A2:
    /// *path queries per pulse under their bound*). The harness asserts
    /// `harness.max_path_queries_per_pulse` against
    /// [`ReflexCounters::path_queries`] and reports the state distribution.
    pub reflex: ReflexCounters,
    /// **The squads with something new to decide about** (16b cut N.2; doc 16 §6
    /// "Deliberation"), already floored by `[tacsim.arena].deliberate_min_ms` in
    /// arena time.
    ///
    /// Empty on every pulse of a match with no squads, which is every fixture and
    /// every match of pilots. The world half turns each into one request through
    /// its own LLM re-entry seam and the reply comes back as an ordinary
    /// [`crate::arena::intents::IntentKind::Order`]; the arena waits for neither
    /// and is told nothing if the world drops it.
    pub deliberation: Vec<crate::arena::command::SquadSignal>,
    /// **The staff picture of the field** (16b cut I.4; 16c N10), built in
    /// phase 5 while [`crate::arena::state::Arena::watched`] is set, **at most
    /// once per pulse and at most once an arena second** — and on the pulse the
    /// match ended, so a watcher sees the end. `None` otherwise, on the ended
    /// early return and on a stopped pulse.
    ///
    /// A type of its own ([`crate::arena::observer::Observer`]) and never part
    /// of [`StepOutput::frames`]: it names every unit, and a pilot's frame names
    /// only what their platform holds (doc 16 §1).
    pub observer: Option<crate::arena::observer::Observer>,
}

// ---------------------------------------------------------------------------
// The two hidden fault-injection hooks (16b cut T.2)
// ---------------------------------------------------------------------------
//
// Cut T's task owes two properties the library cannot be asked to demonstrate on
// its own: *a panic inside `step` ends the match `Void` and nothing the faulted
// pulse produced leaves*, and *a pulse past `tempo.stall_ms` is voided by the
// watchdog* (doc 16 §3.1 "Failure"). Both need a `step` that misbehaves on
// demand, and neither can be produced by any legal input — which is exactly why
// they are hooks and not test fixtures.
//
// They are armed **by seed**, not by a flag on the arena: [`Arena`] is
// `state.rs`' and cut T is not allowed to add a field to it, and a plain global
// flag would be armed by one async test and consumed by another running
// beside it in the same binary. The arming method takes `&mut self` and reads
// `self.seed`, so a test can only ever arm the arena it is holding, and two
// tests with two seeds cannot collide. A seed of `0` disarms (and no test uses
// it).
//
// `std::panic::resume_unwind` rather than `panic!`: this crate's root denies
// `clippy::panic` (which also refuses `panic_any`) and
// `panic_budget/tacsim.txt` counts the `panic!` spelling, and both exist to stop
// *accidental* panic surface. A `#[doc(hidden)]` hook whose whole job is to
// unwind, reachable only from the test that armed it by seed, is not that — and
// silencing the lint would move the pinned restriction-lint census instead,
// which is strictly worse than one named call in one place. `resume_unwind`
// raises exactly the unwind `catch_unwind` catches, with exactly the payload
// given, and skips the panic hook — so a test that expects the fault does not
// print a backtrace for it either.

/// The seed of the arena whose next [`Arena::step`] must panic; `0` for none.
static PANIC_SEED: AtomicU64 = AtomicU64::new(0);
/// The seed of the arena whose next [`Arena::step`] must stall; `0` for none.
static STALL_SEED: AtomicU64 = AtomicU64::new(0);
/// How long that stall lasts, milliseconds.
static STALL_MS: AtomicU64 = AtomicU64::new(0);

impl Arena {
    /// **Test hook:** make the next [`Arena::step`] of *this* arena panic,
    /// once.
    ///
    /// Nothing in the shipped lane calls it. See this module's
    /// fault-injection note for why it is armed by seed.
    #[doc(hidden)]
    pub fn panic_on_next_step_for_tests(&mut self) {
        PANIC_SEED.store(self.seed, Ordering::SeqCst);
    }

    /// **Test hook:** make the next [`Arena::step`] of *this* arena block for
    /// `how_long` before doing its work, once — a pulse the task's watchdog
    /// must void.
    ///
    /// It blocks the calling thread, because that is what a stalled pulse does:
    /// the arena has no preemption, and the watchdog reads the clock *after*
    /// `step` returns.
    #[doc(hidden)]
    pub fn stall_next_step_for_tests(&mut self, how_long: std::time::Duration) {
        STALL_MS.store(
            u64::try_from(how_long.as_millis()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
        STALL_SEED.store(self.seed, Ordering::SeqCst);
    }

    /// Consume whichever fault this arena's seed is armed with. A no-op — one
    /// relaxed load of each of two atomics — on every arena nobody armed, which
    /// is every arena in the shipped lane.
    fn injected_fault(&self) {
        if self.seed != 0 && PANIC_SEED.load(Ordering::SeqCst) == self.seed {
            PANIC_SEED.store(0, Ordering::SeqCst);
            std::panic::resume_unwind(Box::new(
                "arena step panicked (panic_on_next_step_for_tests)",
            ));
        }
        if self.seed != 0 && STALL_SEED.load(Ordering::SeqCst) == self.seed {
            STALL_SEED.store(0, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(
                STALL_MS.swap(0, Ordering::SeqCst),
            ));
        }
    }

    /// **One pulse** — doc 16 §3.1's five phases, in order.
    ///
    /// `wall_now_ms` is the caller's monotonic wall reading; arena time advances by
    /// the interval between two readings, clamped to `max_catch_up_steps × period`
    /// (doc 16 §3.1 "Clock and lag"). The first call establishes the reference and
    /// integrates nothing, so the opening pulse is identical at every tempo.
    ///
    /// A match that has already ended does nothing at all: the output carries its
    /// [`MatchEnd`] and no chunk, no frame and no substep. Cut T stops stepping at
    /// the first output with `ended`; a task that steps once more anyway cannot
    /// move the world afterwards.
    pub fn step(
        &mut self,
        wall_now_ms: i64,
        controls: &[Control],
        intents: &[Intent],
    ) -> StepOutput {
        self.injected_fault();
        if let Some(ended) = self.ended {
            return StepOutput {
                ledger_chunks: Vec::new(),
                frames: Vec::new(),
                tempo: self.clock.tempo(),
                next_due_ms: wall_now_ms,
                lagged: false,
                ended: Some(ended),
                substeps: 0,
                perception: PerceptionCounters::default(),
                reflex: ReflexCounters::default(),
                deliberation: Vec::new(),
                observer: None,
            };
        }
        self.substep_hashes.clear();
        let mut perception = PerceptionCounters::default();
        let mut reflex = ReflexCounters::default();
        // **16b cut N.1**: what the pulse fired, wherever it fired it. Cut W.1 read
        // this off phase 4's own return, which stopped being the whole answer the
        // moment the reflex controller began accepting shots inside a substep. The
        // shot sequence is the one counter that moves on every accepted shot and on
        // nothing else, so the difference across the pulse is the fact.
        let shots_before = self.next_shot_seq;

        // ---- phase 1: authority, and the world's `Stop` ----
        //
        // Controls first, and every control before any intent: the epoch table
        // this phase commits is what phase 2 authorises against, which is the
        // whole mechanism behind doc 16 §3.1's `FIRE` then `EXIT` burst.
        for control in controls {
            apply_control(self, control);
        }
        if let Some(stop) = self.stop {
            return self.stopped(wall_now_ms, stop, intents);
        }

        // ---- phase 2: orders ----
        //
        // **16b cut D.2** (`B-03`): this loop used to throw every refusal away, so
        // an order to a dead tank, to a tank whose engine was gone, or one past
        // the queue's bound simply vanished — three of six smoke matches left a
        // pilot typing into a wreck and being told nothing. Four of the refusals
        // are now staged, and the other five are silent on purpose: see
        // [`Arena::phase_two_tells`].
        let mut fire_intent_queued = false;
        // **16b cut Y.1**: phase 2 runs at one arena timestamp, so every `ordered`
        // line of one pulse belongs to one hash — an `ORDER ALL` over four pairs
        // costs one `state_hash`, not eight. 16b cut M.3's rule (`T-23`) kept: the
        // slot is `Pending`, the first `ordered` line takes it
        // (`Arena::event_hash`), and a pulse with no order takes nothing. **16b
        // cut Z.2**: the `order_released` line is the other thing phase 2
        // journals, and it takes the same slot, so the `ordered` and
        // `order_released` lines of one pulse share one hash. **16b cut I.3**: so
        // does the `place_refused` line of a staff place this phase refuses;
        // nothing else here journals (`stage_refusal` writes no ledger line).
        self.action_hash = ActionHash::Pending;
        for intent in intents {
            // **16b cut Z.2** (16c Q44): a release is counted where it lands — a
            // unit whose standing order the release took away. A release to a
            // unit under no order, or under another squad's, dropped nothing and
            // counts nothing.
            let releasing = matches!(intent.kind, IntentKind::ReleaseOrder { .. })
                && self.reflex.order(intent.unit).is_some();
            // **16b cut M.1** (`T-82`): every order in this slice came through the
            // world's seam, so it is a seat's and counts against the rate.
            let outcome = apply_intent(self, intent, IntentSource::Seat);
            if releasing && self.reflex.order(intent.unit).is_none() {
                reflex.orders_released = reflex.orders_released.saturating_add(1);
            }
            if outcome == IntentOutcome::Queued && matches!(intent.kind, IntentKind::Fire { .. }) {
                fire_intent_queued = true;
            }
            // **16b cut I.3** (16c N10: *journalled always, announced never*):
            // a staff `Place` refused here — `unknown`, `epoch` or `channel`, the
            // three gates it meets — still leaves its line, under phase 2's one
            // hash like the `ordered` lines, so every `Place` the arena receives
            // is on the tape by one rule. Nothing is staged:
            // `Arena::phase_two_tells` answers `false` for a place.
            if let (IntentOutcome::Refused(refusal), IntentKind::Place { x, y }) =
                (outcome, &intent.kind)
            {
                let hash = self.event_hash();
                let at = self.now_ms;
                let _ = self.journal(
                    at,
                    EventKind::PlaceRefused {
                        unit: intent.unit,
                        x: *x,
                        y: *y,
                        reason: SmolStr::new_static(refusal.as_str()),
                    },
                    hash,
                );
            }
            if let IntentOutcome::Refused(refusal) = outcome
                && Arena::phase_two_tells(&intent.kind, refusal)
            {
                let weapon = match &intent.kind {
                    IntentKind::Fire { weapon } => Some(weapon.clone()),
                    _ => None,
                };
                // The word is `IntentKind::as_str`'s and phase 4 uses the same
                // accessor, so the two phases cannot spell one order two ways.
                self.stage_refusal(intent.unit, intent.kind.as_str(), refusal, weapon);
            }
        }
        // **16b cut Y.1**: the phase-2 window closes before time moves.
        self.action_hash = ActionHash::Outside;

        // ---- phase 3: time, in substeps ----
        //
        // **16b cut I.3** (16c N10; `crate::arena::clock`'s head): whether this
        // pulse is quantised is read BEFORE `advance` overwrites the previous
        // wake's floor, so the first pulse after a lift is quantised too.
        let floored = self.clock.floored_now_or_then();
        let (dt_ms, clock_lagged) = self.clock.advance(wall_now_ms, &self.arena_cfg);
        // **16b cut I.3, the lattice.** Under a staff floor the task wakes on a
        // WALL interval (`Arena::next_due_ms`), because the arena is behind the
        // wall on purpose and a lattice target in arena time cannot be reached
        // by a wake that is. So the pulse itself is cut back to the substep
        // lattice: `dt` becomes the greatest lattice point at or below
        // `now + dt`, less `now` — which may be ZERO, a wake that came inside a
        // substep being a pulse of no time (the scripted drivers deliver every
        // beat in one). The excess is discarded as the clamp discards its excess.
        // Without it, the pull-in pulse on a set and every wake below the clamp
        // would add the OS timer's overshoot to arena time and never return to
        // the lattice, so due events would be applied against positions that
        // depend on the wall moment staff typed the verb (D-16-2; 16b cut U.1).
        // A floor is a multiple of `engaged_ms`, which is a multiple of the
        // substep for every shipped configuration (16b cut M.2, 16c Q24), so an
        // on-time floored wake moves exactly `min(cap, floor)` and loses nothing.
        let dt_ms = if floored {
            let lattice = self.substep_len_ms();
            let end = floor_to(self.now_ms.saturating_add(i64::from(dt_ms)), lattice);
            u32::try_from(end.saturating_sub(self.now_ms).max(0)).unwrap_or(dt_ms)
        } else {
            dt_ms
        };
        let length = u32::try_from(self.substep_len_ms()).unwrap_or(u32::MAX);
        let pieces = substeps(dt_ms, length);
        // **16b cut M.2** (16c `T-10`): what ran, counted as it runs — the loop
        // below breaks on the substep that ended the match, and the partition's
        // length would report the pieces it never ran.
        let mut ran: u32 = 0;
        let mut ledger_full = false;
        for piece in pieces {
            ledger_full |= self.substep(piece, &mut perception, &mut reflex);
            ran = ran.saturating_add(1);
            // **16b cut W.2**: a substep that ended the match ends the pulse. A
            // match is over at the arena millisecond it was won, and stepping the
            // rest of the pulse would journal contacts expiring and summaries
            // running after `match_ended` — lines whose count is a function of the
            // cadence, which is exactly what doc 16 §3.1's rate independence forbids.
            if self.ended.is_some() {
                break;
            }
        }

        // ---- phase 4: the queued discrete actions ----
        //
        // Doc 16 §3.1: *resolve newly queued discrete actions that became legal this
        // pulse*. Nothing is re-queued — an action is resolved or refused in the
        // pulse it was drained in, so a bound another order needs is never held by
        // one that cannot be resolved — and a refusal is the pilot's business, not
        // the ledger's: **16b cut F.2** stages a `FrameNotice::Refused` here and
        // `crate::match_::cockpit::refusal_line` is what turns it into a
        // sentence, so a refused order is said and still never written to the
        // black box.
        //
        // One hash for the whole phase, taken once: the phase runs at one arena
        // timestamp with no time advancing inside it, so every action it resolves
        // happened at one moment and belongs to one hash — the same rule the substep
        // above follows, and see `Arena::action_hash` for what it costs not to.
        // A match that ended inside phase 3 resolves nothing here: a shot accepted
        // after `match_ended` would be a line in the black box about a match that was
        // already over (16b cut W.2).
        //
        // **16b cut M.3** (16c `T-23`): the slot is `Pending`, not a hash — the
        // first resolver that journals takes it (`Arena::event_hash`), so a pulse
        // with nothing queued, or an ended match, takes no `state_hash` here at
        // all. It used to be taken unconditionally, every pulse, before the
        // `ended` guard.
        self.action_hash = ActionHash::Pending;
        // **16b cut M.2** (16c `T-02`; doc 16 §11's D-16-2 as amended): a pilot's
        // discrete action resolves at the end of the pulse that accepted it, so
        // its arena millisecond is this one — which in production is a wall-clock
        // instant, since arena time is elapsed wall time and a pulse ends on the
        // substep lattice (the earlier of the next period multiple and the first
        // boundary at or after the next due event, 16b cut U.1) plus the timer's
        // overshoot. Everything the action
        // schedules from here is a due event and rate-independent. The resolvers
        // take the millisecond as an argument because a machine's actions resolve
        // at its tick instead (`crate::arena::reflex`).
        let at_ms = self.now_ms;
        let ids: Vec<ObjId> = if self.ended.is_some() {
            Vec::new()
        } else {
            self.units.iter().map(|unit| unit.id).collect()
        };
        for id in ids {
            for order in drain_pending(self, id) {
                // **16b cut D.2**: the order's own word, through the one accessor
                // phase 2 reads. This phase passed the literals `"startup"`,
                // `"lock"` and `"fire"`, which was a second spelling of a
                // vocabulary `IntentKind::as_str` already owns — and the bridge's
                // `Starting` hint matches on that accessor's answer.
                let word = order.kind.as_str();
                match order.kind {
                    // **16b cut I.3** (16c N10): the staff place, resolved at the
                    // pulse's end under the phase's one hash like every action
                    // here. `resolve_place` journals its own outcome, `placed` or
                    // `place_refused`, and nothing is ever staged to a pilot —
                    // a refusal naming a round in flight would tell the pilot of
                    // an impact their sensors never saw.
                    IntentKind::Place { x, y } => {
                        let _ = self.resolve_place(id, x, y, at_ms);
                    }
                    // Doc 16 §3.3 "Startup".
                    IntentKind::Startup => {
                        if let Err(refusal) = resolve_startup(self, id, at_ms) {
                            self.stage_refusal(id, word, refusal, None);
                        }
                    }
                    // **16b cut O.2** (16c N12): the pod's switch, in `Startup`'s
                    // shape. `resolve_emcon` journals and stages its own flip; a
                    // no-op journals and stages nothing.
                    IntentKind::Emcon { on } => {
                        if let Err(refusal) = resolve_emcon(self, id, on, at_ms) {
                            self.stage_refusal(id, word, refusal, None);
                        }
                    }
                    // **16b cut W.1** (doc 16 §3.7, §3.5): the lock, and the shot.
                    IntentKind::Lock { contact } => match self.resolve_lock(id, contact, at_ms) {
                        // **16b cut F.2** (doc 16 §3.5 step 5): the percentage
                        // *on `LOCK` before any shot*. The lock resolves here, at
                        // its own arena timestamp, so this is where it is said —
                        // the cockpit could only ever say it against the frame it
                        // held *before* the order, which is why a first `LOCK`
                        // was silent.
                        // 16b cut Q.1: named off the live contact at the moment
                        // the lock resolved.
                        Ok(()) => {
                            let named = self.name_of(id, contact);
                            self.stage_notice(id, FrameNotice::Locked { contact: named });
                        }
                        Err(refusal) => self.stage_refusal(id, word, refusal, None),
                    },
                    IntentKind::Fire { ref weapon } => {
                        if let Err(refusal) = self.resolve_fire(id, weapon, at_ms) {
                            self.stage_refusal(id, word, refusal, Some(weapon.clone()));
                        }
                    }
                    // Continuous orders never queue (phase 2 applies them), and
                    // neither does a squad's standing order (16b cut N.2): it is
                    // written onto the reflex machine at apply and read by that
                    // machine's own tick.
                    IntentKind::Heading { .. }
                    | IntentKind::Speed { .. }
                    | IntentKind::Order(_)
                    | IntentKind::ReleaseOrder { .. } => {}
                }
            }
        }
        self.action_hash = ActionHash::Outside;

        // ---- the phase-4 drain (16b cut M.2, 16c `B-51`) ----
        //
        // A pilot's beam flies in no time, so phase 4 schedules its impact at
        // `now_ms` — and phase 4 drains nothing. The round then resolved in the
        // NEXT pulse's first substep, against positions later than the fire
        // timestamp it is stamped with, and `next_due_ms` answered the wall
        // reading itself, so the task's timer slept for nothing. One zero-length
        // substep here pops exactly what phase 4 left due at `now_ms`.
        //
        // **The `ran > 0` guard is the rule, not a precaution.** After at least
        // one substep, every event at or before `now_ms` that was in the heap
        // before phase 4 has already been popped by the last substep's due loop,
        // and the substep's staging scan schedules nothing at or before `now_ms`
        // (a held loss whose window closes inside the substep is settled by the
        // scan itself, never scheduled into the past), so anything still due now
        // was scheduled BY phase 4 — a beam's zero-flight impact — and the drain
        // is exactly "phase 4's own zero-flight events". (A `WeaponReady` cannot
        // be one with shipped content: `cycle_ms` is floored at 1,000 by W14,
        // longer than any substep.) The opening pulse runs no substep and drains
        // nothing: its `t = 0` events (a `SensorPass` or a `ReflexTick` for a
        // unit whose stagger is nought — `stagger_ms`, 16b cut U.1 — and a
        // `MatchDeadline` of `0`) pop in the next pulse's first substep exactly
        // as they always did, so the opening pulse's hash sequence, its
        // `substeps == 0` and phase 5's duration backstop are untouched. A
        // zero-length substep is well-defined (`Arena::substep` skips motion at
        // `dt 0`, `dissipate_heat(0)` sheds nothing, the index is rebuilt once,
        // the due loop pops what phase 4 scheduled, elimination settles, one hash
        // is recorded).
        //
        // **The drain runs only on a pulse that ran a substep** — or, since 16b
        // cut I.3, on a floored one (below). Under the scripted zero-`dt`
        // drivers, which are unfloored, a beat's pulse runs none, so there the
        // drain never runs: a beam fired on a beat still lands in the next
        // pulse's first substep, the same at every cadence, which is why those
        // replays agree and why they do not prove the drain. It is proved on a
        // driver whose pulse ends on the beat, the production shape
        // (`tests/extensions.rs`'s beam replay and `tests/weapons.rs`'s
        // `a_beam_is_scheduled_at_now`).
        //
        // **16b cut I.3: and on a quantised pulse of no time, past the opening.**
        // Under a staff floor a production pulse can run no substep — the
        // pull-in on a set, a lift or an abort, or any wake inside a substep,
        // cut back to the lattice point it started on. A beam `FIRE` such a
        // pulse resolves at `now_ms` would otherwise wait out the floor and land
        // in the next wake's first substep, against positions a substep later
        // than the same `FIRE` resolved at the same millisecond by a pulse that
        // ran one: the outcome would turn on the wall moment staff typed the
        // verb, which is what the quantisation exists to exclude. The rule above
        // holds for it by induction: once arena time has moved (`now_ms > 0`),
        // the pulse before this one either ran a substep ending at `now_ms` or
        // was itself a floored pulse of no time and drained — an unfloored
        // production pulse always runs one, `arena_task`'s `deadline` giving it
        // at least a millisecond — so anything due now was scheduled by this
        // phase 4. At `now_ms == 0` nothing has run and the `t = 0` events are
        // the next substep's, exactly as after the opening pulse, so a floored
        // pulse of no time there drains nothing (`tests/weapons.rs`'s
        // `i3_a_beam_fired_in_a_floored_pulse_of_no_time_lands_in_it`).
        let drains = ran > 0 || (floored && self.now_ms > 0);
        if drains && self.ended.is_none() && self.due.peek_due(self.now_ms).is_some() {
            ledger_full |= self.substep(0, &mut perception, &mut reflex);
            ran = ran.saturating_add(1);
        }

        // ---- phase 5: the pulse's own record ----
        //
        // Doc 16 §3.1's `Alert` row: *any unit holds a contact (classified or not)
        // whose Euclidean range is under `tempo.alert_range_cm`*, *or a fire intent
        // is queued*. It is evaluated here, after phase 3 minted and expired this
        // pulse's contacts and phase 4 resolved this pulse's shots, so the tempo
        // answers to the world the pulse ended with.
        //
        // The `Engaged` row's first two triggers are one fact in this engine: doc 16
        // §3.5 makes every accepted shot schedule an impact, so a pulse that fired is
        // a pulse that scheduled one.
        //
        // **16b cut N.1 fills the other two.** Its **third** `Engaged` trigger —
        // *hostile units are within weapons range of each other* — was left `false`
        // by cut W.1 because a field-wide sweep is a cost nobody had bounded; it is
        // not a sweep, it is a read of the contacts each platform already holds
        // (`Arena::hostile_in_weapons_range`), which is the same pass the
        // controller's `Approach` and `Engage` scoring makes and is bounded by the
        // contact store rather than by the field. And the `Alert` row's third
        // trigger — *a controller has entered `Approach`* — is the controller's own
        // state, which is now a thing that exists.
        let shot_fired = self.next_shot_seq > shots_before;
        let signals = TempoSignals {
            fire_intent_queued,
            contact_within_alert_range: self.contact_within_alert_range(),
            ai_approaching: self.reflex.any_approaching(),
            shot_fired,
            impact_scheduled: shot_fired,
            hostile_in_weapons_range: self.hostile_in_weapons_range(),
        };
        //
        // **16b cut M.2**: a match a substep already ended moves no tempo and
        // journals no `tempo` or `lagged` line. Cut W.2's rule is that nothing is
        // journalled after `match_ended`, and with the duration bound settled
        // inside the substep (16c `T-01`) a timeout is now such an end too: the
        // shot in flight that the deadline cut off would otherwise raise
        // `Engaged` a line after the match was over.
        let over = self.ended.is_some();
        let changed = if over {
            None
        } else {
            self.clock
                .evaluate(self.now_ms, &signals, &self.arena_cfg.tempo)
        };
        // One hash for every event this phase appends, taken after the tempo is
        // settled so that a `tempo` event carries the tempo it announces.
        let hash = state_hash(self);
        if let Some((from, to)) = changed {
            ledger_full |= !self.journal(self.now_ms, EventKind::Tempo { from, to }, hash);
        }
        // **16b cut I.3** (16c N10): a staff floor set or lifted since the last
        // pulse, once per change, beside the `tempo` line and under the same
        // hash. The floor is a view of the wake cadence and no hash folds it;
        // this line moves the ledger's sequence like every line (`hash.rs` item
        // 11). Not on a match a substep already ended: nothing is journalled
        // after `match_ended` (16b cut W.2), and the change is left untaken.
        if !over && let Some(period_ms) = self.clock.take_floor_change() {
            ledger_full |= !self.journal(self.now_ms, EventKind::TempoFloor { period_ms }, hash);
        }
        // One notice per pulse whichever half is behind: the clock absorbed more
        // wall time than the clamp allows (doc 16 §3.1 "Clock and lag"), or the
        // egress is backing up past `ledger.max_unflushed_chunks` (§3.1's egress
        // paragraph). `dt_ms` is the clamped `dt` the pulse actually ran with.
        let egress_lagged = self.ledger.lagged();
        if !over && (clock_lagged || egress_lagged) {
            ledger_full |= !self.journal(self.now_ms, EventKind::Lagged { dt_ms }, hash);
        }
        // **16b cut W.2**, the **backstop** for doc 16 §3.8's team elimination. Every
        // ordinary elimination is settled inside the substep the last unit went out of
        // action in (see `Arena::substep`), because that is what makes its timestamp a
        // fact of arena time rather than of the cadence; `settle_elimination` answers
        // `None` for a match that has already ended, so this arm fires only for a side
        // put out by something that is not a substep — a direct `destroy_unit` from a
        // test rig today, and whatever a later cut adds. It is **before** the duration
        // check, so a match whose last unit dies on the pulse the clock runs out is a
        // victory and not a timeout.
        //
        // **16b cut Q.7** put `Spent` beside it, in that order: a side eliminated
        // on the same pulse is an elimination with a winner, and a draw over the
        // top of it would lose the result.
        //
        // **16b cut O.1 adds no Hold backstop here, on purpose.** A Hold win is
        // settled by its own due event inside a substep (`DueKind::HoldSettles`),
        // and nothing outside a substep moves a pose or wrecks a unit on a
        // schedule the clock could miss: a staff `@arena/place` moves a pose that
        // the next substep's end reads, and a direct `destroy_unit` from a rig
        // is read by the next substep's presence settle and by the event's own
        // fresh read at its pop.
        let mut settled = self.settle_elimination();
        if settled.is_none() {
            settled = self.settle_spent();
        }
        if let Some(end) = settled {
            ledger_full |= !self.journal(
                self.now_ms,
                EventKind::MatchEnded { reason: end.reason },
                hash,
            );
        }
        // **16b cut M.2** (16c `T-01`): the duration bound is a due event now
        // (`DueKind::MatchDeadline`), settled inside the substep at its own
        // millisecond, so this check is the **backstop** and fires only on a pulse
        // that ran no substep — the opening pulse of a match whose
        // `max_duration_ms` is `0`, which AA.3's stalled-terminal proof relies on
        // and which the phase-4 drain never touches. On a pulse whose deadline the
        // due loop already settled, `ended` is set and this is skipped.
        if self.ended.is_none() && self.now_ms >= i64::from(self.match_cfg.max_duration_ms) {
            let reason = EndReason::Timeout;
            ledger_full |= !self.journal(self.now_ms, EventKind::MatchEnded { reason }, hash);
            self.ended = Some(MatchEnd {
                reason,
                t_ms: self.now_ms,
            });
        }
        // Doc 16 §4: at 100% of `ledger.max_bytes` the match is stopped with
        // `Timeout`. There is no event to append — the ledger is exactly what has
        // run out of room — so the end is recorded in the result and not in the
        // black box.
        if self.ended.is_none() && ledger_full {
            self.ended = Some(MatchEnd {
                reason: EndReason::Timeout,
                t_ms: self.now_ms,
            });
        }
        // The world cannot drain the egress and the match is **void** — nothing it
        // derived may be committed. `EndReason::LedgerOverflow` is A.1's declared
        // name for exactly this condition and the `void` event is what marks the
        // match void.
        //
        // Doc 16 §3.1: *past `ledger.max_unflushed_chunks` the match is stopped with
        // `Void`*. `Ledger::overflowed` is that bound exactly, and `Ledger::lagged`
        // (half of it) is the warning journalled before the match is lost; ruled at
        // the review of cut A.2, so the arena tolerates the documented depth and no
        // more.
        if self.ended.is_none() && self.ledger.overflowed() {
            let _ = self.journal(self.now_ms, EventKind::Void, hash);
            self.ended = Some(MatchEnd {
                reason: EndReason::LedgerOverflow,
                t_ms: self.now_ms,
            });
        }
        let ended = self.ended;
        if ended.is_some() {
            // The last chunk leaves with the match rather than with the next
            // pulse, which there is not going to be.
            self.ledger.flush();
        }
        let tempo = self.clock.tempo();
        let frames = self.frames(tempo);
        // **16b cut I.4** (16c N10): the staff picture, after the pilots' frames
        // and from the same committed state, while anybody watches — at most
        // once per pulse and at most once an arena second, and on the pulse the
        // match ended whatever second it is, so a watcher sees the end. The gate
        // is a view (`Arena::watched`, `Arena::last_observer_s`) that no hash
        // folds and nothing that steers reads, so a watched match is the same
        // match at every tempo; the one cost is this `bool` read on a pulse
        // nobody watches.
        let observer = if self.watched
            && (self.last_observer_s != Some(self.now_ms.div_euclid(1_000)) || ended.is_some())
        {
            self.last_observer_s = Some(self.now_ms.div_euclid(1_000));
            Some(self.observer())
        } else {
            None
        };
        StepOutput {
            ledger_chunks: self.ledger.take_chunks(),
            frames,
            tempo,
            next_due_ms: self.next_due_ms(wall_now_ms),
            lagged: self.clock.lagged() || self.ledger.lagged(),
            ended,
            substeps: ran,
            perception,
            reflex,
            // **16b cut N.2** (doc 16 §6): the last thing the pulse does, after the
            // frames, so a signal describes the world the pilots were just shown.
            // A match that ended this pulse asks for nothing: an order for a match
            // that is over is a provider call nobody will read.
            deliberation: if ended.is_some() {
                Vec::new()
            } else {
                self.take_deliberation_signals()
            },
            observer,
        }
    }

    /// **One substep** of doc 16 §3.1 phase 3, answering whether the ledger
    /// refused an event.
    ///
    /// (a) motion, in unit-id order, from the substep-start snapshot; (b) the index
    /// rebuilt; (c) arena time advanced and every due event at or before it
    /// applied. The events (a) and (c) produced are appended **last**, in that
    /// order — the `blocked` notices in unit-id order, then the due events in
    /// `(t_ms, seq)` order — against one hash taken at the substep's end.
    fn substep(
        &mut self,
        dt_ms: u32,
        perception: &mut PerceptionCounters,
        reflex: &mut ReflexCounters,
    ) -> bool {
        let mut notices: Vec<Notice> = Vec::new();
        // **16b cut F.2**: the stops this substep's motion produced, in unit-id
        // order, collected inside the borrow of `self.units` and staged after it.
        // A pilot's blocked notice is staged here and not from the journal scan
        // below the way every other one is; since 16b cut M.3 it fires on exactly
        // the edge the ledger's own `blocked` line does (`MotionOutcome::held_edge`),
        // at the same point in the same order, which is what the scan would have
        // given.
        let mut stops: Vec<(ObjId, crate::arena::motion::BlockedCause)> = Vec::new();
        // **16b cut Q.7**: the same stops again, with the cell, for the reflex
        // controller's own memory — the same edge once more (16b cut M.3). They
        // are collected here and applied below — after arena time has advanced,
        // so the timestamp a controller remembers is the one the `blocked` ledger
        // line carries, and before the due-event loop, so the `ReflexTick` of
        // this very substep already knows.
        let mut learned: Vec<(ObjId, (u16, u16), crate::arena::motion::BlockedCause)> = Vec::new();

        // **16b cut E.3** (16c `B-44`), before motion: a machine steering at a
        // goal with no route is steering at a direction nobody has checked, and
        // until this cut it only found that out at its own decide tick — once a
        // second of arena time, against eight substeps of driving. The pre-pass
        // asks the crossing's own question of the cell in front of every such
        // machine and orders the ones that would be refused to stop, through the
        // same door a pilot's `SPEED 0` goes through.
        //
        // **16b cut M.2**: neither it nor motion runs in a zero-length substep —
        // the phase-4 drain (`Arena::step`), which exists to pop what phase 4
        // scheduled at `now_ms` and nothing else. No time passes in it, so there
        // is no motion to integrate, and `integrate` at `dt 0` is not a pure
        // no-op: it clears the per-substep `blocked` flag. (Until 16b cut M.3 that
        // flag's rising edge was the `blocked` line, so a drain would have made
        // the next real substep's re-collision with the same bank a second line;
        // the line now fires on `held`'s edge, which only a refusal raises, and
        // the guard stands because there is still nothing to move.)
        if dt_ms > 0 {
            self.hold_routeless(reflex);
        }

        // ---- (a) motion ----
        if dt_ms > 0 {
            // **16b cut E.3**: the arena millisecond this substep *ends* at, and
            // the key that prices a traffic hold against it. Both are read before
            // the borrow below, because arena time itself is not advanced until
            // (c) — a hold opened here is stamped with the boundary the motion
            // belongs to, which is the same boundary the `blocked` notices below
            // are stamped with.
            let end_ms = self.now_ms.saturating_add(i64::from(dt_ms));
            let occupied_stop_ms = self.arena_cfg.occupied_stop_ms;
            self.index.rebuild(&self.definition.terrain, &self.units);
            // **16b cut U.2** (16c `T-29`): the jamming memo goes with the index it
            // was folded over — motion is about to move the poses it read.
            self.jam_fold.clear();
            let terrain = &self.definition.terrain;
            let index = &mut self.index;
            for slot in 0..self.units.len() {
                let Some(unit) = self.units.get_mut(slot) else {
                    continue;
                };
                let id = unit.id;
                let from = terrain.cell_of(&unit.pose);
                let outcome = integrate(unit, dt_ms, terrain, index, end_ms, occupied_stop_ms);
                if outcome.moved
                    && let Some(from) = from
                    && let Some(to) = terrain.cell_of(&unit.pose)
                    && from != to
                {
                    // Doc 16 §3.4 step 5's snapshot rule: the unit that integrates
                    // next must see where this one went, or two units enter one
                    // cell in one substep.
                    index.move_unit(id, from, to);
                }
                // **One edge for three readers** (16b cut M.3; 16c `T-04`,
                // `B-27`, §5's Q22): the ledger's `blocked` line, the pilot's
                // one notice and the controller's remembered stop all fire on
                // `UnitState::held`'s new edge — a stop at a cell or for a cause
                // the unit was not already held against — and on nothing else.
                // A tank that re-collides with the same bank every substep is
                // one stop however the pulse is cut; a bank and then an occupied
                // cell met inside the same cell after a turn are two. The
                // per-substep `blocked` flag journals nothing any more: its
                // rising edge was a function of the partition. `blocked_at` and
                // `blocked_by` are written together by `stop_at_boundary`, so the
                // cell and the cause are both in hand at the edge.
                if outcome.held_edge
                    && let (Some((x, y)), Some(cause)) = (outcome.blocked_at, outcome.blocked_by)
                {
                    notices.push((
                        0,
                        EventKind::Blocked {
                            unit: id,
                            x,
                            y,
                            cause,
                        },
                        Vec::new(),
                    ));
                    // 16b cut Q.7: the controller learns from the edge and not
                    // from the per-substep flag, which is cleared before the next
                    // tick could read it.
                    learned.push((id, (x, y), cause));
                    stops.push((id, cause));
                }
            }
        }
        for (unit, cause) in stops {
            self.stage_notice(unit, FrameNotice::Blocked { cause });
        }

        // ---- (b) the index over the positions motion committed ----
        self.index.rebuild(&self.definition.terrain, &self.units);
        // **16b cut U.2** (16c `T-29`): and the jamming memo is emptied with it.
        // From here to the next rebuild no pose moves, so a fold taken by the due
        // loop below stays true until a `StartupComplete` or an `Impact` moves a
        // pod's power or wreck — the two arms that clear it again.
        self.jam_fold.clear();

        // ---- (c) arena time, and every due event at or before it ----
        self.now_ms = self.now_ms.saturating_add(i64::from(dt_ms));
        for notice in &mut notices {
            // The `blocked` notices belong to the interval just integrated, which
            // ends here.
            notice.0 = self.now_ms;
        }
        // 16b cut Q.7: and so does what the controller remembers of them.
        for (unit, cell, cause) in learned {
            self.note_blocked(unit, cell, cause, self.now_ms);
        }

        // **16b cut X.1** (doc 16 §9 item 1): the heat the interval just
        // integrated shed, at the same boundary the motion belongs to. It is a
        // rate over **arena time** and not a thing that happens per pulse, which
        // is why it is here and not in phase 5 — and why the remainder is carried
        // rather than truncated (`crate::arena::heat`). It journals nothing: doc
        // 16 §4's event kinds are a closed list and heat is a cell, whose changes
        // are M32's own journal's to record.
        self.dissipate_heat(dt_ms, self.now_ms);
        // **16b cut M.2** (16c `T-01`): the duration bound's millisecond, once its
        // event has popped in this substep, and the loop's horizon with it.
        let mut deadline_hit: Option<i64> = None;
        // **16b cut M.2** (16c `T-83`): the held losses whose deadline popped in
        // this loop, each with how many journal entries preceded it, so the
        // staging scan below settles it at exactly its place in `(t_ms, seq)`
        // order — see the `LossSettles` arm.
        let mut settles: Vec<(usize, ObjId, ContactId, i64)> = Vec::new();
        // **16b cut O.1** (16c N11): the Hold win this loop confirmed, and its
        // millisecond — ended after the loop, before the two settles.
        let mut hold_won: Option<(Team, i64)> = None;
        // The horizon is the substep's end until the deadline pops, and the
        // deadline's own millisecond after: `pop_due` then answers only what
        // shares that millisecond (everything earlier has already been popped),
        // and the loop stops with every later event left in the heap.
        while let Some(event) = self.due.pop_due(deadline_hit.unwrap_or(self.now_ms)) {
            match event.kind {
                // **16b cut M.2** (16c `T-01`; doc 16 §3.8's `Timeout`): the match's
                // duration bound, at its own arena millisecond. The arm only
                // records the millisecond; the loop's horizon then shrinks to it,
                // so an event due at the deadline's own millisecond still lands —
                // an impact scheduled there resolves, and a kill there still wins,
                // because elimination is settled below before the timeout is — but
                // an impact, pass, share or tick due AFTER the deadline and at or
                // before this substep's end is NOT applied and journals nothing.
                // Cut W.2's rule is that nothing is journalled after the match is
                // over, and without the horizon a round landing after the deadline
                // turned a timeout into an elimination. What is left in the heap
                // is never applied: the match is over.
                DueKind::MatchDeadline => {
                    deadline_hit = Some(event.t_ms);
                }
                // **16b cut O.1** (16c N11; plan items 1, 15, 40): a sole
                // occupant's clock has run out. The win is awarded only when this
                // event is the clock that is still running — `held_by` names this
                // very `(team, since_ms)`, so an occupancy that was contested or
                // re-opened since leaves a stale event that fires into nothing —
                // **and a presence reading taken FRESH, here, at the pop** still
                // finds `team` alone. `held_by` was written at the previous
                // boundary; the fresh read is over the positions this substep's
                // motion committed and every impact this loop has applied so far,
                // so a holder wrecked since, or an enemy driven into the square
                // during this substep, voids the win. A confirmed win shrinks the
                // horizon exactly as the deadline's arm does, so nothing due after
                // its millisecond is applied or journalled.
                //
                // **The three true tie rules**, each pinned by a test:
                //
                // * `HoldSettles` is scheduled at `since_ms`, at least
                //   `MIN_HOLD_MS` (10 s) before it pops, so it carries a lower
                //   sequence than any impact due at the same millisecond (a round
                //   flies at most 2.56 km at 1,000 m/s) and pops first; and
                //   `MatchDeadline`, scheduled at check-out, is lower still;
                // * (1) **a holder wrecked at an EARLIER millisecond of the last
                //   substep voids the win**: the fresh read finds nobody;
                // * (2) **a holder wrecked at the hold's OWN millisecond still
                //   wins**: the impact pops after this arm, inside the shrunk
                //   horizon, is applied and journalled, and then the match ends as
                //   `hold` — the one moment the objective was met, it was met;
                // * (3) **a hold at the deadline's own millisecond beats
                //   `Timeout`**: the hold is settled before the timeout arm below,
                //   which asks `self.ended.is_none()`.
                DueKind::HoldSettles { team, since_ms } => {
                    if self.ended.is_none()
                        && self
                            .hold
                            .as_ref()
                            .is_some_and(|hold| hold.held_by == Some((team, since_ms)))
                        && self.hold_presence() == Some(team)
                    {
                        hold_won = Some((team, event.t_ms));
                        deadline_hit = Some(event.t_ms);
                    }
                }
                // **16b cut M.2** (16c `T-83`; doc 16 §3.6 "What a pilot sees"): a
                // held loss's hysteresis has run out. **The rule**: the loss is
                // said at the first substep boundary at or after the deadline
                // unless the platform re-detected the contact at or before it. A
                // sensor pass landing exactly on the deadline is applied before
                // the expiry (it was scheduled earlier, so its sequence is lower),
                // and a mesh share landing exactly on it after (it is scheduled
                // by that pass), which is the one millisecond the two arms can
                // differ on and is the same at every cadence.
                //
                // It is **settled in the staging scan**, not here, at the position
                // the event popped at. A re-detection's `detected` is journalled
                // into `notices` by its pass and only reaches
                // `Arena::stage_pilot_notice` — which is where a held loss is
                // cancelled — in the scan below, after this whole loop; removing
                // the entry here would say a loss that a pass earlier in this same
                // substep had already cancelled. Recording the position keeps the
                // two in `(t_ms, seq)` order, and it also means every write to
                // `held_losses` happens in the scan, after the substep's hash, so
                // each is first folded at *k + 1* (the `spent_said` rule).
                DueKind::LossSettles { viewer, contact } => {
                    settles.push((notices.len(), viewer, contact, event.t_ms));
                }
                // Doc 16 §3.3 "Startup": `match.startup_ms` after `STARTUP` was
                // accepted. `resolve_startup` already journalled the transition
                // into `starting`.
                //
                // **16b cut Q.1** gave the far end its own event. Doc 16 §4's
                // kind list had no `power_online`, so the moment a tank became
                // driveable was the one transition in the engine nothing recorded
                // and nobody was told about: the owner's first hand-played match
                // found it by typing `HEADING` until one of them stopped being
                // refused. The line is journalled at the **event's** timestamp
                // and only on the real flip, so a startup overtaken by a wreck
                // writes nothing, and `stage_pilot_notice` projects the pilot's
                // `Power online.` off it exactly as every other notice is a
                // projection of a journalled event.
                DueKind::StartupComplete { unit } => {
                    if let Some(state) = self.unit_mut(unit)
                        && state.power == Power::Starting
                    {
                        state.power = Power::Online;
                        notices.push((event.t_ms, EventKind::PowerOnline { unit }, Vec::new()));
                    }
                    // **16b cut U.2** (16c `T-29`): a pod that has just come online
                    // radiates from this millisecond, so every fold taken earlier in
                    // this substep is stale. Cleared whether or not the flip
                    // happened: cheaper to ask again than to reason about it.
                    self.jam_fold.clear();
                }
                // Doc 16 §4: every unit, every `ledger.summary_ms` of arena time,
                // in every tempo. The next one is scheduled from **this one's
                // timestamp** and not from `now_ms`, so the cadence is a function
                // of arena time alone and cannot drift with the substep boundary
                // the summary happened to land on. It is rescheduled even when the
                // 90% bound has silenced it, so summaries resume if the bound is
                // raised.
                DueKind::PositionSummary => {
                    if self.ledger.summaries_enabled() {
                        notices.push((
                            event.t_ms,
                            EventKind::PositionSummary {
                                units: self.position_summary(),
                            },
                            Vec::new(),
                        ));
                    }
                    let cadence = i64::from(self.ledger.summary_ms().max(1));
                    self.due
                        .schedule(event.t_ms.saturating_add(cadence), DueKind::PositionSummary);
                }
                // **16b cut S.1** (doc 16 §3.6): one unit's pass, applied at the
                // event's own timestamp against the positions this substep
                // committed. The next pass is scheduled from **this one's
                // timestamp** and not from `now_ms`, exactly as the position
                // summary above is, so the cadence is a function of arena time
                // alone and cannot drift with the substep boundary the pass
                // happened to land on.
                //
                // A pass that leaves the unit holding **anything the mesh may
                // carry** — a classified contact its own sensors saw inside the
                // time to live, which is 16b cut F.3's two share rules and is
                // written once in `ShadowContact::shareable`; `Arena::sensor_pass`'s
                // head has the argument — schedules a `Share` at the *same*
                // timestamp: the heap orders
                // `(t_ms, seq, kind)` and the share is inserted with a later
                // sequence, so it is applied after the pass, in this same substep,
                // and the mesh never lags a pulse behind the tracks it carries. It
                // schedules nothing itself, so the loop terminates.
                DueKind::SensorPass { unit, pass_seq } => {
                    let shared =
                        self.sensor_pass(unit, pass_seq, event.t_ms, perception, &mut notices);
                    if shared {
                        self.due.schedule(event.t_ms, DueKind::Share { unit });
                    }
                    // **16b cut F.3**: rescheduled for every unit still in the
                    // match, not only for one that carries a sensor. The pass is
                    // where contacts expire, and a platform with no sight of its
                    // own still holds what the mesh handed it
                    // (`Arena::schedule_first_passes` has the argument).
                    if self.unit(unit).is_some() {
                        let cadence = i64::from(self.sensors_cfg.pass_ms.max(1));
                        self.due.schedule(
                            event.t_ms.saturating_add(cadence),
                            DueKind::SensorPass {
                                unit,
                                pass_seq: pass_seq.saturating_add(1),
                            },
                        );
                    }
                }
                // **16b cut S.1** (doc 16 §3.6 step 5, W6).
                DueKind::Share { unit } => {
                    self.share_pass(unit, event.t_ms, perception, &mut notices);
                }
                // **16b cut W.1** (doc 16 §3.5 "Scheduled impact"): the shot arrives
                // at its own timestamp, against the positions this substep
                // committed, which is what makes a moving target's escape a fact of
                // arena time and not of the pulse rate. **Cut W.2** gave it its body
                // — the four re-validations, the draw, the damage plan and the
                // consequences (`crate::arena::damage`) — and the draw is keyed on the
                // shot's sequence, so which pulse the round lands in cannot change
                // whether it hits.
                DueKind::Impact { shot_seq } => {
                    self.resolve_impact(shot_seq, event.t_ms, &mut notices);
                    // **16b cut U.2** (16c `T-29`): a round can wreck the hull a
                    // pod rides on, which turns its power off and stops it
                    // radiating (`Arena::destroy_unit`), so a fold taken before
                    // it may count a pod that is gone.
                    self.jam_fold.clear();
                }
                // **16b cut W.1**: the weapon's cycle has run down. It changes no
                // state — readiness is `ready_at_ms <= now_ms`, a comparison — and
                // it is scheduled for `next_due_ms`: it makes the arena pulse at the
                // first substep boundary at or after the millisecond the weapon
                // comes ready (16b cut U.1: within one substep of it, 250 ms on the
                // shipped content), so the pilot's frame says "ready" then rather
                // than at the next period boundary.
                DueKind::WeaponReady { .. } => {}
                // **16b cut N.1** (doc 16 §6): one AI unit's decision, applied at
                // its own arena timestamp against the positions this substep
                // committed — which is what makes the controller rate-independent
                // and is why it is a due event and not pulse phase 4
                // (`crate::arena::reflex`'s module head has the argument).
                //
                // A match that ended inside this substep decides nothing more: doc
                // 16 §3.8's elimination is settled below, and an order given after
                // `match_ended` would be a line in the black box about a match that
                // was already over.
                //
                // The one hash, taken **lazily**: the discrete actions a tick
                // resolves journal through `Arena::event_hash`, and a fresh hash per
                // action is the 5 ms a pulse cut W.1 measured. A substep with no
                // reflex work pays nothing for this.
                //
                // **16b cut M.2** (16c `T-02`): the tick's actions are resolved at
                // `event.t_ms`, the tick's own millisecond, and not at `now_ms`,
                // which this substep has already advanced to its end. Anything a
                // tick schedules at or before `now_ms` — a beam's zero-flight
                // impact — is popped by this same loop in `(t_ms, seq)` order:
                // **the same-substep property**. (A `WeaponReady` could be one only
                // with a `cycle_ms` shorter than a substep, which W14's 1,000 ms
                // floor rules out for shipped content.)
                DueKind::ReflexTick { unit, tick_seq } => {
                    if self.ended.is_none() {
                        // 16b cut M.3 (`T-23`): opened, not taken — the first
                        // line a tick of this substep journals takes it.
                        if self.action_hash == ActionHash::Outside {
                            self.action_hash = ActionHash::Pending;
                        }
                        if self.reflex_tick(unit, event.t_ms, reflex) {
                            let cadence = i64::from(self.arena_cfg.reflex.decide_ms.max(1));
                            self.due.schedule(
                                event.t_ms.saturating_add(cadence),
                                DueKind::ReflexTick {
                                    unit,
                                    tick_seq: tick_seq.saturating_add(1),
                                },
                            );
                        }
                    }
                }
            }
        }
        // The lazy hash is scratch for this substep's due events exactly as phase
        // 4's is scratch for that phase; it never leaves the substep it was taken
        // in.
        self.action_hash = ActionHash::Outside;

        // **16b cut W.2** (doc 16 §3.8): team elimination is settled **here**, at the
        // substep the last unit of a side went out of action in, and not at the end of
        // the pulse. The difference is the whole of rate independence: the wreck is
        // applied at a due event's own timestamp inside a substep, so the moment the
        // side was eliminated is a fact of arena time — and a `match_ended` stamped at
        // the pulse boundary instead would carry a different timestamp at every
        // cadence.
        //
        // **16b cut Q.7** settles `Spent` here too, and after it: the last round
        // in flight lands as a due event inside this very substep, so the substep
        // that resolves it is the substep in which nobody can shoot any more.
        //
        // **16b cut O.1** (16c N11) puts the objective in front of both. First
        // who holds the key spot, over the poses motion committed and every
        // impact the loop above applied (`Arena::settle_hold_presence`, which
        // opens or drops the clock); then a win the `HoldSettles` arm confirmed,
        // at its own millisecond. The two settles below then answer `None` on
        // the ended match — elimination is *settled second*, the owner's default
        // — and so does the deadline's timeout arm, which is why a hold at the
        // deadline's own millisecond is a hold.
        self.settle_hold_presence();
        if let Some((winner, t_ms)) = hold_won {
            let reason = EndReason::Hold { winner };
            notices.push((t_ms, EventKind::MatchEnded { reason }, Vec::new()));
            self.ended = Some(MatchEnd { reason, t_ms });
        }
        let mut settled = self.settle_elimination();
        if settled.is_none() {
            settled = self.settle_spent();
        }
        if let Some(end) = settled {
            notices.push((
                self.now_ms,
                EventKind::MatchEnded { reason: end.reason },
                Vec::new(),
            ));
        }
        // **16b cut M.2** (16c `T-01`): the duration bound, at the deadline's own
        // millisecond, after the two settles above and only when neither ended
        // the match — an elimination or a `Spent` in the same substep wins, the
        // order phase 5's backstop has always had.
        if settled.is_none()
            && self.ended.is_none()
            && let Some(t_ms) = deadline_hit
        {
            let reason = EndReason::Timeout;
            notices.push((t_ms, EventKind::MatchEnded { reason }, Vec::new()));
            self.ended = Some(MatchEnd { reason, t_ms });
        }

        // ---- the substep's record ----
        let hash = state_hash(self);
        self.substep_hashes.push(hash);
        let mut refused = false;
        // **16b cut F.2, narrowed by 16b cut D.4 (`T-08`)**: which impact is being
        // resolved, and it now serves `EventKind::Miss` **alone**.
        //
        // A `miss` line carries the shot's sequence and no shooter, and it may not
        // gain one: that would change the bytes of every `miss` line ever written.
        // But `EventKind::Impact` immediately precedes exactly one `hit` or `miss`
        // for the same shot and carries both, so the scan reads the impact and its
        // outcome as the pair they are — and **clears the pair the moment that
        // outcome has been staged**, which is what makes the read a read of this
        // round rather than of whatever round went last.
        //
        // Cut F.2 left it set for the rest of the substep, and two later arms —
        // the gunner's `target_section_lost` and his kill line — read it after the
        // pair had resolved. Anything that destroyed a section or a hull later in
        // the same substep and was not that round was credited to it, and since
        // cut Q.2 one of those arms writes hashed contact state. Both now read
        // `by` off their own event, which carries the shooter where a round did
        // it and `None` where nothing did.
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        // 16b cut M.2: the held losses that came due, each settled just before the
        // first journal entry that popped after it (`LossSettles`' arm).
        let mut settles = settles.into_iter().peekable();
        // **16b cut M.2** (the M.2 review): and the held losses whose window
        // closes inside THIS substep — staged by this very scan with a deadline at
        // or before `now_ms`, which only a `[tacsim.sensors].pass_ms` shorter than
        // the substep can make (config allows 50 against a 250 ms substep). A due
        // event for one would be scheduled into the past: it would pop a boundary
        // late, and at a pulse's end the phase-4 drain would run a substep for it
        // at one cadence that another cadence never runs — two hash sequences.
        // So it never reaches the heap: it is settled here, before the first
        // entry of this scan whose millisecond is past the deadline (an entry AT
        // the deadline is a re-detection "at or before" it and goes first), which
        // is exactly the place `(t_ms, seq)` order would have popped it — its
        // event would have been scheduled after every event this loop popped, so
        // it would sort after all of them on its own millisecond.
        let mut closing: Vec<(i64, ObjId, ContactId)> = Vec::new();
        // Everything to settle before the next entry, ordered by millisecond and,
        // on one millisecond, a due event's settle before a closing window's.
        let mut due_now: Vec<(i64, u8, ObjId, ContactId)> = Vec::new();
        for (index, (t_ms, kind, draws)) in notices.into_iter().enumerate() {
            while let Some((_, viewer, contact, at)) =
                settles.next_if(|(before, ..)| *before <= index)
            {
                due_now.push((at, 0, viewer, contact));
            }
            closing.retain(|&(at, viewer, contact)| {
                if at < t_ms {
                    due_now.push((at, 1, viewer, contact));
                    false
                } else {
                    true
                }
            });
            self.settle_held_losses(&mut due_now);
            // 16b cut S.2: the pilot's notice and the ledger's event come off the
            // same journal entry, in the same order, so a line a pilot read is a
            // line the black box can be searched for.
            if let Some(window) = self.stage_pilot_notice(t_ms, &kind, &mut in_flight) {
                closing.push(window);
            }
            refused |= !self.journal_drawn(t_ms, kind, draws, hash);
        }
        due_now.extend(
            settles
                .map(|(_, viewer, contact, at)| (at, 0, viewer, contact))
                .chain(
                    closing
                        .into_iter()
                        .map(|(at, viewer, contact)| (at, 1, viewer, contact)),
                ),
        );
        self.settle_held_losses(&mut due_now);
        refused
    }

    /// Settle every held loss in `due` in `(t_ms, kind, viewer, contact)` order
    /// and leave it empty (16b cut M.2) — the one ordering both of the staging
    /// scan's sources of expiries are put through.
    fn settle_held_losses(&mut self, due: &mut Vec<(i64, u8, ObjId, ContactId)>) {
        due.sort_unstable();
        for (at, _, viewer, contact) in due.drain(..) {
            self.settle_held_loss(viewer, contact, at);
        }
    }

    /// **Say one held loss whose hysteresis ran out** (16b cut M.2, 16c `T-83`;
    /// doc 16 §3.6 "What a pilot sees"), for the [`DueKind::LossSettles`] that
    /// came due at `at_ms`.
    ///
    /// Only when the entry is still there **with that deadline**: a loss the
    /// platform re-detected at or before it was taken out by the `detected` arm
    /// of `Arena::stage_pilot_notice`, and one lost again since carries a later
    /// deadline and its own event, so either way this event fires into nothing.
    /// A held loss whose platform has left the match is dropped rather than said:
    /// there is nobody it is a fact about any more (the rule the pulse-end sweep
    /// this replaced had).
    fn settle_held_loss(&mut self, viewer: ObjId, contact: ContactId, at_ms: i64) {
        let Some((at, designator)) = self.held_losses.get(&(viewer, contact)).copied() else {
            return;
        };
        if at != at_ms {
            return;
        }
        self.held_losses.remove(&(viewer, contact));
        if self.unit(viewer).is_none() {
            return;
        }
        // 16b cut Q.1: the designator travels with the deadline, because the
        // contact row the name could have been read off went at the expiry.
        self.stage_notice(
            viewer,
            FrameNotice::Contact {
                kind: NoticeKind::Lost,
                contact: Named {
                    id: contact,
                    designator,
                },
                class: Classification::Unknown,
                was: None,
            },
        );
    }

    /// The world asked the arena to stop (doc 16 §3.1's `Stop`): the reason is
    /// recorded, the match ends, and the ledger's tail leaves with it.
    ///
    /// The reasons journal different things, which is doc 16 §4's kind list read
    /// literally: `Abort` is an `aborted` event, `Timeout` is
    /// `match_ended{timeout}`, and `Void` is a `void` event — the one that says
    /// nothing the match derived may be committed.
    ///
    /// **16b cut D.3 (`B-33`) added two ends that are ends**: a concede and an
    /// unattended match both journal `match_ended` with their own reason, exactly
    /// as a timeout does, because both are matches that ran to a conclusion and
    /// whose cells the world commits. A concede names the side that did **not**
    /// give up, so the reader of the black box does not have to invert anything.
    ///
    /// **16b cut I.3** (16c N10: *journalled always, announced never*): a staff
    /// `Place` among the stopping pulse's `intents` — one the task buffered
    /// before the `Stop` arrived — reaches no phase 2 and no phase 4, so it is
    /// answered here, `place_refused` with the reason `stopped`, BEFORE the stop's
    /// own line (nothing follows `match_ended`, 16b cut W.2) and under the same
    /// hash, the stop pulse's one moment. Every other order in the pulse is
    /// dropped silently as it always was: the match is over and nobody is told.
    fn stopped(&mut self, wall_now_ms: i64, stop: StopReason, intents: &[Intent]) -> StepOutput {
        let hash = state_hash(self);
        for intent in intents {
            if let IntentKind::Place { x, y } = intent.kind {
                let _ = self.journal(
                    self.now_ms,
                    EventKind::PlaceRefused {
                        unit: intent.unit,
                        x,
                        y,
                        reason: SmolStr::new_static(PLACE_STOPPED),
                    },
                    hash,
                );
            }
        }
        let (kind, reason) = match stop {
            StopReason::Abort => (EventKind::Aborted, EndReason::Abort),
            StopReason::Timeout => (
                EventKind::MatchEnded {
                    reason: EndReason::Timeout,
                },
                EndReason::Timeout,
            ),
            StopReason::Void => (EventKind::Void, EndReason::Void),
            // 16b cut D.3: the side that conceded is the side that lost.
            StopReason::Concede { by } => {
                let reason = EndReason::Conceded { winner: by.other() };
                (EventKind::MatchEnded { reason }, reason)
            }
            StopReason::Unattended => (
                EventKind::MatchEnded {
                    reason: EndReason::Unattended,
                },
                EndReason::Unattended,
            ),
        };
        let _ = self.journal(self.now_ms, kind, hash);
        let ended = MatchEnd {
            reason,
            t_ms: self.now_ms,
        };
        self.ended = Some(ended);
        self.ledger.flush();
        StepOutput {
            ledger_chunks: self.ledger.take_chunks(),
            frames: Vec::new(),
            tempo: self.clock.tempo(),
            next_due_ms: wall_now_ms,
            lagged: self.clock.lagged() || self.ledger.lagged(),
            ended: Some(ended),
            substeps: 0,
            perception: PerceptionCounters::default(),
            reflex: ReflexCounters::default(),
            deliberation: Vec::new(),
            // 16b cut I.4: a stopped pulse sends no pilot a frame and no
            // watcher a picture either; the world's `finish` clears every
            // watcher when the check-in lands.
            observer: None,
        }
    }

    /// Append one event, answering whether it was recorded. A `false` is the
    /// ledger's byte bound (doc 16 §4), which phase 5 turns into the end of the
    /// match; it is never an error the caller has to handle mid-substep, because a
    /// black box out of tape is not a reason to lose the pulse in progress.
    fn journal(&mut self, t_ms: i64, kind: EventKind, hash: u64) -> bool {
        self.ledger.append(t_ms, kind, Vec::new(), hash).is_ok()
    }

    /// The same, with the draws doc 16 §4 asks an event to carry *where they
    /// occurred* — cut S.1's `classified` was the first event that had one, and
    /// 16b cut M.3's `hit` is the first with two (16c `T-07`).
    fn journal_drawn(
        &mut self,
        t_ms: i64,
        kind: EventKind,
        draws: Vec<(String, u64)>,
        hash: u64,
    ) -> bool {
        self.ledger.append(t_ms, kind, draws, hash).is_ok()
    }

    /// One frame per pilot whose view changed, in pilot-id order (doc 16 §3.1's
    /// latest-wins slots).
    ///
    /// Two passes on purpose: the first reads the units, the second writes the
    /// remembered frames. A pilot seated on a unit this arena does not hold gets
    /// nothing rather than an empty frame.
    fn frames(&mut self, tempo: Tempo) -> Vec<(ObjId, Frame)> {
        let mut candidates: Vec<(ObjId, Frame)> = Vec::with_capacity(self.pilots.len());
        for pilot in &self.pilots {
            let Some(unit) = self.unit(pilot.unit) else {
                continue;
            };
            let lock = unit.lock;
            // 16b cut Q.2: the scope and the vocabularies its readings are in,
            // built in one walk (`Arena::frame_scope`).
            let (contacts, scan_tables) = self.frame_scope(unit.id);
            // **16b cut W.1, re-shaped by cut F.2**: the lock's own target, range
            // and cover. Cut W.1 needed only the range — the band each weapon's
            // lock falls in — and read it off the contact list the frame already
            // carries; cut F.2 resolves **each weapon's** own chance, which needs
            // the target and one trace. It is one trace per *seated* platform per
            // pulse, which is what `lock_p_permille` already cost.
            let aim = self.lock_aim(unit.id);
            candidates.push((
                pilot.pilot,
                Frame {
                    unit: unit.id,
                    team: unit.team,
                    // 16b cut Q.1: the platform's own public name, and the cell
                    // it is standing in.
                    callsign: unit.callsign,
                    cell: self.definition.terrain.cell_of(&unit.pose),
                    pose: unit.pose.clone(),
                    speed_cms: unit.speed_cms,
                    power: unit.power,
                    // 16b cut F.2: the condition, not the substep's flag.
                    held: unit.held.is_some(),
                    // 16b cut D.2: the pilot's own two limits, so the cockpit can
                    // refuse an order to a dead tank at once and say why.
                    wreck: unit.limits.wreck,
                    immobile: unit.limits.motion_disabled,
                    tempo,
                    // **16b cut D.1**: the nominal reach of the platform's own
                    // sight, `max_range_cells × cell_cm`, and `0` for a hull
                    // that carries no sensor. The same product
                    // `Arena::detections` starts from, unscaled — see
                    // [`Frame::sight_cm`]. **16b cut E.1** made it one product
                    // in one place ([`UnitState::sight_cm`]), because the
                    // machine's own fighting reach is priced off it too.
                    sight_cm: unit.sight_cm(self.definition.terrain.cell_cm()),
                    contacts,
                    scan_tables,
                    // Per platform, so two pilots in one tank are both told.
                    notices: self
                        .pending_notices
                        .get(&unit.id)
                        .cloned()
                        .unwrap_or_default(),
                    lock,
                    weapons: self.frame_weapons(unit.id, aim.as_ref()),
                    // 16b cut F.2: doc 16 §5's status block, off the same M32
                    // cells the impact writes.
                    sections: self.frame_sections(unit.id),
                    // 16b cut X.1: two integers off the unit's own cells, and
                    // `None` for a platform that carries no heat at all.
                    heat: match (unit.heat_now(), unit.heat_max()) {
                        (Some(heat), Some(max)) => Some(FrameHeat { heat, max }),
                        _ => None,
                    },
                    // 16b cut D.4: the platform's own receiver, as its last
                    // sensor pass left it.
                    receiver: unit.receiver,
                    // **16b cut O.2**: the pod's switch, for a platform that
                    // carries a pod.
                    ecm: unit.jammer.map(|_| unit.ecm_on),
                    // 16b cut Y.4: taken in the second pass below, which may
                    // write the arena.
                    catchup: None,
                },
            ));
        }
        // One pulse's worth, and no more: a platform nobody is seated on
        // accumulates nothing.
        self.pending_notices.clear();
        candidates.sort_by_key(|(pilot, _)| *pilot);
        // 16b cut U.2 (16c `T-30`): the cadence the stale clause is read against,
        // as `Frame::same_as` compares an age.
        let pass_ms = i64::from(self.sensors_cfg.pass_ms.max(1));
        let mut changed: Vec<(ObjId, Frame)> = Vec::new();
        for (pilot, mut frame) in candidates {
            // **16b cut Y.4**: the catch-up report a reclaim left this pilot,
            // once — taken here, so it rides exactly one frame, and that frame
            // is always sent (`Frame::same_as`).
            frame.catchup = self.pending_catchup.remove(&pilot);
            let told = !frame.notices.is_empty();
            if told
                || self
                    .last_frames
                    .get(&pilot)
                    .is_none_or(|last| !last.same_as(&frame, pass_ms))
            {
                // The remembered copy carries no notice: `CONTACTS` and `VIEW`
                // re-render it, and a notice said twice is a notice said wrong.
                // 16b cut Y.4: nor a catch-up, for the same reason.
                let mut remembered = frame.clone();
                remembered.notices.clear();
                remembered.catchup = None;
                self.last_frames.insert(pilot, remembered);
                changed.push((pilot, frame));
            }
        }
        changed
    }

    /// **Stage the pilot notice one journalled perception event owes**, and apply
    /// [`FrameNotice`]'s hysteresis (16b cut S.2; doc 16 §3.6 "What a pilot sees").
    ///
    /// Called with every event the pass journalled, so the pilot's lines are a
    /// projection of the black box's own record and the two cannot disagree. Three
    /// arms and one rule each:
    ///
    /// * `detected` — an acquisition, **unless** a held loss for the same
    ///   `(platform, contact)` is still inside its window, in which case the
    ///   contact merely flickered: the held loss is cancelled and nothing is said
    ///   — unless the row came back under another name, which is said as a
    ///   regression (16b cut D.4) or, for a mesh insert that arrived told apart,
    ///   as an identification (16b cut M.2).
    /// * `classified` — always said; being told apart is not something that
    ///   flickers (the pass keeps a classification across a gap by design).
    /// * `lost` — held for one further pass rather than said, which is the whole
    ///   hysteresis; a `DueKind::LossSettles` at the deadline is what eventually
    ///   says it (16b cut M.2 — a sweep at the end of every pulse said it until
    ///   then, which made the end of the window a pulse boundary).
    ///
    /// Answers `Some((deadline, viewer, contact))` for a `lost` whose window
    /// already closes at or before `now_ms` — only a pass shorter than the
    /// substep does that — so the staging scan settles it itself rather than a
    /// due event in the past; `None` for everything else.
    fn stage_pilot_notice(
        &mut self,
        t_ms: i64,
        kind: &EventKind,
        in_flight: &mut Option<(ObjId, ObjId)>,
    ) -> Option<(i64, ObjId, ContactId)> {
        let window_ms = i64::from(self.sensors_cfg.pass_ms.max(1));
        match *kind {
            EventKind::Detected {
                viewer, contact, ..
            } => {
                if let Some((_, was)) = self.held_losses.remove(&(viewer, contact)) {
                    // **16b cut D.4** (`B-20`; 16c §5 Q16). A loss cancelled
                    // inside the hysteresis window is still silent when the row
                    // comes back under the **same** name — an `X2` that flickered
                    // behind a treeline, or any contact under
                    // `designate_on_detection` — which is cut Q.1's rule intact.
                    //
                    // It is **not** silent when the name moved. A contact that
                    // had been told apart comes back `Unknown`, so its row reads
                    // `X2` where it read `BA`, and `LOCK BA` answers
                    // `No such contact.` for three seconds with nothing to say
                    // why. That is a regression the pilot has to be told about,
                    // and it is one line.
                    //
                    // The old name is quoted and never learned: `resolve_designator`
                    // is untouched, because accepting `BA` for this row would say
                    // BA is still out there — which is exactly what doc 16 §3.6's
                    // one byte-identical refusal exists to hide (16c §6).
                    //
                    // The test is **the row's own class** and not the name alone,
                    // because the name can move the other way: `Arena::share_pass`
                    // journals a `detected` for a mesh insert and that row arrives
                    // `Classified`, so a hold lost as `X2` and handed back over the
                    // mesh inside the window reads `BA` here. That is a contact
                    // becoming *more* identified, and saying
                    // `X2 is unidentified again: BA.` about it would be the
                    // sentence backwards. A regression is a row that is `Unknown`
                    // now, which also makes the notice's own `class` true by
                    // construction.
                    let named = self.name_of(viewer, contact);
                    let regressed = self
                        .contacts
                        .resolve(viewer, contact)
                        .is_some_and(|held| held.class == Classification::Unknown);
                    // **16b cut M.2** (16c `B-74`) says the third case, which
                    // D.4's rule left silent: the name moved and the row is NOT
                    // `Unknown` — a hold lost as `X1` and handed back over the
                    // mesh inside the window, already told apart, so it reads
                    // `BA`. Before D.4 the pilot read `Contact X1 identified: BA,
                    // blue.` there and after it nothing, so a row renamed itself
                    // under the pilot's eyes. It is the `classified` notice with
                    // the old name, exactly as a share that classifies a
                    // still-held row says it, with the class read off the same
                    // `resolve`. No new state: the name and the class were both
                    // already in hand.
                    //
                    // **Only a mesh insert that arrived told apart**, which is
                    // 16c `B-74`'s own case, read off the row the scan sees at
                    // the end of the substep: `numbered` false (the row came over
                    // the mesh, not from this platform's own pass) and
                    // `Classified`. Anything else stays silent here, because
                    // anything else is said elsewhere or is not true: a row this
                    // platform's own pass re-opened (`numbered`) and a share then
                    // classified later in the same substep is said by that
                    // share's own `classified` line, with this very `was`, and
                    // saying it here too would be the sentence twice; and a row
                    // that is already gone again by the scan (a stale mesh copy
                    // the viewer's own pass expired) has no class to announce and
                    // would read `identified: X0, unknown`.
                    let identified = self
                        .contacts
                        .resolve(viewer, contact)
                        .filter(|held| {
                            !held.numbered
                                && matches!(held.class, Classification::Classified { .. })
                        })
                        .map(|held| held.class);
                    if regressed && named.designator != was {
                        self.stage_notice(
                            viewer,
                            FrameNotice::Contact {
                                kind: NoticeKind::Unidentified,
                                contact: named,
                                class: Classification::Unknown,
                                was: Some(was),
                            },
                        );
                    } else if let Some(class) = identified
                        && named.designator != was
                    {
                        self.stage_notice(
                            viewer,
                            FrameNotice::Contact {
                                kind: NoticeKind::Classified,
                                contact: named,
                                class,
                                was: Some(was),
                            },
                        );
                    }
                    return None;
                }
                self.stage_notice(
                    viewer,
                    FrameNotice::Contact {
                        kind: NoticeKind::Acquired,
                        contact: self.name_of(viewer, contact),
                        // **16b cut D.4** (`T-19`): the class the row actually
                        // carries, which is `Classified` for a mesh arrival —
                        // the acquisition and the classification are one moment
                        // there, and the notice used to say `Unknown` about a row
                        // the same substep had already told apart. `notice_line`
                        // does not read it; the notice object's consumers may.
                        class: self
                            .contacts
                            .resolve(viewer, contact)
                            .map_or(Classification::Unknown, |held| held.class),
                        was: None,
                    },
                );
            }
            EventKind::Classified {
                viewer,
                contact,
                target,
                team,
            } => {
                // **16b cut Q.1**: what the row was called a moment ago, so the
                // rename is one sentence and not two rows a pilot has to match
                // up. `None` for a contact that was never provisional — a mesh
                // arrival, or a match run with `designate_on_detection` on — and
                // the renderer then says nothing at all, because the acquisition
                // already read `New contact RB: …`.
                //
                // **16b cut D.4** (`B-21`) fixed where "never provisional" is
                // read from. It was the **book**, which is permanent, so a target
                // this platform numbered in an EARLIER hold and now received over
                // the mesh — already classified — printed `New contact BA: …` and
                // at once `Contact X2 identified: BA, blue.`, a rename of a row
                // the pilot is not looking at. It is read off **this hold**
                // (`ShadowContact::numbered`) now: a mesh insert says nothing
                // after its acquisition, and a share that classifies a row this
                // platform's own sensors opened still says the rename.
                let was = if self.sensors_cfg.designate_on_detection {
                    None
                } else if self
                    .contacts
                    .resolve(viewer, contact)
                    .is_some_and(|held| held.numbered)
                {
                    self.designators
                        .provisional(viewer, target)
                        .map(crate::arena::state::Designator::Provisional)
                } else {
                    None
                };
                self.stage_notice(
                    viewer,
                    FrameNotice::Contact {
                        kind: NoticeKind::Classified,
                        contact: self.name_of(viewer, contact),
                        class: Classification::Classified { team },
                        was,
                    },
                );
            }
            // **16b cut Q.1**: the far end of `STARTUP`, said to the platform's
            // own pilots and to nobody else.
            EventKind::PowerOnline { unit } => {
                self.stage_notice(unit, FrameNotice::PowerOnline);
            }
            // **16b cut D.4** (`B-22`): the platform's own receiver, said to its
            // own pilots and to nobody else. One sentence per transition.
            EventKind::Receiver { unit, degraded } => {
                self.stage_notice(unit, FrameNotice::Receiver { degraded });
            }
            // **16b cut F.2**, the four combat arms. Each is produced where its
            // event is produced, at that event's own arena timestamp, so the list
            // a scripted run gives a pilot is the same at every cadence.
            EventKind::Impact { unit, target, .. } => *in_flight = Some((unit, target)),
            EventKind::Hit {
                unit,
                target,
                shot_seq,
                ref section,
                ref facet,
                armor_left,
                internals_left,
                ..
            } => {
                // **16b cut D.2** (`B-15`): whether this round found a section
                // that was already destroyed. The scratch is filled by
                // `apply_hit` in this very substep and is drained here, so it
                // never survives the substep it was filled in
                // (`Arena::spent_hits`).
                let spent = self.spent_hits.remove(&shot_seq);
                if spent {
                    // Pruned **on touch** and against the event's own timestamp,
                    // never by a per-pulse sweep: a sweep would make the table a
                    // function of the cadence, which the cross-tempo property
                    // refuses.
                    self.spent_said
                        .retain(|_, at| t_ms.saturating_sub(*at) < SPENT_NOTICE_MS);
                }
                // The shooter sees the round land, and nothing about what it did:
                // a gunner reads the strike, not the target's internals.
                //
                // **16b cut Q.2** gave it the one word that is not the target's
                // internals: which section the round found. The gate is
                // `classified` and not merely `held`, because a section id is a
                // chassis vocabulary — see `FrameNotice::Hit`'s `section`.
                let held = self.named_target(unit, target);
                let classified = held.is_some() && self.target_classified(unit, target);
                let named_section = classified.then(|| section.clone());
                // **16b cut D.2**: the gunner who has told the target apart can
                // tell a spent section from a live one, so his line says so — and
                // is therefore rate-limited. The gunner who cannot is told
                // exactly what he was told before this cut, every round, with no
                // window and no entry: see `Arena::spent_said`.
                if !(spent && classified) {
                    self.stage_notice(
                        unit,
                        FrameNotice::Hit {
                            contact: held,
                            section: named_section,
                            spent: false,
                        },
                    );
                } else if self.say_spent(unit, target, section, t_ms) {
                    self.stage_notice(
                        unit,
                        FrameNotice::Hit {
                            contact: held,
                            section: named_section,
                            spent: true,
                        },
                    );
                }
                // The victim reads their own tank. **Nothing about the shooter** —
                // no id, no bearing, no team, no weapon: where a round came from
                // is a thing only a sensor may tell.
                //
                // **16b cut D.2**: a round on a section that is already gone has
                // no numbers left to report — both cells are nought — so it is
                // one short sentence under the same window, keyed on the victim's
                // own tank.
                if spent {
                    if self.say_spent(target, target, section, t_ms) {
                        self.stage_notice(
                            target,
                            FrameNotice::StruckSpent {
                                section: section.clone(),
                            },
                        );
                    }
                } else {
                    let (armor_section, armor_max, internals_max) =
                        self.struck_maxima(target, section, facet);
                    self.stage_notice(
                        target,
                        FrameNotice::Struck {
                            section: section.clone(),
                            armor_section,
                            facet: facet.clone(),
                            armor_left,
                            armor_max,
                            internals_left,
                            internals_max,
                        },
                    );
                }
                // **16b cut D.4** (`T-08`): the pair has resolved, so the scratch
                // is empty again. Nothing after this line in this substep may be
                // credited to this round unless its own event says so.
                *in_flight = None;
            }
            // **16b cut D.2** (`B-03`): the victim's own two lines, off the same
            // journalled events the gunner's kill line comes off, and staged
            // after it so the gunner's is untouched.
            EventKind::UnitDisabled { unit } => {
                self.stage_notice(unit, FrameNotice::OwnDisabled);
            }
            EventKind::Miss { ref reason, .. } => {
                if let Some((shooter, target)) = *in_flight {
                    let held = self.named_target(shooter, target);
                    // **16b cut AE.2**: the reason is the platform's only while
                    // it holds the target. `moved`, `blocked`, `cover`, `wreck`
                    // are facts about where the target is and what stands round
                    // it — a position and cover check the sensors have not
                    // earned once the contact is gone, which a round with a
                    // flight time makes an everyday case. So a miss on a target
                    // the shooter no longer holds carries no reason at all, and
                    // renders `Miss.` (`crate::match_::cockpit::notice_line`).
                    // `Hit.` on an unheld target stays: a round striking metal
                    // is the shooter's own physics, paid for with a round and a
                    // cycle, and it says nothing of where or why.
                    let reason = if held.is_some() {
                        reason.clone()
                    } else {
                        SmolStr::new_static("")
                    };
                    self.stage_notice(
                        shooter,
                        FrameNotice::Miss {
                            contact: held,
                            reason,
                        },
                    );
                }
                // 16b cut D.4 (`T-08`): the pair has resolved, as on the `Hit`
                // arm above. This is the one arm that still reads the scratch.
                *in_flight = None;
            }
            EventKind::SectionDestroyed {
                unit,
                ref section,
                by,
                ..
            } => {
                // The consequences that **fired**, not the authored list the
                // ledger line carries: `disable_motion_when_all_of` is written
                // down on each of a biped's legs and does nothing until the
                // second one goes (16b cut X.2), and a pilot told their tank
                // cannot move while it is still walking would act on it.
                let consequences = self.fired_consequences(unit, section);
                self.stage_notice(
                    unit,
                    FrameNotice::SectionLost {
                        section: section.clone(),
                        consequences,
                    },
                );
                // **16b cut Q.2**: and the gunner's half of it, under the same
                // held-and-classified gate the section word on a hit is under,
                // and only when a round is what did it — a section lost to
                // anything else is nobody's shot.
                //
                // **16b cut D.4** (`T-08`): off the event's own `by` and not off
                // the substep's in-flight scratch, which by this line may be a
                // round that has already resolved. The write below is hashed
                // state, so a stale read of it is a state divergence and not only
                // a wrong line.
                if let Some(shooter) = by
                    && self.target_classified(shooter, unit)
                    && let Some(named) = self.named_target(shooter, unit)
                {
                    self.stage_notice(
                        shooter,
                        FrameNotice::TargetSectionLost {
                            contact: named,
                            section: section.clone(),
                        },
                    );
                    // The gunner's own eyes: a section this platform watched come
                    // apart is `destroyed` on its own reading from now on, so
                    // `SCAN` cannot contradict the line the pilot was just sent.
                    self.observe_section_lost(shooter, unit, section);
                }
            }
            // The kill line the shooter is owed, and only when a round is what did
            // it: a unit destroyed by anything else is nobody's shot.
            //
            // **16b cut D.4** (`T-08`): off the event's own `by`, for the reason
            // the section arm above reads it.
            EventKind::UnitDestroyed { unit, by } => {
                if let Some(shooter) = by {
                    let held = self.named_target(shooter, unit);
                    self.stage_notice(shooter, FrameNotice::Kill { contact: held });
                    // **16b cut E.2** (`B-45`; 16c §5's Q11): the shooter's own
                    // eyes, the way the section arm above writes the band its
                    // gunner watched go. The kill is knowledge this platform
                    // earned — it is the very fact the `Kill` line it was just
                    // sent carries — so its hold records it and
                    // `Arena::contact_reads_dead` lets its machine drop the
                    // target on the next tick. It is written **only** on the
                    // hold the shooter already has: a platform that killed
                    // something it never held learns nothing, because there is
                    // no row for the fact to live on.
                    //
                    // Nobody else's hold moves. A teammate that watched through
                    // its own sight has the destroyed band from the close look;
                    // one that did not keeps its track until the track expires,
                    // which is doc 16 §1's observer rule and not a courtesy.
                    if let Some(id) = self.contacts.id_of(shooter, unit)
                        && let Some(held) = self.contacts.get_mut(shooter, id)
                    {
                        held.known_dead = true;
                    }
                }
                // **16b cut D.2** (`B-03`), after the gunner's line: the pilot in
                // the wreck, who until this cut read nothing at all.
                self.stage_notice(unit, FrameNotice::OwnDestroyed);
            }
            EventKind::Lost {
                viewer,
                contact,
                target,
            } => {
                // **16b cut Q.1**: the row is already gone, so the name it read
                // as comes from the side table `sensor_pass` filled at the
                // expiry, and the entry is taken out here — this scan is what
                // drains it, so nothing the expiry put there survives the
                // substep. (It is *not* empty when this substep's hash is taken
                // a few lines above; it does not have to be, because the table
                // is not folded — see `Arena::lost_designators`.) The fallback
                // is the book's own number, which is what a `lost` produced by
                // anything that did not go through the expiry would honestly be
                // called.
                let named = Named {
                    id: contact,
                    designator: self.lost_designators.remove(&(viewer, contact)).unwrap_or(
                        crate::arena::state::Designator::Provisional(
                            self.designators.provisional(viewer, target).unwrap_or(0),
                        ),
                    ),
                };
                // **16b cut W.1**: a lock is *a lock on a contact the platform
                // holds* (doc 16 §3.5), so a contact that stops being held is a lock
                // that stops being one. This is the one place it can be done: the
                // `lost` event is what "this platform no longer holds this contact"
                // *is*, whatever produced it — the sensor pass's time-to-live now, a
                // wreck's cleared store later.
                //
                // Said at once, with none of the hysteresis below: see
                // `NoticeKind::LockLost`.
                if self
                    .unit(viewer)
                    .is_some_and(|state| state.lock == Some(contact))
                {
                    if let Some(state) = self.unit_mut(viewer) {
                        state.lock = None;
                    }
                    self.stage_notice(
                        viewer,
                        FrameNotice::Contact {
                            kind: NoticeKind::LockLost,
                            contact: named,
                            class: Classification::Unknown,
                            was: None,
                        },
                    );
                }
                // **16b cut M.2** (16c `T-83`): the end of the hysteresis is a due
                // event at the very deadline written into the table, so it is
                // said at an arena millisecond and not at whichever pulse
                // boundary came next. This runs in the staging scan, after the
                // substep's hash, so the entry and its event are first folded at
                // *k + 1*.
                //
                // A deadline at or before `now_ms` — a window shorter than the
                // substep it opened in — is not scheduled into the past: it is
                // handed back to the staging scan, which settles it at its place
                // in this same scan (see `Arena::substep`).
                let deadline = t_ms.saturating_add(window_ms);
                self.held_losses
                    .insert((viewer, contact), (deadline, named.designator));
                if deadline <= self.now_ms {
                    return Some((deadline, viewer, contact));
                }
                self.due
                    .schedule(deadline, DueKind::LossSettles { viewer, contact });
            }
            _ => {}
        }
        None
    }

    /// **Whose armour the round passed, and what that facet and this section hold
    /// when they are whole** (16b cut F.2), so the victim's line can say *of* as
    /// well as *left*.
    ///
    /// The armour is **not** always the struck section's. Doc 16 §3.3: a section
    /// with no armour of its own is *struck through* another's — the engine
    /// through `hull.rear` — and `crate::arena::damage::Arena::plan_hit` spends
    /// the round on that other section's cell, so the `armor_left` the ledger
    /// carries is that cell's and its maximum has to be read from the same place.
    /// Reading it at `engine/rear/armor`, a cell the stock tank does not declare,
    /// answered nought, and a pilot shot in the back read `110 of 0 armour left`.
    /// So the pair is resolved through the profile's own `struck_through`, exactly
    /// as the plan was, and the section it names is returned with it for the
    /// sentence to be honest about.
    ///
    /// The internals are always the **struck** section's: that is where the
    /// overflow went, whoever's armour it passed first.
    ///
    /// Both maxima are read off the cells' own `max`, which is content and does
    /// not move, so they are facts about the tank rather than about the round.
    fn struck_maxima(&self, unit: ObjId, section: &str, facet: &str) -> (SmolStr, u32, u32) {
        let Some(state) = self.unit(unit) else {
            return (SmolStr::new(section), 0, 0);
        };
        let armor_section = state
            .profile
            .section(section)
            .and_then(|profile| profile.struck_through.as_ref())
            .map_or_else(
                || SmolStr::new(section),
                |(through, _)| SmolStr::new(through.as_str()),
            );
        let armor = mush_tactics::SectionId::new(armor_section.as_str())
            .ok()
            .zip(mush_tactics::FacetId::new(facet).ok())
            .and_then(|(section, facet)| {
                mush_tactics::CellId::new(crate::definitions::unit::ARMOR_CELL)
                    .ok()
                    .map(|cell| mush_tactics::CellAddr::Facet {
                        section,
                        facet,
                        cell,
                    })
            })
            .and_then(|addr| state.unit.cell(&addr))
            .map_or(0, |cell| u32::try_from(cell.max.max(0)).unwrap_or(u32::MAX));
        let internals = mush_tactics::SectionId::new(section)
            .ok()
            .and_then(|section| {
                mush_tactics::CellId::new(crate::definitions::unit::INTERNALS_CELL)
                    .ok()
                    .map(|cell| mush_tactics::CellAddr::Section { section, cell })
            })
            .and_then(|addr| state.unit.cell(&addr))
            .map_or(0, |cell| u32::try_from(cell.max.max(0)).unwrap_or(u32::MAX));
        (armor_section, armor, internals)
    }

    /// **Whether this reader may be told about a spent section now** (16b cut
    /// D.2, `B-15`), and the write that goes with a `true`.
    ///
    /// The key is `(the platform told, the target, the section)`: the victim's
    /// own key is `(victim, victim, section)`, because his tank is both. A `true`
    /// writes the entry, so the next round inside
    /// [`SPENT_NOTICE_MS`] of arena time is
    /// silent.
    ///
    /// It runs **whether or not anybody is seated on the platform and whether or
    /// not a frame is ever sent**: the staging scan runs per substep at the
    /// event's own timestamp, so the table is a function of arena time alone and
    /// the fold below it is rate-independent. The pruning is the caller's, on
    /// touch, for the same reason.
    fn say_spent(&mut self, told: ObjId, target: ObjId, section: &SmolStr, t_ms: i64) -> bool {
        let key = (told, target, section.clone());
        if let Some(at) = self.spent_said.get(&key)
            && t_ms.saturating_sub(*at) < SPENT_NOTICE_MS
        {
            return false;
        }
        self.spent_said.insert(key, t_ms);
        true
    }

    /// **Whether pulse phase 2 says a refusal out loud** (16b cut D.2, `B-03`).
    ///
    /// Four of the fifteen, and no others:
    ///
    /// * [`Refusal::Wreck`] — the pilot is typing into a dead tank and nothing
    ///   else was telling them so;
    /// * [`Refusal::MotionDisabled`] — the engine is gone and `HEADING` does
    ///   nothing;
    /// * [`Refusal::PowerNotOnline`] — doc 16 §3.3's cold tank, refused at apply
    ///   when the cockpit's own mirror was a pulse stale;
    /// * [`Refusal::QueueFull`] — the ninth order of a burst was **dropped**, and
    ///   silently, which is the defect `B-03` names. The bound it is the far
    ///   side of is a seat's discrete orders a unit per **arena second**
    ///   (`apply_intent`, 16b cut M.1, `T-82`), the same at every tempo; before
    ///   that cut it was a queue drained once a pulse, which made how many orders
    ///   a pilot might burst a function of the tempo.
    ///
    /// The other five have nobody to tell. [`Refusal::Epoch`] is doc 16 §3.1's
    /// `FIRE` then `EXIT` burst working exactly as written: the pilot who left is
    /// told nothing about the shot they did not take. [`Refusal::Channel`],
    /// [`Refusal::Unknown`], [`Refusal::Linkdead`] and [`Refusal::NotReflex`] are
    /// answers to something no seated pilot's cockpit can produce — a wrong
    /// channel is a routing fault, an absent unit has no pilot, a linkdead seat
    /// has nobody at it, and an `ORDER` is a squad's.
    ///
    /// An [`IntentKind::Order`] is never staged whatever it was refused for: it
    /// is a commander's doctrine and not a pilot's order, and a tank's pilots are
    /// not the audience for it. **16b cut Z.2**: nor is an
    /// [`IntentKind::ReleaseOrder`], for the same reason — a release hands a
    /// squad's doctrine back, and the tank it lands on is not the pilot who gave
    /// it.
    ///
    /// **16b cut I.3**: nor is an [`IntentKind::Place`] — a staff act the pilot
    /// is told nothing of (16c N10: *journalled always, announced never*); its
    /// phase-2 refusal is a `place_refused` line on the tape instead.
    const fn phase_two_tells(kind: &IntentKind, refusal: Refusal) -> bool {
        if matches!(
            kind,
            IntentKind::Order(_) | IntentKind::ReleaseOrder { .. } | IntentKind::Place { .. }
        ) {
            return false;
        }
        matches!(
            refusal,
            Refusal::Wreck | Refusal::MotionDisabled | Refusal::QueueFull | Refusal::PowerNotOnline
        )
    }

    /// **Stage the line one refused order owes its pilot** (16b cut F.2; doc 16
    /// §5: *`Notice` events for transitions and refusals*).
    ///
    /// The recorded open point K8, in its broad form: pulse phase 4 threw every
    /// refusal away — `let _ = self.resolve_fire(…)` — and the only refusals a
    /// pilot ever saw were the cockpit's own pre-checks against a frame up to one
    /// pulse old. So a second `FIRE` inside one pulse printed `Firing cannon.` and
    /// spent nothing, and a lock that expired between the frame and the apply was
    /// silent.
    ///
    /// The numbers are read **here**, at the refusal, because that is the only
    /// moment they are true; `crate::match_::cockpit` turns the pair into the same
    /// sentence its own pre-check prints for the same condition, which is what
    /// keeps the early answer and the late one from drifting apart.
    ///
    /// Two silences are deliberate. An intent **purged** by an epoch raise never
    /// reaches this phase at all (`apply_control` drops it), which is doc 16
    /// §3.1's `FIRE` then `EXIT` burst working exactly as written — the pilot who
    /// left is told nothing about the shot they did not take. And the reflex
    /// controller resolves its own shots inside a substep and never through this
    /// phase, so an AI refusal reaches nobody.
    ///
    /// Pulse **phase 2** stages through here too since 16b cut D.2 (`B-03`), for
    /// the four refusals [`Arena::phase_two_tells`] admits; the order word is
    /// [`IntentKind::as_str`] in both phases, so one order cannot be spelled two
    /// ways.
    fn stage_refusal(
        &mut self,
        unit: ObjId,
        order: &'static str,
        refusal: Refusal,
        weapon: Option<SmolStr>,
    ) {
        let detail = match refusal {
            Refusal::Cycling => {
                let secs = weapon
                    .as_ref()
                    .and_then(|weapon| {
                        self.unit(unit)
                            .map(|state| state.weapon_ready_in_ms(weapon, self.now_ms))
                    })
                    .unwrap_or(0);
                RefusalDetail::Cycling {
                    secs: secs.saturating_add(999) / 1_000,
                }
            }
            Refusal::Overheated => match self
                .unit(unit)
                .and_then(|state| state.heat_now().zip(state.heat_max()))
            {
                Some((heat, max)) => RefusalDetail::Overheated { heat, max },
                None => RefusalDetail::None,
            },
            _ => RefusalDetail::None,
        };
        self.stage_notice(
            unit,
            FrameNotice::Refused {
                order: SmolStr::new_static(order),
                refusal,
                weapon,
                detail,
            },
        );
    }

    // -----------------------------------------------------------------------
    // The staff place (16b cut I.3; 16c N10)
    // -----------------------------------------------------------------------
    //
    // Every `Place` the arena receives leaves one line: `place_refused` at
    // phase 2 (`unknown`, `epoch`, `channel`), `placed` or `place_refused` here
    // at phase 4, or `place_refused` with `stopped` in the pulse a `Stop` ends
    // the match in (`Arena::stopped`). The one exception is a place queued at
    // phase 2 of the pulse whose own substep ends the match — a kill or the
    // duration bound — which phase 4 never drains, because no line may follow
    // `match_ended` (16b cut W.2).

    /// **Put one unit on a cell, for staff** (16b cut I.3; 16c N10): the
    /// phase-4 resolution of an [`IntentKind::Place`], at the pulse's end and
    /// under phase 4's one hash like every action there.
    ///
    /// The tests, in this order, the first that fails naming the reason:
    ///
    /// 1. the unit exists — else `unknown`. Unreachable past phase 2's own gate,
    ///    and journalled all the same, so *every `Place` the arena receives
    ///    leaves a line* holds by one rule;
    /// 2. the cell is on the window, passable and has a way out **for the
    ///    placed unit's own cost row**
    ///    ([`crate::arena::state::ArenaTerrain::has_way_out_for`] on
    ///    `UnitState::cost_row`, 16b cut AE.1 — the rule an opening cell is
    ///    held to; a car onto rubble is refused though a tank would stand
    ///    there) — else `impassable`;
    /// 3. nobody else stands in it ([`crate::arena::index::SpatialIndex::occupant`]
    ///    is empty or the unit itself) — else `occupied`: doc 16 §3.4 step 5's
    ///    one unit per cell;
    /// 4. **no round is in flight to or from the unit** — no [`Arena::shots`]
    ///    row whose `target` or `shooter` it is — else `in_flight`. An impact
    ///    traces its line from the SHOOTER's pose when the round arrives
    ///    (`damage::resolve_impact`) and teaches its machine `no line` on a
    ///    blocked one, so a placed shooter would decide its own round's outcome,
    ///    and a placed target would dodge (or meet) a round aimed elsewhere.
    ///
    /// On success the pose goes to the cell's centre at the cell's `elev_cm`,
    /// heading kept; speed, desired speed and the motion carries go to nought;
    /// the hold, the traffic hold, `blocked` and `grade_pct` are cleared; the
    /// index moves the unit (rebuilt whole when it stood off the window); and a
    /// machine forgets its route ([`crate::arena::motor::MotorTable::forget_route`])
    /// so it re-plans at its next tick. **No contact is dropped**: a contact id
    /// and its designator are drawn once per (viewer, target) for the whole
    /// match (`draw_contact_id`, `DesignatorBook::assign`), so a drop could mint
    /// no new name, and the loss path is split between the sensor pass and the
    /// staging scan; every other platform's contact on the unit is left as it
    /// is, and the next sensor pass moves the track to where the unit now is —
    /// which is the truth of that sensor. Then one `placed` line, `from_x` and
    /// `from_y` `u16::MAX` for a unit that stood off the window.
    ///
    /// **Every outcome is journalled and none is staged** (16c N10: *journalled
    /// always, announced never*): the pilot's next frame carries the new cell
    /// and nothing says why. The hash is asked before anything is written,
    /// `Arena::event_hash`'s rule.
    ///
    /// # Errors
    ///
    /// The reason word, after its `place_refused` line is journalled.
    pub(crate) fn resolve_place(
        &mut self,
        unit: ObjId,
        x: u16,
        y: u16,
        at_ms: i64,
    ) -> Result<(), SmolStr> {
        let refused = self.place_refusal(unit, x, y);
        let hash = self.event_hash();
        if let Some(reason) = refused {
            let reason = SmolStr::new_static(reason);
            let _ = self.journal(
                at_ms,
                EventKind::PlaceRefused {
                    unit,
                    x,
                    y,
                    reason: reason.clone(),
                },
                hash,
            );
            return Err(reason);
        }
        let terrain = &self.definition.terrain;
        let cell_cm = i64::from(terrain.cell_cm());
        let elev_cm = terrain.get(x, y).map_or(0, |cell| i32::from(cell.elev_cm));
        let from = self
            .unit(unit)
            .and_then(|state| self.definition.terrain.cell_of(&state.pose));
        if let Some(state) = self.unit_mut(unit) {
            state.pose.x_cm = i64::from(x)
                .saturating_mul(cell_cm)
                .saturating_add(cell_cm / 2);
            state.pose.y_cm = i64::from(y)
                .saturating_mul(cell_cm)
                .saturating_add(cell_cm / 2);
            state.pose.z_cm = elev_cm;
            state.speed_cms = 0;
            state.desired_speed_cms = 0;
            state.carry = crate::arena::state::MotionCarry::default();
            state.held = None;
            state.occupied_hold = None;
            state.blocked = false;
            state.grade_pct = 0;
        }
        match from {
            Some(from) => self.index.move_unit(unit, from, (x, y)),
            None => self.index.rebuild(&self.definition.terrain, &self.units),
        }
        self.motors.forget_route(unit);
        let (from_x, from_y) = from.unwrap_or((u16::MAX, u16::MAX));
        let _ = self.journal(
            at_ms,
            EventKind::Placed {
                unit,
                x,
                y,
                from_x,
                from_y,
            },
            hash,
        );
        Ok(())
    }

    /// **Why a place would be refused**, or `None` (16b cut I.3): the four
    /// tests of [`Arena::resolve_place`], in its order, as their wire words.
    fn place_refusal(&self, unit: ObjId, x: u16, y: u16) -> Option<&'static str> {
        let Some(state) = self.unit(unit) else {
            return Some("unknown");
        };
        // **16b cut AE.1** (16c `B-58`): passable, with a way out, **to the
        // placed unit's own row** — a car put down on rubble is `impassable`,
        // whatever a tank could have done there.
        let terrain = &self.definition.terrain;
        if !terrain.has_way_out_for(state.cost_row, x, y) {
            return Some("impassable");
        }
        if self
            .index
            .occupant(x, y)
            .is_some_and(|holder| holder != unit)
        {
            return Some("occupied");
        }
        if self
            .shots
            .values()
            .any(|shot| shot.target == unit || shot.shooter == unit)
        {
            return Some("in_flight");
        }
        None
    }

    /// Put one notice in the platform's queue for this pulse.
    ///
    /// `pub(crate)` since 16b cut O.2, for
    /// [`crate::arena::intents::resolve_emcon`], which stages its own notice as
    /// it journals its own line.
    pub(crate) fn stage_notice(&mut self, platform: ObjId, notice: FrameNotice) {
        self.pending_notices
            .entry(platform)
            .or_default()
            .push(notice);
    }

    /// **The pair one contact id is named by right now** (16b cut Q.1), for a
    /// notice staged while the row is still held.
    ///
    /// A row the platform no longer holds cannot happen on the arms that call it
    /// — a `detected` or a `classified` is staged in the same substep the store
    /// was written in — and answers `X0`, a name nothing resolves, rather than
    /// taking the arena down.
    fn name_of(&self, viewer: ObjId, contact: ContactId) -> Named {
        Named {
            id: contact,
            designator: self
                .contacts
                .get(viewer, contact)
                .map_or(crate::arena::state::Designator::Provisional(0), |held| {
                    self.designator_for(viewer, held)
                }),
        }
    }

    /// **The contact one platform holds a target under, named** (16b cut Q.1), or
    /// `None` when it holds none — which is what makes a shooter's line read
    /// `Hit.` rather than `Hit on BA.`
    fn named_target(&self, viewer: ObjId, target: ObjId) -> Option<Named> {
        let id = self.contacts.id_of(viewer, target)?;
        Some(self.name_of(viewer, id))
    }

    /// **Whether this platform has told that target apart, right now** (16b cut
    /// Q.2) — the gate on every word a gunner is told about the target's hull.
    ///
    /// Read off the live contact and not off a flag, for the reason
    /// `Arena::designator_for` is: a contact is not a durable object, so a target
    /// that was classified, lost and re-detected is `Unknown` again and the
    /// section words stop until it has been told apart a second time.
    fn target_classified(&self, viewer: ObjId, target: ObjId) -> bool {
        self.contacts
            .id_of(viewer, target)
            .and_then(|id| self.contacts.get(viewer, id))
            .is_some_and(|held| matches!(held.class, Classification::Classified { .. }))
    }

    /// **Write what the gunner watched happen into the reading they hold** (16b
    /// cut Q.2): that section's band becomes
    /// [`crate::arena::condition::ConditionBand::Destroyed`].
    ///
    /// The **stamp is untouched**, and so is `own_seen.condition`. The stamp is
    /// the age of the *look*, and this is not a look — it is one fact observed
    /// through a gunsight, folded into a picture that is otherwise as old as it
    /// says it is. Leaving `own_seen` alone is what keeps it off the mesh: what a
    /// teammate is handed is what this platform's sensors read, not what its
    /// gunner inferred from a fire.
    ///
    /// A platform with no reading is left with none: a gunner who has never been
    /// close enough to read the hull has not earned a picture by shooting at it.
    ///
    /// The index is the section's position in the **target's** own section order,
    /// which is the order every band on that contact is in.
    fn observe_section_lost(&mut self, viewer: ObjId, target: ObjId, section: &str) {
        let Some(index) = self.unit(target).and_then(|state| {
            state
                .unit
                .sections()
                .position(|(id, _)| id.as_str() == section)
        }) else {
            return;
        };
        let Some(id) = self.contacts.id_of(viewer, target) else {
            return;
        };
        if let Some(held) = self.contacts.get_mut(viewer, id)
            && let Some(reading) = held.seen_condition.as_mut()
        {
            reading.condition = reading
                .condition
                .with_band(index, crate::arena::condition::ConditionBand::Destroyed);
        }
    }

    /// **What one platform holds, as its pilots' frame carries it** (16b cut S.1;
    /// doc 16 §3.6 "What a pilot sees"), in contact-id order.
    ///
    /// **16b cut F.3 made it a plot rather than a tether.** Bearing, range and
    /// cell are taken from the pose the target was in **at the sighting that last
    /// refreshed this contact** — this platform's own, or the sharer's own as the
    /// share carried it — and never from the target's live pose. Cut S.1 read the
    /// target live and justified it with "the contact was refreshed within the
    /// last `sensors.pass_ms` or it would not be here", which was false by a factor
    /// of `lost_after_passes`: a tank that ducked behind a building was tracked
    /// through it, cell by cell, for the whole time to live, by pilots and by the
    /// reflex controller alike.
    ///
    /// `age_ms` is the age of **that sighting** (`now − last_seen_ms`) and not the
    /// hold time since first detection, because what a pilot needs off this column
    /// is whether the plot in front of them is a live track or a dying one. While
    /// a target is in continuous view it is at most one `sensors.pass_ms` old.
    ///
    /// A contact whose target has left the match is dropped rather than reported:
    /// there is no unit for it to be a fact about.
    #[must_use]
    pub fn frame_contacts(&self, platform: ObjId) -> Vec<FrameContact> {
        self.frame_scope(platform).0
    }

    /// **The section vocabularies one platform's readings are in** (16b cut
    /// Q.2), the companion of [`Arena::frame_contacts`] and the other half of
    /// [`Frame::scan_tables`].
    ///
    /// The two are built by one walk (`Arena::frame_scope`); this is the
    /// caller that wants only the tables, exactly as `frame_contacts` is the one
    /// that wants only the rows.
    #[must_use]
    pub fn frame_scan_tables(&self, platform: ObjId) -> Vec<Vec<SmolStr>> {
        self.frame_scope(platform).1
    }

    /// **The scope and the section vocabularies its readings are in** (16b cut
    /// Q.2), built together because a [`FrameCondition`]'s `table` is an index
    /// into the list beside it and the two cannot be built apart without one of
    /// them being able to name a row the other does not have.
    ///
    /// A table is added by the **first** row that carries a reading in that
    /// chassis's vocabulary, and a second row on the same chassis re-uses the
    /// index — so a frame holding four stock tanks carries one table. A row with
    /// no reading contributes nothing, which is the whole of the rule
    /// [`Frame::scan_tables`] states.
    fn frame_scope(&self, platform: ObjId) -> (Vec<FrameContact>, Vec<Vec<SmolStr>>) {
        let Some(from) = self.unit(platform) else {
            return (Vec::new(), Vec::new());
        };
        let mut tables: Vec<Vec<SmolStr>> = Vec::new();
        let mut rows: Vec<FrameContact> = Vec::new();
        for (id, held) in self.contacts.of(platform) {
            // A contact whose target has left the match is dropped: only the
            // unit's presence is read here, never its pose.
            let Some(target) = self.unit(held.target) else {
                continue;
            };
            // **16b cut Q.2**: the reading, and the chassis vocabulary it is in —
            // resolved only for a row that has one, so a row with no reading
            // cannot put a section id on the frame at all.
            let condition = held.seen_condition.map(|reading| {
                let names: Vec<SmolStr> = target
                    .unit
                    .sections()
                    .map(|(section, _)| SmolStr::new(section.as_str()))
                    .collect();
                let table = match tables.iter().position(|held| *held == names) {
                    Some(at) => at,
                    None => {
                        tables.push(names);
                        tables.len().saturating_sub(1)
                    }
                };
                FrameCondition {
                    table: u8::try_from(table).unwrap_or(u8::MAX),
                    bands: reading.condition,
                    age_ms: self.now_ms.saturating_sub(reading.at_ms),
                }
            });
            rows.push(FrameContact {
                id: *id,
                // **16b cut Q.1**: read off the live contact, so a row that
                // has been re-acquired and not yet re-classified is `X<n>`
                // again, and the alias is what keeps a lock taken under that
                // number typable after the rename.
                designator: self.designator_for(platform, held),
                alias: self.alias_for(platform, held),
                class: held.class,
                bearing_mdeg: held.seen_at.bearing_from(&from.pose),
                range_cm: held.seen_at.range_from(&from.pose),
                age_ms: self.now_ms.saturating_sub(held.last_seen_ms),
                cell: held.seen_at.cell(&self.definition.terrain),
                condition,
            });
        }
        (rows, tables)
    }

    /// **What one tank has left, as its own pilots' frame carries it** (16b cut
    /// F.2; doc 16 §5's status block: *armor and internals per section as current
    /// over max*), in **section-id then facet-id order** — which is the order
    /// [`mush_tactics::TacticalUnit`]'s own ordered maps hold them in, and not the
    /// order the pack authored them in.
    ///
    /// A projection of the M32 cells and never a second model: the impact's plan
    /// is the single writer of both (doc 16 §3.5 "Damage"), so there is nothing
    /// here to keep in step. It is built **per pilot per pulse** and the walk is
    /// the unit's own five sections and their facets — a handful of small reads,
    /// which is why a platform nobody is seated on pays nothing for it.
    ///
    /// It names the pilot's **own** tank and is therefore outside the observer
    /// rule: what a crew can read off its own gauges is physical knowledge.
    #[must_use]
    pub fn frame_sections(&self, platform: ObjId) -> Vec<FrameSection> {
        let Some(state) = self.unit(platform) else {
            return Vec::new();
        };
        let Ok(armor_cell) = mush_tactics::CellId::new(crate::definitions::unit::ARMOR_CELL) else {
            return Vec::new();
        };
        let Ok(internals_cell) =
            mush_tactics::CellId::new(crate::definitions::unit::INTERNALS_CELL)
        else {
            return Vec::new();
        };
        state
            .unit
            .sections()
            .map(|(id, section)| FrameSection {
                id: SmolStr::new(id.as_str()),
                condition: SmolStr::new(section.condition.as_str()),
                internals: state
                    .unit
                    .cell(&mush_tactics::CellAddr::Section {
                        section: id.clone(),
                        cell: internals_cell.clone(),
                    })
                    .map(|cell| (cell.current, cell.max)),
                facets: section
                    .facets
                    .iter()
                    .filter_map(|facet| {
                        let cell = state.unit.cell(&mush_tactics::CellAddr::Facet {
                            section: id.clone(),
                            facet: facet.clone(),
                            cell: armor_cell.clone(),
                        })?;
                        Some(FrameFacet {
                            id: SmolStr::new(facet.as_str()),
                            armor: cell.current,
                            armor_max: cell.max,
                        })
                    })
                    .collect(),
            })
            .collect()
    }

    /// **The substep length**, arena milliseconds (doc 16 §3.1 "Rate
    /// independence"): `substep_ms` over this match's tempo block, its cell and
    /// its true speed ceiling.
    ///
    /// **16b cut U.1**: one number with three readers — the partition in
    /// [`Arena::step`], the lattice [`Arena::next_due_ms`] wakes the task on, and
    /// the stopping look-ahead in [`Arena::hold_routeless`] — where the first and
    /// the last each computed the product for themselves.
    ///
    /// **It is read live, and it is not frozen in general.** The tempo block and
    /// the cell are fixed at [`Arena::new`], but [`Arena::max_speed_cms`] is
    /// recomputed on every call over the units that are neither a wreck nor
    /// motion-disabled, and both of those change mid-match. The ceiling can only
    /// fall, so the substep can only lengthen, up to `tempo.substep_max_ms`. On
    /// the shipped content the answer is constant because that cap binds for
    /// every fielded unit (250 ms; 16c Q24); on a config whose fastest unit cuts
    /// a shorter substep, losing that unit lengthens it from the next pulse, and
    /// both the step's partition and [`Arena::next_due_ms`]'s lattice read the
    /// new figure there. Freezing the ceiling at check-out would be a behaviour
    /// change and is not this cut's.
    pub(crate) fn substep_len_ms(&self) -> i64 {
        i64::from(substep_ms(
            &self.arena_cfg.tempo,
            self.cell_cm(),
            self.max_speed_cms(),
        ))
    }

    /// **Resume a decoded arena on this process's wall** (16b cut AF.1; doc 16
    /// §3.1 "Clock and lag"; 16c N16), answering the first deadline the task
    /// should wake at, in the caller's own wall milliseconds — exactly what a
    /// pulse's [`StepOutput::next_due_ms`] would have answered at that wall.
    ///
    /// The clock is re-anchored ([`crate::arena::clock::Clock::reanchor`]):
    /// **arena time does not advance across the outage**, the first resumed
    /// pulse measures its `dt` from `wall_now_ms`, no `lagged` line is journalled
    /// for the gap, and a staff floor the old process had set is gone (16c
    /// `Q-AF1`). Everything that decides *when* something happens next — the
    /// due heap, the shots in flight, the staggered sensor passes and reflex
    /// ticks, the wake lattice itself — is carried integers in arena time, so
    /// the deadline answered here is the one the match would have woken at, and
    /// the cross-tempo property holds across a resume as it holds across a
    /// pulse (`crates/tacsim/tests/arena.rs`'s `af1_*` tests; a round carried
    /// in flight across a resume, at two cadences, is
    /// `crates/tacsim/tests/shipped_scale.rs`'
    /// `af1_the_shipped_centre_encodes_under_the_expectation`).
    pub fn resume_at(&mut self, wall_now_ms: i64) -> i64 {
        self.clock.reanchor(wall_now_ms);
        self.next_due_ms(wall_now_ms)
    }

    /// **When the task should step again**, in the wall milliseconds
    /// `wall_now_ms` was given in (doc 16 §3.1 phase 5; 16c Q25).
    ///
    /// The rule, in **absolute arena time**: the earlier of the next multiple of
    /// the tempo's period strictly after `now_ms`, and — when anything is due —
    /// the first multiple of the substep length at or after the next due event
    /// (never `now_ms` itself); answered as `wall_now_ms + (target − now_ms)`, so
    /// never sooner than a millisecond away.
    ///
    /// **Why the targets are absolute** (16b cut U.1; 16c `T-22`). A due event
    /// inside a substep is applied in that substep, at its own timestamp,
    /// against the positions at the substep's **end** (§3.1 phase 3). Woken at
    /// the due event itself — 16c's first sketch — the pulse would end there, and
    /// the next substep would be cut from that wake: the position the event is
    /// applied against would be a function of when the task woke, which is
    /// exactly what D-16-2 forbids. So the wake is the substep boundary the event
    /// already belongs to. And the target is a lattice point in arena time, not
    /// an interval from the last wake: a rule of `wall + period` or `wall +
    /// (due − now)` measures from wherever the last wake landed, so after one
    /// late wake it would never return to the lattice; this one pulls a late
    /// wake back (from `now = 253` with a due at 500 on a 250 ms lattice, it asks
    /// for 247 ms). The period is a multiple of the substep for every shipped
    /// configuration (asserted for every fielded unit since 16b cut M.2, 16c
    /// Q24), so the period multiples are lattice points too, and a task that
    /// wakes on time wakes only on the lattice.
    ///
    /// In production arena time is wall time, so a wake is the lattice plus the
    /// OS timer's overshoot (wall jitter, §3.1); a late wake's pulse ends off the
    /// lattice, and the next one returns to it.
    ///
    /// The lattice is the **current** [`Arena::substep_len_ms`], which is
    /// constant on the shipped content and can lengthen mid-match elsewhere (see
    /// there); the partition inside a pulse is still cut from that pulse's start.
    ///
    /// **16b cut I.3: under a staff floor the answer is a WALL interval** —
    /// `wall_now_ms + max(period, floor)` ([`crate::arena::clock::Clock::wake_period_ms`]),
    /// the due-event branch not consulted. A floored arena is behind the wall on
    /// purpose, so the rule above — `wall + (target − now)` with the target in
    /// arena time — would wake the task after 5,000, then 3,000, then 1,000 wall
    /// ms as the arena caught up with lattice points it can only reach a clamp
    /// at a time. The due events are still applied at their own timestamps,
    /// inside the catch-up substeps, as always; and the lattice is kept by
    /// [`Arena::step`] quantising every floored pulse instead (see there). With
    /// no floor the rule is unchanged.
    fn next_due_ms(&self, wall_now_ms: i64) -> i64 {
        if self.clock.floor_ms().is_some() {
            return wall_now_ms
                .saturating_add(i64::from(self.clock.wake_period_ms(&self.arena_cfg.tempo)));
        }
        let now = self.now_ms;
        let period = i64::from(self.clock.tempo().period_ms(&self.arena_cfg.tempo));
        let substep = self.substep_len_ms();
        let after_now = now.saturating_add(1);
        let target_period = ceil_to(after_now, period);
        let target = match self.due.next_t_ms() {
            Some(due) => target_period.min(ceil_to(due.max(after_now), substep)),
            None => target_period,
        };
        wall_now_ms.saturating_add(target.saturating_sub(now))
    }
}

/// **`t` rounded up to a multiple of `m`** (16b cut U.1), saturating: the least
/// multiple of `m` at or after `t`. An `m` of nought or below answers `t`, so a
/// degenerate period or substep asks for the moment itself rather than for
/// nothing. Integer arithmetic only; `t` is arena milliseconds and never
/// negative where [`Arena::next_due_ms`] calls it.
fn ceil_to(t: i64, m: i64) -> i64 {
    if m <= 0 {
        return t;
    }
    t.saturating_add(m - 1).div_euclid(m).saturating_mul(m)
}

/// **The reason word of a staff place that reached the pulse a `Stop` ends the
/// match in** (16b cut I.3): `place_refused` with `stopped`, journalled by
/// [`Arena::stopped`] before the stop's own line.
const PLACE_STOPPED: &str = "stopped";

/// **`t` rounded down to a multiple of `m`** (16b cut I.3), [`ceil_to`]'s other
/// half: the greatest multiple of `m` at or below `t`. An `m` of nought or below
/// answers `t`, so a degenerate substep quantises nothing rather than stopping
/// time. Integer arithmetic only; [`Arena::step`] calls it with `t` the arena
/// millisecond a floored pulse would end at.
fn floor_to(t: i64, m: i64) -> i64 {
    if m <= 0 {
        return t;
    }
    t.div_euclid(m).saturating_mul(m)
}

/// The remembered frames, as [`Arena`] holds them — a type alias so the field's
/// declaration reads as one thing rather than as a nested generic.
pub type FrameSlots = BTreeMap<ObjId, Frame>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::intents::{Control, ControlState};
    use crate::arena::ledger::LedgerEvent;
    use crate::arena::state::{Channel, Team};
    use crate::arena::test_support::{arena_at_epoch, pose, tank};
    use crate::config::{ArenaConfig, MatchConfig};

    const A: ObjId = ObjId(4);

    fn arena() -> Arena {
        arena_at_epoch(vec![tank(A, Team::Red, pose(3_500, 3_500))], 1)
    }

    fn startup() -> Intent {
        Intent {
            unit: A,
            channel: Channel::Helm,
            epoch: 1,
            kind: IntentKind::Startup,
            seq: 1,
        }
    }

    /// Everything the arena has journalled: the chunks it handed out as it ran,
    /// plus whatever is still open when the assertion asks.
    #[derive(Default)]
    struct Log {
        bytes: Vec<u8>,
    }

    impl Log {
        fn absorb(&mut self, out: &StepOutput) {
            for chunk in &out.ledger_chunks {
                self.bytes.extend_from_slice(&chunk.bytes);
            }
        }

        fn events(&mut self, arena: &mut Arena) -> Vec<LedgerEvent> {
            arena.ledger.flush();
            for chunk in arena.ledger.take_chunks() {
                self.bytes.extend_from_slice(&chunk.bytes);
            }
            String::from_utf8(self.bytes.clone())
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
                .collect()
        }
    }

    #[test]
    fn the_opening_pulse_establishes_the_wall_reference_and_integrates_nothing() {
        let mut arena = arena();
        let out = arena.step(1_000_000, &[], &[]);
        assert_eq!(out.substeps, 0);
        assert_eq!(arena.now_ms, 0);
        assert!(!out.lagged);
        assert!(out.ended.is_none());
        assert_eq!(out.tempo, Tempo::Quiet);
        // The next step is asked for at the period, at the first summary, or at
        // the first sensor pass, whichever is soonest. **16b cut F.3** makes that
        // the pass: every unit has one. **16b cut U.1**: staggered at
        // `stagger_ms(seed, id, pass_ms)` — unit #4's is 482 ms at seed 2,350 —
        // and the wake is the first substep boundary at or after it, 500 on
        // this fixture's 250 ms lattice (16c Q25).
        let pass = crate::arena::rng::stagger_ms(arena.seed, A, 1_000);
        assert_eq!(pass, 482, "the fixture's precondition");
        assert_eq!(arena.substep_len_ms(), 250);
        assert_eq!(out.next_due_ms, 1_000_500);
        assert!(arena.substep_hashes.is_empty());
        // One frame, because the pilot has never been told anything.
        assert_eq!(out.frames.len(), 1);
        assert_eq!(
            out.frames.first().map(|(pilot, _)| *pilot),
            Some(ObjId(1000))
        );
        // And nothing more until something changes.
        let again = arena.step(1_000_000, &[], &[]);
        assert!(again.frames.is_empty());
    }

    /// An arena at arena millisecond `now` whose heap holds nothing but `due`,
    /// for [`Arena::next_due_ms`]'s rule on its own. `Quiet`, so the period is
    /// 5,000 ms; the fixture tank's 1,000 cm/s on 1,000 cm cells cuts a 250 ms
    /// substep.
    fn at_now_with(now: i64, due: Option<i64>) -> Arena {
        let mut arena = arena();
        arena.due.retain(|_| false);
        arena.now_ms = now;
        if let Some(t_ms) = due {
            arena.due.schedule(t_ms, DueKind::PositionSummary);
        }
        assert_eq!(arena.clock.tempo(), Tempo::Quiet);
        assert_eq!(arena.substep_len_ms(), 250, "the fixture's lattice");
        arena
    }

    /// **16b cut U.1** (16c `T-22`, Q25): the wake is on the substep lattice in
    /// absolute arena time — a due event wakes the arena at the first substep
    /// boundary at or after it, the period term is the next period multiple, and
    /// a late arena is pulled back onto the lattice.
    #[test]
    fn next_due_is_the_first_lattice_point_at_or_after_the_due_event() {
        const WALL: i64 = 1_000_000;
        // From `now` on the lattice, a due event 3 ms ahead is the next boundary.
        assert_eq!(
            at_now_with(1_000, Some(1_003)).next_due_ms(WALL),
            WALL + 250
        );
        // One a substep and a millisecond ahead is the boundary after that.
        assert_eq!(
            at_now_with(1_000, Some(1_251)).next_due_ms(WALL),
            WALL + 500
        );
        // One exactly on a boundary is that boundary.
        assert_eq!(
            at_now_with(1_000, Some(1_250)).next_due_ms(WALL),
            WALL + 250
        );
        // One due now — or already past — is the next boundary, never now.
        assert_eq!(
            at_now_with(1_000, Some(1_000)).next_due_ms(WALL),
            WALL + 250
        );
        assert_eq!(at_now_with(1_000, Some(400)).next_due_ms(WALL), WALL + 250);
        // An empty heap is the next period multiple.
        assert_eq!(at_now_with(1_000, None).next_due_ms(WALL), WALL + 4_000);
        // The period term wins when it is the nearer.
        assert_eq!(
            at_now_with(4_900, Some(6_000)).next_due_ms(WALL),
            WALL + 100
        );
        // **The pull-back**: a wake that landed late, at 253, with a due event
        // at 500 asks for 500 — 247 ms — and not for 250 after the late wake.
        assert_eq!(at_now_with(253, Some(500)).next_due_ms(WALL), WALL + 247);
        assert_eq!(at_now_with(253, None).next_due_ms(WALL), WALL + 4_747);
    }

    /// **16b cut U.1**: a period shorter than the substep answers the period,
    /// and the answer is never sooner than a millisecond away, wherever `now`
    /// and the due event sit.
    #[test]
    fn next_due_takes_a_short_period_and_is_never_below_a_millisecond() {
        const WALL: i64 = 7_000;
        let mut arena = at_now_with(1_000, Some(3_000));
        arena.arena_cfg.tempo.quiet_ms = 100;
        assert_eq!(arena.next_due_ms(WALL), WALL + 100);
        for now in [0_i64, 1, 249, 250, 251, 999, 4_999, 5_000] {
            for due in [None, Some(0), Some(now), Some(now + 1), Some(now + 250)] {
                let answer = at_now_with(now, due).next_due_ms(WALL);
                assert!(
                    answer > WALL,
                    "now {now}, due {due:?}: {answer} is not a millisecond away"
                );
                // And every target is a lattice point.
                assert_eq!((now + answer - WALL) % 250, 0, "now {now}, due {due:?}");
            }
        }
    }

    #[test]
    fn a_pulse_runs_the_substeps_the_clamped_dt_is_cut_into() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        // 1,000 cm cells and a 1,000 cm/s tank: the substep is the 250 ms cap.
        let out = arena.step(1_000, &[], &[]);
        assert_eq!(out.substeps, 4);
        assert_eq!(arena.now_ms, 1_000);
        assert_eq!(arena.substep_hashes.len(), 4);
        // Every substep's hash is recorded, and they differ because `now_ms` does.
        let unique: std::collections::BTreeSet<u64> =
            arena.substep_hashes.iter().copied().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn startup_is_queued_in_phase_two_and_resolved_in_phase_four() {
        let mut arena = arena();
        let out = arena.step(0, &[], &[startup()]);
        assert!(arena.pending.is_empty(), "phase 4 drained the queue");
        assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Starting));
        assert!(out.ended.is_none());
        // `match.startup_ms` later, at the substep boundary at or after it.
        let mut wall = 0_i64;
        for _ in 0..3 {
            wall += 5_000;
            arena.step(wall, &[], &[]);
        }
        assert_eq!(arena.now_ms, 15_000);
        assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Online));
    }

    #[test]
    fn a_stop_ends_the_match_at_phase_one_and_a_further_step_does_nothing() {
        for (stop, reason, word) in [
            (StopReason::Abort, EndReason::Abort, "aborted"),
            (StopReason::Timeout, EndReason::Timeout, "match_ended"),
            (StopReason::Void, EndReason::Void, "void"),
        ] {
            let mut arena = arena();
            let mut journal = Log::default();
            journal.absorb(&arena.step(0, &[], &[]));
            arena.stop(stop);
            let out = arena.step(5_000, &[], &[]);
            journal.absorb(&out);
            assert_eq!(out.ended.map(|end| end.reason), Some(reason));
            assert_eq!(out.substeps, 0);
            assert_eq!(arena.now_ms, 0, "a stop never advances arena time");
            let log = journal.events(&mut arena);
            assert_eq!(log.last().map(|event| event.kind.as_str()), Some(word));
            // And the arena is finished with.
            let after = arena.step(10_000, &[], &[]);
            assert_eq!(after.ended.map(|end| end.reason), Some(reason));
            assert!(after.ledger_chunks.is_empty());
            assert!(after.frames.is_empty());
            assert_eq!(after.substeps, 0);
        }
    }

    #[test]
    fn the_duration_bound_ends_the_match_with_a_timeout() {
        let mut arena = Arena::new(
            crate::arena::test_support::custody(
                vec![tank(A, Team::Red, pose(3_500, 3_500))],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig {
                max_duration_ms: 3_000,
                ..MatchConfig::default()
            },
            &crate::config::SensorsConfig::default(),
        )
        .expect("a three-second match");
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        let out = arena.step(2_000, &[], &[]);
        journal.absorb(&out);
        assert!(out.ended.is_none());
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        // **16b cut M.2** (16c `T-01`): at the deadline's own millisecond, 3,000,
        // and not at the end of the pulse that crossed it (4,000 before the cut).
        assert_eq!(
            out.ended,
            Some(MatchEnd {
                reason: EndReason::Timeout,
                t_ms: 3_000
            })
        );
        // The pulse stopped at the substep the deadline fell in: 3,000 is the end
        // of the fourth of this pulse's eight, and the other four never ran.
        assert_eq!(out.substeps, 4);
        let log = journal.events(&mut arena);
        assert_eq!(
            log.last().map(|event| event.kind.as_str()),
            Some("match_ended")
        );
        // And a further step does nothing at all.
        let after = arena.step(6_000, &[], &[]);
        assert_eq!(after.substeps, 0);
        assert!(after.ledger_chunks.is_empty());
        assert_eq!(arena.now_ms, 3_000);
    }

    #[test]
    fn a_released_seat_in_the_same_burst_purges_the_order_it_arrived_with() {
        // Doc 16 §3.1's burst, end to end through `step`: phase 1 raises the
        // epoch, phase 2 refuses the order that was typed under the old one.
        let mut arena = arena();
        let out = arena.step(
            0,
            &[Control {
                unit: A,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
                pilot: None,
            }],
            &[startup()],
        );
        assert!(out.ended.is_none());
        assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Off));
        assert!(arena.pending.is_empty());
        // **16b cut N.1** amends doc 16 §3.8's *the tank idles*: a released seat
        // hands the tank to the reflex controller in `Withdraw` (doc 07 §1.1's
        // fallback controller), so the schedule now carries its first tick beside
        // the position summary. Nothing else about the burst changed.
        assert_eq!(
            arena.unit(A).map(|unit| unit.controller),
            Some(crate::arena::state::Controller::Reflex)
        );
        assert_eq!(
            arena.reflex.unit(A).map(|row| row.state),
            Some(crate::arena::reflex::ReflexState::Withdraw)
        );
        // 16b cut F.3 adds the third: this platform's own sensor pass. **16b cut
        // M.2** (16c `T-01`) the fourth: the duration bound.
        assert_eq!(
            arena.due.len(),
            4,
            "the position summary, the duration bound, the released tank's first tick, its pass"
        );
        let mut journal = Log::default();
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert_eq!(
            log.iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["match_started"]
        );
    }

    #[test]
    fn a_fire_intent_raises_the_alert_tempo_and_journals_the_change() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        let fire = Intent {
            unit: A,
            channel: Channel::Fire,
            epoch: 1,
            kind: IntentKind::Fire {
                weapon: smol_str::SmolStr::new("cannon"),
            },
            seq: 2,
        };
        let out = arena.step(5_000, &[], &[fire]);
        // A queued fire intent is an `Alert` trigger and an accepted **shot** is an
        // `Engaged` one (doc 16 §3.1's table), and this fixture's tank carries no
        // weapon: the order is queued, phase 4 refuses it `NoWeapon`, and the tempo
        // stops at the trigger the queue itself is.
        assert_eq!(out.tempo, Tempo::Alert);
        let mut journal = Log::default();
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert!(
            log.iter().any(|event| event.kind
                == EventKind::Tempo {
                    from: Tempo::Quiet,
                    to: Tempo::Alert
                }),
            "the tempo change was not journalled: {log:?}"
        );
    }

    #[test]
    fn the_position_summary_keeps_its_cadence_in_arena_time() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        let mut wall = 0_i64;
        for _ in 0..4 {
            wall += 5_000;
            arena.step(wall, &[], &[]);
        }
        assert_eq!(arena.now_ms, 20_000);
        let mut journal = Log::default();
        let log = journal.events(&mut arena);
        let summaries: Vec<i64> = log
            .iter()
            .filter(|event| event.kind.as_str() == "position_summary")
            .map(|event| event.t_ms)
            .collect();
        assert_eq!(summaries, vec![5_000, 10_000, 15_000, 20_000]);
        // The next one is scheduled, always. The four above are already journalled,
        // so it is the *schedule* and not the log that says the cadence continues —
        // and `next_t_ms` cannot say it since 16b cut F.3 gives every unit a sensor
        // pass, which on this fixture falls nearer (unit #4's, at 20,000 ms plus
        // its stagger; 16b cut U.1 made that `stagger_ms`, 482 ms at seed 2,350).
        let offset = crate::arena::rng::stagger_ms(arena.seed, A, 1_000);
        assert!(
            offset > 0,
            "the fixture's precondition: #4's pass is not on a second"
        );
        assert!(
            arena
                .due
                .iter()
                .any(|event| event.t_ms == 25_000 && event.kind == DueKind::PositionSummary),
            "the next position summary is not on the schedule: {:?}",
            arena.due.iter().collect::<Vec<_>>()
        );
        assert_eq!(
            arena.due.next_t_ms(),
            Some(20_000 + offset),
            "unit #4's own stagger"
        );
    }

    /// **16b cut S.1, the pulse end to end**: the sensor pass is due in phase
    /// 3(c), the contact it mints reaches the pilot's frame, the tempo answers to
    /// it, and the counters say what it cost.
    #[test]
    fn a_sensor_pass_runs_in_the_pulse_and_its_contact_reaches_the_frame_and_the_tempo() {
        use crate::arena::test_support::scout;
        const B: ObjId = ObjId(9);
        // Two tanks that see and shoot, four cells apart on the 8 by 8 window.
        // **16b cut E.1** (16c `T-32`): the derived `tempo.alert_range_cm` is the
        // lesser of twice the cannon's 2.5 km and the window's own diagonal, and
        // on an 8 by 8 window of 1,000 cm cells the diagonal is the lesser by a
        // long way — `isqrt(8² + 8²) = 11` cells. A contact 40 m away is still
        // well inside it.
        let mut arena = crate::arena::test_support::arena_at_epoch(
            vec![
                scout(A, Team::Red, pose(500, 500)),
                scout(B, Team::Blue, pose(4_500, 500)),
            ],
            1,
        );
        assert_eq!(arena.alert_range_cm, 11_000, "the window's own diagonal");
        // The opening pulse establishes the wall reference and integrates nothing,
        // so no pass has run yet.
        let out = arena.step(0, &[], &[]);
        assert_eq!(out.perception.passes, 0);
        assert!(arena.contacts.is_empty());
        // One second of arena time: both staggered passes (at their seeded
        // offsets inside the first second, `rng::stagger_ms`; 4 ms and 9 ms
        // under `id % pass_ms` before 16b cut U.1) run, each holds the other,
        // and the tempo rises to `Alert`.
        let out = arena.step(1_000, &[], &[]);
        assert_eq!(out.perception.passes, 2, "{:?}", out.perception);
        assert_eq!(out.perception.detected, 2);
        assert_eq!(out.perception.traces, 2);
        // **16b cut J.1**: two 5-cell lines between cells (0,0) and (4,0) of the
        // flat window, three interior cells each and nothing blocking — six. The
        // walk counted the endpoints too and read ten. **16b cut J.2**: the flat
        // 8 by 8 window is one partly filled 16-block with no flag and no
        // elevation, so each of the two lines is proved clear at one block read
        // and no cell is read at all.
        assert_eq!(
            (out.perception.cell_probes, out.perception.horizon_probes),
            (0, 2)
        );
        assert_eq!(out.tempo, Tempo::Alert);
        assert_eq!(arena.contacts.len(), 2);
        // The pilot of A is told about B, at the right bearing and range, and about
        // nothing else.
        let frame = out
            .frames
            .iter()
            .find(|(pilot, _)| *pilot == ObjId(1000))
            .map(|(_, frame)| frame)
            .unwrap_or_else(|| panic!("A's pilot was sent a frame: {:?}", out.frames));
        assert_eq!(frame.contacts.len(), 1);
        let contact = frame
            .contacts
            .first()
            .copied()
            .unwrap_or_else(|| panic!("one contact"));
        assert_eq!(contact.class, crate::arena::state::Classification::Unknown);
        assert_eq!(contact.bearing_mdeg, 90_000, "B is due east of A");
        assert_eq!(contact.range_cm, 4_000);
        // And it is the id `draw_contact_id` mints for the pair, so a pilot's
        // `LOCK` resolves through `ShadowContacts::resolve`.
        assert_eq!(
            contact.id,
            crate::arena::rng::draw_contact_id(arena.seed, A, B)
        );
        assert!(arena.contacts.resolve(A, contact.id).is_some());
        assert!(
            arena.contacts.resolve(B, contact.id).is_none(),
            "an id minted for A is not B's"
        );
        // Three more seconds carry both past `classify_after_ms`, and the classify
        // draw is in the ledger against the label doc 16 §3.5 pins.
        let mut journal = Log::default();
        let mut wall = 1_000_i64;
        for _ in 0..4 {
            wall += 1_000;
            journal.absorb(&arena.step(wall, &[], &[]));
        }
        let log = journal.events(&mut arena);
        let classified: Vec<&LedgerEvent> = log
            .iter()
            .filter(|event| event.kind.as_str() == "classified")
            .collect();
        assert_eq!(classified.len(), 2, "{:?}", log);
        for event in classified {
            // 16b cut M.3: a list, and a sensor classification makes one draw.
            let [(label, _)] = event.draws.as_slice() else {
                panic!("a classification carries its one draw: {event:?}");
            };
            assert!(label.starts_with("classify:"), "{label}");
        }
    }

    #[test]
    fn a_pulse_past_the_clamp_journals_one_lagged_notice() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        // Quiet is a 5,000 ms period and `max_catch_up_steps` is 4, so 20,000 ms
        // is the clamp and 20,001 is lag.
        let out = arena.step(30_000, &[], &[]);
        assert!(out.lagged);
        assert_eq!(arena.now_ms, 20_000, "the idle wall time is skipped");
        let mut journal = Log::default();
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert!(
            log.iter()
                .any(|event| event.kind == EventKind::Lagged { dt_ms: 20_000 })
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut D.2 — phase 2 speaks (`B-03`)
    // -----------------------------------------------------------------------

    /// The one-tank fixture, under a name a per-case binding does not shadow.
    fn fresh() -> Arena {
        arena()
    }

    /// Every notice this pulse staged for the fixture's one pilot.
    fn told(out: &StepOutput) -> Vec<FrameNotice> {
        out.frames
            .iter()
            .filter(|(pilot, _)| *pilot == ObjId(1000))
            .flat_map(|(_, frame)| frame.notices.iter().cloned())
            .collect()
    }

    /// The refusals among them, as `(order word, refusal, weapon)`.
    fn refusals(out: &StepOutput) -> Vec<(String, Refusal, Option<String>)> {
        told(out)
            .into_iter()
            .filter_map(|notice| match notice {
                FrameNotice::Refused {
                    order,
                    refusal,
                    weapon,
                    ..
                } => Some((
                    order.to_string(),
                    refusal,
                    weapon.map(|weapon| weapon.to_string()),
                )),
                _ => None,
            })
            .collect()
    }

    fn order(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
        Intent {
            unit,
            channel: kind.channel_required(),
            epoch: 1,
            kind,
            seq,
        }
    }

    /// **16b cut D.2** (`B-03`): a wreck answers every order, in the order's own
    /// word, and the word is [`IntentKind::as_str`]'s in both phases.
    #[test]
    fn d2_phase_two_names_the_order_it_refused() {
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.limits.wreck = true;
        }
        let out = arena.step(
            5_000,
            &[],
            &[
                order(A, IntentKind::Startup, 1),
                order(A, IntentKind::Heading { mdeg: 90_000 }, 2),
                order(A, IntentKind::Speed { cms: 100 }, 3),
                order(
                    A,
                    IntentKind::Lock {
                        contact: ContactId(7),
                    },
                    4,
                ),
                order(
                    A,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                    },
                    5,
                ),
            ],
        );
        assert_eq!(
            refusals(&out),
            vec![
                ("startup".to_owned(), Refusal::Wreck, None),
                ("heading".to_owned(), Refusal::Wreck, None),
                ("speed".to_owned(), Refusal::Wreck, None),
                ("lock".to_owned(), Refusal::Wreck, None),
                ("fire".to_owned(), Refusal::Wreck, Some("cannon".to_owned())),
            ],
            "the five order words are `IntentKind::as_str`'s, and only `fire` names a weapon"
        );
    }

    /// The other three of the four phase 2 says out loud, each on its own order.
    #[test]
    fn d2_phase_two_says_the_motion_limit_the_cold_tank_and_the_full_queue() {
        // (a) motion disabled, on a live tank with its power up.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.power = Power::Online;
            unit.limits.motion_disabled = true;
        }
        let out = arena.step(
            5_000,
            &[],
            &[
                order(A, IntentKind::Heading { mdeg: 90_000 }, 1),
                order(A, IntentKind::Speed { cms: 100 }, 2),
            ],
        );
        assert_eq!(
            refusals(&out),
            vec![
                ("heading".to_owned(), Refusal::MotionDisabled, None),
                ("speed".to_owned(), Refusal::MotionDisabled, None),
            ]
        );

        // (b) the cold tank, which is the refusal doc 16 §3.3 names.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        let out = arena.step(5_000, &[], &[order(A, IntentKind::Heading { mdeg: 0 }, 1)]);
        assert_eq!(
            refusals(&out),
            vec![("heading".to_owned(), Refusal::PowerNotOnline, None)]
        );

        // (c) the ninth discrete order of one burst. `max_pending_per_unit` is
        // eight a seat a unit per arena second (16b cut M.1, `T-82`), and all
        // nine are this seat's orders in arena second 0 (phase 2 runs before the
        // clock moves), so the ninth is refused once however fast the arena is
        // pulsing — which is the whole of why this is staged rather than
        // dropped in silence.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        let burst: Vec<Intent> = (0..9)
            .map(|n: u64| order(A, IntentKind::Startup, n + 1))
            .collect();
        let out = arena.step(5_000, &[], &burst);
        assert_eq!(
            refusals(&out),
            vec![("startup".to_owned(), Refusal::QueueFull, None)],
            "one refusal for one dropped order"
        );
    }

    /// **16b cut M.1** (`T-82`): (d) the bound is a rate in ARENA seconds,
    /// counted on the seat that gave the order.
    ///
    /// Phase 2 runs at the arena time the previous pulse ended on, so a pulse
    /// stepped to wall 500 applies its orders at arena 0, and one stepped to wall
    /// 1,500 applies them at arena 1,000.
    #[test]
    fn m1_the_order_bound_is_a_rate_per_arena_second_counted_on_the_seat() {
        // Eight in arena second 0, then a ninth in a later pulse still inside
        // second 0: refused, although the queue was drained in between.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        let eight: Vec<Intent> = (0..8)
            .map(|n: u64| order(A, IntentKind::Startup, n + 1))
            .collect();
        let out = arena.step(500, &[], &eight);
        assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));
        assert!(arena.pending.is_empty(), "phase 4 drained the eight");
        let out = arena.step(900, &[], &[order(A, IntentKind::Startup, 9)]);
        assert_eq!(arena.now_ms, 900);
        assert_eq!(
            refusals(&out),
            vec![("startup".to_owned(), Refusal::QueueFull, None)],
            "the ninth order inside one arena second is refused, whatever the pulse rate"
        );

        // The same ninth in a pulse whose phase 2 runs at arena 1,000: accepted.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        arena.step(500, &[], &eight);
        arena.step(1_000, &[], &[]);
        let out = arena.step(1_500, &[], &[order(A, IntentKind::Startup, 9)]);
        assert!(
            refusals(&out).is_empty(),
            "a new arena second is a new count: {:?}",
            refusals(&out)
        );

        // The machine's own orders are never counted.
        let mut arena = fresh();
        for n in 0..9_u64 {
            assert_eq!(
                apply_intent(
                    &mut arena,
                    &order(A, IntentKind::Startup, n + 1),
                    IntentSource::Machine
                ),
                IntentOutcome::Queued,
                "machine order {n}"
            );
        }

        // A seat's order to a tank the reflex controller drives is still a seat's:
        // a gunner left on a machine-driven hull is bounded like anybody.
        let mut arena = fresh();
        arena.set_controller(
            A,
            crate::arena::state::Controller::Reflex,
            crate::arena::reflex::ReflexState::Withdraw,
        );
        let outcomes: Vec<IntentOutcome> = (0..9_u64)
            .map(|n| {
                apply_intent(
                    &mut arena,
                    &order(A, IntentKind::Startup, n + 1),
                    IntentSource::Seat,
                )
            })
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| **o == IntentOutcome::Queued)
                .count(),
            8
        );
        assert_eq!(
            outcomes.last(),
            Some(&IntentOutcome::Refused(Refusal::QueueFull)),
            "the ninth seat order on a reflex-driven unit is refused"
        );
    }

    /// **The five phase 2 does not say**, each for its own reason — see
    /// [`Arena::phase_two_tells`].
    #[test]
    fn d2_phase_two_is_silent_where_there_is_nobody_to_tell() {
        // `Epoch`: doc 16 §3.1's `FIRE` then `EXIT` burst, working as written.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        arena.epochs.raise(A, Channel::Helm, 2);
        let out = arena.step(5_000, &[], &[order(A, IntentKind::Startup, 1)]);
        assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

        // `Channel`: a routing fault, not a pilot's order.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        let out = arena.step(
            5_000,
            &[],
            &[Intent {
                unit: A,
                channel: Channel::Fire,
                epoch: 1,
                kind: IntentKind::Startup,
                seq: 1,
            }],
        );
        assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

        // `Unknown`: there is no unit, so there is no pilot.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        let out = arena.step(5_000, &[], &[order(ObjId(4_242), IntentKind::Startup, 1)]);
        assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

        // `Linkdead`: the seat has nobody at it.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.linkdead = true;
        }
        let out = arena.step(5_000, &[], &[order(A, IntentKind::Startup, 1)]);
        assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

        // `NotReflex`, and every `Order` whatever it is refused for: doctrine is
        // a commander's and a tank's pilots are not its audience.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        let doctrine = crate::deliberation::order::Order {
            squad: crate::deliberation::squad::SquadId::parse("red-1").expect("a squad id"),
            objective: crate::deliberation::order::Objective::Hold,
            target: None,
            horizon_ms: 60_000,
            constraints: crate::deliberation::order::Constraints::default(),
            ttl_ms: 30_000,
            issued_ms: 1,
            issuer: None,
            spot: None,
        };
        let out = arena.step(
            5_000,
            &[],
            &[order(A, IntentKind::Order(Box::new(doctrine)), 1)],
        );
        assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

        // **16b cut Z.2**: and every `ReleaseOrder`, for the same reason. The
        // world fans a lead's `RELEASE` to every unit of the squad's roster, and
        // a roster tank that is a wreck is refused `Wreck` — one of the refusals
        // phase 2 otherwise tells — before anything asks whether a machine
        // flies it; that is nobody's news, so nothing is staged.
        let mut arena = fresh();
        arena.step(0, &[], &[]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.limits.wreck = true;
        }
        let released = order(
            A,
            IntentKind::ReleaseOrder {
                squad: crate::deliberation::squad::SquadId::parse("red-1").expect("a squad id"),
                issuer: Some(A),
            },
            1,
        );
        assert_eq!(
            crate::arena::intents::apply_intent(
                &mut arena.clone(),
                &released,
                crate::arena::intents::IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Wreck),
            "the witness is a release refused for a reason phase 2 tells elsewhere"
        );
        let out = arena.step(5_000, &[], &[released]);
        assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));
    }

    /// **16b cut D.2**: the pilot's own two limits ride the frame, so the cockpit
    /// can refuse an order to a dead tank at once instead of a pulse later.
    #[test]
    fn d2_the_frame_carries_the_wreck_and_the_motion_limit() {
        let mut arena = fresh();
        let out = arena.step(0, &[], &[]);
        let frame = out
            .frames
            .first()
            .map(|(_, frame)| frame.clone())
            .unwrap_or_else(|| panic!("the opening frame"));
        assert!(!frame.wreck);
        assert!(!frame.immobile);
        // Both are compared, so the flip sends a frame at once.
        let mut dead = frame.clone();
        dead.wreck = true;
        assert!(!frame.same_as(&dead, 1_000));
        let mut stuck = frame.clone();
        stuck.immobile = true;
        assert!(!frame.same_as(&stuck, 1_000));
        if let Some(unit) = arena.unit_mut(A) {
            unit.limits.wreck = true;
            unit.limits.motion_disabled = true;
        }
        let out = arena.step(5_000, &[], &[]);
        let frame = out
            .frames
            .first()
            .map(|(_, frame)| frame.clone())
            .unwrap_or_else(|| panic!("the flip sends a frame"));
        assert!(frame.wreck && frame.immobile);
    }

    // -----------------------------------------------------------------------
    // 16b cut D.2 — one sentence for a round on a spent section (`B-15`)
    // -----------------------------------------------------------------------

    /// The gunner and the victim of the spent-section cases: two tanks, **nobody
    /// seated on either**, because the whole point of the window is that it is
    /// written whether or not anybody is there to read it.
    fn spent_rig() -> Arena {
        crate::arena::test_support::arena_of(vec![
            tank(A, Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(1_500, 5_500)),
        ])
    }

    /// One `hit` line, as the impact journals it.
    fn hit(shot_seq: u64, section: &str) -> EventKind {
        EventKind::Hit {
            shot_seq,
            unit: A,
            target: ObjId(9),
            section: SmolStr::new(section),
            facet: SmolStr::new("front"),
            armor_section: SmolStr::new(section),
            rounds: 1,
            damage: 0,
            armor_left: 0,
            internals_left: 0,
            consequences: Vec::new(),
            cause: SmolStr::new(format!("shot:{shot_seq}")),
        }
    }

    /// Stage the notices one `hit` owes, with `shot_seq` marked spent or not —
    /// the same call `Arena::substep`'s staging scan makes, and the scratch
    /// filled the way `Arena::apply_hit` fills it in the same substep.
    fn stage_hit(arena: &mut Arena, shot_seq: u64, section: &str, spent: bool, t_ms: i64) {
        if spent {
            arena.spent_hits.insert(shot_seq);
        }
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        arena.stage_pilot_notice(t_ms, &hit(shot_seq, section), &mut in_flight);
    }

    /// What the victim has been told since the last drain.
    fn victims_notices(arena: &mut Arena) -> Vec<FrameNotice> {
        arena.pending_notices.remove(&ObjId(9)).unwrap_or_default()
    }

    /// **The scratch never survives the substep it was filled in** (16b cut D.2),
    /// which is the whole of why it is not folded into the state hash.
    #[test]
    fn d2_the_spent_shot_scratch_is_drained_by_the_staging_scan() {
        let mut arena = spent_rig();
        stage_hit(&mut arena, 1, "hull", true, 0);
        assert!(
            arena.spent_hits.is_empty(),
            "the `hit` arm did not drain the scratch: {:?}",
            arena.spent_hits
        );
        // And a round on a live section leaves nothing in it either.
        stage_hit(&mut arena, 2, "hull", false, 1_000);
        assert!(arena.spent_hits.is_empty());
    }

    /// **The window is arena time, and it is exactly `SPENT_NOTICE_MS`** (16b cut
    /// D.2): a second round at 4,999 ms is silent, one at 5,000 ms is said.
    #[test]
    fn d2_a_spent_section_is_said_once_every_five_seconds() {
        for (second, said) in [(4_999_i64, false), (5_000, true)] {
            let mut arena = spent_rig();
            stage_hit(&mut arena, 1, "hull", true, 0);
            assert_eq!(
                victims_notices(&mut arena),
                vec![FrameNotice::StruckSpent {
                    section: SmolStr::new("hull")
                }],
                "the first round on a spent section is always said"
            );
            stage_hit(&mut arena, 2, "hull", true, second);
            let told = victims_notices(&mut arena);
            assert_eq!(
                told.is_empty(),
                !said,
                "a second round at {second} ms: {told:?}"
            );
        }
        // A DIFFERENT section is a different key, so it is said at once.
        let mut arena = spent_rig();
        stage_hit(&mut arena, 1, "hull", true, 0);
        let _ = victims_notices(&mut arena);
        stage_hit(&mut arena, 2, "turret", true, 1_000);
        assert_eq!(
            victims_notices(&mut arena),
            vec![FrameNotice::StruckSpent {
                section: SmolStr::new("turret")
            }]
        );
    }

    /// **The decision and the write happen with nobody seated** (16b cut D.2),
    /// which is what makes the fold rate-independent: a pilot who sits down four
    /// seconds into the window reads nothing until it runs out.
    #[test]
    fn d2_the_spent_window_is_kept_whether_or_not_anybody_is_seated() {
        let mut arena = spent_rig();
        assert!(arena.pilots.is_empty(), "nobody is seated on either tank");
        stage_hit(&mut arena, 1, "hull", true, 0);
        assert_eq!(
            arena
                .spent_said
                .get(&(ObjId(9), ObjId(9), SmolStr::new("hull")))
                .copied(),
            Some(0),
            "the entry was written with no frame to carry it: {:?}",
            arena.spent_said
        );
        // Nothing is keyed on the gunner: he holds no contact, so he read what he
        // always reads and earned no window (`Arena::spent_said`).
        assert!(
            arena
                .spent_said
                .keys()
                .all(|(told, _, _)| *told == ObjId(9)),
            "{:?}",
            arena.spent_said
        );
        arena.pending_notices.clear();
        // Somebody takes the seat four seconds in.
        arena.pilots.push(crate::arena::state::Pilot {
            unit: ObjId(9),
            pilot: ObjId(1_001),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        });
        stage_hit(&mut arena, 2, "hull", true, 4_000);
        assert!(
            victims_notices(&mut arena).is_empty(),
            "the window the empty pod kept was not honoured"
        );
        stage_hit(&mut arena, 3, "hull", true, 5_000);
        assert_eq!(
            victims_notices(&mut arena),
            vec![FrameNotice::StruckSpent {
                section: SmolStr::new("hull")
            }]
        );
    }

    /// **Entries are pruned on touch and never by a pulse** (16b cut D.2): a
    /// per-pulse sweep would make the table — which the state hash folds — a
    /// function of the cadence, which is exactly what rate independence refuses.
    #[test]
    fn d2_the_spent_window_is_pruned_on_touch_and_never_by_a_pulse() {
        let mut arena = spent_rig();
        let stale = (ObjId(4_242), ObjId(4_243), SmolStr::new("turret"));
        arena.spent_said.insert(stale.clone(), 0);
        // Ten seconds of pulses, which is twice the window: the entry stands,
        // because nothing touched the table.
        let mut wall = 0_i64;
        for _ in 0..7 {
            wall += 2_000;
            arena.step(wall, &[], &[]);
        }
        assert!(arena.now_ms >= 10_000);
        assert!(
            arena.spent_said.contains_key(&stale),
            "a pulse swept the table: {:?}",
            arena.spent_said
        );
        // One spent round touches it, and the stale entry goes with that touch.
        let now = arena.now_ms;
        stage_hit(&mut arena, 1, "hull", true, now);
        assert!(
            !arena.spent_said.contains_key(&stale),
            "the touch did not prune: {:?}",
            arena.spent_said
        );
    }

    /// **A fresh section is unchanged** (16b cut D.2): the round that finds
    /// armour still says what it always said, and so does the one that finishes
    /// the section.
    #[test]
    fn d2_a_round_on_a_live_section_says_what_it_always_said() {
        let mut arena = spent_rig();
        stage_hit(&mut arena, 1, "hull", false, 0);
        let told = victims_notices(&mut arena);
        assert_eq!(told.len(), 1);
        assert!(
            matches!(told.first(), Some(FrameNotice::Struck { section, .. }) if section == "hull"),
            "{told:?}"
        );
        assert!(
            arena.spent_said.is_empty(),
            "a live section wrote a window entry: {:?}",
            arena.spent_said
        );
        // And the section's own loss line is untouched by any of this.
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        arena.stage_pilot_notice(
            0,
            &EventKind::SectionDestroyed {
                unit: ObjId(9),
                section: SmolStr::new("hull"),
                // 16b cut D.4 (`T-08`): nothing shot it off.
                by: None,
                consequences: Vec::new(),
            },
            &mut in_flight,
        );
        let told = victims_notices(&mut arena);
        assert!(
            matches!(
                told.first(),
                Some(FrameNotice::SectionLost { section, .. }) if section == "hull"
            ),
            "{told:?}"
        );
    }

    /// **The victim's own two lines** (16b cut D.2, `B-03`): staged off the
    /// ledger's own events, to the victim and to nobody else.
    #[test]
    fn d2_the_victim_is_told_when_his_own_tank_goes() {
        let mut arena = spent_rig();
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        arena.stage_pilot_notice(
            0,
            &EventKind::UnitDisabled { unit: ObjId(9) },
            &mut in_flight,
        );
        assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDisabled]);
        arena.stage_pilot_notice(
            0,
            // 16b cut D.4 (`T-08`): nobody's round did it, so nobody is credited.
            &EventKind::UnitDestroyed {
                unit: ObjId(9),
                by: None,
            },
            &mut in_flight,
        );
        assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDestroyed]);
        assert!(
            arena.pending_notices.is_empty(),
            "somebody else was told: {:?}",
            arena.pending_notices
        );
        // With the shooter on the event the gunner's kill line comes first, and
        // the victim's own line after it. **16b cut D.4**: off `by` and no longer
        // off the substep's in-flight scratch, which is left empty here on
        // purpose — a stale scratch must not be what credits a kill.
        arena.stage_pilot_notice(
            0,
            &EventKind::UnitDestroyed {
                unit: ObjId(9),
                by: Some(A),
            },
            &mut in_flight,
        );
        assert_eq!(
            arena.pending_notices.remove(&A).unwrap_or_default(),
            vec![FrameNotice::Kill { contact: None }]
        );
        assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDestroyed]);
    }

    // -----------------------------------------------------------------------
    // 16b cut D.4 — the gunner's arms read the shooter off the event (`T-08`)
    // -----------------------------------------------------------------------

    /// The gunner holding the victim **classified and read**, which is the state
    /// both of `T-08`'s stale readers need: the section word is gated on the
    /// classification, and `observe_section_lost` writes into the reading.
    fn watching_gunner() -> Arena {
        let mut arena = spent_rig();
        let id = crate::arena::rng::draw_contact_id(arena.seed, A, ObjId(9));
        let plot = crate::arena::state::SeenPose::default();
        arena.contacts.insert(
            A,
            id,
            crate::arena::state::ShadowContact {
                seen_condition: Some(crate::arena::condition::Reading {
                    at_ms: 0,
                    condition: crate::arena::condition::Condition::default()
                        .with_band(0, crate::arena::condition::ConditionBand::Green),
                }),
                ..crate::arena::state::ShadowContact::sighted(
                    ObjId(9),
                    Classification::Classified { team: Team::Blue },
                    0,
                    plot,
                )
            },
        );
        arena
    }

    /// What the gunner has been told since the last drain.
    fn gunners_notices(arena: &mut Arena) -> Vec<FrameNotice> {
        arena.pending_notices.remove(&A).unwrap_or_default()
    }

    /// **`T-08`**: a `section_destroyed` or a `unit_destroyed` that no round did
    /// is credited to nobody, even when a round resolved on the same target
    /// earlier in the same substep — and it writes no hashed contact state.
    #[test]
    fn d4_a_destroy_no_round_did_is_credited_to_nobody() {
        let mut arena = watching_gunner();
        let mut in_flight: Option<(ObjId, ObjId)> = None;

        // The round arrives and resolves. The scratch is set by the impact and
        // cleared by the outcome, which is the whole of `T-08`'s fix.
        arena.stage_pilot_notice(
            0,
            &EventKind::Impact {
                shot_seq: 1,
                unit: A,
                target: ObjId(9),
            },
            &mut in_flight,
        );
        assert_eq!(in_flight, Some((A, ObjId(9))));
        arena.stage_pilot_notice(0, &hit(1, "hull"), &mut in_flight);
        assert_eq!(in_flight, None, "the `hit` did not clear the pair");
        let _ = gunners_notices(&mut arena);
        let _ = victims_notices(&mut arena);
        let settled = state_hash(&arena);

        // Now something that is not that round takes a section off the same
        // target, later in the same substep.
        arena.stage_pilot_notice(
            0,
            &EventKind::SectionDestroyed {
                unit: ObjId(9),
                section: SmolStr::new("hull"),
                by: None,
                consequences: Vec::new(),
            },
            &mut in_flight,
        );
        assert!(
            gunners_notices(&mut arena).is_empty(),
            "the earlier shooter was credited with a section nobody shot off"
        );
        assert_eq!(
            state_hash(&arena),
            settled,
            "`observe_section_lost` wrote hashed contact state for a round that had resolved"
        );
        // The victim's own line is untouched: it is a fact about their tank.
        assert!(
            matches!(
                victims_notices(&mut arena).first(),
                Some(FrameNotice::SectionLost { section, .. }) if section == "hull"
            ),
            "the victim was not told"
        );

        arena.stage_pilot_notice(
            0,
            &EventKind::UnitDestroyed {
                unit: ObjId(9),
                by: None,
            },
            &mut in_flight,
        );
        assert!(
            gunners_notices(&mut arena).is_empty(),
            "a kill was credited to a round that had already resolved"
        );
        assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDestroyed]);
    }

    /// **And with the shooter on the event the two lines are exactly as they
    /// were** (16b cut D.4, `T-08`): the fix is about *where* the shooter is read
    /// from and not about what is said.
    #[test]
    fn d4_a_destroy_a_round_did_still_names_the_gunner_and_writes_his_reading() {
        let mut arena = watching_gunner();
        // The scratch is deliberately empty: nothing here depends on it any more.
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        let before = state_hash(&arena);
        arena.stage_pilot_notice(
            0,
            &EventKind::SectionDestroyed {
                unit: ObjId(9),
                section: SmolStr::new("hull"),
                by: Some(A),
                consequences: Vec::new(),
            },
            &mut in_flight,
        );
        assert!(
            matches!(
                gunners_notices(&mut arena).first(),
                Some(FrameNotice::TargetSectionLost { section, .. }) if section == "hull"
            ),
            "the gunner was told nothing about the section he took off"
        );
        assert_ne!(
            state_hash(&arena),
            before,
            "the gunner's own reading was not written"
        );
        let _ = victims_notices(&mut arena);

        arena.stage_pilot_notice(
            0,
            &EventKind::UnitDestroyed {
                unit: ObjId(9),
                by: Some(A),
            },
            &mut in_flight,
        );
        assert!(
            matches!(
                gunners_notices(&mut arena).first(),
                Some(FrameNotice::Kill { contact: Some(_) })
            ),
            "the gunner was not told he had finished the tank"
        );
    }

    /// **The scratch serves `miss` alone now** (16b cut D.4, `T-08`): it is set by
    /// the impact, read by the outcome, and `None` afterwards either way.
    #[test]
    fn d4_the_in_flight_pair_is_cleared_by_a_hit_and_by_a_miss() {
        let mut arena = watching_gunner();
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        arena.stage_pilot_notice(
            0,
            &EventKind::Impact {
                shot_seq: 1,
                unit: A,
                target: ObjId(9),
            },
            &mut in_flight,
        );
        arena.stage_pilot_notice(
            0,
            &EventKind::Miss {
                shot_seq: 1,
                reason: SmolStr::new("dispersion"),
            },
            &mut in_flight,
        );
        assert!(
            matches!(
                gunners_notices(&mut arena).first(),
                Some(FrameNotice::Miss { .. })
            ),
            "the miss was not said"
        );
        assert_eq!(in_flight, None, "the `miss` did not clear the pair");

        // And a second `miss` with nothing in flight says nothing at all.
        arena.stage_pilot_notice(
            0,
            &EventKind::Miss {
                shot_seq: 2,
                reason: SmolStr::new("dispersion"),
            },
            &mut in_flight,
        );
        assert!(
            gunners_notices(&mut arena).is_empty(),
            "a miss with no impact before it named a shooter"
        );
    }

    /// **16b cut AE.2**: a miss keeps its reason for a target the gunner holds,
    /// and carries none for one he does not — `moved`, `blocked` and `cover` are
    /// facts about where the target is that his sensors have not earned. `Hit.`
    /// on an unheld target is untouched.
    #[test]
    fn ae2_a_miss_on_a_target_the_gunner_does_not_hold_carries_no_reason() {
        let mut arena = watching_gunner();
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        let impact = |target: ObjId| EventKind::Impact {
            shot_seq: 1,
            unit: A,
            target,
        };
        let miss = || EventKind::Miss {
            shot_seq: 1,
            reason: SmolStr::new(crate::arena::damage::BLOCKED),
        };
        // Held: the reason, as ever.
        arena.stage_pilot_notice(0, &impact(ObjId(9)), &mut in_flight);
        arena.stage_pilot_notice(0, &miss(), &mut in_flight);
        assert!(
            matches!(
                gunners_notices(&mut arena).first(),
                Some(FrameNotice::Miss { contact: Some(_), reason })
                    if reason == crate::arena::damage::BLOCKED
            ),
            "a held target's miss keeps its reason"
        );
        // Not held: no contact and no reason.
        arena.stage_pilot_notice(0, &impact(ObjId(77)), &mut in_flight);
        arena.stage_pilot_notice(0, &miss(), &mut in_flight);
        assert_eq!(
            gunners_notices(&mut arena),
            vec![FrameNotice::Miss {
                contact: None,
                reason: SmolStr::new_static(""),
            }],
            "an unheld target's miss says nothing of why"
        );
        // And the unheld hit is the bare `Hit.` it always was.
        arena.stage_pilot_notice(0, &impact(ObjId(77)), &mut in_flight);
        arena.stage_pilot_notice(
            0,
            &EventKind::Hit {
                shot_seq: 1,
                unit: A,
                target: ObjId(77),
                section: SmolStr::new("hull"),
                facet: SmolStr::new("front"),
                armor_section: SmolStr::new("hull"),
                rounds: 1,
                damage: 10,
                armor_left: 10,
                internals_left: 10,
                consequences: Vec::new(),
                cause: SmolStr::new("test"),
            },
            &mut in_flight,
        );
        assert!(matches!(
            gunners_notices(&mut arena).first(),
            Some(FrameNotice::Hit {
                contact: None,
                section: None,
                ..
            })
        ));
    }

    // -----------------------------------------------------------------------
    // 16b cut M.2 — the duration bound, the substep count and the drain
    // -----------------------------------------------------------------------

    /// Two scouts four cells apart, each holding the other after its first
    /// pass, on a match whose duration bound is `max_duration_ms`.
    fn m2_pair(max_duration_ms: u32) -> Arena {
        use crate::arena::test_support::scout;
        m2_pair_with(
            max_duration_ms,
            scout(ObjId(9), Team::Blue, pose(4_500, 500)),
        )
    }

    /// The same with `blue` for the target — which must stand where
    /// [`m2_pair`]'s does, four cells east of `A`.
    fn m2_pair_with(max_duration_ms: u32, blue: crate::arena::state::CustodyUnit) -> Arena {
        use crate::arena::test_support::scout;
        Arena::new(
            crate::arena::test_support::custody(
                vec![scout(A, Team::Red, pose(500, 500)), blue],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig {
                max_duration_ms,
                ..MatchConfig::default()
            },
            &crate::config::SensorsConfig::default(),
        )
        .expect("two scouts")
    }

    /// Move the duration bound to `deadline`, as `Arena::new` would have put
    /// it: the one `MatchDeadline` on the heap, and the backstop's number.
    fn m2_move_deadline(arena: &mut Arena, deadline: i64) {
        let _ = arena.due.retain(|kind| *kind != DueKind::MatchDeadline);
        arena.due.schedule(deadline, DueKind::MatchDeadline);
        arena.match_cfg.max_duration_ms = u32::try_from(deadline).unwrap_or(u32::MAX);
    }

    /// **16b cut M.2** (16c `T-01`): a deadline that falls inside a substep ends
    /// the match at its own millisecond on a pulse whose end is later, and an
    /// impact due after it but inside the same substep is **not** applied — no
    /// `impact`, no `hit`, no `miss`, the round still in flight and still on the
    /// heap — so `match_ended { timeout }` is the last line and a round landing
    /// after the whistle cannot turn a timeout into an elimination.
    #[test]
    fn m2_a_deadline_inside_a_substep_ends_the_match_there_and_nothing_after_it_lands() {
        const B: ObjId = ObjId(9);
        let mut arena = m2_pair(60_000);
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        for wall in [1_000, 2_000, 3_000] {
            journal.absorb(&arena.step(wall, &[], &[]));
        }
        assert_eq!(arena.now_ms, 3_000);
        // A live cannon, locked on the contact its own pass holds.
        if let Some(state) = arena.unit_mut(A) {
            state.power = Power::Online;
        }
        let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
        assert_eq!(arena.resolve_lock(A, contact, 3_000), Ok(()));
        let seq = arena
            .resolve_fire(A, "cannon", 3_010)
            .unwrap_or_else(|refusal| panic!("the shot was refused: {refusal:?}"));
        let impact_at = arena.shot(seq).map_or(0, |shot| shot.impact_at_ms);
        assert!(
            impact_at > 3_011 && impact_at <= 3_250,
            "the fixture needs the impact inside the next substep: {impact_at}"
        );
        // The whistle one millisecond before the round arrives.
        let deadline = impact_at - 1;
        m2_move_deadline(&mut arena, deadline);
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(
            out.ended,
            Some(MatchEnd {
                reason: EndReason::Timeout,
                t_ms: deadline
            }),
            "the deadline's own millisecond, not the pulse's end (4,000)"
        );
        // 16c `T-10`: the first of four substeps ended it, so one ran.
        assert_eq!(out.substeps, 1);
        assert_eq!(arena.now_ms, 3_250, "the substep the deadline fell in");
        let log = journal.events(&mut arena);
        for word in ["impact", "hit", "miss"] {
            assert!(
                !log.iter().any(|event| event.kind.as_str() == word),
                "a round due after the deadline journalled `{word}`: {log:?}"
            );
        }
        assert!(arena.shot(seq).is_some(), "the round was resolved");
        assert!(
            arena
                .due
                .iter()
                .any(|event| event.kind == DueKind::Impact { shot_seq: seq }),
            "the impact left the heap"
        );
        assert_eq!(
            log.last().map(|event| (event.t_ms, event.kind.clone())),
            Some((
                deadline,
                EventKind::MatchEnded {
                    reason: EndReason::Timeout
                }
            )),
            "the timeout is the last line, at the deadline"
        );
    }

    /// Run `arena` to 3,000 and have `A` lock `ObjId(9)` and fire its cannon at
    /// 3,010, the way every T-01 test below does: the journal so far, the shot's
    /// sequence and the millisecond its round is due.
    fn m2_fire(arena: &mut Arena) -> (Log, u64, i64) {
        const B: ObjId = ObjId(9);
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        for wall in [1_000, 2_000, 3_000] {
            journal.absorb(&arena.step(wall, &[], &[]));
        }
        if let Some(state) = arena.unit_mut(A) {
            state.power = Power::Online;
        }
        let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
        assert_eq!(arena.resolve_lock(A, contact, 3_000), Ok(()));
        let seq = arena
            .resolve_fire(A, "cannon", 3_010)
            .unwrap_or_else(|refusal| panic!("the shot was refused: {refusal:?}"));
        let impact_at = arena.shot(seq).map_or(0, |shot| shot.impact_at_ms);
        (journal, seq, impact_at)
    }

    /// Every `match_ended` line in `log`, with its millisecond.
    fn m2_ends(log: &[LedgerEvent]) -> Vec<(i64, EventKind)> {
        log.iter()
            .filter(|event| matches!(event.kind, EventKind::MatchEnded { .. }))
            .map(|event| (event.t_ms, event.kind.clone()))
            .collect()
    }

    /// **The production order**: the match's own `MatchDeadline`, scheduled by
    /// `Arena::new` at sequence 2, is on the heap at `impact_at` with a lower
    /// sequence than the round's `Impact`, so it pops FIRST and the impact is
    /// popped under the horizon it shrank.
    fn m2_assert_the_deadline_pops_first(arena: &Arena, seq: u64, impact_at: i64) {
        let deadline = arena
            .due
            .iter()
            .find(|event| event.kind == DueKind::MatchDeadline)
            .map(|event| (event.t_ms, event.seq));
        let impact = arena
            .due
            .iter()
            .find(|event| event.kind == DueKind::Impact { shot_seq: seq })
            .map(|event| (event.t_ms, event.seq));
        assert_eq!(deadline.map(|(t_ms, _)| t_ms), Some(impact_at));
        assert_eq!(impact.map(|(t_ms, _)| t_ms), Some(impact_at));
        assert!(
            deadline.map(|(_, seq)| seq) < impact.map(|(_, seq)| seq),
            "the fixture needs the match's own deadline to pop first: {deadline:?} {impact:?}"
        );
    }

    /// **16b cut M.2** (16c `T-01`): an impact due at the deadline's **own**
    /// millisecond still lands, in the same due loop, before the timeout is
    /// settled — the horizon shrinks to the deadline, not below it.
    ///
    /// In the production order: the duration bound is the one `Arena::new`
    /// scheduled, so it pops BEFORE the impact on their shared millisecond, and
    /// the impact is popped under the horizon the deadline set. A horizon one
    /// millisecond short of the deadline, or a loop that stopped at the
    /// deadline's own arm, would leave this round unapplied.
    #[test]
    fn m2_an_impact_at_the_deadlines_own_millisecond_still_lands() {
        // Where the round lands, from a probe with the bound out of the way —
        // the bound decides nothing before it, so the two runs agree to there.
        let (_, _, impact_at) = m2_fire(&mut m2_pair(60_000));
        let bound = u32::try_from(impact_at).unwrap_or(u32::MAX);
        let mut arena = m2_pair(bound);
        let (mut journal, seq, landed_at) = m2_fire(&mut arena);
        assert_eq!(landed_at, impact_at, "the probe and the run agree");
        m2_assert_the_deadline_pops_first(&arena, seq, impact_at);
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert!(
            log.iter()
                .any(|event| event.kind.as_str() == "impact" && event.t_ms == impact_at),
            "the round on the deadline's own millisecond did not land: {log:?}"
        );
        assert!(arena.shot(seq).is_none(), "the round is still in flight");
        assert_eq!(
            out.ended.map(|end| (end.reason, end.t_ms)),
            Some((EndReason::Timeout, impact_at)),
            "a round that does not kill leaves the timeout standing"
        );
        assert_eq!(
            m2_ends(&log),
            vec![(
                impact_at,
                EventKind::MatchEnded {
                    reason: EndReason::Timeout
                }
            )],
            "one end, the timeout"
        );
    }

    /// **16b cut M.2** (16c `T-01`): **a kill on the deadline's own millisecond
    /// wins.** The same production order as the test above, with a target the
    /// round destroys: one hull section, its armour and internals less than one
    /// round's damage, facing the shooter, whose loss is `DestroyUnit`. The
    /// impact pops after the deadline on their shared millisecond, destroys the
    /// last blue unit, and elimination is settled before the timeout is — so the
    /// match ends by elimination, and no timeout line is journalled beside it.
    #[test]
    fn m2_a_kill_on_the_deadlines_own_millisecond_wins() {
        use crate::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
        use crate::arena::test_support::scout;
        let glass = || {
            let mut blue = scout(ObjId(9), Team::Blue, pose(4_500, 500));
            // Facing west, down the line of fire, so the round meets the one
            // facet `tactical_unit` has: `hull/front`, ten armour over eight
            // internals against the cannon's 120.
            blue.pose.heading_mdeg = 270_000;
            blue.profile = UnitProfile {
                bands: vec![BandProfile {
                    band: SmolStr::new("front"),
                    width_cm: 350,
                    height_cm: 250,
                    sections: vec![(SmolStr::new("hull"), 100)],
                }],
                sections: vec![SectionProfile {
                    id: SmolStr::new("hull"),
                    facets: vec![SmolStr::new("front")],
                    consequences: vec![Consequence::DestroyUnit],
                    struck_through: None,
                }],
            };
            blue
        };
        let (_, _, impact_at) = m2_fire(&mut m2_pair_with(60_000, glass()));
        let bound = u32::try_from(impact_at).unwrap_or(u32::MAX);
        let mut arena = m2_pair_with(bound, glass());
        let (mut journal, seq, landed_at) = m2_fire(&mut arena);
        assert_eq!(landed_at, impact_at, "the probe and the run agree");
        m2_assert_the_deadline_pops_first(&arena, seq, impact_at);
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert!(
            log.iter()
                .any(|event| event.kind.as_str() == "hit" && event.t_ms == impact_at),
            "the fixture needs the round to hit on the deadline: {log:?}"
        );
        assert!(
            arena.unit(ObjId(9)).is_some_and(|unit| unit.limits.wreck),
            "the round on the deadline did not destroy the target: {log:?}"
        );
        assert_eq!(
            out.ended.map(|end| end.reason),
            Some(EndReason::Elimination {
                winner: Some(Team::Red)
            }),
            "the kill on the whistle is an elimination"
        );
        let ends = m2_ends(&log);
        assert_eq!(
            ends.iter()
                .map(|(_, kind)| kind.clone())
                .collect::<Vec<_>>(),
            vec![EventKind::MatchEnded {
                reason: EndReason::Elimination {
                    winner: Some(Team::Red)
                }
            }],
            "exactly one end, the elimination, and no timeout beside it: {ends:?}"
        );
        assert_eq!(
            log.last().map(|event| event.kind.as_str()),
            Some("match_ended"),
            "nothing after the end"
        );
    }

    /// **16b cut M.2** (16c `T-01`): an elimination settled in the substep the
    /// deadline falls in wins, the order phase 5 always had. The wreck is the
    /// fixture's hand, placed so the side goes out in the very substep whose
    /// due loop pops the deadline. Every pulse's output is absorbed — the ending
    /// pulse flushes the ledger's tail into its own `ledger_chunks` — so the
    /// line count below is read off the whole black box.
    #[test]
    fn m2_an_elimination_in_the_deadlines_own_substep_wins() {
        const B: ObjId = ObjId(9);
        let mut arena = m2_pair(3_100);
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        journal.absorb(&arena.step(3_000, &[], &[]));
        assert!(arena.ended.is_none());
        if let Some(state) = arena.unit_mut(B) {
            state.limits.wreck = true;
        }
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(
            out.ended.map(|end| end.reason),
            Some(EndReason::Elimination {
                winner: Some(Team::Red)
            })
        );
        // The elimination is stamped at the end of the substep it was settled
        // in, 3,250 — the deadline's substep — which is `settle_elimination`'s
        // rule from before this cut, not the deadline's millisecond.
        assert_eq!(out.ended.map(|end| end.t_ms), Some(3_250));
        let log = journal.events(&mut arena);
        assert!(!log.is_empty(), "the journal read nothing");
        assert_eq!(
            m2_ends(&log),
            vec![(
                3_250,
                EventKind::MatchEnded {
                    reason: EndReason::Elimination {
                        winner: Some(Team::Red)
                    }
                }
            )],
            "exactly one end, the elimination: {log:?}"
        );
    }

    /// **16b cut AP.2** (16c `B-87`): the settle order Q.7 set — elimination
    /// first, `Spent` second — holds for the disarmed side. Both magazines are
    /// emptied and blue's gun shot off by the fixture's hand between two pulses,
    /// so the next substep finds both sides dry AND blue with no gun left: that
    /// is red's elimination win, never the `Spent` draw it was before this cut
    /// (`spent` counts an emptied weapon table as nothing to fire). One end in
    /// the black box, at the substep's end.
    #[test]
    fn ap2_a_side_disarmed_on_a_dry_substep_is_eliminated_not_spent() {
        const B: ObjId = ObjId(9);
        let mut arena = m2_pair(60_000);
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        journal.absorb(&arena.step(1_000, &[], &[]));
        assert!(arena.ended.is_none());
        for id in [A, B] {
            if let Some(state) = arena.unit_mut(id) {
                state.ammo.insert(SmolStr::new("cannon"), 0);
            }
        }
        let mut said: Vec<Notice> = Vec::new();
        arena.apply_consequence(
            B,
            &crate::arena::resolution::Consequence::RemoveWeapons {
                section: SmolStr::new("turret"),
            },
            1_000,
            None,
            &mut said,
        );
        assert!(arena.spent(), "both sides are dry: Q.7's predicate holds");
        let out = arena.step(2_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(
            out.ended.map(|end| end.reason),
            Some(EndReason::Elimination {
                winner: Some(Team::Red)
            }),
            "the side with no gun left is out, settled before Spent"
        );
        assert_eq!(out.ended.map(|end| end.t_ms), Some(1_250));
        let log = journal.events(&mut arena);
        assert_eq!(
            m2_ends(&log),
            vec![(
                1_250,
                EventKind::MatchEnded {
                    reason: EndReason::Elimination {
                        winner: Some(Team::Red)
                    }
                }
            )],
            "exactly one end, the elimination, and no Spent beside it: {log:?}"
        );
    }

    /// **16b cut M.2** (16c `T-10`): `substeps` is what ran. A pulse cut into
    /// four whose second substep ends the match reports two; the opening pulse
    /// reports none.
    #[test]
    fn m2_a_pulse_ended_in_its_second_of_four_substeps_reports_two() {
        let mut arena = m2_pair(1_500);
        let opening = arena.step(0, &[], &[]);
        assert_eq!(opening.substeps, 0);
        arena.step(1_000, &[], &[]);
        let out = arena.step(2_000, &[], &[]);
        assert_eq!(out.ended.map(|end| end.t_ms), Some(1_500));
        assert_eq!(out.substeps, 2);
        assert_eq!(arena.substep_hashes.len(), 2);
    }

    /// **16b cut M.2** (16c `B-51`): the opening pulse drains nothing. A unit
    /// whose stagger is nought has its first pass at `t = 0`, and it runs in the
    /// next pulse's first substep exactly as before the drain existed — the
    /// opening pulse runs no substep, so the drain's `ran > 0` guard leaves it
    /// alone.
    ///
    /// **16b cut U.1**: the stagger is `stagger_ms(seed, id, pass_ms)`, so the
    /// fixture's ids are re-picked for it — `#2355`'s is nought at seed 2,350 and
    /// `#9`'s is 189, inside the first substep — and both are asserted before
    /// the property is.
    #[test]
    fn m2_the_opening_pulse_drains_nothing_and_a_pass_at_zero_runs_next_pulse() {
        use crate::arena::test_support::scout;
        const EARLY: ObjId = ObjId(2_355);
        let mut arena = crate::arena::test_support::arena_at_epoch(
            vec![
                scout(EARLY, Team::Red, pose(500, 500)),
                scout(ObjId(9), Team::Blue, pose(4_500, 500)),
            ],
            1,
        );
        assert_eq!(arena.sensors_cfg.pass_ms, 1_000, "the default cadence");
        let stagger = |id: ObjId| crate::arena::rng::stagger_ms(arena.seed, id, 1_000);
        assert_eq!(stagger(EARLY), 0, "the precondition: a pass due at 0");
        assert!(
            (1..=arena.substep_len_ms()).contains(&stagger(ObjId(9))),
            "the precondition: a pass inside the first substep"
        );
        let opening = arena.step(0, &[], &[]);
        assert_eq!(opening.substeps, 0);
        assert_eq!(
            opening.perception.passes, 0,
            "no pass ran on the opening pulse"
        );
        assert!(
            arena.due.iter().any(|event| event.t_ms == 0
                && matches!(event.kind, DueKind::SensorPass { unit, .. } if unit == EARLY)),
            "the pass at zero left the heap"
        );
        let next = arena.step(250, &[], &[]);
        assert_eq!(next.substeps, 1);
        assert_eq!(
            next.perception.passes, 2,
            "the pass at 0 and the pass at 189 both ran in the first substep"
        );
    }

    /// **16b cut M.2** (16c `T-83`): the held loss is said at its own deadline,
    /// by a due event, and a re-detection inside the window cancels it — the
    /// `LossSettles` it left behind then fires into nothing.
    #[test]
    fn m2_a_held_loss_is_said_by_its_own_due_event_and_a_cancel_leaves_a_no_op() {
        const B: ObjId = ObjId(9);
        let mut arena = m2_pair(60_000);
        arena.step(0, &[], &[]);
        arena.step(1_000, &[], &[]);
        let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
        assert!(arena.contacts.resolve(A, contact).is_some());
        let mut in_flight = None;
        // A `lost` staged at 1,250: held until 2,250 and scheduled there.
        arena.stage_pilot_notice(
            1_250,
            &EventKind::Lost {
                viewer: A,
                contact,
                target: B,
            },
            &mut in_flight,
        );
        assert_eq!(
            arena.held_losses.get(&(A, contact)).map(|(at, _)| *at),
            Some(2_250)
        );
        assert!(
            arena.due.iter().any(|event| event.t_ms == 2_250
                && event.kind == DueKind::LossSettles { viewer: A, contact }),
            "the deadline is on the heap"
        );
        let _ = arena.pending_notices.remove(&A);
        // Said at its deadline, and not before.
        arena.settle_held_loss(A, contact, 2_000);
        assert!(
            arena.held_losses.contains_key(&(A, contact)),
            "a stale settle"
        );
        arena.settle_held_loss(A, contact, 2_250);
        assert!(!arena.held_losses.contains_key(&(A, contact)));
        assert!(
            arena
                .pending_notices
                .get(&A)
                .is_some_and(|told| told.iter().any(|notice| matches!(
                    notice,
                    FrameNotice::Contact {
                        kind: NoticeKind::Lost,
                        ..
                    }
                ))),
            "the loss was not said"
        );
    }

    /// **16b cut M.2** (16c `B-74`, and the review's guard on it): a cancelled
    /// loss whose row came back under another name is said as an identification
    /// **only** for a mesh insert that arrived told apart, and then once.
    ///
    /// Three rows a held `X1` can come back as, each run through the staging
    /// scan's two calls exactly as one substep would make them — the `detected`
    /// and then the `classified` a share journals:
    ///
    /// * a mesh insert, already `Classified` (`numbered` false): the cancel says
    ///   `identified` with `was: X1`, and the share's own `classified` carries no
    ///   `was` and renders nothing — one line;
    /// * this platform's own pass re-opened the row (`numbered` true) and a share
    ///   classified it later in the same substep, so the scan reads it
    ///   `Classified`: the cancel stays silent and the share's `classified` says
    ///   the rename — one line, not the same sentence twice;
    /// * the row is gone again by the scan (a stale mesh copy the viewer's own
    ///   pass expired): nothing at all, where the unguarded arm said
    ///   `identified: X0, unknown`.
    #[test]
    fn m2_a_renamed_cancel_is_said_once_and_only_for_a_mesh_insert_told_apart() {
        use crate::arena::state::Designator;
        const B: ObjId = ObjId(9);
        let mut arena = m2_pair(60_000);
        arena.step(0, &[], &[]);
        arena.step(1_000, &[], &[]);
        let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
        assert!(arena.contacts.resolve(A, contact).is_some(), "A holds B");
        let was = Designator::Provisional(
            arena
                .designators
                .provisional(A, B)
                .unwrap_or_else(|| panic!("the fixture needs B numbered")),
        );
        let detected = EventKind::Detected {
            viewer: A,
            contact,
            target: B,
        };
        let classified = EventKind::Classified {
            viewer: A,
            contact,
            target: B,
            team: Team::Blue,
        };
        // The identifications staged for `A` by one `detected` then one
        // `classified`, with the held loss and the row set up by `shape`.
        let mut run = |shape: &dyn Fn(&mut Arena)| -> Vec<FrameNotice> {
            shape(&mut arena);
            arena.held_losses.insert((A, contact), (5_000, was));
            let _ = arena.pending_notices.remove(&A);
            let mut in_flight = None;
            let _ = arena.stage_pilot_notice(1_100, &detected, &mut in_flight);
            if arena.contacts.resolve(A, contact).is_some() {
                let _ = arena.stage_pilot_notice(1_100, &classified, &mut in_flight);
            }
            assert!(
                !arena.held_losses.contains_key(&(A, contact)),
                "the `detected` did not cancel the held loss"
            );
            arena.pending_notices.remove(&A).unwrap_or_default()
        };
        let said = |notices: &[FrameNotice]| -> Vec<FrameNotice> {
            notices
                .iter()
                .filter(|notice| matches!(notice, FrameNotice::Contact { was: Some(_), .. }))
                .cloned()
                .collect()
        };

        // A mesh insert, told apart.
        let mesh = run(&|arena: &mut Arena| {
            if let Some(held) = arena.contacts.get_mut(A, contact) {
                held.numbered = false;
                held.class = Classification::Classified { team: Team::Blue };
            }
        });
        let mesh_said = said(&mesh);
        assert_eq!(mesh_said.len(), 1, "one identification: {mesh:?}");
        assert!(
            matches!(
                mesh_said.first(),
                Some(FrameNotice::Contact {
                    kind: NoticeKind::Classified,
                    was: Some(old),
                    ..
                }) if *old == was
            ),
            "the mesh insert said the rename from {was:?}: {mesh:?}"
        );

        // This platform's own row, classified by a share in the same substep.
        let own = run(&|arena: &mut Arena| {
            if let Some(held) = arena.contacts.get_mut(A, contact) {
                held.numbered = true;
                held.class = Classification::Classified { team: Team::Blue };
            }
        });
        assert_eq!(
            said(&own).len(),
            1,
            "the rename was said twice (or not at all): {own:?}"
        );

        // The row already gone again.
        let gone = run(&|arena: &mut Arena| {
            let _ = arena.contacts.remove(A, contact);
        });
        assert!(
            gone.is_empty(),
            "a row that is gone was announced: {gone:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut M.3 — the lazy phase-4 hash (16c `T-23`)
    // -----------------------------------------------------------------------

    /// **16b cut M.3** (16c `T-23`): a pulse with nothing queued and no reflex
    /// unit takes no hash in phase 4 — counted, because the slot reads `Outside`
    /// after any pulse whatever phase 4 did inside it: the pulse takes exactly
    /// one `state_hash` per substep (the substep's record) and phase 5's one,
    /// and an eager phase 4 would be one more. Outside a run `event_hash` is a
    /// fresh `state_hash` that stores nothing. A phase that resolves two
    /// actions takes **one** hash between them — one more than the quiet pulse,
    /// not two — and both lines carry it (the W.1 property, restated), taken at
    /// its first line.
    #[test]
    fn m3_phase_four_takes_its_hash_at_its_first_line_and_a_quiet_pulse_takes_none() {
        const B: ObjId = ObjId(9);
        let mut arena = m2_pair(600_000);
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        for wall in [1_000, 2_000, 3_000] {
            journal.absorb(&arena.step(wall, &[], &[]));
        }
        assert!(
            arena
                .units
                .iter()
                .all(|unit| unit.controller == crate::arena::state::Controller::Pilot),
            "no machine: nothing but phase 4 could open the slot"
        );
        let taken = || crate::arena::hash::STATE_HASHES_TAKEN.with(std::cell::Cell::get);
        // A quiet pulse, counted: the substeps' records and phase 5's, and none
        // for phase 4.
        let before = taken();
        let quiet = arena.step(4_000, &[], &[]);
        let quiet_taken = taken() - before;
        journal.absorb(&quiet);
        assert!(quiet.substeps > 0, "the quiet pulse ran its substeps");
        assert_eq!(
            quiet_taken,
            u64::from(quiet.substeps) + 1,
            "one hash per substep and phase 5's; phase 4 took none"
        );
        assert_eq!(arena.action_hash, ActionHash::Outside);
        let before = taken();
        assert_eq!(arena.event_hash(), state_hash(&arena));
        assert_eq!(taken() - before, 2, "outside a run, a fresh hash each time");
        assert_eq!(
            arena.action_hash,
            ActionHash::Outside,
            "a hash taken outside a run is not stored"
        );
        // Two discrete actions in one pulse: A's `STARTUP` and its `LOCK`.
        let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
        let order = |kind: IntentKind, seq: u64, arena: &Arena| {
            let channel = kind.channel_required();
            Intent {
                unit: A,
                channel,
                epoch: arena.epochs.current(A, channel),
                kind,
                seq,
            }
        };
        let intents = vec![
            order(IntentKind::Startup, 1, &arena),
            order(IntentKind::Lock { contact }, 2, &arena),
        ];
        let before = taken();
        let busy = arena.step(5_000, &[], &intents);
        let busy_taken = taken() - before;
        journal.absorb(&busy);
        assert_eq!(
            busy_taken,
            u64::from(busy.substeps) + 2,
            "the two actions shared one phase-4 hash"
        );
        assert_eq!(
            arena.action_hash,
            ActionHash::Outside,
            "closed after the phase"
        );
        let log = journal.events(&mut arena);
        let phase_four: Vec<&LedgerEvent> = log
            .iter()
            .filter(|event| matches!(event.kind.as_str(), "startup" | "locked"))
            .collect();
        assert_eq!(phase_four.len(), 2, "both actions resolved: {phase_four:?}");
        assert!(
            phase_four
                .iter()
                .all(|event| event.hash == phase_four[0].hash && event.t_ms == 5_000),
            "one phase, one moment, one hash: {phase_four:?}"
        );
    }

    /// **16b cut Y.1**: phase 2's `ordered` lines take **one** hash between them
    /// — M.3's rule (`T-23`) kept for the phase that journals now. An order to
    /// two units in one pulse takes exactly one `state_hash` more than a quiet
    /// pulse (the substeps' records and phase 5's), both lines carry it, both
    /// sit at the pulse's opening arena millisecond, and the slot is closed
    /// again after the phase.
    ///
    /// The two scouts are handed to the machine by the controller byte alone,
    /// with no reflex tick scheduled, so nothing inside the substeps can open a
    /// slot of its own and the count is phase 2's.
    #[test]
    fn y1_phase_two_takes_one_hash_for_every_ordered_line_of_a_pulse() {
        const B: ObjId = ObjId(9);
        let mut arena = m2_pair(600_000);
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        for wall in [1_000, 2_000, 3_000] {
            journal.absorb(&arena.step(wall, &[], &[]));
        }
        for unit in [A, B] {
            if let Some(state) = arena.unit_mut(unit) {
                state.controller = crate::arena::state::Controller::Reflex;
            }
        }
        let taken = || crate::arena::hash::STATE_HASHES_TAKEN.with(std::cell::Cell::get);
        let before = taken();
        let quiet = arena.step(4_000, &[], &[]);
        let quiet_taken = taken() - before;
        journal.absorb(&quiet);
        assert_eq!(
            quiet_taken,
            u64::from(quiet.substeps) + 1,
            "the baseline: one hash per substep and phase 5's"
        );
        let told = crate::deliberation::order::Order {
            squad: crate::deliberation::squad::SquadId::parse("red-1").expect("a squad id"),
            objective: crate::deliberation::order::Objective::Hold,
            target: None,
            horizon_ms: 600_000,
            constraints: crate::deliberation::order::Constraints::default(),
            ttl_ms: 600_000,
            issued_ms: 1,
            issuer: Some(A),
            spot: None,
        };
        let intents: Vec<Intent> = [A, B]
            .into_iter()
            .zip(1_u64..)
            .map(|(unit, seq)| Intent {
                unit,
                channel: Channel::Helm,
                epoch: arena.epochs.current(unit, Channel::Helm),
                kind: IntentKind::Order(Box::new(told.clone())),
                seq,
            })
            .collect();
        let before = taken();
        let busy = arena.step(5_000, &[], &intents);
        let busy_taken = taken() - before;
        journal.absorb(&busy);
        assert_eq!(
            busy_taken,
            u64::from(busy.substeps) + 2,
            "the two `ordered` lines shared phase 2's one hash"
        );
        assert_eq!(
            arena.action_hash,
            ActionHash::Outside,
            "closed after the pulse"
        );
        let log = journal.events(&mut arena);
        let ordered: Vec<&LedgerEvent> = log
            .iter()
            .filter(|event| event.kind.as_str() == "ordered")
            .collect();
        assert_eq!(ordered.len(), 2, "one line per unit: {ordered:?}");
        assert!(
            ordered
                .iter()
                .all(|event| event.hash == ordered[0].hash && event.t_ms == 4_000),
            "one phase, one moment, one hash: {ordered:?}"
        );

        // **16b cut Z.2** (16c Q44): a release takes the same slot. One pulse
        // with a release for A and a fresh order for B journals one
        // `order_released` and one `ordered` line under ONE hash — one more
        // than a quiet pulse — at the pulse's opening millisecond, and counts
        // one `orders_released`.
        let squad = told.squad;
        let intents = vec![
            Intent {
                unit: A,
                channel: Channel::Helm,
                epoch: arena.epochs.current(A, Channel::Helm),
                kind: IntentKind::ReleaseOrder {
                    squad,
                    issuer: Some(A),
                },
                seq: 3,
            },
            Intent {
                unit: B,
                channel: Channel::Helm,
                epoch: arena.epochs.current(B, Channel::Helm),
                kind: IntentKind::Order(Box::new(told.clone())),
                seq: 4,
            },
        ];
        let before = taken();
        let mixed = arena.step(6_000, &[], &intents);
        let mixed_taken = taken() - before;
        journal.absorb(&mixed);
        assert_eq!(
            mixed_taken,
            u64::from(mixed.substeps) + 2,
            "the release and the order shared phase 2's one hash"
        );
        assert_eq!(mixed.reflex.orders_released, 1, "one machine dropped");
        assert_eq!(arena.reflex.order(A), None, "the release landed");
        let log = journal.events(&mut arena);
        let phase_two: Vec<&LedgerEvent> = log
            .iter()
            .filter(|event| {
                event.t_ms == 5_000 && matches!(event.kind.as_str(), "ordered" | "order_released")
            })
            .collect();
        assert_eq!(
            phase_two
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["order_released", "ordered"],
            "in the pilot's order: {phase_two:?}"
        );
        assert!(
            phase_two
                .iter()
                .all(|event| event.hash == phase_two[0].hash),
            "one phase, one moment, one hash: {phase_two:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut U.2 — the age compared at every resolution a surface derives
    // (16c `T-30`, Q27, Q28)
    // -----------------------------------------------------------------------

    /// One contact row `age_ms` old, everything else fixed.
    fn u2_row(age_ms: i64) -> FrameContact {
        FrameContact {
            id: ContactId(0x2350),
            designator: crate::arena::state::Designator::Provisional(1),
            alias: None,
            class: Classification::Unknown,
            bearing_mdeg: 90_000,
            range_cm: 43_400,
            age_ms,
            cell: Some((12, 7)),
            condition: None,
        }
    }

    /// **16b cut U.2**: an unlocked row is the same row across a sub-second
    /// move of its age, and a different one the moment the whole second moves
    /// or the stale clause flips — at `pass_ms` 1,000 that is `1,000` against
    /// `1,001`, inside one whole second.
    #[test]
    fn u2_an_unlocked_row_moves_with_the_second_and_the_stale_clause() {
        let same = |a: i64, b: i64| u2_row(a).same_as(&u2_row(b), 1_000, false);
        assert!(same(400, 900), "inside the first second, before the clause");
        assert!(!same(900, 1_000), "the whole second moved");
        assert!(
            !same(1_000, 1_001),
            "the stale clause flips past one pass: `, plotted 1 s ago`"
        );
        assert!(same(1_001, 1_999), "one second, stale throughout");
        assert!(!same(1_999, 2_000), "and the next second");
        // A shorter pass moves the clause and nothing else.
        assert!(!u2_row(400).same_as(&u2_row(600), 500, false));
        assert!(u2_row(600).same_as(&u2_row(900), 500, false));
    }

    /// **16b cut U.2**: the locked row also moves with the lock's range step,
    /// which crosses from 10 m to 20 m at 648 ms — below any second — and the
    /// same two ages on an unlocked row are one row.
    #[test]
    fn u2_a_locked_row_moves_with_the_locks_range_step() {
        use crate::arena::perception::lock_range_step_m;
        assert_eq!(lock_range_step_m(600), 10);
        assert_eq!(lock_range_step_m(700), 20);
        assert!(
            !u2_row(600).same_as(&u2_row(700), 1_000, true),
            "a locked pilot would read RANGE at a different step"
        );
        assert!(
            u2_row(600).same_as(&u2_row(700), 1_000, false),
            "an unlocked row prints no step"
        );
        assert!(
            u2_row(700).same_as(&u2_row(900), 1_000, true),
            "20 m either side"
        );
    }

    /// **16b cut U.2**: a reading's own age follows the same rule, and a row
    /// that gained or lost its reading is a different row.
    #[test]
    fn u2_a_readings_age_is_compared_at_the_same_resolutions() {
        let read = |age_ms: i64| FrameContact {
            condition: Some(FrameCondition {
                table: 0,
                bands: crate::arena::condition::Condition::default(),
                age_ms,
            }),
            ..u2_row(200)
        };
        assert!(read(100).same_as(&read(800), 1_000, false));
        assert!(!read(800).same_as(&read(1_000), 1_000, false));
        assert!(!read(1_000).same_as(&read(1_001), 1_000, false));
        assert!(!read(100).same_as(&u2_row(200), 1_000, false));
        let (Some(young), Some(old)) = (read(100).condition, read(2_500).condition) else {
            panic!("both rows carry a reading");
        };
        assert!(!young.same_as(&old, 1_000));
        assert!(young.same_as(&young, 1_000));
    }

    /// **16b cut U.2**: `Frame::same_as` over two frames that differ only in a
    /// sub-second unlocked age is `true` — nothing a pilot prints moved — and
    /// over two that differ in a row's range is `false`; a row more or less is
    /// always a new frame.
    #[test]
    fn u2_a_frame_is_sent_when_a_printed_figure_moves_and_not_per_millisecond() {
        let mut arena = fresh();
        let out = arena.step(0, &[], &[]);
        let base = out
            .frames
            .first()
            .map(|(_, frame)| frame.clone())
            .unwrap_or_else(|| panic!("the opening frame"));
        let with = |row: FrameContact| Frame {
            contacts: vec![row],
            ..base.clone()
        };
        assert!(with(u2_row(250)).same_as(&with(u2_row(750)), 1_000));
        assert!(!with(u2_row(750)).same_as(&with(u2_row(1_000)), 1_000));
        let moved = FrameContact {
            range_cm: 44_400,
            ..u2_row(250)
        };
        assert!(!with(u2_row(250)).same_as(&with(moved), 1_000));
        assert!(!with(u2_row(250)).same_as(&base, 1_000));
        // Locked on the row: the step decides.
        let locked = |row: FrameContact| Frame {
            lock: Some(row.id),
            ..with(row)
        };
        assert!(!locked(u2_row(600)).same_as(&locked(u2_row(700)), 1_000));
        assert!(locked(u2_row(100)).same_as(&locked(u2_row(600)), 1_000));
    }

    // -----------------------------------------------------------------------
    // 16b cut I.3 — the staff floor and the staff place (16c N10)
    // -----------------------------------------------------------------------

    /// **16b cut I.3**: under a floor the task's wake is a WALL interval,
    /// `wall + max(period, floor)`, and no due event is consulted; without one
    /// the lattice rule stands.
    #[test]
    fn i3_next_due_under_a_floor_is_the_wall_plus_the_wake_period() {
        const WALL: i64 = 1_000_000;
        let mut arena = at_now_with(1_000, Some(1_003));
        assert_eq!(arena.next_due_ms(WALL), WALL + 250, "the lattice rule");
        arena.clock.set_floor(Some(5_000));
        assert_eq!(
            arena.next_due_ms(WALL),
            WALL + 5_000,
            "the due event 3 ms ahead is not consulted"
        );
        // Off the lattice, it is still a wall interval: nothing pulls it back.
        let mut late = at_now_with(1_253, Some(1_500));
        late.clock.set_floor(Some(5_000));
        assert_eq!(late.next_due_ms(WALL), WALL + 5_000);
        // `max(period, floor)`: a floor under the tempo's period is the period.
        arena.clock.set_floor(Some(1_000));
        assert_eq!(arena.next_due_ms(WALL), WALL + 5_000, "Quiet is 5,000");
        arena.clock.pin_tempo_for_tests(Tempo::Engaged);
        assert_eq!(arena.next_due_ms(WALL), WALL + 1_000, "Engaged under 1,000");
        arena.clock.set_floor(None);
        assert_eq!(
            arena.next_due_ms(WALL),
            WALL + 250,
            "lifted: the lattice again"
        );
    }

    /// **16b cut I.3, the quantisation**: a floored pulse from an off-lattice
    /// `now` ends on the lattice.
    #[test]
    fn i3_a_floored_pulse_from_off_the_lattice_ends_on_it() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        // Unfloored, a late wake ends off the lattice, as it always has.
        arena.step(253, &[], &[]);
        assert_eq!(arena.now_ms, 253);
        arena.clock.set_floor(Some(5_000));
        let out = arena.step(5_253, &[], &[]);
        assert_eq!(
            arena.now_ms, 5_250,
            "the overshoot past the lattice is discarded"
        );
        assert_eq!(arena.now_ms % arena.substep_len_ms(), 0);
        assert!(!out.lagged);
    }

    /// **16b cut I.3**: an on-lattice pulse whose wall interval is the floor
    /// plus a seven-millisecond overshoot moves exactly `min(cap, floor)` —
    /// 2,000 at `Engaged`, 5,000 at `Quiet` — and is not lag.
    #[test]
    fn i3_an_on_time_floored_wake_moves_the_clamp_or_the_floor_and_no_more() {
        for (tempo, moves) in [(Tempo::Engaged, 2_000_i64), (Tempo::Quiet, 5_000)] {
            let mut arena = arena();
            arena.clock.pin_tempo_for_tests(tempo);
            arena.clock.set_floor(Some(5_000));
            arena.step(0, &[], &[]);
            let out = arena.step(5_007, &[], &[]);
            assert_eq!(arena.now_ms, moves, "{tempo:?}");
            assert!(!out.lagged, "{tempo:?}: a slowed match is not lagging");
            let out = arena.step(10_014, &[], &[]);
            assert_eq!(arena.now_ms, 2 * moves, "{tempo:?}");
            assert!(!out.lagged);
        }
    }

    /// **16b cut I.3**: a wake inside a substep is a pulse of no time — no
    /// substep, no arena millisecond — and the pulse after a lift is still
    /// quantised, the one after that not.
    #[test]
    fn i3_a_wake_inside_a_substep_is_a_zero_dt_pulse_and_a_lift_is_quantised_once() {
        let mut arena = arena();
        arena.clock.set_floor(Some(1_000));
        arena.step(0, &[], &[]);
        let out = arena.step(137, &[], &[]);
        assert_eq!((arena.now_ms, out.substeps), (0, 0));
        let out = arena.step(1_140, &[], &[]);
        assert_eq!(arena.now_ms, 1_000, "1,003 of wall is 1,000 of arena");
        assert_eq!(out.substeps, 4);
        arena.clock.set_floor(None);
        arena.step(2_151, &[], &[]);
        assert_eq!(
            arena.now_ms, 2_000,
            "the interval after a lift began under the floor"
        );
        arena.step(3_154, &[], &[]);
        assert_eq!(arena.now_ms, 3_003, "and the one after it is not quantised");
    }

    /// Every `tempo_floor` line in `log`, as `(t_ms, period_ms, hash)`.
    fn i3_floor_lines(log: &[LedgerEvent]) -> Vec<(i64, Option<u32>, u64)> {
        log.iter()
            .filter_map(|event| match event.kind {
                EventKind::TempoFloor { period_ms } => Some((event.t_ms, period_ms, event.hash)),
                _ => None,
            })
            .collect()
    }

    /// **16b cut I.3**: the floor is journalled once per change, at the pulse's
    /// end, under phase 5's one hash — the `tempo` line of the same pulse
    /// carries the same — and a repeat, or a pulse with no change, journals
    /// nothing.
    #[test]
    fn i3_the_tempo_floor_line_is_one_a_change_under_the_phase_five_hash() {
        let mut arena = arena();
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        let fire = Intent {
            unit: A,
            channel: Channel::Fire,
            epoch: 1,
            kind: IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
            },
            seq: 2,
        };
        arena.clock.set_floor(Some(5_000));
        journal.absorb(&arena.step(5_000, &[], &[fire]));
        journal.absorb(&arena.step(10_000, &[], &[]));
        arena.clock.set_floor(Some(5_000));
        journal.absorb(&arena.step(15_000, &[], &[]));
        arena.clock.set_floor(None);
        journal.absorb(&arena.step(20_000, &[], &[]));
        // The fire intent put the arena in `Alert` at 5,000, whose clamp is
        // 4,000: every later 5,000 ms wake moved 4,000 of arena time, so the
        // lift's pulse ended at 17,000.
        assert_eq!(arena.now_ms, 17_000);
        let log = journal.events(&mut arena);
        let floors = i3_floor_lines(&log);
        assert_eq!(
            floors
                .iter()
                .map(|(t_ms, period, _)| (*t_ms, *period))
                .collect::<Vec<_>>(),
            vec![(5_000, Some(5_000)), (17_000, None)],
            "one line a change, the repeat none: {log:?}"
        );
        assert!(
            !log.iter().any(|event| event.kind.as_str() == "lagged"),
            "a slowed match is not lagging"
        );
        let tempo_hash = log
            .iter()
            .find(|event| matches!(event.kind, EventKind::Tempo { .. }))
            .map(|event| (event.t_ms, event.hash));
        assert_eq!(
            tempo_hash,
            floors.first().map(|(t_ms, _, hash)| (*t_ms, *hash)),
            "the tempo line and the floor line of one pulse share phase 5's hash"
        );
    }

    /// **16b cut I.3**: never on an ended match — neither on a stop, nor on a
    /// pulse whose substep ended the match.
    #[test]
    fn i3_no_tempo_floor_line_is_journalled_on_an_ended_match() {
        let mut stopped = arena();
        let mut journal = Log::default();
        journal.absorb(&stopped.step(0, &[], &[]));
        stopped.stop(StopReason::Abort);
        stopped.clock.set_floor(Some(5_000));
        journal.absorb(&stopped.step(5_000, &[], &[]));
        let log = journal.events(&mut stopped);
        assert!(i3_floor_lines(&log).is_empty(), "{log:?}");

        let mut timed = Arena::new(
            crate::arena::test_support::custody(
                vec![tank(A, Team::Red, pose(3_500, 3_500))],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig {
                max_duration_ms: 3_000,
                ..MatchConfig::default()
            },
            &crate::config::SensorsConfig::default(),
        )
        .expect("a three-second match");
        let mut journal = Log::default();
        journal.absorb(&timed.step(0, &[], &[]));
        journal.absorb(&timed.step(2_000, &[], &[]));
        timed.clock.set_floor(Some(5_000));
        let out = timed.step(7_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(out.ended.map(|end| end.t_ms), Some(3_000));
        let log = journal.events(&mut timed);
        assert!(i3_floor_lines(&log).is_empty(), "{log:?}");
    }

    // -- the place ------------------------------------------------------------

    const I3_B: ObjId = ObjId(9);

    /// Two tanks at epoch 1 — `A` at cell (3, 3), `B` at (5, 5) — over a window
    /// whose cell (6, 2) stands 120 cm high, (1, 6) is impassable and (7, 0) is a
    /// passable cell walled in on both of its window sides.
    fn i3_pair() -> Arena {
        use crate::arena::state::{ArenaTerrain, TerrainCell};
        let mut arena = arena_at_epoch(
            vec![
                tank(A, Team::Red, pose(3_500, 3_500)),
                tank(I3_B, Team::Blue, pose(5_500, 5_500)),
            ],
            1,
        );
        let ground = crate::arena::test_support::ground();
        let wall = TerrainCell {
            move_cost: 0,
            ..ground
        };
        let mut cells = vec![ground; 64];
        let mut paint = |x: usize, y: usize, cell: TerrainCell| {
            if let Some(slot) = cells.get_mut(y * 8 + x) {
                *slot = cell;
            }
        };
        paint(
            6,
            2,
            TerrainCell {
                elev_cm: 120,
                ..ground
            },
        );
        paint(1, 6, wall);
        paint(6, 0, wall);
        paint(7, 1, wall);
        arena.definition.terrain = ArenaTerrain::new(8, 8, 1_000, cells).expect("the window");
        arena
    }

    /// A staff place of `unit` on `(x, y)` at epoch 1, as `@arena/place` stamps
    /// it.
    fn i3_place(unit: ObjId, x: u16, y: u16, seq: u64) -> Intent {
        order(unit, IntentKind::Place { x, y }, seq)
    }

    /// Every `placed` and `place_refused` line in `log`.
    fn i3_place_lines(log: &[LedgerEvent]) -> Vec<LedgerEvent> {
        log.iter()
            .filter(|event| matches!(event.kind.as_str(), "placed" | "place_refused"))
            .cloned()
            .collect()
    }

    /// **16b cut I.3**: an accepted place puts the pose on the cell's centre at
    /// its `elev_cm`, heading kept; zeroes the speed, the desired speed and the
    /// carries; clears the hold, the traffic hold, `blocked` and `grade_pct`;
    /// moves the index; leaves another platform's contact on the unit as it was;
    /// and journals `placed` under phase 4's one hash, which the same pulse's
    /// `startup` line shares. Nothing is staged.
    #[test]
    fn i3_a_place_moves_the_unit_and_journals_placed_under_the_phase_four_hash() {
        let mut arena = i3_pair();
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        // B holds A as a classified contact.
        let contact = crate::arena::rng::draw_contact_id(arena.seed, I3_B, A);
        arena.contacts.insert(
            I3_B,
            contact,
            crate::arena::state::ShadowContact::sighted(
                A,
                Classification::Classified { team: Team::Red },
                0,
                crate::arena::state::SeenPose::default(),
            ),
        );
        let before = arena.contacts.get(I3_B, contact).cloned();
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 45_000;
            unit.speed_cms = 300;
            unit.desired_speed_cms = 500;
            unit.carry.east = 7;
            unit.carry.turn = 3;
            unit.held = Some(crate::arena::motion::Held {
                x: 3,
                y: 4,
                cause: crate::arena::motion::BlockedCause::Occupied,
            });
            unit.occupied_hold = Some(crate::arena::state::OccupiedHold {
                cell: (3, 4),
                since_ms: 0,
            });
            unit.blocked = true;
            unit.grade_pct = 12;
        }
        let startup = order(I3_B, IntentKind::Startup, 2);
        let out = arena.step(0, &[], &[i3_place(A, 6, 2, 1), startup]);
        journal.absorb(&out);
        let unit = arena.unit(A).expect("A");
        assert_eq!(
            (unit.pose.x_cm, unit.pose.y_cm, unit.pose.z_cm),
            (6_500, 2_500, 120)
        );
        assert_eq!(unit.pose.heading_mdeg, 45_000, "the heading is kept");
        assert_eq!((unit.speed_cms, unit.desired_speed_cms), (0, 0));
        assert_eq!(unit.carry, crate::arena::state::MotionCarry::default());
        assert!(unit.held.is_none() && unit.occupied_hold.is_none() && !unit.blocked);
        assert_eq!(unit.grade_pct, 0);
        assert_eq!(arena.index.occupant(6, 2), Some(A));
        assert_eq!(arena.index.occupant(3, 3), None);
        assert_eq!(arena.index.cell_of_unit(A), Some((6, 2)));
        assert_eq!(
            arena.contacts.get(I3_B, contact).cloned(),
            before,
            "the other platform's contact is untouched: same id, same class, same plot"
        );
        assert!(told(&out).is_empty(), "nothing is said to the pilot");
        let log = journal.events(&mut arena);
        let placed = i3_place_lines(&log);
        assert_eq!(
            placed.iter().map(|event| &event.kind).collect::<Vec<_>>(),
            vec![&EventKind::Placed {
                unit: A,
                x: 6,
                y: 2,
                from_x: 3,
                from_y: 3,
            }]
        );
        let startup_hash = log
            .iter()
            .find(|event| matches!(event.kind, EventKind::Startup { unit } if unit == I3_B))
            .map(|event| event.hash);
        assert_eq!(
            placed.first().map(|event| event.hash),
            startup_hash,
            "one phase-4 hash"
        );
    }

    /// **16b cut I.3**: each refusal journals `place_refused` with its word and
    /// moves nothing — off the window, impassable, no way out, occupied — and
    /// none is said to anybody.
    #[test]
    fn i3_a_refused_place_journals_its_word_and_moves_nothing() {
        for (x, y, word) in [
            (8_u16, 0_u16, "impassable"),
            (1, 6, "impassable"),
            (7, 0, "impassable"),
            (5, 5, "occupied"),
        ] {
            let mut arena = i3_pair();
            let mut journal = Log::default();
            journal.absorb(&arena.step(0, &[], &[]));
            let out = arena.step(0, &[], &[i3_place(A, x, y, 1)]);
            journal.absorb(&out);
            assert!(told(&out).is_empty(), "({x}, {y}) was said");
            let unit = arena.unit(A).expect("A");
            assert_eq!((unit.pose.x_cm, unit.pose.y_cm), (3_500, 3_500));
            assert_eq!(arena.index.occupant(3, 3), Some(A));
            let log = journal.events(&mut arena);
            assert_eq!(
                i3_place_lines(&log)
                    .into_iter()
                    .map(|event| event.kind)
                    .collect::<Vec<_>>(),
                vec![EventKind::PlaceRefused {
                    unit: A,
                    x,
                    y,
                    reason: SmolStr::new_static(word),
                }],
                "({x}, {y})"
            );
        }
    }

    /// **16b cut I.3**: a round in flight refuses a place at EITHER end of it —
    /// the target, and the shooter, whose pose the impact traces its line from.
    #[test]
    fn i3_a_round_in_flight_refuses_a_place_of_its_target_and_its_shooter() {
        let mut arena = m2_pair(60_000);
        let (_, seq, _) = m2_fire(&mut arena);
        assert!(arena.shot(seq).is_some(), "the round is in flight");
        for unit in [ObjId(9), A] {
            let before = arena
                .unit(unit)
                .map(|state| (state.pose.x_cm, state.pose.y_cm));
            assert_eq!(
                arena.resolve_place(unit, 2, 6, arena.now_ms),
                Err(SmolStr::new_static("in_flight")),
                "#{}",
                unit.0
            );
            assert_eq!(
                arena
                    .unit(unit)
                    .map(|state| (state.pose.x_cm, state.pose.y_cm)),
                before
            );
        }
        let mut journal = Log::default();
        let log = journal.events(&mut arena);
        assert_eq!(
            i3_place_lines(&log)
                .iter()
                .map(|event| event.kind.clone())
                .collect::<Vec<_>>(),
            [ObjId(9), A]
                .into_iter()
                .map(|unit| EventKind::PlaceRefused {
                    unit,
                    x: 2,
                    y: 6,
                    reason: SmolStr::new_static("in_flight"),
                })
                .collect::<Vec<_>>()
        );
    }

    /// **16b cut I.3**: the three gates a place skips — a wreck is placed, a
    /// linkdead unit is placed, and a place spends none of the seat's eight a
    /// second (a unit at its bound is still placed, and its count is untouched).
    #[test]
    fn i3_a_place_skips_the_wreck_the_linkdead_and_the_rate_gates() {
        type Setup = fn(&mut crate::arena::state::UnitState);
        let max = crate::config::ArenaConfig::default()
            .intents
            .max_pending_per_unit;
        let cases: [(&str, Setup); 3] = [
            ("wreck", |unit| unit.limits.wreck = true),
            ("linkdead", |unit| unit.linkdead = true),
            ("rate", |unit| {
                unit.orders_window_s = 0;
                unit.orders_in_window = crate::config::ArenaConfig::default()
                    .intents
                    .max_pending_per_unit;
            }),
        ];
        for (case, setup) in cases {
            let mut arena = i3_pair();
            arena.step(0, &[], &[]);
            if let Some(unit) = arena.unit_mut(A) {
                setup(unit);
            }
            let out = arena.step(0, &[], &[i3_place(A, 2, 6, 1)]);
            assert!(told(&out).is_empty(), "{case}");
            let unit = arena.unit(A).expect("A");
            assert_eq!(
                (unit.pose.x_cm, unit.pose.y_cm),
                (2_500, 6_500),
                "{case}: the unit was not placed"
            );
            if case == "rate" {
                assert_eq!(
                    unit.orders_in_window, max,
                    "a place spends none of the pilot's orders"
                );
            }
        }
        // And a pilot's order after a place in the same second is counted from
        // nought, as if the place had never been.
        let mut arena = i3_pair();
        arena.step(0, &[], &[]);
        arena.step(0, &[], &[i3_place(A, 2, 6, 1)]);
        assert_eq!(arena.unit(A).map(|unit| unit.orders_in_window), Some(0));
    }

    /// **16b cut I.3**: a place refused at phase 2 — a stale epoch here — still
    /// journals `place_refused` with the phase's word under phase 2's hash, and
    /// stages nothing; nor does a phase-4 refusal.
    #[test]
    fn i3_a_stale_place_journals_epoch_and_no_place_is_ever_said() {
        let mut arena = i3_pair();
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        let stale = Intent {
            epoch: 0,
            ..i3_place(A, 2, 6, 1)
        };
        let wrong_channel = Intent {
            channel: Channel::Fire,
            ..i3_place(A, 2, 6, 2)
        };
        let out = arena.step(0, &[], &[stale, wrong_channel, i3_place(A, 5, 5, 3)]);
        journal.absorb(&out);
        assert!(
            refusals(&out).is_empty(),
            "a place is never a Refused notice"
        );
        assert!(told(&out).is_empty());
        let log = journal.events(&mut arena);
        let lines = i3_place_lines(&log);
        assert_eq!(
            lines
                .iter()
                .map(|event| match &event.kind {
                    EventKind::PlaceRefused { reason, .. } => reason.to_string(),
                    other => other.as_str().to_owned(),
                })
                .collect::<Vec<_>>(),
            vec!["epoch", "channel", "occupied"]
        );
        // The two phase-2 lines share phase 2's hash; the phase-4 line has its own.
        let hashes: Vec<u64> = lines.iter().map(|event| event.hash).collect();
        assert_eq!(hashes.first(), hashes.get(1));
        assert_eq!(arena.unit(A).map(|unit| unit.pose.x_cm), Some(3_500));
    }

    /// **16b cut I.3**: a place that reaches the pulse a `Stop` ends the match in
    /// — one the task buffered before the `Stop` arrived — never meets phase 2
    /// or phase 4, and still leaves its line: `place_refused` with `stopped`,
    /// just before the stop's own `aborted` line, at the same millisecond and
    /// under the same hash. The unit is not moved, and nothing is said.
    #[test]
    fn i3_a_place_in_the_stopping_pulse_is_refused_stopped_before_the_stop_line() {
        let mut arena = i3_pair();
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        journal.absorb(&arena.step(500, &[], &[]));
        arena.stop(StopReason::Abort);
        let out = arena.step(
            700,
            &[],
            &[
                order(A, IntentKind::Startup, 1),
                i3_place(A, 2, 6, 2),
                i3_place(I3_B, 6, 6, 3),
            ],
        );
        journal.absorb(&out);
        assert_eq!(out.ended.map(|end| end.reason), Some(EndReason::Abort));
        assert!(refusals(&out).is_empty());
        assert!(told(&out).is_empty());
        let log = journal.events(&mut arena);
        let tail: Vec<(String, i64, u64)> = log
            .iter()
            .skip(log.len().saturating_sub(3))
            .map(|event| {
                let word = match &event.kind {
                    EventKind::PlaceRefused { unit, reason, .. } => format!("{unit}:{reason}"),
                    other => other.as_str().to_owned(),
                };
                (word, event.t_ms, event.hash)
            })
            .collect();
        let at = arena.now_ms;
        let hash = tail.last().map_or(0, |line| line.2);
        assert_eq!(
            tail,
            vec![
                (format!("{A}:stopped"), at, hash),
                (format!("{I3_B}:stopped"), at, hash),
                ("aborted".to_owned(), at, hash),
            ],
            "the two places, then the stop, one moment and one hash"
        );
        assert_eq!(i3_place_lines(&log).len(), 2, "and nothing else of a place");
        assert_eq!(arena.unit(A).map(|unit| unit.pose.x_cm), Some(3_500));
        assert_eq!(arena.unit(I3_B).map(|unit| unit.pose.x_cm), Some(5_500));
    }

    // -----------------------------------------------------------------------
    // 16b cut O.1 — the Hold clock, the fresh read and the three tie rules
    // -----------------------------------------------------------------------

    /// The blue holder the O.1 tie tests stand on the spot: `m2_pair`'s scout at
    /// (4, 0), and (with `glass`) the one-section target a single round of the
    /// cannon destroys, facing west down the line of fire — the M.2 kill test's.
    fn o1_holder(glass: bool) -> crate::arena::state::CustodyUnit {
        use crate::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
        use crate::arena::test_support::scout;
        let mut blue = scout(ObjId(9), Team::Blue, pose(4_500, 500));
        if glass {
            blue.pose.heading_mdeg = 270_000;
            blue.profile = UnitProfile {
                bands: vec![BandProfile {
                    band: SmolStr::new("front"),
                    width_cm: 350,
                    height_cm: 250,
                    sections: vec![(SmolStr::new("hull"), 100)],
                }],
                sections: vec![SectionProfile {
                    id: SmolStr::new("hull"),
                    facets: vec![SmolStr::new("front")],
                    consequences: vec![Consequence::DestroyUnit],
                    struck_through: None,
                }],
            };
        }
        blue
    }

    /// **A Hold field**: `m2_pair`'s red scout `A` at (0, 0), `blue` on the key
    /// spot at (4, 0) — one cell round it, `hold_ms` long — and a second blue
    /// tank far off it at (7, 7), so a wrecked holder does not eliminate blue.
    fn o1_hold_field(
        hold_ms: u32,
        blue: crate::arena::state::CustodyUnit,
        max_duration_ms: u32,
    ) -> Arena {
        use crate::arena::state::{ArenaMode, CustodyOut, HoldRule, KeySpot, MatchMode};
        use crate::arena::test_support::{custody, definition, scout};
        let mut ground = definition();
        ground.key_spots = vec![KeySpot {
            name: SmolStr::new("The Line"),
            x: 4,
            y: 0,
        }];
        ground.modes = vec![ArenaMode::Hold(HoldRule {
            spot: 0,
            radius_cells: 1,
            hold_ms,
        })];
        Arena::new(
            CustodyOut {
                definition: ground,
                mode: MatchMode::Hold,
                ..custody(
                    vec![
                        scout(A, Team::Red, pose(500, 500)),
                        blue,
                        tank(ObjId(20), Team::Blue, pose(7_500, 7_500)),
                    ],
                    Vec::new(),
                )
            },
            &ArenaConfig::default(),
            &MatchConfig {
                max_duration_ms,
                ..MatchConfig::default()
            },
            &crate::config::SensorsConfig::default(),
        )
        .expect("the hold field")
    }

    /// The `HoldSettles` events on the heap, as `(t_ms, team, since_ms)`.
    fn o1_holds(arena: &Arena) -> Vec<(i64, Team, i64)> {
        arena
            .due
            .iter()
            .filter_map(|event| match event.kind {
                DueKind::HoldSettles { team, since_ms } => Some((event.t_ms, team, since_ms)),
                _ => None,
            })
            .collect()
    }

    /// **16b cut O.1**: the clock opens at the first substep boundary the sole
    /// occupant is seen at, and the win is settled at `since + hold_ms` —
    /// its own millisecond, inside a substep — with no line after it, and
    /// `match_ended { hold }` the last line. A skirmish of the same field
    /// schedules no hold at all. (Nothing else is due in that substep here; the
    /// horizon's shrink over a round due after the win is
    /// `o1_a_round_due_after_the_holds_millisecond_in_its_substep_is_never_applied`.)
    #[test]
    fn o1_a_hold_is_won_at_its_own_millisecond_and_nothing_after_it_lands() {
        // Opened at 250 (the first boundary); due at 250 + 10,100 = 10,350,
        // inside the substep 10,250..10,500.
        let mut arena = o1_hold_field(10_100, o1_holder(false), 60_000);
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        journal.absorb(&arena.step(1_000, &[], &[]));
        assert_eq!(
            arena.hold.and_then(|hold| hold.held_by),
            Some((Team::Blue, 250))
        );
        assert_eq!(o1_holds(&arena), vec![(10_350, Team::Blue, 250)]);
        let mut ended = None;
        for wall in (2_000..=11_000).step_by(1_000) {
            let out = arena.step(wall, &[], &[]);
            journal.absorb(&out);
            if out.ended.is_some() {
                ended = out.ended;
                break;
            }
        }
        assert_eq!(
            ended,
            Some(MatchEnd {
                reason: EndReason::Hold { winner: Team::Blue },
                t_ms: 10_350
            })
        );
        assert_eq!(arena.now_ms, 10_500, "the substep the win fell in");
        let log = journal.events(&mut arena);
        assert_eq!(
            log.last().map(|event| (event.t_ms, event.kind.clone())),
            Some((
                10_350,
                EventKind::MatchEnded {
                    reason: EndReason::Hold { winner: Team::Blue }
                }
            ))
        );
        assert!(
            log.iter().all(|event| event.t_ms <= 10_350),
            "a line after the hold's millisecond was journalled: {log:?}"
        );
        assert_eq!(
            log.iter()
                .filter(|event| event.kind.as_str() == "match_ended")
                .count(),
            1
        );
        // The same field at a skirmish schedules no hold.
        let mut skirmish = m2_pair_with(60_000, o1_holder(false));
        skirmish.step(0, &[], &[]);
        skirmish.step(1_000, &[], &[]);
        assert!(skirmish.hold.is_none());
        assert!(o1_holds(&skirmish).is_empty());
    }

    /// **16b cut O.1**: a clock dropped by a contest leaves its event to fire
    /// into nothing at its own millisecond; the side alone again re-opens with a
    /// later `since` and a new event, and wins at that one.
    #[test]
    fn o1_a_contest_drops_the_clock_and_its_stale_event_fires_into_nothing() {
        let mut arena = o1_hold_field(10_000, o1_holder(false), 60_000);
        arena.step(0, &[], &[]);
        arena.step(1_000, &[], &[]);
        assert_eq!(
            arena.hold.and_then(|hold| hold.held_by),
            Some((Team::Blue, 250))
        );
        // Red drives in (placed, between two pulses): contested at 1,250.
        if let Some(red) = arena.unit_mut(A) {
            red.pose = pose(3_500, 500);
        }
        arena.step(2_000, &[], &[]);
        assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
        // Red leaves: blue alone again at 2,250.
        if let Some(red) = arena.unit_mut(A) {
            red.pose = pose(500, 500);
        }
        arena.step(3_000, &[], &[]);
        assert_eq!(
            arena.hold.and_then(|hold| hold.held_by),
            Some((Team::Blue, 2_250))
        );
        assert_eq!(
            o1_holds(&arena),
            vec![(10_250, Team::Blue, 250), (12_250, Team::Blue, 2_250)]
        );
        // The stale event pops at 10,250 and ends nothing.
        for wall in (4_000..=11_000).step_by(1_000) {
            assert_eq!(arena.step(wall, &[], &[]).ended, None, "at {wall}");
        }
        assert_eq!(o1_holds(&arena), vec![(12_250, Team::Blue, 2_250)]);
        let out = arena.step(12_000, &[], &[]);
        assert_eq!(out.ended, None);
        let out = arena.step(13_000, &[], &[]);
        assert_eq!(
            out.ended,
            Some(MatchEnd {
                reason: EndReason::Hold { winner: Team::Blue },
                t_ms: 12_250
            })
        );
    }

    /// Fire `A`'s cannon at the holder as `m2_fire` does: the journal, the
    /// shot's sequence and the millisecond its round is due.
    fn o1_fire(arena: &mut Arena) -> (Log, u64, i64) {
        m2_fire(arena)
    }

    /// **Tie rule (1)** (16b cut O.1; plan item 40): **a holder wrecked at an
    /// EARLIER millisecond of the last substep voids the win.** The round lands
    /// at `impact_at` and the hold is due one millisecond later in the same
    /// substep: the arm's fresh read finds nobody on the spot, so it awards
    /// nothing, and the substep's presence settle drops the clock. Blue still
    /// has a tank, so the match runs on.
    #[test]
    fn o1_tie_1_a_holder_wrecked_earlier_in_the_last_substep_voids_the_win() {
        let (_, _, impact_at) = o1_fire(&mut o1_hold_field(3_600_000, o1_holder(true), 60_000));
        assert!(
            impact_at > 3_011 && impact_at < 3_250,
            "the fixture needs the round and the hold inside one substep: {impact_at}"
        );
        let hold_ms = u32::try_from(impact_at + 1 - 250).unwrap_or(u32::MAX);
        let mut arena = o1_hold_field(hold_ms, o1_holder(true), 60_000);
        let (mut journal, _, landed_at) = o1_fire(&mut arena);
        assert_eq!(landed_at, impact_at, "the probe and the run agree");
        assert_eq!(o1_holds(&arena), vec![(impact_at + 1, Team::Blue, 250)]);
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        assert!(
            arena.unit(ObjId(9)).is_some_and(|unit| unit.limits.wreck),
            "the fixture needs the round to destroy the holder"
        );
        assert_eq!(out.ended, None, "a wrecked holder wins nothing");
        assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
        let log = journal.events(&mut arena);
        assert!(
            !log.iter().any(|event| event.kind.as_str() == "match_ended"),
            "{log:?}"
        );
    }

    /// **Tie rule (2)** (16b cut O.1; plan item 40): **a holder wrecked at the
    /// hold's OWN millisecond still wins.** The hold's event was scheduled at
    /// `since` — long before the shot — so it carries the lower sequence and
    /// pops first; its fresh read finds the holder alive; the horizon shrinks to
    /// that millisecond; the round due at it is still applied and journalled;
    /// and then the match ends as `hold`, the last line.
    #[test]
    fn o1_tie_2_a_holder_wrecked_at_the_holds_own_millisecond_still_wins() {
        let (_, _, impact_at) = o1_fire(&mut o1_hold_field(3_600_000, o1_holder(true), 60_000));
        let hold_ms = u32::try_from(impact_at - 250).unwrap_or(u32::MAX);
        let mut arena = o1_hold_field(hold_ms, o1_holder(true), 60_000);
        let (mut journal, seq, landed_at) = o1_fire(&mut arena);
        assert_eq!(landed_at, impact_at, "the probe and the run agree");
        let hold_seq = arena
            .due
            .iter()
            .find(|event| matches!(event.kind, DueKind::HoldSettles { .. }))
            .map(|event| (event.t_ms, event.seq));
        let impact = arena
            .due
            .iter()
            .find(|event| event.kind == DueKind::Impact { shot_seq: seq })
            .map(|event| (event.t_ms, event.seq));
        assert_eq!(hold_seq.map(|(t_ms, _)| t_ms), Some(impact_at));
        assert!(
            hold_seq.map(|(_, seq)| seq) < impact.map(|(_, seq)| seq),
            "the hold pops first: {hold_seq:?} {impact:?}"
        );
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(
            out.ended,
            Some(MatchEnd {
                reason: EndReason::Hold { winner: Team::Blue },
                t_ms: impact_at
            })
        );
        assert!(
            arena.unit(ObjId(9)).is_some_and(|unit| unit.limits.wreck),
            "the round on the hold's millisecond was applied"
        );
        let log = journal.events(&mut arena);
        let hit = log
            .iter()
            .position(|event| event.kind.as_str() == "hit" && event.t_ms == impact_at);
        let end = log
            .iter()
            .position(|event| event.kind.as_str() == "match_ended");
        assert!(
            hit.is_some() && hit < end,
            "the hit is journalled before the end: {log:?}"
        );
        assert_eq!(
            log.last().map(|event| event.kind.clone()),
            Some(EventKind::MatchEnded {
                reason: EndReason::Hold { winner: Team::Blue }
            })
        );
    }

    /// **The horizon** (16b cut O.1; plan items 1, 15; cut W.2's rule): a win
    /// settled at its own millisecond shrinks the loop's horizon to it, exactly
    /// as the deadline's arm does. The round is due ONE millisecond after the
    /// hold, inside the same substep: it is never applied — the holder stands
    /// whole, the `Impact` is left on the heap — nothing is journalled after the
    /// hold's millisecond, and `match_ended { hold }` is the last line. Without
    /// the shrink that round would wreck the holder of a match already won.
    #[test]
    fn o1_a_round_due_after_the_holds_millisecond_in_its_substep_is_never_applied() {
        let (_, _, impact_at) = o1_fire(&mut o1_hold_field(3_600_000, o1_holder(true), 60_000));
        assert!(
            impact_at > 3_011 && impact_at < 3_250,
            "the fixture needs the hold and the round inside one substep: {impact_at}"
        );
        let won_at = impact_at - 1;
        let hold_ms = u32::try_from(won_at - 250).unwrap_or(u32::MAX);
        let mut arena = o1_hold_field(hold_ms, o1_holder(true), 60_000);
        let (mut journal, seq, landed_at) = o1_fire(&mut arena);
        assert_eq!(landed_at, impact_at, "the probe and the run agree");
        assert_eq!(o1_holds(&arena), vec![(won_at, Team::Blue, 250)]);
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(
            out.ended,
            Some(MatchEnd {
                reason: EndReason::Hold { winner: Team::Blue },
                t_ms: won_at
            })
        );
        assert!(
            arena.unit(ObjId(9)).is_some_and(|unit| !unit.limits.wreck),
            "the round due after the win was applied"
        );
        assert!(
            arena
                .due
                .iter()
                .any(|event| event.kind == DueKind::Impact { shot_seq: seq }
                    && event.t_ms == impact_at),
            "the round is left on the heap, never popped"
        );
        let log = journal.events(&mut arena);
        assert!(
            log.iter().all(|event| event.t_ms <= won_at),
            "a line after the hold's millisecond was journalled: {log:?}"
        );
        assert_eq!(
            log.last().map(|event| (event.t_ms, event.kind.clone())),
            Some((
                won_at,
                EventKind::MatchEnded {
                    reason: EndReason::Hold { winner: Team::Blue }
                }
            ))
        );
    }

    /// **The fresh read, the motion half** (16b cut O.1; plan item 15): **an
    /// enemy that drives into the square during the last substep gives no
    /// win.** The boundary before the pop read blue alone, so `held_by` still
    /// names the clock when its event pops; only the fresh read at the pop —
    /// over the poses this substep's motion committed — sees red inside, and it
    /// awards nothing; the substep's presence settle then drops the clock. The
    /// control, the same field with red kept out, wins at that millisecond.
    #[test]
    fn o1_an_enemy_driving_into_the_square_in_the_last_substep_voids_the_win() {
        // Opened at 250, due at 10,250: the last substep is 10,000..10,250.
        let run_to_the_last_boundary = |arena: &mut Arena| {
            for wall in (0..=10_000).step_by(1_000) {
                assert_eq!(arena.step(wall, &[], &[]).ended, None, "at {wall}");
            }
            assert_eq!(arena.now_ms, 10_000);
            assert_eq!(
                arena.hold.and_then(|hold| hold.held_by),
                Some((Team::Blue, 250))
            );
            assert_eq!(o1_holds(arena), vec![(10_250, Team::Blue, 250)]);
        };
        // The control: nobody comes, and blue wins at its own millisecond.
        let mut control = o1_hold_field(10_000, o1_holder(false), 60_000);
        run_to_the_last_boundary(&mut control);
        assert_eq!(
            control.step(11_000, &[], &[]).ended,
            Some(MatchEnd {
                reason: EndReason::Hold { winner: Team::Blue },
                t_ms: 10_250
            })
        );
        // Red, one cell west of the square at the last boundary, driving east
        // at 5 m/s: 125 cm in the substep puts it over the square's edge.
        let mut arena = o1_hold_field(10_000, o1_holder(false), 60_000);
        run_to_the_last_boundary(&mut arena);
        if let Some(red) = arena.unit_mut(A) {
            red.pose = pose(2_950, 500);
            red.pose.heading_mdeg = 90_000;
            red.power = Power::Online;
            red.speed_cms = 500;
            red.desired_speed_cms = 500;
        }
        let cell = |arena: &Arena| {
            arena
                .unit(A)
                .and_then(|red| arena.definition.terrain.cell_of(&red.pose))
        };
        assert_eq!(cell(&arena), Some((2, 0)), "outside the square at 10,000");
        let mut journal = Log::default();
        let out = arena.step(10_250, &[], &[]);
        journal.absorb(&out);
        assert_eq!(arena.now_ms, 10_250, "the one substep the clock ran out in");
        assert!(
            cell(&arena).is_some_and(|at| arena.hold.is_some_and(|hold| hold.square_contains(at))),
            "the fixture needs red inside the square by the pop: {:?}",
            cell(&arena)
        );
        assert_eq!(out.ended, None, "a contested square wins nothing");
        assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
        assert!(o1_holds(&arena).is_empty(), "the popped event was the last");
        let log = journal.events(&mut arena);
        assert!(
            !log.iter().any(|event| event.kind.as_str() == "match_ended"),
            "{log:?}"
        );
    }

    /// **Tie rule (3)** (16b cut O.1; plan item 40): **a hold at the deadline's
    /// own millisecond beats `Timeout`.** The deadline pops first (it was
    /// scheduled at check-out), the hold second; the hold is settled before the
    /// timeout arm, which then finds the match ended.
    #[test]
    fn o1_tie_3_a_hold_at_the_deadlines_own_millisecond_beats_the_timeout() {
        let mut arena = o1_hold_field(2_750, o1_holder(false), 3_000);
        let mut journal = Log::default();
        let mut ended = None;
        for wall in [0, 1_000, 2_000, 3_000] {
            let out = arena.step(wall, &[], &[]);
            journal.absorb(&out);
            ended = ended.or(out.ended);
        }
        assert_eq!(
            ended,
            Some(MatchEnd {
                reason: EndReason::Hold { winner: Team::Blue },
                t_ms: 3_000
            })
        );
        let log = journal.events(&mut arena);
        let ends: Vec<(i64, EventKind)> = m2_ends(&log);
        assert_eq!(
            ends,
            vec![(
                3_000,
                EventKind::MatchEnded {
                    reason: EndReason::Hold { winner: Team::Blue }
                }
            )],
            "one end, the hold, and no timeout beside it"
        );
    }
}
