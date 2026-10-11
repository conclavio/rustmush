//! **Doctrine's shared vocabulary** (17b cut M4.6a; 17b §3's M4 row, finding
//! (iv) "the survivors"; doc 17 §4, §11, §13): the words and the arithmetic
//! the stock mind, the scripted doctrine's constants and the arena's readers
//! of a machine's state share — moved here out of the reflex controller's
//! module (`crate::arena::reflex`), which 17b cut M4.6 deleted, so they
//! outlive it. A refactor: every item kept its value, its words and its
//! codes, so nothing hashed, carried, journaled or printed moved.
//!
//! * [`MachineState`] — where a machine is in doc 16 §6's state machine: the
//!   reflex's `ReflexState` until this cut, renamed and nothing else; 17b cut
//!   M4.6c added [`MachineState::Unknown`] (code 6, word `unknown`) for a mind
//!   not sealed on Standard Doctrine, and [`MachineState::DOCTRINE`], the six
//!   states the doctrine itself writes;
//! * the doctrine's constants — [`STAND_TICKS`] (the reflex's
//!   `REFLEX_STAND_TICKS`; the scripted doctrine still reads it as
//!   `stand_ticks`), the exposure weights, [`SCREEN_HALF_ARC_MDEG`] (`pub`:
//!   the twin lane's shadow reads it) and the support role's two distances;
//! * the shared arithmetic — [`Ranges`], [`stand_off_want`] and
//!   `arc_between`;
//! * **the stock mind's identity on a seal** — [`SEALED_MIND`],
//!   [`SEALED_CHARTER`], [`SEALED_LINE`] and [`SEALED_LICENCE`], which
//!   `crate::match_::seal_for` writes on every hull. They live here so the
//!   arena can recognise Standard Doctrine by identity without reaching into
//!   the match half (the arena's production code imports neither
//!   `crate::match_` nor `crate::minds`); `crate::match_::seal` and
//!   `crate::match_` re-export them under the paths they had.

use serde::{Deserialize, Serialize};

use crate::arena::motor::STAND_OFF_MARGIN_CELLS;

/// **How many decide cycles a cornered `Withdraw` stands and fights** (16b cut
/// Q.7) before it asks again whether it still has a line.
///
/// Three, because the live line a `Withdraw` is refusing to turn its back on
/// flickers: a treeline between two tanks breaks and remakes the trace several
/// times a second of arena time, and a unit that resumed its run on the first
/// broken tick would spend the next one turning round again.
///
/// The reflex's `REFLEX_STAND_TICKS` until 17b cut M4.6a, which renamed it
/// here; the scripted doctrine's constant `stand_ticks` reads it unchanged.
pub const STAND_TICKS: i64 = 3;

/// The exposure, per mille, of each facet band a candidate may present — the
/// `weight_facet` term of doc 16 §6's utility score.
///
/// Lower is a better target: a tank showing its rear is worth closing on and one
/// showing its front is not. The numbers are doctrine and nothing reads them but
/// the score — the reflex controller's (`crate::arena::reflex`), and since 17b
/// cut M1a.6 the stock mind's `best_hostile`, which is the same score over the
/// view (hence `pub(crate)`), and the scripted doctrine's constants
/// (`crate::minds::context`).
pub(crate) const EXPOSURE_FRONT: u32 = 1_000;
/// A flank.
pub(crate) const EXPOSURE_FLANK: u32 = 600;
/// The rear.
pub(crate) const EXPOSURE_REAR: u32 = 200;

/// **Half the arc a screen covers**, millidegrees either side of its facing (16b
/// cut Z.2; 16c N6): `Objective::Screen` locks and fires on a classified hostile
/// whose plot bears inside this of `facing_mdeg`, and on nothing else.
///
/// Sixty degrees, so a 120° sector: wide enough that a tank crossing the front
/// is met, narrow enough that a screen has a flank — a screen that saw all
/// round would be a `Hold` that shoots, and the order would say nothing about
/// where the squad is looking.
pub const SCREEN_HALF_ARC_MDEG: u32 = 60_000;

