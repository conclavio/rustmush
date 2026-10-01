//! The pilot (cut AJ.2; cut AK.1): a deterministic tactician over the
//! [`Picture`].
//!
//! [`Pilot::decide`] reads the picture and a [`Doctrine`] and says what to type
//! next — a heading, a speed, a lock, the guns to fire, whether to ask where The
//! Crossroads lies. No I/O and no clock: `now_ms` is the runner's session clock,
//! and the lane decides which of the orders go out this tick and tells the pilot
//! ([`Pilot::sent_heading`], [`Pilot::sent_speed`], [`Pilot::sent_lock`]). What
//! it did in words ([`Pilot::take_notes`]) the lane hands the runner as `arena`
//! events.
//!
//! **Written from the player help alone.** Nothing of the machines' own
//! controller is read, imported or mirrored. Every number below is a sentence
//! of `help simulator` ("Fighting the machines", "The ground") or `help cockpit`
//! ("Driving", "Seeing", "Shooting", "Refusals"), or a reading of the bot's own
//! transcripts:
//!
//! * a machine "closes: to about seven tenths of what its own sight and its
//!   gun's inner band allow (420 m for the stock tank), where it stops, turns
//!   onto you and shoots" — so the pilot does not stand and trade at that range;
//! * "on grass a stopped tank can be seen out to 80 per cent of that (480 m)
//!   and a moving one out to all of it" (600 m) — so the orbit's default radius,
//!   **450 m**, sits just outside the stand-off and inside the stopped-tank
//!   sight: the pilot keeps a stopped machine in view while making it turn. The
//!   machines stop to shoot, so the stopped tank's 480 m, less a margin for the
//!   scope's 50 m rounding and the tank's own drift, is the **ceiling** of every
//!   band the pilot flies ([`SIGHT_BAND_MAX_M`], 450 m), the fall back's
//!   included: the moving sight's 600 m is where it loses them;
//! * "a side is twice the target, 7 m of flank against 3.5 m of bow", and "one
//!   caught from behind spends its thinnest" plate — so the orbit circles toward
//!   the target's **rear** when the picture holds a track on it;
//! * a machine "drives at where it last saw you", and a contact "is drawn where
//!   it was last seen" — so without a track the orbit **reverses** every 20 s,
//!   which makes the machine's plot of the pilot go stale;
//! * "`SPEED` says both when you ask for more than it has" — so the first speed
//!   order is `999`, which reads the tank's own ceilings; until they are known
//!   "full ahead" is 40 kph, under the stock tank's 41 on grass;
//! * "Three seconds of holding a return is enough to tell it apart" whatever is
//!   locked — so an unidentified contact is never locked, and never fired on;
//! * the `% to hit` is printed only for the lock and the band word is the
//!   scope's ("It is **not** a judgement of your gun") — so the target is the
//!   nearest hostile with a fresh plot, and a held lock is kept unless another
//!   is at least 100 m nearer;
//! * "Past a gun's last band the chance is 0%: the round is still fired and
//!   spent" — so a gun whose last read said 0% is not fired;
//! * **each gun fires on its own clock** ([`super::picture`]'s module head): the
//!   seconds of `cycling (N s)` and `Still cycling: N s to go.` are rounded up,
//!   a row read inside the first second after the ack prints the whole cycle,
//!   and the shot goes on the arena's next 250 ms lattice point after the ack
//!   (all read from the bot's own transcripts; "you and the arena are a pulse
//!   apart"), so a gun is believed ready only when its one clock,
//!   [`super::picture::Weapon`]'s `next_ready_ms`, says so, or — with no clock
//!   yet — when its row says `ready` well after the ack;
//! * a gun the tank says it does not carry, a gun whose rounds the picture
//!   counts down to none ([`super::picture::Weapon::rounds_left`]), and every
//!   gun while a lost section's `its weapons are gone` waits for `WEAPONSTATUS`
//!   are not fired (cut AL.1; the bot's own transcripts typed `FIRE` at guns
//!   lost with the turret for seconds);
//! * "Ground a tank cannot enter stops it, and the cockpit says so once per
//!   stop", and "`STATUS` carries `HELD` until you order something that frees
//!   it" — so a `Held:` notice turns the tank 45° for 3 s and orders the speed
//!   again (insurance: no transcript shows a `SPEED` freeing a held tank), and a
//!   tank `STATUS` still reads held at 0 kph is turned further **from the
//!   heading it was stopped on** (90°, 135°, 180°, each judged a whole
//!   [`STALL_MS`] later, above the 3.1 s p90 and 4.0 s longest lag the bot's
//!   own transcript shows from `Steering to D°.` to a `STATUS` reading D), and
//!   at last backs off ("a negative speed reverses", `help cockpit`);
//! * "Hurt badly enough it breaks off for its own entry zone", "Open the range
//!   past its sight and it comes after you again", and "A machine that loses you
//!   goes to where it last saw you" — so a lost hostile is **hunted**: to its
//!   last plot, then along its estimated course (its track, or the way the
//!   range was opening) while the plot is young, then one bounded sweep, and
//!   only then the search. "The proving ground is 2.56 km square" and `STATUS`
//!   counts `at {x} m east, {y} m north` from its south-west corner (the bot's
//!   own transcripts read a tank on cell (100, 97) at (1005, 974)), so no hunt
//!   aim ever lies past the edge, and the course leg, driven on along the
//!   course once level with its moving aim, ends where the course's next
//!   100 m would cross the edge or once it is that far past the aim; a wreck
//!   is never hunted, targeted or shot at (its row stays listed until it
//!   expires);
//! * "A machine that loses you … crosses to the other side's entry zone to
//!   look", and the proving ground's roads meet at The Crossroads (B) — so with
//!   no identified hostile held for 5 s (an unidentified contact does not
//!   count) and no hunt left, the pilot drives for `B`, past it 800 m, and back.
//!
//! **The pilot's own rule above any doctrine.** Hurt to half its hull's
//! internals, or with a plate **of its hull** at a quarter or gone, it falls
//! back: away from the target for 10 s or until it has opened to its widened
//! band, whichever is first; the widened band is 150 m wider than the
//! doctrine's, never narrower than it and never past [`SIGHT_BAND_MAX_M`]. After
//! that it flies its doctrine again at the widened band, `close` flown as
//! `orbit` there (a failing hull does not drive in to 200 m).
//!
//! **One stance precedence** (`Pilot::motion`), each step returning when it
//! applies: (1) the held turn and the stall; (2) the pilot's own fall-back
//! withdrawal; (3) a doctrine `withdraw`; (4) the doctrine's stance round a
//! hostile whose plot is younger than [`HUNT_STALE_PLOT_MS`]; (5) the hunt;
//! (4 again) the doctrine's stance round an older row never seen fresh; (6) the
//! search. A doctrine `withdraw` reaches step 3 only as a bounded hold on a
//! failing hull (the lane enforces it, cut AK.2), and it outranks the widened
//! orbit after a fall back, so it is what turns the tank away for its hold. A
//! doctrine `hunt` (cut AK.2) drives straight at a hostile until its band and
//! circles it there, but only while its plot is fresh ([`FRESH_PLOT_MS`]): past
//! that it hunts at once from the trail, and with no trail it searches.
//!
//! **What a tuning round may move** (cut AL.3, between its measurement runs,
//! and nothing else). **AK's constants are frozen**: the hunt's, the fall
//! back's, the stall's and the held turn's stay as AK.3 left them. AL.3 may
//! move only AL's named constants within their bounds; AL.1's are, in the
//! picture, [`super::picture::FIRE_SLACK_START_MS`],
//! [`super::picture::FIRE_SLACK_UP_MS`] and [`super::picture::FIRE_RETRY_MS`],
//! and in the lane [`super::lane::WEAPONS_LEARNED_MS`]. The brief bounds them
//! 0..=500, 100..=300, 150..=500 and 5 000..=10 000; **the lane's tests admit
//! less**, one constant moved at a time from the built tree (the rest as
//! built): `FIRE_SLACK_START_MS` 50..=410 (seen passing at 50, 100, 150, 200,
//! 250 built, 300, 350, 400 and 410; at 0 the lattice rig's cannon is refused
//! 3 lines of 22, over its one in ten, and from 420 its median gap is over
//! its bound), `FIRE_SLACK_UP_MS` 100..=200 (100, 150 built and 200 pass; at
//! 250 and 300 the lattice rig's cannon median is over its bound),
//! `FIRE_RETRY_MS` 150..=500 (both ends and 250 built pass; under
//! [`super::lane::FIRE_GAP_MS`] the lane's gap after the refused line governs
//! the retry instead), and `WEAPONS_LEARNED_MS` 6 001..=10 000 (6 001, 7 000,
//! 8 000 built and 10 000 pass; the lock-line test needs it above
//! `WEAPONS_LOCKED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS`, 6 000, and does not
//! compile at or under it). Two moved together are untried. Not tunable:
//! [`super::picture::ENGAGED_PULSE_MS`],
//! [`super::picture::FIRE_SLACK_MAX_MS`] (a ruling: one pulse),
//! [`super::picture::FIRE_SLACK_MIN_MS`],
//! [`super::picture::FIRE_SLACK_DOWN_MS`], [`super::lane::FIRE_GAP_MS`],
//! [`super::lane::POLL_STARVE_MS`], and
//! [`super::picture::CYCLE_READ_WITHIN_MS`] and
//! [`super::lane::WEAPONS_AFTER_SHOT_MS`] (both read from the transcript); nor
//! [`SIGHT_BAND_MAX_M`] and [`FALL_BACK_MAX_M`] (a ruling) or
//! [`HUNT_STALE_PLOT_MS`] (a guard: no live row read older than 2 s). Every
//! test asserts through these names, never their values; the one exception
//! is the lead's own bounds on the offline median gaps (1 200 ms for the
//! machine gun, 6 500 for the cannon), which a round must keep.
use std::collections::BTreeSet;

use serde::Serialize;

use super::{
    Side,
    picture::{Contact, EventKind, Phase, Picture, Power, Track, bearing_deg, distance_m},
};

/// "Full ahead" before the tank's own ceilings are read.
pub const UNKNOWN_FULL_KPH: i32 = 40;
/// The speed that asks the tank for its ceilings.
pub const CEILING_ASK_KPH: i32 = 999;
/// A plot older than this is not a fresh one.
pub const FRESH_PLOT_MS: u64 = 3_000;
/// …and a row read longer ago than this is not a current reading of it.
pub const FRESH_READ_MS: u64 = 2_000;
/// A held lock is kept unless another hostile is at least this much nearer.
pub const RETARGET_NEARER_M: u32 = 100;
/// The least time between two `LOCK` orders.
pub const LOCK_EVERY_MS: u64 = 1_500;
/// A `ready` read this soon after an ack may be the pre-shot frame.
pub const READY_AFTER_ACK_MS: u64 = 1_500;
/// The orbit's radial correction: at most this many degrees off the tangent…
pub const ORBIT_CORRECTION_DEG: f64 = 60.0;
/// …reached this many metres off the band.
pub const ORBIT_CORRECTION_SPAN_M: f64 = 150.0;
/// `Close` drives straight in to this range, then orbits there.
pub const CLOSE_M: f64 = 200.0;
/// `StandOff` holds within this of its band.
pub const STAND_OFF_SLACK_M: f64 = 50.0;
/// The orbit reverses after this long without a track to steer by.
pub const FLIP_EVERY_MS: u64 = 20_000;
/// A track is advanced by its plot's age, at most this far.
pub const TRACK_LEAD_MAX_MS: u64 = 3_000;
/// **The widest band the pilot flies** (cut AK.1): the 480 m at which a
/// stopped tank on grass is seen (`help cockpit`, "Seeing"), less a margin for
/// the scope's 50 m rounding and the tank's own drift. The machines stop to
/// shoot ("where it stops, turns onto you and shoots", `help simulator`), so
/// the moving sight's 600 m is the wrong ceiling. A ruling, not a tuning
/// constant: `[arena] band_m` is refused above it.
pub const SIGHT_BAND_MAX_M: u32 = 450;
/// Falling back: the withdrawal's longest length (it ends sooner once the
/// target is at the widened band)…
pub const WITHDRAW_MS: u64 = 10_000;
/// …the band's widening after it…
pub const FALL_BACK_WIDEN_M: u32 = 150;
/// …and the widest band it widens to: the sight's own ceiling (not tunable).
pub const FALL_BACK_MAX_M: u32 = SIGHT_BAND_MAX_M;
/// A plate of the hull at or under `1 / FALL_BACK_FACET_DIVISOR` of its
/// armour falls the pilot back.
pub const FALL_BACK_FACET_DIVISOR: i64 = 4;
/// A `Held:` notice turns the tank this far…
pub const HELD_TURN_DEG: f64 = 45.0;
/// …and holds the new heading this long.
pub const HELD_FOR_MS: u64 = 3_000;
/// A tank still held at 0 kph this long after the stall was last judged is
/// turned further (see the module head for the lag it sits above).
pub const STALL_MS: u64 = 4_000;
/// The back-off's length, astern, when three turns have not freed the tank.
pub const HELD_REVERSE_MS: u64 = 3_000;
/// Astern before `SPEED`'s usage line has read the tank's own ceiling: a slow
/// reverse, well inside any tank's. The ceilings are asked first thing
/// (`SPEED 999`), so it is rarely used.
pub const UNKNOWN_ASTERN_KPH: i32 = 10;
/// With no hostile held this long the pilot searches.
pub const SEARCH_AFTER_MS: u64 = 5_000;
/// Within this of The Crossroads the search drives on past it…
pub const SEARCH_ARRIVE_M: f64 = 100.0;
/// …this far, then turns back.
pub const SEARCH_BEYOND_M: f64 = 800.0;
/// A heading is re-ordered only when it differs by this much…
pub const HEADING_STEP_DEG: u32 = 8;
/// …or by any amount after this long.
pub const HEADING_REFRESH_MS: u64 = 4_000;
/// The spot the search drives for (`help simulator`, "The ground").
pub const SEARCH_SPOT: char = 'B';
/// **A hostile row older than this is not held** for the hunt's purposes: it
/// is hunted rather than orbited round a stale plot, and it no longer counts
/// as a hostile held for the search. A guard, not a tuning constant: in the
/// live smoke no `CONTACTS` row read older than 2 s (a contact goes as
/// `Contact BA lost.`).
pub const HUNT_STALE_PLOT_MS: u64 = 6_000;
/// A hunt starts only from a trail whose plot is at most this old; the course
/// leg runs while it is younger. At its ceiling since the re-measure, which
/// raised it from 45 s after a tank slowed by a lost track took 88 s to reach
/// its plot; the course leg still never ran at 90 s (every hunting tank took
/// 88 to 211 s to reach its plot), so the value was kept on the tie-breakers
/// of three runs (fewer timeouts), not on a measured effect.
pub const HUNT_TRAIL_MS: u64 = 90_000;
/// The estimated speed of a contact lost with no published track.
pub const HUNT_ASSUMED_MPS: f64 = 8.0;
/// The course leg's aim runs at most this far past the last plot.
pub const HUNT_ALONG_MAX_M: f64 = 1_200.0;
/// Within this of a leg's aim the leg is done.
pub const HUNT_ARRIVE_M: f64 = 100.0;
/// The sweep's legs…
pub const HUNT_SWEEP_LEGS: u8 = 4;
/// …each this long…
pub const HUNT_SWEEP_LEG_M: f64 = 300.0;
/// …and the whole sweep at most this long.
pub const HUNT_SWEEP_MS: u64 = 60_000;
/// "The proving ground is 2.56 km square" (`help simulator`, "The ground"),
/// counted from its south-west corner as `STATUS` counts.
pub const ARENA_SIDE_M: f64 = 2_560.0;
/// No hunt aim lies nearer the edge than this.
pub const EDGE_MARGIN_M: f64 = 60.0;

// The doctrine the pilot flies moved to the commander's module (cut AJ.3),
// which parses and clamps the model's answer into it; these names stay here.
pub use super::commander::{Doctrine, Sense, Stance};

