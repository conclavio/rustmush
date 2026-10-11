//! The pilot's picture of the match (cut AJ.1): every line a seated line-mode
//! pilot is told, folded into one structure.
//!
//! [`Picture::apply`] takes one classified [`Line`] and says whether it belongs
//! in the bot's journal. **A read reply's rows apply as they come** (cut AM.1):
//! [`Picture::settle`] folds them in [`SNAPSHOT_SETTLE_MS`] after the last
//! one, as a merge, and a line of chatter does the same; **what a reply says
//! by leaving something out applies only at its end**, the first line after it
//! that is neither one of its rows nor chatter. A pause never ends a reply: on
//! a distant box one reply arrives in pieces. The game prints a reply's lines
//! back to back from one ordered outbox, so a notice read after the rows was
//! printed after them, and the reply closes before that notice is applied: a
//! reply never undoes it. Nothing here reads anything but the pilot's own
//! session text, and the numbers each rule uses come from the player help:
//!
//! * a `CONTACTS` reply is the platform's **whole** list ("`CONTACTS` is your
//!   platform's own picture and nothing else"), so a contact missing from a
//!   reply is gone once that reply has ended (and a held lock on it with it);
//!   until then its rows only update or add contacts;
//! * a row's range is rounded to 50 m and held, its bearing is whole degrees,
//!   and it is where the contact **was last seen** ("A contact is drawn where it
//!   was last seen, not where it is now, and the age is how stale that is"), so
//!   every fix carries an uncertainty and a track needs real motion to show;
//! * the lock reads finer ("a fresh one reads to 10 m"), so a bare `RANGE` on
//!   the lock read just after its row refines that row's fix;
//! * every `FIRE` refusal reads `FIRE <weapon> refused. …` whichever half
//!   answered ("Every `FIRE` refusal names the weapon, whichever half
//!   answered"), so a refusal is paired with an outstanding `Firing` ack before
//!   it is allowed to un-count a shot — and one that answers a `FIRE` the
//!   runner reported ([`Picture::sent_fire`]) is that `FIRE`'s own pre-check,
//!   never paired.
//!
//! **The fire clock** (cut AK.1, remade by cut AL.1). [`Weapon::next_ready_ms`]
//! is the one clock a gun's readiness is read from; `None` means "no evidence
//! yet: read the row". Three facts set it, all read from the bot's own
//! transcripts (AK.3's three kept runs), not from anything of the server's:
//!
//! * **(a)** the seconds in `cycling (N s)` and `Still cycling: N s to go.` are
//!   the seconds left **rounded up** — twenty cannon refusals 1.00 to 1.25 s
//!   after that gun's ack read `6 s to go` on a 6 s gun, where a truncated
//!   figure would say 5 — so `N s` means more than `N − 1` seconds and at most
//!   `N`;
//! * **(b)** a row read **inside the first second after the gun's ack**, once
//!   the shot has left, prints the whole cycle, and a row read later can print
//!   a second less: rows 500 to 1 000 ms after an ack read the cannon's `6 s`
//!   25 times of 25 and the machine gun's `1 s` 126 of 126, while rows 1 250 to
//!   1 500 ms after a cannon ack read `5 s` 30 times of 38 (and rows 250 to
//!   500 ms after a machine-gun ack still read `ready` 11 times of 49: the frame
//!   before the shot left). So the cycle ([`Weapon::cycle_ms`]) is learned only
//!   from a `cycling` row read under [`CYCLE_READ_WITHIN_MS`] after the ack.
//!   **On a wire the row is bounded by the send of that gun's `FIRE`, not by
//!   its ack** (cut AM.1; [`Weapon::fired_sent_ms`]): the bot reads its two
//!   legs only as their sum, and only the send bounds the row's age at print
//!   whatever each leg took. Above a steady round trip of about 250 ms (the
//!   `FIRE`'s and the poll's together under the half second the post-shot read
//!   leaves), the cycle stays unlearned and the gun fires from its rows, rather
//!   than being learned a second short;
//! * **(c)** the arena's own notices arrive on a **250 ms lattice** within a
//!   match, and a gun is ready one cycle after the first lattice point after
//!   its ack: every machine-gun refusal came before +150 ms of that instant and
//!   every acceptance after +50 ms, which is the help's "you and the arena are
//!   a pulse apart" (`help cockpit`, "Refusals") at the engaged pulse.
//!
//! So the ack sets the clock to the ack plus the learned cycle plus a
//! per-weapon slack ([`Weapon::slack_ms`]) that covers only the step from the
//! ack to the arena's next lattice point: never more than one engaged pulse
//! ([`FIRE_SLACK_MAX_MS`]), falling toward nothing ([`FIRE_SLACK_MIN_MS`]) by
//! [`FIRE_SLACK_DOWN_MS`] with every ack and rising by [`FIRE_SLACK_UP_MS`] on
//! each `Still cycling` the cockpit answers a `FIRE` this runner typed. A
//! `cycling (N s)` row read at `r` bounds the clock both ways, between
//! `r + (N − 1) s` and `r + N s` plus the slack (fact (a)); a `Still cycling:
//! N s to go.` read at `t` sets it to `t + (N − 1) s` plus [`FIRE_RETRY_MS`], so
//! the gun is tried again inside the second it was refused for, a lattice step
//! on (fact (c)); an accepted `ready` row sets it to the row's own time (the row
//! is the evidence); and an arena refusal of an acked shot that is not `Still
//! cycling` (the shot never left) sets it to now.
//!
//! **The lattice clock** (cut AN.3; [`Lattice`], `[arena] lattice`, on by
//! default). AL's clock cannot see where the arena's next lattice point is, so
//! its slack climbs and every shot pays a second lattice step (AL.3's runs:
//! machine-gun shots a median 1 497 to 1 499 ms apart against a line-mode floor
//! of about 1 250, and 58 to 75 of 412 to 434 `FIRE` lines refused). A `Firing
//! {w}.` ack answers its `FIRE` within a few milliseconds and **carries no
//! phase**; the arena's own notices do. So:
//!
//! * **The phase** is read from the arena's notices (fact (c): within a match
//!   every hit, miss, contact notice and `Held:` falls in one 50 ms bin on a
//!   250 ms lattice): the read times of the last [`LATTICE_SAMPLES`] `New
//!   contact`, identification, `Contact … lost.`, hit scored, miss, hit taken
//!   and `Held:` lines of a running match — never an ack, a refusal, a reply
//!   row or a lobby line — and the densest [`LATTICE_BIN_MS`] window of them
//!   on the [`LATTICE_MS`] lattice, read as its earliest sample
//!   ([`Lattice::phase`]).
//! * **The round trip** is each ack's read time less the send of the `FIRE` it
//!   answered, the median of the last [`LATTICE_RTT_SAMPLES`]
//!   ([`Lattice::rtt`]). In the read frame a lattice point is the true one plus
//!   the down leg, so sending at `L_read + cycle + guard − (up + down)` arrives
//!   at `L_true + cycle + guard`: the round trip comes off.
//! * **The clock**: an ack read at `a`, on a gun whose cycle `c` is learned,
//!   with the phase `p` and a round trip `r` at most [`LATTICE_RTT_MAX_MS`]
//!   known and the gun not benched, is ready at `next_lattice(a, p) + c +`
//!   [`LATTICE_GUARD_MS`]` − r` ([`next_lattice`]; [`Weapon::on_lattice`]);
//!   otherwise AL's clock as before.
//! * **Why the guard** (the frame of the evidence): AL's transcript reading
//!   measured each `FIRE`'s **send** time against the lattice read from the
//!   notices' **read** times, on local runs whose round trip was a few
//!   milliseconds, where send and arrival, read frame and true frame,
//!   coincide. Below +50 ms of the instant the shot is ready every send was
//!   refused, +50 to +150 was mixed, and from +150 every one was accepted. The
//!   bot takes those as arrival-frame edges, which is why the round trip comes
//!   off. **Anyone re-reading live transcripts** (a round trip of 35 to 48 ms
//!   on the dev box) **must read them net of the round trip**, or it is taken
//!   off twice. **The edge is about a lattice step past the instant** (cut
//!   AO.2): read against the phase learned here, the bot's own muster
//!   transcripts (local, a round trip of 1 to 2 ms) had the arena accept 42
//!   of 98 `FIRE`s sent 150 to 199 ms past it, 33 of 64 at 200 to 249 and 55
//!   of 61 at 250 to 299, so AN.3's guard of 175 was refused about half the
//!   time and the bench kept most guns on AL's clock. The clock now waits for
//!   that edge: with the runner's 50 ms tick, a guard of 275 lands the arrival
//!   at +275 to +325, past it. The shot is then expected one cycle and two
//!   lattice steps after the last — about 1 500 ms for the machine gun, as
//!   AL's clock reads live; no line-mode clock is predicted to beat that — and
//!   what the lattice clock can buy at that gap is fewer refused lines. Whether
//!   the knob stays on by default is left to the pooled runs.
//! * **The bench**: two `Still cycling` answers to a lattice-clocked gun's own
//!   `FIRE` inside [`LATTICE_BENCH_MS`] put that gun back on AL's clock for
//!   that long ([`LATTICE_MISSES_OFF`]; `The {w} is off the lattice clock for
//!   30 s: refused twice.`); the retry is AL's.
//! * **On a slow wire** nothing changes: above [`LATTICE_RTT_MAX_MS`] the
//!   lattice is off and AL's clock flies, and above a round trip of about
//!   250 ms no cycle is learned at all (fact (b)), so no lattice clock is ever
//!   set there; on the slow rig (a mean round trip of about 820 ms) the bot
//!   types exactly what it types with the lattice off.
//!
//! **A gun the tank has lost** (cut AL.1; the bot's own transcripts again: once
//! the turret went, every `FIRE` of its guns for seconds was answered ``That
//! unit carries no weapon called `cannon`.``). That answer strikes the gun
//! ([`Picture::struck`]: never re-added by a row this match); `The turret is
//! destroyed: its weapons are gone.` (from 17b cut M3.6 the shipped line goes
//! on `; its core is gone.`, which the reader reads past) holds every gun
//! ([`Picture::weapons_doubt`]) until a `WEAPONSTATUS` reply or `This unit carries no weapon.` says what is
//! left; and `FIRE {w} refused. Empty: …` empties that gun's rounds. What the
//! picture says of it goes to the runner as `arena` events
//! ([`Picture::take_notes`]).
//!
//! **`VIEW BRIEF`** (cut AN.2; `help cockpit`: "`VIEW BRIEF` says the same
//! window in sentences"): its header and eight bearing lines are read into
//! [`Picture::ground`] — the cells round the tank, run by run, from where it
//! stood — and nothing else; the brief's contact sentences and spot lines are
//! [`Line::Other`], and its own empty sentence `No contacts in the window.`
//! ([`Line::NoContactsInWindow`], the game's cut AP.4) is applied as the
//! `CONTACTS` empty form (the brief prints it only when the frame holds no
//! contact) but opens nothing in the lane's ledger ([`Open::brief`]). The
//! brief is open from its header or any bearing line to its end line, `End
//! of brief.` ([`Line::BriefEnd`]), which the game prints last in every
//! brief (cut AP.4): its contact sentences, spot lines, chatter and the
//! pieces of a line the terminal wrapped inside it keep it open, and the end
//! line closes it and makes its ground whole.
//! The board's `mode:` line is
//! read into [`Picture::match_mode`], and the ordinary `Power online.` that
//! starts this match into [`Picture::powered_here`].
//!
//! **The observer rule, bot side.** The picture holds only designators the
//! stream named; it never invents a contact, a side or a section, and no line but
//! a contact row or a contact notice ever adds a contact.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;

pub use super::lines::{HeldCause, MatchMode, Power, ResultWord, Scope};
use super::{
    Side,
    lines::{AskKind, ContactRow, Line, STALE_TAIL_MS, WeaponRow, is_centre_room},
};

/// **An open reply's rows are applied this long after its last row; a reply
/// is closed only by the first line after it that is not one of its rows and
/// not chatter** ([`Picture::apply`]; cut AM.1).
pub const SNAPSHOT_SETTLE_MS: u64 = 150;
/// Fixes kept per contact.
pub const MAX_FIXES: usize = 12;
/// Events kept in the ring.
pub const MAX_EVENTS: usize = 32;
/// Outstanding `Firing` acks kept per weapon.
pub const MAX_ACKS: usize = 8;
/// A `FIRE <w> refused.` pairs with an ack of `w` at most this old.
pub const ACK_PAIR_MS: u64 = 2_000;
/// A `ready` row read this soon after an ack may be the pre-shot frame.
pub const READY_AFTER_ACK_MS: u64 = 1_500;
/// A bare `RANGE` on the lock refines a row's fix read at most this long before.
pub const LOCK_FIX_MS: u64 = 400;
/// Tracks read fixes from this window only.
pub const TRACK_WINDOW_MS: u64 = 10_000;
/// A track needs fixes spanning at least this long…
pub const TRACK_SPAN_MS: u64 = 4_000;
/// …and at least this many of them…
pub const TRACK_MIN_FIXES: usize = 4;
/// …and a fitted speed of at least this, metres a second.
pub const TRACK_MIN_MPS: f64 = 3.0;
/// The widest line [`Picture::summary_lines`] writes (the terminal-width floor).
pub const SUMMARY_COLUMNS: usize = 120;
/// The engaged pulse the lane's polls are sized to ("500 ms engaged", the
/// module head of [`super`]; "you and the arena are a pulse apart", `help
/// cockpit`, "Refusals"). Not tunable.
pub const ENGAGED_PULSE_MS: u64 = 500;
/// **A `cycling` row read this soon after the gun's ack prints the whole
/// cycle** (cut AL.1; fact (b) of the module head: rows 500 to 1 000 ms after
/// an ack read the cannon's `6 s` 25 of 25 and the machine gun's `1 s` 126 of
/// 126; rows 1 250 to 1 500 ms after a cannon ack read `5 s` 30 of 38). Only
/// such a row sets [`Weapon::cycle_ms`]. Read from the transcript, not tunable.
pub const CYCLE_READ_WITHIN_MS: u64 = 1_000;
/// A weapon's slack before any ack or refusal has moved it (cut AL.1; AL.3 may
/// tune it within 0..=500 by the brief, but the lane's tests admit less: see
/// the tuning list in [`super::pilot`]). 300 since AL.3's re-measure (it was
/// 250): at 250, 20 to 28 per cent of each gun's first five `FIRE` lines in a
/// match were refused as still cycling (two runs); at 300, 14 per cent of the
/// cannon's and 18 of the machine gun's (one run).
pub const FIRE_SLACK_START_MS: u64 = 300;
/// The slack grows this much on each `Still cycling` answer to a `FIRE` this
/// runner typed (the cockpit's own pre-check: the clock was early; cut AL.1,
/// AL.3 may tune it within 100..=300 by the brief, but the lane's tests admit
/// only 100..=200: see the tuning list in [`super::pilot`])…
pub const FIRE_SLACK_UP_MS: u64 = 150;
/// …and shrinks this much on each ack (not tunable). An additive loop settles
/// where refusals are DOWN / (UP + DOWN) of the `FIRE` lines, so with DOWN at
/// most a tenth of UP it is at most one line in eleven — **while the slack the
/// gun needs stays at least UP under [`FIRE_SLACK_MAX_MS`]**: a refusal then
/// raises the slack only to the ceiling, `MAX − need` rather than UP, and the
/// share is DOWN / (MAX − need + DOWN). A rig whose shot starts up to a whole
/// engaged pulse after its `FIRE` needs nearly the ceiling at its worst phase
/// (a gun that needs 440 ms is refused one line in seven there), and the climb
/// from [`FIRE_SLACK_START_MS`] to that need costs its own refusals besides,
/// one per UP…
pub const FIRE_SLACK_DOWN_MS: u64 = 10;
/// …never below nothing: with the cycle read whole (fact (b)) the slack covers
/// only the step from the ack to the arena's next lattice point (fact (c)),
/// which a gun whose acks land on the lattice does not pay (not tunable)…
pub const FIRE_SLACK_MIN_MS: u64 = 0;
/// …and never above one engaged pulse (a ruling, not tunable).
pub const FIRE_SLACK_MAX_MS: u64 = ENGAGED_PULSE_MS;
/// **A `Still cycling: N s to go.` is tried again this long past the whole
/// seconds it says are over**: `N s` means more than `N − 1` (fact (a)), and
/// the arena's notices arrive on a 250 ms lattice (fact (c)), so the retry
/// lands one lattice step on inside the second it was refused for, not a whole
/// second later (cut AL.1; AL.3 may tune it within 150..=500).
pub const FIRE_RETRY_MS: u64 = 250;
/// **The arena's lattice** (cut AN.3; fact (c): the bot's own transcripts read
/// every notice of a match on it). Not tunable.
pub const LATTICE_MS: u64 = 250;
/// The bin a lattice point's notices fall in (the transcripts' 50 ms). Not
/// tunable.
pub const LATTICE_BIN_MS: u64 = 50;
/// The notices kept for the phase, newest last. Not tunable.
pub const LATTICE_SAMPLES: usize = 24;
/// The phase is read only from at least this many in its bin… Not tunable.
pub const LATTICE_MIN_SAMPLES: usize = 8;
/// …and at least this share of the kept notices, per mille. Not tunable.
pub const LATTICE_SHARE_PER_MILLE: usize = 750;
/// **The guard** (cut AN.3; widened in cut AO.2): a lattice-clocked gun is
/// typed this long past the instant it is ready, net of the round trip. The
/// bot's own transcripts (local, a round trip of 1 to 2 ms) read the arena's
/// edge about a lattice step past that instant: it accepted 42 of 98 `FIRE`s
/// sent 150 to 199 ms past it, 33 of 64 at 200 to 249, and 55 of 61 at 250 to
/// 299. With the runner's 50 ms tick, a guard of 275 lands the arrival at +275
/// to +325, past the edge (the module head). Tunable 150..=300.
pub const LATTICE_GUARD_MS: u64 = 275;
/// Above this round trip the lattice is off and AL's clock flies. Not
/// tunable.
pub const LATTICE_RTT_MAX_MS: u64 = 200;
/// The round trips kept, newest last. Not tunable.
pub const LATTICE_RTT_SAMPLES: usize = 8;
/// This many `Still cycling` answers to a lattice-clocked gun's own `FIRE`…
/// Not tunable.
pub const LATTICE_MISSES_OFF: usize = 2;
/// …inside this long bench the gun from the lattice clock for this long. Not
/// tunable.
pub const LATTICE_BENCH_MS: u64 = 30_000;
const _: () = assert!(LATTICE_BIN_MS < LATTICE_MS && LATTICE_GUARD_MS < 2 * LATTICE_MS);

/// **The next lattice point after `at`** (cut AN.3): the smallest `L > at`
/// with `L % LATTICE_MS == p` (`p` is taken modulo [`LATTICE_MS`]).
pub fn next_lattice(at: u64, p: u64) -> u64 {
    let p = p % LATTICE_MS;
    let base = at - at % LATTICE_MS + p;
    if base > at {
        base
    } else {
        base.saturating_add(LATTICE_MS)
    }
}

/// **The lattice, as the pilot's own session reads it** (cut AN.3; the module
/// head): whether it is used, the read times of the arena's own notices, the
/// round trips of the guns' `FIRE`s, and whether this match's first lattice
/// clock has been said. Times and figures only: it names nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Lattice {
    /// `[arena] lattice` ([`Picture::set_lattice`]); off by default.
    pub on: bool,
    /// The read times of the last [`LATTICE_SAMPLES`] notices of a running
    /// match, oldest first.
    pub samples: VecDeque<u64>,
    /// The last [`LATTICE_RTT_SAMPLES`] round trips, oldest first.
    pub rtts: VecDeque<u64>,
    /// `Firing on the arena's lattice: …` has been said this match.
    pub said: bool,
}

impl Lattice {
    /// **The phase** (cut AN.3): for each `p` in `0, 10, … 240`, the samples
    /// whose offset past `p` on the lattice is under [`LATTICE_BIN_MS`]; the
    /// window with the most (the smallest `p` on a tie), read as **its
    /// earliest sample** — a tie-broken window can start up to 45 ms before
    /// its cluster, and that would eat the guard. `None` unless that window
    /// holds at least [`LATTICE_MIN_SAMPLES`] and
    /// [`LATTICE_SHARE_PER_MILLE`] of the samples.
    pub fn phase(&self) -> Option<u64> {
        let offset = |s: u64, p: u64| (s % LATTICE_MS + LATTICE_MS - p) % LATTICE_MS;
        let mut best: Option<(u64, usize)> = None;
        for p in (0..LATTICE_MS).step_by(10) {
            let count = self
                .samples
                .iter()
                .filter(|s| offset(**s, p) < LATTICE_BIN_MS)
                .count();
            if best.is_none_or(|(_, most)| count > most) {
                best = Some((p, count));
            }
        }
        let (p, count) = best?;
        if count < LATTICE_MIN_SAMPLES
            || count.saturating_mul(1_000)
                < self.samples.len().saturating_mul(LATTICE_SHARE_PER_MILLE)
        {
            return None;
        }
        let earliest = self
            .samples
            .iter()
            .map(|s| offset(*s, p))
            .filter(|o| *o < LATTICE_BIN_MS)
            .min()?;
        Some((p + earliest) % LATTICE_MS)
    }

    /// **The round trip** (cut AN.3): with at least three, the median of the
    /// kept round trips by the index rule (`floor(0.5 × (n − 1))` of the
    /// sorted list).
    pub fn rtt(&self) -> Option<u64> {
        if self.rtts.len() < 3 {
            return None;
        }
        let mut sorted: Vec<u64> = self.rtts.iter().copied().collect();
        sorted.sort_unstable();
        sorted.get((sorted.len() - 1) / 2).copied()
    }
}
// The floor is nothing today, which the lint reads as always true; the
// assertion stays written for the bounds a tuning round moves within.
#[allow(clippy::absurd_extreme_comparisons)]
const _: () =
    assert!(FIRE_SLACK_MIN_MS <= FIRE_SLACK_START_MS && FIRE_SLACK_START_MS <= FIRE_SLACK_MAX_MS);
const _: () = assert!(FIRE_SLACK_DOWN_MS * 10 <= FIRE_SLACK_UP_MS);

/// The slack an ack leaves behind a weapon whose slack was `slack`: lower by
/// [`FIRE_SLACK_DOWN_MS`], never below [`FIRE_SLACK_MIN_MS`].
// The floor is nothing today, which the lint reads as a no-op; it stays written.
#[allow(clippy::unnecessary_min_or_max)]
fn slack_after_ack(slack: u64) -> u64 {
    slack
        .saturating_sub(FIRE_SLACK_DOWN_MS)
        .max(FIRE_SLACK_MIN_MS)
}

/// Where the pilot is in the simulator's life cycle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing read yet.
    #[default]
    Unknown,
    /// Logged in, outside the centre.
    Walking,
    /// In the lobby, a team room or a pod, holding no slot the lane knows of.
    InCentre,
    /// Holding a slot ([`Picture::claimed`]).
    Claimed { side: Side, index: u8 },
    /// The followed match is boarding.
    Boarding { match_id: u64 },
    /// In the pod, the followed match not yet running.
    Seated { match_id: u64 },
    /// The followed match is running.
    Running { match_id: u64 },
    /// The followed match is over.
    Over { match_id: u64, result: ResultWord },
}

impl Phase {
    fn words(&self) -> String {
        match self {
            Phase::Unknown => "unknown".into(),
            Phase::Walking => "walking".into(),
            Phase::InCentre => "in the centre".into(),
            Phase::Claimed { side, index } => format!("claimed {} {index}", side.as_str()),
            Phase::Boarding { match_id } => format!("boarding (match {match_id})"),
            Phase::Seated { match_id } => format!("seated (match {match_id})"),
            Phase::Running { match_id } => format!("running (match {match_id})"),
            Phase::Over { match_id, result } => {
                format!("over (match {match_id}, {})", result.as_str())
            }
        }
    }
}

/// One section of the own tank, as `STATUS` and the victim's lines print it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Section {
    pub condition: String,
    pub internals: Option<(i64, i64)>,
    /// Facet → `(armour left, armour max)`.
    pub facets: BTreeMap<String, (i64, i64)>,
}

/// **A hit the own tank took on a named facet** (cut AL.2): when it was read,
/// the plate it struck (the `through` section when the line names one, else
/// the section hit) and the facet, exactly as the victim's line printed them.
/// Nothing in it names who fired: the line says nothing of that.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FacetHit {
    pub at_ms: u64,
    pub plate: String,
    pub facet: String,
}

/// The own tank.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Own {
    pub callsign: Option<String>,
    /// The tank's name, `STATUS`'s second line (`Red Tank 1`).
    pub name: Option<String>,
    /// The bracketed chassis, when it is not the pod's own.
    pub chassis: Option<String>,
    pub power: Power,
    /// Negative astern.
    pub speed_kph: i32,
    pub heading_deg: u32,
    /// Metres, +x east and +y north (`STATUS`'s `at {x} m east, {y} m north`).
    pub pos_m: Option<(f64, f64)>,
    /// When `pos_m` was read.
    pub pos_read_ms: u64,
    pub held: Option<HeldCause>,
    pub destroyed: bool,
    pub disabled: bool,
    pub immobile: bool,
    pub sections: BTreeMap<String, Section>,
    /// The newest hit taken on a named facet (cut AL.2): the pilot turns its
    /// bow toward a side or rear hit. A new match starts without one.
    pub last_hit: Option<FacetHit>,
    pub scope: Scope,
    /// `SPEED`'s own ceilings, from its usage line.
    pub max_ahead_kph: Option<i32>,
    pub max_astern_kph: Option<i32>,
    /// `(heat, max)` on a platform that carries heat.
    pub heat: Option<(u32, u32)>,
    pub overheated: bool,
    /// The receiver is under interference.
    pub jammed: bool,
    /// The jamming pod's switch, on a tank that carries one.
    pub emcon: Option<bool>,
}

/// One fix on a contact: where its plot put it, and how far to trust that.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fix {
    /// When the plot was taken (the read less the printed age, less half a
    /// second for the rounding).
    pub at_ms: u64,
    pub pos_m: (f64, f64),
    pub sigma_m: f64,
    /// Refined by a bare `RANGE` on the lock.
    pub from_lock: bool,
}

/// A contact's fitted motion.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Track {
    /// Clockwise from north.
    pub course_deg: f64,
    pub speed_mps: f64,
}

/// One contact the platform holds, keyed by its current designator.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Contact {
    pub designator: String,
    /// The name it had before its last rename.
    pub was: Option<String>,
    /// `None` until a row or a notice resolves it.
    pub side: Option<Side>,
    pub bearing_deg: u32,
    pub rel_deg: u32,
    pub band: String,
    /// `None` for a contact known only from its `New contact` notice.
    pub range_m: Option<u32>,
    /// The plot's age when last read.
    pub age_ms: u64,
    /// When it was last read.
    pub read_ms: u64,
    pub fixes: VecDeque<Fix>,
    pub track: Option<Track>,
    /// **17b cut M1a.3**: the aspect's side word off the last row read, or
    /// `None` (a row with no tail, or a contact known only from a notice). Read
    /// by nothing yet.
    pub aspect: Option<String>,
    /// **17b cut M1a.3**: the main gun's `(weapon, percent)` off the last row
    /// read, or `None` (the dash, an unarmed hull, a row with no tail). Read by
    /// nothing yet.
    pub odds_percent: Option<(String, u32)>,
}

impl Contact {
    fn new(designator: &str, now_ms: u64) -> Contact {
        Contact {
            designator: designator.to_owned(),
            was: None,
            side: None,
            bearing_deg: 0,
            rel_deg: 0,
            band: String::new(),
            range_m: None,
            age_ms: 0,
            read_ms: now_ms,
            fixes: VecDeque::new(),
            track: None,
            aspect: None,
            odds_percent: None,
        }
    }

    fn push_fix(&mut self, fix: Fix) {
        self.fixes.push_back(fix);
        while self.fixes.len() > MAX_FIXES {
            self.fixes.pop_front();
        }
    }

    /// When the plot behind the last row was taken, by the rule every row fix
    /// uses.
    fn row_plot_ms(&self) -> u64 {
        self.read_ms.saturating_sub(self.age_ms.saturating_add(500))
    }
}

/// One weapon of the own tank.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Weapon {
    pub id: String,
    pub short: Option<String>,
    pub ready: bool,
    pub cycling_s: Option<u32>,
    pub rounds: u32,
    pub rounds_max: u32,
    pub band: Option<(u32, u32)>,
    pub percent: Option<u32>,
    /// When its row was last read; `0` for a weapon known only from an ack.
    /// A row the stale-ready guard holds (it may be the frame before the shot
    /// left) leaves it, as it leaves the rest of the row.
    pub read_ms: u64,
    /// When its chance to hit (`band`, `percent`) was last read (cut AL.1):
    /// every row reads it, a row the stale-ready guard holds too, since the
    /// chance is the target's, not the shot's. The lane's learned cadence
    /// asks it to be no older than the last lock line.
    pub chance_ms: u64,
    /// The last `Firing` ack not un-counted by a paired refusal.
    pub fired_ms: Option<u64>,
    /// **When the `FIRE` that ack answered was typed** (cut AM.1): the send
    /// time of the mark the last counted ack took ([`Picture::sent_fire`]), or
    /// `None` when the ack took no mark (a picture never told of its `FIRE`s).
    /// Set and cleared with [`Weapon::fired_ms`]; it bounds the cycle's read.
    pub fired_sent_ms: Option<u64>,
    /// **The gun's cycle** (cut AL.1): the whole seconds of a `cycling (N s)`
    /// row read under [`CYCLE_READ_WITHIN_MS`] after this gun's `FIRE` went out
    /// (cut AM.1: [`Weapon::fired_sent_ms`], or the ack when no send is known),
    /// which prints the whole cycle (fact (b) of the module head); the largest
    /// such figure when more than one is read (they agree). A row read later,
    /// a `ready` row, an ack and a refusal never set it.
    pub cycle_ms: Option<u64>,
    /// **The one readiness clock** (cut AK.1): ready at this time; `None` when
    /// nothing has set it yet, and then the row's own `ready` is read (see the
    /// module head).
    pub next_ready_ms: Option<u64>,
    /// What the clock adds to a cycle for the step from the ack to the arena's
    /// next lattice point ([`FIRE_SLACK_START_MS`] for a weapon this picture
    /// made; between [`FIRE_SLACK_MIN_MS`] and [`FIRE_SLACK_MAX_MS`]).
    pub slack_ms: u64,
    /// `Firing` acks not yet paired with a refusal, oldest first.
    pub acks: VecDeque<u64>,
    /// `Firing` acks since this weapon's last row was applied, less the ones a
    /// paired refusal un-counted: the rounds a row must be short of the last
    /// row's figure before it shows every one of them has left.
    pub shots_since_read: u32,
    /// **The last ack set the lattice clock** (cut AN.3; the module head):
    /// set or cleared by every ack.
    pub on_lattice: bool,
    /// The `Still cycling` answers to this gun's own `FIRE` while it was
    /// lattice-clocked, inside the last [`LATTICE_BENCH_MS`] (cut AN.3).
    pub lattice_misses: VecDeque<u64>,
    /// Off the lattice clock until then (cut AN.3): [`LATTICE_MISSES_OFF`]
    /// misses benched it.
    pub lattice_bench_until_ms: Option<u64>,
}

impl Weapon {
    /// Every weapon the picture holds is made here, so each starts with its
    /// slack at [`FIRE_SLACK_START_MS`] (`Default` gives 0).
    fn new(id: &str) -> Weapon {
        Weapon {
            id: id.to_owned(),
            slack_ms: FIRE_SLACK_START_MS,
            ..Weapon::default()
        }
    }