/// **How far a supporter may stray from its charge**, cells (16b cut Z.2; 16c
/// N6): past this, measured to the supporter's own plot of the charge, the leash
/// wins over the chase and the supporter drives back toward its station.
///
/// Fifteen, which is 150 m on the shipped ground: inside the `close` band of the
/// stock cannon, so a supporter fights the same target its charge fights rather
/// than one of its own a ridge away.
pub const SUPPORT_LEASH_CELLS: u16 = 15;

/// **The station a supporter keeps on its charge**, cells (16b cut Z.2; 16c N6):
/// near, not on top. The station cell — projected back from the plot of the
/// charge toward the supporter — is the goal until the unit stands inside this
/// of the plot, and then it holds.
pub const SUPPORT_STATION_CELLS: u16 = 5;

// ---------------------------------------------------------------------------
// The machine
// ---------------------------------------------------------------------------

/// **Where one AI unit is in doc 16 §6's machine.**
///
/// The reflex's `ReflexState` until 17b cut M4.6a, which renamed it and moved
/// it here with its words and codes unchanged; the stock mind's memory and
/// `MindState::doctrine_state` carry it, and the scripted doctrine writes the
/// same codes (the six of [`MachineState::DOCTRINE`]).
///
/// **17b cut M4.6** added [`MachineState::Unknown`], last, code 6 and word
/// `unknown`: what `Arena::machine_state` answers for a hull a mind flies
/// whose seal is not Standard Doctrine (`SealRef::doctrine`), which keeps no
/// doctrine state of its own in its memory. No doctrine memory decodes to it, so the
/// stock mind's own machine never enters it, and codes 0 to 5 did not move.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineState {
    /// Cold: the only order it gives is `STARTUP`.
    #[default]
    Startup,
    /// **Looking for something to fight.** Its goal is the first of these that
    /// answers (16b cut E.4):
    ///
    /// 1. **a movement order** — `Advance`'s cell, or `Hold`'s "where you
    ///    stand" (16b cut N.2);
    /// 2. **the search** — the last plot (the reflex row's `last_plot`, the
    ///    stock mind's memory since 17b cut M1a.6), while it is younger than
    ///    [`crate::config::ReflexConfig::search_ms`] and the machine has not
    ///    arrived at it: where it last saw the target it has just lost;
    /// 3. **the hunt** — the reflex row's `hunt` (the stock mind's memory
    ///    since 17b cut M1a.6), the other side's entry zone,
    ///    once [`crate::config::ReflexConfig::hunt_after_ms`] has passed with
    ///    nothing hostile held at all;
    /// 4. **the waypoint walk** — the motor's free `patrol_step` (the motor's
    ///    `Arena::patrol_step` since 17b cut M1a.1, lifted free by 17b cut
    ///    M1a.6; the stock mind's walk), the arena pack's graph, which is what
    ///    `Patrol` was and still is when the three above have nothing to say.
    ///
    /// **16b cut Z.2**: a machine under `Objective::Screen` stays here — parked,
    /// faced onto the order's bearing — and fires from its tick's *stand*
    /// without ever entering `Approach` or `Engage`, so a screening tick reads
    /// `patrol` in the state distribution. So does a supporter holding its
    /// station, or standing where it is because it cannot see its charge.
    ///
    /// **16b cut O.1 (16c N11; plan items 2, 16, 32, 33): in a Hold match the
    /// key spot is doctrine's fallback goal.** The list is then 1. the movement
    /// order, 2. the search, 3. **the Hold spot** (`Arena::hold_goal`) in place
    /// of BOTH the hunt and the waypoint walk, and 4. the waypoint walk only
    /// while `hold_goal` declines — a spot no route reaches, given up for
    /// `max(hunt_after_ms, repath_ms)`
    /// ([`MotorRow::hold_retry_ms`](crate::arena::motor::MotorRow::hold_retry_ms)). A
    /// machine arrives anywhere inside `radius − 1` of the spot and parks there
    /// (presence is all a hold needs, and one cell cannot seat a side); it is
    /// **not** a standing `Objective::Advance` written by the arena, because a
    /// live `Advance` suppresses the hostile edges for its whole TTL and arrival
    /// does not end it. A hostile sighted from the spot is fought **from inside
    /// the square** (the `Approach` arm's inside rule), and an `Advance` a
    /// commander sends into the square hands over to this goal on arrival
    /// (beside AA.1's arm). Both sides' machines take it. What is NOT changed,
    /// recorded for the owner in Q-O1: an attacking machine stops at
    /// `engage_cm` and never contests the square.
    Patrol,
    /// Closing on the hostile the utility score picked.
    Approach,
    /// Holding the band, locked, firing when the breech is free.
    Engage,
    /// Below the health threshold and making for its own entry zone.
    Withdraw,
    /// A wreck, or disabled: it issues nothing at all.
    Disabled,
    /// **Not a doctrine state** (17b cut M4.6; the conventions' D2): a hull a
    /// mind flies whose seal is not Standard Doctrine, whose memory says
    /// nothing in the doctrine's words. Read off the seal, never decoded from
    /// a memory, and unreachable in production before 17b cut M6 (every
    /// production seal is Standard Doctrine, the owner's OQ-2); since 17b cut
    /// M5.2 a fixture's mind sealed through the holdings seam reaches it. The
    /// catch-up sentence says it as the mind's own since 17b cut M5.3 (D14):
    /// `  The machine was flying on its own mind under no order.` (the three
    /// renderers opened the word, `… was unknown under …`, until then).
    Unknown,
}