/// What to type this tick; everything optional.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Orders {
    pub heading: Option<u32>,
    pub speed: Option<i32>,
    pub lock: Option<String>,
    /// Full weapon ids, main gun first.
    pub fire: Vec<String>,
    /// Ask `BEARING B` (the lane paces it).
    pub ask_spot_b: bool,
}

/// The search's leg.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum SearchLeg {
    /// Driving for The Crossroads.
    ToB,
    /// Past it, on a fixed heading, from `origin`.
    Beyond {
        origin: (f64, f64),
        heading_deg: f64,
    },
}

/// Why the pilot falls back ([`fall_back_due`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum FallBackWhy {
    /// The hull's internals are at half or less ([`hull_failing`]).
    HullHalf,
    /// A plate of the hull is at a quarter (`gone: false`) or gone.
    Plate { facet: String, gone: bool },
}

/// The last hostile the pilot held, as its own scope last plotted it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Trail {
    /// A designator the stream named (a contact row the pilot targeted),
    /// never invented.
    pub designator: String,
    /// The last plot: the contact's last fix, else its row from where the
    /// pilot was.
    pub plot_m: (f64, f64),
    /// When that plot was taken: the row's read less its printed age.
    pub plot_ms: u64,
    /// The picture's published track at the last fresh read.
    pub track: Option<Track>,
    /// Where the pilot was when that plot was read.
    pub seen_from_m: Option<(f64, f64)>,
}

/// The hunt's leg.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum HuntLeg {
    /// To the last plot.
    ToPlot,
    /// Along the estimated course from the last plot, while the plot is young.
    Along,
    /// One leg of the sweep: from `origin` on `heading_deg`.
    Sweep {
        leg: u8,
        origin: (f64, f64),
        heading_deg: f64,
    },
}

/// A hunt for a lost hostile.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hunt {
    pub trail: Trail,
    pub leg: HuntLeg,
    /// When the hunt began.
    pub since_ms: u64,
    /// When the current leg began.
    pub leg_since_ms: u64,
    /// The estimated course and speed, fixed at the start.
    pub course_deg: f64,
    pub speed_mps: f64,
    /// When the sweep began, once it has.
    pub sweep_since_ms: Option<u64>,
}

/// The pilot's own state. One per match.
#[derive(Clone, Debug, Serialize)]
pub struct Pilot {
    /// `+1` counter-clockwise round the target, `-1` clockwise.
    pub sense: i8,
    /// When the current stretch of orbiting began.
    pub orbit_since_ms: Option<u64>,
    /// When the sense last reversed (or the orbit began).
    pub last_flip_ms: u64,
    /// Withdrawing until then (falling back), unless the target reaches the
    /// widened band first.
    pub withdraw_until_ms: Option<u64>,
    /// The pilot has fallen back; its band is widened for the match.
    pub fallen_back: bool,
    /// Holding `held_heading` until then after a `Held:` notice.
    pub held_until_ms: u64,
    pub held_heading: Option<u32>,
    /// The newest `Held:` event already turned for.
    pub held_seen_ms: u64,
    /// The heading the tank was stopped on at the last `Held:` notice.
    pub held_from_deg: Option<u32>,
    /// The stall watchdog: when the stall was last opened or raised…
    pub stall_since_ms: Option<u64>,
    /// …how far it has escalated (1 to 3 turned further, 4 backed off)…
    pub stall_level: u8,
    /// …the heading it holds…
    pub stall_heading: Option<u32>,
    /// …the `STATUS` read (its position's time) it last judged…
    pub stall_status_ms: u64,
    /// …the stall heading kept until then after the tank was freed…
    pub stall_clear_until_ms: u64,
    /// …and backing off until then.
    pub reverse_until_ms: u64,
    pub search: SearchLeg,
    /// The own position at `Power online.` (the first `STATUS` after it).
    pub start_pos: Option<(f64, f64)>,
    /// Where The Crossroads is, once a `STATUS` has named it nearest
    /// ([`super::picture::Spot::plotted_m`]). The search steers by it, which is
    /// the latest `BEARING B` answer made current: the answer is up to 5 s old,
    /// the plotted spot is where the ground is.
    pub spot_b_pos: Option<(f64, f64)>,
    /// When power came online, as the pilot first saw it.
    pub power_ms: Option<u64>,
    /// The last time a hostile row no older than [`HUNT_STALE_PLOT_MS`] was
    /// held.
    pub last_hostile_ms: Option<u64>,
    /// The last hostile held fresh, as last plotted.
    pub trail: Option<Trail>,
    /// The hunt in progress.
    pub hunt: Option<Hunt>,
    /// Every designator the pilot has held fresh this match: a row of one of
    /// them gone stale with no hunt left is not orbited but searched for.
    pub trailed: BTreeSet<String>,
    /// Designators the stream said were destroyed: never trailed or hunted.
    pub wrecks: BTreeSet<String>,
    /// The newest kill event already read.
    pub kill_seen_ms: u64,
    /// The last lock the picture held (an unnamed `Target destroyed.` names
    /// it).
    pub lock_seen: Option<String>,
    /// The last heading ordered, and when.
    pub last_heading_order: Option<(u32, u64)>,
    pub last_speed_order: Option<i32>,
    pub last_lock_order_ms: Option<u64>,
    /// `SPEED 999` has gone out (or the ceilings were known).
    pub ceilings_asked: bool,
    /// What the pilot said since [`Pilot::take_notes`].
    #[serde(skip)]
    notes: Vec<String>,
}

impl Default for Pilot {
    fn default() -> Self {
        Pilot {
            sense: 1,
            orbit_since_ms: None,
            last_flip_ms: 0,
            withdraw_until_ms: None,
            fallen_back: false,
            held_until_ms: 0,
            held_heading: None,
            held_seen_ms: 0,
            held_from_deg: None,
            stall_since_ms: None,
            stall_level: 0,
            stall_heading: None,
            stall_status_ms: 0,
            stall_clear_until_ms: 0,
            reverse_until_ms: 0,
            search: SearchLeg::ToB,
            start_pos: None,
            spot_b_pos: None,
            power_ms: None,
            last_hostile_ms: None,
            trail: None,
            hunt: None,
            trailed: BTreeSet::new(),
            wrecks: BTreeSet::new(),
            kill_seen_ms: 0,
            lock_seen: None,
            last_heading_order: None,
            last_speed_order: None,
            last_lock_order_ms: None,
            ceilings_asked: false,
            notes: Vec::new(),
        }
    }
}

/// Degrees into `0..360`.
fn norm(deg: f64) -> f64 {
    deg.rem_euclid(360.0)
}

/// A heading as `HEADING` takes it.
fn whole(deg: f64) -> u32 {
    // `norm` is in 0..360, so the rounded value is 0..=360 and fits.
    (norm(deg).round() as u32) % 360
}

/// The smaller angle between two whole headings.
fn angle_between(a: u32, b: u32) -> u32 {
    let d = a.abs_diff(b) % 360;
    d.min(360 - d)
}

/// `pos` moved `metres` along `bearing`.
fn along(pos: (f64, f64), bearing: f64, metres: f64) -> (f64, f64) {
    let r = bearing.to_radians();
    (pos.0 + metres * r.sin(), pos.1 + metres * r.cos())
}

/// **Never past the edge**: a point clamped into the proving ground, at least
/// [`EDGE_MARGIN_M`] inside each side of its [`ARENA_SIDE_M`] square.
pub fn clamp_arena(p: (f64, f64)) -> (f64, f64) {
    let (lo, hi) = (EDGE_MARGIN_M, ARENA_SIDE_M - EDGE_MARGIN_M);
    (p.0.clamp(lo, hi), p.1.clamp(lo, hi))
}

/// Whether [`clamp_arena`] moves `p`.
fn past_the_edge(p: (f64, f64)) -> bool {
    let inside = EDGE_MARGIN_M..=ARENA_SIDE_M - EDGE_MARGIN_M;
    !inside.contains(&p.0) || !inside.contains(&p.1)
}

/// **The hull is failing**: the `hull` section's internals `(left, max)` are
/// at half or less (`max > 0 && left * 2 <= max`). The fall back's first rule,
/// and the one a commander's `withdraw` is kept on (cut AK.2).
pub fn hull_failing(picture: &Picture) -> bool {
    picture
        .own
        .sections
        .get("hull")
        .and_then(|section| section.internals)
        .is_some_and(|(left, max)| max > 0 && left.saturating_mul(2) <= max)
}

/// **Whether the pilot falls back, and why**: [`hull_failing`], or else the
/// first facet **of the `hull` section only** whose plate is gone (`left ==
/// 0`, which also reads a `(0, 0)` stored by an `armour gone` line on a facet
/// never read) or at a quarter (`max > 0 && left * FALL_BACK_FACET_DIVISOR <=
/// max`). A turret's or a track's facet never falls it back.
pub fn fall_back_due(picture: &Picture) -> Option<FallBackWhy> {
    if hull_failing(picture) {
        return Some(FallBackWhy::HullHalf);
    }
    let hull = picture.own.sections.get("hull")?;
    hull.facets.iter().find_map(|(facet, (left, max))| {
        let gone = *left == 0;
        (gone || (*max > 0 && left.saturating_mul(FALL_BACK_FACET_DIVISOR) <= *max)).then(|| {
            FallBackWhy::Plate {
                facet: facet.clone(),
                gone,
            }
        })
    })
}

/// **The widened band** after a fall back: [`FALL_BACK_WIDEN_M`] wider than
/// `band_m`, never past [`FALL_BACK_MAX_M`], and never narrower than `band_m`.
pub fn widened_band(band_m: u32) -> u32 {
    band_m.max(
        band_m
            .saturating_add(FALL_BACK_WIDEN_M)
            .min(FALL_BACK_MAX_M),
    )
}

/// A leg's aim, clamped into the ground, and whether the clamp moved it.
fn leg_aim(hunt: &Hunt, now_ms: u64) -> ((f64, f64), bool) {
    let raw = match &hunt.leg {
        HuntLeg::ToPlot => hunt.trail.plot_m,
        HuntLeg::Along => {
            let run_m = hunt.speed_mps * now_ms.saturating_sub(hunt.trail.plot_ms) as f64 / 1_000.0;
            along(
                hunt.trail.plot_m,
                hunt.course_deg,
                run_m.min(HUNT_ALONG_MAX_M),
            )
        }
        HuntLeg::Sweep {
            origin,
            heading_deg,
            ..
        } => along(*origin, *heading_deg, HUNT_SWEEP_LEG_M),
    };
    (clamp_arena(raw), past_the_edge(raw))
}

/// **How old a hostile's plot may be and still be fought** rather than hunted:
/// [`HUNT_STALE_PLOT_MS`], or under a `hunt` doctrine (cut AK.2) only a fresh
/// plot, [`FRESH_PLOT_MS`] — a commander that says hunt wants the last one
/// lost hunted at once, not a row a few seconds old circled.
fn held_limit_ms(doctrine: &Doctrine) -> u64 {
    if doctrine.stance == Stance::Hunt {
        FRESH_PLOT_MS
    } else {
        HUNT_STALE_PLOT_MS
    }
}

/// Whether a weapon is believed ready now: its one clock
/// ([`super::picture::Weapon::next_ready_ms`]) when anything has set it, else
/// a `ready` row read well after the last ack.
fn believed_ready(weapon: &super::picture::Weapon, now_ms: u64) -> bool {
    match weapon.next_ready_ms {
        Some(at) => now_ms >= at,
        None => {
            weapon.ready
                && weapon.read_ms > 0
                && weapon
                    .fired_ms
                    .is_none_or(|fired| weapon.read_ms > fired.saturating_add(READY_AFTER_ACK_MS))
        }
    }
}

impl Pilot {
    pub fn new() -> Pilot {
        Pilot::default()
    }

    /// **Decide.** What to type now, given the picture and the doctrine.
    pub fn decide(&mut self, picture: &Picture, doctrine: &Doctrine, now_ms: u64) -> Orders {
        let mut orders = Orders::default();
        // Rule 1: nothing unless flying a powered tank in a running match, and
        // nothing ever once it is destroyed.
        if picture.own.destroyed
            || !matches!(picture.phase, Phase::Running { .. })
            || !picture.aboard
            || picture.own.power != Power::Online
        {
            return orders;
        }
        let Some(side) = picture.side else {
            return orders;
        };
        let other = side.other();
        // The doctrine's own sense, when it names one, is `s` everywhere — the
        // orbit, a `Held:` turn, the stall and the sweep alike; `Auto` keeps
        // the pilot's running one.
        match doctrine.sense {
            Sense::Cw => self.sense = -1,
            Sense::Ccw => self.sense = 1,
            Sense::Auto => {}
        }
        self.observe(picture, doctrine, other, now_ms);

        // Rule 3: the target.
        let target = self.target(picture, doctrine, other);

        // Rule 4: the lock.
        if let Some(target) = &target
            && picture.lock.as_deref() != Some(target.as_str())
            && self
                .last_lock_order_ms
                .is_none_or(|at| now_ms >= at.saturating_add(LOCK_EVERY_MS))
        {
            orders.lock = Some(target.clone());
        }

        // Rule 5: fire.
        if let Some(target) = &target {
            orders.fire = fireable(picture, target, other, now_ms);
        }

        // Rules 2 and 6: the speed ceilings, then movement.
        if picture.own.immobile || picture.own.disabled {
            return orders;
        }
        if picture.own.max_ahead_kph.is_some() {
            self.ceilings_asked = true;
        }
        self.follow(picture, doctrine, other, target.as_deref(), now_ms);
        let full = picture.own.max_ahead_kph.unwrap_or(UNKNOWN_FULL_KPH);
        let motion = self.motion(picture, doctrine, target.as_deref(), full, now_ms);
        let (heading, mut speed, ask) = motion;
        orders.ask_spot_b = ask;
        if !self.ceilings_asked {
            speed = Some(CEILING_ASK_KPH);
        }
        if let Some(deg) = heading {
            let deg = whole(deg);
            let due = match self.last_heading_order {
                None => true,
                Some((last, at)) => {
                    let diff = angle_between(deg, last);
                    diff >= HEADING_STEP_DEG
                        || (diff > 0 && now_ms >= at.saturating_add(HEADING_REFRESH_MS))
                }
            };
            if due {
                orders.heading = Some(deg);
            }
        }
        if let Some(kph) = speed
            && self.last_speed_order != Some(kph)
        {
            orders.speed = Some(kph);
        }
        orders
    }

    /// The lane typed `HEADING deg`.
    pub fn sent_heading(&mut self, deg: u32, now_ms: u64) {
        self.last_heading_order = Some((deg, now_ms));
    }

    /// The lane typed `SPEED kph`.
    pub fn sent_speed(&mut self, kph: i32) {
        self.last_speed_order = Some(kph);
        if kph == CEILING_ASK_KPH {
            self.ceilings_asked = true;
        }
    }

    /// The lane typed a `LOCK`.
    pub fn sent_lock(&mut self, now_ms: u64) {
        self.last_lock_order_ms = Some(now_ms);
    }

    /// A heading order went unanswered: order it again when re-decided.
    pub fn forget_heading(&mut self) {
        self.last_heading_order = None;
    }

    /// A speed order went unanswered: order it again when re-decided (the
    /// ceilings too, if that is what was asked).
    pub fn forget_speed(&mut self) {
        if self.last_speed_order == Some(CEILING_ASK_KPH) {
            self.ceilings_asked = false;
        }
        self.last_speed_order = None;
    }

    /// **What the pilot said** since the last call, oldest first, verbatim
    /// (the lane hands each to the runner as an `arena` event): a hunt's
    /// start, legs and end, a fall back and its reason, a held tank turned
    /// further or backed off. Designators in them are ones the stream named.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    // -- internals -------------------------------------------------------------

    fn note(&mut self, text: String) {
        self.notes.push(text);
    }