    /// Whether `name` (as a `FIRE` line, an ack or a refusal named it) is this
    /// weapon: its id or its short name.
    fn named(&self, name: &str) -> bool {
        self.id == name || self.short.as_deref() == Some(name)
    }

    /// **The rounds left as the picture counts them** (cut AL.1): the last
    /// row's figure less the acks since it, so a gun is not typed at once its
    /// magazine is spent, whenever the next `WEAPONSTATUS` comes.
    pub fn rounds_left(&self) -> u32 {
        self.rounds.saturating_sub(self.shots_since_read)
    }
}

/// A named spot of the ground, as `RANGE`, `BEARING` and `STATUS` read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Spot {
    pub name: String,
    pub range_m: Option<u32>,
    pub bearing_deg: Option<u32>,
    pub read_ms: u64,
    /// Where the spot lies, whole metres east and north (cut AJ.2): worked out
    /// only from a `STATUS` block that read the tank's own position and this
    /// spot as the nearest together, so its range and bearing were taken from
    /// the same place. A `RANGE` or `BEARING` answer changes one figure of the
    /// pair at a time and never moves it; the ground does not move either.
    pub plotted_m: Option<(i64, i64)>,
}

/// What the pilot's own shooting and the tank's own damage came to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// `Firing` acks.
    pub shots: u32,
    /// Refusals paired with an ack: shots that never left.
    pub refused_shots: u32,
    /// Refusals with no ack to pair: the cockpit's own pre-check, or any other
    /// order refused.
    pub refused_orders: u32,
    pub hits_scored: u32,
    pub misses: u32,
    pub kills: u32,
    pub hits_taken: u32,
    /// **The facet split of the hits taken** (cut AL.2), keyed `"{plate}
    /// {facet}"` (`hull left`, `right_track right`, `turret rear`): every hit
    /// line that names a facet counts once here as well as in `hits_taken`; a
    /// hit with no facet counts only there. What the facet you show cost.
    pub hits_taken_by_facet: BTreeMap<String, u32>,
    pub sections_lost: Vec<String>,
    /// **When the scope first held something this match** (cut AN.1), on the
    /// lane clock: the first `New contact` notice, identification or applied
    /// `CONTACTS` row (an unknown included) read while the match runs, of a
    /// contact not read as the own side — a row or identification naming the
    /// own side sets nothing, and a notice sets it only when its designator
    /// is not held as the own side, so a wingman's report of its teammate is
    /// never the first contact. Set once. It names nothing: the bot's own
    /// observer rule.
    pub first_contact_ms: Option<u64>,
    /// **When the first hit was taken this match** (cut AN.1), on the lane
    /// clock: the first hit line on the own tank read while the match runs.
    /// Set once.
    pub first_hit_taken_ms: Option<u64>,
    /// **When the first hit was scored this match** (cut AN.1), on the lane
    /// clock: the first `Hit on …` line read while the match runs that is not
    /// marked `(FRIENDLY)`. Set once. It names nothing.
    pub first_hit_scored_ms: Option<u64>,
}

impl Tally {
    /// The shots the report counts: acks less the refusals paired with them.
    pub fn fired(&self) -> u32 {
        self.shots.saturating_sub(self.refused_shots)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ContactNew,
    Identified,
    Unidentified,
    Lost,
    LockBroken,
    Locked,
    HitScored,
    Missed,
    Kill,
    TargetSectionLost,
    HitTaken,
    SectionLost,
    OwnDestroyed,
    OwnDisabled,
    Held,
    PowerOnline,
    Refused,
    PhaseChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Event {
    pub at_ms: u64,
    pub kind: EventKind,
    pub text: String,
}

/// What one [`Picture::apply`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Whether the line belongs in the journal (`false` for read replies, acks,
    /// `OUTSIDE_COCKPIT`, transients, and a lock line that tells nothing new).
    pub journal: bool,
    pub event: Option<Event>,
    /// **The read reply this line begins** (cut AM.1), for the lane's answer
    /// ledger: `Contacts` on a `CONTACTS` row that begins a reply (none open,
    /// or a designator the open one already holds) and on `No contacts.`;
    /// `Weapons` on a `WEAPONSTATUS` row that begins one (none open, or a gun
    /// the open one already holds) and on `This unit carries no weapon.`;
    /// `Status` on `Callsign {c}.` and on the untold head; else `None`.
    pub opened: Option<AskKind>,
}

/// **Lines that are never part of a reply and change no contact, gun or
/// lock** (cut AM.1): chatter and every other line read as [`Line::Other`] (a
/// wrapped continuation among them), a hint, and the receiver's two notices.
/// Neutral for an open `CONTACTS` or `WEAPONSTATUS` reply only: such a line
/// applies that reply's rows read so far and leaves it open, and it keeps a
/// lock line heading the rows after it. **No line is neutral for `STATUS`**:
/// its rows are indented, and a page's or a say's hanging-indent continuation
/// would read as a facet run, so a `STATUS` block survives only a pause.
pub fn neutral(line: &Line) -> bool {
    matches!(
        line,
        Line::Other(_) | Line::Hint(_) | Line::ReceiverDegraded | Line::ReceiverClear
    )
}

/// An open `CONTACTS` reply: rows with the time each was read.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ContactsReply {
    pub rows: Vec<(ContactRow, u64)>,
    pub last_ms: u64,
    /// The rows already folded into [`Picture::contacts`] (cut AM.1): a merge
    /// at each apply point; the reply's close drops what it never named.
    pub applied: usize,
}

/// An open `WEAPONSTATUS` reply.
#[derive(Clone, Debug, Default, Serialize)]
pub struct WeaponsReply {
    /// A lock line headed the first row (across a pause or chatter, cut AM.1).
    pub locked: bool,
    pub rows: Vec<(WeaponRow, u64)>,
    pub last_ms: u64,
    /// The rows already folded into [`Picture::weapons`] (cut AM.1).
    pub applied: usize,
}

/// An open `STATUS` block: its sections, staged until it closes.
#[derive(Clone, Debug, Default, Serialize)]
pub struct StatusBlock {
    pub sections: BTreeMap<String, Section>,
    pub last_ms: u64,
    /// The block's own position row, for the nearest spot's row after it.
    pub pos_m: Option<(f64, f64)>,
    /// The block's sections as they stand are merged into the tank's (cut
    /// AM.1): set by an apply, cleared by every section or facet row after it.
    pub applied: bool,
}

/// **The ground `VIEW BRIEF` described** (cut AN.2; `help cockpit`: "`VIEW
/// BRIEF` says the same window in sentences"): what the pilot's own brief said
/// of the cells round it, read off the header and the eight bearing lines.
/// It names no unit and never adds, changes or drops a contact.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Ground {
    /// When the brief's header (or, with none read just before, its first
    /// bearing line) was read.
    pub read_ms: u64,
    /// The own position then ([`Picture::own_pos_at`]): every ray is judged
    /// from it.
    pub from_m: Option<(f64, f64)>,
    /// The arena's size in metres, `(W × m, H × m)` from the header; `None`
    /// with no header read.
    pub side_m: Option<(f64, f64)>,
    /// The metres a cell, from the header; `None` with no header read.
    pub cell_m: Option<u32>,
    /// Bearing (0, 45, … 315) → `(runs, edge)`: each run `(class, reach in
    /// metres)`, and whether the ray ends at the arena's edge.
    pub rays: BTreeMap<u32, (Vec<(String, u32)>, bool)>,
    /// **Read whole**, by the brief's own structure, not a timer: set when
    /// the eighth bearing is read, or when the brief closes ([`Open::brief`]:
    /// its `End of brief.`, or a line of no brief's kind), whichever comes
    /// first. Its contact sentences, spot lines, chatter or a wrapped piece
    /// read among them do not make it whole.
    pub complete: bool,
}

/// The replies being read now.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Open {
    pub contacts: Option<ContactsReply>,
    pub weapons: Option<WeaponsReply>,
    pub status: Option<StatusBlock>,
    /// **A lock line was read and no `WEAPONSTATUS` row has taken it yet**
    /// (cut AM.1): kept across [`neutral`] lines and a pause, cleared by any
    /// other line, lapsed [`STALE_TAIL_MS`] after it was read (one output's
    /// pieces arrive at most that far apart).
    pub lock_line: bool,
    /// The section the next facet run belongs to.
    pub section: Option<String>,
    /// A `Back aboard …` block is being read: its `  Power {w}.` row counts.
    /// Set by the block's first line and cleared by the first line that is
    /// neither one of its rows (`Other`) nor that row.
    pub catchup: bool,
    /// Each `FIRE` the runner wrote ([`Picture::sent_fire`]): the weapon it
    /// named and when it went out, oldest first, while its own answer —
    /// `Firing {w}.` or the cockpit's pre-check `FIRE {w} refused. …` — has not
    /// been read. An answer takes the oldest mark **of its own weapon** (cut
    /// AK.1). A mark lapses after [`ACK_PAIR_MS`].
    pub fire_sent: VecDeque<(String, u64)>,
    /// **A `VIEW BRIEF` is being read** (cut AN.2): set by its header and by
    /// each bearing line (only `VIEW BRIEF` prints one, so a header the
    /// terminal wrapped into pieces still opens it at its first bearing),
    /// kept across its own `No contacts in the window.` and every [`neutral`]
    /// line — its contact sentences and spot lines, chatter, a receiver
    /// notice, a wrapped piece of a bearing line — and closed by its end line,
    /// `End of brief.` (the game's cut AP.4, printed last in every brief), or,
    /// should that never come, by the first line of no brief's kind (after
    /// that line is applied). The brief's `No contacts in the window.` is
    /// applied as the `CONTACTS` empty form (the brief prints it only when the
    /// frame holds no contact) but answers nothing in the lane's ledger
    /// ([`Applied::opened`] `None`), and its end line answers nothing either,
    /// so a `CONTACTS`, `STATUS` or `FIRE` typed after `VIEW BRIEF` keeps its
    /// own line.
    pub brief: bool,
}

/// The pilot's picture of the match.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Picture {
    pub phase: Phase,
    /// The match this picture follows: the first `Match {n} is boarding` after
    /// [`Picture::claimed`]. `Seated`, `Running` and `Over` move only on lines
    /// about it.
    pub following: Option<u64>,
    pub room: Option<String>,
    pub side: Option<Side>,
    /// The character's own display name, from `Connected as`.
    pub me: Option<String>,
    /// In the cockpit of a tank.
    pub aboard: bool,
    /// Tanks taken back after a drop, this match.
    pub reclaims: u32,
    /// How long the last drop lasted.
    pub away_ms: u64,
    /// When `Power online.` first came this match (cut AJ.3: the commander's
    /// picture counts its seconds from it) — or, after a resume into a powered
    /// tank, the first of `Power is already online.`, a `Back aboard` block's
    /// `  Power online.` row, or a running match's `STATUS` head reading
    /// `power online` (cut AK.2).
    pub power_ms: Option<u64>,
    pub own: Own,
    pub contacts: BTreeMap<String, Contact>,
    /// **`CONTACTS` replies read** (cut AM.1), each counted once at its first
    /// apply or its close, whichever comes first (`No contacts.` counts): the
    /// instant the whole reply used to be committed, so the lane's lock
    /// `RANGE` keeps its window.
    pub contacts_read: u64,
    pub lock: Option<String>,
    pub lock_friendly: bool,
    pub lock_percent: Option<u32>,
    pub lock_read_ms: u64,
    pub weapons: Vec<Weapon>,
    /// **Guns the tank no longer carries** (cut AL.1): each id a ``That unit
    /// carries no weapon called `{w}`.`` named this match. A struck gun is out
    /// of [`Picture::weapons`], and no `WEAPONSTATUS` row puts it back.
    pub struck: BTreeSet<String>,
    /// **Every gun is in doubt since then** (cut AL.1): set by a lost section
    /// whose cost is `its weapons are gone`, and cleared by the next
    /// `WEAPONSTATUS` reply to close (cut AM.1: not by its rows applying), by
    /// `This unit carries no weapon.` and by a new match. While it is set the
    /// pilot fires nothing, and the lane asks `WEAPONSTATUS` once for it at
    /// once.
    pub weapons_doubt: Option<u64>,
    pub spots: BTreeMap<char, Spot>,
    /// **The ground the last `VIEW BRIEF` described** (cut AN.2); `None` until
    /// one is read this match, and after a dark window.
    pub ground: Option<Ground>,
    /// **The rule the board says** (cut AN.2): a skirmish at every board head
    /// ([`Line::Board`]) and the mode its `  mode: …` line names. A new
    /// followed match keeps it: the board said it for this match.
    pub match_mode: MatchMode,
    /// **This match's power was seen to come up** (cut AN.2): set only when the
    /// ordinary column-0 `Power online.` is the line that sets
    /// [`Picture::power_ms`] — not `Power is already online.`, a `Back
    /// aboard` block's row or a running `STATUS` head, which a bot restarted
    /// or attached into a running match reads. Cleared by a new match.
    pub powered_here: bool,
    /// **The arena's lattice** (cut AN.3): the knob, its samples and round
    /// trips ([`Lattice`]).
    pub lattice: Lattice,
    pub tally: Tally,
    pub events: VecDeque<Event>,
    /// Bumped on every change.
    pub revision: u64,
    /// The latest `now_ms` [`Picture::apply`] or [`Picture::settle`] was given.
    pub last_ms: u64,
    /// The replies being read now.
    pub open: Open,
    /// What the picture said of the guns since [`Picture::take_notes`] (cut
    /// AL.1): a gun struck, emptied, or every gun held.
    #[serde(skip)]
    pub notes: Vec<String>,
}

fn is_status_form(line: &Line) -> bool {
    matches!(
        line,
        Line::StatusCallsign { .. }
            | Line::StatusHead(_)
            | Line::StatusPosition { .. }
            | Line::StatusNearestSpot { .. }
            | Line::StatusHeat { .. }
            | Line::StatusReceiver { .. }
            | Line::StatusEmcon { .. }
            | Line::StatusSection { .. }
            | Line::StatusFacets { .. }
    )
}

/// The bearing from `from` to `to`, degrees clockwise from north in `0..360`
/// (+x east, +y north).
pub fn bearing_deg(from: (f64, f64), to: (f64, f64)) -> f64 {
    (to.0 - from.0)
        .atan2(to.1 - from.1)
        .to_degrees()
        .rem_euclid(360.0)
}

/// Metres between two points.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The raw least-squares fit over a contact's fixes inside the track window,
/// before any gate is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    vx: f64,
    vy: f64,
    speed_mps: f64,
    fixes: usize,
    span_ms: u64,
    mean_sigma: f64,
}

/// The least-squares velocity over the fixes of the last [`TRACK_WINDOW_MS`],
/// weighted by each fix's `1/σ²`; `None` only when they do not span two
/// distinct times.
fn fit(fixes: &VecDeque<Fix>, now_ms: u64) -> Option<Fit> {
    let floor = now_ms.saturating_sub(TRACK_WINDOW_MS);
    let recent: Vec<&Fix> = fixes.iter().filter(|fix| fix.at_ms >= floor).collect();
    let first = recent.iter().map(|fix| fix.at_ms).min()?;
    let last = recent.iter().map(|fix| fix.at_ms).max()?;
    let span_ms = last - first;
    let (mut sw, mut st, mut sx, mut sy) = (0.0, 0.0, 0.0, 0.0);
    for fix in &recent {
        let w = 1.0 / fix.sigma_m.max(1.0).powi(2);
        let t = (fix.at_ms - first) as f64 / 1_000.0;
        sw += w;
        st += w * t;
        sx += w * fix.pos_m.0;
        sy += w * fix.pos_m.1;
    }
    let (tbar, xbar, ybar) = (st / sw, sx / sw, sy / sw);
    let (mut stt, mut stx, mut sty) = (0.0, 0.0, 0.0);
    for fix in &recent {
        let w = 1.0 / fix.sigma_m.max(1.0).powi(2);
        let dt = (fix.at_ms - first) as f64 / 1_000.0 - tbar;
        stt += w * dt * dt;
        stx += w * dt * (fix.pos_m.0 - xbar);
        sty += w * dt * (fix.pos_m.1 - ybar);
    }
    if stt <= f64::EPSILON {
        return None;
    }
    let (vx, vy) = (stx / stt, sty / stt);
    Some(Fit {
        vx,
        vy,
        speed_mps: vx.hypot(vy),
        fixes: recent.len(),
        span_ms,
        mean_sigma: recent.iter().map(|fix| fix.sigma_m).sum::<f64>() / recent.len() as f64,
    })
}

/// A contact's track: the [`fit`], published only when it is motion the fixes
/// can carry. Four gates, each able to refuse on its own: at least
/// [`TRACK_MIN_FIXES`] fixes, spanning at least [`TRACK_SPAN_MS`], a speed of at
/// least [`TRACK_MIN_MPS`], and a fitted displacement across the span of at
/// least three times the fixes' mean `σ` (so a held 50 m scope figure that
/// steps, or a stopped machine read from a moving tank, does not fit as motion).
/// With the fixes this picture makes (`σ` at least 10 m) and the window's
/// 10 s, the displacement gate already implies the speed floor; the floor is
/// kept as its own gate so a change of either constant cannot quietly drop it.
fn fit_track(fixes: &VecDeque<Fix>, now_ms: u64) -> Option<Track> {
    let fit = fit(fixes, now_ms)?;
    let span_s = fit.span_ms as f64 / 1_000.0;
    if fit.fixes < TRACK_MIN_FIXES
        || fit.span_ms < TRACK_SPAN_MS
        || !fit.speed_mps.is_finite()
        || fit.speed_mps < TRACK_MIN_MPS
        || fit.speed_mps * span_s < 3.0 * fit.mean_sigma
    {
        return None;
    }
    Some(Track {
        course_deg: fit.vx.atan2(fit.vy).to_degrees().rem_euclid(360.0),
        speed_mps: fit.speed_mps,
    })
}

fn clip(line: String) -> String {
    if line.chars().count() <= SUMMARY_COLUMNS {
        return line;
    }
    line.chars().take(SUMMARY_COLUMNS).collect()
}