impl MachineState {
    /// Every state, in code order — what a distribution is reported over
    /// (the leader's `doing:` histogram, the harness's `states_at_end`):
    /// seven since 17b cut M4.6, [`MachineState::Unknown`] last.
    pub const ALL: [MachineState; 7] = [
        MachineState::Startup,
        MachineState::Patrol,
        MachineState::Approach,
        MachineState::Engage,
        MachineState::Withdraw,
        MachineState::Disabled,
        MachineState::Unknown,
    ];

    /// **The six states the doctrine writes** (17b cut M4.6), in code order —
    /// [`MachineState::ALL`] without [`MachineState::Unknown`]: what a
    /// doctrine memory's `state` code decodes over
    /// (`crate::arena::stock_mind::state_of`; an unknown code reads
    /// `Startup`, as it always did) and the table the scripted doctrine's
    /// source header lists (`tests/content.rs`' content pin).
    pub const DOCTRINE: [MachineState; 6] = [
        MachineState::Startup,
        MachineState::Patrol,
        MachineState::Approach,
        MachineState::Engage,
        MachineState::Withdraw,
        MachineState::Disabled,
    ];

    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            MachineState::Startup => "startup",
            MachineState::Patrol => "patrol",
            MachineState::Approach => "approach",
            MachineState::Engage => "engage",
            MachineState::Withdraw => "withdraw",
            MachineState::Disabled => "disabled",
            MachineState::Unknown => "unknown",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            MachineState::Startup => 0,
            MachineState::Patrol => 1,
            MachineState::Approach => 2,
            MachineState::Engage => 3,
            MachineState::Withdraw => 4,
            MachineState::Disabled => 5,
            // 17b cut M4.6: last, so codes 0 to 5 never renumber.
            MachineState::Unknown => 6,
        }
    }
}

// ---------------------------------------------------------------------------
// The three distances a tick fights at
// ---------------------------------------------------------------------------