    /// Fold in what the picture has learned since the last decision: when power
    /// came up, where the pilot started, where The Crossroads is, whether a
    /// hostile is held, a `Held:` notice, the stall watchdog, and the fall-back
    /// rule.
    fn observe(&mut self, picture: &Picture, doctrine: &Doctrine, other: Side, now_ms: u64) {
        let power_ms = *self.power_ms.get_or_insert(now_ms);
        if self.last_hostile_ms.is_none() {
            self.last_hostile_ms = Some(power_ms);
        }
        // Events from before this match's power came up are not this
        // pilot's (the picture's ring outlives a match).
        self.held_seen_ms = self.held_seen_ms.max(power_ms);
        self.kill_seen_ms = self.kill_seen_ms.max(power_ms);
        if self.start_pos.is_none()
            && let Some(pos) = picture.own.pos_m
            && picture.own.pos_read_ms >= power_ms
        {
            self.start_pos = Some(pos);
        }
        // The Crossroads' own position, as the picture plotted it from a
        // `STATUS` block that named it nearest (range and bearing from the
        // block's own position; a `BEARING B` answer never moves it).
        if let Some((x, y)) = picture
            .spots
            .get(&SEARCH_SPOT)
            .and_then(|spot| spot.plotted_m)
        {
            self.spot_b_pos = Some((x as f64, y as f64));
        }
        // A kill, before anything reads the contacts: its wreck is no hostile.
        self.take_kills(picture);
        // Only a row young enough to be held counts (cut AK.1): a stale row
        // still listed after a failed hunt does not keep the search away, and
        // nor does a wreck's row, listed until it expires.
        if picture
            .contacts
            .values()
            .any(|c| self.live_hostile(c, other) && c.age_ms <= HUNT_STALE_PLOT_MS)
        {
            self.last_hostile_ms = Some(now_ms);
        }
        if let Some(held) = picture
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Held && event.at_ms > self.held_seen_ms)
            .map(|event| event.at_ms)
            .max()
        {
            self.held_seen_ms = held;
            // The heading the tank was stopped on: every further turn is
            // measured from it, never from a later reading.
            self.held_from_deg = Some(picture.own.heading_deg);
            self.held_heading = Some(whole(
                f64::from(picture.own.heading_deg) + HELD_TURN_DEG * f64::from(self.sense),
            ));
            self.held_until_ms = now_ms.saturating_add(HELD_FOR_MS);
            // Insurance, not the fix: the speed goes again straight after the
            // turn (no transcript shows a `SPEED` freeing a held tank).
            self.forget_speed();
            // A new stop is a new episode: any stall starts again from level
            // 0, measured from this stop's heading.
            self.stall_since_ms = None;
            self.stall_level = 0;
            self.stall_heading = None;
            self.stall_clear_until_ms = 0;
        }
        // The back-off is over: the forward speed goes again, and the stall
        // closes at level 0.
        if self.reverse_until_ms > 0 && now_ms >= self.reverse_until_ms {
            self.reverse_until_ms = 0;
            self.forget_speed();
            self.stall_level = 0;
            self.stall_since_ms = None;
        }
        self.watch_stall(picture, now_ms);
        if !self.fallen_back
            && let Some(why) = fall_back_due(picture)
        {
            self.fallen_back = true;
            self.withdraw_until_ms = Some(now_ms.saturating_add(WITHDRAW_MS));
            let band = widened_band(doctrine.band_m);
            self.note(match why {
                FallBackWhy::HullHalf => format!("Falling back to {band} m: the hull is at half."),
                FallBackWhy::Plate { facet, gone: false } => {
                    format!("Falling back to {band} m: the hull's {facet} plate is at a quarter.")
                }
                FallBackWhy::Plate { facet, gone: true } => {
                    format!("Falling back to {band} m: the hull's {facet} plate is gone.")
                }
            });
        }
    }

    /// **The stall watchdog**, judged once per new `STATUS` read. Held at
    /// 0 kph after the held turn (and any back-off) opens a stall; still held
    /// [`STALL_MS`] or more after it was opened or last raised raises it: levels
    /// 1 to 3 turn to 90°, 135° and 180° from the heading the tank was stopped
    /// on, level 4 backs off for [`HELD_REVERSE_MS`] and closes it at level 0.
    /// A read that is not held at 0 kph closes it, keeping a turned heading
    /// [`HELD_FOR_MS`] more to clear the ground.
    fn watch_stall(&mut self, picture: &Picture, now_ms: u64) {
        if picture.own.pos_read_ms <= self.stall_status_ms {
            return;
        }
        self.stall_status_ms = picture.own.pos_read_ms;
        let held = picture.own.held.is_some() && picture.own.speed_kph == 0;
        if !held {
            if self.stall_level >= 1 && self.stall_heading.is_some() {
                self.stall_clear_until_ms = now_ms.saturating_add(HELD_FOR_MS);
            }
            // Free on a read taken after the held turn (a read inside it may
            // be a frame from before the stop — "you and the arena are a pulse
            // apart"): the episode is over, and a later stall with no notice
            // read is measured from its own heading, not this one's.
            if picture.own.pos_read_ms >= self.held_until_ms {
                self.held_from_deg = None;
            }
            self.stall_since_ms = None;
            self.stall_level = 0;
            return;
        }
        match self.stall_since_ms {
            None => {
                if now_ms >= self.held_until_ms && now_ms >= self.reverse_until_ms {
                    self.stall_since_ms = Some(now_ms);
                    // Held with no notice read (a re-board, say): the heading
                    // it reads now is the one it was stopped on.
                    self.held_from_deg.get_or_insert(picture.own.heading_deg);
                }
            }
            Some(since) if now_ms >= since.saturating_add(STALL_MS) => {
                self.stall_level = self.stall_level.saturating_add(1);
                self.stall_since_ms = Some(now_ms);
                self.forget_speed();
                if self.stall_level <= 3 {
                    let from = self.held_from_deg.unwrap_or(picture.own.heading_deg);
                    let deg = whole(
                        f64::from(from)
                            + HELD_TURN_DEG
                                * f64::from(self.stall_level + 1)
                                * f64::from(self.sense),
                    );
                    self.stall_heading = Some(deg);
                    self.forget_heading();
                    self.note(format!("Held at 0 kph: turning to {deg:03}°."));
                } else {
                    // Level 4 stands while the tank backs off; the back-off's
                    // end closes the stall (in `observe`).
                    self.reverse_until_ms = now_ms.saturating_add(HELD_REVERSE_MS);
                    self.stall_heading = None;
                    self.stall_since_ms = None;
                    self.note("Held at 0 kph: backing off.".to_owned());
                }
            }
            Some(_) => {}
        }
    }

    /// Rule 3: the doctrine's target when it is a held hostile young enough to
    /// be fought ([`held_limit_ms`]), or when no other hostile is; else the
    /// nearest fresh hostile, keeping the lock's contact unless another is at
    /// least 100 m nearer. Never an unidentified contact, never a friend, never
    /// a wreck (cut AK.1: its row stays listed until it expires, and the pilot
    /// neither orbits nor shoots it). A doctrine naming a row gone stale while
    /// another hostile is in sight (cut AK.2) would otherwise leave the pilot
    /// with nothing to fly at: too old to fight, its trail not hunted while
    /// the other is held, and the search kept away by the other.
    fn target(&self, picture: &Picture, doctrine: &Doctrine, other: Side) -> Option<String> {
        let young =
            |c: &Contact| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine);
        if let Some(wanted) = &doctrine.target
            && let Some(contact) = picture.contacts.get(wanted)
            && self.live_hostile(contact, other)
            && (young(contact) || !picture.contacts.values().any(young))
        {
            return Some(wanted.clone());
        }
        let best = picture
            .contacts
            .values()
            .filter(|c| self.live_hostile(c, other))
            .min_by_key(|c| (c.age_ms > FRESH_PLOT_MS, c.range_m.unwrap_or(u32::MAX)))?;
        if let Some(lock) = &picture.lock
            && !picture.lock_friendly
            && let Some(held) = picture.contacts.get(lock)
            && self.live_hostile(held, other)
        {
            let held_range = held.range_m.unwrap_or(u32::MAX);
            let best_range = best.range_m.unwrap_or(u32::MAX);
            if best_range.saturating_add(RETARGET_NEARER_M) > held_range {
                return Some(lock.clone());
            }
        }
        Some(best.designator.clone())
    }

    /// A contact of the other side that is not a wreck this match has seen
    /// destroyed.
    fn live_hostile(&self, contact: &Contact, other: Side) -> bool {
        contact.side == Some(other) && !self.wrecks.contains(&contact.designator)
    }

    /// **The kills** since the last decision, `{d} destroyed.` or `Target
    /// destroyed.` of the lock: each wreck is remembered for the match (never
    /// targeted, trailed or hunted again), and a kill of the trail's hostile
    /// drops the trail and ends any hunt.
    fn take_kills(&mut self, picture: &Picture) {
        let lock = picture.lock.clone().or_else(|| self.lock_seen.clone());
        let kills: Vec<(u64, String)> = picture
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Kill && event.at_ms > self.kill_seen_ms)
            .map(|event| (event.at_ms, event.text.clone()))
            .collect();
        for (at_ms, text) in kills {
            self.kill_seen_ms = self.kill_seen_ms.max(at_ms);
            let wreck = if text == "Target destroyed." {
                lock.clone()
            } else {
                text.strip_suffix(" destroyed.").map(str::to_owned)
            };
            let Some(wreck) = wreck else {
                continue;
            };
            if self
                .trail
                .as_ref()
                .is_some_and(|trail| trail.designator == wreck)
            {
                self.trail = None;
                if let Some(hunt) = self.hunt.take() {
                    let d = hunt.trail.designator;
                    self.note(format!("The hunt for {d} ends; searching."));
                }
            }
            self.wrecks.insert(wreck);
        }
        if picture.lock.is_some() {
            self.lock_seen = picture.lock.clone();
        }
    }

    /// **The hunt's bookkeeping**, every decision: a fresh hostile target
    /// refreshes the trail; a fresh hostile ends a hunt; a `Held:` notice ends
    /// its leg; and with no hostile held young, a live trail and no withdrawal
    /// flying, a hunt starts at the last plot. An unidentified contact neither
    /// starts nor ends one, and nor does a wreck (the kills are taken in
    /// `observe`).
    fn follow(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        target: Option<&str>,
        now_ms: u64,
    ) {
        // The trail: the target's last plot while that plot is fresh.
        if let Some(designator) = target
            && !self.wrecks.contains(designator)
            && let Some(contact) = picture.contacts.get(designator)
            && contact.side == Some(other)
            && contact.age_ms <= FRESH_PLOT_MS
            && let Some(plot_m) = last_plot(picture, contact)
        {
            self.trail = Some(Trail {
                designator: designator.to_owned(),
                plot_m,
                plot_ms: contact.read_ms.saturating_sub(contact.age_ms),
                track: contact.track.clone(),
                seen_from_m: picture.own_pos_at(contact.read_ms),
            });
            self.trailed.insert(designator.to_owned());
        }
        // A fresh hostile ends the hunt: the pilot engages it as ever.
        if let Some(hunt) = &self.hunt {
            let fresh = |c: &Contact| self.live_hostile(c, other) && c.age_ms <= FRESH_PLOT_MS;
            let seen = target
                .filter(|t| picture.contacts.get(*t).is_some_and(fresh))
                .map(str::to_owned)
                .or_else(|| {
                    picture
                        .contacts
                        .values()
                        .find(|c| fresh(c))
                        .map(|c| c.designator.clone())
                });
            if let Some(seen) = seen {
                let d = hunt.trail.designator.clone();
                self.hunt = None;
                self.note(format!("The hunt for {d} ends: {seen} is in sight."));
            }
        }
        let own = picture.own_pos_at(now_ms);
        // A `Held:` notice newer than the leg ends the leg (the held turn
        // still happens).
        if let Some(hunt) = &self.hunt
            && self.held_seen_ms > hunt.leg_since_ms
            && let Some(own) = own
        {
            self.next_leg(own, now_ms);
        }
        // The start. A `hunt` doctrine (cut AK.2) hunts as soon as no plot is
        // fresh, rather than flying round a row a few seconds old.
        let held_young = picture
            .contacts
            .values()
            .any(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine));
        let withdrawing = self.withdraw_until_ms.is_some_and(|until| now_ms < until)
            || doctrine.stance == Stance::Withdraw;
        if self.hunt.is_none()
            && !held_young
            && !withdrawing
            && let Some(trail) = &self.trail
            && now_ms.saturating_sub(trail.plot_ms) <= HUNT_TRAIL_MS
            && let Some(own) = own
        {
            let trail = trail.clone();
            // The estimated course, fixed now: the track's; else the way the
            // range was opening when it was lost; else the own heading.
            let (course_deg, speed_mps) = match &trail.track {
                Some(track) => (track.course_deg, track.speed_mps),
                None => match trail.seen_from_m {
                    Some(from) if distance_m(from, trail.plot_m) > 1.0 => {
                        (bearing_deg(from, trail.plot_m), HUNT_ASSUMED_MPS)
                    }
                    _ => (f64::from(picture.own.heading_deg), HUNT_ASSUMED_MPS),
                },
            };
            let away = distance_m(own, clamp_arena(trail.plot_m)).round();
            self.note(format!(
                "Hunting {}: its last plot, {away:.0} m away.",
                trail.designator
            ));
            self.hunt = Some(Hunt {
                trail,
                leg: HuntLeg::ToPlot,
                since_ms: now_ms,
                leg_since_ms: now_ms,
                course_deg,
                speed_mps,
                sweep_since_ms: None,
            });
        }
    }

    /// The hunt's next leg: the plot → along the course → the sweep's legs, a
    /// quarter turn by the pilot's sense each → the end (the trail dropped,
    /// the search next).
    fn next_leg(&mut self, own: (f64, f64), now_ms: u64) {
        let sense = f64::from(self.sense);
        let Some(hunt) = self.hunt.as_mut() else {
            return;
        };
        let d = hunt.trail.designator.clone();
        let next = match &hunt.leg {
            HuntLeg::ToPlot => {
                let course = whole(hunt.course_deg);
                self.notes.push(format!(
                    "Hunting {d}: along {course:03}° from its last plot."
                ));
                Some(HuntLeg::Along)
            }
            HuntLeg::Along => {
                hunt.sweep_since_ms = Some(now_ms);
                self.notes.push(format!("Sweeping for {d}."));
                Some(HuntLeg::Sweep {
                    leg: 0,
                    origin: clamp_arena(own),
                    heading_deg: norm(hunt.course_deg + 90.0 * sense),
                })
            }
            HuntLeg::Sweep {
                leg, heading_deg, ..
            } => (leg.saturating_add(1) < HUNT_SWEEP_LEGS).then(|| HuntLeg::Sweep {
                leg: leg.saturating_add(1),
                origin: clamp_arena(own),
                heading_deg: norm(heading_deg + 90.0 * sense),
            }),
        };
        match next {
            Some(leg) => {
                hunt.leg = leg;
                hunt.leg_since_ms = now_ms;
            }
            None => self.end_hunt(),
        }
    }

    /// The hunt is over with nothing found: the trail goes, the search follows.
    fn end_hunt(&mut self) {
        if let Some(hunt) = self.hunt.take() {
            self.trail = None;
            let d = hunt.trail.designator;
            self.note(format!("The hunt for {d} ends; searching."));
        }
    }

    /// The hunt's heading now, advancing its legs as they are done; `None`
    /// once it has ended.
    fn hunt_heading(&mut self, own: (f64, f64), now_ms: u64) -> Option<f64> {
        // Each pass either returns or moves the hunt one leg on (or ends it).
        for _ in 0..=usize::from(HUNT_SWEEP_LEGS) + 2 {
            let hunt = self.hunt.as_ref()?;
            match hunt.leg {
                HuntLeg::Along if now_ms.saturating_sub(hunt.trail.plot_ms) > HUNT_TRAIL_MS => {
                    self.next_leg(own, now_ms);
                    continue;
                }
                HuntLeg::Sweep { .. }
                    if hunt
                        .sweep_since_ms
                        .is_some_and(|since| now_ms >= since.saturating_add(HUNT_SWEEP_MS)) =>
                {
                    self.end_hunt();
                    return None;
                }
                _ => {}
            }
            let (aim, clamped) = leg_aim(hunt, now_ms);
            let d = distance_m(own, aim);
            let along_leg = hunt.leg == HuntLeg::Along;
            // The course leg's aim moves; it is done only where the clamp
            // stopped it (the edge).
            if d <= HUNT_ARRIVE_M && (!along_leg || clamped) {
                self.next_leg(own, now_ms);
                continue;
            }
            if along_leg && !clamped {
                let r = hunt.course_deg.to_radians();
                let (dx, dy) = (own.0 - aim.0, own.1 - aim.1);
                let ahead = dx * r.sin() + dy * r.cos();
                let aside = (dx * r.cos() - dy * r.sin()).abs();
                // Past the moving aim by more than the arrival: the estimate
                // is behind, and the course leg is done.
                if ahead > HUNT_ARRIVE_M && aside <= HUNT_ARRIVE_M {
                    self.next_leg(own, now_ms);
                    continue;
                }
                // Caught up with it, or level with it and at most the arrival
                // ahead: drive on along the course rather than turn back to it
                // — never past the edge (the course's own next
                // `HUNT_ARRIVE_M` would cross it: that is the edge reached).
                if d <= HUNT_ARRIVE_M || (ahead >= 0.0 && aside <= HUNT_ARRIVE_M) {
                    if past_the_edge(along(own, hunt.course_deg, HUNT_ARRIVE_M)) {
                        self.next_leg(own, now_ms);
                        continue;
                    }
                    return Some(hunt.course_deg);
                }
            }
            return Some(bearing_deg(own, aim));
        }
        None
    }

    /// Rule 6: `(heading, speed, ask the bearing to B)`, by the one stance
    /// precedence of the module head.
    fn motion(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        let Some(own) = picture.own_pos_at(now_ms) else {
            return (None, None, false);
        };
        // 1. A `Held:` notice outranks everything for its three seconds; then
        // the back-off; then a stall's turn while it is open, and after it
        // frees the tank until the ground is cleared.
        if now_ms < self.held_until_ms
            && let Some(held) = self.held_heading
        {
            return (Some(f64::from(held)), Some(full), false);
        }
        if now_ms < self.reverse_until_ms {
            let astern = picture.own.max_astern_kph.unwrap_or(UNKNOWN_ASTERN_KPH);
            return (None, Some(-astern), false);
        }
        if let Some(stall) = self.stall_heading {
            if self.stall_level >= 1 || now_ms < self.stall_clear_until_ms {
                return (Some(f64::from(stall)), Some(full), false);
            }
            self.stall_heading = None;
        }
        let contact = target.and_then(|t| picture.contacts.get(t));
        let aim = contact.and_then(|c| target_pos(picture, c, now_ms));
        let away = |aim: Option<(f64, f64)>| match aim {
            Some(aim) => (Some(bearing_deg(own, aim) + 180.0), Some(full), false),
            None => (None, Some(full), false),
        };
        // 2. The pilot's own fall-back withdrawal: until its time is up or the
        // target is out at the widened band.
        if let Some(until) = self.withdraw_until_ms {
            let widened = f64::from(widened_band(doctrine.band_m));
            let opened = aim.is_some_and(|aim| distance_m(own, aim) >= widened);
            if now_ms >= until || opened {
                self.withdraw_until_ms = None;
            } else {
                self.orbit_since_ms = None;
                return away(aim);
            }
        }
        // 3. A doctrine `withdraw`, ahead of the widened band.
        if doctrine.stance == Stance::Withdraw {
            self.orbit_since_ms = None;
            return away(aim);
        }
        // 4. A hostile held young: the doctrine's stance (a `hunt` doctrine's
        // only while its plot is fresh).
        if let (Some(contact), Some(aim)) = (contact, aim)
            && contact.age_ms <= held_limit_ms(doctrine)
        {
            return self.engage(doctrine, contact, aim, own, full, now_ms);
        }
        // 5. The hunt, before any stance round a stale row.
        if self.hunt.is_some()
            && let Some(heading) = self.hunt_heading(own, now_ms)
        {
            self.orbit_since_ms = None;
            return (Some(heading), Some(full), false);
        }
        // 4, for an older row the pilot never held fresh (nothing to hunt).
        if let (Some(contact), Some(aim)) = (contact, aim)
            && !self.trailed.contains(&contact.designator)
        {
            return self.engage(doctrine, contact, aim, own, full, now_ms);
        }
        // 6. Nothing to fly at: the search, after five quiet seconds. A stale
        // row whose hunt is spent is nothing to fly at.
        self.orbit_since_ms = None;
        let spent = contact.is_some_and(|c| self.trailed.contains(&c.designator));
        let quiet_since = self.last_hostile_ms.unwrap_or(now_ms);
        if (target.is_none() || spent) && now_ms >= quiet_since.saturating_add(SEARCH_AFTER_MS) {
            return self.search(picture, own, full);
        }
        (None, None, false)
    }

    /// The doctrine's stance round a held hostile: after the fall back at the
    /// widened band, and `close` flown as `orbit` there.
    fn engage(
        &mut self,
        doctrine: &Doctrine,
        contact: &Contact,
        aim: (f64, f64),
        own: (f64, f64),
        full: i32,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        // A hostile is held: the next search starts over.
        self.search = SearchLeg::ToB;
        let (stance, band) = if self.fallen_back {
            let stance = match doctrine.stance {
                Stance::Close => Stance::Orbit,
                stance => stance,
            };
            (stance, f64::from(widened_band(doctrine.band_m)))
        } else {
            (doctrine.stance, f64::from(doctrine.band_m))
        };
        let d = distance_m(own, aim);
        let beta = bearing_deg(own, aim);
        match stance {
            Stance::Orbit => (
                Some(self.orbit(doctrine, contact, aim, own, d, beta, band, now_ms)),
                Some(full),
                false,
            ),
            Stance::Close if d <= CLOSE_M => (
                Some(self.orbit(doctrine, contact, aim, own, d, beta, CLOSE_M, now_ms)),
                Some(full),
                false,
            ),
            Stance::Close => {
                self.orbit_since_ms = None;
                (Some(beta), Some(full), false)
            }
            Stance::StandOff => {
                self.orbit_since_ms = None;
                if d > band + STAND_OFF_SLACK_M {
                    (Some(beta), Some(full), false)
                } else if d < band - STAND_OFF_SLACK_M {
                    (Some(beta + 180.0), Some(full), false)
                } else {
                    (Some(beta), Some(0), false)
                }
            }
            // Hunt (cut AK.2): straight at it until the band, then round it
            // there.
            Stance::Hunt if d > band + STAND_OFF_SLACK_M => {
                self.orbit_since_ms = None;
                (Some(beta), Some(full), false)
            }
            Stance::Hunt => (
                Some(self.orbit(doctrine, contact, aim, own, d, beta, band, now_ms)),
                Some(full),
                false,
            ),
            Stance::Withdraw => {
                self.orbit_since_ms = None;
                (Some(beta + 180.0), Some(full), false)
            }
        }
    }

    /// The orbit's heading: the tangent at the band, bent up to 60° toward the
    /// target when outside it and away when inside, reached 150 m off the band.
    #[allow(clippy::too_many_arguments)]
    fn orbit(
        &mut self,
        doctrine: &Doctrine,
        contact: &Contact,
        aim: (f64, f64),
        own: (f64, f64),
        d: f64,
        beta: f64,
        band: f64,
        now_ms: u64,
    ) -> f64 {
        if self.orbit_since_ms.is_none() {
            self.orbit_since_ms = Some(now_ms);
            self.last_flip_ms = now_ms;
        }
        let s = match doctrine.sense {
            Sense::Cw => {
                self.sense = -1;
                -1.0
            }
            Sense::Ccw => {
                self.sense = 1;
                1.0
            }
            Sense::Auto => match &contact.track {
                // Toward the rear: clockwise when the rear lies clockwise of the
                // pilot as the target sees it. No timed flip while a track holds.
                Some(track) => {
                    let theta = norm(track.course_deg + 180.0 - bearing_deg(aim, own));
                    self.sense = if theta > 0.0 && theta < 180.0 { -1 } else { 1 };
                    self.last_flip_ms = now_ms;
                    f64::from(self.sense)
                }
                None => {
                    if now_ms >= self.last_flip_ms.saturating_add(FLIP_EVERY_MS) {
                        self.sense = -self.sense;
                        self.last_flip_ms = now_ms;
                    }
                    f64::from(self.sense)
                }
            },
        };
        let offset = ((d - band) / ORBIT_CORRECTION_SPAN_M).clamp(-1.0, 1.0) * ORBIT_CORRECTION_DEG;
        beta + s * (90.0 - offset)
    }

    /// The search: for The Crossroads, past it 800 m along the start's line
    /// through it, then back.
    fn search(
        &mut self,
        picture: &Picture,
        own: (f64, f64),
        full: i32,
    ) -> (Option<f64>, Option<i32>, bool) {
        if let SearchLeg::Beyond {
            origin,
            heading_deg,
        } = self.search
        {
            if distance_m(origin, own) < SEARCH_BEYOND_M {
                return (Some(heading_deg), Some(full), true);
            }
            self.search = SearchLeg::ToB;
        }
        if let Some(spot) = self.spot_b_pos
            && distance_m(own, spot) <= SEARCH_ARRIVE_M
        {
            let heading = match self.start_pos {
                Some(start) if distance_m(start, spot) > 1.0 => bearing_deg(start, spot),
                _ => f64::from(picture.own.heading_deg),
            };
            self.search = SearchLeg::Beyond {
                origin: own,
                heading_deg: heading,
            };
            return (Some(heading), Some(full), true);
        }
        let heading = match self.spot_b_pos {
            Some(spot) => Some(bearing_deg(own, spot)),
            None => picture
                .spots
                .get(&SEARCH_SPOT)
                .and_then(|spot| spot.bearing_deg)
                .map(f64::from),
        };
        (heading, Some(full), true)
    }
}