impl Picture {
    /// Fold one line in. `now_ms` is the runner's session clock when the line
    /// was read.
    pub fn apply(&mut self, now_ms: u64, line: &Line) -> Applied {
        self.last_ms = self.last_ms.max(now_ms);
        // **A reply ends at the first line after it that is not one of its
        // rows** (cut AM.1: and, for `CONTACTS` and `WEAPONSTATUS`, not
        // chatter, a hint or a receiver notice; a pause never ends one). It
        // closes here, before the line's own effect: the game prints a reply's
        // lines back to back, so a notice read after the rows was printed after
        // them, and the reply's whole-list rule never undoes it.
        let neutral = neutral(line);
        if !neutral && !matches!(line, Line::ContactRow(_)) {
            self.close_contacts(now_ms);
        }
        // The game's 17b cut M1b.4: an armed weapon's arming line sits under its
        // row inside the reply, so it is one of the reply's lines and ends
        // nothing.
        if !neutral && !matches!(line, Line::WeaponRow(_) | Line::WeaponArming { .. }) {
            self.close_weapons(now_ms);
        }
        if !is_status_form(line) {
            self.close_status();
        }
        // Chatter inside a reply is read as itself: the rows so far apply, and
        // the reply stays open for the rest.
        if neutral {
            self.apply_contacts(now_ms);
            self.apply_weapons();
        }
        if !matches!(line, Line::Other(_) | Line::CatchupPower { .. }) {
            self.open.catchup = false;
        }
        // The lock line heads the first `WEAPONSTATUS` row after it across a
        // pause and chatter; any other line ends it (cut AM.1).
        let lock_line_before = self.open.lock_line;
        if !neutral {
            self.open.lock_line = false;
        }
        // A `STATUS` row is one only inside the block its `Callsign` line
        // opened: a row read anywhere else (a four-space facet run is what a
        // three-letter speaker's wrapped speech looks like) is someone else's
        // words, and it changes nothing.
        if is_status_form(line)
            && !matches!(line, Line::StatusCallsign { .. })
            && self.open.status.is_none()
        {
            return Applied {
                journal: true,
                event: None,
                opened: None,
            };
        }
        let mut journal = true;
        let mut event = None;
        let mut opened = None;
        // **The brief's ground is whole** (cut AN.2) when the brief closes:
        // at its `End of brief.` (the game's cut AP.4), or at a line of no
        // brief's kind should that never come — not at one of its bearings,
        // its own empty sentence, its contact sentences and spot lines, or
        // chatter or a wrapped piece read among them ([`Open::brief`]). A
        // `No contacts.` read inside an open brief keeps it too: the game no
        // longer prints it there, so it is read as nothing at all.
        let brief_kept = matches!(
            line,
            Line::ViewHeader { .. } | Line::GroundRay { .. } | Line::NoContactsInWindow
        ) || (self.open.brief && matches!(line, Line::NoContacts))
            || neutral;
        if self.open.brief
            && !brief_kept
            && let Some(ground) = self.ground.as_mut()
        {
            ground.complete = true;
        }
        match line {
            // -- read replies ------------------------------------------------
            Line::ContactRow(row) => {
                journal = false;
                // A designator the open reply already holds begins the next
                // reply (two back to back, nothing between): the first closes.
                if self.open.contacts.as_ref().is_some_and(|open| {
                    open.rows
                        .iter()
                        .any(|(held, _)| held.designator == row.designator)
                }) {
                    self.close_contacts(now_ms);
                }
                if self.open.contacts.is_none() {
                    opened = Some(AskKind::Contacts);
                }
                let open = self
                    .open
                    .contacts
                    .get_or_insert_with(ContactsReply::default);
                open.rows.push((row.clone(), now_ms));
                open.last_ms = now_ms;
            }
            Line::NoContacts => {
                journal = false;
                // Inside an open brief it is nothing: since the game's cut
                // AP.4 the brief says `No contacts in the window.` in its
                // place and never `CONTACTS`' `No contacts.`, so this line is
                // no brief's and no reply the reader can place.
                if !self.open.brief {
                    opened = Some(AskKind::Contacts);
                    // The empty form is a whole reply of its own.
                    self.open.contacts = Some(ContactsReply::default());
                    self.close_contacts(now_ms);
                }
            }
            Line::NoContactsInWindow => {
                journal = false;
                // The brief's own empty sentence (the game's cut AP.4) is
                // applied as the `CONTACTS` empty form, the frame holding no
                // contact, and opens no reply in the lane's ledger. Outside a
                // brief it is nothing.
                if self.open.brief {
                    self.open.contacts = Some(ContactsReply::default());
                    self.close_contacts(now_ms);
                }
            }
            Line::BriefEnd => {
                // The brief's close (the game's cut AP.4): the brief shuts
                // below (it is not a kept line) and its ground is whole above.
                // It opens nothing; outside a brief it is nothing.
                journal = false;
            }
            Line::Lock {
                designator,
                friendly,
                percent,
            } => {
                // The one sentence two things print: `WEAPONSTATUS`'s head and the
                // arena's `Locked` notice. Its state is the same either way; it is
                // news, and journalled, only when it changes the lock.
                let news = self.lock.as_deref() != Some(designator.as_str())
                    || self.lock_friendly != *friendly;
                self.lock = Some(designator.clone());
                self.lock_friendly = *friendly;
                self.lock_percent = *percent;
                self.lock_read_ms = now_ms;
                self.open.lock_line = true;
                self.touch();
                journal = news;
                if news {
                    event = Some(self.event(
                        now_ms,
                        EventKind::Locked,
                        match percent {
                            Some(p) => format!("Locked on {designator}, {p}% to hit."),
                            None => format!("Locked on {designator}."),
                        },
                    ));
                }
            }
            Line::WeaponRow(row) => {
                journal = false;
                // A gun the open reply already holds begins the next reply.
                if self
                    .open
                    .weapons
                    .as_ref()
                    .is_some_and(|open| open.rows.iter().any(|(held, _)| held.id == row.id))
                {
                    self.close_weapons(now_ms);
                }
                if self.open.weapons.is_none() {
                    opened = Some(AskKind::Weapons);
                }
                let open = self.open.weapons.get_or_insert_with(|| WeaponsReply {
                    locked: lock_line_before,
                    ..WeaponsReply::default()
                });
                open.rows.push((row.clone(), now_ms));
                open.last_ms = now_ms;
            }
            // The game's 17b cut M1b.4: read, never acted on.
            Line::WeaponArming { .. } => {
                journal = false;
            }
            Line::NoWeapon => {
                journal = false;
                opened = Some(AskKind::Weapons);
                self.weapons.clear();
                // It says what is left (cut AL.1): nothing, so nothing is in
                // doubt.
                self.weapons_doubt = None;
                self.touch();
            }
            Line::NoSuchWeapon { weapon } => {
                journal = false;
                // The answer to that gun's `FIRE` (cut AL.1): the tank has lost
                // it, so it is struck for the match and never typed at again.
                self.take_fire_mark(now_ms, Some(weapon));
                let mut gone: Vec<String> = self
                    .weapons
                    .iter()
                    .filter(|w| w.named(weapon))
                    .map(|w| w.id.clone())
                    .collect();
                self.weapons.retain(|w| !w.named(weapon));
                gone.push(weapon.clone());
                let mut news = false;
                for id in gone {
                    news |= self.struck.insert(id);
                }
                if news {
                    self.notes.push(format!(
                        "No {weapon} aboard any more: struck from the fire schedule."
                    ));
                }
                self.touch();
            }
            Line::StatusCallsign { callsign } => {
                journal = false;
                opened = Some(AskKind::Status);
                // A second `Callsign` begins the next block: the first closes.
                self.close_status();
                self.own.callsign = Some(callsign.clone());
                // What the block does not print it no longer has.
                self.own.heat = None;
                self.own.overheated = false;
                self.own.jammed = false;
                self.own.emcon = None;
                self.open.status = Some(StatusBlock {
                    last_ms: now_ms,
                    ..StatusBlock::default()
                });
                self.open.section = None;
                self.touch();
            }
            Line::StatusHead(head) => {
                journal = false;
                self.own.name = Some(head.name.clone());
                self.own.chassis = head.chassis.clone();
                self.own.power = head.power;
                // The tank's own panel says the power is up in a running
                // match (cut AK.2): a resume read no `Power online.`.
                if head.power == Power::Online && matches!(self.phase, Phase::Running { .. }) {
                    self.power_ms.get_or_insert(now_ms);
                }
                self.own.speed_kph = head.kph;
                self.own.heading_deg = head.heading_deg;
                self.own.held = head
                    .held
                    .then(|| self.own.held.unwrap_or(HeldCause::Unstated));
                self.own.destroyed = head.destroyed;
                self.status_row(now_ms);
            }
            Line::StatusPosition { x_m, y_m, scope } => {
                journal = false;
                self.own.pos_m = Some((*x_m as f64, *y_m as f64));
                self.own.pos_read_ms = now_ms;
                self.own.scope = *scope;
                if let Some(block) = self.open.status.as_mut() {
                    block.pos_m = self.own.pos_m;
                }
                self.status_row(now_ms);
            }
            Line::StatusNearestSpot {
                name,
                letter,
                range_m,
                bearing_deg,
            } => {
                journal = false;
                // Range and bearing from the block's own position: where the
                // spot lies. Without that row, the spot keeps what it had.
                let plotted_m = self
                    .open
                    .status
                    .as_ref()
                    .and_then(|block| block.pos_m)
                    .map(|(x, y)| {
                        let range = f64::from(*range_m);
                        let bearing = f64::from(*bearing_deg).to_radians();
                        (
                            (x + range * bearing.sin()).round() as i64,
                            (y + range * bearing.cos()).round() as i64,
                        )
                    })
                    .or_else(|| self.spots.get(letter).and_then(|spot| spot.plotted_m));
                self.spots.insert(
                    *letter,
                    Spot {
                        name: name.clone(),
                        range_m: Some(*range_m),
                        bearing_deg: Some(*bearing_deg),
                        read_ms: now_ms,
                        plotted_m,
                    },
                );
                self.status_row(now_ms);
            }
            Line::StatusHeat {
                heat,
                max,
                overheated,
            } => {
                journal = false;
                self.own.heat = Some((*heat, *max));
                self.own.overheated = *overheated;
                self.status_row(now_ms);
            }
            Line::StatusReceiver { .. } => {
                journal = false;
                self.own.jammed = true;
                self.status_row(now_ms);
            }
            Line::StatusEmcon { on } => {
                journal = false;
                self.own.emcon = Some(*on);
                self.status_row(now_ms);
            }
            Line::StatusSection {
                id,
                condition,
                internals,
            } => {
                journal = false;
                // Staged in the open block (the guard above holds one open).
                if let Some(block) = self.open.status.as_mut() {
                    block.applied = false;
                    block.sections.insert(
                        id.clone(),
                        Section {
                            condition: condition.clone(),
                            internals: *internals,
                            facets: BTreeMap::new(),
                        },
                    );
                }
                self.open.section = Some(id.clone());
                self.status_row(now_ms);
            }
            Line::StatusFacets { facets } => {
                journal = false;
                if let Some(id) = self.open.section.clone()
                    && let Some(section) = self.open.status.as_mut().and_then(|block| {
                        block.applied = false;
                        block.sections.get_mut(&id)
                    })
                {
                    for (facet, left, max) in facets {
                        section.facets.insert(facet.clone(), (*left, *max));
                    }
                }
                self.status_row(now_ms);
            }
            Line::StatusUntold { name } => {
                journal = false;
                opened = Some(AskKind::Status);
                self.own.name = Some(name.clone());
                self.own.power = Power::Off;
                self.touch();
            }
            Line::NoFrameYet | Line::NoSuchContact => journal = false,
            // -- `VIEW BRIEF` (cut AN.2) ---------------------------------------
            Line::ViewHeader {
                extent,
                cell_m,
                you: _,
            } => {
                journal = false;
                let metres = f64::from(*cell_m);
                self.ground = Some(Ground {
                    read_ms: now_ms,
                    from_m: self.own_pos_at(now_ms),
                    side_m: Some((f64::from(extent.0) * metres, f64::from(extent.1) * metres)),
                    cell_m: Some(*cell_m),
                    rays: BTreeMap::new(),
                    complete: false,
                });
                self.open.brief = true;
                self.touch();
            }
            Line::GroundRay {
                bearing_deg,
                runs,
                edge,
            } => {
                journal = false;
                // A ray belongs to the brief being read, or to a ground begun
                // within one output's span; else it begins a ground of its
                // own, with no header's size.
                let current = self.ground.as_ref().is_some_and(|ground| {
                    self.open.brief || now_ms <= ground.read_ms.saturating_add(STALE_TAIL_MS)
                });
                if !current {
                    self.ground = Some(Ground {
                        read_ms: now_ms,
                        from_m: self.own_pos_at(now_ms),
                        ..Ground::default()
                    });
                }
                if let Some(ground) = self.ground.as_mut() {
                    ground.rays.insert(*bearing_deg, (runs.clone(), *edge));
                    if ground.rays.len() >= 8 {
                        ground.complete = true;
                    }
                }
                // Only `VIEW BRIEF` prints a bearing line: the brief is open
                // even when its header was wrapped into pieces the reader
                // could not take, so its `No contacts in the window.` still
                // answers nothing.
                self.open.brief = true;
                self.touch();
            }
            Line::WindowDark => {
                journal = false;
                self.ground = None;
                self.touch();
            }
            Line::Range {
                designator,
                range_m,
                plotted_s,
            } => {
                journal = false;
                self.range_reply(now_ms, designator, *range_m, *plotted_s);
            }
            Line::Bearing {
                designator,
                bearing_deg,
                rel_deg,
                ..
            } => {
                journal = false;
                if let Some(contact) = self.contacts.get_mut(designator) {
                    contact.bearing_deg = *bearing_deg;
                    contact.rel_deg = *rel_deg;
                    self.touch();
                }
            }
            Line::SpotRange {
                name,
                letter,
                range_m,
            } => {
                journal = false;
                let spot = self.spot(*letter, name, now_ms);
                spot.range_m = Some(*range_m);
                self.touch();
            }
            Line::SpotBearing {
                name,
                letter,
                bearing_deg,
                ..
            } => {
                journal = false;
                let spot = self.spot(*letter, name, now_ms);
                spot.bearing_deg = Some(*bearing_deg);
                self.touch();
            }
            Line::NoLock | Line::NoLockOrName => {
                journal = false;
                self.clear_lock(now_ms);
            }
            Line::SpeedLimits {
                ahead_kph,
                astern_kph,
            } => {
                journal = false;
                self.own.max_ahead_kph = Some(*ahead_kph);
                self.own.max_astern_kph = Some(*astern_kph);
                self.touch();
            }
            Line::DestroyedNoOrders => {
                journal = false;
                self.own.destroyed = true;
                self.touch();
            }
            Line::EmconReading { on } => {
                journal = false;
                self.own.emcon = Some(*on);
                self.touch();
            }

            // -- acks --------------------------------------------------------
            Line::StartupBegun | Line::PowerStarting => {
                journal = false;
                self.own.power = Power::Starting;
                self.touch();
            }
            Line::PowerAlreadyOnline => {
                journal = false;
                self.own.power = Power::Online;
                // A resume into a powered tank (cut AK.2): the commander's
                // `t_s` counts from here when no `Power online.` was read.
                self.power_ms.get_or_insert(now_ms);
                self.touch();
            }
            Line::PowerNotOnline => {
                journal = false;
                self.own.power = Power::Off;
                self.touch();
            }
            Line::Immobile => {
                journal = false;
                self.own.immobile = true;
                self.touch();
            }
            Line::Steering { .. }
            | Line::Making { .. }
            | Line::Locking { .. }
            | Line::EmconAck { .. } => journal = false,
            Line::Firing { weapon } => {
                journal = false;
                // The ack is the answer to the oldest `FIRE` of this weapon
                // still waiting for one; that `FIRE`'s send bounds the cycle's
                // read (cut AM.1).
                let sent = self.take_fire_mark(now_ms, Some(weapon));
                self.tally.shots = self.tally.shots.saturating_add(1);
                // Cut AN.3: the round trip, and the lattice as it stands.
                if let Some(sent) = sent {
                    self.lattice.rtts.push_back(now_ms.saturating_sub(sent));
                    while self.lattice.rtts.len() > LATTICE_RTT_SAMPLES {
                        self.lattice.rtts.pop_front();
                    }
                }
                let lattice = self
                    .lattice
                    .on
                    .then(|| self.lattice.phase().zip(self.lattice.rtt()));
                let entry = self.weapon_mut(weapon);
                entry.fired_ms = Some(now_ms);
                entry.fired_sent_ms = sent;
                entry.ready = false;
                // The one clock (cut AK.1; cut AL.1): the ack lowers the slack
                // toward nothing, then the gun is ready one learned cycle and
                // that slack on — never sooner, whatever an earlier row left
                // behind. With no cycle learned yet, the row is read (and the
                // lane asks for it inside the second, while it prints the
                // whole cycle).
                entry.slack_ms = slack_after_ack(entry.slack_ms);
                let slack = entry.slack_ms;
                entry.next_ready_ms = entry
                    .cycle_ms
                    .map(|cycle| now_ms.saturating_add(cycle).saturating_add(slack));
                // Cut AN.3: on the lattice, when the knob is on, the cycle is
                // learned, the phase and a round trip short enough are known,
                // and the gun is not benched — the next lattice point after
                // the ack, the cycle and the guard, less the round trip.
                let benched = entry
                    .lattice_bench_until_ms
                    .is_some_and(|until| now_ms < until);
                let on_lattice = match (lattice.flatten(), entry.cycle_ms) {
                    (Some((p, r)), Some(cycle)) if r <= LATTICE_RTT_MAX_MS && !benched => {
                        entry.next_ready_ms = Some(
                            next_lattice(now_ms, p)
                                .saturating_add(cycle)
                                .saturating_add(LATTICE_GUARD_MS)
                                .saturating_sub(r),
                        );
                        Some((p, r))
                    }
                    _ => None,
                };
                entry.on_lattice = on_lattice.is_some();
                entry.shots_since_read = entry.shots_since_read.saturating_add(1);
                entry.acks.push_back(now_ms);
                while entry.acks.len() > MAX_ACKS {
                    entry.acks.pop_front();
                }
                if let Some((p, r)) = on_lattice
                    && !self.lattice.said
                {
                    self.lattice.said = true;
                    self.notes.push(format!(
                        "Firing on the arena's lattice: phase {p} ms, round trip {r} ms."
                    ));
                }
                self.touch();
            }

            // -- the unseated, reclaim and resume sentences ------------------
            Line::OutsideCockpit => {
                journal = false;
                self.set_aboard(false);
            }
            Line::AlreadyAboard
            | Line::NotStartedYet
            | Line::ClimbBack { .. }
            | Line::BackAtStation { .. } => self.set_aboard(true),
            Line::LeftForGood { .. } | Line::ClimbOut { .. } | Line::Resumed => {
                self.set_aboard(false);
            }
            Line::BackAboard { callsign, away_ms } => {
                self.reclaims = self.reclaims.saturating_add(1);
                self.away_ms = *away_ms;
                if self.own.callsign.is_none() {
                    self.own.callsign = Some(callsign.clone());
                }
                self.aboard = true;
                self.open.catchup = true;
                self.touch();
            }
            // The block's own power row, and only inside the block: the same
            // words two-space indented anywhere else are not the tank's.
            Line::CatchupPower { power } => {
                if std::mem::replace(&mut self.open.catchup, false) {
                    self.own.power = *power;
                    if *power == Power::Online {
                        self.power_ms.get_or_insert(now_ms);
                    }
                    self.touch();
                }
            }
            Line::OnItsMind { tank } => {
                if self.own.name.as_deref().is_none_or(|own| own == tank) {
                    self.set_aboard(false);
                }
            }
            Line::Connected { name, .. } => {
                self.me = Some(name.clone());
                if self.phase == Phase::Unknown {
                    event = self.set_phase(now_ms, Phase::Walking);
                }
                self.touch();
            }

            // -- notices -----------------------------------------------------
            Line::NewContact {
                designator,
                bearing_deg,
                band,
            } => {
                self.lattice_sample(now_ms);
                // Cut AN.1: the side the notice's designator is held as, if
                // any (a teammate a wingman reported is held as the own side).
                let held = self.contacts.get(designator).and_then(|c| c.side);
                self.first_contact(now_ms, held);
                let contact = self
                    .contacts
                    .entry(designator.clone())
                    .or_insert_with(|| Contact::new(designator, now_ms));
                if let Some(bearing) = bearing_deg {
                    contact.bearing_deg = *bearing;
                }
                if let Some(band) = band {
                    contact.band = band.clone();
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::ContactNew,
                    format!("New contact {designator}."),
                ));
            }
            Line::Identified {
                was,
                designator,
                side,
            } => {
                self.lattice_sample(now_ms);
                self.first_contact(now_ms, Some(*side));
                self.rekey(now_ms, was, designator, Some(*side));
                event = Some(self.event(
                    now_ms,
                    EventKind::Identified,
                    format!("Contact {was} identified: {designator}, {}.", side.as_str()),
                ));
            }
            Line::Unidentified { was, designator } => {
                self.rekey(now_ms, was, designator, None);
                event = Some(self.event(
                    now_ms,
                    EventKind::Unidentified,
                    format!("{was} is unidentified again: {designator}."),
                ));
            }
            Line::LockBroken { designator } => {
                self.clear_lock(now_ms);
                event = Some(self.event(
                    now_ms,
                    EventKind::LockBroken,
                    format!("Lock on {designator} broken."),
                ));
            }
            Line::ContactLost { designator } => {
                self.lattice_sample(now_ms);
                self.contacts.remove(designator);
                if self.lock.as_deref() == Some(designator.as_str()) {
                    self.clear_lock(now_ms);
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::Lost,
                    format!("Contact {designator} lost."),
                ));
            }
            Line::HitScored {
                designator,
                friendly,
                section,
                ..
            } => {
                self.lattice_sample(now_ms);
                self.tally.hits_scored = self.tally.hits_scored.saturating_add(1);
                // Cut AN.1: the first hit scored on the other side.
                if !*friendly && self.running() {
                    self.tally.first_hit_scored_ms.get_or_insert(now_ms);
                }
                self.touch();
                // Cut AN.2: a friendly hit keeps its `(FRIENDLY)` mark, so the
                // pilot's exchange counts only hits on the other side.
                let mark = if *friendly { " (FRIENDLY)" } else { "" };
                let text = match (designator, section) {
                    (Some(d), Some(s)) => format!("Hit on {d}{mark}: {s}."),
                    (Some(d), None) => format!("Hit on {d}{mark}."),
                    (None, _) => "Hit.".to_owned(),
                };
                event = Some(self.event(now_ms, EventKind::HitScored, text));
            }
            Line::Missed { designator, words } => {
                self.lattice_sample(now_ms);
                self.tally.misses = self.tally.misses.saturating_add(1);
                self.touch();
                let text = match (designator, words) {
                    (Some(d), Some(w)) => format!("Miss on {d}: {w}."),
                    (None, Some(w)) => format!("Miss: {w}."),
                    _ => "Miss.".to_owned(),
                };
                event = Some(self.event(now_ms, EventKind::Missed, text));
            }
            Line::Killed { designator } => {
                self.tally.kills = self.tally.kills.saturating_add(1);
                self.touch();
                let text = match designator {
                    Some(d) => format!("{d} destroyed."),
                    None => "Target destroyed.".to_owned(),
                };
                event = Some(self.event(now_ms, EventKind::Kill, text));
            }
            Line::TargetSectionLost {
                designator,
                section,
            } => {
                event = Some(self.event(
                    now_ms,
                    EventKind::TargetSectionLost,
                    format!("{designator}'s {section} is destroyed."),
                ));
            }
            Line::HitTaken {
                section,
                through,
                facet,
                armour,
                internals,
                already_destroyed,
            } => {
                self.lattice_sample(now_ms);
                self.tally.hits_taken = self.tally.hits_taken.saturating_add(1);
                // Cut AN.1: the first hit taken.
                if self.running() {
                    self.tally.first_hit_taken_ms.get_or_insert(now_ms);
                }
                let plate = through.as_ref().unwrap_or(section).clone();
                if let Some(facet) = facet {
                    // Cut AL.2: the facet split, and the newest facet hit for
                    // the pilot's turn into the shot.
                    let count = self
                        .tally
                        .hits_taken_by_facet
                        .entry(format!("{plate} {facet}"))
                        .or_default();
                    *count = count.saturating_add(1);
                    self.own.last_hit = Some(FacetHit {
                        at_ms: now_ms,
                        plate: plate.clone(),
                        facet: facet.clone(),
                    });
                    let cell = self
                        .own
                        .sections
                        .entry(plate)
                        .or_default()
                        .facets
                        .entry(facet.clone())
                        .or_insert((0, 0));
                    *cell = match armour {
                        Some(figures) => *figures,
                        None => (0, cell.1),
                    };
                }
                if let Some(inside) = internals {
                    self.own
                        .sections
                        .entry(section.clone())
                        .or_default()
                        .internals = Some(*inside);
                }
                if *already_destroyed {
                    self.own
                        .sections
                        .entry(section.clone())
                        .or_default()
                        .condition = "destroyed".to_owned();
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::HitTaken,
                    format!("Hit on the {section}."),
                ));
            }
            Line::SectionLost { section, cost } => {
                if !self.tally.sections_lost.contains(section) {
                    self.tally.sections_lost.push(section.clone());
                }
                self.own
                    .sections
                    .entry(section.clone())
                    .or_default()
                    .condition = "destroyed".to_owned();
                if cost
                    .as_deref()
                    .is_some_and(|cost| cost.contains("the tank cannot move"))
                {
                    self.own.immobile = true;
                }
                // `The turret is destroyed: its weapons are gone.` (cut AL.1;
                // from 17b cut M3.6 the shipped line goes on `; its core is
                // gone.` or `; its core is degraded.`, so the clause is looked
                // for anywhere in the cost, never at its end): which guns went
                // with it only `WEAPONSTATUS` says, so every gun is held until
                // it does (the doubt keeps its first time).
                if cost
                    .as_deref()
                    .is_some_and(|cost| cost.contains("its weapons are gone"))
                    && self.weapons_doubt.is_none()
                {
                    self.weapons_doubt = Some(now_ms);
                    self.notes.push(format!(
                        "The {section}'s weapons are gone: no FIRE until WEAPONSTATUS says what \
                         is left."
                    ));
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::SectionLost,
                    format!("The {section} is destroyed."),
                ));
            }
            Line::OwnDestroyed => {
                self.own.destroyed = true;
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::OwnDestroyed,
                    "Your tank is destroyed.".to_owned(),
                ));
            }
            Line::OwnDisabled => {
                self.own.disabled = true;
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::OwnDisabled,
                    "Your tank can neither move nor shoot.".to_owned(),
                ));
            }
            Line::PowerOnline => {
                self.own.power = Power::Online;
                self.aboard = true;
                // Cut AN.2: this match's power seen to come up, only when this
                // line is the one that says so first.
                if self.power_ms.is_none() {
                    self.power_ms = Some(now_ms);
                    self.powered_here = true;
                }
                self.touch();
                event = Some(self.event(now_ms, EventKind::PowerOnline, "Power online.".into()));
            }
            Line::Held { cause } => {
                self.lattice_sample(now_ms);
                self.own.held = Some(*cause);
                self.touch();
                let words = match cause {
                    HeldCause::Impassable => "the ground ahead will not take the tank",
                    HeldCause::Occupied => "something is in the way",
                    HeldCause::Edge => "this is the edge of the arena",
                    HeldCause::TooSteep => "the slope ahead is too steep for the tank",
                    HeldCause::Unstated => "the tank is held",
                };
                event = Some(self.event(now_ms, EventKind::Held, format!("Held: {words}.")));
            }
            Line::ReceiverDegraded | Line::ReceiverClear => {
                self.own.jammed = matches!(line, Line::ReceiverDegraded);
                self.touch();
            }
            Line::EmconSwitched { on, .. } => {
                self.own.emcon = Some(*on);
                self.touch();
            }
            Line::Refused {
                order,
                weapon,
                said,
                cycling_s,
            } => {
                self.refused(now_ms, order, weapon.as_deref(), said, *cycling_s);
                let text = match weapon {
                    Some(w) => format!("{order} {w} refused. {said}"),
                    None => format!("{order} refused. {said}"),
                };
                event = Some(self.event(now_ms, EventKind::Refused, text));
            }

            // -- the lobby and the centre ------------------------------------
            Line::Board { .. } => {
                // Cut AN.2: a skirmish unless the board's mode line says
                // otherwise.
                self.match_mode = MatchMode::Skirmish;
                if matches!(self.phase, Phase::Unknown | Phase::Walking) {
                    event = self.set_phase(now_ms, Phase::InCentre);
                }
            }
            Line::BoardMode(mode) => {
                self.match_mode = *mode;
                self.touch();
            }
            Line::Boarding { match_id } => {
                if matches!(self.phase, Phase::Claimed { .. }) && self.following.is_none() {
                    self.following = Some(*match_id);
                    self.new_match();
                    event = self.set_phase(
                        now_ms,
                        Phase::Boarding {
                            match_id: *match_id,
                        },
                    );
                }
            }
            Line::ClimbIn { .. } => {
                self.set_aboard(true);
                if let Phase::Boarding { match_id } = self.phase {
                    event = self.set_phase(now_ms, Phase::Seated { match_id });
                }
            }
            Line::Running { match_id } => {
                if self.following == Some(*match_id)
                    && matches!(self.phase, Phase::Boarding { .. } | Phase::Seated { .. })
                {
                    event = self.set_phase(
                        now_ms,
                        Phase::Running {
                            match_id: *match_id,
                        },
                    );
                }
            }
            Line::Over { match_id, result } => {
                event = self.over(now_ms, *match_id, result.clone());
            }
            Line::Aborted { match_id, .. } | Line::CouldNotStart { match_id, .. } => {
                event = self.over(now_ms, *match_id, ResultWord::Abort);
            }
            Line::AllLeft { match_id } => {
                event = self.over(now_ms, *match_id, ResultWord::Timeout);
            }
            Line::CallsOff { .. } => {
                if let Phase::Boarding { match_id } | Phase::Seated { match_id } = self.phase {
                    event = self.over(now_ms, match_id, ResultWord::Abort);
                }
            }
            Line::HatchOpen { .. } => self.set_aboard(false),
            Line::Room(name) => {
                self.room = Some(name.clone());
                self.touch();
                let centre = is_centre_room(name);
                if centre && matches!(self.phase, Phase::Unknown | Phase::Walking) {
                    event = self.set_phase(now_ms, Phase::InCentre);
                } else if !centre && matches!(self.phase, Phase::Unknown | Phase::InCentre) {
                    event = self.set_phase(now_ms, Phase::Walking);
                }
            }
            Line::Transient(_) => journal = false,
            Line::NotInAPod
            | Line::OutsideTeamRoom
            | Line::HandingOver { .. }
            | Line::BoardSide { .. }
            | Line::AiFill { .. }
            | Line::BoardNext
            | Line::Claimed { .. }
            | Line::GaveUp { .. }
            | Line::ClimbsIn { .. }
            | Line::AbortsMatch { .. }
            | Line::BoardCleared
            | Line::AlreadyHold { .. }
            | Line::SideFull { .. }
            | Line::NotLead
            | Line::EmptySide { .. }
            | Line::WrongCentreState { .. }
            | Line::NoCentre
            | Line::LayerOff
            | Line::NobodyPlaying
            | Line::OutsideCentre
            | Line::NoSlot
            | Line::NothingToAbort
            | Line::Hint(_)
            | Line::Other(_) => {}
        }
        // The brief closes at its first line of any other kind (cut AN.2).
        if !brief_kept {
            self.open.brief = false;
        }
        Applied {
            journal,
            event,
            opened,
        }
    }

    /// **Apply** (cut AM.1) the rows of any open reply whose last row is
    /// [`SNAPSHOT_SETTLE_MS`] old: a merge that drops nothing, clears no lock
    /// and leaves the doubt; the reply stays open for a later row, however
    /// late, and only its close applies the whole-list rule. A lock line not
    /// taken by a row lapses [`STALE_TAIL_MS`] after it was read.
    pub fn settle(&mut self, now_ms: u64) {
        self.last_ms = self.last_ms.max(now_ms);
        let due = |last: u64| now_ms >= last.saturating_add(SNAPSHOT_SETTLE_MS);
        if self.open.contacts.as_ref().is_some_and(|o| due(o.last_ms)) {
            self.apply_contacts(now_ms);
        }
        if self.open.weapons.as_ref().is_some_and(|o| due(o.last_ms)) {
            self.apply_weapons();
        }
        if self.open.status.as_ref().is_some_and(|o| due(o.last_ms)) {
            self.apply_status();
        }
        if self.open.lock_line && now_ms >= self.lock_read_ms.saturating_add(STALE_TAIL_MS) {
            self.open.lock_line = false;
        }
        let mut changed = false;
        for contact in self.contacts.values_mut() {
            let track = fit_track(&contact.fixes, now_ms);
            if track != contact.track {
                contact.track = track;
                changed = true;
            }
        }
        if changed {
            self.touch();
        }
    }

    /// The lane holds `side` slot `index` (it read its own claim, or `You
    /// already hold`): the picture follows the next match that boards.
    pub fn claimed(&mut self, side: Side, index: u8) {
        self.side = Some(side);
        self.following = None;
        let _ = self.set_phase(self.last_ms, Phase::Claimed { side, index });
        self.touch();
    }

    /// **Use the arena's lattice** (cut AN.3; `[arena] lattice`, which the lane
    /// sets once, at its construction): off by default, so a picture made
    /// anywhere else keeps AL's clock.
    pub fn set_lattice(&mut self, on: bool) {
        self.lattice.on = on;
    }

    /// The runner wrote a `FIRE` line (a re-send included) at `now_ms`.
    ///
    /// Every `FIRE` is answered at once by exactly one line of its own —
    /// `Firing {w}.`, or the cockpit's pre-check `FIRE {w} refused. …` — and the
    /// arena's later refusal of an acked shot says the very same `FIRE {w}
    /// refused. …` (a machine gun cycles in a second, so a second `FIRE` inside
    /// two seconds of an ack is routine). A refusal read while a mark is
    /// outstanding is that `FIRE`'s own answer, the pre-check, and never
    /// un-counts a shot that left; one read with no mark outstanding pairs with
    /// an ack, as before. A picture that is never told falls back to the
    /// pairing alone. A mark lapses after [`ACK_PAIR_MS`] (a `FIRE` answered by
    /// something else: a transient, `This unit carries no weapon.`).
    ///
    /// Marks are kept per weapon (cut AK.1): `weapon` is the name the runner
    /// typed, and an answer takes only a mark of its own weapon, so a cannon's
    /// refusal read while only the machine gun's `FIRE` is outstanding is the
    /// arena's refusal of the cannon's acked shot, not a pre-check.
    pub fn sent_fire(&mut self, now_ms: u64, weapon: &str) {
        self.last_ms = self.last_ms.max(now_ms);
        self.lapse_fire_sent(now_ms);
        self.open.fire_sent.push_back((weapon.to_owned(), now_ms));
        while self.open.fire_sent.len() > MAX_ACKS {
            self.open.fire_sent.pop_front();
        }
    }

    /// **A `FIRE` of `weapon` is still unanswered** (cut AL.1): a mark of it (by
    /// its id or its short name, either way round) is outstanding, the marks
    /// older than [`ACK_PAIR_MS`] at [`Picture::last_ms`] lapsed. The lane
    /// never types a gun again while its own last `FIRE` waits.
    pub fn fire_pending(&self, weapon: &str) -> bool {
        let carried = self.weapons.iter().find(|w| w.named(weapon));
        self.open.fire_sent.iter().any(|(sent, at)| {
            self.last_ms.saturating_sub(*at) <= ACK_PAIR_MS
                && (sent == weapon || carried.is_some_and(|w| w.named(sent)))
        })
    }

    /// Whether `a` and `b`, each a name a `FIRE` line, an ack or a refusal
    /// gave a gun, are one gun: equal, or the carried gun named `a` (by its id
    /// or its short name) is named `b`, either way round — the matching
    /// [`Picture::fire_pending`] does (cut AM.1, for the lane's answer ledger).
    pub fn same_gun(&self, a: &str, b: &str) -> bool {
        a == b || self.weapons.iter().any(|w| w.named(a) && w.named(b))
    }

    /// **A `FIRE` of `weapon` was answered `please retry in a moment`** (cut
    /// AM.1; the lane's answer ledger says which line a transient answered):
    /// its oldest mark is taken, so the gun is typed again on its own clock
    /// rather than held until the mark lapses. Nothing else moves: the shot
    /// never left, so the gun's clock, rounds and the tally are as they were.
    /// Whether there was a mark to take.
    pub fn fire_answered_by_transient(&mut self, now_ms: u64, weapon: &str) -> bool {
        self.take_fire_mark(now_ms, Some(weapon)).is_some()
    }

    /// **What the picture said of the guns** since the last call (cut AL.1),
    /// each a sentence for an `arena` event: `No {w} aboard any more: struck
    /// from the fire schedule.`, `The {section}'s weapons are gone: no FIRE
    /// until WEAPONSTATUS says what is left.`, `The {w} is empty: struck from
    /// the fire schedule.`; and (cut AN.3) `Firing on the arena's lattice:
    /// phase {p} ms, round trip {r} ms.` (once a match) and `The {w} is off
    /// the lattice clock for 30 s: refused twice.`
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// The own position at `at_ms`, dead-reckoned from the last `STATUS` read at
    /// the speed and heading read with it.
    pub fn own_pos_at(&self, at_ms: u64) -> Option<(f64, f64)> {
        let (x, y) = self.own.pos_m?;
        let dt = (at_ms as f64 - self.own.pos_read_ms as f64) / 1_000.0;
        let mps = f64::from(self.own.speed_kph) / 3.6;
        let heading = f64::from(self.own.heading_deg).to_radians();
        Some((x + mps * dt * heading.sin(), y + mps * dt * heading.cos()))
    }

    /// The picture in a few lines for a person (`/arena`), each at most
    /// [`SUMMARY_COLUMNS`] columns.
    pub fn summary_lines(&self, now_ms: u64) -> Vec<String> {
        let ago = |at: u64| format!("{:.1} s ago", now_ms.saturating_sub(at) as f64 / 1_000.0);
        let mut lines = Vec::new();
        lines.push(format!(
            "phase {}; room {}; {}; me {}; side {}",
            self.phase.words(),
            self.room.as_deref().unwrap_or("unknown"),
            if self.aboard { "aboard" } else { "not aboard" },
            self.me.as_deref().unwrap_or("unknown"),
            self.side.map_or("none", Side::as_str),
        ));
        let own = &self.own;
        let mut head = format!(
            "own {}: power {}, {} kph, heading {:03}°",
            own.callsign.as_deref().unwrap_or("?"),
            own.power.as_str(),
            own.speed_kph,
            own.heading_deg
        );
        if let Some((x, y)) = own.pos_m {
            head.push_str(&format!(
                ", at {x:.0} m east {y:.0} m north (read {})",
                ago(own.pos_read_ms)
            ));
        }
        for (flag, word) in [
            (own.held.is_some(), "HELD"),
            (own.immobile, "IMMOBILE"),
            (own.disabled, "DISABLED"),
            (own.destroyed, "DESTROYED"),
        ] {
            if flag {
                head.push_str(", ");
                head.push_str(word);
            }
        }
        lines.push(head);
        if !own.sections.is_empty() {
            let words: Vec<String> = own
                .sections
                .iter()
                .map(|(id, section)| format!("{id} {}", section.condition))
                .collect();
            lines.push(format!("sections: {}", words.join(", ")));
        }
        lines.push(match &self.lock {
            Some(lock) => format!(
                "lock {lock}{}{} (read {})",
                if self.lock_friendly {
                    " (FRIENDLY)"
                } else {
                    ""
                },
                self.lock_percent
                    .map_or(String::new(), |p| format!(", {p}% to hit")),
                ago(self.lock_read_ms)
            ),
            None => "no lock".to_owned(),
        });
        for weapon in &self.weapons {
            lines.push(format!(
                "weapon {}: {}, {} of {} round(s){}",
                weapon.id,
                if weapon.ready { "ready" } else { "not ready" },
                weapon.rounds,
                weapon.rounds_max,
                weapon.cycle_ms.map_or(String::new(), |c| format!(
                    ", cycle {:.1} s",
                    c as f64 / 1_000.0
                )),
            ));
        }
        if self.contacts.is_empty() {
            lines.push("no contacts".to_owned());
        }
        for contact in self.contacts.values() {
            lines.push(format!(
                "contact {} {} {:03}° {} {}, plot {} s old{}",
                contact.designator,
                contact.side.map_or("unknown", Side::as_str),
                contact.bearing_deg,
                contact
                    .range_m
                    .map_or("? m".to_owned(), |m| format!("{m} m")),
                contact.band,
                contact.age_ms / 1_000,
                contact
                    .track
                    .as_ref()
                    .map_or(String::new(), |track| format!(
                        ", track {:03.0}° {:.1} m/s",
                        track.course_deg, track.speed_mps
                    )),
            ));
        }
        for (letter, spot) in &self.spots {
            lines.push(format!(
                "spot {letter} {}: {} at {}",
                spot.name,
                spot.range_m.map_or("? m".to_owned(), |m| format!("{m} m")),
                spot.bearing_deg
                    .map_or("?°".to_owned(), |b| format!("{b:03}°"))
            ));
        }
        let tally = &self.tally;
        lines.push(format!(
            "tally: fired {} (acks {}, refused {}), orders refused {}, hits {}, misses {}, kills {}, \
             hits taken {}, sections lost {}",
            tally.fired(),
            tally.shots,
            tally.refused_shots,
            tally.refused_orders,
            tally.hits_scored,
            tally.misses,
            tally.kills,
            tally.hits_taken,
            if tally.sections_lost.is_empty() {
                "none".to_owned()
            } else {
                tally.sections_lost.join(" ")
            }
        ));
        lines.into_iter().map(clip).collect()
    }

    // -- internals -----------------------------------------------------------

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn event(&mut self, at_ms: u64, kind: EventKind, text: String) -> Event {
        let event = Event { at_ms, kind, text };
        self.events.push_back(event.clone());
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        self.touch();
        event
    }

    fn set_phase(&mut self, now_ms: u64, phase: Phase) -> Option<Event> {
        if self.phase == phase {
            return None;
        }
        self.phase = phase;
        let text = format!("phase: {}", self.phase.words());
        Some(self.event(now_ms, EventKind::PhaseChanged, text))
    }

    fn set_aboard(&mut self, aboard: bool) {
        self.aboard = aboard;
        self.touch();
    }

    /// A followed match's end, and only the followed match's.
    fn over(&mut self, now_ms: u64, match_id: u64, result: ResultWord) -> Option<Event> {
        if self.following != Some(match_id) || matches!(self.phase, Phase::Over { .. }) {
            return None;
        }
        self.aboard = false;
        self.set_phase(now_ms, Phase::Over { match_id, result })
    }

    /// A new followed match starts from a clean tank, scope and tally. Any
    /// reply still open closes first (cut AM.1; the line that brought the
    /// match in has closed them already, being no reply's row).
    fn new_match(&mut self) {
        let now = self.last_ms;
        self.close_contacts(now);
        self.close_weapons(now);
        self.close_status();
        self.own = Own::default();
        self.contacts.clear();
        self.lock = None;
        self.lock_friendly = false;
        self.lock_percent = None;
        self.weapons.clear();
        self.struck.clear();
        self.weapons_doubt = None;
        self.spots.clear();
        self.tally = Tally::default();
        self.reclaims = 0;
        self.away_ms = 0;
        self.power_ms = None;
        // Cut AN.2: the ground and the power seen are this match's; the mode
        // is the board's, said for this match.
        self.ground = None;
        self.powered_here = false;
        // Cut AN.3: the lattice is learned afresh each match (the knob is
        // the table's, kept).
        self.lattice.samples.clear();
        self.lattice.rtts.clear();
        self.lattice.said = false;
        self.aboard = false;
        self.open = Open::default();
        self.touch();
    }

    /// Drop the `FIRE` marks older than [`ACK_PAIR_MS`].
    fn lapse_fire_sent(&mut self, now_ms: u64) {
        while self
            .open
            .fire_sent
            .front()
            .is_some_and(|(_, at)| now_ms.saturating_sub(*at) > ACK_PAIR_MS)
        {
            self.open.fire_sent.pop_front();
        }
    }

    /// Take the oldest outstanding `FIRE` mark of `weapon` (its id or its short
    /// name, either way round), after lapsing the old ones; when that `FIRE`
    /// was typed, or `None` when there was no mark (cut AM.1: it was whether
    /// there was one). An answer that names no weapon takes the oldest mark
    /// of any.
    fn take_fire_mark(&mut self, now_ms: u64, weapon: Option<&str>) -> Option<u64> {
        self.lapse_fire_sent(now_ms);
        let index = match weapon {
            None => (!self.open.fire_sent.is_empty()).then_some(0),
            Some(name) => {
                let carried = self.weapons.iter().find(|w| w.named(name));
                self.open
                    .fire_sent
                    .iter()
                    .position(|(sent, _)| sent == name || carried.is_some_and(|w| w.named(sent)))
            }
        };
        index
            .and_then(|index| self.open.fire_sent.remove(index))
            .map(|(_, sent_ms)| sent_ms)
    }

    fn clear_lock(&mut self, now_ms: u64) {
        self.lock = None;
        self.lock_friendly = false;
        self.lock_percent = None;
        self.lock_read_ms = now_ms;
        self.touch();
    }

    fn status_row(&mut self, now_ms: u64) {
        if let Some(block) = self.open.status.as_mut() {
            block.last_ms = now_ms;
        }
        self.touch();
    }

    /// **Apply** an open `STATUS` block (cut AM.1): each section it names
    /// replaces that section of the tank's; the block stays open.
    fn apply_status(&mut self) {
        let Some(block) = self.open.status.as_mut() else {
            return;
        };
        if block.applied {
            return;
        }
        block.applied = true;
        for (id, section) in &block.sections {
            self.own.sections.insert(id.clone(), section.clone());
        }
        self.touch();
    }

    /// **Close** an open `STATUS` block: the tank's sections are the block's
    /// (when it printed any), and the next facet run belongs to no section.
    fn close_status(&mut self) {
        if let Some(block) = self.open.status.take() {
            if !block.sections.is_empty() {
                self.own.sections = block.sections;
            }
            self.open.section = None;
            self.touch();
        }
    }

    fn spot(&mut self, letter: char, name: &str, now_ms: u64) -> &mut Spot {
        let spot = self.spots.entry(letter).or_insert_with(|| Spot {
            name: name.to_owned(),
            range_m: None,
            bearing_deg: None,
            read_ms: now_ms,
            plotted_m: None,
        });
        spot.name = name.to_owned();
        spot.read_ms = now_ms;
        spot
    }

    fn weapon_mut(&mut self, id: &str) -> &mut Weapon {
        let index = match self
            .weapons
            .iter()
            .position(|w| w.id == id || w.short.as_deref() == Some(id))
        {
            Some(index) => index,
            None => {
                self.weapons.push(Weapon::new(id));
                self.weapons.len() - 1
            }
        };
        &mut self.weapons[index]
    }

    /// A contact's rename (`identified`) or its un-naming (`unidentified again`):
    /// the fixes and the track move with it, and so does the lock.
    fn rekey(&mut self, now_ms: u64, was: &str, designator: &str, side: Option<Side>) {
        let mut contact = self
            .contacts
            .remove(was)
            .or_else(|| self.contacts.remove(designator))
            .unwrap_or_else(|| Contact::new(designator, now_ms));
        contact.designator = designator.to_owned();
        contact.was = Some(was.to_owned());
        contact.side = side;
        self.contacts.insert(designator.to_owned(), contact);
        if self.lock.as_deref() == Some(was) {
            self.lock = Some(designator.to_owned());
        }
        self.touch();
    }

    /// A fix from one contact row, or `None` before the first `STATUS`.
    fn row_fix(&self, row: &ContactRow, read_ms: u64) -> Option<Fix> {
        let origin = self.own_pos_at(read_ms)?;
        let range = f64::from(row.range_m);
        let bearing = f64::from(row.bearing_deg).to_radians();
        let reckoned_s = read_ms.saturating_sub(self.own.pos_read_ms) as f64 / 1_000.0;
        Some(Fix {
            at_ms: read_ms.saturating_sub(u64::from(row.age_s) * 1_000 + 500),
            pos_m: (
                origin.0 + range * bearing.sin(),
                origin.1 + range * bearing.cos(),
            ),
            sigma_m: 50.0 + range * 0.5_f64.to_radians() + 3.0 * reckoned_s,
            from_lock: false,
        })
    }

    /// **Apply** an open `CONTACTS` reply (cut AM.1): every row from
    /// `applied` on updates or adds its contact (its fix, track and read
    /// time). Nothing is removed and the lock is untouched; the reply stays
    /// open. The reply's first apply counts it in [`Picture::contacts_read`].
    fn apply_contacts(&mut self, now_ms: u64) {
        let Some(reply) = self.open.contacts.as_mut() else {
            return;
        };
        if reply.applied >= reply.rows.len() {
            return;
        }
        // A reply opened by a row has one, so `applied` is 0 only before its
        // first apply.
        let first = reply.applied == 0;
        let rows: Vec<(ContactRow, u64)> = reply.rows.iter().skip(reply.applied).cloned().collect();
        reply.applied = reply.rows.len();
        if first {
            self.contacts_read = self.contacts_read.saturating_add(1);
        }
        for (row, read_ms) in rows {
            self.contact_row(now_ms, &row, read_ms);
        }
        self.touch();
    }

    /// **Close** an open `CONTACTS` reply: what is left applies, then the
    /// whole-list rule — a reply is the platform's whole list, so a contact it
    /// did not name is gone, and a lock on one with it. A reply closed before
    /// any apply (`No contacts.` always is) is counted in
    /// [`Picture::contacts_read`] here.
    fn close_contacts(&mut self, now_ms: u64) {
        let Some(reply) = self.open.contacts.take() else {
            return;
        };
        if reply.applied == 0 {
            self.contacts_read = self.contacts_read.saturating_add(1);
        }
        for (row, read_ms) in reply.rows.iter().skip(reply.applied) {
            self.contact_row(now_ms, row, *read_ms);
        }
        let named: BTreeSet<&str> = reply
            .rows
            .iter()
            .map(|(row, _)| row.designator.as_str())
            .collect();
        self.contacts
            .retain(|designator, _| named.contains(designator.as_str()));
        if let Some(lock) = &self.lock
            && !self.contacts.contains_key(lock)
        {
            self.clear_lock(now_ms);
        }
        self.touch();
    }

    /// **A lattice sample** (cut AN.3): the read time of one of the arena's
    /// own notices, while the match runs, at most [`LATTICE_SAMPLES`] kept.
    fn lattice_sample(&mut self, now_ms: u64) {
        if !self.running() {
            return;
        }
        self.lattice.samples.push_back(now_ms);
        while self.lattice.samples.len() > LATTICE_SAMPLES {
            self.lattice.samples.pop_front();
        }
    }

    /// The followed match is running (cut AN.1: the report's first times are
    /// set only then).
    fn running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    /// **The first contact of a running match** (cut AN.1): set once, at
    /// `now_ms`, for a contact whose side is `side` — never one read as the
    /// own side ([`Picture::side`]); an unknown (`None`) always counts.
    fn first_contact(&mut self, now_ms: u64, side: Option<Side>) {
        let own = side.is_some() && side == self.side;
        if self.running() && !own {
            self.tally.first_contact_ms.get_or_insert(now_ms);
        }
    }

    /// One `CONTACTS` row folded in: its contact updated or added, with its
    /// fix (unless the same plot was read again), its track and read time.
    fn contact_row(&mut self, now_ms: u64, row: &ContactRow, read_ms: u64) {
        // Cut AN.1: any row not of the own side, an unknown included.
        self.first_contact(now_ms, row.side);
        let fix = self.row_fix(row, read_ms);
        let mut contact = self
            .contacts
            .remove(&row.designator)
            .unwrap_or_else(|| Contact::new(&row.designator, read_ms));
        // The same plot read again has only aged: it is not a second fix.
        let same_plot = contact.range_m == Some(row.range_m)
            && contact.bearing_deg == row.bearing_deg
            && u64::from(row.age_s) * 1_000 > contact.age_ms;
        contact.side = row.side;
        contact.bearing_deg = row.bearing_deg;
        contact.rel_deg = row.rel_deg;
        contact.band = row.band.clone();
        contact.range_m = Some(row.range_m);
        contact.age_ms = u64::from(row.age_s) * 1_000;
        contact.read_ms = read_ms;
        // 17b cut M1a.3: the row's tail, kept as read.
        contact.aspect = row.aspect.clone();
        contact.odds_percent = row.odds.clone().flatten();
        if !same_plot && let Some(fix) = fix {
            contact.push_fix(fix);
        }
        contact.track = fit_track(&contact.fixes, now_ms);
        self.contacts.insert(row.designator.clone(), contact);
    }

    /// **Apply** an open `WEAPONSTATUS` reply (cut AM.1): every row from
    /// `applied` on updates or adds its gun ([`Picture::weapon_row`]: the
    /// stale-ready guard and the cycle included). Nothing is removed, and the
    /// lock and the doubt are untouched; the reply stays open.
    fn apply_weapons(&mut self) {
        let Some(reply) = self.open.weapons.as_mut() else {
            return;
        };
        if reply.applied >= reply.rows.len() {
            return;
        }
        let rows: Vec<(WeaponRow, u64)> = reply.rows.iter().skip(reply.applied).cloned().collect();
        reply.applied = reply.rows.len();
        for (row, read_ms) in rows {
            self.weapon_row(&row, read_ms);
        }
        self.touch();
    }

    /// **Close** an open `WEAPONSTATUS` reply: what is left applies, then the
    /// tank carries exactly the guns it named, in its order (a struck gun
    /// never comes back); the doubt is over, and a reply no lock line headed
    /// means no lock.
    fn close_weapons(&mut self, now_ms: u64) {
        let Some(reply) = self.open.weapons.take() else {
            return;
        };
        for (row, read_ms) in reply.rows.iter().skip(reply.applied) {
            self.weapon_row(row, *read_ms);
        }
        let mut carried = Vec::new();
        for (row, _) in &reply.rows {
            if let Some(index) = self.weapons.iter().position(|w| w.id == row.id) {
                carried.push(self.weapons.remove(index));
            }
        }
        self.weapons = carried;
        // A reply says what is left (cut AL.1): the doubt is over.
        self.weapons_doubt = None;
        // No lock line in front of the rows means no lock.
        if !reply.locked {
            self.clear_lock(now_ms);
        }
        self.touch();
    }

    /// One `WEAPONSTATUS` row folded in: its gun updated or added (a gun the
    /// tank said it does not carry stays struck, cut AL.1), its chance read,
    /// and — unless the stale-ready guard holds the row — its readiness,
    /// rounds, clock and cycle.
    fn weapon_row(&mut self, row: &WeaponRow, read_ms: u64) {
        if self.struck.contains(&row.id)
            || row.short.as_ref().is_some_and(|s| self.struck.contains(s))
        {
            return;
        }
        let index = match self.weapons.iter().position(|w| w.id == row.id) {
            Some(index) => index,
            None => {
                self.weapons.push(Weapon::new(&row.id));
                self.weapons.len() - 1
            }
        };
        let weapon = &mut self.weapons[index];
        // A `ready` row this soon after the ack may be the pre-shot frame —
        // unless its rounds show every acked shot has left since the last
        // row (a machine gun cycles in a second, well inside the guard).
        let shots_left = weapon.read_ms > 0
            && row.rounds.saturating_add(weapon.shots_since_read) <= weapon.rounds;
        let stale_ready = row.ready
            && !shots_left
            && weapon
                .fired_ms
                .is_some_and(|fired| read_ms.saturating_sub(fired) < READY_AFTER_ACK_MS);
        // The chance is the target's, not the shot's (cut AL.1): a row the
        // guard holds still reads it, against the lock its reply was read
        // under.
        weapon.band = row.band;
        weapon.percent = row.percent;
        weapon.chance_ms = read_ms;
        if stale_ready {
            return;
        }
        weapon.shots_since_read = 0;
        weapon.short = row.short.clone();
        weapon.ready = row.ready;
        weapon.cycling_s = row.cycling_s;
        weapon.rounds = row.rounds;
        weapon.rounds_max = row.rounds_max;
        weapon.read_ms = read_ms;
        // The one clock (cut AK.1; cut AL.1): an accepted `ready` row is the
        // evidence, ready from its own time (so a machine gun whose row lands
        // inside the guard, its rounds showing the shot has left, is ready
        // now, not at the next row); a `cycling (N s)` row's seconds are
        // rounded up (fact (a)), so the gun is ready after `N − 1` whole
        // seconds and by `N` and the slack: the row bounds the ack's clock
        // both ways, and with no clock it is the later bound; any other row
        // leaves nothing to count but the row.
        match row.cycling_s {
            // Cut AO.2: a row read inside the guard is the cockpit's frame,
            // not the arena's acceptance, so a lattice-clocked gun keeps its
            // later clock; AL's clock (the gun off the lattice) is the row's
            // time, as before.
            None if row.ready && weapon.on_lattice => {
                weapon.next_ready_ms =
                    Some(weapon.next_ready_ms.map_or(read_ms, |at| at.max(read_ms)));
            }
            None if row.ready => weapon.next_ready_ms = Some(read_ms),
            None => weapon.next_ready_ms = None,
            Some(secs) => {
                let left_ms = u64::from(secs) * 1_000;
                let lo = read_ms.saturating_add(left_ms.saturating_sub(1_000));
                // Cut AN.3: a lattice-clocked gun's upper bound carries at
                // least the guard, so a row never pulls that clock early.
                let pad = if weapon.on_lattice {
                    weapon.slack_ms.max(LATTICE_GUARD_MS)
                } else {
                    weapon.slack_ms
                };
                let hi = read_ms.saturating_add(left_ms).saturating_add(pad);
                weapon.next_ready_ms = Some(weapon.next_ready_ms.unwrap_or(hi).max(lo).min(hi));
                // The cycle (fact (b)): only a row read inside the first
                // second after this gun's shot prints it whole — counted from
                // the send of its `FIRE` (cut AM.1), which bounds the row's
                // age at print whatever each leg of the wire took; from the
                // ack only when no send is known.
                let fired_sent_ms = weapon.fired_sent_ms;
                if left_ms > 0
                    && weapon.fired_ms.is_some_and(|fired| {
                        read_ms.saturating_sub(fired_sent_ms.unwrap_or(fired))
                            < CYCLE_READ_WITHIN_MS
                    })
                {
                    weapon.cycle_ms =
                        Some(weapon.cycle_ms.map_or(left_ms, |held| held.max(left_ms)));
                }
            }
        }
    }

    /// A `RANGE` answer: the contact's figure, and — for the lock read just
    /// after its row — a finer fix in that row's place.
    fn range_reply(&mut self, now_ms: u64, designator: &str, range_m: u32, plotted_s: Option<u32>) {
        let locked = self.lock.as_deref() == Some(designator);
        let origin_status = self.own.pos_read_ms;
        let Some(contact) = self.contacts.get(designator) else {
            return;
        };
        let refine = locked
            && now_ms.saturating_sub(contact.read_ms) <= LOCK_FIX_MS
            && contact
                .fixes
                .back()
                .is_some_and(|fix| !fix.from_lock && fix.at_ms == contact.row_plot_ms());
        let row_read = contact.read_ms;
        let bearing = f64::from(contact.bearing_deg).to_radians();
        let origin = self.own_pos_at(row_read);
        let Some(contact) = self.contacts.get_mut(designator) else {
            return;
        };
        if refine && let Some(origin) = origin {
            let range = f64::from(range_m);
            let reckoned_s = row_read.saturating_sub(origin_status) as f64 / 1_000.0;
            // As fine as the plot is fresh (`help cockpit`, "Seeing"): "a fresh
            // one reads to 10 m, a one-second plot to 20, and a plot a couple
            // of seconds old reads no better than the rest of the scope".
            let base = match plotted_s {
                None | Some(0) => 10.0,
                Some(1) => 20.0,
                Some(_) => 50.0,
            };
            let at_ms = contact.row_plot_ms();
            contact.fixes.pop_back();
            contact.push_fix(Fix {
                at_ms,
                pos_m: (
                    origin.0 + range * bearing.sin(),
                    origin.1 + range * bearing.cos(),
                ),
                sigma_m: base + range * 0.5_f64.to_radians() + 3.0 * reckoned_s,
                from_lock: true,
            });
            contact.track = fit_track(&contact.fixes, now_ms);
        }
        self.touch();
    }

    /// `ORDER[ weapon] refused. said` — paired with an outstanding `Firing` ack
    /// of that weapon, it is the arena's refusal of a shot that never left;
    /// unpaired, it is the cockpit's pre-check or another order refused.
    fn refused(
        &mut self,
        now_ms: u64,
        order: &str,
        weapon: Option<&str>,
        said: &str,
        cycling_s: Option<u32>,
    ) {
        let mut paired = false;
        let mut bench_note = None;
        // A `FIRE` of this weapon the runner wrote and has not yet seen
        // answered: this refusal is that answer, the cockpit's pre-check
        // ([`Picture::sent_fire`]).
        let answers_a_send = order == "FIRE" && self.take_fire_mark(now_ms, weapon).is_some();
        let carried = self
            .weapons
            .iter()
            .position(|w| weapon.is_some_and(|id| w.id == id));
        if order == "FIRE"
            && let Some(index) = carried
        {
            let entry = &mut self.weapons[index];
            while entry
                .acks
                .front()
                .is_some_and(|at| now_ms.saturating_sub(*at) > ACK_PAIR_MS)
            {
                entry.acks.pop_front();
            }
            if !answers_a_send && let Some(at) = entry.acks.pop_front() {
                paired = true;
                entry.shots_since_read = entry.shots_since_read.saturating_sub(1);
                if entry.fired_ms == Some(at) {
                    entry.fired_ms = None;
                    entry.fired_sent_ms = None;
                }
            }
            if let Some(secs) = cycling_s {
                // The one clock (cut AK.1; cut AL.1). The cockpit's own
                // pre-check of a `FIRE` this runner typed means the clock was
                // early: the slack grows. `N s to go` is more than `N − 1`
                // (fact (a)), so whoever answered, the gun is tried again a
                // lattice step past those whole seconds (fact (c)), inside the
                // second it was refused for.
                if answers_a_send {
                    entry.slack_ms = entry
                        .slack_ms
                        .saturating_add(FIRE_SLACK_UP_MS)
                        .min(FIRE_SLACK_MAX_MS);
                    // Cut AN.3: a lattice-clocked gun refused twice inside
                    // the bench's span goes back to AL's clock for as long.
                    if entry.on_lattice {
                        entry.lattice_misses.push_back(now_ms);
                        while entry
                            .lattice_misses
                            .front()
                            .is_some_and(|at| now_ms.saturating_sub(*at) > LATTICE_BENCH_MS)
                        {
                            entry.lattice_misses.pop_front();
                        }
                        if entry.lattice_misses.len() >= LATTICE_MISSES_OFF {
                            entry.lattice_misses.clear();
                            entry.lattice_bench_until_ms =
                                Some(now_ms.saturating_add(LATTICE_BENCH_MS));
                            bench_note = Some(format!(
                                "The {} is off the lattice clock for {} s: refused twice.",
                                entry.id,
                                LATTICE_BENCH_MS / 1_000
                            ));
                        }
                    }
                }
                entry.ready = false;
                entry.next_ready_ms = Some(
                    now_ms
                        .saturating_add(u64::from(secs).saturating_sub(1) * 1_000)
                        .saturating_add(FIRE_RETRY_MS),
                );
            } else if said.starts_with("Empty:") {
                // `Empty: that weapon has nothing left to fire.` (cut AL.1):
                // struck from the fire schedule until a row counts rounds in.
                let news = entry.rounds_left() > 0;
                entry.rounds = 0;
                entry.shots_since_read = 0;
                entry.ready = false;
                if paired {
                    entry.next_ready_ms = Some(now_ms);
                }
                if news {
                    let id = entry.id.clone();
                    self.notes
                        .push(format!("The {id} is empty: struck from the fire schedule."));
                }
            } else if paired {
                // The arena refused an acked shot for something other than its
                // cycle (`Too many orders at once`, `No lock: …`): the shot
                // never left and the cockpit's pre-check had passed it, so the
                // gun is ready now; the lane's own wait governs the retry.
                entry.next_ready_ms = Some(now_ms);
            }
        }
        if let Some(note) = bench_note {
            self.notes.push(note);
        }
        if paired {
            self.tally.refused_shots = self.tally.refused_shots.saturating_add(1);
        } else {
            self.tally.refused_orders = self.tally.refused_orders.saturating_add(1);
        }
        if said == "Your tank cannot move." {
            self.own.immobile = true;
        }
        // `STARTUP refused. Your tank is a wreck.` (cut AM.2): the tank is
        // destroyed, so nothing starts it again. Only `STARTUP`'s: a refused
        // `FIRE` saying so is left as it was.
        if order == "STARTUP" && said.starts_with("Your tank is a wreck") {
            self.own.destroyed = true;
        }
        if said.starts_with("No lock:") {
            self.clear_lock(now_ms);
        }
        if said == "Power not online: STARTUP first." {
            self.own.power = Power::Off;
        }
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::classify;

    /// Apply each line at its time.
    fn feed(picture: &mut Picture, lines: &[(u64, &str)]) -> Vec<Applied> {
        lines
            .iter()
            .map(|(at, line)| picture.apply(*at, &classify(line)))
            .collect()
    }

    fn row_line(designator: &str, class: &str, bearing: u32, range: u32, age: u32) -> String {
        format!("{designator:<4}{class}  bearing {bearing:03}° (rel 000°)  far  {range} m  {age} s")
    }

    #[test]
    fn a_contacts_reply_is_the_whole_list() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s"),
                (
                    1,
                    "X2  unknown  bearing 315° (rel 270°)  effective  250 m  0 s",
                ),
            ],
        );
        assert!(applied.iter().all(|a| !a.journal));
        assert!(p.contacts.is_empty(), "held open until the reply ends");
        p.settle(100);
        assert!(
            p.contacts.is_empty(),
            "150 ms after the last row, not before"
        );
        p.settle(151);
        assert_eq!(p.contacts.keys().collect::<Vec<_>>(), ["BA", "X2"]);
        assert_eq!(p.contacts["BA"].side, Some(Side::Blue));
        assert_eq!(p.contacts["BA"].range_m, Some(450));
        assert_eq!(p.contacts["X2"].side, None);
        // The next reply drops what it does not name; the next game line
        // closes it.
        feed(
            &mut p,
            &[
                (
                    1_000,
                    "X2  unknown  bearing 315° (rel 270°)  effective  250 m  1 s",
                ),
                (1_001, "Callsign RA."),
            ],
        );
        assert_eq!(p.contacts.keys().collect::<Vec<_>>(), ["X2"]);
        let applied = feed(&mut p, &[(2_000, "No contacts.")]);
        assert!(!applied[0].journal);
        assert!(p.contacts.is_empty());
    }

    #[test]
    fn a_weapons_reply_with_and_without_a_lock_and_a_bare_lock_notice() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "Locked on BA, 62% to hit."),
                (
                    1,
                    "cannon  ready  40 of 40 round(s)  band 1 of 2  62% to hit",
                ),
                (
                    2,
                    "machine_gun (mg)  ready  500 of 500 round(s)  21% to hit",
                ),
            ],
        );
        assert!(applied[1..].iter().all(|a| !a.journal));
        p.settle(200);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        assert_eq!(p.lock_percent, Some(62));
        assert_eq!(p.weapons.len(), 2);
        assert_eq!(p.weapons[1].short.as_deref(), Some("mg"));
        // The same lock again (a poll reply) is not news.
        let applied = feed(
            &mut p,
            &[
                (1_000, "Locked on BA, 58% to hit."),
                (
                    1_001,
                    "cannon  ready  40 of 40 round(s)  band 1 of 2  58% to hit",
                ),
            ],
        );
        assert!(!applied[0].journal);
        p.settle(1_200);
        assert_eq!(p.lock_percent, Some(58));
        // The cannon row begins the next reply (a gun the open one holds):
        // the first closes, locked, cannon only.
        feed(&mut p, &[(2_000, "cannon  ready  40 of 40 round(s)")]);
        p.settle(2_200);
        assert_eq!(p.lock.as_deref(), Some("BA"), "applied, not yet closed");
        assert_eq!(
            p.weapons.len(),
            1,
            "the machine gun is not carried any more"
        );
        // A bare lock line is the arena's notice: it sets the lock and is news.
        let applied = feed(
            &mut p,
            &[(3_000, "Locked on X2."), (3_050, "Power online.")],
        );
        assert!(applied[0].journal);
        assert_eq!(
            applied[0].event.as_ref().map(|e| e.kind),
            Some(EventKind::Locked)
        );
        assert_eq!(p.lock.as_deref(), Some("X2"));
        // `This unit carries no weapon.` empties the list.
        feed(&mut p, &[(4_000, "This unit carries no weapon.")]);
        assert!(p.weapons.is_empty());
    }

    #[test]
    fn a_rename_and_back_keep_the_fixes_and_the_lock() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  at 1000 m east, 975 m north; scope clear"),
                (10, "X2  unknown  bearing 090° (rel 090°)  far  450 m  0 s"),
                (20, "Locked on X2."),
            ],
        );
        assert_eq!(p.contacts["X2"].fixes.len(), 1);
        let applied = feed(&mut p, &[(500, "Contact X2 identified: BA, blue.")]);
        assert!(applied[0].journal);
        assert!(!p.contacts.contains_key("X2"));
        let ba = &p.contacts["BA"];
        assert_eq!(ba.was.as_deref(), Some("X2"));
        assert_eq!(ba.side, Some(Side::Blue));
        assert_eq!(ba.fixes.len(), 1);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        feed(&mut p, &[(900, "BA is unidentified again: X2.")]);
        let x2 = &p.contacts["X2"];
        assert_eq!(x2.side, None);
        assert_eq!(x2.was.as_deref(), Some("BA"));
        assert_eq!(x2.fixes.len(), 1);
        assert_eq!(p.lock.as_deref(), Some("X2"));
        // `lost` removes the row and the lock with it.
        feed(&mut p, &[(1_000, "Contact X2 lost.")]);
        assert!(p.contacts.is_empty());
        assert_eq!(p.lock, None);
    }

    #[test]
    fn the_status_block() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "Callsign RB."),
                (
                    0,
                    "Red Tank 2 [biped_mk1]: power online, -12 kph, heading 270°, HELD, DESTROYED",
                ),
                (0, "  at -40 m east, 975 m north; scope holds a hostile"),
                (0, "  nearest spot: The Ford (A), 120 m at 045°"),
                (0, "  heat 100 of 100, OVERHEATED"),
                (0, "  receiver degraded: 15 of 40 through cover, 60% reach"),
                (0, "  emcon on: the pod radiates"),
                (0, "  hull  intact  internals 60/60"),
                (0, "    front 400/400  left 250/250"),
                (0, "    right 240/250  rear 150/150"),
                (0, "  left_track  destroyed  internals 0/24"),
                (0, "    left 0/120"),
                (1, "No contacts."),
            ],
        );
        assert!(applied.iter().all(|a| !a.journal));
        let own = &p.own;
        assert_eq!(own.callsign.as_deref(), Some("RB"));
        assert_eq!(own.name.as_deref(), Some("Red Tank 2"));
        assert_eq!(own.chassis.as_deref(), Some("biped_mk1"));
        assert_eq!(own.power, Power::Online);
        assert_eq!(own.speed_kph, -12);
        assert_eq!(own.heading_deg, 270);
        assert_eq!(own.held, Some(HeldCause::Unstated));
        assert!(own.destroyed);
        assert_eq!(own.pos_m, Some((-40.0, 975.0)));
        assert_eq!(own.scope, Scope::Hostile);
        assert_eq!(own.heat, Some((100, 100)));
        assert!(own.overheated && own.jammed);
        assert_eq!(own.emcon, Some(true));
        assert_eq!(own.sections["hull"].internals, Some((60, 60)));
        assert_eq!(
            own.sections["hull"].facets.len(),
            4,
            "the wrapped run joins its section"
        );
        assert_eq!(own.sections["hull"].facets["right"], (240, 250));
        assert_eq!(own.sections["left_track"].condition, "destroyed");
        assert_eq!(p.spots[&'A'].range_m, Some(120));
        // 120 m at 045 from (-40, 975): where The Ford lies.
        assert_eq!(p.spots[&'A'].plotted_m, Some((45, 1060)));
        // A head without HELD clears it; one with it keeps a known cause.
        feed(
            &mut p,
            &[
                (5_000, "Held: something is in the way."),
                (6_000, "Callsign RB."),
                (6_000, "Red Tank 2: power online, 0 kph, heading 270°, HELD"),
                (6_001, "Callsign RB."),
            ],
        );
        assert_eq!(p.own.held, Some(HeldCause::Occupied));
        assert!(!p.own.destroyed);
        feed(
            &mut p,
            &[(6_002, "Red Tank 2: power online, 0 kph, heading 270°")],
        );
        assert_eq!(p.own.held, None);
        // A block that has closed replaced the sections: the second block
        // printed none, so the first block's stay.
        p.settle(7_000);
        assert!(p.own.sections.contains_key("hull"));
    }

    #[test]
    fn dead_reckoning_and_the_plot_fix() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (1_000, "Callsign RA."),
                (1_000, "Red Tank 1: power online, 36 kph, heading 090°"),
                (1_000, "  at 1000 m east, 975 m north; scope clear"),
            ],
        );
        let (x, y) = p.own_pos_at(3_000).unwrap();
        assert!(
            (x - 1_020.0).abs() < 1e-6 && (y - 975.0).abs() < 1e-6,
            "{x} {y}"
        );
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  at 1000 m east, 975 m north; scope clear"),
                (1_000, "BA  blue  bearing 090° (rel 090°)  far  450 m  0 s"),
                (1_001, "Callsign RA."),
            ],
        );
        let fix = p.contacts["BA"].fixes.back().unwrap().clone();
        assert!((fix.pos_m.0 - 1_450.0).abs() < 1e-6 && (fix.pos_m.1 - 975.0).abs() < 1e-6);
        assert_eq!(fix.at_ms, 500);
        assert!(!fix.from_lock);
        let expected = 50.0 + 450.0 * 0.5_f64.to_radians() + 3.0;
        assert!((fix.sigma_m - expected).abs() < 1e-9);
        assert!((bearing_deg((0.0, 0.0), (10.0, 0.0)) - 90.0).abs() < 1e-9);
        assert!((bearing_deg((0.0, 0.0), (0.0, -10.0)) - 180.0).abs() < 1e-9);
        assert!((bearing_deg((0.0, 0.0), (-10.0, 0.0)) - 270.0).abs() < 1e-9);
        assert!((distance_m((0.0, 0.0), (3.0, 4.0)) - 5.0).abs() < 1e-9);
    }

    /// Own tank still at the origin, reading STATUS, CONTACTS and a bare RANGE
    /// on the lock every second while the target runs 10 m/s east.
    fn lock_fixed_run(fixes: u64, spacing_ms: u64) -> Picture {
        let mut p = Picture::default();
        feed(&mut p, &[(0, "Locked on BA.")]);
        for i in 0..fixes {
            let t = 10_000 + i * spacing_ms;
            let target = (300.0 + 10.0 * (t as f64 / 1_000.0 - 10.0), 400.0);
            let bearing = bearing_deg((0.0, 0.0), target).round() as u32 % 360;
            let range = distance_m((0.0, 0.0), target);
            let scope = ((range / 50.0).round() * 50.0) as u32;
            let lock = ((range / 10.0).round() * 10.0) as u32;
            let row = row_line("BA", "blue", bearing, scope, 0);
            let range_line = format!("Range to BA: {lock} m.");
            feed(
                &mut p,
                &[
                    (t, "Callsign RA."),
                    (t, "Red Tank 1: power online, 0 kph, heading 000°"),
                    (t, "  at 0 m east, 0 m north; scope holds a hostile"),
                    (t + 100, &row),
                    (t + 300, &range_line),
                ],
            );
        }
        p
    }

    #[test]
    fn a_moving_target_read_through_the_lock_gets_a_track() {
        let p = lock_fixed_run(7, 1_000);
        let ba = &p.contacts["BA"];
        assert!(ba.fixes.iter().all(|f| f.from_lock));
        let track = ba.track.as_ref().expect("a track");
        let course_off = (track.course_deg - 90.0).abs();
        assert!(course_off <= 10.0, "course {}", track.course_deg);
        assert!(
            (track.speed_mps - 10.0).abs() <= 2.0,
            "speed {}",
            track.speed_mps
        );
        // Three fixes are not enough, and neither are fixes spanning 3 s.
        assert_eq!(lock_fixed_run(3, 2_000).contacts["BA"].track, None);
        assert_eq!(lock_fixed_run(4, 1_000).contacts["BA"].track, None);
    }

    /// The game's scope, read the way the pilot reads it: whole degrees, 50 m
    /// steps held against the last figure printed.
    fn scope_range(true_m: f64, held: Option<u32>) -> u32 {
        if let Some(held) = held
            && (true_m - f64::from(held)).abs() <= 25.0 + 6.25
        {
            return held;
        }
        ((true_m / 50.0).round() * 50.0).max(50.0) as u32
    }

    /// What one stopped-target run came to.
    struct StoppedRun {
        /// Fixes the contact holds at the end.
        fixes: usize,
        /// Rows whose printed range was a held figure, not the nearest step.
        held_reads: usize,
        /// Times the printed range stepped to a new figure.
        steps: usize,
        /// Raw fits that cleared the count, span and speed gates, so the
        /// displacement gate alone kept them from being published.
        decided_by_displacement: usize,
        /// The fastest raw fit seen, metres a second.
        fastest_mps: f64,
    }

    /// A stopped `BA` at the origin, read from an own tank orbiting the point
    /// `centre_east_m` east of it at `radius` and 15 m/s (a pilot circling a
    /// target it has lost track of), `STATUS` every 2 s, a `CONTACTS` row every
    /// second, whole-degree bearings and the 50 m held scope figure, for 20 s.
    /// Asserts at every row that the contact has no track.
    fn stopped_target_run(centre_east_m: f64, radius: f64) -> StoppedRun {
        let mut p = Picture::default();
        let target = (0.0_f64, 0.0_f64);
        let omega = 15.0 / radius;
        let mut held = None;
        let mut run = StoppedRun {
            fixes: 0,
            held_reads: 0,
            steps: 0,
            decided_by_displacement: 0,
            fastest_mps: 0.0,
        };
        for step in 0..=40_u64 {
            let t = step * 500;
            let angle = omega * t as f64 / 1_000.0;
            let own = (centre_east_m + radius * angle.sin(), -radius * angle.cos());
            let heading = (angle.to_degrees() + 90.0).rem_euclid(360.0).round() as u32 % 360;
            if step % 4 == 0 {
                let lines = [
                    "Callsign RA.".to_owned(),
                    format!("Red Tank 1: power online, 54 kph, heading {heading:03}°"),
                    format!(
                        "  at {} m east, {} m north; scope holds a hostile",
                        own.0.trunc() as i64,
                        own.1.trunc() as i64
                    ),
                ];
                for line in &lines {
                    p.apply(t, &classify(line));
                }
            }
            if step % 2 == 0 {
                let true_m = distance_m(own, target);
                let range = scope_range(true_m, held);
                if range != ((true_m / 50.0).round() * 50.0).max(50.0) as u32 {
                    run.held_reads += 1;
                }
                if held.is_some_and(|was| was != range) {
                    run.steps += 1;
                }
                held = Some(range);
                let bearing = bearing_deg(own, target).round() as u32 % 360;
                p.apply(
                    t + 50,
                    &classify(&row_line("BA", "blue", bearing, range, 0)),
                );
                p.apply(t + 60, &classify("No such contact."));
                let ba = &p.contacts["BA"];
                assert_eq!(ba.track, None, "at {t} ms, centre {centre_east_m} m east");
                if let Some(raw) = fit(&ba.fixes, t + 60) {
                    run.fastest_mps = run.fastest_mps.max(raw.speed_mps);
                    if raw.fixes >= TRACK_MIN_FIXES
                        && raw.span_ms >= TRACK_SPAN_MS
                        && raw.speed_mps >= TRACK_MIN_MPS
                    {
                        run.decided_by_displacement += 1;
                    }
                }
            }
        }
        run.fixes = p.contacts["BA"].fixes.len();
        run
    }

    /// **The displacement gate is what keeps a stopped machine still** (review
    /// AJ-F8/AJ-R7). With the orbit off the target, the printed range swings
    /// across 50 m steps and holds between them, and the raw fit reads that as
    /// several metres a second — past the count, span and speed gates — yet no
    /// track is ever published.
    #[test]
    fn a_stopped_target_read_through_the_rounded_scope_never_gets_a_track() {
        let (mut held_reads, mut steps, mut decided) = (0, 0, 0);
        for centre_east_m in [30.0, 60.0, 90.0, 200.0] {
            let run = stopped_target_run(centre_east_m, 420.0);
            assert!(run.fixes >= TRACK_MIN_FIXES, "{centre_east_m}");
            held_reads += run.held_reads;
            steps += run.steps;
            decided += run.decided_by_displacement;
            assert!(
                run.fastest_mps > TRACK_MIN_MPS,
                "centre {centre_east_m} m east: the fastest raw fit was {} m/s",
                run.fastest_mps
            );
        }
        assert!(held_reads > 0, "the scope's hold never fired");
        assert!(steps > 0, "the printed range never stepped");
        assert!(
            decided > 0,
            "no raw fit reached the displacement gate: the run does not test it"
        );
        // Centred on the target, the true range never changes: the case the
        // gate is not needed for, kept as the plain control.
        let still = stopped_target_run(0.0, 420.0);
        assert_eq!(still.steps, 0);
        assert!(still.fixes >= TRACK_MIN_FIXES);
    }

    /// Fixes on a straight line: `n` of them `spacing_ms` apart, moving `mps`
    /// east, each with `sigma_m`, the last one at 20 s.
    fn line_of_fixes(n: u64, spacing_ms: u64, mps: f64, sigma_m: f64) -> VecDeque<Fix> {
        let end = 20_000;
        (0..n)
            .map(|i| {
                let at_ms = end - (n - 1 - i) * spacing_ms;
                Fix {
                    at_ms,
                    pos_m: (mps * at_ms as f64 / 1_000.0, 500.0),
                    sigma_m,
                    from_lock: true,
                }
            })
            .collect()
    }

    /// **Each gate of the track decides on its own** (review AJ-F8/AJ-R7): every
    /// refused case clears the other three gates, and its twin one step across
    /// the refusing gate is published.
    #[test]
    fn each_clause_of_the_track_gate_decides_on_its_own() {
        let now = 20_000;
        let published = |fixes: &VecDeque<Fix>| fit_track(fixes, now).is_some();
        // Count: three fixes over 4 s at 25 m/s (100 m against 3 × 10 m) are
        // refused; five over the same 4 s are published.
        let three = line_of_fixes(3, 2_000, 25.0, 10.0);
        let raw = fit(&three, now).unwrap();
        assert!(raw.span_ms >= TRACK_SPAN_MS && raw.speed_mps >= TRACK_MIN_MPS);
        assert!(raw.speed_mps * 4.0 >= 3.0 * raw.mean_sigma);
        assert!(!published(&three));
        assert!(published(&line_of_fixes(5, 1_000, 25.0, 10.0)));
        // Span: four fixes over 3 s at 25 m/s (75 m against 30 m) are refused;
        // over 4 s they are published.
        let short = line_of_fixes(4, 1_000, 25.0, 10.0);
        let raw = fit(&short, now).unwrap();
        assert!(raw.fixes >= TRACK_MIN_FIXES && raw.speed_mps >= TRACK_MIN_MPS);
        assert!(raw.speed_mps * 3.0 >= 3.0 * raw.mean_sigma);
        assert!(!published(&short));
        assert!(published(&line_of_fixes(5, 1_000, 25.0, 10.0)));
        // Displacement: 10 m/s over 6 s is 60 m, short of 3 × 55 m row fixes;
        // the same motion read through 15 m fixes (45 m) is published.
        let coarse = line_of_fixes(7, 1_000, 10.0, 55.0);
        let raw = fit(&coarse, now).unwrap();
        assert!(raw.fixes >= TRACK_MIN_FIXES && raw.span_ms >= TRACK_SPAN_MS);
        assert!(raw.speed_mps >= TRACK_MIN_MPS);
        assert!(!published(&coarse));
        assert!(published(&line_of_fixes(7, 1_000, 10.0, 15.0)));
        // Speed: 2 m/s over 6 s is 12 m, past 3 × 1 m, but under the floor;
        // 4 m/s is over it. (The picture's own fixes never read finer than
        // 10 m, which makes the displacement gate imply the floor; fixes this
        // fine are built by hand to show the floor refuses on its own.)
        let slow = line_of_fixes(7, 1_000, 2.0, 1.0);
        let raw = fit(&slow, now).unwrap();
        assert!(raw.fixes >= TRACK_MIN_FIXES && raw.span_ms >= TRACK_SPAN_MS);
        assert!(raw.speed_mps * 6.0 >= 3.0 * raw.mean_sigma);
        assert!(!published(&slow));
        assert!(published(&line_of_fixes(7, 1_000, 4.0, 1.0)));
        // The window: the same fixes read 11 s later have left it.
        assert_eq!(
            fit_track(&line_of_fixes(7, 1_000, 25.0, 10.0), 31_000),
            None
        );
    }

    #[test]
    fn the_tally_pairs_refusals_with_acks() {
        let mut p = Picture::default();
        // The cockpit's pre-check answers instead of an ack.
        feed(
            &mut p,
            &[(0, "FIRE cannon refused. No lock: LOCK a contact first.")],
        );
        assert_eq!(
            (p.tally.shots, p.tally.refused_orders, p.tally.refused_shots),
            (0, 1, 0)
        );
        // An ack, then the arena's refusal inside two seconds: nothing left.
        let applied = feed(
            &mut p,
            &[
                (1_000, "Firing cannon."),
                (1_500, "FIRE cannon refused. Still cycling: 4 s to go."),
            ],
        );
        assert!(!applied[0].journal && applied[1].journal);
        assert_eq!(p.tally.shots, 1);
        assert_eq!(p.tally.refused_shots, 1);
        assert_eq!(p.tally.fired(), 0);
        let cannon = p.weapons.iter().find(|w| w.id == "cannon").unwrap();
        // The ack lowered the slack; the arena's refusal of the acked shot
        // answers no mark of the runner's, so it does not raise it. `4 s` is
        // rounded up (more than three): tried again a lattice step past three
        // whole seconds (cut AL.1).
        let slack = after_ack(FIRE_SLACK_START_MS);
        assert_eq!(cannon.slack_ms, slack);
        assert_eq!(cannon.next_ready_ms, Some(1_500 + 3_000 + FIRE_RETRY_MS));
        // Two refusals with nothing to pair never underflow.
        feed(
            &mut p,
            &[
                (
                    9_000,
                    "FIRE cannon refused. Empty: that weapon has nothing left to fire.",
                ),
                (
                    9_001,
                    "FIRE cannon refused. Empty: that weapon has nothing left to fire.",
                ),
            ],
        );
        assert_eq!(p.tally.refused_orders, 3);
        assert_eq!(p.tally.fired(), 0);
        // An ack older than two seconds is a shot that left.
        feed(
            &mut p,
            &[
                (10_000, "Firing cannon."),
                (12_500, "FIRE cannon refused. Still cycling: 1 s to go."),
            ],
        );
        assert_eq!(p.tally.fired(), 1);
        // Hits, misses, kills, hits taken and a lost section.
        feed(
            &mut p,
            &[
                (13_000, "Hit on BA: turret."),
                (13_001, "Miss."),
                (13_002, "BA destroyed."),
                (
                    13_003,
                    "Hit on the hull's front facet: 280 of 400 armour left.",
                ),
                (
                    13_004,
                    "Hit on the engine, through the hull's rear facet: armour gone, 20 of 40 inside.",
                ),
                (
                    13_005,
                    "The left_track is destroyed: the tank is slowed; the tank cannot move.",
                ),
            ],
        );
        let t = &p.tally;
        assert_eq!(
            (t.hits_scored, t.misses, t.kills, t.hits_taken),
            (1, 1, 1, 2)
        );
        assert_eq!(t.sections_lost, ["left_track"]);
        assert_eq!(p.own.sections["hull"].facets["front"], (280, 400));
        assert_eq!(p.own.sections["hull"].facets["rear"], (0, 0));
        assert_eq!(p.own.sections["engine"].internals, Some((20, 40)));
        assert_eq!(p.own.sections["left_track"].condition, "destroyed");
        assert!(p.own.immobile);
    }

    /// **The facet split** (cut AL.2): a hit through a plate counts on that
    /// plate's facet, not the section behind it; a hit naming no facet counts
    /// only in `hits_taken`; a new match starts the split again.
    #[test]
    fn hits_taken_are_split_by_facet() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (
                    1_000,
                    "Hit on the hull's left facet: 200 of 250 armour left.",
                ),
                (
                    1_100,
                    "Hit on the hull's left facet: 150 of 250 armour left.",
                ),
                (
                    1_200,
                    "Hit on the engine, through the hull's rear facet: armour gone, 20 of 40 inside.",
                ),
                (
                    1_300,
                    "Hit on the right_track's right facet: 40 of 100 armour left.",
                ),
                (
                    1_400,
                    "Hit on the turret's front facet: 300 of 350 armour left.",
                ),
                (1_500, "Hit on the left_track, which is already destroyed."),
            ],
        );
        assert_eq!(p.tally.hits_taken, 6);
        let split: Vec<(&str, u32)> = p
            .tally
            .hits_taken_by_facet
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        assert_eq!(
            split,
            [
                ("hull left", 2),
                ("hull rear", 1),
                ("right_track right", 1),
                ("turret front", 1)
            ]
        );
        assert!(
            !p.tally.hits_taken_by_facet.contains_key("engine rear"),
            "the plate, not the section behind it"
        );
        assert_eq!(
            serde_json::to_value(&p.tally).expect("json")["hits_taken_by_facet"]["hull left"],
            2
        );
        assert!(p.own.last_hit.is_some());
        // A new match: the split, and the last facet hit with it, start again
        // (a hit carried over would turn the next match's tank at power-on).
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(3_000, "Match 9 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.following, Some(9));
        assert!(p.tally.hits_taken_by_facet.is_empty());
        assert_eq!(p.tally.hits_taken, 0);
        assert_eq!(p.own.last_hit, None);
    }

    /// **The last facet hit is kept** (cut AL.2) for the pilot's turn into the
    /// shot: its time, plate and facet; a hit with no facet leaves it alone.
    #[test]
    fn the_last_facet_hit_is_kept() {
        let mut p = Picture::default();
        assert_eq!(p.own.last_hit, None);
        feed(
            &mut p,
            &[(
                2_000,
                "Hit on the hull's left facet: 200 of 250 armour left.",
            )],
        );
        assert_eq!(
            p.own.last_hit,
            Some(FacetHit {
                at_ms: 2_000,
                plate: "hull".into(),
                facet: "left".into(),
            })
        );
        feed(
            &mut p,
            &[(
                2_500,
                "Hit on the engine, through the hull's rear facet: armour gone, 20 of 40 inside.",
            )],
        );
        let rear = p.own.last_hit.clone().expect("a facet hit");
        assert_eq!(
            (rear.at_ms, rear.plate.as_str(), rear.facet.as_str()),
            (2_500, "hull", "rear")
        );
        feed(
            &mut p,
            &[(3_000, "Hit on the left_track, which is already destroyed.")],
        );
        assert_eq!(p.own.last_hit, Some(rear), "no facet: kept");
    }

    #[test]
    fn cycles_are_learned_from_cycling_rows_only() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Firing cannon."),
                (800, "cannon  ready  40 of 40 round(s)"),
                (900, "Callsign RA."),
            ],
        );
        let cannon = &p.weapons[0];
        assert!(
            !cannon.ready,
            "a ready row 800 ms after the ack is the pre-shot frame"
        );
        assert_eq!(cannon.cycle_ms, None);
        assert_eq!(cannon.next_ready_ms, None, "and it sets no clock");
        const { assert!(950 < CYCLE_READ_WITHIN_MS, "inside the first second") };
        feed(
            &mut p,
            &[
                (950, "cannon  cycling (6 s)  39 of 40 round(s)"),
                (951, "Power online."),
            ],
        );
        let cannon = &p.weapons[0];
        assert_eq!(cannon.cycle_ms, Some(6_000));
        assert_eq!(cannon.rounds, 39);
        feed(
            &mut p,
            &[
                (7_000, "cannon  ready  39 of 40 round(s)"),
                (7_001, "Power online."),
            ],
        );
        let cannon = &p.weapons[0];
        assert!(cannon.ready);
        assert_eq!(
            cannon.next_ready_ms,
            Some(7_000),
            "a ready row: ready from its own time"
        );
        assert_eq!(cannon.cycle_ms, Some(6_000));
        // The next shot's ack sets the clock from the learned cycle; the
        // pre-shot frame read after it (rounds unchanged, inside the guard)
        // leaves that clock alone — the gun is not ready at that row.
        feed(&mut p, &[(8_000, "Firing cannon.")]);
        let due = p.weapons[0].next_ready_ms;
        assert_eq!(
            due,
            Some(8_000 + 6_000 + p.weapons[0].slack_ms),
            "the ack's clock"
        );
        feed(
            &mut p,
            &[
                (8_300, "cannon  ready  39 of 40 round(s)"),
                (8_301, "Power online."),
            ],
        );
        let cannon = &p.weapons[0];
        assert!(!cannon.ready);
        assert_eq!(
            cannon.next_ready_ms, due,
            "the pre-shot frame sets no clock"
        );
    }

    #[test]
    fn the_reclaim_sentences_flip_aboard() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[(
                0,
                "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
            )],
        );
        assert!(p.aboard);
        let applied = feed(
            &mut p,
            &[(1, "You are not at the station of a deployed unit.")],
        );
        assert!(!applied[0].journal);
        assert!(!p.aboard);
        feed(&mut p, &[(2, "Back aboard RA after 2 m 14 s away.")]);
        assert!(p.aboard);
        assert_eq!(p.reclaims, 1);
        assert_eq!(p.away_ms, 134_000);
        feed(
            &mut p,
            &[(
                3,
                "The server restarted and the match resumed from its last save; what happened after that save did not count.",
            )],
        );
        assert!(!p.aboard);
        feed(&mut p, &[(4, "Power online.")]);
        assert!(p.aboard);
        // The reconnect line (`… is flying on doctrine; …` until the game's
        // 17b cut M5.3).
        feed(
            &mut p,
            &[(5, "Red Tank 1 is flying on its mind; BOARD takes it back.")],
        );
        assert!(!p.aboard);
        feed(&mut p, &[(6, "You are already in your pod.")]);
        assert!(p.aboard);
    }

    #[test]
    fn only_the_followed_match_moves_the_phase() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Connected as Pilot (account pilot, 1 connection)."),
                (1, "Welcome Hall #3"),
                (2, "Simulator Lobby #9"),
            ],
        );
        assert_eq!(p.phase, Phase::InCentre);
        assert_eq!(p.me.as_deref(), Some("Pilot"));
        // Before a claim, another match's lines move nothing.
        feed(
            &mut p,
            &[(3, "Match 7 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.phase, Phase::InCentre);
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(4, "Match 8 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.following, Some(8));
        assert_eq!(p.phase, Phase::Boarding { match_id: 8 });
        feed(
            &mut p,
            &[
                (5, "Match 9 is running. Good hunting."),
                (
                    6,
                    "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
                ),
            ],
        );
        assert_eq!(p.phase, Phase::Seated { match_id: 8 });
        feed(&mut p, &[(7, "Match 8 is running. Good hunting.")]);
        assert_eq!(p.phase, Phase::Running { match_id: 8 });
        feed(
            &mut p,
            &[(8, "Match 9 is over (red). Its units are checked in.")],
        );
        assert_eq!(p.phase, Phase::Running { match_id: 8 });
        feed(
            &mut p,
            &[
                (9, "The hatch of Red Pod 1 swings open."),
                (10, "Red Room #12"),
                (11, "Match 8 is over (blue). Its units are checked in."),
            ],
        );
        assert_eq!(
            p.phase,
            Phase::Over {
                match_id: 8,
                result: ResultWord::Blue
            }
        );
        assert!(!p.aboard);
        let kinds: Vec<EventKind> = p.events.iter().map(|e| e.kind).collect();
        assert!(
            kinds
                .iter()
                .filter(|k| **k == EventKind::PhaseChanged)
                .count()
                >= 5
        );
    }

    #[test]
    fn no_line_but_a_contact_row_or_notice_adds_a_contact() {
        let lines = [
            "Locked on BA, 62% to hit.",
            "cannon  ready  40 of 40 round(s)",
            "This unit carries no weapon.",
            "Callsign RA.",
            "Red Tank 1: power online, 27 kph, heading 090°",
            "  at 10 m east, 20 m north; scope holds a hostile",
            "  hull  intact  internals 60/60",
            "    front 400/400",
            "Range to BA: 440 m.",
            "Bearing to BA: 087° (rel 357°).",
            "Range to The Ford (A): 430 m.",
            "No lock: LOCK a contact first, or name one.",
            "No such contact.",
            "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.",
            "Startup sequence begun; power comes online shortly.",
            "Steering to 90°.",
            "Making 27 kph.",
            "Locking BA.",
            "Firing cannon.",
            "That unit carries no weapon called `cannon`.",
            "You are not at the station of a deployed unit.",
            "Back aboard RA after 2 m 14 s away.",
            // `… is flying on doctrine; …` until the game's 17b cut M5.3.
            "Red Tank 1 is flying on its mind; BOARD takes it back.",
            "Connected as Pilot (account pilot, 1 connection).",
            "Lock on BA broken: contact gone.",
            "Contact BA lost.",
            "Hit on BA: turret.",
            "Miss on BA: the target moved.",
            "BA destroyed.",
            "BA's turret is destroyed.",
            "Hit on the hull's front facet: 280 of 400 armour left.",
            "The left_track is destroyed: the tank is slowed.",
            "Power online.",
            "Held: something is in the way.",
            "Receiver degraded: something is jamming the sight.",
            "FIRE cannon refused. Still cycling: 4 s to go.",
            "Simulator centre: running (match 8)",
            "  red: 1 (RA): Pilot, pod Red Pod 1, seated",
            "  blue: 1 (BA): Ada",
            "Pilot takes red slot 1. `SIM START` begins the match.",
            "Match 8 is boarding. Every pilot: BOARD your pod.",
            "Match 8 is running. Good hunting.",
            "Match 8 is over (red). Its units are checked in.",
            "Red Room #12",
            "Hint: LOCK <contact> holds the contact your panel just named.",
            "The world is still starting; please try again shortly.",
            "Ada says, \"BA is at 087, X2 behind it.\"",
        ];
        assert!(lines.len() >= 40);
        let mut p = Picture::default();
        for (i, line) in lines.iter().enumerate() {
            let read = classify(line);
            assert!(
                !matches!(
                    read,
                    Line::ContactRow(_)
                        | Line::NewContact { .. }
                        | Line::Identified { .. }
                        | Line::Unidentified { .. }
                ),
                "{line}"
            );
            p.apply(i as u64 * 10, &read);
        }
        p.settle(100_000);
        assert!(
            p.contacts.is_empty(),
            "{:?}",
            p.contacts.keys().collect::<Vec<_>>()
        );
    }

    /// **A refusal answering the runner's own `FIRE` never un-counts a shot
    /// that left** (the machine gun cycles in a second, so a second `FIRE`
    /// inside two seconds of an ack is routine and its pre-check says the very
    /// sentence the arena's refusal says).
    #[test]
    fn a_refusal_that_answers_a_sent_fire_is_the_pre_check() {
        let mut p = Picture::default();
        p.sent_fire(0, "machine_gun");
        feed(&mut p, &[(20, "Firing machine_gun.")]);
        p.sent_fire(1_000, "machine_gun");
        feed(
            &mut p,
            &[(1_020, "FIRE machine_gun refused. Still cycling: 1 s to go.")],
        );
        let t = &p.tally;
        assert_eq!(
            (t.shots, t.refused_shots, t.refused_orders, t.fired()),
            (1, 0, 1, 1)
        );
        let mg = &p.weapons[0];
        assert_eq!(mg.fired_ms, Some(20), "the cycle anchor is kept");
        assert_eq!(mg.acks.len(), 1);
        // The pre-check says the clock was early: the slack (lowered by the
        // ack) grows, and `1 s to go` is tried again a lattice step on, inside
        // that second (cut AL.1).
        let slack = (after_ack(FIRE_SLACK_START_MS) + FIRE_SLACK_UP_MS).min(FIRE_SLACK_MAX_MS);
        assert_eq!(mg.slack_ms, slack);
        assert_eq!(mg.next_ready_ms, Some(1_020 + FIRE_RETRY_MS));
        // With no `FIRE` outstanding, the same sentence is the arena's refusal
        // of the acked shot, as before.
        feed(
            &mut p,
            &[(1_500, "FIRE machine_gun refused. Still cycling: 1 s to go.")],
        );
        assert_eq!((p.tally.refused_shots, p.tally.fired()), (1, 0));
        // A mark lapses after two seconds: a `FIRE` answered by something else
        // (a transient, then re-sent) does not turn a later arena refusal into
        // a pre-check.
        p.sent_fire(3_000, "machine_gun");
        feed(
            &mut p,
            &[(
                3_010,
                "The world is committing another action; please retry in a moment.",
            )],
        );
        p.sent_fire(3_050, "machine_gun");
        feed(
            &mut p,
            &[
                (3_100, "Firing machine_gun."),
                (
                    5_060,
                    "FIRE machine_gun refused. Empty: that weapon has nothing left to fire.",
                ),
            ],
        );
        assert_eq!(
            (p.tally.shots, p.tally.refused_shots, p.tally.refused_orders),
            (2, 2, 1),
            "the ack took one mark, the other lapsed at 5 050: the refusal is the arena's"
        );
        p.sent_fire(10_000, "machine_gun");
        p.sent_fire(10_010, "machine_gun");
        feed(
            &mut p,
            &[(
                12_500,
                "FIRE machine_gun refused. No lock: LOCK a contact first.",
            )],
        );
        assert!(p.open.fire_sent.is_empty(), "both marks lapsed");
        assert_eq!(p.tally.refused_orders, 2);
    }

    /// **A `ready` row read inside the guard counts when its rounds show the
    /// shot has left** (the machine gun's one-second cycle ends well inside the
    /// 1.5 s guard); the pre-shot frame, rounds unchanged, is still ignored.
    #[test]
    fn a_ready_row_whose_rounds_show_the_shot_left_is_read() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (100, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (101, "Power online."),
                (1_000, "Firing machine_gun."),
                (1_300, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (1_301, "Power online."),
            ],
        );
        let mg = &p.weapons[0];
        assert!(!mg.ready, "the pre-shot frame is ignored");
        assert_eq!((mg.rounds, mg.shots_since_read), (500, 1));
        assert_eq!(
            mg.next_ready_ms, None,
            "nor does it set the clock (the ack left none: no cycle known)"
        );
        feed(
            &mut p,
            &[
                (2_200, "machine_gun (mg)  ready  499 of 500 round(s)"),
                (2_201, "Power online."),
            ],
        );
        let mg = &p.weapons[0];
        assert!(
            mg.ready,
            "1.2 s after the ack, one round gone: the cycle is over"
        );
        assert_eq!(
            (mg.rounds, mg.shots_since_read, mg.read_ms),
            (499, 0, 2_200)
        );
        // Two acks since the last row: one round gone is not both.
        feed(
            &mut p,
            &[
                (3_000, "Firing machine_gun."),
                (3_100, "Firing machine_gun."),
                (3_900, "machine_gun (mg)  ready  498 of 500 round(s)"),
                (3_901, "Power online."),
            ],
        );
        assert!(!p.weapons[0].ready);
        assert_eq!(p.weapons[0].rounds, 499);
        feed(
            &mut p,
            &[
                (4_000, "machine_gun (mg)  ready  497 of 500 round(s)"),
                (4_001, "Power online."),
            ],
        );
        assert!(p.weapons[0].ready);
        assert_eq!(p.weapons[0].rounds, 497);
    }

    /// The slack an ack leaves behind a weapon whose slack was `slack`.
    /// (Worked out here, not by the picture's own rule, so the tests hold the
    /// rule to its names; the floor is nothing today, which the lint reads as
    /// a no-op.)
    #[allow(clippy::unnecessary_min_or_max)]
    fn after_ack(slack: u64) -> u64 {
        slack
            .saturating_sub(FIRE_SLACK_DOWN_MS)
            .max(FIRE_SLACK_MIN_MS)
    }

    /// **An ack sets the one clock from the learned cycle** (cut AK.1): the
    /// ack plus the cycle plus the slack the ack leaves; with no cycle learned
    /// yet there is no clock, and the row is read.
    #[test]
    fn an_ack_sets_the_clock_from_the_learned_cycle() {
        let mut p = Picture::default();
        feed(&mut p, &[(0, "Firing cannon.")]);
        let cannon = &p.weapons[0];
        assert_eq!(cannon.cycle_ms, None);
        assert_eq!(cannon.next_ready_ms, None, "no cycle learned: read the row");
        assert_eq!(cannon.slack_ms, after_ack(FIRE_SLACK_START_MS));
        feed(
            &mut p,
            &[
                (500, "cannon  cycling (6 s)  39 of 40 round(s)"),
                (501, "Power online."),
            ],
        );
        assert_eq!(p.weapons[0].cycle_ms, Some(6_000));
        // A row left the clock in the past; the next ack moves it on, whatever
        // that row said.
        feed(
            &mut p,
            &[
                (7_000, "cannon  ready  39 of 40 round(s)"),
                (7_001, "Power online."),
            ],
        );
        assert_eq!(p.weapons[0].next_ready_ms, Some(7_000));
        feed(&mut p, &[(10_000, "Firing cannon.")]);
        let cannon = &p.weapons[0];
        let slack = after_ack(after_ack(FIRE_SLACK_START_MS));
        assert_eq!(cannon.slack_ms, slack);
        assert_eq!(cannon.next_ready_ms, Some(10_000 + 6_000 + slack));
        assert!(!cannon.ready);
    }

    /// **The seconds of a `cycling` row and of a `Still cycling` refusal are
    /// rounded up** (the bot's own transcript: `6 s to go` a whole second into
    /// a 6 s cycle): a row with no clock before it is counted whole, plus the
    /// slack; a refusal means more than a second less, so the gun is tried
    /// again a lattice step past that (cut AL.1).
    #[test]
    fn a_cycling_row_and_a_refusal_count_the_rounded_up_seconds() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (2_000, "cannon  cycling (4 s)  39 of 40 round(s)"),
                (2_001, "Power online."),
            ],
        );
        let slack = FIRE_SLACK_START_MS;
        assert_eq!(p.weapons[0].slack_ms, slack);
        assert_eq!(p.weapons[0].next_ready_ms, Some(2_000 + 4_000 + slack));
        // A refusal no `FIRE` of the runner's is waiting on and no ack pairs
        // with: the retry clock, the slack unmoved.
        feed(
            &mut p,
            &[(5_000, "FIRE cannon refused. Still cycling: 3 s to go.")],
        );
        assert_eq!(p.weapons[0].slack_ms, slack);
        assert_eq!(
            p.weapons[0].next_ready_ms,
            Some(5_000 + 2_000 + FIRE_RETRY_MS)
        );
        assert!(!p.weapons[0].ready);
    }

    /// **The slack rises on the cockpit's own refusal and falls on an ack**,
    /// never past [`FIRE_SLACK_MAX_MS`] or below [`FIRE_SLACK_MIN_MS`].
    #[test]
    fn the_slack_rises_on_its_own_refusal_and_falls_on_an_ack() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "cannon  ready  40 of 40 round(s)"),
                (1, "Power online."),
            ],
        );
        let mut at = 1_000;
        let mut expected = FIRE_SLACK_START_MS;
        let rises = (FIRE_SLACK_MAX_MS - FIRE_SLACK_MIN_MS) / FIRE_SLACK_UP_MS + 2;
        for _ in 0..rises {
            p.sent_fire(at, "cannon");
            feed(
                &mut p,
                &[(at + 10, "FIRE cannon refused. Still cycling: 2 s to go.")],
            );
            expected = (expected + FIRE_SLACK_UP_MS).min(FIRE_SLACK_MAX_MS);
            assert_eq!(p.weapons[0].slack_ms, expected);
            assert_eq!(
                p.weapons[0].next_ready_ms,
                Some(at + 10 + 1_000 + FIRE_RETRY_MS)
            );
            at += 3_000;
        }
        assert_eq!(p.weapons[0].slack_ms, FIRE_SLACK_MAX_MS, "held at the top");
        let falls = (FIRE_SLACK_MAX_MS - FIRE_SLACK_MIN_MS) / FIRE_SLACK_DOWN_MS + 2;
        for _ in 0..falls {
            feed(&mut p, &[(at, "Firing cannon.")]);
            expected = after_ack(expected);
            assert_eq!(p.weapons[0].slack_ms, expected);
            at += 3_000;
        }
        assert_eq!(
            p.weapons[0].slack_ms, FIRE_SLACK_MIN_MS,
            "held at the floor"
        );
    }

    /// **`FIRE` marks are kept per weapon** (cut AK.1): a cannon refusal read
    /// while only the machine gun's `FIRE` is outstanding is not the cannon's
    /// pre-check but the arena's refusal of the cannon's acked shot.
    #[test]
    fn fire_marks_are_kept_per_weapon() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "cannon  ready  40 of 40 round(s)"),
                (1, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (2, "Power online."),
                (1_000, "Firing cannon."),
            ],
        );
        p.sent_fire(1_100, "machine_gun");
        feed(
            &mut p,
            &[(1_200, "FIRE cannon refused. Still cycling: 5 s to go.")],
        );
        let t = &p.tally;
        assert_eq!(
            (t.shots, t.refused_shots, t.refused_orders),
            (1, 1, 0),
            "paired with the cannon's ack"
        );
        assert_eq!(p.open.fire_sent.len(), 1, "the machine gun's mark stands");
        assert_eq!(
            p.weapons[0].slack_ms,
            after_ack(FIRE_SLACK_START_MS),
            "the arena's refusal does not raise the slack"
        );
        // The machine gun's own answer takes its mark, by its short name too.
        feed(&mut p, &[(1_250, "Firing mg.")]);
        assert!(p.open.fire_sent.is_empty());
    }

    /// **A shot the arena refused for something other than its cycle leaves
    /// the gun ready now** (it never left; the lane's own wait governs the
    /// retry).
    #[test]
    fn a_shot_the_arena_refused_leaves_the_gun_ready() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Firing cannon."),
                (500, "cannon  cycling (6 s)  39 of 40 round(s)"),
                (501, "Power online."),
                (10_000, "Firing cannon."),
            ],
        );
        assert_eq!(p.weapons[0].cycle_ms, Some(6_000));
        assert_eq!(
            p.weapons[0].next_ready_ms,
            Some(10_000 + 6_000 + after_ack(after_ack(FIRE_SLACK_START_MS))),
            "a whole cycle and the slack on"
        );
        feed(
            &mut p,
            &[(
                10_300,
                "FIRE cannon refused. Too many orders at once; try that again.",
            )],
        );
        assert_eq!(p.tally.refused_shots, 1, "paired with the ack");
        assert_eq!(p.weapons[0].next_ready_ms, Some(10_300));
    }

    /// **A `ready` row accepted inside the guard is ready now**: the machine
    /// gun's row whose rounds show the shot has left is the evidence, so the
    /// clock is the row's own time.
    #[test]
    fn a_ready_row_inside_the_guard_is_ready_now() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (100, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (101, "Power online."),
                (1_000, "Firing machine_gun."),
                (2_200, "machine_gun (mg)  ready  499 of 500 round(s)"),
                (2_201, "Power online."),
            ],
        );
        const { assert!(2_200 - 1_000 < READY_AFTER_ACK_MS, "inside the guard") };
        let mg = &p.weapons[0];
        assert!(mg.ready);
        assert_eq!(mg.next_ready_ms, Some(2_200));
    }

    /// **A row the stale-ready guard holds still reads its chance** (cut
    /// AL.1): its readiness, rounds and read time stay as they were (it may be
    /// the frame before the shot left), but its band and chance to hit are
    /// the target's, read now, under the lock line its reply opened with.
    #[test]
    fn a_held_row_still_reads_its_chance() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (100, "Locked on BA, 62% to hit."),
                (
                    101,
                    "machine_gun (mg)  ready  500 of 500 round(s)  band 1 of 3  62% to hit",
                ),
                (102, "Power online."),
                (1_000, "Firing machine_gun."),
                (1_200, "Locked on BA, 40% to hit."),
                (
                    1_201,
                    "machine_gun (mg)  ready  500 of 500 round(s)  band 2 of 3  40% to hit",
                ),
                (1_202, "Power online."),
            ],
        );
        const { assert!(1_201 - 1_000 < READY_AFTER_ACK_MS, "inside the guard") };
        let mg = &p.weapons[0];
        assert_eq!(
            (mg.read_ms, mg.rounds, mg.shots_since_read),
            (101, 500, 1),
            "the row held"
        );
        assert_eq!((mg.band, mg.percent), (Some((2, 3)), Some(40)));
        assert_eq!(mg.chance_ms, 1_201);
        assert!(mg.chance_ms >= p.lock_read_ms, "read under the lock line");
    }

    // -- cut AL.1: the cycle read inside the first second, the clock, a lost gun --

    /// A weapons reply of `rows` read at `at`, committed by the line after it.
    fn rows_at(p: &mut Picture, at: u64, rows: &[&str]) {
        for row in rows {
            p.apply(at, &classify(row));
        }
        p.apply(at + 1, &classify("Power online."));
    }

    fn weapon<'a>(p: &'a Picture, id: &str) -> &'a Weapon {
        p.weapons
            .iter()
            .find(|w| w.id == id)
            .unwrap_or_else(|| panic!("no {id}: {:?}", p.weapons))
    }

    /// **The cycle is the whole seconds of a row read inside the first second
    /// after the ack** (fact (b)); a row read later, a row with no ack before
    /// it, a `ready` row and a refusal set nothing.
    #[test]
    fn the_cycle_is_read_inside_the_first_second() {
        let mut p = Picture::default();
        // A cycling row with no ack before it: nothing learned.
        rows_at(
            &mut p,
            100,
            &[
                "cannon  cycling (3 s)  40 of 40 round(s)",
                "machine_gun (mg)  cycling (1 s)  500 of 500 round(s)",
            ],
        );
        assert_eq!(weapon(&p, "cannon").cycle_ms, None);
        assert_eq!(weapon(&p, "machine_gun").cycle_ms, None);
        // A ready row, and a refusal, after an ack: nothing learned either.
        feed(&mut p, &[(10_000, "Firing machine_gun.")]);
        rows_at(
            &mut p,
            10_000 + CYCLE_READ_WITHIN_MS - 1,
            &["machine_gun (mg)  ready  499 of 500 round(s)"],
        );
        feed(
            &mut p,
            &[(
                10_000 + 1_100,
                "FIRE machine_gun refused. Still cycling: 1 s to go.",
            )],
        );
        assert_eq!(weapon(&p, "machine_gun").cycle_ms, None);
        // With no cycle held yet, a `cycling (5 s)` row read at the end of the
        // first second after the cannon's ack, or past it, sets nothing (a row
        // read then can print a second less: the cannon's `5 s`).
        feed(&mut p, &[(12_000, "Firing cannon.")]);
        for late in [CYCLE_READ_WITHIN_MS, CYCLE_READ_WITHIN_MS + 50] {
            rows_at(
                &mut p,
                12_000 + late,
                &["cannon  cycling (5 s)  39 of 40 round(s)"],
            );
            assert_eq!(weapon(&p, "cannon").cycle_ms, None, "read {late} ms on");
        }
        // `cycling (1 s)` 500 ms after the machine gun's ack: one second.
        feed(&mut p, &[(20_000, "Firing machine_gun.")]);
        rows_at(
            &mut p,
            20_500,
            &["machine_gun (mg)  cycling (1 s)  498 of 500 round(s)"],
        );
        assert_eq!(weapon(&p, "machine_gun").cycle_ms, Some(1_000));
        // `cycling (6 s)` 900 ms after the cannon's ack: six.
        feed(&mut p, &[(30_000, "Firing cannon.")]);
        rows_at(
            &mut p,
            30_900,
            &["cannon  cycling (6 s)  39 of 40 round(s)"],
        );
        assert_eq!(weapon(&p, "cannon").cycle_ms, Some(6_000));
        // `cycling (5 s)` 1 050 ms after the next ack: past the first second,
        // so the cycle stays six.
        feed(&mut p, &[(40_000, "Firing cannon.")]);
        rows_at(
            &mut p,
            41_050,
            &["cannon  cycling (5 s)  38 of 40 round(s)"],
        );
        assert_eq!(weapon(&p, "cannon").cycle_ms, Some(6_000));
        // …and a row inside it that reads less never lowers it.
        feed(&mut p, &[(50_000, "Firing cannon.")]);
        rows_at(
            &mut p,
            50_400,
            &["cannon  cycling (5 s)  37 of 40 round(s)"],
        );
        assert_eq!(weapon(&p, "cannon").cycle_ms, Some(6_000));
    }

    /// **An ack sets the clock one cycle and the slack on** (cut AL.1), the
    /// slack lowered first; with no cycle, no clock.
    #[test]
    fn an_ack_sets_the_clock_one_cycle_and_the_slack_on() {
        let mut p = Picture::default();
        feed(&mut p, &[(1_000, "Firing machine_gun.")]);
        assert_eq!(weapon(&p, "machine_gun").next_ready_ms, None);
        rows_at(
            &mut p,
            1_000 + crate::arena::lane::WEAPONS_AFTER_SHOT_MS,
            &["machine_gun (mg)  cycling (1 s)  499 of 500 round(s)"],
        );
        let slack = weapon(&p, "machine_gun").slack_ms;
        assert_eq!(slack, after_ack(FIRE_SLACK_START_MS));
        feed(&mut p, &[(5_000, "Firing mg.")]);
        let mg = weapon(&p, "machine_gun");
        assert_eq!(mg.slack_ms, after_ack(slack));
        assert_eq!(mg.next_ready_ms, Some(5_000 + 1_000 + after_ack(slack)));
        assert_eq!(mg.fired_ms, Some(5_000));
    }

    /// **A `cycling (N s)` row bounds the clock both ways** (cut AL.1): an
    /// ack's clock inside `[r + (N − 1) s, r + N s + slack]` is kept, one
    /// outside is clamped to it, and with no clock the row gives the later
    /// bound.
    #[test]
    fn a_cycling_row_bounds_the_clock_both_ways() {
        // A 6 s cannon whose cycle is learned, fired at 10 000.
        let learned = || {
            let mut p = Picture::default();
            feed(&mut p, &[(0, "Firing cannon.")]);
            rows_at(&mut p, 500, &["cannon  cycling (6 s)  39 of 40 round(s)"]);
            feed(&mut p, &[(10_000, "Firing cannon.")]);
            p
        };
        let mut p = learned();
        let slack = weapon(&p, "cannon").slack_ms;
        let clock = 10_000 + 6_000 + slack;
        assert_eq!(weapon(&p, "cannon").next_ready_ms, Some(clock));
        // Read at 11 000, `6 s`: 16 000 ..= 17 000 + slack holds the clock.
        rows_at(
            &mut p,
            11_000,
            &["cannon  cycling (6 s)  38 of 40 round(s)"],
        );
        assert_eq!(weapon(&p, "cannon").next_ready_ms, Some(clock));
        // Read at 12 000, `3 s` (the cycle nearer its end than the ack said):
        // clamped down to 15 000 + slack.
        let mut p = learned();
        rows_at(
            &mut p,
            12_000,
            &["cannon  cycling (3 s)  38 of 40 round(s)"],
        );
        assert_eq!(
            weapon(&p, "cannon").next_ready_ms,
            Some(12_000 + 3_000 + slack)
        );
        // Read at 12 000, `6 s` (further from its end): clamped up to 17 000.
        let mut p = learned();
        rows_at(
            &mut p,
            12_000,
            &["cannon  cycling (6 s)  38 of 40 round(s)"],
        );
        assert_eq!(weapon(&p, "cannon").next_ready_ms, Some(12_000 + 5_000));
        // No clock (no cycle learned): the later bound.
        let mut p = Picture::default();
        rows_at(&mut p, 2_000, &["cannon  cycling (4 s)  39 of 40 round(s)"]);
        assert_eq!(
            weapon(&p, "cannon").next_ready_ms,
            Some(2_000 + 4_000 + FIRE_SLACK_START_MS)
        );
    }

    /// **A `Still cycling` is tried again inside its second** (cut AL.1): the
    /// cockpit's own pre-check of a `FIRE` this runner typed raises the slack
    /// (never past one pulse); an arena refusal paired with an ack, and one that
    /// answers no mark and pairs with no ack, set the same clock and leave the
    /// slack alone.
    #[test]
    fn a_still_cycling_refusal_is_retried_inside_its_second() {
        let mut p = Picture::default();
        rows_at(&mut p, 0, &["cannon  ready  40 of 40 round(s)"]);
        let mut slack = FIRE_SLACK_START_MS;
        for (at, secs) in [
            (1_000, 1u64),
            (2_000, 3),
            (3_000, 1),
            (4_000, 1),
            (5_000, 2),
        ] {
            p.sent_fire(at, "cannon");
            feed(
                &mut p,
                &[(
                    at + 10,
                    format!("FIRE cannon refused. Still cycling: {secs} s to go.").as_str(),
                )],
            );
            slack = (slack + FIRE_SLACK_UP_MS).min(FIRE_SLACK_MAX_MS);
            let cannon = weapon(&p, "cannon");
            assert_eq!(cannon.slack_ms, slack, "{secs} s at {at}");
            assert_eq!(
                cannon.next_ready_ms,
                Some(at + 10 + (secs - 1) * 1_000 + FIRE_RETRY_MS),
                "{secs} s at {at}"
            );
        }
        assert_eq!(slack, FIRE_SLACK_MAX_MS, "capped at one pulse");
        // Paired with an ack (the arena's refusal of a shot that never left).
        feed(&mut p, &[(10_000, "Firing cannon.")]);
        let slack = weapon(&p, "cannon").slack_ms;
        feed(
            &mut p,
            &[(10_400, "FIRE cannon refused. Still cycling: 3 s to go.")],
        );
        assert_eq!(p.tally.refused_shots, 1, "paired");
        let cannon = weapon(&p, "cannon");
        assert_eq!(cannon.slack_ms, slack);
        assert_eq!(cannon.next_ready_ms, Some(10_400 + 2_000 + FIRE_RETRY_MS));
        // Answering no mark, pairing with no ack.
        feed(
            &mut p,
            &[(20_000, "FIRE cannon refused. Still cycling: 1 s to go.")],
        );
        let cannon = weapon(&p, "cannon");
        assert_eq!(cannon.slack_ms, slack);
        assert_eq!(cannon.next_ready_ms, Some(20_000 + FIRE_RETRY_MS));
    }

    /// **The slack falls to nothing and never passes one pulse** (cut AL.1).
    #[test]
    fn the_slack_falls_to_nothing_and_never_passes_one_pulse() {
        const { assert!(FIRE_SLACK_MAX_MS == ENGAGED_PULSE_MS) };
        let mut p = Picture::default();
        feed(&mut p, &[(0, "Firing cannon.")]);
        rows_at(&mut p, 500, &["cannon  cycling (6 s)  39 of 40 round(s)"]);
        let mut at = 10_000;
        let falls = FIRE_SLACK_START_MS / FIRE_SLACK_DOWN_MS + 2;
        for _ in 0..falls {
            feed(&mut p, &[(at, "Firing cannon.")]);
            at += 10_000;
        }
        let cannon = weapon(&p, "cannon");
        assert_eq!(cannon.slack_ms, FIRE_SLACK_MIN_MS);
        assert_eq!(cannon.next_ready_ms, Some(at - 10_000 + 6_000));
        let rises = FIRE_SLACK_MAX_MS / FIRE_SLACK_UP_MS + 2;
        for _ in 0..rises {
            p.sent_fire(at, "cannon");
            feed(
                &mut p,
                &[(at + 10, "FIRE cannon refused. Still cycling: 1 s to go.")],
            );
            assert!(weapon(&p, "cannon").slack_ms <= FIRE_SLACK_MAX_MS);
            at += 10_000;
        }
        assert_eq!(weapon(&p, "cannon").slack_ms, FIRE_SLACK_MAX_MS);
    }

    /// **A `FIRE` is pending until its own answer** (cut AL.1): a mark by the
    /// gun's id is pending by its short name too and the other way round; the
    /// ack, the cockpit's pre-check and the no-such-weapon answer each end it,
    /// another gun's mark is not it, and an unanswered one lapses after
    /// [`ACK_PAIR_MS`].
    #[test]
    fn a_fire_is_pending_until_its_own_answer() {
        let mut p = Picture::default();
        rows_at(
            &mut p,
            10,
            &[
                "cannon  ready  40 of 40 round(s)",
                "machine_gun (mg)  ready  500 of 500 round(s)",
            ],
        );
        assert!(!p.fire_pending("machine_gun"));
        // By its id, answered by an ack by its short name.
        p.sent_fire(1_000, "machine_gun");
        assert!(p.fire_pending("machine_gun") && p.fire_pending("mg"));
        assert!(!p.fire_pending("cannon"), "another gun's mark");
        feed(&mut p, &[(1_010, "Firing mg.")]);
        assert!(!p.fire_pending("machine_gun") && !p.fire_pending("mg"));
        // By its short name, answered by the cockpit's pre-check by its id.
        p.sent_fire(2_000, "mg");
        assert!(p.fire_pending("machine_gun"));
        feed(
            &mut p,
            &[(2_010, "FIRE machine_gun refused. Still cycling: 1 s to go.")],
        );
        assert!(!p.fire_pending("machine_gun"));
        // Answered as a gun not carried.
        p.sent_fire(3_000, "cannon");
        assert!(p.fire_pending("cannon"));
        feed(
            &mut p,
            &[(3_010, "That unit carries no weapon called `cannon`.")],
        );
        assert!(!p.fire_pending("cannon"));
        // Unanswered: pending through `ACK_PAIR_MS`, lapsed after it.
        p.sent_fire(4_000, "machine_gun");
        p.settle(4_000 + ACK_PAIR_MS);
        assert!(p.fire_pending("machine_gun"));
        p.settle(4_000 + ACK_PAIR_MS + 1);
        assert!(!p.fire_pending("machine_gun"));
    }

    /// **A gun the tank no longer carries is struck** (cut AL.1): its mark is
    /// taken, it leaves the list, a later row does not bring it back, the
    /// picture says so once, and a new match forgets it.
    #[test]
    fn a_gun_the_tank_no_longer_carries_is_struck() {
        let mut p = following_eight("running");
        rows_at(
            &mut p,
            10,
            &[
                "cannon  ready  40 of 40 round(s)",
                "machine_gun (mg)  ready  500 of 500 round(s)",
            ],
        );
        p.sent_fire(1_000, "cannon");
        let applied = feed(
            &mut p,
            &[(1_005, "That unit carries no weapon called `cannon`.")],
        );
        assert!(!applied[0].journal);
        assert!(p.open.fire_sent.is_empty(), "the mark is taken");
        assert!(p.struck.contains("cannon"));
        assert_eq!(
            p.weapons.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["machine_gun"]
        );
        assert_eq!(
            p.take_notes(),
            ["No cannon aboard any more: struck from the fire schedule."]
        );
        // Answered again (a `FIRE` already on its way): no second note.
        feed(
            &mut p,
            &[(1_050, "That unit carries no weapon called `cannon`.")],
        );
        assert!(p.take_notes().is_empty());
        // A row naming it again is not read.
        rows_at(
            &mut p,
            2_000,
            &[
                "cannon  ready  40 of 40 round(s)",
                "machine_gun (mg)  ready  500 of 500 round(s)",
            ],
        );
        assert_eq!(p.weapons.len(), 1);
        assert_eq!(p.weapons[0].id, "machine_gun");
        // A new match: the tank is whole again.
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(3_000, "Match 9 is boarding. Every pilot: BOARD your pod.")],
        );
        assert!(p.struck.is_empty());
        rows_at(&mut p, 4_000, &["cannon  ready  40 of 40 round(s)"]);
        assert_eq!(p.weapons[0].id, "cannon");
    }

    /// **`its weapons are gone` holds every gun until `WEAPONSTATUS`** (cut
    /// AL.1): the doubt keeps when it began, a committed reply clears it, and
    /// so does `This unit carries no weapon.`.
    #[test]
    fn its_weapons_are_gone_holds_every_gun_until_weaponstatus() {
        let mut p = following_eight("running");
        rows_at(&mut p, 10, &["cannon  ready  40 of 40 round(s)"]);
        feed(
            &mut p,
            &[(5_000, "The turret is destroyed: its weapons are gone.")],
        );
        assert_eq!(p.weapons_doubt, Some(5_000));
        assert_eq!(
            p.take_notes(),
            ["The turret's weapons are gone: no FIRE until WEAPONSTATUS says what is left."]
        );
        // Another section's loss neither moves nor re-says it.
        feed(
            &mut p,
            &[
                (5_500, "The left_track is destroyed: the tank is slowed."),
                (6_000, "The turret is destroyed: its weapons are gone."),
            ],
        );
        assert_eq!(p.weapons_doubt, Some(5_000));
        assert!(p.take_notes().is_empty());
        // A reply (here, a gun still carried) clears it.
        rows_at(
            &mut p,
            7_000,
            &["machine_gun (mg)  ready  500 of 500 round(s)"],
        );
        assert_eq!(p.weapons_doubt, None);
        assert_eq!(p.weapons.len(), 1);
        // So does `This unit carries no weapon.`.
        feed(
            &mut p,
            &[
                (8_000, "The turret is destroyed: its weapons are gone."),
                (8_500, "This unit carries no weapon."),
            ],
        );
        assert_eq!(p.weapons_doubt, None);
        assert!(p.weapons.is_empty());
        // And a new match.
        feed(
            &mut p,
            &[(9_000, "The turret is destroyed: its weapons are gone.")],
        );
        assert_eq!(p.weapons_doubt, Some(9_000));
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(9_500, "Match 9 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.weapons_doubt, None);
    }

    /// **An empty gun is struck** (cut AL.1): `Empty:` zeroes its rounds, so
    /// nothing is left to count down, and the picture says so once.
    #[test]
    fn an_empty_gun_is_struck() {
        let mut p = Picture::default();
        rows_at(&mut p, 0, &["machine_gun (mg)  ready  3 of 500 round(s)"]);
        feed(&mut p, &[(1_000, "Firing machine_gun.")]);
        assert_eq!(weapon(&p, "machine_gun").rounds_left(), 2);
        p.sent_fire(3_000, "machine_gun");
        feed(
            &mut p,
            &[(
                3_010,
                "FIRE machine_gun refused. Empty: that weapon has nothing left to fire.",
            )],
        );
        let mg = weapon(&p, "machine_gun");
        assert_eq!(
            (mg.rounds, mg.shots_since_read, mg.rounds_left()),
            (0, 0, 0)
        );
        assert_eq!(
            p.take_notes(),
            ["The machine_gun is empty: struck from the fire schedule."]
        );
        feed(
            &mut p,
            &[(
                4_000,
                "FIRE machine_gun refused. Empty: that weapon has nothing left to fire.",
            )],
        );
        assert!(p.take_notes().is_empty(), "said once");
        // A row that counts rounds in again puts it back.
        rows_at(
            &mut p,
            5_000,
            &["machine_gun (mg)  ready  200 of 500 round(s)"],
        );
        assert_eq!(weapon(&p, "machine_gun").rounds_left(), 200);
    }

    /// **Rounds come down by acks between rows** (cut AL.1), and a row sets
    /// them again.
    #[test]
    fn rounds_come_down_by_acks_between_rows() {
        let mut p = Picture::default();
        rows_at(&mut p, 0, &["cannon  ready  3 of 40 round(s)"]);
        assert_eq!(weapon(&p, "cannon").rounds_left(), 3);
        for (k, at) in [10_000, 20_000, 30_000].into_iter().enumerate() {
            feed(&mut p, &[(at, "Firing cannon.")]);
            assert_eq!(weapon(&p, "cannon").rounds_left(), 2 - k as u32);
        }
        feed(&mut p, &[(40_000, "Firing cannon.")]);
        assert_eq!(weapon(&p, "cannon").rounds_left(), 0, "never below none");
        // A paired arena refusal gives its round back.
        feed(
            &mut p,
            &[(
                40_500,
                "FIRE cannon refused. Too many orders at once; try that again.",
            )],
        );
        assert_eq!(weapon(&p, "cannon").shots_since_read, 3);
        rows_at(&mut p, 50_000, &["cannon  ready  0 of 40 round(s)"]);
        let cannon = weapon(&p, "cannon");
        assert_eq!((cannon.rounds, cannon.shots_since_read), (0, 0));
    }

    /// **A lock's fix is as fine as its plot is fresh** (`help cockpit`,
    /// "Seeing"): the refined fix's base is 10 m with no `plotted` clause, 20 m
    /// a second old, and the scope's 50 m older than that.
    #[test]
    fn a_lock_fix_is_as_fine_as_its_plot_is_fresh() {
        for (range_line, base) in [
            ("Range to BA: 440 m.", 10.0),
            ("Range to BA: 440 m, plotted 1 s ago.", 20.0),
            ("Range to BA: 440 m, plotted 4 s ago.", 50.0),
        ] {
            let mut p = Picture::default();
            feed(
                &mut p,
                &[
                    (0, "Locked on BA."),
                    (10_000, "Callsign RA."),
                    (10_000, "Red Tank 1: power online, 0 kph, heading 000°"),
                    (10_000, "  at 0 m east, 0 m north; scope holds a hostile"),
                    (10_100, &row_line("BA", "blue", 90, 450, 0)),
                    (10_300, range_line),
                ],
            );
            let fix = p.contacts["BA"].fixes.back().expect("a fix").clone();
            assert!(fix.from_lock, "{range_line}");
            let expected = base + 440.0 * 0.5_f64.to_radians() + 3.0 * 0.1;
            assert!(
                (fix.sigma_m - expected).abs() < 1e-9,
                "{range_line}: {} against {expected}",
                fix.sigma_m
            );
        }
    }

    /// **The `journal` flag is a contract** AJ.2's journal and AJ.3's commander
    /// read: every read reply and every ack is `false`, every notice, lobby
    /// line, room and stray line `true`.
    #[test]
    fn the_journal_flag_for_every_kind_of_line() {
        let quiet = [
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s",
            "No contacts.",
            "cannon  ready  40 of 40 round(s)",
            "This unit carries no weapon.",
            "Callsign RA.",
            "The panel has told you nothing yet.",
            "Red Tank 1: power off. The panel has told you nothing yet.",
            "No such contact.",
            "Range to BA: 440 m.",
            "Range to BA: 450 m, plotted 4 s ago.",
            "Bearing to BA: 087° (rel 357°).",
            "Range to The Ford (A): 430 m.",
            "Bearing to The Ford (A): 270° (rel 180°).",
            "No lock: LOCK a contact first.",
            "No lock: LOCK a contact first, or name one.",
            "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.",
            "Your tank is destroyed and takes no orders. EXIT leaves the pod.",
            "EMCON on: the pod radiates.",
            "Startup sequence begun; power comes online shortly.",
            "Power is already online.",
            "Power is still coming up; hold on.",
            "Power not online: STARTUP first.",
            "Your tank cannot move.",
            "Steering to 90°.",
            "Making 27 kph.",
            "Locking BA.",
            "EMCON coming on; the pod radiates from the next pulse.",
            "EMCON is already off.",
            "Firing cannon.",
            "That unit carries no weapon called `cannon`.",
            "You are not at the station of a deployed unit.",
            "The world is committing another action; please retry in a moment.",
            "The world is still starting; please try again shortly.",
            "That cockpit will not wake for you: control state unknown",
            "The arena is busy; try that again in a moment.",
        ];
        for line in quiet {
            let read = classify(line);
            assert!(
                !matches!(read, Line::Other(_)),
                "{line} must read as what it is"
            );
            let applied = Picture::default().apply(0, &read);
            assert!(!applied.journal, "{line} is journalled");
        }
        // A STATUS row inside its block.
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  at 10 m east, 20 m north; scope clear"),
                (0, "  nearest spot: The Ford (A), 120 m at 045°"),
                (0, "  heat 10 of 100"),
                (0, "  receiver degraded: 15 of 40 through cover, 60% reach"),
                (0, "  emcon off: the pod is silent"),
                (0, "  hull  intact  internals 60/60"),
                (0, "    front 400/400"),
            ],
        );
        assert!(applied.iter().all(|a| !a.journal));
        let loud = [
            "New contact X2: bearing 315°, effective.",
            "Contact X2 identified: BA, blue.",
            "BA is unidentified again: X2.",
            "Lock on BA broken: contact gone.",
            "Contact BA lost.",
            "Locked on BA.",
            "Hit on BA: turret.",
            "Miss.",
            "BA destroyed.",
            "BA's turret is destroyed.",
            "Hit on the hull's front facet: 280 of 400 armour left.",
            "The left_track is destroyed.",
            "Power online.",
            "Held: something is in the way.",
            "Receiver degraded: something is jamming the sight.",
            "FIRE cannon refused. Still cycling: 4 s to go.",
            "Back aboard RA after 2 m 14 s away.",
            "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
            "The hatch of Red Pod 1 swings open.",
            "Simulator centre: idle",
            "Pilot takes red slot 1. `SIM START` begins the match.",
            "Match 8 is boarding. Every pilot: BOARD your pod.",
            "Match 8 is running. Good hunting.",
            "Match 8 is over (red). Its units are checked in.",
            "The simulator centre is running; that is not something you can do to it now.",
            "Red Room #12",
            "Hint: STARTUP brings your power up.",
            "Connected as Pilot (account pilot, 1 connection).",
            "Ada says, \"hello\"",
        ];
        for line in loud {
            let applied = Picture::default().apply(0, &classify(line));
            assert!(applied.journal, "{line} is not journalled");
        }
        // A STATUS row with no block open is someone else's words.
        let applied = Picture::default().apply(0, &classify("    front 0/400"));
        assert!(applied.journal);
    }

    /// Claimed red 1 and following match 8, at `phase`.
    fn following_eight(phase: &str) -> Picture {
        let mut p = Picture::default();
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(1, "Match 8 is boarding. Every pilot: BOARD your pod.")],
        );
        if phase != "boarding" {
            feed(
                &mut p,
                &[(
                    2,
                    "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
                )],
            );
        }
        if phase == "running" {
            feed(&mut p, &[(3, "Match 8 is running. Good hunting.")]);
        }
        p
    }

    /// **Every way a followed match can end moves the phase**, and only for
    /// the followed match.
    #[test]
    fn a_followed_match_ends_every_way_it_can() {
        let ends: [(&str, ResultWord); 4] = [
            ("Match {n} is aborted: nobody boarded.", ResultWord::Abort),
            (
                "Match {n} could not start: nope. The pods are free again.",
                ResultWord::Abort,
            ),
            (
                "Match {n} ends: every pilot has left it.",
                ResultWord::Timeout,
            ),
            (
                "Match {n} is over (draw). Its units are checked in.",
                ResultWord::Draw,
            ),
        ];
        for start in ["boarding", "seated", "running"] {
            for (sentence, word) in &ends {
                let mut p = following_eight(start);
                let before = p.phase.clone();
                let (other, ours) = (sentence.replace("{n}", "9"), sentence.replace("{n}", "8"));
                feed(&mut p, &[(10, other.as_str())]);
                assert_eq!(p.phase, before, "{sentence} for match 9 from {start}");
                feed(&mut p, &[(11, ours.as_str())]);
                assert_eq!(
                    p.phase,
                    Phase::Over {
                        match_id: 8,
                        result: word.clone()
                    },
                    "{sentence} from {start}"
                );
                assert!(!p.aboard);
            }
            // `calls the match off` names no match: it ends the followed one
            // before it runs, and a running match only by its `is over` line.
            let mut p = following_eight(start);
            let before = p.phase.clone();
            feed(&mut p, &[(10, "Ada calls the match off.")]);
            if start == "running" {
                assert_eq!(p.phase, before);
            } else {
                assert_eq!(
                    p.phase,
                    Phase::Over {
                        match_id: 8,
                        result: ResultWord::Abort
                    },
                    "called off from {start}"
                );
            }
        }
    }

    /// **The tally is per match**: a new followed match starts from a clean
    /// tank, scope and tally, and follows the new id.
    #[test]
    fn a_new_followed_match_starts_clean() {
        let mut p = following_eight("running");
        feed(
            &mut p,
            &[
                (10, "Callsign RA."),
                (10, "Red Tank 1: power online, 0 kph, heading 000°"),
                (10, "  at 10 m east, 20 m north; scope clear"),
                (20, "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s"),
                (30, "Locked on BA."),
                (40, "Firing cannon."),
                (50, "Hit on BA: turret."),
                (60, "Hit on the hull's front facet: 280 of 400 armour left."),
                (70, "Match 8 is over (red). Its units are checked in."),
            ],
        );
        assert_eq!(p.tally.shots, 1);
        assert!(!p.contacts.is_empty() && p.lock.is_some());
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(100, "Match 9 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.following, Some(9));
        assert_eq!(p.phase, Phase::Boarding { match_id: 9 });
        assert_eq!(p.tally, Tally::default());
        assert!(p.contacts.is_empty() && p.weapons.is_empty() && p.spots.is_empty());
        assert_eq!(p.lock, None);
        assert_eq!(p.own.callsign, None);
        assert_eq!(p.own.pos_m, None);
        assert!(p.own.sections.is_empty());
    }

    /// **Rows out of their block change nothing** (review of AJ.1): a facet
    /// run is what a three-letter speaker's wrapped speech looks like, and a
    /// catch-up block's power row means nothing outside the block.
    #[test]
    fn indented_rows_count_only_inside_their_block() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  hull  intact  internals 60/60"),
                (0, "    front 400/400"),
                (1, "No contacts."),
            ],
        );
        let before = p.own.sections.clone();
        feed(
            &mut p,
            &[
                (10, "    front 0/400"),
                (11, "  hull  destroyed  internals 0/60"),
                (12, "  at 900 m east, 900 m north; scope holds a hostile"),
                (13, "Red Tank 1: power off, 0 kph, heading 000°, DESTROYED"),
            ],
        );
        assert_eq!(p.own.sections, before);
        assert_eq!(p.own.pos_m, None);
        assert!(!p.own.destroyed);
        assert_eq!(p.own.power, Power::Online);
        // The catch-up block's own row counts; the same row alone does not.
        feed(&mut p, &[(20, "  Power off.")]);
        assert_eq!(p.own.power, Power::Online);
        feed(
            &mut p,
            &[
                (30, "Back aboard RA after 45 s away."),
                (30, "  The machine had no state."),
                (30, "  Power starting."),
            ],
        );
        assert_eq!(p.own.power, Power::Starting);
        feed(&mut p, &[(40, "No contacts."), (41, "  Power online.")]);
        assert_eq!(p.own.power, Power::Starting);
    }

    #[test]
    fn power_ms_is_set_by_every_sentence_that_says_power_is_up() {
        let running = || Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            ..Picture::default()
        };
        // `Power online.` itself, the ordinary start.
        let mut p = running();
        feed(&mut p, &[(1_500, "Power online.")]);
        assert_eq!(p.power_ms, Some(1_500));
        // `Power is already online.` (a `STARTUP` into a powered tank).
        let mut p = running();
        feed(&mut p, &[(1_000, "Power is already online.")]);
        assert_eq!(p.power_ms, Some(1_000));
        // The first sentence counts; a later one does not move it.
        feed(&mut p, &[(2_000, "Power online.")]);
        assert_eq!(p.power_ms, Some(1_000));
        // The `Back aboard` block's own row, and only inside the block.
        let mut p = running();
        feed(&mut p, &[(1_000, "  Power online.")]);
        assert_eq!(p.power_ms, None, "the row alone is not the tank's");
        feed(
            &mut p,
            &[
                (2_000, "Back aboard RA after 45 s away."),
                (2_000, "  The machine had no state."),
                (2_010, "  Power online."),
            ],
        );
        assert_eq!(p.power_ms, Some(2_010));
        let mut p = running();
        feed(
            &mut p,
            &[
                (2_000, "Back aboard RA after 45 s away."),
                (2_010, "  Power starting."),
            ],
        );
        assert_eq!(p.power_ms, None, "starting is not up");
        // A running match's `STATUS` head reading `power online`.
        let mut p = running();
        feed(
            &mut p,
            &[
                (3_000, "Callsign RA."),
                (3_000, "Red Tank 1: power online, 30 kph, heading 090°"),
            ],
        );
        assert_eq!(p.power_ms, Some(3_000));
        // …and not before the match runs, nor with the power off.
        let mut seated = Picture {
            phase: Phase::Seated { match_id: 1 },
            ..running()
        };
        feed(
            &mut seated,
            &[
                (3_000, "Callsign RA."),
                (3_000, "Red Tank 1: power online, 30 kph, heading 090°"),
            ],
        );
        assert_eq!(seated.power_ms, None);
        let mut p = running();
        feed(
            &mut p,
            &[
                (3_000, "Callsign RA."),
                (3_000, "Red Tank 1: power off, 0 kph, heading 090°"),
            ],
        );
        assert_eq!(p.power_ms, None);
    }

    /// **The report's first times** (cut AN.1): each is set once, only while
    /// the followed match runs; a contact read as the own side, a teammate's
    /// identification and a friendly hit set nothing; a new match clears all
    /// three.
    #[test]
    fn the_first_times_are_set_once_in_a_running_match() {
        let mut p = Picture::default();
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[
                (100, "Match 9 is boarding. Every pilot: BOARD your pod."),
                // Before the match runs: nothing.
                (500, "New contact X1: bearing 000°, far."),
                (600, "Hit on the hull's left facet: 200 of 250 armour left."),
                (700, "Hit on X1: turret."),
            ],
        );
        assert_eq!(p.following, Some(9));
        assert_eq!(p.tally.first_contact_ms, None, "not running yet");
        assert_eq!(p.tally.first_hit_taken_ms, None);
        assert_eq!(p.tally.first_hit_scored_ms, None);
        feed(&mut p, &[(1_000, "Match 9 is running. Good hunting.")]);
        assert!(matches!(p.phase, Phase::Running { match_id: 9 }));
        // A row of the own side, a teammate identified as the own side, and a
        // notice of a designator held as the own side: nothing.
        feed(
            &mut p,
            &[(2_000, "RB  red  bearing 180° (rel 090°)  close  100 m  0 s")],
        );
        p.settle(2_200);
        assert!(p.contacts.contains_key("RB"));
        assert_eq!(p.tally.first_contact_ms, None, "the own side's row");
        feed(
            &mut p,
            &[
                (2_300, "Contact X5 identified: RC, red."),
                (2_400, "New contact RB: bearing 180°, close."),
            ],
        );
        assert_eq!(p.tally.first_contact_ms, None, "the own side, named");
        // A friendly hit scored is not the first hit scored.
        feed(&mut p, &[(2_500, "Hit on RB (FRIENDLY): turret.")]);
        assert_eq!(p.tally.hits_scored, 2, "the one before the run counted too");
        assert_eq!(p.tally.first_hit_scored_ms, None, "a friendly hit");
        // The first row of anything else — an unknown — sets it, at its
        // apply; a later notice does not move it.
        feed(
            &mut p,
            &[(
                3_000,
                "X2  unknown  bearing 090° (rel 000°)  far  450 m  0 s",
            )],
        );
        p.settle(3_200);
        assert_eq!(p.tally.first_contact_ms, Some(3_200));
        feed(
            &mut p,
            &[
                (4_000, "New contact X3: bearing 270°, far."),
                (4_100, "Contact X2 identified: BA, blue."),
            ],
        );
        assert_eq!(p.tally.first_contact_ms, Some(3_200), "set once");
        // The first hit taken and the first hit scored, each once.
        feed(
            &mut p,
            &[
                (
                    5_000,
                    "Hit on the hull's left facet: 200 of 250 armour left.",
                ),
                (6_000, "Hit on BA: turret."),
                (
                    7_000,
                    "Hit on the hull's front facet: 300 of 400 armour left.",
                ),
                (8_000, "Hit on BA: hull."),
            ],
        );
        assert_eq!(p.tally.first_hit_taken_ms, Some(5_000));
        assert_eq!(p.tally.first_hit_scored_ms, Some(6_000));
        // A notice is a first contact when nothing came before it.
        let mut q = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            ..Picture::default()
        };
        feed(&mut q, &[(1_500, "New contact X1: bearing 000°, far.")]);
        assert_eq!(q.tally.first_contact_ms, Some(1_500));
        // …and so is an identification of the other side.
        let mut q = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            ..Picture::default()
        };
        feed(&mut q, &[(1_700, "Contact X1 identified: BA, blue.")]);
        assert_eq!(q.tally.first_contact_ms, Some(1_700));
        // A new match clears all three.
        feed(
            &mut p,
            &[(9_000, "Match 9 is over (red). Its units are checked in.")],
        );
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(10_000, "Match 10 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.following, Some(10));
        assert_eq!(p.tally.first_contact_ms, None);
        assert_eq!(p.tally.first_hit_taken_ms, None);
        assert_eq!(p.tally.first_hit_scored_ms, None);
    }

    #[test]
    fn the_summary_fits_the_terminal_and_the_picture_serialises() {
        let mut p = lock_fixed_run(7, 1_000);
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[
                (20_000, "Firing cannon."),
                (20_001, "Range to The Crossroads (B): 850 m."),
            ],
        );
        let lines = p.summary_lines(20_500);
        assert!(
            lines
                .iter()
                .all(|line| line.chars().count() <= SUMMARY_COLUMNS)
        );
        assert!(
            lines.iter().any(|line| line.starts_with("contact BA")),
            "{lines:#?}"
        );
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"B\""));
    }

    // -- cut AM.1: a reply read whole on a slow wire ---------------------------

    /// The sorted designators the picture holds.
    fn held(p: &Picture) -> Vec<&str> {
        p.contacts.keys().map(String::as_str).collect()
    }

    /// The guns the picture holds, in its order.
    fn guns(p: &Picture) -> Vec<&str> {
        p.weapons.iter().map(|w| w.id.as_str()).collect()
    }

    /// **A pause never ends a reply** (cut AM.1): the rows read before a
    /// 400 ms gap apply at the settle, the row after it joins the same reply,
    /// nothing is dropped at any settle, and only the next game line closes
    /// it — dropping what it never named.
    #[test]
    fn a_reply_split_by_a_gap_keeps_its_contacts() {
        let mut p = Picture::default();
        // A contact held before the reply, and not in it.
        feed(
            &mut p,
            &[
                (0, &row_line("X9", "unknown", 200, 600, 0)),
                (1, "Steering to 90°."),
            ],
        );
        assert_eq!(held(&p), ["X9"]);
        feed(
            &mut p,
            &[
                (100, &row_line("BA", "blue", 90, 450, 0)),
                (101, &row_line("X2", "unknown", 300, 250, 0)),
            ],
        );
        assert_eq!(held(&p), ["X9"], "rows are not applied as they are read");
        p.settle(101 + SNAPSHOT_SETTLE_MS);
        assert_eq!(held(&p), ["BA", "X2", "X9"], "applied at the settle");
        p.settle(500);
        assert_eq!(held(&p), ["BA", "X2", "X9"], "a pause drops nothing");
        assert!(p.open.contacts.is_some(), "and the reply stays open");
        // The third row, 400 ms after the second: the same reply.
        let applied = feed(&mut p, &[(501, &row_line("RB", "red", 180, 500, 0))]);
        assert_eq!(applied[0].opened, None, "it begins nothing");
        p.settle(501 + SNAPSHOT_SETTLE_MS);
        assert_eq!(held(&p), ["BA", "RB", "X2", "X9"]);
        // The next game line closes it: exactly its three rows.
        feed(&mut p, &[(700, "Steering to 90°.")]);
        assert_eq!(held(&p), ["BA", "RB", "X2"]);
        assert_eq!(p.contacts_read, 2, "the first reply and this one");
    }

    /// **Chatter inside a reply is read as itself** (cut AM.1): a receiver
    /// notice and a channel line between two rows are journalled and applied
    /// in their own right, the rows around them are one reply, and nothing is
    /// dropped before the next game line — for `CONTACTS` and for
    /// `WEAPONSTATUS`, whose lock line still heads its rows.
    #[test]
    fn chatter_inside_a_reply_is_read_as_itself() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, &row_line("X9", "unknown", 200, 600, 0)),
                (1, "Steering to 90°."),
                (2, "Receiver degraded: something is jamming the sight."),
            ],
        );
        assert!(p.own.jammed);
        let applied = feed(
            &mut p,
            &[
                (100, &row_line("BA", "blue", 90, 450, 0)),
                (110, "Receiver clear."),
                (120, "[RedNet] Ada: on your left"),
                (130, &row_line("X2", "unknown", 300, 250, 0)),
            ],
        );
        assert!(applied[1].journal && applied[2].journal, "both journalled");
        assert!(!p.own.jammed, "the notice is applied");
        assert_eq!(applied[3].opened, None, "the second row is the same reply");
        assert_eq!(
            held(&p),
            ["BA", "X9"],
            "the first row applied at the chatter"
        );
        p.settle(130 + SNAPSHOT_SETTLE_MS);
        assert_eq!(
            held(&p),
            ["BA", "X2", "X9"],
            "nothing dropped before the end"
        );
        feed(&mut p, &[(400, "Steering to 90°.")]);
        assert_eq!(held(&p), ["BA", "X2"]);
        // The same for `WEAPONSTATUS`.
        let mut p = Picture::default();
        rows_at(&mut p, 0, &["rocket  ready  4 of 4 round(s)"]);
        assert_eq!(guns(&p), ["rocket"]);
        let applied = feed(
            &mut p,
            &[
                (100, "Locked on BA, 62% to hit."),
                (
                    101,
                    "cannon  ready  40 of 40 round(s)  band 1 of 2  62% to hit",
                ),
                (110, "Receiver clear."),
                (120, "[RedNet] Ada: on your left"),
                (
                    130,
                    "machine_gun (mg)  ready  500 of 500 round(s)  21% to hit",
                ),
            ],
        );
        assert!(applied[2].journal && applied[3].journal);
        assert_eq!(applied[1].opened, Some(AskKind::Weapons));
        assert_eq!(applied[4].opened, None);
        p.settle(130 + SNAPSHOT_SETTLE_MS);
        assert_eq!(guns(&p), ["rocket", "cannon", "machine_gun"]);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        feed(&mut p, &[(400, "Steering to 90°.")]);
        assert_eq!(guns(&p), ["cannon", "machine_gun"]);
        assert_eq!(
            p.lock.as_deref(),
            Some("BA"),
            "the lock line headed the rows"
        );
    }

    /// **A designator already read begins the next reply** (cut AM.1): two
    /// `CONTACTS` replies back to back with no line between are two, so the
    /// second's close drops what only the first named.
    #[test]
    fn a_second_reply_starts_at_a_designator_already_read() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, &row_line("BA", "blue", 90, 450, 0)),
                (1, &row_line("X2", "unknown", 300, 250, 0)),
                (2, &row_line("BA", "blue", 90, 450, 0)),
            ],
        );
        let opened: Vec<Option<AskKind>> = applied.iter().map(|a| a.opened).collect();
        assert_eq!(
            opened,
            [Some(AskKind::Contacts), None, Some(AskKind::Contacts)]
        );
        // The first reply closed at the second BA row: it named both.
        assert_eq!(held(&p), ["BA", "X2"]);
        assert_eq!(p.contacts_read, 1);
        feed(&mut p, &[(3, "Steering to 90°.")]);
        assert_eq!(held(&p), ["BA"], "the second reply named BA alone");
        assert_eq!(p.contacts_read, 2);
    }

    /// **Only the close drops, clears and replaces** (cut AM.1): at a settle a
    /// reply's rows are merged and every provisional thing stands; the next
    /// game line applies the whole-list rule — guns, the lock and the doubt;
    /// contacts; and a `STATUS` block's sections, which close at chatter too.
    #[test]
    fn only_the_close_drops_clears_and_replaces() {
        // Guns, the lock and the doubt.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Locked on BA."),
                (1, "cannon  ready  40 of 40 round(s)"),
                (2, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (3, "Steering to 90°."),
                (10, "The turret is destroyed: its weapons are gone."),
            ],
        );
        assert_eq!(p.lock.as_deref(), Some("BA"));
        assert_eq!(guns(&p).len(), 2);
        assert_eq!(p.weapons_doubt, Some(10));
        feed(&mut p, &[(1_000, "cannon  ready  40 of 40 round(s)")]);
        p.settle(1_000 + SNAPSHOT_SETTLE_MS);
        assert_eq!(p.lock.as_deref(), Some("BA"), "provisional: the lock");
        assert_eq!(
            guns(&p),
            ["cannon", "machine_gun"],
            "provisional: both guns"
        );
        assert_eq!(p.weapons_doubt, Some(10), "provisional: the doubt");
        feed(&mut p, &[(1_200, "Steering to 90°.")]);
        assert_eq!(p.lock, None, "no lock line headed the rows");
        assert_eq!(guns(&p), ["cannon"]);
        assert_eq!(p.weapons_doubt, None);
        // Contacts.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, &row_line("BA", "blue", 90, 450, 0)),
                (1, &row_line("X2", "unknown", 300, 250, 0)),
                (2, "Steering to 90°."),
                (1_000, &row_line("BA", "blue", 90, 450, 1)),
            ],
        );
        p.settle(1_000 + SNAPSHOT_SETTLE_MS);
        assert_eq!(held(&p), ["BA", "X2"], "provisional: both");
        feed(&mut p, &[(1_200, "Steering to 90°.")]);
        assert_eq!(held(&p), ["BA"]);
        // A `STATUS` block naming two of three sections.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  hull  intact  internals 60/60"),
                (0, "  turret  intact"),
                (0, "  left_track  intact  internals 24/24"),
                (1, "No contacts."),
                (1_000, "Callsign RA."),
                (1_000, "Red Tank 1: power online, 0 kph, heading 000°"),
                (1_000, "  hull  damaged  internals 50/60"),
                (1_000, "    front 300/400"),
                (1_000, "  turret  intact"),
            ],
        );
        p.settle(1_000 + SNAPSHOT_SETTLE_MS);
        let ids: Vec<&str> = p.own.sections.keys().map(String::as_str).collect();
        assert_eq!(ids, ["hull", "left_track", "turret"], "provisional: merged");
        assert_eq!(p.own.sections["hull"].condition, "damaged");
        assert_eq!(p.own.sections["hull"].facets["front"], (300, 400));
        // Chatter closes a `STATUS` block: no line is neutral for it.
        feed(&mut p, &[(1_300, "Ada pages: hi")]);
        let ids: Vec<&str> = p.own.sections.keys().map(String::as_str).collect();
        assert_eq!(ids, ["hull", "turret"], "replaced at the close");
    }

    /// **A notice read after a reply stands** (cut AM.1): the game prints a
    /// reply's lines back to back, so a notice after the rows came after them;
    /// the reply closes before it is applied, and never undoes it.
    #[test]
    fn in_order_notices_after_a_reply_stand() {
        // An identification after a reply naming the unknown it renames.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Locked on X1."),
                (10, &row_line("X1", "unknown", 90, 450, 0)),
            ],
        );
        p.settle(10 + SNAPSHOT_SETTLE_MS);
        feed(
            &mut p,
            &[
                (300, "Contact X1 identified: BA, blue."),
                (400, "Steering to 90°."),
            ],
        );
        assert_eq!(held(&p), ["BA"]);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        // `Locked on` after an unlocked `WEAPONSTATUS` reply.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Locked on BA."),
                (1, "cannon  ready  40 of 40 round(s)"),
                (2, "Steering to 90°."),
                (1_000, "cannon  ready  40 of 40 round(s)"),
            ],
        );
        p.settle(1_000 + SNAPSHOT_SETTLE_MS);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        feed(
            &mut p,
            &[(1_300, "Locked on X2."), (1_400, "Steering to 90°.")],
        );
        assert_eq!(p.lock.as_deref(), Some("X2"));
        // A section lost after a `STATUS` block that named it intact.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  hull  intact  internals 60/60"),
                (0, "  left_track  intact  internals 24/24"),
            ],
        );
        p.settle(SNAPSHOT_SETTLE_MS);
        feed(
            &mut p,
            &[
                (300, "The left_track is destroyed: the tank is slowed."),
                (400, "Steering to 90°."),
            ],
        );
        assert_eq!(p.own.sections["left_track"].condition, "destroyed");
        // A new contact after a reply that did not name it.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, &row_line("BA", "blue", 90, 450, 0)),
                (10, "New contact X3: bearing 090°, far."),
                (20, "Steering to 90°."),
            ],
        );
        assert_eq!(held(&p), ["BA", "X3"]);
    }

    /// **A page never writes `STATUS` rows** (cut AM.1): no line is neutral
    /// for `STATUS`, so a page closes the block, and the four-space run its
    /// wrapped tail would make is someone else's words.
    #[test]
    fn a_page_never_writes_status_rows() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  hull  intact  internals 60/60"),
                (0, "    front 400/400"),
            ],
        );
        let block = p.open.status.clone().expect("open").sections;
        feed(
            &mut p,
            &[
                (10, "Ada pages: a long page"),
                (11, "    front 20/20  left 20/20"),
            ],
        );
        assert_eq!(p.own.sections, block);
        assert_eq!(p.own.sections["hull"].facets.len(), 1);
    }

    /// **A lock line heads the first row after it across a pause and
    /// chatter** (cut AM.1), lapses [`STALE_TAIL_MS`] after it was read, and
    /// back-to-back locked replies each take their own.
    #[test]
    fn a_lock_line_split_from_its_rows_still_heads_them() {
        let mut p = Picture::default();
        feed(&mut p, &[(0, "Locked on BA, 62% to hit.")]);
        p.settle(200);
        p.settle(400);
        feed(
            &mut p,
            &[
                (400, "Receiver clear."),
                (
                    410,
                    "cannon  ready  40 of 40 round(s)  band 1 of 2  62% to hit",
                ),
                (
                    411,
                    "machine_gun (mg)  ready  500 of 500 round(s)  21% to hit",
                ),
            ],
        );
        assert!(p.open.weapons.as_ref().is_some_and(|w| w.locked));
        feed(&mut p, &[(500, "Steering to 90°.")]);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        // `STALE_TAIL_MS` of silence: the rows after it are read unlocked.
        feed(&mut p, &[(1_000, "Locked on BA, 62% to hit.")]);
        p.settle(1_000 + STALE_TAIL_MS - 1);
        assert!(p.open.lock_line, "still heading what comes");
        p.settle(1_000 + STALE_TAIL_MS);
        feed(
            &mut p,
            &[
                (1_010 + STALE_TAIL_MS, "cannon  ready  40 of 40 round(s)"),
                (1_100 + STALE_TAIL_MS, "Steering to 90°."),
            ],
        );
        assert_eq!(p.lock, None, "read unlocked");
        // Two locked replies back to back: each takes its own lock line.
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Locked on BA."),
                (1, "cannon  ready  40 of 40 round(s)"),
            ],
        );
        assert!(p.open.weapons.as_ref().is_some_and(|w| w.locked));
        feed(
            &mut p,
            &[
                (2, "Locked on X2."),
                (3, "cannon  ready  40 of 40 round(s)"),
            ],
        );
        assert!(p.open.weapons.as_ref().is_some_and(|w| w.locked));
        assert_eq!(p.lock.as_deref(), Some("X2"));
        feed(&mut p, &[(4, "Steering to 90°.")]);
        assert_eq!(p.lock.as_deref(), Some("X2"));
    }

    /// **The cycle is bounded by the `FIRE`'s send, not its ack** (cut AM.1):
    /// a `cycling` row read inside a second of the ack but past a second of
    /// the `FIRE` teaches nothing.
    #[test]
    fn the_cycle_is_bounded_by_the_fire_not_the_ack() {
        // 550 ms after the ack, 1 150 after the `FIRE`.
        let mut p = Picture::default();
        p.sent_fire(0, "cannon");
        feed(&mut p, &[(600, "Firing cannon.")]);
        assert_eq!(weapon(&p, "cannon").fired_sent_ms, Some(0));
        rows_at(&mut p, 1_150, &["cannon  cycling (6 s)  39 of 40 round(s)"]);
        assert_eq!(weapon(&p, "cannon").cycle_ms, None);
        // The `FIRE` sent at 200: 950 ms before the row, learned.
        let mut p = Picture::default();
        p.sent_fire(200, "cannon");
        feed(&mut p, &[(600, "Firing cannon.")]);
        rows_at(&mut p, 1_150, &["cannon  cycling (6 s)  39 of 40 round(s)"]);
        assert_eq!(weapon(&p, "cannon").cycle_ms, Some(6_000));
        // A row 950 ms after the ack and 1 200 after the `FIRE` reading a
        // second short never sets it.
        let mut p = Picture::default();
        p.sent_fire(0, "cannon");
        feed(&mut p, &[(250, "Firing cannon.")]);
        rows_at(&mut p, 1_200, &["cannon  cycling (5 s)  39 of 40 round(s)"]);
        assert_eq!(weapon(&p, "cannon").cycle_ms, None);
        // An ack that took no mark is bounded by itself, as before.
        let mut p = Picture::default();
        feed(&mut p, &[(250, "Firing cannon.")]);
        assert_eq!(weapon(&p, "cannon").fired_sent_ms, None);
        rows_at(&mut p, 1_200, &["cannon  cycling (6 s)  39 of 40 round(s)"]);
        assert_eq!(weapon(&p, "cannon").cycle_ms, Some(6_000));
    }

    /// **A transient takes its gun's mark** (cut AM.1) and nothing else: the
    /// shot never left.
    #[test]
    fn a_transient_takes_its_guns_mark() {
        let mut p = Picture::default();
        rows_at(
            &mut p,
            0,
            &[
                "cannon  ready  40 of 40 round(s)",
                "machine_gun (mg)  ready  500 of 500 round(s)",
            ],
        );
        p.sent_fire(1_000, "cannon");
        p.sent_fire(1_010, "machine_gun");
        let (tally, weapons) = (p.tally.clone(), p.weapons.clone());
        assert!(p.fire_answered_by_transient(1_020, "cannon"));
        assert!(!p.fire_pending("cannon"));
        assert!(p.fire_pending("machine_gun"));
        assert_eq!(p.tally, tally);
        assert_eq!(
            p.weapons, weapons,
            "the clock, rounds and slack as they were"
        );
        assert!(
            !p.fire_answered_by_transient(1_030, "cannon"),
            "no mark left to take"
        );
    }

    /// **Each `CONTACTS` reply counts once** (cut AM.1), at its first apply or
    /// its close: rows across two settles, the empty form, and a reply closed
    /// before any settle.
    #[test]
    fn contacts_read_counts_each_reply_once() {
        let mut p = Picture::default();
        feed(&mut p, &[(0, &row_line("BA", "blue", 90, 450, 0))]);
        assert_eq!(p.contacts_read, 0);
        p.settle(SNAPSHOT_SETTLE_MS);
        assert_eq!(p.contacts_read, 1);
        feed(&mut p, &[(300, &row_line("X2", "unknown", 300, 250, 0))]);
        p.settle(300 + SNAPSHOT_SETTLE_MS);
        feed(&mut p, &[(600, "Steering to 90°.")]);
        assert_eq!(p.contacts_read, 1, "one reply across two settles");
        feed(&mut p, &[(1_000, "No contacts.")]);
        assert_eq!(p.contacts_read, 2);
        feed(
            &mut p,
            &[
                (2_000, &row_line("BA", "blue", 90, 450, 0)),
                (2_001, "Steering to 90°."),
            ],
        );
        assert_eq!(p.contacts_read, 3, "closed before any settle");
        p.settle(10_000);
        assert_eq!(p.contacts_read, 3);
    }

    /// A brief's eight bearing lines over the same `class` and `metres`.
    fn brief_rays(class: &str, metres: u32) -> Vec<String> {
        crate::arena::lines::BRIEF_BEARINGS
            .iter()
            .map(|(word, _)| format!("{word}: {metres} m of {class}, then beyond the window."))
            .collect()
    }

    const HEADER: &str = "Window around RA: cells 0 to 38 east and 88 to 107 north of 256 by 256; \
                          you at (6, 97), heading 090°; 10 m a cell.";

    /// **A brief is read into the ground and adds no contact** (cut AN.2;
    /// the game's cut AP.4 shape): a whole brief — the header, eight rays,
    /// `No contacts in the window.`, the spot lines, `End of brief.` — gives
    /// a ground of eight rays, whole at the eighth, from the own position, on
    /// the header's 2 560 m square; it adds, changes or drops no contact and
    /// touches no lock. The brief's empty sentence opens nothing and keeps the
    /// brief open, the spot lines keep it open, and its end line closes it and
    /// opens nothing; a `No contacts.` read after it still opens `Contacts`,
    /// and the brief's two sentences read outside a brief are nothing. A brief
    /// cut after five rays stays open across a contact sentence naming a held
    /// hostile, is whole at its end line, and leaves that contact as it was.
    /// A ray with no header just before begins a ground of its own, and a
    /// dark window leaves none.
    #[test]
    fn a_view_brief_is_read_into_the_ground_and_adds_no_contact() {
        let mut p = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            aboard: true,
            ..Picture::default()
        };
        feed(
            &mut p,
            &[
                (900, "Callsign RA."),
                (900, "Red Tank 1: power online, 0 kph, heading 090°"),
                (900, "  at 60 m east, 975 m north; scope clear"),
            ],
        );
        p.settle(1_100);
        let read_before = p.contacts_read;
        let header = feed(&mut p, &[(1_000, HEADER)]);
        assert!(!header[0].journal);
        assert_eq!(header[0].opened, None);
        assert!(p.open.brief);
        let rays = brief_rays("grass", 90);
        for (i, ray) in rays.iter().enumerate() {
            let at = 1_001 + i as u64;
            let applied = feed(&mut p, &[(at, ray)]);
            assert!(!applied[0].journal);
            let ground = p.ground.as_ref().expect("a ground");
            assert_eq!(ground.complete, i == 7, "whole at the eighth ray only");
            assert!(p.open.brief, "kept by each ray");
        }
        let rest = feed(
            &mut p,
            &[
                (1_010, "No contacts in the window."),
                (1_011, "In the window: A The Ford, 190 m at 093°."),
                (1_012, "Off the window: C North Barn, 1390 m at 045°."),
            ],
        );
        assert_eq!(rest[0].opened, None, "the brief's own empty sentence");
        assert!(rest.iter().all(|applied| applied.event.is_none()));
        assert!(rest.iter().all(|applied| applied.opened.is_none()));
        assert!(
            p.open.brief,
            "kept by its empty sentence and its spot lines"
        );
        let end = feed(&mut p, &[(1_013, "End of brief.")]);
        assert_eq!(end[0].opened, None, "the end line opens nothing");
        assert!(!end[0].journal);
        assert!(end[0].event.is_none());
        assert!(!p.open.brief, "closed by its end line");
        let ground = p.ground.clone().expect("a ground");
        assert_eq!(ground.rays.len(), 8);
        assert!(ground.complete);
        assert_eq!(ground.read_ms, 1_000);
        assert_eq!(ground.from_m, Some((60.0, 975.0)));
        assert_eq!(ground.side_m, Some((2_560.0, 2_560.0)));
        assert_eq!(ground.cell_m, Some(10));
        assert_eq!(ground.rays[&45], (vec![("grass".to_owned(), 90)], false));
        assert!(p.contacts.is_empty());
        assert_eq!(p.lock, None);
        assert_eq!(
            p.contacts_read,
            read_before + 1,
            "applied as the empty form"
        );
        // Outside a brief, `No contacts.` still opens `Contacts`, and the
        // brief's own two sentences are nothing.
        let plain = feed(&mut p, &[(2_000, "No contacts.")]);
        assert_eq!(plain[0].opened, Some(AskKind::Contacts));
        let read_now = p.contacts_read;
        let stray = feed(
            &mut p,
            &[
                (2_001, "No contacts in the window."),
                (2_002, "End of brief."),
            ],
        );
        assert!(stray.iter().all(|applied| applied.opened.is_none()));
        assert_eq!(p.contacts_read, read_now, "no empty form outside a brief");
        assert!(!p.open.brief);

        // A held hostile and its lock, then a brief cut after five rays whose
        // contact sentence names it: whole at the sentence, BA as it was.
        feed(
            &mut p,
            &[
                (3_000, &row_line("BA", "blue", 90, 450, 1)),
                (3_100, "Locked on BA, 62% to hit."),
            ],
        );
        p.settle(3_400);
        let before = p.contacts.clone();
        assert!(before.contains_key("BA"));
        let mut lines: Vec<(u64, &str)> = vec![(4_000, HEADER)];
        for ray in rays.iter().take(5) {
            lines.push((4_001, ray));
        }
        feed(&mut p, &lines);
        assert!(!p.ground.as_ref().expect("a ground").complete, "five rays");
        feed(
            &mut p,
            &[(
                4_002,
                "BA, blue: bearing 090° (rel 000°), far, 450 m, 1 s old.",
            )],
        );
        assert!(p.open.brief, "kept by its contact sentence");
        assert!(
            !p.ground.as_ref().expect("a ground").complete,
            "not whole at a contact sentence"
        );
        feed(&mut p, &[(4_003, "End of brief.")]);
        assert!(!p.open.brief);
        let ground = p.ground.as_ref().expect("a ground");
        assert!(ground.complete, "whole at the end line");
        assert_eq!(ground.rays.len(), 5);
        assert_eq!(p.contacts, before, "the brief changes no contact");
        assert_eq!(p.lock.as_deref(), Some("BA"));

        // A ray with no header just before: a ground of its own, no size.
        feed(
            &mut p,
            &[(
                9_000,
                "North: 10 m of road, then grass to 90 m, then beyond the window.",
            )],
        );
        let ground = p.ground.as_ref().expect("a ground");
        assert_eq!(ground.read_ms, 9_000);
        assert_eq!(ground.side_m, None);
        assert_eq!(ground.rays.len(), 1);
        // A dark window: no ground.
        feed(
            &mut p,
            &[(
                9_500,
                "The window is dark: there is no ground under the panel to draw.",
            )],
        );
        assert_eq!(p.ground, None);
        // A new match clears it.
        feed(&mut p, &[(10_000, HEADER)]);
        assert!(p.ground.is_some());
        p.new_match();
        assert_eq!(p.ground, None);
        assert!(!p.open.brief);
    }

    /// **A brief stays open across chatter and the pieces of a wrapped line**
    /// (cut AN.2; the game's cut AP.4 shape): a header the terminal wrapped
    /// into two pieces the reader cannot take, two bearings, a teammate's
    /// channel line and `Receiver clear.` (the latency rig's injections), a
    /// bearing wrapped into two pieces, and the last five bearings: the brief
    /// opens at its first bearing and stays open, its ground is not whole
    /// before its end line (seven bearings read), its own `No contacts in the
    /// window.` opens nothing and keeps it open, and `End of brief.` closes
    /// it. A contact sentence or a spot line no longer closes a brief (the
    /// game prints its end line every time); the end line does, and a `No
    /// contacts.` after it opens `Contacts`.
    #[test]
    fn a_brief_stays_open_across_chatter_and_wrapped_pieces() {
        let mut p = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Blue),
            aboard: true,
            ..Picture::default()
        };
        feed(
            &mut p,
            &[
                (900, "Callsign BA."),
                (900, "Blue Tank 1: power online, 0 kph, heading 270°"),
                (900, "  at 2495 m east, 1075 m north; scope clear"),
            ],
        );
        p.settle(1_100);
        feed(
            &mut p,
            &[
                (
                    1_000,
                    "Window around BA: cells 217 to 255 east and 100 to 119 north of 256 by 256; you",
                ),
                (1_000, "at (249, 107), heading 270°; 10 m a cell."),
            ],
        );
        assert!(!p.open.brief, "a wrapped header is not read");
        let rays = brief_rays("grass", 90);
        for ray in rays.iter().take(2) {
            feed(&mut p, &[(1_001, ray)]);
            assert!(p.open.brief, "opened by a bearing");
        }
        let ground = p.ground.as_ref().expect("a ground of its own");
        assert_eq!(ground.side_m, None);
        let kept = feed(
            &mut p,
            &[
                (1_002, "[RedNet] Ada: on your left"),
                (1_002, "Receiver clear."),
                (
                    1_003,
                    "East: 10 m of road, then grass to 30 m, then light woods to 50 m, then",
                ),
                (1_003, "rubble to 70 m, then beyond the window."),
            ],
        );
        assert!(kept.iter().all(|applied| applied.opened.is_none()));
        assert!(p.open.brief, "kept across chatter and a wrapped bearing");
        assert!(!p.ground.as_ref().expect("a ground").complete);
        for ray in rays.iter().skip(3) {
            feed(&mut p, &[(1_004, ray)]);
        }
        let ground = p.ground.as_ref().expect("a ground");
        assert_eq!(ground.rays.len(), 7);
        assert!(!ground.complete, "seven bearings, and no tail yet");
        assert!(p.open.brief);
        let empty = feed(&mut p, &[(1_005, "No contacts in the window.")]);
        assert_eq!(empty[0].opened, None, "the brief's own empty sentence");
        assert!(p.open.brief, "kept by its own empty sentence");
        assert!(!p.ground.as_ref().expect("a ground").complete);
        let end = feed(&mut p, &[(1_006, "End of brief.")]);
        assert_eq!(end[0].opened, None, "the end line opens nothing");
        assert!(p.ground.as_ref().expect("a ground").complete);
        assert!(!p.open.brief);
        // A contact sentence or a spot line keeps a brief open; its end line
        // closes it, and a `No contacts.` after that opens `Contacts`.
        for kept in [
            "BA, blue: bearing 090° (rel 000°), far, 450 m, 1 s old.",
            "X12, unknown: bearing 045° (rel 135°), far, 550 m, 0 s old.",
            "1 contact plots beyond this window; CONTACTS lists it.",
            "In the window: A The Ford, 190 m at 093°.",
            "Off the window: C North Barn, 1390 m at 045°.",
        ] {
            feed(&mut p, &[(2_000, HEADER)]);
            for ray in &rays {
                feed(&mut p, &[(2_001, ray)]);
            }
            feed(&mut p, &[(2_002, "[RedNet] Ada: on your left")]);
            assert!(p.open.brief, "{kept}");
            let applied = feed(&mut p, &[(2_003, kept)]);
            assert_eq!(applied[0].opened, None, "{kept}");
            assert!(p.open.brief, "kept by {kept}");
            feed(&mut p, &[(2_004, "End of brief.")]);
            assert!(!p.open.brief, "closed by its end line after {kept}");
            let after = feed(&mut p, &[(2_005, "No contacts.")]);
            assert_eq!(after[0].opened, Some(AskKind::Contacts), "{kept}");
        }
    }

    /// **The board says the mode** (cut AN.2): a board head resets the rule to
    /// a skirmish, a mode line sets it, and a new match keeps it (the board
    /// said it for this match).
    #[test]
    fn the_board_says_the_mode() {
        let mut p = Picture::default();
        assert_eq!(p.match_mode, MatchMode::Skirmish);
        feed(
            &mut p,
            &[
                (1_000, "Simulator centre: selection"),
                (1_000, "  red: nobody"),
                (
                    1_000,
                    "  mode: hold — keep The Crossroads (B) uncontested for 120 s",
                ),
            ],
        );
        assert_eq!(p.match_mode, MatchMode::Hold);
        p.new_match();
        assert_eq!(p.match_mode, MatchMode::Hold, "kept by a new match");
        feed(&mut p, &[(2_000, "Simulator centre: selection")]);
        assert_eq!(p.match_mode, MatchMode::Skirmish, "a board head resets it");
        feed(
            &mut p,
            &[(2_000, "  mode: range — 2 targets on the blue side")],
        );
        assert_eq!(p.match_mode, MatchMode::Range);
        feed(&mut p, &[(3_000, "Simulator centre: idle")]);
        assert_eq!(p.match_mode, MatchMode::Skirmish);
    }

    /// **Only this match's own `Power online.` is seen to come up** (cut
    /// AN.2): `powered_here` is set by the ordinary line when it is the one
    /// that sets the power's time, never by `Power is already online.`, a
    /// `Back aboard` row or a running `STATUS` head, nor by a `Power online.`
    /// after one of those; a new match clears it.
    #[test]
    fn powered_here_only_from_the_own_power_online() {
        let running = || Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            ..Picture::default()
        };
        let mut p = running();
        feed(&mut p, &[(1_500, "Power online.")]);
        assert!(p.powered_here);
        p.new_match();
        assert!(!p.powered_here);
        let mut p = running();
        feed(
            &mut p,
            &[
                (1_000, "Power is already online."),
                (2_000, "Power online."),
            ],
        );
        assert!(!p.powered_here);
        let mut p = running();
        feed(
            &mut p,
            &[
                (2_000, "Back aboard RA after 45 s away."),
                (2_010, "  Power online."),
                (2_020, "Power online."),
            ],
        );
        assert!(!p.powered_here);
        let mut p = running();
        feed(
            &mut p,
            &[
                (3_000, "Callsign RA."),
                (3_000, "Red Tank 1: power online, 30 kph, heading 090°"),
                (3_100, "Power online."),
            ],
        );
        assert!(!p.powered_here);
    }

    /// **A friendly hit keeps its mark in the event** (cut AN.2), so the
    /// pilot's exchange counts only hits on the other side.
    #[test]
    fn a_friendly_hit_event_says_so() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (1_000, "Hit on RB (FRIENDLY): turret."),
                (1_100, "Hit on BA: turret."),
                (1_200, "Hit on RB (FRIENDLY)."),
            ],
        );
        let texts: Vec<String> = applied
            .into_iter()
            .map(|a| a.event.expect("an event").text)
            .collect();
        assert_eq!(
            texts,
            [
                "Hit on RB (FRIENDLY): turret.",
                "Hit on BA: turret.",
                "Hit on RB (FRIENDLY)."
            ]
        );
    }

    // -- cut AN.3: the lattice ----------------------------------------------------

    /// A running picture with the lattice knob `on`.
    fn lattice_picture(on: bool) -> Picture {
        let mut p = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            ..Picture::default()
        };
        p.set_lattice(on);
        p
    }

    /// Ten of the arena's notices on its lattice at phase 5: `Hit on BA:
    /// turret.` read at 1 005 + 250k.
    fn notices_at_phase_five(p: &mut Picture) {
        for k in 0..10 {
            feed(p, &[(1_005 + 250 * k, "Hit on BA: turret.")]);
        }
    }

    /// The machine gun's `FIRE` typed `rtt_ms` before its ack read at `ack`.
    fn mg_shot(p: &mut Picture, ack: u64, rtt_ms: u64) {
        p.sent_fire(ack - rtt_ms, "machine_gun");
        feed(p, &[(ack, "Firing machine_gun.")]);
    }

    /// The machine gun's cycle (1 000 ms) learned from two shots whose
    /// round trips were `rtt_ms`, each read whole half a second on: two round
    /// trips kept, so the third ack is the first that may set a lattice clock.
    fn mg_learned(p: &mut Picture, rtt_ms: u64) {
        for ack in [4_000, 6_000] {
            mg_shot(p, ack, rtt_ms);
            rows_at(
                p,
                ack + 500,
                &["machine_gun (mg)  cycling (1 s)  480 of 500 round(s)"],
            );
        }
        assert_eq!(weapon(p, "machine_gun").cycle_ms, Some(1_000));
    }

    /// **The phase is read from the arena's notices** (cut AN.3): the densest
    /// 50 ms bin on the 250 ms lattice, read as its earliest sample; acks,
    /// reply rows, refusals and lobby lines add nothing; too few, or too small
    /// a share, read nothing.
    #[test]
    fn the_lattice_phase_is_read_from_the_arenas_notices() {
        // Ten notices at 1 005 + 250k and two of noise 120 ms on.
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        feed(
            &mut p,
            &[
                (1_125, "Miss on BA: the target moved."),
                (1_375, "New contact X1: bearing 000°, far."),
            ],
        );
        assert_eq!(p.lattice.samples.len(), 12);
        assert_eq!(p.lattice.phase(), Some(5));
        // A cluster at 245 + 250k is read at 245, not at the tie-broken
        // window's start, 200.
        let mut p = lattice_picture(true);
        for k in 0..10 {
            feed(&mut p, &[(245 + 250 * k, "Held: something is in the way.")]);
        }
        assert_eq!(p.lattice.phase(), Some(245));
        // Every kind the phase reads counts: a contact notice, an
        // identification, a loss, a hit scored, a miss, a hit taken, a stop.
        let mut p = lattice_picture(true);
        feed(
            &mut p,
            &[
                (1_005, "New contact X1: bearing 000°, far."),
                (1_255, "Contact X1 identified: BA, blue."),
                (1_505, "Contact BA lost."),
                (1_755, "Hit on BA: turret."),
                (2_005, "Miss on BA: the round went wide."),
                (
                    2_255,
                    "Hit on the hull's front facet: 300 of 400 armour left.",
                ),
                (2_505, "Held: the ground ahead will not take the tank."),
            ],
        );
        assert_eq!(p.lattice.samples.len(), 7);
        // Acks, reply rows, refusals, lock lines and the lobby add no
        // sample; nor does a notice of a match not running.
        p.sent_fire(2_600, "machine_gun");
        feed(
            &mut p,
            &[
                (2_610, "Firing machine_gun."),
                (2_620, "FIRE cannon refused. Still cycling: 3 s to go."),
                (2_630, "Locked on BA, 62% to hit."),
                (2_640, "BA  blue  bearing 090° (rel 000°)  far  450 m  0 s"),
                (
                    2_650,
                    "machine_gun (mg)  cycling (1 s)  499 of 500 round(s)",
                ),
                (2_660, "Callsign RA."),
                (2_670, "Red Tank 1: power online, 0 kph, heading 090°"),
                (2_680, "Steering to 90°."),
                (2_690, "Match 2 is boarding. Every pilot: BOARD your pod."),
            ],
        );
        assert_eq!(p.lattice.samples.len(), 7, "{:?}", p.lattice.samples);
        // Seven samples, however clustered, read nothing.
        assert_eq!(p.lattice.phase(), None);
        let mut q = lattice_picture(true);
        for k in 0..7 {
            feed(&mut q, &[(1_005 + 250 * k, "Hit on BA: turret.")]);
        }
        assert_eq!(q.lattice.phase(), None, "seven");
        feed(&mut q, &[(1_005 + 250 * 7, "Hit on BA: turret.")]);
        assert_eq!(q.lattice.phase(), Some(5), "eight");
        let mut idle = Picture::default();
        feed(&mut idle, &[(1_005, "Hit on BA: turret.")]);
        assert!(idle.lattice.samples.is_empty(), "not running");
        // Nine in the bin of fifteen (60 per cent) read nothing; the other six
        // spread 30 ms apart, no two of them within one bin of the cluster.
        let mut q = lattice_picture(true);
        for k in 0..9 {
            feed(&mut q, &[(1_005 + 250 * k, "Hit on BA: turret.")]);
        }
        for (k, offset) in [80, 110, 140, 170, 200, 230].into_iter().enumerate() {
            feed(
                &mut q,
                &[(
                    5_000 + 250 * k as u64 + offset,
                    "Miss on BA: the target moved.",
                )],
            );
        }
        assert_eq!(q.lattice.samples.len(), 15);
        assert_eq!(q.lattice.phase(), None, "60 per cent");
        // At most the last 24 are kept.
        for k in 0..30 {
            feed(&mut q, &[(20_005 + 250 * k, "Hit on BA: turret.")]);
        }
        assert_eq!(q.lattice.samples.len(), LATTICE_SAMPLES);
        assert_eq!(q.lattice.phase(), Some(5));
        // The next lattice point is strictly after.
        assert_eq!(next_lattice(10_102, 5), 10_255);
        assert_eq!(next_lattice(10_005, 5), 10_255);
        assert_eq!(next_lattice(10_004, 5), 10_005);
        assert_eq!(next_lattice(10_249, 245), 10_495);
        assert_eq!(next_lattice(0, 0), LATTICE_MS);
    }

    /// **A learned gun fires a guard past the next lattice point** (cut AN.3):
    /// with the cycle 1 000, the phase 5 and round trips of 40 ms, an ack read
    /// at 10 102 is ready at `next_lattice(10 102, 5) + 1 000 + guard − 40`,
    /// on the lattice; the note is said once a match; and the round trip is
    /// each ack's read less its `FIRE`'s send, the median of the last eight.
    #[test]
    fn a_learned_gun_fires_a_guard_past_the_next_lattice_point() {
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        mg_learned(&mut p, 40);
        assert_eq!(p.lattice.rtt(), None, "two round trips");
        assert!(!weapon(&p, "machine_gun").on_lattice);
        assert!(p.take_notes().is_empty());
        mg_shot(&mut p, 10_102, 40);
        assert_eq!(p.lattice.rtt(), Some(40));
        let mg = weapon(&p, "machine_gun");
        let ready = next_lattice(10_102, 5) + 1_000 + LATTICE_GUARD_MS - 40;
        assert_eq!(ready, 10_255 + 1_000 + LATTICE_GUARD_MS - 40);
        assert_eq!(mg.next_ready_ms, Some(ready));
        assert!(mg.on_lattice);
        assert_eq!(
            p.take_notes(),
            ["Firing on the arena's lattice: phase 5 ms, round trip 40 ms."]
        );
        // The next lattice-clocked ack says nothing more.
        mg_shot(&mut p, 12_102, 40);
        assert!(weapon(&p, "machine_gun").on_lattice);
        assert!(p.take_notes().is_empty(), "once a match");
        // The median by the index rule: of 40, 40, 40, 40, 60, 90 the lower
        // middle; at most eight kept.
        for (ack, rtt) in [(14_102, 60), (16_102, 90)] {
            mg_shot(&mut p, ack, rtt);
        }
        assert_eq!(p.lattice.rtts.len(), 6);
        assert_eq!(p.lattice.rtt(), Some(40));
        for k in 0..6 {
            mg_shot(&mut p, 20_102 + 2_000 * k, 120);
        }
        assert_eq!(p.lattice.rtts.len(), LATTICE_RTT_SAMPLES);
        assert_eq!(p.lattice.rtt(), Some(120));
        // An ack with no send of its own (a picture never told of the `FIRE`)
        // keeps no round trip.
        feed(&mut p, &[(40_000, "Firing machine_gun.")]);
        assert_eq!(p.lattice.rtts.len(), LATTICE_RTT_SAMPLES);
    }

    /// **A slow round trip, an unlearned cycle or the knob off keeps AL's
    /// clock** (cut AN.3).
    #[test]
    fn a_slow_round_trip_or_an_unlearned_cycle_keeps_als_clock() {
        // Round trips of 250 ms: over the bound.
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        mg_learned(&mut p, 250);
        mg_shot(&mut p, 10_102, 250);
        assert_eq!(p.lattice.rtt(), Some(250));
        const { assert!(250 > LATTICE_RTT_MAX_MS) };
        let mg = weapon(&p, "machine_gun");
        assert!(!mg.on_lattice);
        assert_eq!(mg.next_ready_ms, Some(10_102 + 1_000 + mg.slack_ms));
        assert!(p.take_notes().is_empty());
        // At the bound itself, the lattice.
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        mg_learned(&mut p, LATTICE_RTT_MAX_MS);
        mg_shot(&mut p, 10_102, LATTICE_RTT_MAX_MS);
        assert!(weapon(&p, "machine_gun").on_lattice);
        // No cycle learned: no clock at all, as AL left it.
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        for ack in [4_000, 6_000, 10_102] {
            mg_shot(&mut p, ack, 40);
        }
        assert_eq!(p.lattice.rtt(), Some(40));
        let mg = weapon(&p, "machine_gun");
        assert_eq!(
            (mg.cycle_ms, mg.next_ready_ms, mg.on_lattice),
            (None, None, false)
        );
        // No phase (too few notices): AL's clock.
        let mut p = lattice_picture(true);
        mg_learned(&mut p, 40);
        mg_shot(&mut p, 10_102, 40);
        let mg = weapon(&p, "machine_gun");
        assert!(!mg.on_lattice);
        assert_eq!(mg.next_ready_ms, Some(10_102 + 1_000 + mg.slack_ms));
        // The knob off: AL's clock, whatever is known.
        let mut p = lattice_picture(false);
        notices_at_phase_five(&mut p);
        mg_learned(&mut p, 40);
        mg_shot(&mut p, 10_102, 40);
        assert_eq!(p.lattice.phase(), Some(5));
        assert_eq!(p.lattice.rtt(), Some(40));
        let mg = weapon(&p, "machine_gun");
        assert!(!mg.on_lattice);
        assert_eq!(mg.next_ready_ms, Some(10_102 + 1_000 + mg.slack_ms));
        assert!(p.take_notes().is_empty());
    }

    /// **Two refusals bench the gun** (cut AN.3): two `Still cycling` answers
    /// to a lattice-clocked gun's own `FIRE`s inside 30 s put it on AL's clock
    /// for 30 s, said once; the retry is AL's; after the bench, the lattice.
    #[test]
    fn two_refusals_bench_the_gun() {
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        mg_learned(&mut p, 40);
        mg_shot(&mut p, 10_102, 40);
        assert!(weapon(&p, "machine_gun").on_lattice);
        p.take_notes();
        let refuse = |p: &mut Picture, at: u64| {
            p.sent_fire(at - 40, "machine_gun");
            feed(
                p,
                &[(at, "FIRE machine_gun refused. Still cycling: 1 s to go.")],
            );
        };
        // One refusal: the retry is AL's, and no bench.
        refuse(&mut p, 11_300);
        let mg = weapon(&p, "machine_gun");
        assert_eq!(mg.next_ready_ms, Some(11_300 + FIRE_RETRY_MS));
        assert_eq!(mg.lattice_misses.len(), 1);
        assert_eq!(mg.lattice_bench_until_ms, None);
        assert!(p.take_notes().is_empty());
        // The retry acked, on the lattice again; a second refusal inside the
        // bench's span benches it.
        mg_shot(&mut p, 11_600, 40);
        assert!(weapon(&p, "machine_gun").on_lattice);
        refuse(&mut p, 12_800);
        let mg = weapon(&p, "machine_gun");
        assert_eq!(mg.lattice_bench_until_ms, Some(12_800 + LATTICE_BENCH_MS));
        assert_eq!(mg.next_ready_ms, Some(12_800 + FIRE_RETRY_MS));
        assert_eq!(
            p.take_notes(),
            ["The machine_gun is off the lattice clock for 30 s: refused twice."]
        );
        // Benched: AL's clock on every ack until it lapses.
        mg_shot(&mut p, 13_100, 40);
        let mg = weapon(&p, "machine_gun");
        assert!(!mg.on_lattice);
        assert_eq!(mg.next_ready_ms, Some(13_100 + 1_000 + mg.slack_ms));
        mg_shot(&mut p, 12_800 + LATTICE_BENCH_MS - 1, 40);
        assert!(!weapon(&p, "machine_gun").on_lattice);
        mg_shot(&mut p, 12_800 + LATTICE_BENCH_MS + 1_000, 40);
        assert!(weapon(&p, "machine_gun").on_lattice, "the bench lapsed");
        assert!(p.take_notes().is_empty(), "the lattice said once a match");
        // Two refusals more than 30 s apart bench nothing; an AL-clocked
        // gun's refusals count for nothing.
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        mg_learned(&mut p, 40);
        mg_shot(&mut p, 10_102, 40);
        refuse(&mut p, 11_300);
        mg_shot(&mut p, 11_600, 40);
        refuse(&mut p, 11_300 + LATTICE_BENCH_MS + 1);
        assert_eq!(weapon(&p, "machine_gun").lattice_bench_until_ms, None);
        let mut p = lattice_picture(false);
        mg_learned(&mut p, 40);
        mg_shot(&mut p, 10_102, 40);
        refuse(&mut p, 11_300);
        refuse(&mut p, 11_800);
        let mg = weapon(&p, "machine_gun");
        assert!(mg.lattice_misses.is_empty());
        assert_eq!(mg.lattice_bench_until_ms, None);
    }

    /// **A cycling row never pulls a lattice clock early** (cut AN.3): the
    /// row's upper bound carries at least the guard on a lattice-clocked gun,
    /// where a gun on AL's clock with no slack left is pulled to the row's
    /// whole seconds.
    #[test]
    fn a_cycling_row_never_pulls_a_lattice_clock_early() {
        let at_10_102 = |on: bool| {
            let mut p = lattice_picture(on);
            notices_at_phase_five(&mut p);
            mg_learned(&mut p, 40);
            if let Some(mg) = p.weapons.iter_mut().find(|w| w.id == "machine_gun") {
                mg.slack_ms = 0;
            }
            mg_shot(&mut p, 10_102, 40);
            p
        };
        let mut p = at_10_102(true);
        let clock = 10_255 + 1_000 + LATTICE_GUARD_MS - 40;
        assert_eq!(weapon(&p, "machine_gun").next_ready_ms, Some(clock));
        assert_eq!(weapon(&p, "machine_gun").slack_ms, 0);
        rows_at(
            &mut p,
            10_250,
            &["machine_gun (mg)  cycling (1 s)  478 of 500 round(s)"],
        );
        assert_eq!(weapon(&p, "machine_gun").next_ready_ms, Some(clock));
        // The same clock on a gun off the lattice with no slack left: the
        // row pulls it to the row's whole second.
        let mut p = at_10_102(false);
        if let Some(mg) = p.weapons.iter_mut().find(|w| w.id == "machine_gun") {
            assert!(!mg.on_lattice);
            mg.next_ready_ms = Some(clock);
        }
        rows_at(
            &mut p,
            10_250,
            &["machine_gun (mg)  cycling (1 s)  478 of 500 round(s)"],
        );
        assert_eq!(weapon(&p, "machine_gun").next_ready_ms, Some(11_250));
    }

    /// **A ready row never pulls a lattice clock early** (cut AO.2): a
    /// `ready` row read inside the guard — inside [`READY_AFTER_ACK_MS`] of
    /// the ack, its rounds one fewer, so the stale-ready guard lets it through
    /// — is the cockpit's frame, not the arena's acceptance, and a
    /// lattice-clocked gun keeps its later clock; with the lattice off the
    /// same row sets AL's clock to its own time, as before.
    #[test]
    fn a_ready_row_never_pulls_a_lattice_clock_early() {
        let at_10_102 = |on: bool| {
            let mut p = lattice_picture(on);
            notices_at_phase_five(&mut p);
            mg_learned(&mut p, 40);
            mg_shot(&mut p, 10_102, 40);
            p
        };
        let row = "machine_gun (mg)  ready  479 of 500 round(s)";
        // The ack's lattice clock, and the row read before it.
        let mut p = at_10_102(true);
        let clock = next_lattice(10_102, 5) + 1_000 + LATTICE_GUARD_MS - 40;
        let read = 11_200;
        const { assert!(11_200 - 10_102 < READY_AFTER_ACK_MS) };
        assert!(read < clock);
        let mg = weapon(&p, "machine_gun");
        assert!(mg.on_lattice);
        assert_eq!(mg.next_ready_ms, Some(clock));
        assert_eq!((mg.rounds, mg.shots_since_read), (480, 1));
        rows_at(&mut p, read, &[row]);
        let mg = weapon(&p, "machine_gun");
        assert_eq!(mg.read_ms, read, "the stale-ready guard let it through");
        assert!(mg.ready);
        assert_eq!(mg.next_ready_ms, Some(clock), "the lattice clock kept");
        // A ready row read after that clock is the evidence, as before.
        mg_shot(&mut p, 12_102, 40);
        let later = next_lattice(12_102, 5) + 1_000 + LATTICE_GUARD_MS - 40 + 10;
        rows_at(
            &mut p,
            later,
            &["machine_gun (mg)  ready  478 of 500 round(s)"],
        );
        assert_eq!(weapon(&p, "machine_gun").next_ready_ms, Some(later));
        // The lattice off: the same row sets the clock to its read time.
        let mut p = at_10_102(false);
        let mg = weapon(&p, "machine_gun");
        assert!(!mg.on_lattice);
        assert_eq!(mg.next_ready_ms, Some(10_102 + 1_000 + mg.slack_ms));
        rows_at(&mut p, read, &[row]);
        assert_eq!(weapon(&p, "machine_gun").next_ready_ms, Some(read));
    }

    /// **The lattice is forgotten at a new match** (cut AN.3): the samples,
    /// the round trips and the note go; the knob stays.
    #[test]
    fn the_lattice_is_forgotten_at_a_new_match() {
        let mut p = lattice_picture(true);
        notices_at_phase_five(&mut p);
        mg_learned(&mut p, 40);
        mg_shot(&mut p, 10_102, 40);
        assert!(p.lattice.said);
        assert!(p.lattice.phase().is_some() && p.lattice.rtt().is_some());
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(20_000, "Match 2 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.following, Some(2));
        assert!(p.lattice.on, "the knob is the table's");
        assert!(p.lattice.samples.is_empty() && p.lattice.rtts.is_empty());
        assert!(!p.lattice.said);
        assert_eq!((p.lattice.phase(), p.lattice.rtt()), (None, None));
        assert!(p.weapons.is_empty(), "the guns with it");
        // The default is off.
        assert!(!Picture::default().lattice.on);
    }
}