/// **What one machine's ranges are this tick** (16b cut E.1; 16c `B-05`,
/// `B-39`, `T-32`, `T-46`, `T-47`), all integer centimetres, computed once from
/// the unit, the doctrine and the order.
///
/// Before this cut every range test the controller made was against
/// [`UnitState::weapon_reach_cm`](crate::arena::state::UnitState::weapon_reach_cm)
/// — the *longest* reach the platform carries,
/// 3 km on a stock tank — on a map where the sight is 600 m and a mesh hop adds
/// 300 m. All of them were therefore always true: `Approach` became `Engage` at
/// the first line anywhere, `Engage` could never fall back, and a hurt machine
/// stood and fought at any range. The one place the doctrine band was *used*
/// was worse: 700 per mille of 3 km is 2.1 km, and `stand_off_cell` projects
/// that back from the plot, so the "stand-off" goal was up to 1.2 km **behind**
/// the machine.
///
/// So every distance here is priced off
/// [`UnitState::fight_inner_cm`](crate::arena::state::UnitState::fight_inner_cm)
/// and [`UnitState::fight_limit_cm`](crate::arena::state::UnitState::fight_limit_cm)
/// — the lesser of the platform's own sight and the band of the weapon it would
/// fire.
///
/// # The doctrine band is applied after the order's cap, and not before it
///
/// `engage_cm` is `engage_band_permille` of the reach the **order** allows, and
/// `release_cm` is that reach itself. Capping the two separately collapses them
/// to one distance under two of the commander's three words (`close` gives
/// 200 / 200 on a stock tank, `effective` 400 / 400), and a target drifting
/// about that one distance flips the machine between stopping to shoot and
/// driving to a stand-off cell every tick. Applied after the cap the hysteresis
/// is the same three tenths under every order, and the machine may fire across
/// the whole of its `Engage` region — so it never stands in `Engage` at a range
/// it may not shoot from.
///
/// The stock tank, engage / release / stand-off goal:
///
/// | order band | `engage_cm` | `release_cm` | stand-off `want` |
/// |---|---|---|---|
/// | none, or `far` | 420 m | 600 m | 390 m |
/// | `effective` | 280 m | 400 m | 250 m |
/// | `close` | 140 m | 200 m | 110 m |
///
/// `stand_cm` is 200 m under all three. **16b cut AA.1**: a machine that cannot
/// close ([`UnitState::cannot_close`](crate::arena::state::UnitState::cannot_close))
/// enters `Engage` at `release_cm` and not at `engage_cm`, so its region is the
/// whole row.
///
/// **Shared arithmetic, not a leaky reader** (17b cut M1a.6): `pub` because the
/// stock mind's [`crate::arena::stock_mind::view_ranges`] answers this struct
/// over the view, so T0 and T1 fight at the same three distances. It outlives
/// the reflex's decision half: 17b cut M4.6a moved `Ranges` and
/// [`stand_off_want`] here, out of the reflex's module, before 17b cut M4.6
/// deleted that. Only the reflex's `ranges` — the reading off the live
/// [`UnitState`](crate::arena::state::UnitState) — was T0's and left with it
/// in 17b cut M4.6.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ranges {
    /// Where `Approach` stops and turns: the doctrine band of the allowed reach —
    /// for a machine that can close (16b cut AA.1).
    pub engage_cm: u64,
    /// Where `Engage` gives up and closes again — **and the ceiling
    /// `Arena::engage_orders` will fire out to**, so the two cannot disagree.
    pub release_cm: u64,
    /// What a `Withdraw` still on its way home will turn and fight inside: the
    /// `close` band, capped by the order **and by `release_cm`**.
    ///
    /// The last cap is not redundant. The `close` band is a third of the *last*
    /// band edge held to the sight, and `release_cm` is the *first* one held to
    /// the same sight, so a gun whose inner band is under a third of its outer
    /// reach — nothing the game ships, and an ordinary shape for content to
    /// author — would otherwise let a hurt machine stop in front of a hostile
    /// that `Arena::engage_orders`' own ceiling then refuses to fire at. That
    /// is the parked target 16b cut Q.7 was paid to remove, arrived at by
    /// doctrine instead of by a pilot, and `Arena::withdraw_stand`'s doc says
    /// a unit that cannot shoot is never cornered.
    pub stand_cm: u64,
    /// The furthest this platform fights at at all; `0` is a platform with
    /// nothing loaded, which has no target.
    pub limit_cm: u64,
}

