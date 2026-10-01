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
//!   and a moving one out to all of it" (600 m) — so the default band,
//!   **450 m**, sits just outside the machines' stand-off and inside the
//!   stopped-tank sight: the pilot keeps a stopped machine in view. The
//!   machines stop to shoot, so the stopped tank's 480 m, less a margin for the
//!   scope's 50 m rounding and the tank's own drift, is the **ceiling** of every
//!   band the pilot flies ([`SIGHT_BAND_MAX_M`], 450 m), the fall back's
//!   included: the moving sight's 600 m is where it loses them;
//! * **"The facet you show decides what a hit costs you"** (`help cockpit`,
//!   "Seeing"): "from within 45 degrees of your bow it strikes the front", "a
//!   side is twice the target, 7 m of flank against 3.5 m of bow", the stock
//!   hull carries 400 in front, 250 a side and 150 behind, and a side shot
//!   lands on that side's track three times in ten — "Turn into the shot". So
//!   the pilot's default engagement (cut AL.2, [`Stance::Hold`]) **stops at the
//!   band with its bow on the hostile** and changes range only **along the
//!   line of fire**: ahead to close, in reverse to open ("a negative speed
//!   reverses"; "the stock tank makes 54 kph forward and 27 astern"). The bow
//!   is held within [`HOLD_BOW_DEG`] of the hostile, inside the help's 45°
//!   front arc. The orbit showed the machine a side the whole fight: of the
//!   hits the bot took in its own measured run, hull left 97, hull right 80,
//!   turret left 64 and right 60, the tracks 61 and 58, and the front only 14
//!   (hull) and 9 (turret). The range moves only past the band's slack and a
//!   hysteresis of half the scope's 50 m rounding ([`HOLD_HYSTERESIS_M`]), so a
//!   machine at the help's 420 m, read `400 m`, never sets the tank reversing
//!   and back;
//! * a side or rear hit turns the bow to the shooter ([`THREAT_TURN_MS`]): the
//!   struck facet is the help's 45-degree face (`left` 225..=315 degrees
//!   clockwise from the bow, `right` 45..=135, `rear` 135..=225), and a hostile
//!   is in that arc when its own row's relative bearing (`rel`, read every
//!   second) lies in it; the hit line says nothing of who fired, so with none
//!   in the arc the bow turns to the arc's centre, and with only the held
//!   target to blame it is taken as the shooter (the hold already faces it).
//!   A hit read after its turn would be over, one with nothing to blame while
//!   the current turn is not yet read by `STATUS`, and with one track gone one
//!   whose turn goes the dead way turn nothing; the turn's sentence is said
//!   when it is flown, never for a turn a held stop or a withdrawal outranks;
//! * the **orbit** is a stance a commander or `[arena] engagement = "orbit"`
//!   asks for, no longer the default: it circles toward the target's **rear**
//!   when the picture holds a track on it ("one caught from behind spends its
//!   thinnest" plate), and without a track **reverses** every 20 s. (That this
//!   spoils the machines' aim was not borne out: every one of the bot's own
//!   113 misses in its measured run read `the round went wide`, none `the
//!   target moved`.)
//! * "Lose one track and the tank is slowed and will not turn that way" — so
//!   with one track gone ([`live_turn`]) every turn the pilot makes on its own
//!   (the held turn, the stall's, the sweep's legs, the detour round held
//!   ground) goes the live way, and an orbit circles the way that turns the
//!   live way (a named change to the orbit, with one track gone only). Two
//!   signs are kept apart: the orbit's circling sense ([`Pilot::sense`], `+1`
//!   counter-clockwise round the target) and the compass turn sign
//!   ([`Pilot::turn_sign`], `+1` clockwise, the heading growing). The bot's own
//!   transcript shows the server turning a one-track tank the long, live way
//!   when ordered a heading on the dead side, about 30° a second, so a small
//!   correction toward the dead side is a whole circle: a heading whose turn
//!   from the [`Pilot::reference_heading`] is toward the dead side by at most
//!   [`ONE_TRACK_DEADBAND_DEG`] (the help's front arc) is not ordered at all —
//!   with one track gone the hold's bow may stand up to that far off the
//!   target on the dead side, still inside the front arc — and a larger one is
//!   ordered once and then no heading goes until `STATUS` reads it or
//!   [`ONE_TRACK_COMMIT_MS`] (a whole circle at that rate) has passed. "Or
//!   reverses to turn" is not built: how a one-track tank reverses is
//!   unobserved;
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
//!   at last backs off ("a negative speed reverses", `help cockpit`). A tank
//!   the pilot itself stopped or reverses is never "stalled": the watchdog
//!   opens only after a forward speed of the pilot's own choosing;
//! * held ground is **remembered** (cut AL.2, [`HeldMark`]): the bot's own
//!   transcript shows the search driving back into one slope ten times, 12 s
//!   apart. A stop on ground — `too steep`, `will not take the tank` or `the
//!   edge`, never `something is in the way` (another unit, which moves) —
//!   marks the point [`HELD_MARK_AHEAD_M`] ahead of where the tank stopped
//!   (behind it, when it stopped reversing), and a forward heading whose next
//!   [`HELD_MARK_NEAR_M`] passes within [`HELD_MARK_HALF_WIDTH_M`] of a mark
//!   more than [`HELD_MARK_CLEAR_M`] off is turned aside by
//!   [`HELD_MARK_DETOUR_DEG`] (up to three times) the pilot's turn way. A mark
//!   is **a point, not a bank**: a detour can meet the same bank further
//!   along, and that is a new stop and a new mark. A stop while reversing (a
//!   guard: no transcript holds one) runs no forward held turn — the tank
//!   stops with its bow kept — and a reverse whose ray behind the tank meets a
//!   mark stops instead;
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
//! back: **in reverse with its bow on the target** (cut AL.2: it used to turn
//! its stern, or its damaged flank, to the machine) for 10 s or until it has
//! opened to its widened band, whichever is first; the widened band is 150 m
//! wider than the doctrine's, never narrower than it and never past
//! [`SIGHT_BAND_MAX_M`]. After that it flies its doctrine again at the widened
//! band, `close` flown as `hold` there (a failing hull does not drive in to
//! 200 m).
//!
//! **One stance precedence** (`Pilot::motion`, cut AL.2), each step returning
//! when it applies: (1) the held turn (or, after a stop while reversing, a
//! stop with the bow kept), the back-off and an open stall; (2) the pilot's
//! own fall-back withdrawal, in reverse bow-on (the current heading at full
//! ahead with no target to face); (3) a doctrine `withdraw`, the same; (3b)
//! the turn into a side or rear hit, stopped, for [`THREAT_TURN_MS`] (never
//! under an `orbit` doctrine, and nothing when the threat is within
//! [`HOLD_BOW_DEG`] of a young target the pilot already faces); (4) the
//! doctrine's stance round a hostile whose plot is younger than
//! [`HUNT_STALE_PLOT_MS`]; (5) the hunt; (4 again) the doctrine's stance round
//! an older row never seen fresh; (6) the search. Every heading steps (2) to
//! (6) return at a forward speed is steered round held ground; every reverse
//! (the hold's, a withdrawal's, the back-off) whose ray behind the tank meets a
//! mark is a stop instead, the bow kept. The stances: `hold` stops at the band
//! bow-on ([`Pilot::hold`]); `close` is a hold at [`CLOSE_M`]; `hunt` drives at
//! a fresh hostile and holds at the band; `withdraw` backs away bow-on;
//! `orbit` circles as cut AK built it. A doctrine `withdraw` reaches step 3
//! only as a bounded hold on a failing hull (the lane enforces it, cut AK.2),
//! and it outranks the widened band after a fall back. A doctrine `hunt` (cut
//! AK.2) holds a hostile only while its plot is fresh ([`FRESH_PLOT_MS`]):
//! past that it hunts at once from the trail, and with no trail it searches.
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
//! [`HUNT_STALE_PLOT_MS`] (a guard: no live row read older than 2 s).
//!
//! AL.2's, in this module, within these bounds: [`HOLD_SLACK_M`] 50..=100,
//! [`THREAT_TURN_MS`] 2 000..=8 000, [`ONE_TRACK_DEADBAND_DEG`] 45..=90,
//! [`ONE_TRACK_COMMIT_MS`] 8 000..=16 000, [`HELD_MARK_NEAR_M`] 80..=200,
//! [`HELD_MARK_HALF_WIDTH_M`] 20..=60 and [`HELD_MARK_DETOUR_DEG`] 30..=90.
//! Not tunable: [`HOLD_BOW_DEG`] (a ruling: the help's 45° front arc less a
//! margin), [`HOLD_HYSTERESIS_M`] (half the scope's rounding),
//! [`HELD_MARK_AHEAD_M`], [`HELD_MARK_CLEAR_M`] (derived) and
//! [`HELD_MARKS_MAX`].
//!
//! **The tests admit less of two of them**, one constant moved at a time from
//! the built tree (the rest as built; every other AL.2 constant passes at both
//! ends of its bound):
//! `HELD_MARK_NEAR_M` 90..=200 (80 fails: the bot's own run-2 case, `091` from
//! (1 330, 1 052), meets that stop's point 89.7 m ahead, and the brief fixes
//! that geometry; 90, 120 built and 200 pass), and `HELD_MARK_HALF_WIDTH_M`
//! 25..=60 (at 20 the clear distance falls to 40 m, and a mark dead-reckoned
//! up to 2 s of driving past the true stop lies beyond it: the back-off from
//! that forward stop is then stopped by the stop's own mark, and a search
//! drifting on its dead reckoning slips past a mark's narrower line; 25, 28,
//! 30 built and 60 pass). Also tried at both ends and passing:
//! [`HOLD_SLACK_M`] (and 75), [`THREAT_TURN_MS`], [`ONE_TRACK_DEADBAND_DEG`]
//! (and 60), [`ONE_TRACK_COMMIT_MS`] and [`HELD_MARK_DETOUR_DEG`]. Two moved
//! together are untried.
//!
//! Every test asserts through these names, never their values; the one
//! exception is the lead's own bounds on the offline median gaps (1 200 ms for
//! the machine gun, 6 500 for the cannon), which a round must keep.
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
/// `Close` drives straight in to this range bow-on, then holds there (cut
/// AL.2: it used to circle there).
pub const CLOSE_M: f64 = 200.0;
/// **The hold's slack** (cut AL.2; renamed from `STAND_OFF_SLACK_M`): a hold
/// changing range stops once within this of its band. Tunable 50..=100.
pub const HOLD_SLACK_M: f64 = 50.0;
/// **The hold's hysteresis** (cut AL.2): a stopped hold changes range only
/// past the slack and this much more. Half the scope's rounding: a row's range
/// is "rounded to 50 m" (`help cockpit`), so a machine at the help's 420 m
/// reads `400 m`, and a threshold on that step would toggle the speed every
/// read. Not tunable.
pub const HOLD_HYSTERESIS_M: f64 = 25.0;
/// **The bow's tolerance** (cut AL.2): a hold faces within this of the
/// hostile, and a threat this close to the held target's bearing is the
/// target. A ruling, not tunable: "from within 45 degrees of your bow it
/// strikes the front" (`help cockpit`, "Seeing"), less a margin for
/// whole-degree bearings and a machine that moves.
pub const HOLD_BOW_DEG: u32 = 30;
/// **The turn into a side or rear hit** lasts this long after the hit (cut
/// AL.2). Tunable 2 000..=8 000.
pub const THREAT_TURN_MS: u64 = 4_000;
/// **One track gone** (cut AL.2): a heading toward the dead side by at most
/// this is not ordered. The help's front arc: a dead-side error inside it
/// still shows the front. Tunable 45..=90.
pub const ONE_TRACK_DEADBAND_DEG: u32 = 45;
/// …and after a larger one is ordered, no heading goes for this long unless
/// `STATUS` reads it first. The bot's own transcript turned a one-track tank
/// about 30° a second (307° to 164° counter-clockwise in 4.8 s), so a whole
/// circle is about 12 s. Tunable 8 000..=16 000.
pub const ONE_TRACK_COMMIT_MS: u64 = 12_000;
/// **Held ground** (cut AL.2): a mark is the point this far ahead of where the
/// tank stopped (behind it, stopped reversing). Not tunable.
pub const HELD_MARK_AHEAD_M: f64 = 20.0;
/// A forward heading is blocked by a mark at most this far ahead along it…
/// Tunable 80..=200.
pub const HELD_MARK_NEAR_M: f64 = 120.0;
/// …and within this of its line. Tunable 20..=60.
pub const HELD_MARK_HALF_WIDTH_M: f64 = 30.0;
/// A mark this close to the own tank does not block a forward heading: it is
/// the held turn's and the stall's business, so a tank at its own stop is not
/// boxed in. Derived, not tunable.
pub const HELD_MARK_CLEAR_M: f64 = HELD_MARK_AHEAD_M + HELD_MARK_HALF_WIDTH_M;
/// A blocked heading is turned aside this much at a time, up to three times.
/// Tunable 30..=90.
pub const HELD_MARK_DETOUR_DEG: f64 = 45.0;
/// The most marks a match keeps; the oldest goes first. Not tunable.
pub const HELD_MARKS_MAX: usize = 16;
/// The three `Held:` sentences that name ground (cut AL.2): only these are
/// marked. `Held: something is in the way.` is another unit, which moves, and
/// `Held: the tank is held.` names nothing.
pub const HELD_GROUND: [&str; 3] = [
    "Held: the slope ahead is too steep for the tank.",
    "Held: the ground ahead will not take the tank.",
    "Held: this is the edge of the arena.",
];
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