/// A contact's last plot: its last fix, else its row's range and bearing from
/// where the pilot was when the row was read.
fn last_plot(picture: &Picture, contact: &Contact) -> Option<(f64, f64)> {
    match contact.fixes.back() {
        Some(fix) => Some(fix.pos_m),
        None => Some(along(
            picture.own_pos_at(contact.read_ms)?,
            f64::from(contact.bearing_deg),
            f64::from(contact.range_m?),
        )),
    }
}

/// Where the target is now, as best the picture says: its last plot (a fix, or
/// the row's range and bearing from where the pilot was), advanced along a
/// published track by the plot's age, at most 3 s.
pub fn target_pos(picture: &Picture, contact: &Contact, now_ms: u64) -> Option<(f64, f64)> {
    let plot = last_plot(picture, contact)?;
    Some(match &contact.track {
        Some(track) => {
            let age_ms = contact
                .age_ms
                .saturating_add(now_ms.saturating_sub(contact.read_ms))
                .min(TRACK_LEAD_MAX_MS);
            along(
                plot,
                track.course_deg,
                track.speed_mps * age_ms as f64 / 1_000.0,
            )
        }
        None => plot,
    })
}

/// Rule 5: the guns to fire at `target`, main gun first. None while the
/// picture holds every gun in doubt (a lost section's `its weapons are gone`,
/// cut AL.1), and never a gun whose counted rounds are gone or one the tank
/// said it does not carry.
fn fireable(picture: &Picture, target: &str, other: Side, now_ms: u64) -> Vec<String> {
    if picture.lock.as_deref() != Some(target)
        || picture.lock_friendly
        || picture.weapons_doubt.is_some()
    {
        return Vec::new();
    }
    let Some(contact) = picture.contacts.get(target) else {
        return Vec::new();
    };
    if contact.side != Some(other)
        || contact.age_ms > FRESH_PLOT_MS
        || now_ms.saturating_sub(contact.read_ms) > FRESH_READ_MS
    {
        return Vec::new();
    }
    let main = picture
        .weapons
        .iter()
        .position(|w| w.id == "cannon")
        .unwrap_or(0);
    let order = std::iter::once(main).chain((0..picture.weapons.len()).filter(|i| *i != main));
    order
        .filter_map(|index| picture.weapons.get(index))
        .filter(|w| {
            !picture.struck.contains(&w.id)
                && w.rounds_left() > 0
                && w.percent != Some(0)
                && believed_ready(w, now_ms)
        })
        .map(|w| w.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::arena::{
        classify,
        picture::{Event, Fix, Section, Track, Weapon},
        settings::Settings,
    };

    const NOW: u64 = 100_000;

    /// A red tank at the origin, powered and aboard in a running match, heading
    /// north, its ceilings read (54 ahead), `SPEED 999` already out.
    fn flying() -> (Picture, Pilot) {
        let mut picture = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            aboard: true,
            ..Picture::default()
        };
        picture.own.power = Power::Online;
        picture.own.pos_m = Some((0.0, 0.0));
        picture.own.pos_read_ms = NOW;
        picture.own.max_ahead_kph = Some(54);
        picture.own.max_astern_kph = Some(27);
        let mut pilot = Pilot::new();
        pilot.power_ms = Some(NOW - 10_000);
        pilot.ceilings_asked = true;
        (picture, pilot)
    }

    /// A contact at `bearing`/`range` from the origin, read now, fresh.
    fn contact(designator: &str, side: Option<Side>, bearing: u32, range: u32) -> Contact {
        contact_from((0.0, 0.0), designator, side, bearing, range)
    }

    /// A contact at `bearing`/`range` from `from`, read now, fresh.
    fn contact_from(
        from: (f64, f64),
        designator: &str,
        side: Option<Side>,
        bearing: u32,
        range: u32,
    ) -> Contact {
        let pos = along(from, f64::from(bearing), f64::from(range));
        Contact {
            designator: designator.into(),
            was: None,
            side,
            bearing_deg: bearing,
            rel_deg: bearing,
            band: "far".into(),
            range_m: Some(range),
            age_ms: 0,
            read_ms: NOW,
            fixes: VecDeque::from([Fix {
                at_ms: NOW - 500,
                pos_m: pos,
                sigma_m: 60.0,
                from_lock: false,
            }]),
            track: None,
        }
    }

    fn hold(picture: &mut Picture, contact: Contact) {
        picture.contacts.insert(contact.designator.clone(), contact);
    }

    fn weapon(id: &str, ready: bool) -> Weapon {
        Weapon {
            id: id.into(),
            ready,
            rounds: 10,
            rounds_max: 40,
            read_ms: NOW - 100,
            ..Weapon::default()
        }
    }

    fn doctrine() -> Doctrine {
        Doctrine::pilot_default(&Settings::default())
    }

    fn decide(picture: &Picture, pilot: &mut Pilot) -> Orders {
        pilot.decide(picture, &doctrine(), NOW)
    }

    fn heading(orders: &Orders) -> u32 {
        orders.heading.expect("a heading")
    }

    #[test]
    fn orbit_takes_the_tangent_at_the_band_and_bends_outside_and_inside_it() {
        for (sense, tangent) in [(Sense::Ccw, 90), (Sense::Cw, 270)] {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            let doctrine = Doctrine {
                sense,
                ..doctrine()
            };
            let orders = pilot.decide(&picture, &doctrine, NOW);
            assert_eq!(heading(&orders), tangent, "{sense:?} at the band");
            assert_eq!(orders.speed, Some(54), "full ahead");
            // Far: 30° off the bearing, toward it.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 750));
            let far = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(far, if sense == Sense::Ccw { 30 } else { 330 });
            // Near: 150° off it, opening.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 150));
            let near = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(near, if sense == Sense::Ccw { 150 } else { 210 });
            // Half the span off the band (R + 75): half the correction, 30°.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 525));
            let half = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(half, if sense == Sense::Ccw { 60 } else { 300 });
        }
    }

    #[test]
    fn close_stand_off_and_withdraw_headings() {
        let run = |stance: Stance, range: u32| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 90, range));
            let doctrine = Doctrine {
                stance,
                sense: Sense::Ccw,
                ..doctrine()
            };
            pilot.decide(&picture, &doctrine, NOW)
        };
        assert_eq!(run(Stance::Close, 600).heading, Some(90));
        assert_eq!(run(Stance::Close, 600).speed, Some(54));
        // Inside 200 m it orbits at 200: at 200 exactly, the tangent.
        assert_eq!(run(Stance::Close, 200).heading, Some(180));
        assert_eq!(run(Stance::StandOff, 700).heading, Some(90));
        assert_eq!(run(Stance::StandOff, 200).heading, Some(270));
        let inside = run(Stance::StandOff, 460);
        assert_eq!((inside.heading, inside.speed), (Some(90), Some(0)));
        let away = run(Stance::Withdraw, 400);
        assert_eq!((away.heading, away.speed), (Some(270), Some(54)));
    }

    #[test]
    fn auto_sense_circles_toward_a_tracked_targets_rear_with_no_timed_flip() {
        // The target is north of the pilot, coursing east (090): its rear points
        // west. Seen from the target, the pilot is due south (180); the rear
        // (270) lies clockwise of it, so the pilot circles clockwise.
        let (mut picture, mut pilot) = flying();
        let mut ba = contact("BA", Some(Side::Blue), 0, 450);
        ba.track = Some(Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        let orders = decide(&picture, &mut pilot);
        assert_eq!(pilot.sense, -1, "clockwise, toward the rear");
        // The track is advanced by the plot's age: with age 0 read now the aim is
        // the plot, so the heading is the tangent clockwise, 270 give or take the
        // radial correction.
        let h = heading(&orders);
        assert!(angle_between(h, 270) <= 5, "heading {h}");
        // Coursing west instead: the rear is east, counter-clockwise of south.
        let (mut picture, mut pilot) = flying();
        let mut ba = contact("BA", Some(Side::Blue), 0, 450);
        ba.track = Some(Track {
            course_deg: 270.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        decide(&picture, &mut pilot);
        assert_eq!(pilot.sense, 1, "counter-clockwise, toward the rear");
        // No timed flip while the track holds, however long.
        for step in 1..=5u64 {
            pilot.decide(&picture, &doctrine(), NOW + step * FLIP_EVERY_MS);
            assert_eq!(pilot.sense, 1);
        }
    }

    #[test]
    fn with_no_track_the_orbit_reverses_every_twenty_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let mut senses = Vec::new();
        for at in [0, 19_999, 20_000, 39_999, 40_000] {
            let mut picture = picture.clone();
            if let Some(c) = picture.contacts.get_mut("BA") {
                c.read_ms = NOW + at;
            }
            picture.own.pos_read_ms = NOW + at;
            pilot.decide(&picture, &doctrine(), NOW + at);
            senses.push(pilot.sense);
        }
        assert_eq!(senses, [1, 1, -1, -1, 1]);
    }

    /// The orbit's heading at `d` from a target at `beta`, circling by `s` at
    /// `band`, computed from the named constants.
    fn orbit_heading(beta: f64, d: f64, band: u32, s: f64) -> u32 {
        let offset = ((d - f64::from(band)) / ORBIT_CORRECTION_SPAN_M).clamp(-1.0, 1.0)
            * ORBIT_CORRECTION_DEG;
        whole(beta + s * (90.0 - offset))
    }

    /// Half the hull's internals gone.
    fn half_hull(picture: &mut Picture) {
        picture.own.sections.insert(
            "hull".into(),
            Section {
                condition: "damaged".into(),
                internals: Some((30, 60)),
                facets: Default::default(),
            },
        );
    }

    #[test]
    fn falling_back_on_half_the_hull_withdraws_then_widens_the_band() {
        // A band of 300 widens toward the sight's ceiling.
        let band = 300;
        let widened = widened_band(band);
        assert_eq!(
            widened,
            band.saturating_add(FALL_BACK_WIDEN_M).min(FALL_BACK_MAX_M)
        );
        let (mut picture, mut pilot) = flying();
        // Inside the band, and so inside the widened band.
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 250));
        half_hull(&mut picture);
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            band_m: band,
            ..doctrine()
        };
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert!(pilot.fallen_back);
        assert_eq!(heading(&orders), 180, "withdrawing: straight away");
        // `WITHDRAW_MS` on it orbits at the widened band: at 250 m that is
        // inside it, so the heading opens past the tangent.
        let mut later = picture.clone();
        let at = NOW + WITHDRAW_MS;
        later.own.pos_read_ms = at;
        if let Some(c) = later.contacts.get_mut("BA") {
            c.read_ms = at;
        }
        let orders = pilot.decide(&later, &doctrine, at);
        assert_eq!(pilot.withdraw_until_ms, None);
        assert_eq!(heading(&orders), orbit_heading(0.0, 250.0, widened, 1.0));
        // A band of 450 stays 450, the sight's ceiling: with the target at
        // 450 m that is the tangent (090), not a wider band's closing turn.
        assert_eq!(widened_band(450), 450);
        let mut at_450 = later.clone();
        hold(&mut at_450, contact("BA", Some(Side::Blue), 0, 450));
        if let Some(c) = at_450.contacts.get_mut("BA") {
            c.read_ms = at;
        }
        let wide = Doctrine {
            band_m: 450,
            ..doctrine.clone()
        };
        let orders = pilot.decide(&at_450, &wide, at);
        assert_eq!(heading(&orders), 90, "the band stays 450");
    }

    #[test]
    fn falling_back_on_a_quarter_facet_from_a_hit_line() {
        let quarter = 400 / FALL_BACK_FACET_DIVISOR;
        let (mut picture, mut pilot) = flying();
        // Inside the widened band, so the withdrawal runs its time.
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify(&format!(
                "Hit on the hull's front facet: {quarter} of 400 armour left."
            )),
        );
        let orders = decide(&picture, &mut pilot);
        assert!(pilot.fallen_back);
        assert_eq!(pilot.withdraw_until_ms, Some(NOW + WITHDRAW_MS));
        assert_eq!(heading(&orders), 180);
        // One more is not a quarter.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify(&format!(
                "Hit on the hull's front facet: {} of 400 armour left.",
                quarter + 1
            )),
        );
        decide(&picture, &mut pilot);
        assert!(!pilot.fallen_back);
    }

    #[test]
    fn a_held_notice_turns_forty_five_degrees_by_sense_for_three_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.own.heading_deg = 100;
        picture.events.push_back(Event {
            at_ms: NOW - 10,
            kind: EventKind::Held,
            text: "Held: something is in the way.".into(),
        });
        let doctrine = Doctrine {
            sense: Sense::Cw,
            ..doctrine()
        };
        // A clockwise doctrine: s = -1, so the turn is -45, whatever the
        // pilot's running sense was (+1 until an orbit sets it).
        assert_eq!(pilot.sense, 1);
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert_eq!(heading(&orders), 55);
        pilot.sent_heading(55, NOW);
        let orders = pilot.decide(&picture, &doctrine, NOW + HELD_FOR_MS - 1);
        assert_eq!(orders.heading, None, "held on 055");
        let orders = pilot.decide(&picture, &doctrine, NOW + HELD_FOR_MS);
        assert_eq!(heading(&orders), 270, "back on the clockwise tangent");
        // Counter-clockwise: +45.
        let held = |sense: Sense, running: i8| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.own.heading_deg = 100;
            pilot.sense = running;
            picture.events.push_back(Event {
                at_ms: NOW - 10,
                kind: EventKind::Held,
                text: "Held: something is in the way.".into(),
            });
            let doctrine = Doctrine {
                sense,
                ..Doctrine::pilot_default(&Settings::default())
            };
            heading(&pilot.decide(&picture, &doctrine, NOW))
        };
        assert_eq!(held(Sense::Ccw, -1), 145);
        assert_eq!(held(Sense::Cw, 1), 55);
        // `Auto`: the pilot's running sense.
        assert_eq!(held(Sense::Auto, 1), 145);
        assert_eq!(held(Sense::Auto, -1), 55);
    }

    #[test]
    fn an_immobile_tank_orders_no_motion_and_a_destroyed_one_nothing() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.weapons = vec![weapon("cannon", true)];
        picture.own.immobile = true;
        let orders = decide(&picture, &mut pilot);
        assert_eq!((orders.heading, orders.speed), (None, None));
        assert_eq!(orders.lock.as_deref(), Some("BA"), "it still locks");
        picture.own.immobile = false;
        picture.own.disabled = true;
        let orders = decide(&picture, &mut pilot);
        assert_eq!((orders.heading, orders.speed), (None, None));
        picture.own.destroyed = true;
        assert_eq!(decide(&picture, &mut pilot), Orders::default());
    }

    #[test]
    fn nothing_unless_aboard_powered_and_running() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let mut off = picture.clone();
        off.aboard = false;
        assert_eq!(decide(&off, &mut pilot), Orders::default());
        let mut dark = picture.clone();
        dark.own.power = Power::Starting;
        assert_eq!(decide(&dark, &mut pilot), Orders::default());
        let mut seated = picture.clone();
        seated.phase = Phase::Seated { match_id: 1 };
        assert_eq!(decide(&seated, &mut pilot), Orders::default());
    }

    #[test]
    fn never_a_friend_or_an_unknown_locked_or_fired_on() {
        let (mut picture, mut pilot) = flying();
        picture.weapons = vec![weapon("cannon", true)];
        hold(&mut picture, contact("X1", None, 0, 300));
        hold(&mut picture, contact("RB", Some(Side::Red), 90, 200));
        let orders = decide(&picture, &mut pilot);
        assert_eq!(orders.lock, None);
        assert!(orders.fire.is_empty());
        // Even locked, a friend or an unknown is never fired on.
        for (lock, friendly) in [("X1", false), ("RB", true)] {
            picture.lock = Some(lock.into());
            picture.lock_friendly = friendly;
            let orders = decide(&picture, &mut pilot);
            assert!(orders.fire.is_empty(), "{lock}");
            assert_eq!(orders.lock, None);
        }
    }

    #[test]
    fn the_lock_keeps_its_target_until_another_is_a_hundred_metres_nearer() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 360));
        picture.lock = Some("BA".into());
        assert_eq!(decide(&picture, &mut pilot).lock, None, "90 m nearer: kept");
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 350));
        assert_eq!(
            decide(&picture, &mut pilot).lock.as_deref(),
            Some("BB"),
            "100 m nearer"
        );
        // With no lock the nearest fresh plot wins, a stale one last.
        let (mut picture, mut pilot) = flying();
        let mut stale = contact("BA", Some(Side::Blue), 0, 100);
        stale.age_ms = 4_000;
        hold(&mut picture, stale);
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 500));
        assert_eq!(decide(&picture, &mut pilot).lock.as_deref(), Some("BB"));
        // A lock order goes at most once every 1.5 s.
        pilot.sent_lock(NOW);
        assert_eq!(pilot.decide(&picture, &doctrine(), NOW + 1_499).lock, None);
        assert_eq!(
            pilot
                .decide(&picture, &doctrine(), NOW + 1_500)
                .lock
                .as_deref(),
            Some("BB")
        );
    }

    #[test]
    fn fire_is_gated_on_readiness_rounds_freshness_and_chance() {
        let armed = || {
            let (mut picture, pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.lock = Some("BA".into());
            picture.weapons = vec![weapon("machine_gun", true), weapon("cannon", true)];
            (picture, pilot)
        };
        let (picture, mut pilot) = armed();
        assert_eq!(
            decide(&picture, &mut pilot).fire,
            ["cannon", "machine_gun"],
            "the main gun first"
        );
        // Not ready.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        // A `ready` read inside 1.5 s of the ack is not believed.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].fired_ms = Some(NOW - 1_000);
        picture.weapons[1].read_ms = NOW - 100;
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].fired_ms = Some(NOW - 2_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // A cycle not yet elapsed; then elapsed — on the one clock, which the
        // ack sets from the cycle. A shot and a cycle with no clock set are
        // no clock: the row is read, and it says not ready.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        picture.weapons[1].fired_ms = Some(NOW - 4_000);
        picture.weapons[1].cycle_ms = Some(4_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].next_ready_ms = Some(NOW - 4_000 + 6_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].next_ready_ms = Some(NOW - 4_000 + 4_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // `next_ready_ms` from a cycling row.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        picture.weapons[1].next_ready_ms = Some(NOW + 1);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].next_ready_ms = Some(NOW);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // No rounds; 0%.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].rounds = 0;
        picture.weapons[0].percent = Some(0);
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // A stale lock: the plot too old, or the row read too long ago.
        let (mut picture, mut pilot) = armed();
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.age_ms = 3_001;
        }
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        let (mut picture, mut pilot) = armed();
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.read_ms = NOW - 2_001;
        }
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // Locked on something else: nothing.
        let (mut picture, mut pilot) = armed();
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 460));
        picture.lock = Some("BB".into());
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        picture.lock = None;
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // No cannon: the first weapon is the main gun.
        let (mut picture, mut pilot) = armed();
        picture.weapons = vec![weapon("laser", true), weapon("machine_gun", true)];
        assert_eq!(decide(&picture, &mut pilot).fire, ["laser", "machine_gun"]);
    }

    #[test]
    fn the_search_drives_for_b_past_it_eight_hundred_metres_and_back() {
        let (mut picture, mut pilot) = flying();
        picture.spots.insert(
            'B',
            crate::arena::picture::Spot {
                name: "The Crossroads".into(),
                range_m: Some(1_000),
                bearing_deg: Some(90),
                read_ms: NOW,
                plotted_m: Some((1_000, 0)),
            },
        );
        // Nothing held since power came up ten seconds ago: searching.
        let orders = decide(&picture, &mut pilot);
        assert!(orders.ask_spot_b);
        assert_eq!(heading(&orders), 90);
        assert_eq!(pilot.spot_b_pos.map(|p| p.0.round()), Some(1_000.0));
        assert_eq!(pilot.start_pos, Some((0.0, 0.0)));
        // Within 100 m of B: on along start→B (090) for 800 m.
        let mut at_b = picture.clone();
        at_b.own.pos_m = Some((950.0, 0.0));
        at_b.own.heading_deg = 90;
        // A `BEARING B` answer read in the same tick as the position changes the
        // spot's bearing, not where the spot lies.
        if let Some(spot) = at_b.spots.get_mut(&'B') {
            spot.bearing_deg = Some(95);
            spot.read_ms = NOW + 1;
        }
        at_b.own.pos_read_ms = NOW + 1;
        pilot.sent_heading(90, NOW);
        let orders = pilot.decide(&at_b, &doctrine(), NOW + 1);
        assert!(matches!(pilot.search, SearchLeg::Beyond { .. }));
        assert!(orders.ask_spot_b);
        assert_eq!(orders.heading, None, "already on 090");
        let mut on = at_b.clone();
        on.own.pos_m = Some((1_700.0, 0.0));
        let orders = pilot.decide(&on, &doctrine(), NOW + 2);
        assert_eq!(orders.heading, None, "still going: 750 m out");
        let mut out = at_b.clone();
        out.own.pos_m = Some((1_760.0, 0.0));
        let orders = pilot.decide(&out, &doctrine(), NOW + 3);
        assert_eq!(pilot.search, SearchLeg::ToB);
        assert_eq!(heading(&orders), 270, "back for B");
        // A hostile held: no search, and not before five quiet seconds.
        let (mut picture, mut pilot) = flying();
        pilot.power_ms = Some(NOW - 4_999);
        assert!(!decide(&picture, &mut pilot).ask_spot_b);
        hold(&mut picture, contact("X1", None, 0, 500));
        let (_, mut fresh) = flying();
        assert!(
            fresh.decide(&picture, &doctrine(), NOW).ask_spot_b,
            "an unknown is not a hostile"
        );
    }

    #[test]
    fn the_crossroads_is_plotted_from_a_status_block_never_from_a_bearing_answer() {
        let (mut picture, mut pilot) = flying();
        let read = |picture: &mut Picture, at: u64, lines: &[&str]| {
            for line in lines {
                picture.apply(at, &classify(line));
            }
        };
        // At (1005, 975) The Crossroads is nearest, 290 m at 090: (1295, 975).
        read(
            &mut picture,
            NOW,
            &[
                "Callsign RA.",
                "Red Tank 1: power online, 0 kph, heading 090°",
                "  at 1005 m east, 975 m north; scope clear",
                "  nearest spot: The Crossroads (B), 290 m at 090°",
                "No contacts.",
            ],
        );
        decide(&picture, &mut pilot);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
        // Twenty seconds on, at (828, 839) with The Ford nearest, a `STATUS` and
        // a `BEARING B` answer land together: the bearing is fresh, the spot's
        // range is the old block's. B stays where it is.
        let later = NOW + 20_000;
        read(
            &mut picture,
            later,
            &[
                "Callsign RA.",
                "Red Tank 1: power online, 0 kph, heading 090°",
                "  at 828 m east, 839 m north; scope clear",
                "  nearest spot: The Ford (A), 300 m at 200°",
                "Bearing to The Crossroads (B): 070° (rel 340°).",
            ],
        );
        assert_eq!(picture.spots[&'B'].bearing_deg, Some(70));
        assert_eq!(picture.spots[&'B'].range_m, Some(290));
        let orders = pilot.decide(&picture, &doctrine(), later);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
        // bearing (828, 839) -> (1295, 975) is 073.8.
        assert_eq!(orders.heading, Some(74));
        // A `RANGE B` answer does not move it either.
        read(
            &mut picture,
            later + 10,
            &["Range to The Crossroads (B): 480 m."],
        );
        pilot.decide(&picture, &doctrine(), later + 10);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
    }

    #[test]
    fn a_heading_goes_again_only_eight_degrees_off_or_after_four_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        assert_eq!(pilot.decide(&picture, &doctrine, NOW).heading, Some(90));
        pilot.sent_heading(90, NOW);
        pilot.sent_speed(54);
        // 7° off (the target 7° further round): not yet.
        hold(&mut picture, contact("BA", Some(Side::Blue), 7, 450));
        let orders = pilot.decide(&picture, &doctrine, NOW + 100);
        assert_eq!((orders.heading, orders.speed), (None, None));
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 4_000).heading,
            Some(97),
            "any amount after 4 s"
        );
        hold(&mut picture, contact("BA", Some(Side::Blue), 8, 450));
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 100).heading,
            Some(98)
        );
        // An unanswered order is forgotten and goes again.
        pilot.forget_heading();
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 101).heading,
            Some(98)
        );
    }

    #[test]
    fn the_first_speed_order_is_nine_nine_nine() {
        let (mut picture, mut pilot) = flying();
        picture.own.max_ahead_kph = None;
        pilot.ceilings_asked = false;
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let orders = decide(&picture, &mut pilot);
        assert_eq!(orders.speed, Some(CEILING_ASK_KPH));
        pilot.sent_speed(CEILING_ASK_KPH);
        // Asked, not yet read: full ahead is 40.
        assert_eq!(decide(&picture, &mut pilot).speed, Some(UNKNOWN_FULL_KPH));
        // Unanswered: asked again.
        pilot.forget_speed();
        assert_eq!(decide(&picture, &mut pilot).speed, Some(CEILING_ASK_KPH));
        // Read: 54.
        picture.apply(
            NOW,
            &classify("That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54."),
        );
        assert_eq!(decide(&picture, &mut pilot).speed, Some(54));
    }

    // -- cut AK.1: the hunt ------------------------------------------------------

    /// Mid-field, where no hunt aim of these tests meets the edge.
    const MID: (f64, f64) = (1_280.0, 1_000.0);

    /// `flying()`, the tank at `pos`.
    fn flying_at(pos: (f64, f64)) -> (Picture, Pilot) {
        let (mut picture, pilot) = flying();
        picture.own.pos_m = Some(pos);
        (picture, pilot)
    }

    /// The own tank read at `pos` at `at`.
    fn read_at(picture: &mut Picture, pos: (f64, f64), at: u64) {
        picture.own.pos_m = Some(pos);
        picture.own.pos_read_ms = at;
    }

    /// A fresh `BA` at `bearing`/`range` from `from`, decided on at `NOW`, then
    /// gone from the scope a second later: the pilot's orders then.
    fn lost(
        from: (f64, f64),
        bearing: u32,
        range: u32,
        track: Option<Track>,
    ) -> (Picture, Pilot, Orders) {
        let (mut picture, mut pilot) = flying_at(from);
        let mut ba = contact_from(from, "BA", Some(Side::Blue), bearing, range);
        ba.track = track;
        hold(&mut picture, ba);
        decide(&picture, &mut pilot);
        picture.contacts.clear();
        let orders = pilot.decide(&picture, &doctrine(), NOW + 1_000);
        (picture, pilot, orders)
    }

    fn hunt_of(pilot: &Pilot) -> Hunt {
        pilot.hunt.clone().expect("hunting")
    }

    fn held_event(picture: &mut Picture, at_ms: u64) {
        picture.events.push_back(Event {
            at_ms,
            kind: EventKind::Held,
            text: "Held: the ground ahead will not take the tank.".into(),
        });
    }

    #[test]
    fn a_lost_hostile_is_hunted_to_its_last_plot() {
        let plot = along(MID, 0.0, 450.0);
        for track in [
            None,
            Some(Track {
                course_deg: 90.0,
                speed_mps: 8.0,
            }),
            // Coursing back at the pilot: the first leg is still the plot.
            Some(Track {
                course_deg: 180.0,
                speed_mps: 8.0,
            }),
        ] {
            let (_, mut pilot, orders) = lost(MID, 0, 450, track.clone());
            let hunt = hunt_of(&pilot);
            assert_eq!(hunt.leg, HuntLeg::ToPlot, "{track:?}");
            assert_eq!(hunt.trail.designator, "BA");
            assert!(distance_m(hunt.trail.plot_m, plot) < 1e-6);
            assert_eq!(hunt.trail.track, track);
            assert_eq!(heading(&orders), whole(bearing_deg(MID, plot)));
            assert_eq!(orders.speed, Some(54), "full ahead");
            assert_eq!(
                pilot.take_notes(),
                ["Hunting BA: its last plot, 450 m away."]
            );
        }
    }

    #[test]
    fn a_tracked_hostile_is_hunted_along_its_course() {
        let track = Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        };
        // West of mid-field, so the capped aim stays inside the ground.
        let from = (300.0, 1_000.0);
        let (mut picture, mut pilot, _) = lost(from, 0, 450, Some(track.clone()));
        pilot.take_notes();
        let hunt = hunt_of(&pilot);
        let plot = hunt.trail.plot_m;
        assert_eq!((hunt.course_deg, hunt.speed_mps), (90.0, 8.0));
        // At the plot, ten seconds after it was taken: 80 m on along 090.
        let t = hunt.trail.plot_ms + 10_000;
        read_at(&mut picture, plot, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        let hunt = hunt_of(&pilot);
        assert_eq!(hunt.leg, HuntLeg::Along);
        let (aim, clamped) = leg_aim(&hunt, t);
        assert!(!clamped);
        assert!(distance_m(aim, (plot.0 + 80.0, plot.1)) < 1e-6, "{aim:?}");
        assert_eq!(heading(&orders), 90);
        assert_eq!(
            pilot.take_notes(),
            ["Hunting BA: along 090° from its last plot."]
        );
        // Far enough on, the aim stops `HUNT_ALONG_MAX_M` past the plot.
        let far =
            hunt.trail.plot_ms + (HUNT_ALONG_MAX_M / track.speed_mps * 1_000.0) as u64 + 5_000;
        let (aim, clamped) = leg_aim(&hunt, far);
        assert!(!clamped);
        assert!(
            distance_m(aim, (plot.0 + HUNT_ALONG_MAX_M, plot.1)) < 1e-6,
            "{aim:?}"
        );
    }

    #[test]
    fn the_hunt_never_aims_past_the_arena_edge() {
        let edge = ARENA_SIDE_M - EDGE_MARGIN_M;
        let track = Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        };
        // A plot 200 m east of a tank at 2 300 m east: at the edge's margin,
        // coursing east.
        let (mut picture, mut pilot, _) = lost((2_300.0, 1_000.0), 90, 200, Some(track));
        let hunt = hunt_of(&pilot);
        let plot = hunt.trail.plot_m;
        let t = hunt.trail.plot_ms + 10_000;
        let course_leg = Hunt {
            leg: HuntLeg::Along,
            ..hunt.clone()
        };
        let (aim, clamped) = leg_aim(&course_leg, t);
        assert!(clamped, "the course runs past the edge");
        assert!(aim.0 <= edge, "{aim:?}");
        // Arriving there ends the course leg: the sweep, inside the ground.
        read_at(&mut picture, plot, t);
        pilot.decide(&picture, &doctrine(), t);
        let hunt = hunt_of(&pilot);
        assert!(
            matches!(hunt.leg, HuntLeg::Sweep { leg: 0, .. }),
            "{hunt:?}"
        );
        let (aim, _) = leg_aim(&hunt, t);
        assert!((EDGE_MARGIN_M..=edge).contains(&aim.0) && (EDGE_MARGIN_M..=edge).contains(&aim.1));
        // A plot outside the square: the leg to it aims inside.
        let (_, pilot, orders) = lost((2_000.0, 1_000.0), 90, 900, None);
        let hunt = hunt_of(&pilot);
        assert!(hunt.trail.plot_m.0 > ARENA_SIDE_M);
        let (aim, clamped) = leg_aim(&hunt, NOW + 1_000);
        assert!(clamped);
        assert!(aim.0 <= edge && aim.0 >= EDGE_MARGIN_M, "{aim:?}");
        assert_eq!(heading(&orders), 90);
    }

    /// The course leg, driven on along the course once level with its moving
    /// aim, never runs the tank past the edge (its aim still short of the
    /// clamp), nor on far past the aim: either ends the leg in the sweep.
    #[test]
    fn the_course_leg_ends_at_the_edge_or_past_its_aim() {
        let edge = ARENA_SIDE_M - EDGE_MARGIN_M;
        // A plot 250 m short of the edge's margin, coursing east at the speed
        // that puts the aim 50 m behind a tank 50 m short of it at half the
        // trail's life.
        let (plot, own_x) = (edge - 250.0, edge - 50.0);
        let half = HUNT_TRAIL_MS / 2;
        let speed = (own_x - 50.0 - plot) / (half as f64 / 1_000.0);
        let course = |speed_mps| Track {
            course_deg: 90.0,
            speed_mps,
        };
        let on_the_course = |from_x: f64, speed_mps: f64| {
            let (mut picture, mut pilot, _) =
                lost((from_x - 200.0, 1_000.0), 90, 200, Some(course(speed_mps)));
            let hunt = hunt_of(&pilot);
            let t = NOW + 2_000;
            read_at(&mut picture, hunt.trail.plot_m, t);
            pilot.decide(&picture, &doctrine(), t);
            assert_eq!(hunt_of(&pilot).leg, HuntLeg::Along);
            pilot.take_notes();
            (picture, pilot, hunt.trail.plot_ms)
        };
        let (mut picture, mut pilot, plot_ms) = on_the_course(plot, speed);
        let t = plot_ms + half;
        let (aim, clamped) = leg_aim(&hunt_of(&pilot), t);
        assert!(!clamped && (aim.0 - (own_x - 50.0)).abs() < 1e-6, "{aim:?}");
        // Level with the aim, 50 m ahead of it: the course's next 100 m
        // crosses the edge, so the leg is done there.
        read_at(&mut picture, (own_x, 1_000.0), t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        let hunt = hunt_of(&pilot);
        let HuntLeg::Sweep {
            leg: 0,
            heading_deg,
            origin,
        } = hunt.leg
        else {
            panic!("the sweep at the edge: {hunt:?}");
        };
        assert_ne!(heading(&orders), 90, "not on toward the edge");
        assert_eq!(heading(&orders), whole(heading_deg));
        assert!(origin.0 <= edge);
        assert_eq!(pilot.take_notes(), ["Sweeping for BA."]);
        // Mid-field: level with the aim and at most `HUNT_ARRIVE_M` ahead, it
        // drives on along the course; further ahead, the estimate is behind
        // it and the leg is done.
        let track = course(8.0);
        let (mut picture, mut pilot, plot_ms) = on_the_course(MID.0, track.speed_mps);
        let t = plot_ms + 10_000;
        let (aim, clamped) = leg_aim(&hunt_of(&pilot), t);
        assert!(!clamped);
        read_at(&mut picture, (aim.0 + HUNT_ARRIVE_M - 10.0, aim.1), t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::Along);
        assert_eq!(heading(&orders), 90, "on along the course");
        let t = t + 1_000;
        let (aim, _) = leg_aim(&hunt_of(&pilot), t);
        read_at(&mut picture, (aim.0 + HUNT_ARRIVE_M + 50.0, aim.1), t);
        pilot.decide(&picture, &doctrine(), t);
        assert!(
            matches!(hunt_of(&pilot).leg, HuntLeg::Sweep { leg: 0, .. }),
            "past the aim: the sweep"
        );
    }

    #[test]
    fn a_held_line_ends_a_hunt_leg() {
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, None);
        let t = NOW + 2_000;
        held_event(&mut picture, t - 10);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::Along);
        assert_eq!(
            orders.heading, pilot.held_heading,
            "the held turn still happens"
        );
        let t = t + 1_000;
        held_event(&mut picture, t - 10);
        pilot.decide(&picture, &doctrine(), t);
        assert!(matches!(hunt_of(&pilot).leg, HuntLeg::Sweep { leg: 0, .. }));
        assert!(pilot.take_notes().contains(&"Sweeping for BA.".to_owned()));
    }

    /// Drive a hunt on `track` from mid-field to its sweep; the time then.
    fn to_the_sweep(picture: &mut Picture, pilot: &mut Pilot) -> u64 {
        let hunt = hunt_of(pilot);
        let plot = hunt.trail.plot_m;
        read_at(picture, plot, NOW + 5_000);
        pilot.decide(picture, &doctrine(), NOW + 5_000);
        assert_eq!(hunt_of(pilot).leg, HuntLeg::Along);
        let t = hunt.trail.plot_ms + HUNT_TRAIL_MS + 1;
        read_at(picture, plot, t);
        pilot.decide(picture, &doctrine(), t);
        assert!(matches!(hunt_of(pilot).leg, HuntLeg::Sweep { leg: 0, .. }));
        t
    }

    #[test]
    fn the_hunt_sweeps_then_hands_over_to_the_search() {
        let track = Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        };
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, Some(track.clone()));
        let mut now = to_the_sweep(&mut picture, &mut pilot);
        pilot.take_notes();
        // Each leg a quarter turn on by the pilot's running sense, from the
        // course.
        let sense = f64::from(pilot.sense);
        for k in 0..HUNT_SWEEP_LEGS {
            let orders = pilot.decide(&picture, &doctrine(), now);
            let hunt = hunt_of(&pilot);
            let HuntLeg::Sweep {
                leg, heading_deg, ..
            } = hunt.leg
            else {
                panic!("sweeping: {hunt:?}");
            };
            assert_eq!(leg, k);
            let expected = whole(track.course_deg + 90.0 * sense * f64::from(k + 1));
            assert_eq!(whole(heading_deg), expected);
            assert_eq!(heading(&orders), expected, "leg {k}");
            let (aim, _) = leg_aim(&hunt, now);
            now += 1_000;
            read_at(&mut picture, aim, now);
        }
        let orders = pilot.decide(&picture, &doctrine(), now);
        assert!(pilot.hunt.is_none() && pilot.trail.is_none());
        assert_eq!(pilot.take_notes(), ["The hunt for BA ends; searching."]);
        assert!(orders.ask_spot_b, "the search asks for B");
        // Separately: the sweep ends `HUNT_SWEEP_MS` after it began.
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, Some(track));
        let began = to_the_sweep(&mut picture, &mut pilot);
        pilot.decide(&picture, &doctrine(), began + HUNT_SWEEP_MS - 1);
        assert!(pilot.hunt.is_some());
        let orders = pilot.decide(&picture, &doctrine(), began + HUNT_SWEEP_MS);
        assert!(pilot.hunt.is_none());
        assert!(
            pilot
                .take_notes()
                .contains(&"The hunt for BA ends; searching.".to_owned())
        );
        assert!(orders.ask_spot_b);
    }

    #[test]
    fn a_fresh_hostile_ends_the_hunt() {
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, None);
        pilot.take_notes();
        let t = NOW + 2_000;
        let mut bb = contact_from(MID, "BB", Some(Side::Blue), 90, 450);
        bb.read_ms = t;
        hold(&mut picture, bb);
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        let orders = pilot.decide(&picture, &doctrine, t);
        assert!(pilot.hunt.is_none());
        assert_eq!(
            pilot.take_notes(),
            ["The hunt for BA ends: BB is in sight."]
        );
        assert_eq!(heading(&orders), 180, "the tangent round BB, at 090");
    }

    /// A stale `BA` row, the plot `HUNT_STALE_PLOT_MS` and more old, read at `t`.
    fn stale_ba(picture: &mut Picture, t: u64) {
        let mut stale = contact_from(MID, "BA", Some(Side::Blue), 0, 450);
        stale.age_ms = HUNT_STALE_PLOT_MS + 1;
        stale.read_ms = t;
        hold(picture, stale);
    }

    #[test]
    fn a_stale_hostile_is_hunted_not_orbited() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        let t = NOW + HUNT_STALE_PLOT_MS + 2_000;
        stale_ba(&mut picture, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::ToPlot);
        assert_eq!(
            heading(&orders),
            0,
            "the bearing to its plot, not a tangent"
        );
        // A row that is already that old at its first sight has no trail and
        // is nothing to hunt: it is flown round like a young one.
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        let (mut picture, mut pilot) = flying_at(MID);
        stale_ba(&mut picture, NOW);
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert!(pilot.trail.is_none() && pilot.hunt.is_none());
        assert_eq!(
            heading(&orders),
            orbit_heading(0.0, 450.0, doctrine.band_m, 1.0),
            "the orbit's tangent"
        );
        assert!(!orders.ask_spot_b);
    }

    #[test]
    fn a_stale_row_does_not_block_the_search_after_a_hunt() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        let plot = pilot.trail.clone().expect("a trail").plot_m;
        // Hunted to the plot, along, into the sweep and out of it, the stale
        // row listed all the while.
        let mut t = NOW + HUNT_STALE_PLOT_MS + 2_000;
        for (pos, at) in [(MID, t), (plot, t + 1_000), (plot, NOW + HUNT_TRAIL_MS + 1)] {
            t = at;
            stale_ba(&mut picture, t);
            read_at(&mut picture, pos, t);
            pilot.decide(&picture, &doctrine(), t);
        }
        assert!(matches!(hunt_of(&pilot).leg, HuntLeg::Sweep { .. }));
        t += HUNT_SWEEP_MS;
        stale_ba(&mut picture, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert!(pilot.hunt.is_none());
        assert_eq!(
            pilot.last_hostile_ms,
            Some(NOW),
            "the stale row does not count as held"
        );
        assert!(t >= NOW + SEARCH_AFTER_MS);
        assert!(orders.ask_spot_b, "searching, not orbiting the stale plot");
        assert_eq!(pilot.orbit_since_ms, None);
    }

    #[test]
    fn a_kill_drops_the_trail() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        assert!(pilot.trail.is_some());
        picture.events.push_back(Event {
            at_ms: NOW + 100,
            kind: EventKind::Kill,
            text: "BA destroyed.".into(),
        });
        // The wreck is still listed, fresh: it is not trailed again.
        pilot.decide(&picture, &doctrine(), NOW + 200);
        assert!(pilot.trail.is_none());
        assert!(pilot.wrecks.contains("BA"));
        picture.contacts.clear();
        picture.events.push_back(Event {
            at_ms: NOW + 900,
            kind: EventKind::Lost,
            text: "Contact BA lost.".into(),
        });
        pilot.decide(&picture, &doctrine(), NOW + 1_000);
        assert!(pilot.hunt.is_none());
        assert!(pilot.take_notes().is_empty(), "no hunt started");
        // An unnamed kill of the lock.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        picture.lock = Some("BA".into());
        decide(&picture, &mut pilot);
        picture.lock = None;
        picture.events.push_back(Event {
            at_ms: NOW + 100,
            kind: EventKind::Kill,
            text: "Target destroyed.".into(),
        });
        picture.contacts.clear();
        pilot.decide(&picture, &doctrine(), NOW + 1_000);
        assert!(pilot.trail.is_none() && pilot.hunt.is_none());
    }

    /// A wreck's row, still listed fresh, is no hostile: it neither holds off
    /// nor ends the hunt for another, is never targeted, locked or shot at,
    /// and does not keep the search away.
    #[test]
    fn a_wreck_neither_holds_nor_ends_a_hunt() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BB", Some(Side::Blue), 0, 300),
        );
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 90, 450),
        );
        decide(&picture, &mut pilot);
        assert_eq!(
            pilot.trail.as_ref().map(|t| t.designator.as_str()),
            Some("BB")
        );
        // BA is destroyed (its row stays, fresh), and BB drops out of sight.
        picture.lock = Some("BA".into());
        picture.events.push_back(Event {
            at_ms: NOW + 100,
            kind: EventKind::Kill,
            text: "BA destroyed.".into(),
        });
        picture.contacts.remove("BB");
        let wreck = |picture: &mut Picture, t: u64| {
            if let Some(ba) = picture.contacts.get_mut("BA") {
                ba.read_ms = t;
            }
        };
        let t = NOW + 1_000;
        wreck(&mut picture, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).trail.designator, "BB", "the hunt starts");
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Hunting BB: its last plot, {:.0} m away.",
                distance_m(MID, along(MID, 0.0, 300.0))
            )]
        );
        assert!(
            orders.fire.is_empty() && orders.lock.is_none(),
            "{orders:?}"
        );
        assert_eq!(heading(&orders), 0, "to BB's plot, not round the wreck");
        // Still listed, still fresh: the hunt goes on, nothing is said.
        let t = t + 1_000;
        wreck(&mut picture, t);
        pilot.decide(&picture, &doctrine(), t);
        assert!(pilot.hunt.is_some());
        assert!(pilot.take_notes().is_empty());
        assert_eq!(pilot.last_hostile_ms, Some(NOW), "the wreck is not held");
    }

    #[test]
    fn an_unidentified_contact_neither_starts_nor_ends_a_hunt() {
        // An unknown alone is never trailed, so it is never hunted.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(&mut picture, contact_from(MID, "X1", None, 0, 450));
        decide(&picture, &mut pilot);
        assert!(pilot.trail.is_none());
        picture.contacts.clear();
        pilot.decide(&picture, &doctrine(), NOW + 1_000);
        assert!(pilot.hunt.is_none());
        // A hunt starts with a fresh unknown held, and one does not end it.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        picture.contacts.clear();
        for t in [NOW + 1_000, NOW + 2_000] {
            let mut x2 = contact_from(MID, "X2", None, 90, 300);
            x2.read_ms = t;
            hold(&mut picture, x2);
            pilot.decide(&picture, &doctrine(), t);
            assert_eq!(hunt_of(&pilot).trail.designator, "BA");
        }
    }

    /// **The bot-side observer rule for the new state**: fed through
    /// [`Picture::apply`], the trail and the hunt name only a designator a
    /// contact row or a contact notice named.
    #[test]
    fn the_trail_names_only_a_designator_the_stream_named() {
        let (mut picture, mut pilot) = flying();
        let lines = [
            (NOW, "Callsign RA."),
            (NOW, "Red Tank 1: power online, 0 kph, heading 000°"),
            (NOW, "  at 1280 m east, 1000 m north; scope holds a hostile"),
            (NOW + 10, "New contact X1: bearing 000°, far."),
            (NOW + 20, "Contact X1 identified: BA, blue."),
            (
                NOW + 30,
                "BA  blue  bearing 000° (rel 000°)  far  450 m  0 s",
            ),
            (NOW + 40, "No such contact."),
        ];
        let mut named = BTreeSet::new();
        let mut read = |picture: &mut Picture, lines: &[(u64, &str)]| {
            for (at, text) in lines {
                let line = classify(text);
                match &line {
                    crate::arena::Line::ContactRow(row) => {
                        named.insert(row.designator.clone());
                    }
                    crate::arena::Line::NewContact { designator, .. }
                    | crate::arena::Line::ContactLost { designator } => {
                        named.insert(designator.clone());
                    }
                    crate::arena::Line::Identified {
                        was, designator, ..
                    } => {
                        named.insert(was.clone());
                        named.insert(designator.clone());
                    }
                    _ => {}
                }
                picture.apply(*at, &line);
            }
        };
        read(&mut picture, &lines);
        pilot.decide(&picture, &doctrine(), NOW + 50);
        let trail = pilot.trail.clone().expect("a trail");
        read(&mut picture, &[(NOW + 1_000, "Contact BA lost.")]);
        pilot.decide(&picture, &doctrine(), NOW + 1_010);
        let hunt = hunt_of(&pilot);
        assert!(named.contains(&trail.designator), "{named:?}");
        assert!(named.contains(&hunt.trail.designator));
        for note in pilot.take_notes() {
            let who = note
                .strip_prefix("Hunting ")
                .and_then(|rest| rest.split(':').next())
                .expect("a hunt sentence");
            assert!(named.contains(who), "{note}");
        }
    }

    #[test]
    fn only_the_hulls_own_plates_fall_back() {
        let quarter = 400 / FALL_BACK_FACET_DIVISOR;
        let plate = |section: &str, left: i64, max: i64| {
            let (mut picture, _) = flying();
            picture.own.sections.insert(
                section.into(),
                Section {
                    condition: "damaged".into(),
                    internals: None,
                    facets: [("front".to_owned(), (left, max))].into_iter().collect(),
                },
            );
            fall_back_due(&picture)
        };
        assert_eq!(plate("turret", quarter, 400), None, "a turret plate");
        assert_eq!(plate("left_track", 0, 100), None, "a track plate");
        // Nor does the pilot, deciding: it flies exactly as an unhurt tank
        // would, no withdrawal and nothing said.
        let flown = |plate: Option<(&str, i64, i64)>| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
            if let Some((section, left, max)) = plate {
                picture.own.sections.insert(
                    section.into(),
                    Section {
                        condition: "damaged".into(),
                        internals: None,
                        facets: [("front".to_owned(), (left, max))].into_iter().collect(),
                    },
                );
            }
            let orders = decide(&picture, &mut pilot);
            (orders.heading, pilot.fallen_back, pilot.take_notes())
        };
        let unhurt = flown(None);
        assert_ne!(unhurt.0, Some(180), "an orbit, not a withdrawal");
        assert!(!unhurt.1 && unhurt.2.is_empty(), "{unhurt:?}");
        for hurt in [("turret", quarter, 400), ("left_track", 0, 100)] {
            assert_eq!(flown(Some(hurt)), unhurt, "{hurt:?}");
        }
        assert_eq!(
            plate("hull", quarter, 400),
            Some(FallBackWhy::Plate {
                facet: "front".into(),
                gone: false
            })
        );
        // Each that does falls back once, with its sentence.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify(&format!(
                "Hit on the hull's front facet: {quarter} of 400 armour left."
            )),
        );
        decide(&picture, &mut pilot);
        let band = widened_band(doctrine().band_m);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Falling back to {band} m: the hull's front plate is at a quarter."
            )]
        );
        // A plate the `STATUS` never read, gone: stored as (0, 0).
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify("Hit on the hull's rear facet: armour gone, 140 of 200 inside."),
        );
        assert_eq!(picture.own.sections["hull"].facets["rear"], (0, 0));
        assert!(!hull_failing(&picture));
        decide(&picture, &mut pilot);
        assert!(pilot.fallen_back);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Falling back to {band} m: the hull's rear plate is gone."
            )]
        );
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        assert!(hull_failing(&picture));
        assert_eq!(fall_back_due(&picture), Some(FallBackWhy::HullHalf));
        decide(&picture, &mut pilot);
        assert_eq!(
            pilot.take_notes(),
            [format!("Falling back to {band} m: the hull is at half.")]
        );
        decide(&picture, &mut pilot);
        assert!(pilot.take_notes().is_empty(), "once a match");
    }

    #[test]
    fn the_fall_back_band_never_narrows_and_stays_inside_the_sight() {
        for band in (150..=SIGHT_BAND_MAX_M).step_by(10) {
            let widened = widened_band(band);
            assert!(band <= widened && widened <= SIGHT_BAND_MAX_M, "{band}");
        }
        assert_eq!(FALL_BACK_MAX_M, SIGHT_BAND_MAX_M);
        // The withdrawal ends once the target is out at the widened band,
        // before its time.
        let band = 300;
        let widened = widened_band(band);
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            band_m: band,
            ..doctrine()
        };
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 100));
        half_hull(&mut picture);
        assert_eq!(heading(&pilot.decide(&picture, &doctrine, NOW)), 180);
        let t = NOW + 1_000;
        assert!(t < NOW + WITHDRAW_MS);
        let mut out = contact("BA", Some(Side::Blue), 0, widened);
        out.read_ms = t;
        hold(&mut picture, out);
        let orders = pilot.decide(&picture, &doctrine, t);
        assert_eq!(pilot.withdraw_until_ms, None);
        assert_eq!(heading(&orders), 90, "the tangent at the widened band");
    }

    #[test]
    fn after_the_fall_back_the_doctrine_still_steers() {
        let band = 300;
        let widened = widened_band(band);
        let fly = |stance: Stance| {
            let doctrine = Doctrine {
                stance: Stance::Orbit,
                sense: Sense::Ccw,
                band_m: band,
                ..doctrine()
            };
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 100));
            half_hull(&mut picture);
            pilot.decide(&picture, &doctrine, NOW);
            assert!(pilot.fallen_back);
            let t = NOW + WITHDRAW_MS;
            let mut at = contact("BA", Some(Side::Blue), 0, widened);
            at.read_ms = t;
            hold(&mut picture, at);
            let orders = pilot.decide(&picture, &Doctrine { stance, ..doctrine }, t);
            (orders.heading, orders.speed)
        };
        // Stand off at the widened band: at it, stopped facing the target.
        assert_eq!(fly(Stance::StandOff), (Some(0), Some(0)));
        // Close is flown as an orbit there (not straight in to 200 m).
        assert_eq!(fly(Stance::Close), (Some(90), Some(54)));
        // A doctrine withdraw turns away, ahead of the widened orbit.
        assert_eq!(fly(Stance::Withdraw), (Some(180), Some(54)));
        // A hunt flies at the widened band too: at it, round it.
        assert_eq!(fly(Stance::Hunt), (Some(90), Some(54)));
    }

    // -- cut AK.2: the hunt stance ---------------------------------------------------

    fn hunting(band_m: u32) -> Doctrine {
        Doctrine {
            stance: Stance::Hunt,
            sense: Sense::Ccw,
            band_m,
            ..doctrine()
        }
    }

    #[test]
    fn the_hunt_stance_drives_in_to_the_band_then_orbits() {
        let band = 300;
        let fly = |range: u32| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, range));
            let orders = pilot.decide(&picture, &hunting(band), NOW);
            (orders.heading, orders.speed)
        };
        // Outside the band and its slack: straight at it, full ahead.
        assert_eq!(fly(900), (Some(0), Some(54)));
        let slack = STAND_OFF_SLACK_M as u32;
        assert_eq!(fly(band + slack + 10), (Some(0), Some(54)));
        // At the band: the orbit's tangent; inside the slack, the orbit bent
        // toward the band, as `orbit` flies it.
        assert_eq!(fly(band), (Some(90), Some(54)));
        assert_eq!(
            fly(band + slack).0,
            Some(orbit_heading(0.0, f64::from(band + slack), band, 1.0))
        );
        assert_eq!(
            fly(band - 100).0,
            Some(orbit_heading(0.0, f64::from(band - 100), band, 1.0))
        );
        // A fresh friend is never hunted: nothing to fly at yet.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("RB", Some(Side::Red), 0, 900));
        assert_eq!(pilot.decide(&picture, &hunting(band), NOW).heading, None);
    }

    #[test]
    fn the_hunt_stance_with_nothing_held_hunts_the_trail_at_once() {
        // BA held fresh, then its row four seconds old: under an orbit
        // doctrine it is still circled; under a hunt it is hunted at once.
        let stale = |doctrine: &Doctrine| {
            let (mut picture, mut pilot) = flying_at(MID);
            hold(
                &mut picture,
                contact_from(MID, "BA", Some(Side::Blue), 0, 450),
            );
            pilot.decide(&picture, doctrine, NOW);
            assert!(pilot.hunt.is_none());
            let t = NOW + 1_000;
            if let Some(ba) = picture.contacts.get_mut("BA") {
                ba.age_ms = FRESH_PLOT_MS + 1_000;
                ba.read_ms = t;
            }
            const { assert!(FRESH_PLOT_MS + 1_000 <= HUNT_STALE_PLOT_MS) };
            read_at(&mut picture, MID, t);
            let orders = pilot.decide(&picture, doctrine, t);
            (pilot, orders)
        };
        // What is flown tells the two limits apart: the orbit's tangent round
        // the four-second row (BA due north at the band), not its last plot.
        let (pilot, orders) = stale(&doctrine());
        assert!(pilot.hunt.is_none(), "an orbit circles it");
        assert!(
            [90, 270].contains(&heading(&orders)),
            "the tangent: {orders:?}"
        );
        for target in [None, Some("BA".to_owned())] {
            let doctrine = Doctrine {
                target,
                ..hunting(450)
            };
            let (mut pilot, orders) = stale(&doctrine);
            let hunt = hunt_of(&pilot);
            assert_eq!(hunt.leg, HuntLeg::ToPlot);
            assert_eq!(hunt.trail.designator, "BA");
            // Toward its last plot, due north, full ahead — not round the row.
            assert_eq!((orders.heading, orders.speed), (Some(0), Some(54)));
            assert!(
                pilot
                    .take_notes()
                    .iter()
                    .any(|n| n.starts_with("Hunting BA: its last plot, "))
            );
        }
        // The row gone altogether: the same hunt, at once.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        pilot.decide(&picture, &hunting(450), NOW);
        picture.contacts.clear();
        let orders = pilot.decide(&picture, &hunting(450), NOW + 1_000);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::ToPlot);
        assert_eq!(heading(&orders), 0, "toward its last plot, due north");
        // With no trail, the search as ever: the power came up ten seconds
        // ago with nothing held since, more than five quiet seconds.
        let (picture, mut pilot) = flying_at(MID);
        assert!(NOW >= pilot.power_ms.expect("power") + SEARCH_AFTER_MS);
        let orders = pilot.decide(&picture, &hunting(450), NOW);
        assert!(pilot.hunt.is_none());
        assert!(orders.ask_spot_b, "the search");
    }

    #[test]
    fn a_doctrine_naming_a_row_too_old_to_fight_gives_way_to_a_fresh_hostile() {
        // BA held fresh (so trailed) due north, then its row four seconds old,
        // while BB is fresh 500 m due south: the doctrine still names BA.
        let fly = |doctrine: &Doctrine| {
            let (mut picture, mut pilot) = flying_at(MID);
            hold(
                &mut picture,
                contact_from(MID, "BA", Some(Side::Blue), 0, 450),
            );
            pilot.decide(&picture, doctrine, NOW);
            assert!(pilot.trailed.contains("BA"));
            let t = NOW + 1_000;
            if let Some(ba) = picture.contacts.get_mut("BA") {
                ba.age_ms = FRESH_PLOT_MS + 1_000;
                ba.read_ms = t;
            }
            let mut bb = contact_from(MID, "BB", Some(Side::Blue), 180, 500);
            bb.read_ms = t;
            hold(&mut picture, bb);
            read_at(&mut picture, MID, t);
            let orders = pilot.decide(&picture, doctrine, t);
            (pilot, orders)
        };
        // A hunt fights only a fresh plot: BA is too old, so the pilot flies
        // at BB (outside the band) rather than at nothing.
        let hunt = Doctrine {
            target: Some("BA".to_owned()),
            ..hunting(300)
        };
        let (pilot, orders) = fly(&hunt);
        assert!(pilot.hunt.is_none(), "BB is in sight");
        assert_eq!((orders.heading, orders.speed), (Some(180), Some(54)));
        // An orbit still fights a four-second row: BA is circled as named.
        let orbit = Doctrine {
            target: Some("BA".to_owned()),
            ..doctrine()
        };
        let (_, orders) = fly(&orbit);
        assert!(
            [90, 270].contains(&heading(&orders)),
            "round BA: {orders:?}"
        );
    }

    // -- cut AK.1: the held tank ---------------------------------------------------

    #[test]
    fn a_held_stop_orders_speed_again() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.own.heading_deg = 100;
        pilot.sent_speed(54);
        assert_eq!(decide(&picture, &mut pilot).speed, None, "already ordered");
        held_event(&mut picture, NOW);
        let orders = pilot.decide(&picture, &doctrine(), NOW + 10);
        assert_eq!(orders.heading, Some(whole(100.0 + HELD_TURN_DEG)));
        assert_eq!(orders.speed, Some(54), "the speed goes again");
        assert_eq!(pilot.held_from_deg, Some(100));
    }

    /// A `STATUS` read at `t`: held at 0 kph on `heading`, the contact read
    /// with it.
    fn held_read(picture: &mut Picture, t: u64, heading: u32) {
        picture.own.held = Some(super::super::picture::HeldCause::Impassable);
        picture.own.speed_kph = 0;
        picture.own.heading_deg = heading;
        picture.own.pos_read_ms = t;
        for contact in picture.contacts.values_mut() {
            contact.read_ms = t;
        }
    }

    #[test]
    fn a_tank_still_held_turns_further_from_where_it_stopped_then_backs_off() {
        // Orbiting BA counter-clockwise: the doctrine's heading is 090 or so,
        // never the stall's.
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        let stopped = 90;
        let start = |picture: &mut Picture, pilot: &mut Pilot| {
            hold(picture, contact_from(MID, "BA", Some(Side::Blue), 0, 450));
            picture.own.heading_deg = stopped;
            held_event(picture, NOW - 10);
            let orders = pilot.decide(picture, &doctrine, NOW);
            assert_eq!(
                orders.heading,
                Some(whole(f64::from(stopped) + HELD_TURN_DEG))
            );
            // The held turn is over; the tank still reads held: a stall opens.
            let t0 = NOW + HELD_FOR_MS;
            held_read(picture, t0, stopped + 45);
            pilot.decide(picture, &doctrine, t0);
            assert_eq!((pilot.stall_since_ms, pilot.stall_level), (Some(t0), 0));
            t0
        };
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        // A read inside `STALL_MS` does not escalate.
        held_read(&mut picture, t0 + STALL_MS - 1, 200);
        let orders = pilot.decide(&picture, &doctrine, t0 + STALL_MS - 1);
        assert_eq!(pilot.stall_level, 0);
        assert_eq!(orders.heading, Some(90), "the doctrine still steers");
        pilot.take_notes();
        // Each read `STALL_MS` on turns further from 090, whatever it read.
        let mut t = t0;
        for level in 1..=3_u8 {
            t += STALL_MS;
            held_read(&mut picture, t, 17 * u32::from(level));
            let orders = pilot.decide(&picture, &doctrine, t);
            let expected = whole(f64::from(stopped) + HELD_TURN_DEG * f64::from(level + 1));
            assert_eq!(pilot.stall_level, level);
            assert_eq!(orders.heading, Some(expected), "level {level}");
            assert_eq!(orders.speed, Some(54));
            assert_eq!(
                pilot.take_notes(),
                [format!("Held at 0 kph: turning to {expected:03}°.")]
            );
            // Kept against the doctrine between reads.
            let orders = pilot.decide(&picture, &doctrine, t + 10);
            assert_eq!(orders.heading, Some(expected));
        }
        // The fourth: backing off, astern, for `HELD_REVERSE_MS`.
        t += STALL_MS;
        held_read(&mut picture, t, 270);
        let orders = pilot.decide(&picture, &doctrine, t);
        assert_eq!((orders.heading, orders.speed), (None, Some(-27)));
        assert_eq!(pilot.take_notes(), ["Held at 0 kph: backing off."]);
        assert_eq!(pilot.stall_level, 4);
        let orders = pilot.decide(&picture, &doctrine, t + HELD_REVERSE_MS - 1);
        assert_eq!(orders.speed, Some(-27));
        let orders = pilot.decide(&picture, &doctrine, t + HELD_REVERSE_MS);
        assert_eq!(pilot.stall_level, 0, "closed when the back-off ends");
        assert_eq!(orders.speed, Some(54), "forward again");
        assert_eq!(orders.heading, Some(90), "the doctrine again");
        // A `STATUS` at speed closes the stall; its heading is kept
        // `HELD_FOR_MS` more to clear the ground.
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        let t = t0 + STALL_MS;
        held_read(&mut picture, t, 135);
        let turned = pilot.decide(&picture, &doctrine, t).heading;
        assert_eq!(
            turned,
            Some(whole(f64::from(stopped) + 2.0 * HELD_TURN_DEG))
        );
        let freed = t + 2_000;
        picture.own.held = None;
        picture.own.speed_kph = 54;
        picture.own.pos_read_ms = freed;
        pilot.decide(&picture, &doctrine, freed);
        assert_eq!(pilot.stall_level, 0);
        let orders = pilot.decide(&picture, &doctrine, freed + HELD_FOR_MS - 1);
        assert_eq!(orders.heading, turned, "still clearing the ground");
        let orders = pilot.decide(&picture, &doctrine, freed + HELD_FOR_MS);
        assert_ne!(orders.heading, turned, "the doctrine again");
        // A new stop is a new episode: its own held turn, the stall from 0.
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        let t = t0 + STALL_MS;
        held_read(&mut picture, t, 135);
        pilot.decide(&picture, &doctrine, t);
        assert_eq!(pilot.stall_level, 1);
        picture.own.heading_deg = 200;
        held_event(&mut picture, t + 500);
        let orders = pilot.decide(&picture, &doctrine, t + 600);
        assert_eq!(pilot.stall_level, 0);
        assert_eq!(pilot.held_from_deg, Some(200));
        assert_eq!(orders.heading, Some(whole(200.0 + HELD_TURN_DEG)));
        // Freed, then held again with no notice read (a re-board, say): the
        // new stall turns from the heading it is held on now, not from the
        // last episode's stop.
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        let freed = t0 + 1_000;
        picture.own.held = None;
        picture.own.speed_kph = 54;
        picture.own.pos_read_ms = freed;
        pilot.decide(&picture, &doctrine, freed);
        assert_eq!(pilot.held_from_deg, None, "the episode is over");
        let again = freed + 20_000;
        held_read(&mut picture, again, 220);
        pilot.decide(&picture, &doctrine, again);
        assert_eq!(pilot.held_from_deg, Some(220));
        held_read(&mut picture, again + STALL_MS, 220);
        let orders = pilot.decide(&picture, &doctrine, again + STALL_MS);
        assert_eq!(pilot.stall_level, 1);
        assert_eq!(orders.heading, Some(whole(220.0 + 2.0 * HELD_TURN_DEG)));
    }

    // -- cut AK.1: the fire clock --------------------------------------------------

    #[test]
    // The slack's floor is nothing (cut AL.1), which the lint reads as a
    // no-op; the rule stays written through its names.
    #[allow(clippy::unnecessary_min_or_max)]
    fn after_a_shot_the_weapon_waits_for_its_own_clock() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.lock = Some("BA".into());
        // An earlier row left the clock in the past (the smoke's case).
        picture.weapons = vec![Weapon {
            id: "cannon".into(),
            ready: false,
            rounds: 10,
            rounds_max: 40,
            read_ms: NOW - 5_000,
            cycle_ms: Some(6_000),
            next_ready_ms: Some(NOW - 1_000),
            slack_ms: crate::arena::picture::FIRE_SLACK_START_MS,
            ..Weapon::default()
        }];
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon"]);
        let a = NOW;
        picture.apply(a, &classify("Firing cannon."));
        let slack = picture.weapons[0].slack_ms;
        assert_eq!(
            slack,
            crate::arena::picture::FIRE_SLACK_START_MS
                .saturating_sub(crate::arena::picture::FIRE_SLACK_DOWN_MS)
                .max(crate::arena::picture::FIRE_SLACK_MIN_MS)
        );
        let gap = a + crate::arena::lane::FIRE_GAP_MS;
        assert!(!believed_ready(&picture.weapons[0], gap));
        assert!(pilot.decide(&picture, &doctrine(), gap).fire.is_empty());
        let due = a + 6_000 + slack;
        assert!(!believed_ready(&picture.weapons[0], due - 1));
        assert!(believed_ready(&picture.weapons[0], due));
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.read_ms = due;
        }
        assert_eq!(pilot.decide(&picture, &doctrine(), due).fire, ["cannon"]);
    }

    // -- cut AL.1: a lost gun, an empty gun, every gun in doubt -----------------

    /// **`fireable` skips a struck, an empty and a doubted gun** (cut AL.1):
    /// a gun the tank said it does not carry, a gun whose rounds the acks have
    /// counted down to none, and every gun while a lost section's `its weapons
    /// are gone` waits for `WEAPONSTATUS`.
    #[test]
    fn fireable_skips_a_struck_an_empty_and_a_doubted_gun() {
        let armed = || {
            let (mut picture, _) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.lock = Some("BA".into());
            picture.weapons = vec![weapon("cannon", true), weapon("machine_gun", true)];
            picture
        };
        let picture = armed();
        assert_eq!(
            fireable(&picture, "BA", Side::Blue, NOW),
            ["cannon", "machine_gun"]
        );
        // Struck: the no-such-weapon answer takes it out of the list.
        let mut picture = armed();
        picture.apply(
            NOW,
            &classify("That unit carries no weapon called `cannon`."),
        );
        assert_eq!(fireable(&picture, "BA", Side::Blue, NOW), ["machine_gun"]);
        // …and a gun the struck set names is never fired, whatever the list says.
        let mut picture = armed();
        picture.struck.insert("machine_gun".into());
        assert_eq!(fireable(&picture, "BA", Side::Blue, NOW), ["cannon"]);
        // Empty: every round the last row counted has been acked.
        let mut picture = armed();
        if let Some(mg) = picture.weapons.iter_mut().find(|w| w.id == "machine_gun") {
            mg.shots_since_read = mg.rounds;
        }
        assert_eq!(fireable(&picture, "BA", Side::Blue, NOW), ["cannon"]);
        // Doubted: nothing at all until `WEAPONSTATUS` answers.
        let mut picture = armed();
        picture.apply(
            NOW,
            &classify("The turret is destroyed: its weapons are gone."),
        );
        assert!(fireable(&picture, "BA", Side::Blue, NOW).is_empty());
        picture.apply(NOW + 100, &classify("Locked on BA, 62% to hit."));
        picture.apply(
            NOW + 100,
            &classify("machine_gun (mg)  ready  400 of 500 round(s)"),
        );
        picture.apply(NOW + 101, &classify("Power online."));
        assert_eq!(picture.weapons_doubt, None);
        assert_eq!(
            fireable(&picture, "BA", Side::Blue, NOW + 101),
            ["machine_gun"]
        );
    }
}