/// **How much range an `Approach` aims to keep on the plot**, centimetres (16b
/// cut E.1; 16c `B-05`) — the argument
/// [`Arena::stand_off_cell`](crate::arena::state::Arena::stand_off_cell)
/// projects back from the plot toward the machine.
///
/// Floor first, `min` with the range **last**, and the order is the whole of the
/// function: the literal `engage_cm − margin` underflows a `u64` for a short band
/// or a small sight, and a floor applied *after* the `min` can exceed `range`,
/// which projects the goal behind the machine — `B-05` written a second time. As
/// written the answer is never above `range`, and never nought unless `range` is.
///
/// **Shared arithmetic, not a leaky reader** (17b cut M1a.6): `pub` because the
/// stock mind's `Approach` and `Engage` call it over the view's held range, so
/// it outlives the reflex's decision half — moved here in 17b cut M4.6a (see
/// [`Ranges`]); the scripted doctrine's `stand_off_want` kernel
/// (`crate::minds::kernels`) wraps it.
#[must_use]
pub fn stand_off_want(engage_cm: u64, cell_cm: u64, range_cm: u64) -> u64 {
    engage_cm
        .saturating_sub(u64::from(STAND_OFF_MARGIN_CELLS).saturating_mul(cell_cm))
        .max(cell_cm)
        .min(range_cm)
}

// ---------------------------------------------------------------------------
// The arc a screen reads
// ---------------------------------------------------------------------------

/// **The angle between two bearings**, millidegrees, the short way round (16b
/// cut Z.2): `0` to `180,000`, modulo a full turn, so a screen facing 350° sees
/// a plot at 10° as 20° off its facing and not 340°.
///
/// `pub(crate)` since 17b cut M1a.6: the stock mind's screen reads the same
/// arc, in whole degrees (here since 17b cut M4.6a).
pub(crate) fn arc_between(a_mdeg: u32, b_mdeg: u32) -> u32 {
    let apart = (a_mdeg % 360_000).abs_diff(b_mdeg % 360_000);
    apart.min(360_000 - apart)
}

// ---------------------------------------------------------------------------
// The stock mind's identity on a seal
// ---------------------------------------------------------------------------

/// **The stock mind's name on a seal** (doc 17 §11). Here since 17b cut
/// M4.6a, with the three below; `crate::match_::seal` re-exports all four.
pub const SEALED_MIND: &str = "Standard Doctrine";

/// **The charter the stock mind is filed under** (doc 17 §16).
pub const SEALED_CHARTER: &str = "registry";

/// **The stock mind's line of revisions**.
pub const SEALED_LINE: &str = "standard";