/// **What a hold is doing with the range** (cut AL.2, [`Pilot::hold`]):
/// stopped at the band, closing it ahead, or opening it in reverse — always
/// with the bow on the hostile. Remembered between decisions so the range
/// moves only past the hysteresis, and back to `Stopped` whenever the pilot
/// flies anything but a hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum HoldMotion {
    #[default]
    Stopped,
    Ahead,
    Astern,
}

impl HoldMotion {
    /// `stopped`, `ahead` or `astern` (the `arena-picture` record's word).
    pub fn as_str(self) -> &'static str {
        match self {
            HoldMotion::Stopped => "stopped",
            HoldMotion::Ahead => "ahead",
            HoldMotion::Astern => "astern",
        }
    }
}

/// **A compass turn** (cut AL.2): clockwise is the heading growing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Turn {
    Clockwise,
    CounterClockwise,
}

impl Turn {
    /// The compass turn sign: `+1` clockwise, `-1` counter-clockwise.
    pub fn compass_sign(self) -> f64 {
        match self {
            Turn::Clockwise => 1.0,
            Turn::CounterClockwise => -1.0,
        }
    }

    /// `cw` or `ccw` (the `arena-picture` record's word).
    pub fn as_str(self) -> &'static str {
        match self {
            Turn::Clockwise => "cw",
            Turn::CounterClockwise => "ccw",
        }
    }

    /// `clockwise` or `counter-clockwise`.
    fn words(self) -> &'static str {
        match self {
            Turn::Clockwise => "clockwise",
            Turn::CounterClockwise => "counter-clockwise",
        }
    }

    /// The track whose loss leaves this the only way to turn.
    fn gone_track(self) -> &'static str {
        match self {
            Turn::Clockwise => "left_track",
            Turn::CounterClockwise => "right_track",
        }
    }

    /// Whether a signed turn `s` ([`signed_turn`], positive clockwise) goes
    /// the dead way when this is the only live one.
    fn is_dead(self, s: i64) -> bool {
        match self {
            Turn::Clockwise => s < 0,
            Turn::CounterClockwise => s > 0,
        }
    }
}

/// **The only way a one-track tank turns** (cut AL.2): exactly one of the own
/// sections `left_track` and `right_track` reads `destroyed` → the other way
/// ("Lose one track and the tank is slowed and will not turn that way", `help
/// cockpit`; the bot's own transcript turned a right-track tank the long way,
/// counter-clockwise). Both or neither → `None`.
pub fn live_turn(picture: &Picture) -> Option<Turn> {
    let gone = |track: &str| {
        picture
            .own
            .sections
            .get(track)
            .is_some_and(|section| section.condition == "destroyed")
    };
    match (gone("left_track"), gone("right_track")) {
        (true, false) => Some(Turn::Clockwise),
        (false, true) => Some(Turn::CounterClockwise),
        _ => None,
    }
}

/// **Held ground** (cut AL.2): where the tank stopped on ground it could not
/// enter, the heading it stopped on, whether it was reversing, and the point
/// it could not enter — [`HELD_MARK_AHEAD_M`] ahead of it, or behind it when
/// it stopped reversing. Positions, a heading and a direction only: a mark
/// names no unit. A point, not a bank.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeldMark {
    pub at_m: (f64, f64),
    pub heading_deg: u32,
    pub astern: bool,
    pub blocked_m: (f64, f64),
}