/// **The licence a sim seal records** (D-UM-49: the OOC sim reads no IC
/// holding and holds no licence before 17b cut M6, the owner's OQ-1 and
/// OQ-2).
pub const SEALED_LICENCE: &str = "doctrine";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Controller;

    /// **[`stand_off_want`] as a table** (16b cut E.1; 16c `B-05`).
    ///
    /// Three claims over every row, and each of them is a way the first draft of
    /// this arithmetic went wrong: the answer is never above `range_cm` (a want
    /// past the range projects the goal *behind* the machine), it is never nought
    /// unless the range is (a nought want puts the goal on the plot's own cell),
    /// and nothing underflows a `u64` for a band shorter than the margin.
    ///
    /// 17b cut M4.6a: moved here from `crate::arena::reflex`'s tests with
    /// [`stand_off_want`], its assertions word for word.
    #[test]
    fn e1_the_stand_off_want_is_never_past_the_range_and_never_underflows() {
        let cell = 1_000_u64;
        for (engage_cm, range_cm, want) in [
            // Nothing loaded: the floor of one cell, held to the range.
            (0_u64, 0_u64, 0_u64),
            (0, 500, 500),
            (0, 42_000, 1_000),
            // A band of exactly one cell, and of exactly the margin: the
            // subtraction would underflow, and the floor answers instead.
            (1_000, 42_000, 1_000),
            (3_000, 42_000, 1_000),
            (3_001, 42_000, 1_000),
            // The stock tank's own row: 420 m less three cells is 390 m.
            (42_000, 42_000, 39_000),
            (42_000, 60_000, 39_000),
            // And a range inside the want: the want comes down to it.
            (42_000, 20_000, 20_000),
            (42_000, 900, 900),
            // An engagement band bigger than anything on the window.
            (u64::MAX, 42_000, 42_000),
        ] {
            let have = stand_off_want(engage_cm, cell, range_cm);
            assert_eq!(have, want, "engage {engage_cm}, range {range_cm}");
            assert!(
                have <= range_cm,
                "engage {engage_cm}, range {range_cm}: {have} is past the range, so the goal \
                 would be behind the machine"
            );
            assert!(
                have > 0 || range_cm == 0,
                "engage {engage_cm}, range {range_cm}: a nought want is the plot's own cell"
            );
        }
        // And a window with a nonsense cell size does not panic either.
        assert_eq!(stand_off_want(42_000, u64::MAX, 42_000), 42_000);
    }

    /// The states' words and codes (17b cut M4.6a: moved here with
    /// [`MachineState`]; its two lines on the reflex controller's own code and
    /// word stayed in `crate::arena::reflex`'s tests and died with it in 17b
    /// cut M4.6c).
    ///
    /// **17b cut M4.6c** (the conventions' D2): [`MachineState::Unknown`] is
    /// code 6, word `unknown` — the word the harness's `states_at_end` leaf
    /// already carried — and last in [`MachineState::ALL`], so codes 0 to 5
    /// did not move; [`MachineState::DOCTRINE`] is `ALL` without it, in order;
    /// and the mind's controller byte is still `2`.
    #[test]
    fn the_states_round_trip_their_words_and_codes() {
        for (at, state) in MachineState::ALL.iter().enumerate() {
            assert_eq!(usize::from(state.code()), at);
            assert!(!state.as_str().is_empty());
            assert_eq!(
                serde_json::to_value(state).ok(),
                Some(serde_json::json!(state.as_str())),
                "the serde word is the wire word"
            );
        }
        assert_eq!(MachineState::ALL.len(), 7);
        assert_eq!(MachineState::ALL.last(), Some(&MachineState::Unknown));
        assert_eq!(MachineState::Unknown.code(), 6);
        assert_eq!(MachineState::Unknown.as_str(), "unknown");
        assert_eq!(MachineState::DOCTRINE[..], MachineState::ALL[..6]);
        assert!(!MachineState::DOCTRINE.contains(&MachineState::Unknown));
        assert_eq!(Controller::Pilot.code(), 0);
        assert_eq!(Controller::Mind.code(), 2);
    }

    /// **16b cut Z.2's review**: a screen's arc is measured the short way round.
    /// Every screen the replays and the lance lane fly faces 090° or 270°, where
    /// a plain difference of bearings gives the same answers; a screen facing
    /// 000° or 350° is where the wrap decides whether a tank at 10° is met.
    ///
    /// 17b cut M4.6a: moved here from `crate::arena::reflex`'s tests with
    /// [`arc_between`], its assertions word for word.
    #[test]
    fn z2_the_arc_between_two_bearings_is_the_short_way_round() {
        for (a, b, apart) in [
            (350_000_u32, 10_000_u32, 20_000_u32),
            (10_000, 350_000, 20_000),
            (0, 359_000, 1_000),
            (359_999, 0, 1),
            (90_000, 270_000, 180_000),
            (90_000, 30_000, 60_000),
            (0, 0, 0),
            // A facing written a full turn over is the same facing.
            (360_000, 0, 0),
            (720_000 + 5_000, 355_000, 10_000),
        ] {
            assert_eq!(arc_between(a, b), apart, "{a} and {b}");
        }
        // The half arc admits a tank at 10° to a screen facing north, and a tank
        // at 61° off it is outside.
        assert!(arc_between(10_000, 0) <= SCREEN_HALF_ARC_MDEG);
        assert!(arc_between(299_000, 0) > SCREEN_HALF_ARC_MDEG);
    }
}