/// The pilot's own state. One per match.
#[derive(Clone, Debug, Serialize)]
pub struct Pilot {
    /// **The orbit's circling sense**: `+1` counter-clockwise round the target,
    /// `-1` clockwise. Not a compass turn: the held turn, the stall, the sweep
    /// and the detour turn by [`Pilot::turn_sign`] (cut AL.2).
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
    /// **The hold's range motion** (cut AL.2; [`Pilot::hold`]).
    pub hold_motion: HoldMotion,
    /// A hold was flown this decision (else `hold_motion` goes back to
    /// `Stopped`).
    #[serde(skip)]
    hold_flown: bool,
    /// **The turn into a side or rear hit** (cut AL.2): the newest facet hit
    /// already read…
    pub threat_seen_ms: u64,
    /// …and the bearing to turn to, until when.
    pub threat: Option<(f64, u64)>,
    /// The threat's `Turning into the shot: …` sentence, said only when step
    /// 3b first flies the turn: a turn outranked for its whole time (a held
    /// stop, the back-off, a withdrawal) or dropped is never said.
    #[serde(skip)]
    threat_note: Option<String>,
    /// **One track gone** (cut AL.2): a dead-side heading ordered, and when;
    /// no heading goes until `STATUS` reads it or [`ONE_TRACK_COMMIT_MS`]
    /// passes.
    pub turn_commit: Option<(u32, u64)>,
    /// The dead-side heading this decision would order, committed when the
    /// lane types it.
    #[serde(skip)]
    turn_wanted: Option<u32>,
    /// The one-track sentence has been said this match.
    pub one_track_said: bool,
    /// **Held ground** (cut AL.2), oldest first, at most [`HELD_MARKS_MAX`].
    pub held_marks: Vec<HeldMark>,
    /// The newest `Held:` came while reversing: step 1 stops the tank, the
    /// bow kept, instead of the forward held turn.
    pub held_astern: bool,
    /// A heading is being steered round held ground now.
    pub detour: bool,
    /// The speed the pilot's own motion last chose (not `SPEED 999`, and not
    /// forgotten with an unanswered order): the stall opens only after a
    /// forward one.
    pub last_chosen_speed: Option<i32>,
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
            hold_motion: HoldMotion::Stopped,
            hold_flown: false,
            threat_seen_ms: 0,
            threat: None,
            threat_note: None,
            turn_commit: None,
            turn_wanted: None,
            one_track_said: false,
            held_marks: Vec::new(),
            held_astern: false,
            detour: false,
            last_chosen_speed: None,
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

/// The signed turn from one whole heading to another, in `-179..=180`: positive
/// is clockwise (the heading growing).
fn signed_turn(from: u32, to: u32) -> i64 {
    let d = (i64::from(to) - i64::from(from)).rem_euclid(360);
    if d > 180 { d - 360 } else { d }
}

/// Whether the ray from `o` along `heading` meets `point`: at most
/// [`HELD_MARK_NEAR_M`] ahead along it and within [`HELD_MARK_HALF_WIDTH_M`]
/// of its line — and, with a `clear` distance, only a point more than that
/// from `o`.
fn ray_meets(o: (f64, f64), heading: f64, point: (f64, f64), clear: Option<f64>) -> bool {
    let (dx, dy) = (point.0 - o.0, point.1 - o.1);
    if clear.is_some_and(|clear| dx.hypot(dy) <= clear) {
        return false;
    }
    let r = heading.to_radians();
    let ahead = dx * r.sin() + dy * r.cos();
    let aside = (dx * r.cos() - dy * r.sin()).abs();
    (0.0..=HELD_MARK_NEAR_M).contains(&ahead) && aside <= HELD_MARK_HALF_WIDTH_M
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
        // the pilot's running one. With one track gone the compass turns go the
        // live way instead (cut AL.2): the turn sign is taken once, here.
        match doctrine.sense {
            Sense::Cw => self.sense = -1,
            Sense::Ccw => self.sense = 1,
            Sense::Auto => {}
        }
        let live = live_turn(picture);
        let sign = self.turn_sign(picture);
        self.turn_wanted = None;
        if let Some(turn) = live
            && !self.one_track_said
        {
            self.one_track_said = true;
            self.note(format!(
                "The {} is gone: turning {} only.",
                turn.gone_track(),
                turn.words()
            ));
        }
        self.observe(picture, doctrine, other, sign, now_ms);

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
        self.take_hit(picture, doctrine, other, target.as_deref(), live, now_ms);
        self.follow(picture, doctrine, other, target.as_deref(), sign, now_ms);
        let full = picture.own.max_ahead_kph.unwrap_or(UNKNOWN_FULL_KPH);
        let motion = self.motion(
            picture,
            doctrine,
            target.as_deref(),
            full,
            live,
            sign,
            now_ms,
        );
        let (heading, mut speed, ask) = motion;
        orders.ask_spot_b = ask;
        if !self.ceilings_asked {
            speed = Some(CEILING_ASK_KPH);
        }
        // One track gone (cut AL.2): a small turn toward the dead side is not
        // ordered, a large one once and then nothing until it is read.
        let heading = heading.and_then(|deg| match live {
            Some(turn) => self.one_track(picture, turn, whole(deg), now_ms),
            None => Some(whole(deg)),
        });
        if let Some(deg) = heading {
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

    /// The lane typed `HEADING deg` (with one track gone, a dead-side heading
    /// the pilot asked for is committed to from now, cut AL.2).
    pub fn sent_heading(&mut self, deg: u32, now_ms: u64) {
        self.last_heading_order = Some((deg, now_ms));
        if self.turn_wanted == Some(deg) {
            self.turn_wanted = None;
            self.turn_commit = Some((deg, now_ms));
        }
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

    /// **Back at the controls** (cut AL.2; the lane calls it where it forgets
    /// the heading and the speed): the one-track commit and the turn into a hit
    /// are dropped too, and the newest hit in `picture` is taken as read. The
    /// machine flew the tank meanwhile and it "is yours again where it
    /// stands", so the heading a commit waits for and the bearing a hit came
    /// from — a facet of the hull as it lay then — are not the tank's now.
    pub fn forget_turns(&mut self, picture: &Picture) {
        self.turn_commit = None;
        self.turn_wanted = None;
        self.threat = None;
        self.threat_note = None;
        if let Some(hit) = &picture.own.last_hit {
            self.threat_seen_ms = self.threat_seen_ms.max(hit.at_ms);
        }
    }

    /// **The compass turn sign** (cut AL.2): `+1` clockwise (the heading
    /// growing), `-1` counter-clockwise. With one track gone, the live turn's
    /// ([`live_turn`]); else the orbit's sense as a number, exactly as cut AK
    /// turned by it. Not the orbit's circling sense: the held turn, the
    /// stall's turns, the sweep's legs and the detour round held ground turn
    /// by this.
    pub fn turn_sign(&self, picture: &Picture) -> f64 {
        match live_turn(picture) {
            Some(turn) => turn.compass_sign(),
            None => f64::from(self.sense),
        }
    }

    /// **The reference heading** (cut AL.2): the last heading ordered when it
    /// went after the last `STATUS` read (at or after its position's time),
    /// else the heading that `STATUS` read. What a one-track turn, the arc of
    /// a side hit and a heading-less reverse are measured from.
    pub fn reference_heading(&self, picture: &Picture) -> u32 {
        match self.last_heading_order {
            Some((deg, at)) if at >= picture.own.pos_read_ms => deg,
            _ => picture.own.heading_deg,
        }
    }

    /// **Whether a forward heading `h` from `o` meets held ground**: some
    /// mark's blocked point more than [`HELD_MARK_CLEAR_M`] from `o` lies at
    /// most [`HELD_MARK_NEAR_M`] ahead along `h` and within
    /// [`HELD_MARK_HALF_WIDTH_M`] of its line.
    pub fn blocked(&self, o: (f64, f64), h: f64) -> bool {
        self.held_marks
            .iter()
            .any(|mark| ray_meets(o, h, mark.blocked_m, Some(HELD_MARK_CLEAR_M)))
    }

    /// **Whether a reverse with the bow on `bow` from `o` meets held ground**:
    /// the ray behind the tank (`bow + 180`). A mark taken while reversing is
    /// met however near (it lies just behind the tank that stopped on it); a
    /// mark taken driving ahead keeps its clear distance, as for a forward
    /// heading, so the back-off from a forward stop is not stopped by its own
    /// stop's mark.
    pub fn blocked_behind(&self, o: (f64, f64), bow: f64) -> bool {
        self.held_marks.iter().any(|mark| {
            let clear = (!mark.astern).then_some(HELD_MARK_CLEAR_M);
            ray_meets(o, bow + 180.0, mark.blocked_m, clear)
        })
    }

    /// **Steer round held ground** (cut AL.2): a forward heading `h` from `o`
    /// that [`Pilot::blocked`] is turned `k ×` [`HELD_MARK_DETOUR_DEG`] by the
    /// compass turn sign, `k` = 1, 2, 3, the first that is clear winning; all
    /// three blocked, `h` is left as it is. Says so when a detour starts.
    pub fn steer_round(&mut self, o: (f64, f64), h: f64, sign: f64) -> f64 {
        if !self.blocked(o, h) {
            self.detour = false;
            return h;
        }
        for k in 1..=3_u8 {
            let aside = norm(h + f64::from(k) * HELD_MARK_DETOUR_DEG * sign);
            if !self.blocked(o, aside) {
                if !self.detour {
                    self.note(format!(
                        "Steering round held ground: {:03}° for {:03}°.",
                        whole(aside),
                        whole(h)
                    ));
                }
                self.detour = true;
                return aside;
            }
        }
        self.detour = false;
        h
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
    fn observe(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        sign: f64,
        now_ms: u64,
    ) {
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
        if let Some((held, text)) = picture
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Held && event.at_ms > self.held_seen_ms)
            .max_by_key(|event| event.at_ms)
            .map(|event| (event.at_ms, event.text.clone()))
        {
            self.held_seen_ms = held;
            // The heading the tank was stopped on: every further turn is
            // measured from it, never from a later reading.
            self.held_from_deg = Some(picture.own.heading_deg);
            // Stopped while reversing (cut AL.2; read before the speed is
            // forgotten below): no forward held turn — step 1 stops the tank
            // with its bow kept.
            let astern = self.last_speed_order.is_some_and(|kph| kph < 0);
            self.mark_ground(picture, &text, astern, now_ms);
            self.held_astern = astern;
            self.held_heading =
                (!astern).then(|| whole(f64::from(picture.own.heading_deg) + HELD_TURN_DEG * sign));
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
        self.watch_stall(picture, sign, now_ms);
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
    /// [`HELD_FOR_MS`] more to clear the ground. A stall opens only while the
    /// speed the pilot's own motion last chose was ahead (cut AL.2): a tank
    /// the pilot itself stopped, or reverses, is never "stalled". The turns go
    /// by the compass turn sign `sign` ([`Pilot::turn_sign`]).
    fn watch_stall(&mut self, picture: &Picture, sign: f64, now_ms: u64) {
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
                if now_ms >= self.held_until_ms
                    && now_ms >= self.reverse_until_ms
                    && self.last_chosen_speed.is_some_and(|kph| kph > 0)
                {
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
                        f64::from(from) + HELD_TURN_DEG * f64::from(self.stall_level + 1) * sign,
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

    /// **Mark held ground** (cut AL.2): a `Held:` that names ground
    /// ([`HELD_GROUND`]), with a known position, is marked where the tank is
    /// now, on the heading it stopped on, the blocked point
    /// [`HELD_MARK_AHEAD_M`] ahead of it — or behind it, `astern`. At most
    /// [`HELD_MARKS_MAX`] are kept, the oldest dropped.
    fn mark_ground(&mut self, picture: &Picture, text: &str, astern: bool, now_ms: u64) {
        if !HELD_GROUND.contains(&text) {
            return;
        }
        let Some(at_m) = picture.own_pos_at(now_ms) else {
            return;
        };
        let heading_deg = picture.own.heading_deg;
        let way = f64::from(heading_deg) + if astern { 180.0 } else { 0.0 };
        self.held_marks.push(HeldMark {
            at_m,
            heading_deg,
            astern,
            blocked_m: along(at_m, way, HELD_MARK_AHEAD_M),
        });
        if self.held_marks.len() > HELD_MARKS_MAX {
            self.held_marks.remove(0);
        }
        self.note(format!(
            "Held ground marked at {:.0} m east, {:.0} m north, heading {heading_deg:03}°.",
            at_m.0, at_m.1
        ));
    }

    /// **A side or rear hit turns the bow to the shooter** (cut AL.2), read
    /// from the picture's newest facet hit ([`super::picture::Own::last_hit`])
    /// once. A `front` or `top` hit does nothing, and nor does any hit under an
    /// `orbit` doctrine. The struck arc is the help's 45-degree face of the
    /// facet, degrees clockwise from the bow (`left` 225..=315, `right`
    /// 45..=135, `rear` 135..=225); a hostile is in it when its own row's
    /// relative bearing lies in it (a live hostile with a row, not a wreck,
    /// its plot no older than [`HUNT_STALE_PLOT_MS`]). The threat is the
    /// nearest such hostile other than the held target; with a young target
    /// held and none other in the arc the target is taken as the shooter (the
    /// hold already faces it: any other turn is dropped); with none held and
    /// none in the arc, the arc's centre from the reference heading (the line
    /// says nothing of who fired). A newer hit re-arms the turn only when its
    /// threat is more than [`HOLD_BOW_DEG`] from the current one, so two
    /// shooters in a crossfire do not thrash it.
    ///
    /// Three hits turn nothing (fixes after review): one read when its turn
    /// would already be over (read late, after a stretch immobile or
    /// disabled); one with nothing to blame while the current turn's heading
    /// is ordered and not yet read by `STATUS` — the hull is somewhere in the
    /// turn, so the arc's centre from the ordered heading would send the bow
    /// round another quarter, away from the shooter, at every hit; and, with
    /// one track gone ([`live_turn`]), one whose turn goes the dead way — the
    /// server turns that the long, live way round, longer than the turn lasts
    /// and showing a side and the stern to the target on the way (within
    /// [`ONE_TRACK_DEADBAND_DEG`] it would not be ordered at all, and the
    /// front already faces it). The sentence is said when step 3b first flies
    /// the turn, never when a higher step outranks it for its whole time.
    #[allow(clippy::too_many_arguments)]
    fn take_hit(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        target: Option<&str>,
        live: Option<Turn>,
        now_ms: u64,
    ) {
        let Some(hit) = picture.own.last_hit.as_ref() else {
            return;
        };
        if hit.at_ms <= self.threat_seen_ms {
            return;
        }
        self.threat_seen_ms = hit.at_ms;
        if doctrine.stance == Stance::Orbit || now_ms >= hit.at_ms.saturating_add(THREAT_TURN_MS) {
            return;
        }
        let (arc, centre) = match hit.facet.as_str() {
            "left" => (225..=315, 270),
            "right" => (45..=135, 90),
            "rear" => (135..=225, 180),
            _ => return,
        };
        let Some(own) = picture.own_pos_at(now_ms) else {
            return;
        };
        let young = target
            .and_then(|t| picture.contacts.get(t))
            .filter(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine));
        let shooter = picture
            .contacts
            .values()
            .filter(|c| {
                self.live_hostile(c, other)
                    && c.range_m.is_some()
                    && c.age_ms <= HUNT_STALE_PLOT_MS
                    && arc.contains(&c.rel_deg)
                    && young.is_none_or(|t| t.designator != c.designator)
            })
            .min_by_key(|c| c.range_m.unwrap_or(u32::MAX));
        let (bearing, facing) = match (shooter, young) {
            (Some(c), _) => {
                let bearing = target_pos(picture, c, now_ms)
                    .map_or(f64::from(c.bearing_deg), |pos| bearing_deg(own, pos));
                (bearing, c.designator.clone())
            }
            (None, Some(t)) => {
                // The held target is the shooter: the hold already faces it.
                if let (Some((current, _)), Some(pos)) =
                    (self.threat, target_pos(picture, t, now_ms))
                    && angle_between(whole(current), whole(bearing_deg(own, pos))) > HOLD_BOW_DEG
                {
                    self.threat = None;
                    self.threat_note = None;
                }
                return;
            }
            (None, None) => {
                // Mid-turn, with the turn's heading ordered and not yet read:
                // where the hull lies is not known, so the arc is not either.
                if self.threat.is_some_and(|(_, until)| now_ms < until)
                    && self
                        .last_heading_order
                        .is_some_and(|(_, at)| at >= picture.own.pos_read_ms)
                {
                    return;
                }
                let bearing = norm(f64::from(self.reference_heading(picture)) + f64::from(centre));
                (bearing, format!("{:03}°", whole(bearing)))
            }
        };
        // One track gone: a turn the dead way is the long way round (a turn of
        // exactly 180° is as long either way, and goes).
        if let Some(turn) = live {
            let s = signed_turn(self.reference_heading(picture), whole(bearing));
            if turn.is_dead(s) && s.unsigned_abs() < 180 {
                return;
            }
        }
        if let Some((current, until)) = self.threat
            && now_ms < until
            && angle_between(whole(current), whole(bearing)) <= HOLD_BOW_DEG
        {
            return;
        }
        self.threat = Some((norm(bearing), hit.at_ms.saturating_add(THREAT_TURN_MS)));
        self.threat_note = Some(format!(
            "Turning into the shot: hit on the {}'s {} facet; facing {facing}.",
            hit.plate, hit.facet
        ));
    }

    /// **One track gone** (cut AL.2): whether to order `deg` now. While a
    /// dead-side heading is committed, nothing, until a `STATUS` read after it
    /// reads within [`HEADING_STEP_DEG`] of it or [`ONE_TRACK_COMMIT_MS`] has
    /// passed. Else a heading whose turn from the reference heading is toward
    /// the dead side by at most [`ONE_TRACK_DEADBAND_DEG`] is not ordered; one
    /// by more is ordered and committed to once the lane types it; a live-side
    /// heading is ordered as ever.
    fn one_track(&mut self, picture: &Picture, turn: Turn, deg: u32, now_ms: u64) -> Option<u32> {
        if let Some((committed, at)) = self.turn_commit {
            let read = picture.own.pos_read_ms > at
                && angle_between(picture.own.heading_deg, committed) <= HEADING_STEP_DEG;
            if !read && now_ms < at.saturating_add(ONE_TRACK_COMMIT_MS) {
                return None;
            }
            self.turn_commit = None;
        }
        let s = signed_turn(self.reference_heading(picture), deg);
        if !turn.is_dead(s) {
            return Some(deg);
        }
        if s.unsigned_abs() <= u64::from(ONE_TRACK_DEADBAND_DEG) {
            return None;
        }
        self.turn_wanted = Some(deg);
        Some(deg)
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
        sign: f64,
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
            self.next_leg(own, sign, now_ms);
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
    /// quarter turn each by the compass turn sign `sign` (cut AL.2: the live
    /// way with one track gone) → the end (the trail dropped, the search next).
    fn next_leg(&mut self, own: (f64, f64), sign: f64, now_ms: u64) {
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
                    heading_deg: norm(hunt.course_deg + 90.0 * sign),
                })
            }
            HuntLeg::Sweep {
                leg, heading_deg, ..
            } => (leg.saturating_add(1) < HUNT_SWEEP_LEGS).then(|| HuntLeg::Sweep {
                leg: leg.saturating_add(1),
                origin: clamp_arena(own),
                heading_deg: norm(heading_deg + 90.0 * sign),
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
    fn hunt_heading(&mut self, own: (f64, f64), sign: f64, now_ms: u64) -> Option<f64> {
        // Each pass either returns or moves the hunt one leg on (or ends it).
        for _ in 0..=usize::from(HUNT_SWEEP_LEGS) + 2 {
            let hunt = self.hunt.as_ref()?;
            match hunt.leg {
                HuntLeg::Along if now_ms.saturating_sub(hunt.trail.plot_ms) > HUNT_TRAIL_MS => {
                    self.next_leg(own, sign, now_ms);
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
                self.next_leg(own, sign, now_ms);
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
                    self.next_leg(own, sign, now_ms);
                    continue;
                }
                // Caught up with it, or level with it and at most the arrival
                // ahead: drive on along the course rather than turn back to it
                // — never past the edge (the course's own next
                // `HUNT_ARRIVE_M` would cross it: that is the edge reached).
                if d <= HUNT_ARRIVE_M || (ahead >= 0.0 && aside <= HUNT_ARRIVE_M) {
                    if past_the_edge(along(own, hunt.course_deg, HUNT_ARRIVE_M)) {
                        self.next_leg(own, sign, now_ms);
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
    /// precedence of the module head. A hold not flown this decision goes
    /// back to `Stopped`, and the speed chosen is remembered for the stall
    /// watchdog (cut AL.2).
    #[allow(clippy::too_many_arguments)]
    fn motion(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        live: Option<Turn>,
        sign: f64,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        self.hold_flown = false;
        let out = self.motion_steps(picture, doctrine, target, full, live, sign, now_ms);
        if !self.hold_flown {
            self.hold_motion = HoldMotion::Stopped;
        }
        if let Some(kph) = out.1 {
            self.last_chosen_speed = Some(kph);
        }
        out
    }

    /// The steps of the precedence. Step 1's own headings are never steered
    /// round held ground; every heading steps 2 to 6 return at a forward speed
    /// is, and every reverse they return whose ray behind the tank meets held
    /// ground is a stop with the bow kept (cut AL.2).
    #[allow(clippy::too_many_arguments)]
    fn motion_steps(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        live: Option<Turn>,
        sign: f64,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        let Some(own) = picture.own_pos_at(now_ms) else {
            self.detour = false;
            return (None, None, false);
        };
        let astern = picture.own.max_astern_kph.unwrap_or(UNKNOWN_ASTERN_KPH);
        if let Some(step_one) = self.step_one(picture, own, full, astern, now_ms) {
            self.detour = false;
            return step_one;
        }
        let (heading, speed, ask) = self.stance_motion(
            picture, doctrine, target, full, astern, live, sign, own, now_ms,
        );
        match (heading, speed) {
            (Some(h), Some(kph)) if kph > 0 => (Some(self.steer_round(own, h, sign)), speed, ask),
            (_, Some(kph)) if kph < 0 => {
                self.detour = false;
                let bow = heading.unwrap_or_else(|| f64::from(self.reference_heading(picture)));
                if self.blocked_behind(own, bow) {
                    (heading, Some(0), ask)
                } else {
                    (heading, speed, ask)
                }
            }
            _ => {
                self.detour = false;
                (heading, speed, ask)
            }
        }
    }

    /// Step 1: a `Held:` notice outranks everything for its three seconds —
    /// the held turn, or after a stop while reversing a stop with the bow kept
    /// (cut AL.2); then the back-off (a stop instead when the ray behind the
    /// tank meets held ground); then a stall's turn while it is open, and after
    /// it frees the tank until the ground is cleared.
    fn step_one(
        &mut self,
        picture: &Picture,
        own: (f64, f64),
        full: i32,
        astern: i32,
        now_ms: u64,
    ) -> Option<(Option<f64>, Option<i32>, bool)> {
        if now_ms < self.held_until_ms {
            if self.held_astern {
                return Some((None, Some(0), false));
            }
            if let Some(held) = self.held_heading {
                return Some((Some(f64::from(held)), Some(full), false));
            }
        }
        if now_ms < self.reverse_until_ms {
            let bow = f64::from(self.reference_heading(picture));
            let kph = if self.blocked_behind(own, bow) {
                0
            } else {
                -astern
            };
            return Some((None, Some(kph), false));
        }
        if let Some(stall) = self.stall_heading {
            if self.stall_level >= 1 || now_ms < self.stall_clear_until_ms {
                return Some((Some(f64::from(stall)), Some(full), false));
            }
            self.stall_heading = None;
        }
        None
    }

    /// Steps 2 to 6 of the precedence.
    #[allow(clippy::too_many_arguments)]
    fn stance_motion(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        astern: i32,
        live: Option<Turn>,
        sign: f64,
        own: (f64, f64),
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        let contact = target.and_then(|t| picture.contacts.get(t));
        let aim = contact.and_then(|c| target_pos(picture, c, now_ms));
        // Every withdrawal backs away with the bow on the target (cut AL.2);
        // with no target to face, the current heading at full speed.
        let back = |aim: Option<(f64, f64)>| match aim {
            Some(aim) => (Some(bearing_deg(own, aim)), Some(-astern), false),
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
                return back(aim);
            }
        }
        // 3. A doctrine `withdraw`, ahead of the widened band.
        if doctrine.stance == Stance::Withdraw {
            self.orbit_since_ms = None;
            return back(aim);
        }
        // 3b. The turn into a side or rear hit, stopped (cut AL.2).
        if let Some(threat) = self.threat_turn(doctrine, contact, aim, own, now_ms) {
            self.orbit_since_ms = None;
            // Said now that the turn is flown, not when the hit armed it.
            if let Some(note) = self.threat_note.take() {
                self.note(note);
            }
            return (Some(threat), Some(0), false);
        }
        // 4. A hostile held young: the doctrine's stance (a `hunt` doctrine's
        // only while its plot is fresh).
        if let (Some(contact), Some(aim)) = (contact, aim)
            && contact.age_ms <= held_limit_ms(doctrine)
        {
            return self.engage(doctrine, contact, aim, own, full, astern, live, now_ms);
        }
        // 5. The hunt, before any stance round a stale row.
        if self.hunt.is_some()
            && let Some(heading) = self.hunt_heading(own, sign, now_ms)
        {
            self.orbit_since_ms = None;
            return (Some(heading), Some(full), false);
        }
        // 4, for an older row the pilot never held fresh (nothing to hunt).
        if let (Some(contact), Some(aim)) = (contact, aim)
            && !self.trailed.contains(&contact.designator)
        {
            return self.engage(doctrine, contact, aim, own, full, astern, live, now_ms);
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

    /// **Step 3b** (cut AL.2): the threat's bearing while its turn lasts and
    /// the doctrine is not an orbit — but nothing when a young target is held
    /// within [`HOLD_BOW_DEG`] of it (the hold already faces it).
    fn threat_turn(
        &mut self,
        doctrine: &Doctrine,
        contact: Option<&Contact>,
        aim: Option<(f64, f64)>,
        own: (f64, f64),
        now_ms: u64,
    ) -> Option<f64> {
        let (bearing, until) = self.threat?;
        if now_ms >= until || doctrine.stance == Stance::Orbit {
            self.threat = None;
            self.threat_note = None;
            return None;
        }
        if let (Some(contact), Some(aim)) = (contact, aim)
            && contact.age_ms <= held_limit_ms(doctrine)
            && angle_between(whole(bearing), whole(bearing_deg(own, aim))) <= HOLD_BOW_DEG
        {
            return None;
        }
        Some(bearing)
    }

    /// The doctrine's stance round a held hostile (cut AL.2): `orbit` as cut AK
    /// built it; `hold` and `hunt` a hold at the band; `close` a hold at
    /// [`CLOSE_M`]; `withdraw` a reverse with the bow on. After the fall back,
    /// at the widened band, and `close` flown as `hold` there.
    #[allow(clippy::too_many_arguments)]
    fn engage(
        &mut self,
        doctrine: &Doctrine,
        contact: &Contact,
        aim: (f64, f64),
        own: (f64, f64),
        full: i32,
        astern: i32,
        live: Option<Turn>,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        // A hostile is held: the next search starts over.
        self.search = SearchLeg::ToB;
        let (stance, band) = if self.fallen_back {
            let stance = match doctrine.stance {
                Stance::Close => Stance::Hold,
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
                Some(self.orbit(doctrine, contact, aim, own, d, beta, band, live, now_ms)),
                Some(full),
                false,
            ),
            Stance::Hold | Stance::Hunt => self.hold(beta, d, band, full, astern),
            Stance::Close => self.hold(beta, d, CLOSE_M, full, astern),
            Stance::Withdraw => {
                self.orbit_since_ms = None;
                (Some(beta), Some(-astern), false)
            }
        }
    }

    /// **The hold** (cut AL.2): the bow on the hostile at bearing `beta`
    /// always, and the range changed only along that line. From `Stopped` it
    /// closes (`full` ahead) once `d` is past `band` by [`HOLD_SLACK_M`] and
    /// [`HOLD_HYSTERESIS_M`], and opens (`astern`, in reverse) once short of
    /// it by as much; closing stops once within `band + HOLD_SLACK_M`, opening
    /// once out at `band − HOLD_SLACK_M`. At the default 450 m band the tank
    /// reverses only under 375 m and stops reversing at 400 m, so a machine at
    /// the help's 420 m, read at the scope's `400 m`, is a stop.
    pub fn hold(
        &mut self,
        beta: f64,
        d: f64,
        band: f64,
        full: i32,
        astern: i32,
    ) -> (Option<f64>, Option<i32>, bool) {
        self.orbit_since_ms = None;
        self.hold_flown = true;
        let leave = HOLD_SLACK_M + HOLD_HYSTERESIS_M;
        // Closing or opening stops at the slack; a stop then moves again only
        // past the hysteresis (both in one decision when the range jumped).
        if (self.hold_motion == HoldMotion::Ahead && d <= band + HOLD_SLACK_M)
            || (self.hold_motion == HoldMotion::Astern && d >= band - HOLD_SLACK_M)
        {
            self.hold_motion = HoldMotion::Stopped;
        }
        if self.hold_motion == HoldMotion::Stopped {
            if d > band + leave {
                self.hold_motion = HoldMotion::Ahead;
            } else if d < band - leave {
                self.hold_motion = HoldMotion::Astern;
            }
        }
        let kph = match self.hold_motion {
            HoldMotion::Stopped => 0,
            HoldMotion::Ahead => full,
            HoldMotion::Astern => -astern,
        };
        (Some(beta), Some(kph), false)
    }

    /// The orbit's heading: the tangent at the band, bent up to 60° toward the
    /// target when outside it and away when inside, reached 150 m off the band.
    /// With one track gone the circling sense, after the orbit's own choice, is
    /// the one that turns the live way (cut AL.2: counter-clockwise circling is
    /// a counter-clockwise turn), and the pilot's sense is set to it.
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
        live: Option<Turn>,
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
        let s = match live {
            Some(turn) => {
                self.sense = match turn {
                    Turn::CounterClockwise => 1,
                    Turn::Clockwise => -1,
                };
                f64::from(self.sense)
            }
            None => s,
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
        settings::{Engagement, Settings},
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

    /// **The orbit these tests were written for**: cut AK's default made
    /// explicit (cut AL.2 made the hold the default).
    fn doctrine() -> Doctrine {
        Doctrine::pilot_default(&Settings {
            engagement: Engagement::Orbit,
            ..Settings::default()
        })
    }

    /// The pilot's default doctrine now: the hold (cut AL.2).
    fn hold_doctrine() -> Doctrine {
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
    fn close_hold_and_withdraw_headings() {
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
        // At 200 m it holds there (cut AL.2: it used to circle there): bow on,
        // stopped.
        let close = run(Stance::Close, 200);
        assert_eq!((close.heading, close.speed), (Some(90), Some(0)));
        assert_eq!(run(Stance::Hold, 700).heading, Some(90));
        // Inside the band past the hysteresis it opens the range in reverse,
        // bow on (cut AL.2: it used to turn away, 270).
        let near = run(Stance::Hold, 200);
        assert_eq!((near.heading, near.speed), (Some(90), Some(-27)));
        let inside = run(Stance::Hold, 460);
        assert_eq!((inside.heading, inside.speed), (Some(90), Some(0)));
        // A withdraw backs away bow on (cut AL.2: it used to drive away, 270).
        let away = run(Stance::Withdraw, 400);
        assert_eq!((away.heading, away.speed), (Some(90), Some(-27)));
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
        // Cut AL.2: in reverse, bow on (it used to turn away, 180).
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(0), Some(-27)),
            "withdrawing: backing away bow on"
        );
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
        // Cut AL.2: in reverse, bow on (it used to turn away, 180).
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
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
        // Cut AL.2: in reverse, bow on (it used to turn away, 180).
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
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
        // Hold at the widened band: at it, stopped facing the target.
        assert_eq!(fly(Stance::Hold), (Some(0), Some(0)));
        // Close is flown as a hold there (not straight in to 200 m; cut AL.2:
        // it used to circle there, 090 at 54).
        assert_eq!(fly(Stance::Close), (Some(0), Some(0)));
        // A doctrine withdraw backs away bow on, ahead of the widened band
        // (cut AL.2: it used to turn away, 180 at 54).
        assert_eq!(fly(Stance::Withdraw), (Some(0), Some(-27)));
        // A hunt flies at the widened band too: at it, it holds (cut AL.2: it
        // used to circle, 090 at 54).
        assert_eq!(fly(Stance::Hunt), (Some(0), Some(0)));
        // The orbit is still the orbit there: the tangent.
        assert_eq!(fly(Stance::Orbit), (Some(90), Some(54)));
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
    fn the_hunt_stance_drives_in_to_the_band_then_holds() {
        let band = 300;
        let fly = |range: u32| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, range));
            let orders = pilot.decide(&picture, &hunting(band), NOW);
            (orders.heading, orders.speed)
        };
        // Outside the band, its slack and the hysteresis: straight at it,
        // full ahead.
        assert_eq!(fly(900), (Some(0), Some(54)));
        let slack = HOLD_SLACK_M as u32;
        let leave = (HOLD_SLACK_M + HOLD_HYSTERESIS_M) as u32;
        assert_eq!(fly(band + leave + 10), (Some(0), Some(54)));
        // At the band, and inside the slack and the hysteresis: the hold, bow
        // on and stopped (cut AL.2: it used to circle there).
        assert_eq!(fly(band), (Some(0), Some(0)));
        assert_eq!(fly(band + slack), (Some(0), Some(0)));
        assert_eq!(fly(band + slack + 10), (Some(0), Some(0)));
        // Well inside the band (past the slack and the hysteresis): in
        // reverse, bow on.
        assert_eq!(fly(band - leave - 10), (Some(0), Some(-27)));
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

    // -- cut AL.2: nose-on ------------------------------------------------------

    /// One decision of `pilot` on `doctrine` at `at`, with BA at `bearing` and
    /// `range` from the origin, its orders not held back by an earlier one:
    /// `(heading, speed)`.
    fn fly_on(
        picture: &mut Picture,
        pilot: &mut Pilot,
        doctrine: &Doctrine,
        bearing: u32,
        range: u32,
        at: u64,
    ) -> (Option<u32>, Option<i32>) {
        let mut ba = contact("BA", Some(Side::Blue), bearing, range);
        ba.read_ms = at;
        hold(picture, ba);
        picture.own.pos_read_ms = at;
        pilot.forget_heading();
        pilot.forget_speed();
        let orders = pilot.decide(picture, doctrine, at);
        (orders.heading, orders.speed)
    }

    /// **The default engagement holds bow on at the band** (cut AL.2): for
    /// bearings round the clock, a fresh pilot on the default doctrine is
    /// stopped inside the slack and the hysteresis, closes full ahead beyond
    /// them until back at the slack, opens in reverse short of them until out
    /// at the slack, never reverses for a machine at the help's 420 m or one
    /// read at exactly 400 m, and orders its heading on the hostile throughout.
    #[test]
    fn the_default_engagement_holds_bow_on_at_the_band() {
        let doctrine = hold_doctrine();
        assert_eq!(doctrine.stance, Stance::Hold);
        let band = f64::from(doctrine.band_m);
        let at = |m: f64| m.round() as u32;
        let slack = HOLD_SLACK_M;
        let leave = HOLD_SLACK_M + HOLD_HYSTERESIS_M;
        // Due north the plot lies at exactly its range, so each threshold is
        // pinned on its own metre there; round the clock (where a bearing's
        // sine leaves a hair of float) a metre inside it.
        for bearing in (0..360).step_by(30) {
            let edge = if bearing == 0 { 0.0 } else { 1.0 };
            let check = |(heading, _): (Option<u32>, Option<i32>)| {
                let h = heading.expect("a heading");
                assert!(
                    angle_between(h, bearing) <= HOLD_BOW_DEG,
                    "{h} for {bearing}"
                );
            };
            // A fresh pilot inside the slack and the hysteresis: stopped.
            for range in [
                band - leave + edge,
                band - slack,
                band,
                band + slack,
                band + leave - edge,
            ] {
                let (mut picture, mut pilot) = flying();
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, at(range), NOW);
                check(flown);
                assert_eq!(flown.1, Some(0), "{range} at {bearing}");
                assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
            }
            // Beyond them: full ahead, kept until back at the slack.
            let (mut picture, mut pilot) = flying();
            let mut t = NOW;
            for (range, kph) in [
                (band + leave + 1.0, 54),
                (band + slack + 1.0, 54),
                (band + slack - edge, 0),
                (band + slack + 1.0, 0),
                (band + leave - edge, 0),
            ] {
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, at(range), t);
                check(flown);
                assert_eq!(flown.1, Some(kph), "closing: {range} at {bearing}");
                t += 1_000;
            }
            // Short of them: in reverse, kept until out at the slack.
            let (mut picture, mut pilot) = flying();
            let mut t = NOW;
            for (range, kph) in [
                (band - leave - 1.0, -27),
                (band - slack - 1.0, -27),
                (band - slack + edge, 0),
                (band - slack - 1.0, 0),
                (band - leave + edge, 0),
            ] {
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, at(range), t);
                check(flown);
                assert_eq!(flown.1, Some(kph), "opening: {range} at {bearing}");
                t += 1_000;
            }
            // The machines' 420 m, and a row read at exactly 400 m: never a
            // reverse, fresh or arriving from a close.
            for range in [420, 400] {
                let (mut picture, mut pilot) = flying();
                assert_eq!(
                    fly_on(&mut picture, &mut pilot, &doctrine, bearing, range, NOW).1,
                    Some(0)
                );
                let (mut picture, mut pilot) = flying();
                fly_on(&mut picture, &mut pilot, &doctrine, bearing, 700, NOW);
                assert_eq!(pilot.hold_motion, HoldMotion::Ahead);
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, range, NOW + 1);
                check(flown);
                assert_eq!(flown.1, Some(0), "{range} after a close");
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, range, NOW + 2);
                assert_eq!(flown.1, Some(0), "{range} held");
            }
        }
        // A tank whose astern ceiling is unknown reverses at the slow default.
        let (mut picture, mut pilot) = flying();
        picture.own.max_astern_kph = None;
        let flown = fly_on(&mut picture, &mut pilot, &doctrine, 0, 100, NOW);
        assert_eq!(flown, (Some(0), Some(-UNKNOWN_ASTERN_KPH)));
    }

    /// **The orbit is a stance, no longer the default** (cut AL.2): the default
    /// doctrine holds; with `[arena] engagement = "orbit"` it is the orbit,
    /// flown as cut AK built it.
    #[test]
    fn orbit_is_a_stance_and_no_longer_the_default() {
        assert_eq!(
            Doctrine::pilot_default(&Settings::default()).stance,
            Stance::Hold
        );
        let orbit = Doctrine::pilot_default(&Settings {
            engagement: Engagement::Orbit,
            ..Settings::default()
        });
        assert_eq!(orbit.stance, Stance::Orbit);
        assert_eq!(orbit, doctrine(), "the orbit the older tests fly");
        // At the band, BA due north: the hold stops facing it; the orbit takes
        // its tangent (the pilot's running sense, counter-clockwise) full ahead.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let held = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((held.heading, held.speed), (Some(0), Some(0)));
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let circled = pilot.decide(&picture, &orbit, NOW);
        assert_eq!(
            (circled.heading, circled.speed),
            (Some(orbit_heading(0.0, 450.0, orbit.band_m, 1.0)), Some(54))
        );
        assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
    }

    /// **`close` and `hunt` end in a hold** (cut AL.2): `close` drives in bow
    /// on and holds at [`CLOSE_M`] (reversing only well inside it); `hunt`
    /// drives at a fresh hostile and holds at the band.
    #[test]
    fn close_and_hunt_end_in_a_hold() {
        let close = Doctrine {
            stance: Stance::Close,
            ..hold_doctrine()
        };
        let (mut picture, mut pilot) = flying();
        let slack = HOLD_SLACK_M;
        let leave = HOLD_SLACK_M + HOLD_HYSTERESIS_M;
        let mut t = NOW;
        for (range, kph) in [
            (600.0, 54),
            (CLOSE_M + slack + 1.0, 54),
            (CLOSE_M + slack, 0),
            (CLOSE_M, 0),
            (CLOSE_M - leave, 0),
            (CLOSE_M - leave - 1.0, -27),
        ] {
            let flown = fly_on(&mut picture, &mut pilot, &close, 0, range as u32, t);
            assert_eq!(flown, (Some(0), Some(kph)), "close at {range}");
            t += 1_000;
        }
        let hunt = Doctrine {
            stance: Stance::Hunt,
            ..hold_doctrine()
        };
        let band = f64::from(hunt.band_m);
        let (mut picture, mut pilot) = flying();
        let mut t = NOW;
        for (range, kph) in [
            (900.0, 54),
            (band + slack + 1.0, 54),
            (band + slack, 0),
            (band, 0),
        ] {
            let flown = fly_on(&mut picture, &mut pilot, &hunt, 180, range as u32, t);
            assert_eq!(flown, (Some(180), Some(kph)), "hunt at {range}");
            t += 1_000;
        }
    }

    /// **The fall back and a kept withdraw reverse bow on** (cut AL.2): with a
    /// target, the heading on it and a negative speed; the fall back with no
    /// target to face keeps its heading at full ahead.
    #[test]
    fn the_fall_back_and_a_kept_withdraw_reverse_bow_on() {
        for bearing in (0..360).step_by(45) {
            // The pilot's own fall back, on half the hull.
            let (mut picture, mut pilot) = flying();
            half_hull(&mut picture);
            hold(&mut picture, contact("BA", Some(Side::Blue), bearing, 250));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert!(pilot.withdraw_until_ms.is_some());
            assert_eq!((orders.heading, orders.speed), (Some(bearing), Some(-27)));
            // A kept doctrine withdraw.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), bearing, 450));
            let withdraw = Doctrine {
                stance: Stance::Withdraw,
                hold_s: Some(20),
                ..hold_doctrine()
            };
            let orders = pilot.decide(&picture, &withdraw, NOW);
            assert_eq!((orders.heading, orders.speed), (Some(bearing), Some(-27)));
        }
        // Nothing to face: the fall back keeps the heading, full ahead.
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.withdraw_until_ms.is_some());
        assert_eq!((orders.heading, orders.speed), (None, Some(54)));
    }

    /// A contact at `bearing`/`range` from the origin whose own row reads
    /// `rel` relative to the hull.
    fn contact_rel(designator: &str, bearing: u32, range: u32, rel: u32) -> Contact {
        Contact {
            rel_deg: rel,
            ..contact(designator, Some(Side::Blue), bearing, range)
        }
    }

    fn hit(picture: &mut Picture, at: u64, line: &str) {
        picture.apply(at, &classify(line));
    }

    const LEFT_HIT: &str = "Hit on the hull's left facet: 200 of 250 armour left.";

    /// **A side or rear hit turns the bow to the shooter** (cut AL.2).
    #[test]
    fn a_side_or_rear_hit_turns_the_bow_to_the_shooter() {
        // A hostile in the struck arc by its own row's relative bearing: the
        // `STATUS` heading (090, seconds old) would put BB astern, but its row
        // reads it at 270, on the left. BA is the held target, due north.
        let (mut picture, mut pilot) = flying();
        picture.own.heading_deg = 90;
        hold(&mut picture, contact_rel("BA", 0, 450, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        picture.lock = Some("BA".into());
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(270), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing BB."]
        );
        let (bearing, until) = pilot.threat.expect("a threat");
        assert_eq!((whole(bearing), until), (270, NOW + THREAT_TURN_MS));
        // A newer hit on the same threat does not re-arm it; one on a threat
        // more than `HOLD_BOW_DEG` away does (BC on the right, by its row).
        hit(&mut picture, NOW + 500, LEFT_HIT);
        pilot.decide(&picture, &hold_doctrine(), NOW + 500);
        assert_eq!(pilot.threat, Some((bearing, until)), "the same threat");
        assert!(pilot.take_notes().is_empty());
        hold(&mut picture, contact_rel("BC", 90, 600, 90));
        hit(
            &mut picture,
            NOW + 1_000,
            "Hit on the turret's right facet: 200 of 220 armour left.",
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 1_000);
        assert_eq!(
            pilot.threat.map(|(b, u)| (whole(b), u)),
            Some((90, NOW + 1_000 + THREAT_TURN_MS))
        );
        assert_eq!(orders.heading, Some(90));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the turret's right facet; facing BC."]
        );
        // The turn lasts `THREAT_TURN_MS`; then the hold faces BA again.
        let end = NOW + 1_000 + THREAT_TURN_MS;
        let mut later = picture.clone();
        for c in later.contacts.values_mut() {
            c.read_ms = end;
        }
        later.own.pos_read_ms = end;
        pilot.decide(&later, &hold_doctrine(), end - 1);
        assert!(pilot.threat.is_some());
        let orders = pilot.decide(&later, &hold_doctrine(), end);
        assert_eq!(pilot.threat, None);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));

        // None in the arc and none held: the arc's centre from the reference
        // heading — the `STATUS` heading with no order since…
        let (mut picture, mut pilot) = flying();
        picture.own.heading_deg = 40;
        hit(
            &mut picture,
            NOW,
            "Hit on the hull's rear facet: 100 of 150 armour left.",
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(220), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's rear facet; facing 220°."]
        );
        // …or the heading ordered after it.
        let (mut picture, mut pilot) = flying();
        picture.own.heading_deg = 40;
        pilot.sent_heading(100, NOW + 1);
        hit(&mut picture, NOW + 2, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 2);
        assert_eq!(orders.heading, Some(10));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing 010°."]
        );

        // A young target held and no other hostile in the arc: the target is
        // the shooter, and the hold already faces it — whether it is in the
        // arc itself or not.
        for (bearing, rel) in [(0, 0), (270, 270)] {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact_rel("BA", bearing, 450, rel));
            hit(&mut picture, NOW, LEFT_HIT);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None);
            assert_eq!((orders.heading, orders.speed), (Some(bearing), Some(0)));
            assert!(pilot.take_notes().is_empty());
        }
        // A wreck, an unidentified contact and a notice-only contact in the
        // arc are no shooter.
        let (mut picture, mut pilot) = flying();
        let mut x1 = contact_rel("X1", 270, 300, 270);
        x1.side = None;
        hold(&mut picture, x1);
        // (BD, known only from its notice, has no row: no range, and its plot
        // too old to be held, so it is neither the shooter nor a young target.)
        let mut bd = contact_rel("BD", 260, 300, 260);
        bd.range_m = None;
        bd.age_ms = HUNT_STALE_PLOT_MS + 1;
        hold(&mut picture, bd);
        hold(&mut picture, contact_rel("BE", 280, 300, 280));
        pilot.wrecks.insert("BE".into());
        hit(&mut picture, NOW, LEFT_HIT);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing 270°."]
        );

        // A front or top hit does nothing.
        for line in [
            "Hit on the hull's front facet: 300 of 400 armour left.",
            "Hit on the turret's top facet: 80 of 100 armour left.",
        ] {
            let (mut picture, mut pilot) = flying();
            hit(&mut picture, NOW, line);
            pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None, "{line}");
            assert_eq!(pilot.threat_seen_ms, NOW, "read once");
        }
        // An orbit doctrine does nothing.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        pilot.decide(&picture, &doctrine(), NOW);
        assert_eq!(pilot.threat, None);
        assert!(pilot.take_notes().is_empty());
    }

    /// The compass direction of the turn from `from` to `to`: `+1` clockwise,
    /// `-1` counter-clockwise, `0` none.
    fn turn_way(from: u32, to: u32) -> i64 {
        signed_turn(from, to).signum()
    }

    /// `flying()` with `track` destroyed.
    fn one_track(track: &str) -> (Picture, Pilot) {
        let (mut picture, pilot) = flying();
        lose_track(&mut picture, track);
        (picture, pilot)
    }

    fn lose_track(picture: &mut Picture, track: &str) {
        picture.own.sections.insert(
            track.into(),
            Section {
                condition: "destroyed".into(),
                internals: Some((0, 24)),
                facets: Default::default(),
            },
        );
    }

    /// **A one-track tank turns only the live way** (cut AL.2), asserted on
    /// the compass direction of each ordered heading from the reference
    /// heading, never on the orbit's sense.
    #[test]
    fn a_one_track_tank_turns_only_the_live_way() {
        assert_eq!(live_turn(&flying().0), None);
        let (mut both, _) = one_track("left_track");
        lose_track(&mut both, "right_track");
        assert_eq!(live_turn(&both), None, "both gone: no live turn");
        for (track, live) in [
            ("left_track", Turn::Clockwise),
            ("right_track", Turn::CounterClockwise),
        ] {
            let way = live.compass_sign() as i64;
            let words = if way > 0 {
                "clockwise"
            } else {
                "counter-clockwise"
            };
            let (picture, _) = one_track(track);
            assert_eq!(live_turn(&picture), Some(live));

            // The held turn, the pilot's sense set against the live way.
            let (mut picture, mut pilot) = one_track(track);
            pilot.sense = -(way as i8);
            picture.own.heading_deg = 100;
            held_event(&mut picture, NOW - 10);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            let h = orders.heading.expect("the held turn");
            assert_eq!(turn_way(100, h), way, "{track}: the held turn {h}");
            assert_eq!(angle_between(100, h), HELD_TURN_DEG as u32);
            // The sentence, once a match.
            let notes = pilot.take_notes();
            assert_eq!(
                notes.first().map(String::as_str),
                Some(format!("The {track} is gone: turning {words} only.").as_str())
            );
            pilot.decide(&picture, &hold_doctrine(), NOW + 10);
            assert!(
                !pilot.take_notes().iter().any(|n| n.contains(" is gone: ")),
                "once"
            );

            // A stall's level: 90° from the stop, the live way.
            let t0 = NOW + HELD_FOR_MS;
            held_read(&mut picture, t0, 145);
            pilot.decide(&picture, &hold_doctrine(), t0);
            assert!(pilot.stall_since_ms.is_some(), "{track}: a stall opens");
            let t1 = t0 + STALL_MS;
            held_read(&mut picture, t1, 145);
            let orders = pilot.decide(&picture, &hold_doctrine(), t1);
            assert_eq!(pilot.stall_level, 1);
            let h = orders.heading.expect("the stall's turn");
            assert_eq!(turn_way(100, h), way, "{track}: the stall {h}");
            assert_eq!(angle_between(100, h), 2 * HELD_TURN_DEG as u32);

            // The sweep's next leg, a quarter turn from the course the live
            // way, the pilot's sense set against it.
            let track_of = Track {
                course_deg: 90.0,
                speed_mps: 8.0,
            };
            let (mut picture, mut pilot, _) = lost(MID, 0, 450, Some(track_of));
            lose_track(&mut picture, track);
            pilot.sense = -(way as i8);
            to_the_sweep(&mut picture, &mut pilot);
            let HuntLeg::Sweep { heading_deg, .. } = hunt_of(&pilot).leg else {
                panic!("sweeping");
            };
            assert_eq!(turn_way(90, whole(heading_deg)), way, "{track}: the sweep");
            assert_eq!(angle_between(90, whole(heading_deg)), 90);

            // A detour round held ground: the search on 090 meets a mark
            // ahead (half way between the clear distance and the reach); it
            // turns aside the live way.
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 90;
            picture.spots.insert(
                'B',
                crate::arena::picture::Spot {
                    name: "The Crossroads".into(),
                    range_m: None,
                    bearing_deg: Some(90),
                    read_ms: NOW,
                    plotted_m: None,
                },
            );
            let ahead = mark_ahead_m();
            pilot.held_marks.push(HeldMark {
                at_m: (ahead - HELD_MARK_AHEAD_M, 0.0),
                heading_deg: 90,
                astern: false,
                blocked_m: (ahead, 0.0),
            });
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            let h = orders.heading.expect("the detour");
            assert_eq!(turn_way(90, h), way, "{track}: the detour {h}");
            assert_eq!(f64::from(angle_between(90, h)), HELD_MARK_DETOUR_DEG);

            // An orbit with `Sense::Auto` and no track, across a timed flip:
            // it circles the way that turns the live way. The tank drives
            // 100 m along the first heading the orbit orders; seen from there
            // the orbit's next heading has turned the live way.
            // (The tank already lies on the orbit's tangent: from a bow on BA
            // the tangent is a quarter turn, the dead way for one of the two
            // tracks, which a deadband tuned to 90° would not order.)
            let (mut picture, mut pilot) = one_track(track);
            let ba_at = (0.0, 450.0);
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            let circling = match live {
                Turn::CounterClockwise => 1.0,
                Turn::Clockwise => -1.0,
            };
            picture.own.heading_deg = orbit_heading(0.0, 450.0, doctrine().band_m, circling);
            let first = heading(&pilot.decide(&picture, &doctrine(), NOW));
            assert_eq!(first, picture.own.heading_deg, "{track}: on the tangent");
            pilot.sent_heading(first, NOW);
            let t = NOW + FLIP_EVERY_MS;
            let own = along((0.0, 0.0), f64::from(first), 100.0);
            let mut ba = contact_from(
                own,
                "BA",
                Some(Side::Blue),
                whole(bearing_deg(own, ba_at)),
                distance_m(own, ba_at).round() as u32,
            );
            ba.read_ms = t;
            hold(&mut picture, ba);
            read_at(&mut picture, own, t);
            picture.own.heading_deg = first;
            let second = heading(&pilot.decide(&picture, &doctrine(), t));
            assert_eq!(
                turn_way(first, second),
                way,
                "{track}: {first} then {second}"
            );

            // The deadband: a small dead-side correction, and one of exactly
            // `ONE_TRACK_DEADBAND_DEG`, are not ordered; a live-side one is.
            let dead_by =
                |deg: u32| -> u32 { ((100 - way * i64::from(deg)).rem_euclid(360)) as u32 };
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            for off in [10, ONE_TRACK_DEADBAND_DEG] {
                let flown = fly_on(
                    &mut picture,
                    &mut pilot,
                    &hold_doctrine(),
                    dead_by(off),
                    450,
                    NOW,
                );
                assert_eq!(flown, (None, Some(0)), "{track}: {off}° dead side");
            }
            let live_side = ((100 + way * 40).rem_euclid(360)) as u32;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                live_side,
                450,
                NOW,
            );
            assert_eq!(flown.0, Some(live_side), "{track}: live side at once");
            // A large one is ordered once, then nothing until `STATUS` reads
            // it…
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            let far = dead_by(ONE_TRACK_DEADBAND_DEG + 10);
            let flown = fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, NOW);
            assert_eq!(flown.0, Some(far));
            pilot.sent_heading(far, NOW);
            assert_eq!(pilot.turn_commit, Some((far, NOW)));
            let elsewhere = ((100 + way * 60).rem_euclid(360)) as u32;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                elsewhere,
                450,
                NOW + 500,
            );
            assert_eq!(flown.0, None, "{track}: committed");
            // (a `STATUS` short of it does not end the commit)
            picture.own.heading_deg = (far + 20) % 360;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                elsewhere,
                450,
                NOW + 1_000,
            );
            assert_eq!(flown.0, None, "{track}: not read yet");
            picture.own.heading_deg = ((i64::from(far) - 3 * way).rem_euclid(360)) as u32;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                far,
                450,
                NOW + 1_500,
            );
            assert_eq!(pilot.turn_commit, None, "{track}: read");
            assert_eq!(flown.0, Some(far));
            // …or the commit lapses.
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, NOW);
            pilot.sent_heading(far, NOW);
            let lapse = NOW + ONE_TRACK_COMMIT_MS;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                far,
                450,
                lapse - 1,
            );
            assert_eq!(flown.0, None);
            let flown = fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, lapse);
            assert_eq!(flown.0, Some(far), "{track}: the commit lapsed");
            // The re-board clears it.
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, NOW);
            pilot.sent_heading(far, NOW);
            pilot.forget_turns(&picture);
            assert_eq!(pilot.turn_commit, None);
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                far,
                450,
                NOW + 500,
            );
            assert_eq!(flown.0, Some(far), "{track}: re-boarded");
        }
    }

    /// How far ahead a test puts a mark that must block: half way between the
    /// clear distance and the reach, so it blocks wherever a tuning round
    /// moves either within its bounds.
    fn mark_ahead_m() -> f64 {
        (HELD_MARK_CLEAR_M + HELD_MARK_NEAR_M) / 2.0
    }

    /// The tank read at `pos` on `heading` at `at`, free and stopped.
    fn status_at(picture: &mut Picture, pos: (f64, f64), heading: u32, at: u64) {
        picture.own.pos_m = Some(pos);
        picture.own.heading_deg = heading;
        picture.own.pos_read_ms = at;
        picture.own.held = None;
        picture.own.speed_kph = 0;
    }

    /// **A held stop marks its ground and the search steers round it** (cut
    /// AL.2): the bot's own run-2 loop, and the mark's geometry.
    #[test]
    fn a_held_stop_marks_its_ground_and_the_search_steers_round_it() {
        // Stopped at (1 420, 1 049) on 000 by the slope.
        let stop = (1_420.0, 1_049.0);
        let (mut picture, mut pilot) = flying_at(stop);
        picture.events.push_back(Event {
            at_ms: NOW - 10,
            kind: EventKind::Held,
            text: "Held: the slope ahead is too steep for the tank.".into(),
        });
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(
            pilot.held_marks,
            [HeldMark {
                at_m: stop,
                heading_deg: 0,
                astern: false,
                blocked_m: (1_420.0, 1_049.0 + HELD_MARK_AHEAD_M),
            }]
        );
        assert!(pilot.take_notes().contains(
            &"Held ground marked at 1420 m east, 1049 m north, heading 000°.".to_owned()
        ));
        // Later, searching from (1 330, 1 052) for B on 091: blocked, and the
        // search's heading is turned aside.
        let from = (1_330.0, 1_052.0);
        assert!(pilot.blocked(from, 91.0));
        picture.spots.insert(
            'B',
            crate::arena::picture::Spot {
                name: "The Crossroads".into(),
                range_m: None,
                bearing_deg: Some(91),
                read_ms: NOW,
                plotted_m: None,
            },
        );
        let t = NOW + 20_000;
        status_at(&mut picture, from, 91, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        let aside = whole(91.0 + HELD_MARK_DETOUR_DEG * pilot.turn_sign(&picture));
        assert!(orders.ask_spot_b, "searching");
        assert_eq!(orders.heading, Some(aside));
        assert!(!pilot.blocked(from, f64::from(aside)));
        assert_eq!(
            pilot.take_notes(),
            [format!("Steering round held ground: {aside:03}° for 091°.")]
        );
        assert!(pilot.detour);
        // Said once while the detour lasts.
        pilot.decide(&picture, &hold_doctrine(), t + 50);
        assert!(pilot.take_notes().is_empty());
        // A heading clear of the mark is untouched.
        assert!(!pilot.blocked(from, 270.0));
        assert!(!pilot.blocked(from, 0.0));
        assert_eq!(pilot.steer_round(from, 270.0, 1.0), 270.0);
        assert!(!pilot.detour);
        // A mark beyond `HELD_MARK_NEAR_M` is ignored.
        let far = (stop.0 - HELD_MARK_NEAR_M - 20.0, 1_069.0);
        assert!(!pilot.blocked(far, 90.0));
        // From the stop point itself every heading is free (the clear
        // distance), and from anywhere within it of the mark.
        for h in (0..360).step_by(5) {
            assert!(!pilot.blocked(stop, f64::from(h)), "{h}");
            assert!(!pilot.blocked((1_420.0, 1_040.0), f64::from(h)), "{h}");
        }
        // Three blocked detours leave the heading as it was.
        let mut boxed = pilot.clone();
        for k in 1..=3_u8 {
            let detour = 91.0 + f64::from(k) * HELD_MARK_DETOUR_DEG;
            boxed.held_marks.push(HeldMark {
                at_m: from,
                heading_deg: 0,
                astern: false,
                blocked_m: along(from, detour, mark_ahead_m()),
            });
        }
        assert_eq!(boxed.steer_round(from, 91.0, 1.0), 91.0);
        // A stopped or reversing tank's heading is untouched: a hold at the
        // band on 091, and a hold reversing on 091 (its ray behind it clear),
        // with only the run-2 mark — which a forward heading on 091 would be
        // steered round (fixes after review: the boxed marks left 091 either
        // way).
        assert_ne!(
            pilot.clone().steer_round(from, 91.0, 1.0),
            91.0,
            "steered, were it ahead"
        );
        let (mut picture, mut pilot) = flying_at(from);
        pilot.held_marks = pilot_marks(stop);
        let ba = contact_from(from, "BA", Some(Side::Blue), 91, 450);
        hold(&mut picture, ba);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(91), Some(0)));
        let (mut picture, mut pilot) = flying_at(from);
        pilot.held_marks = pilot_marks(stop);
        hold(
            &mut picture,
            contact_from(from, "BA", Some(Side::Blue), 91, 200),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(91), Some(-27)));
        // The same ahead: steered round.
        let (mut picture, mut pilot) = flying_at(from);
        pilot.held_marks = pilot_marks(stop);
        hold(
            &mut picture,
            contact_from(from, "BA", Some(Side::Blue), 91, 900),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(orders.speed, Some(54));
        assert_ne!(orders.heading, Some(91), "closing: steered round");
        // `Held: something is in the way.` (a unit) and the unstated hold mark
        // nothing.
        for text in ["Held: something is in the way.", "Held: the tank is held."] {
            let (mut picture, mut pilot) = flying_at(stop);
            picture.events.push_back(Event {
                at_ms: NOW - 10,
                kind: EventKind::Held,
                text: text.into(),
            });
            pilot.decide(&picture, &hold_doctrine(), NOW);
            assert!(pilot.held_marks.is_empty(), "{text}");
            assert!(pilot.held_heading.is_some(), "{text}: still turned");
        }
        // The edge is ground; at most `HELD_MARKS_MAX` are kept, the oldest
        // dropped.
        let (mut picture, mut pilot) = flying_at(stop);
        for i in 0..=HELD_MARKS_MAX as u64 {
            picture.events.push_back(Event {
                at_ms: NOW + i,
                kind: EventKind::Held,
                text: "Held: this is the edge of the arena.".into(),
            });
            picture.own.pos_m = Some((stop.0 + i as f64, stop.1));
            pilot.decide(&picture, &hold_doctrine(), NOW + i + 1);
        }
        assert_eq!(pilot.held_marks.len(), HELD_MARKS_MAX);
        assert_eq!(pilot.held_marks[0].at_m, (stop.0 + 1.0, stop.1));
    }

    /// The run-2 stop's one mark.
    fn pilot_marks(stop: (f64, f64)) -> Vec<HeldMark> {
        vec![HeldMark {
            at_m: stop,
            heading_deg: 0,
            astern: false,
            blocked_m: along(stop, 0.0, HELD_MARK_AHEAD_M),
        }]
    }

    /// **A stop while reversing marks behind and stops the reverse** (cut
    /// AL.2; a guard — no transcript holds one): a fall back backing into a
    /// steep bank runs no full-ahead held turn, marks the ground behind, turns
    /// the next reverse into a stop with the bow kept, and opens no stall.
    #[test]
    fn a_stop_while_reversing_marks_behind_and_stops_the_reverse() {
        let (mut picture, mut pilot) = flying_at(MID);
        half_hull(&mut picture);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 250),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
        pilot.sent_heading(0, NOW);
        pilot.sent_speed(-27);
        pilot.take_notes();
        // Held while reversing.
        picture.events.push_back(Event {
            at_ms: NOW + 500,
            kind: EventKind::Held,
            text: "Held: the slope ahead is too steep for the tank.".into(),
        });
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 510);
        assert_eq!(
            (orders.heading, orders.speed),
            (None, Some(0)),
            "no full-ahead held turn: stopped, the bow kept"
        );
        assert!(pilot.held_astern);
        assert_eq!(pilot.held_heading, None);
        assert_eq!(
            pilot.held_marks,
            [HeldMark {
                at_m: MID,
                heading_deg: 0,
                astern: true,
                blocked_m: along(MID, 180.0, HELD_MARK_AHEAD_M),
            }]
        );
        assert_eq!(
            pilot.take_notes(),
            ["Held ground marked at 1280 m east, 1000 m north, heading 000°."]
        );
        assert!(pilot.blocked_behind(MID, 0.0), "the ray behind meets it");
        assert!(
            !pilot.blocked(MID, 180.0),
            "a forward ray keeps its clear distance"
        );
        // After the held stop the fall back still runs: its reverse is a stop
        // with the bow kept.
        let t = NOW + 510 + HELD_FOR_MS;
        held_read(&mut picture, t, 0);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert!(pilot.withdraw_until_ms.is_some());
        assert_eq!(orders.speed, Some(0));
        assert!(
            orders
                .heading
                .is_none_or(|h| angle_between(h, 0) <= HOLD_BOW_DEG),
            "the bow kept: {orders:?}"
        );
        // Read held at 0 kph a whole stall later: no stall, the pilot chose
        // the stop.
        let t = t + STALL_MS;
        held_read(&mut picture, t, 0);
        pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(pilot.stall_since_ms, None);
        assert_eq!(pilot.stall_level, 0);
        // A hold that would open the range here stops instead, bow on.
        let (mut picture, mut pilot) = flying_at(MID);
        pilot.held_marks = vec![HeldMark {
            at_m: MID,
            heading_deg: 0,
            astern: true,
            blocked_m: along(MID, 180.0, HELD_MARK_AHEAD_M),
        }];
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 200),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));
        assert_eq!(
            pilot.hold_motion,
            HoldMotion::Astern,
            "still wanting to open"
        );
        // Facing the other way, the mark is ahead, and the reverse goes.
        let (mut picture, mut pilot) = flying_at(MID);
        pilot.held_marks = vec![HeldMark {
            at_m: MID,
            heading_deg: 0,
            astern: true,
            blocked_m: along(MID, 180.0, HELD_MARK_AHEAD_M),
        }];
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 180, 200),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(180), Some(-27)));
    }

    /// **The back-off stops short of held ground behind** (fixes after
    /// review): step 1's back-off is checked against the ray behind the tank
    /// like every other reverse — a mark taken reversing is met however near,
    /// one taken driving ahead only past the clear distance (so the back-off
    /// from a forward stop is not stopped by that stop's own mark), and a mark
    /// off the ray stops nothing.
    #[test]
    fn the_back_off_stops_short_of_held_ground_behind() {
        let mark = |bearing: f64, metres: f64, astern: bool| HeldMark {
            at_m: MID,
            heading_deg: 0,
            astern,
            blocked_m: along(MID, bearing, metres),
        };
        for (marks, kph, why) in [
            (vec![], -27, "nothing marked"),
            (
                vec![mark(180.0, HELD_MARK_AHEAD_M, true)],
                0,
                "a mark taken reversing, just behind",
            ),
            (
                vec![mark(180.0, mark_ahead_m(), false)],
                0,
                "a mark taken ahead, past the clear distance behind",
            ),
            (
                vec![mark(180.0, HELD_MARK_AHEAD_M, false)],
                -27,
                "a mark taken ahead, inside the clear distance",
            ),
            (
                vec![mark(90.0, mark_ahead_m(), true)],
                -27,
                "a mark off the ray behind",
            ),
        ] {
            let (picture, mut pilot) = flying_at(MID);
            pilot.held_marks = marks;
            pilot.stall_level = 4;
            pilot.reverse_until_ms = NOW + HELD_REVERSE_MS;
            let orders = pilot.decide(&picture, &doctrine(), NOW);
            assert_eq!((orders.heading, orders.speed), (None, Some(kph)), "{why}");
        }
    }

    /// **A hold goes back to `Stopped` once anything else is flown** (fixes
    /// after review): a hold opening the range in reverse, then a decision
    /// under a withdraw, then the hold again inside its hysteresis is a stop,
    /// not the old reverse.
    #[test]
    fn a_hold_stops_again_after_flying_anything_else() {
        let band = f64::from(hold_doctrine().band_m);
        let (mut picture, mut pilot) = flying();
        let opening = (band - HOLD_SLACK_M - HOLD_HYSTERESIS_M - 10.0) as u32;
        let flown = fly_on(&mut picture, &mut pilot, &hold_doctrine(), 0, opening, NOW);
        assert_eq!(flown, (Some(0), Some(-27)));
        assert_eq!(pilot.hold_motion, HoldMotion::Astern);
        let withdraw = Doctrine {
            stance: Stance::Withdraw,
            hold_s: Some(20),
            ..hold_doctrine()
        };
        fly_on(&mut picture, &mut pilot, &withdraw, 0, opening, NOW + 1_000);
        assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
        // Inside the hysteresis, short of the slack: a fresh hold stands.
        let inside = (band - HOLD_SLACK_M - 10.0) as u32;
        let flown = fly_on(
            &mut picture,
            &mut pilot,
            &hold_doctrine(),
            0,
            inside,
            NOW + 2_000,
        );
        assert_eq!(flown, (Some(0), Some(0)));
        assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
    }

    /// **The turn into a hit in a crossfire** (fixes after review): the
    /// nearest hostile in the struck arc is the shooter; a threat within
    /// [`HOLD_BOW_DEG`] of the held target is not turned to (the hold faces
    /// it) and its sentence is not said; and a current threat whose shooter
    /// is gone, with only the held target left to blame, is dropped.
    #[test]
    fn a_crossfire_turns_to_the_nearest_shooter_and_never_off_the_target() {
        // Two in the left arc: the nearer, BB, is faced (BA, nearer still,
        // is the target, due north).
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 0, 200, 0));
        hold(&mut picture, contact_rel("BB", 250, 300, 250));
        hold(&mut picture, contact_rel("BC", 290, 600, 290));
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(250), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing BB."]
        );
        // The shooter (BB, left arc) within `HOLD_BOW_DEG` of the held target
        // (BA, astern-left): the hold already faces it, nothing is said.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 220, 450, 220));
        hold(&mut picture, contact_rel("BB", 240, 500, 240));
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(
            pilot.threat.map(|(b, _)| whole(b)),
            Some(240),
            "armed on BB"
        );
        assert_eq!((orders.heading, orders.speed), (Some(220), Some(0)));
        assert!(pilot.take_notes().is_empty());
        // A threat armed on BB; BB gone, a second left hit with only BA (well
        // off it) to blame: the threat is dropped, and the hold faces BA.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 0, 450, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.threat.is_some());
        picture.contacts.remove("BB");
        hit(&mut picture, NOW + 500, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 500);
        assert_eq!(pilot.threat, None);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));
    }

    /// **A turn into a hit that is not flown is not said** (fixes after
    /// review): a withdrawal outranks step 3b for the whole turn, and the
    /// sentence never goes; a hit read after its turn would be over arms
    /// nothing; and a hit taken while the machine flew the tank is read at the
    /// re-board ([`Pilot::forget_turns`]) and turns nothing.
    #[test]
    fn a_turn_into_a_hit_is_said_only_when_flown() {
        // Falling back on half the hull, BB in the left arc: the reverse bow
        // on BA goes on, and nothing turns or is said.
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        hold(&mut picture, contact_rel("BA", 0, 250, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        for t in [NOW, NOW + THREAT_TURN_MS - 1, NOW + THREAT_TURN_MS] {
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            assert!(pilot.withdraw_until_ms.is_some(), "still falling back");
            assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)), "{t}");
            pilot.forget_heading();
            pilot.forget_speed();
        }
        // The fall back over, the turn's time long past: dropped, unsaid.
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + WITHDRAW_MS);
        assert_eq!(pilot.withdraw_until_ms, None);
        assert_eq!(pilot.threat, None, "lapsed unflown");
        assert_eq!(orders.heading, Some(0), "the hold on BA");
        let notes = pilot.take_notes();
        assert!(
            !notes.iter().any(|n| n.starts_with("Turning into the shot")),
            "{notes:?}"
        );
        // A kept doctrine withdraw outranks it the same way.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 0, 450, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        let withdraw = Doctrine {
            stance: Stance::Withdraw,
            hold_s: Some(20),
            ..hold_doctrine()
        };
        let orders = pilot.decide(&picture, &withdraw, NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
        assert!(pilot.take_notes().is_empty());
        // A hit read a whole turn late (after a stretch immobile, say), with
        // nothing to blame: it would have turned to 270. Not even armed (the
        // fall back running here would leave an armed one standing, spent,
        // in the `arena-picture` record).
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        hit(&mut picture, NOW - THREAT_TURN_MS, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.threat, None);
        assert_eq!(pilot.threat_seen_ms, NOW - THREAT_TURN_MS, "read once");
        assert_ne!(orders.heading, Some(270));
        assert_eq!(
            pilot.take_notes(),
            ["Falling back to 450 m: the hull is at half."]
        );
        // A hit taken while away, read at the re-board.
        let (mut picture, mut pilot) = flying();
        hit(&mut picture, NOW - 1_000, LEFT_HIT);
        pilot.forget_turns(&picture);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.threat, None);
        assert_ne!(orders.heading, Some(270));
        assert!(pilot.take_notes().is_empty());
    }

    /// **A second hit with nothing to blame, mid-turn, does not move the
    /// turn** (fixes after review): the arc's centre is measured from the
    /// reference heading, and while the turn's own heading is ordered and not
    /// yet read by `STATUS` that is where the hull is going, not where it is —
    /// so the turn stands until a `STATUS` reads the hull.
    #[test]
    fn an_unblamed_hit_mid_turn_does_not_send_the_bow_round_again() {
        let (mut picture, mut pilot) = flying();
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(orders.heading, Some(270));
        pilot.sent_heading(270, NOW);
        pilot.take_notes();
        let armed = pilot.threat;
        // Again on the left, 1.2 s on, no `STATUS` between: the same turn.
        hit(&mut picture, NOW + 1_200, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 1_200);
        assert_eq!(pilot.threat, armed);
        assert_eq!(orders.heading, None, "270 already ordered");
        assert!(pilot.take_notes().is_empty());
        // A `STATUS` reads the hull part way round, at 300: a left hit now
        // is placed from it.
        picture.own.heading_deg = 300;
        picture.own.pos_read_ms = NOW + 2_000;
        hit(&mut picture, NOW + 2_100, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 2_100);
        assert_eq!(
            pilot.threat.map(|(b, u)| (whole(b), u)),
            Some((210, NOW + 2_100 + THREAT_TURN_MS))
        );
        assert_eq!(orders.heading, Some(210));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing 210°."]
        );
    }

    /// **One track gone, a hit on the dead side turns nothing** (fixes after
    /// review): the server would turn the bow the long, live way round —
    /// longer than the turn lasts, showing a side and the stern to the held
    /// target on the way — so no turn is armed, ordered or committed, and the
    /// hold keeps its bow; a hit on the live side turns at once; a hit dead
    /// astern (as long either way) turns.
    #[test]
    fn a_one_track_tank_never_turns_the_dead_way_into_a_hit() {
        for (track, dead, live) in [
            ("right_track", ("right", 90), ("left", 270)),
            ("left_track", ("left", 270), ("right", 90)),
        ] {
            let line =
                |facet: &str| format!("Hit on the hull's {facet} facet: 200 of 250 armour left.");
            // A shooter in the dead-side arc, BA held due north.
            let (mut picture, mut pilot) = one_track(track);
            hold(&mut picture, contact_rel("BA", 0, 450, 0));
            hold(&mut picture, contact_rel("BB", dead.1, 500, dead.1));
            hit(&mut picture, NOW, &line(dead.0));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None, "{track}: {} hit", dead.0);
            assert_eq!(
                (orders.heading, orders.speed),
                (Some(0), Some(0)),
                "{track}"
            );
            if let Some(deg) = orders.heading {
                pilot.sent_heading(deg, NOW);
            }
            assert_eq!(pilot.turn_commit, None, "{track}: nothing committed");
            assert!(
                !pilot
                    .take_notes()
                    .iter()
                    .any(|n| n.starts_with("Turning into the shot")),
                "{track}"
            );
            // Nothing held, nothing in the arc: the dead side's centre, the
            // same.
            let (mut picture, mut pilot) = one_track(track);
            hit(&mut picture, NOW, &line(dead.0));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None, "{track}: unblamed {} hit", dead.0);
            assert_ne!(orders.heading, Some(dead.1));
            // The live side: turned at once.
            let (mut picture, mut pilot) = one_track(track);
            hold(&mut picture, contact_rel("BA", 0, 450, 0));
            hold(&mut picture, contact_rel("BC", live.1, 500, live.1));
            hit(&mut picture, NOW, &line(live.0));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(
                (orders.heading, orders.speed),
                (Some(live.1), Some(0)),
                "{track}: {} hit",
                live.0
            );
            // Dead astern, nothing to blame: as long either way, so turned.
            let (mut picture, mut pilot) = one_track(track);
            hit(
                &mut picture,
                NOW,
                "Hit on the hull's rear facet: 100 of 150 armour left.",
            );
            pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat.map(|(b, _)| whole(b)), Some(180), "{track}");
        }
    }

    /// **The bot-side observer rule for AL.2's notes**: fed through
    /// [`Picture::apply`], every designator a new note names is one a contact
    /// row or notice named; a mark holds positions, a heading and a direction.
    #[test]
    fn the_new_notes_name_only_designators_the_stream_named() {
        let (mut picture, mut pilot) = flying();
        let lines = [
            (NOW, "Callsign RA."),
            (NOW, "Red Tank 1: power online, 0 kph, heading 000°"),
            (NOW, "  at 1280 m east, 1000 m north; scope holds a hostile"),
            (NOW, "  right_track  destroyed  internals 0/24"),
            (NOW + 10, "New contact X1: bearing 270°, far."),
            (NOW + 20, "Contact X1 identified: BB, blue."),
            (
                NOW + 30,
                "BB  blue  bearing 270° (rel 270°)  far  500 m  0 s",
            ),
            (
                NOW + 31,
                "BA  blue  bearing 000° (rel 000°)  far  300 m  0 s",
            ),
            (NOW + 40, "No such contact."),
            (
                NOW + 50,
                "Hit on the hull's left facet: 200 of 250 armour left.",
            ),
            // (Read after the turn into the hit is flown: a held stop
            // outranks the turn, and its sentence is said only when flown.)
            (
                NOW + 110,
                "Held: the slope ahead is too steep for the tank.",
            ),
        ];
        let mut named = BTreeSet::new();
        let mut notes = Vec::new();
        for (at, text) in lines {
            if at == NOW + 110 {
                assert_eq!(live_turn(&picture), Some(Turn::CounterClockwise));
                pilot.decide(&picture, &hold_doctrine(), NOW + 100);
                notes.extend(pilot.take_notes());
            }
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
            picture.apply(at, &line);
        }
        pilot.decide(&picture, &hold_doctrine(), NOW + 120);
        notes.extend(pilot.take_notes());
        // The search later meets the mark: the detour's sentence too.
        pilot.held_marks.push(HeldMark {
            at_m: (1_280.0, 900.0),
            heading_deg: 90,
            astern: false,
            blocked_m: (1_280.0 + mark_ahead_m(), 1_000.0),
        });
        assert!(pilot.steer_round((1_280.0, 1_000.0), 90.0, -1.0) != 90.0);
        notes.extend(pilot.take_notes());
        for start in [
            "The right_track is gone",
            "Turning into the shot",
            "Held ground marked",
            "Steering round held ground",
        ] {
            assert!(
                notes.iter().any(|n| n.starts_with(start)),
                "{start}: {notes:?}"
            );
        }
        for note in &notes {
            for word in note
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| {
                    w.len() > 1
                        && w.chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                        && w.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                })
            {
                assert!(named.contains(word), "{note}: {word}");
            }
        }
        assert!(notes.iter().any(|n| n.ends_with("facing BB.")), "{notes:?}");
        // The threat names nothing but a bearing.
        let value = serde_json::to_value(&pilot).expect("json");
        assert!(value["threat"].is_array());
        for mark in value["held_marks"].as_array().expect("marks") {
            let keys: BTreeSet<&str> = mark
                .as_object()
                .expect("a mark")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from(["astern", "at_m", "blocked_m", "heading_deg"])
            );
        }
    }
}
