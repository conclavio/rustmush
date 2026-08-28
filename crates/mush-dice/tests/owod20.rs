//! oWoD20 (Storyteller System) pool mechanic — rules-correctness golden tests.
//!
//! Most tests drive [`Owod20Pool::score`] with explicit faces so they assert
//! the rules directly, independent of the RNG. A few exercise the seeded
//! `PoolMechanic::roll` path for determinism.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use mush_dice::RollCtx;
use mush_dice::owod20::{
    Contest, ExtendedRoll, ExtendedStatus, Owod20Die, Owod20Opts, Owod20Outcome, Owod20Pool,
    RollResult, contest,
};
use mush_dice::pool::PoolMechanic;

fn score(faces: &[u8], opts: &Owod20Opts) -> Owod20Outcome {
    Owod20Pool::score(faces, opts)
}

fn diff(d: u8) -> Owod20Opts {
    Owod20Opts::new(d)
}

// ------------------------------------------------------------- botch truth table

#[test]
fn botch_when_no_success_and_a_one() {
    // diff 6: nothing >= 6, one 1 -> botch severity 1.
    let out = score(&[1, 2, 3], &diff(6));
    assert_eq!(out.result, RollResult::Botch(1));
    assert_eq!(out.net, -1);
    assert!(out.is_botch());
}

#[test]
fn botch_severity_counts_the_ones() {
    let out = score(&[1, 1, 3, 2], &diff(6));
    assert_eq!(out.result, RollResult::Botch(2));
    assert_eq!(out.ones, 2);
}

#[test]
fn any_success_makes_botch_impossible_only_a_failure() {
    // one success (7) plus a 1 -> net 0 -> plain FAILURE, never a botch.
    let out = score(&[7, 1], &diff(6));
    assert_eq!(out.result, RollResult::Failure);
    assert!(!out.is_botch());
    assert_eq!(out.net, 0);
}

#[test]
fn success_present_with_many_ones_is_failure_not_botch() {
    // 7 is a success; three 1s -> net 1-3 = -2 -> FAILURE (a success was rolled).
    let out = score(&[7, 1, 1, 1], &diff(6));
    assert_eq!(out.result, RollResult::Failure);
    assert!(!out.is_botch());
    assert_eq!(out.net, -2);
}

#[test]
fn willpower_prevents_botch() {
    let mut opts = diff(6);
    opts.willpower = true;
    // no die >= 6 and a 1 would botch, but Willpower forbids it.
    let out = score(&[1, 2, 3], &opts);
    assert_eq!(out.result, RollResult::Failure);
    assert!(!out.is_botch());
    // net = 0 successes - 1 one + 1 willpower = 0
    assert_eq!(out.net, 0);
    assert!(out.willpower_success);
}

#[test]
fn extra_successes_do_not_prevent_botch() {
    // Per the crib, botch depends on rolled successes + willpower only.
    let mut opts = diff(6);
    opts.extra_successes = 3;
    let out = score(&[1, 2, 3], &opts);
    assert_eq!(out.result, RollResult::Botch(1));
}

// ------------------------------------------------------------------- specialty

#[test]
fn specialty_doubles_tens() {
    let mut opts = diff(6);
    opts.specialty = true;
    // two 10s (each worth 2) + a 5 (miss) = 4 successes.
    let out = score(&[10, 10, 5], &opts);
    assert_eq!(out.raw_successes, 4);
    assert_eq!(out.result, RollResult::Success(4));
    assert!(out.dice[0].doubled);
    assert!(out.dice[1].doubled);
    assert!(!out.dice[2].doubled);
}

#[test]
fn without_specialty_a_ten_is_one_success() {
    let out = score(&[10, 10, 5], &diff(6));
    assert_eq!(out.raw_successes, 2);
    assert_eq!(out.result, RollResult::Success(2));
    assert!(!out.dice[0].doubled);
}

#[test]
fn again_tens_double_flag_matches_specialty() {
    let mut opts = diff(6);
    opts.again_tens_double = true;
    let out = score(&[10, 7], &opts);
    assert_eq!(out.raw_successes, 3); // 10 doubled + 7
    assert!(out.dice[0].doubled);
}

// ------------------------------------------------------------ difficulty range

#[test]
fn difficulty_two_boundary() {
    // diff 2: 2..10 succeed, only the 1 subtracts.
    let out = score(&[2, 3, 1], &diff(2));
    assert_eq!(out.raw_successes, 2);
    assert_eq!(out.net, 1);
    assert_eq!(out.result, RollResult::Success(1));
}

#[test]
fn difficulty_ten_boundary() {
    // diff 10: only a 10 succeeds.
    let hit = score(&[10, 9, 1], &diff(10));
    assert_eq!(hit.raw_successes, 1);
    assert_eq!(hit.net, 0); // 1 success - 1 one
    assert_eq!(hit.result, RollResult::Failure);

    let miss = score(&[9, 8, 1], &diff(10));
    assert_eq!(miss.raw_successes, 0);
    assert_eq!(miss.result, RollResult::Botch(1));
}

#[test]
fn difficulty_is_clamped_to_two_through_ten() {
    assert_eq!(diff(0).effective_difficulty(), 2);
    assert_eq!(diff(1).effective_difficulty(), 2);
    assert_eq!(diff(6).effective_difficulty(), 6);
    assert_eq!(diff(11).effective_difficulty(), 10);
    assert_eq!(diff(255).effective_difficulty(), 10);
    // score honors the clamp
    let out = score(&[2], &diff(1));
    assert_eq!(out.difficulty, 2);
    assert_eq!(out.result, RollResult::Success(1));
}

// --------------------------------------------------------------- willpower / net

#[test]
fn willpower_adds_one_automatic_success() {
    let mut opts = diff(6);
    opts.willpower = true;
    let out = score(&[7, 7], &opts);
    assert_eq!(out.raw_successes, 2);
    assert_eq!(out.net, 3); // + 1 willpower
    assert_eq!(out.result, RollResult::Success(3));
    assert!(out.willpower_success);
}

#[test]
fn net_math_combines_all_sources() {
    let mut opts = diff(6);
    opts.specialty = true;
    opts.willpower = true;
    opts.extra_successes = 1;
    // faces: 10 (doubled -> 2), 7 (1), 1 (one). raw = 3, ones = 1.
    // net = 3 - 1 + 1(wp) + 1(extra) = 4.
    let out = score(&[10, 7, 1], &opts);
    assert_eq!(out.raw_successes, 3);
    assert_eq!(out.ones, 1);
    assert_eq!(out.net, 4);
    assert_eq!(out.result, RollResult::Success(4));
}

// -------------------------------------------------------------------- provenance

#[test]
fn per_die_flags_are_recorded() {
    let out = score(&[10, 6, 1], &diff(6));
    assert_eq!(
        out.dice[0],
        Owod20Die {
            face: 10,
            is_success: true,
            is_one: false,
            doubled: false
        }
    );
    assert_eq!(
        out.dice[1],
        Owod20Die {
            face: 6,
            is_success: true,
            is_one: false,
            doubled: false
        }
    );
    assert_eq!(
        out.dice[2],
        Owod20Die {
            face: 1,
            is_success: false,
            is_one: true,
            doubled: false
        }
    );
}

// -------------------------------------------------------------------- extended

#[test]
fn extended_accumulates_toward_target() {
    let mut ext = ExtendedRoll::new(5);
    assert_eq!(
        ext.add(score(&[7, 8, 4], &diff(6))),
        ExtendedStatus::Continuing
    ); // +2 = 2
    assert_eq!(
        ext.add(score(&[6, 3, 2], &diff(6))),
        ExtendedStatus::Continuing
    ); // +1 = 3
    assert_eq!(
        ext.add(score(&[9, 9, 7], &diff(6))),
        ExtendedStatus::Complete
    ); // +3 = 6 >= 5
    assert_eq!(ext.accumulated, 6);
    assert!(ext.is_complete());
    assert_eq!(ext.rolls.len(), 3);
}

#[test]
fn extended_flags_botch_and_leaves_policy_to_caller() {
    let mut ext = ExtendedRoll::new(5);
    ext.add(score(&[7, 8, 4], &diff(6))); // +2
    let status = ext.add(score(&[1, 2, 3], &diff(6))); // botch
    assert_eq!(status, ExtendedStatus::Botched);
    assert!(ext.botched);
    assert!(!ext.is_complete());
    // accumulator does not auto-undo; the +2 is still recorded.
    assert_eq!(ext.accumulated, 2);
}

#[test]
fn extended_failure_adds_zero() {
    let mut ext = ExtendedRoll::new(3);
    ext.add(score(&[7, 8], &diff(6))); // +2
    ext.add(score(&[4, 3], &diff(6))); // failure, +0
    assert_eq!(ext.accumulated, 2);
    assert_eq!(ext.status(), ExtendedStatus::Continuing);
}

// ------------------------------------------------------------------- contested

#[test]
fn contested_compares_successes() {
    let strong = score(&[9, 9, 8], &diff(6)); // 3 successes
    let weak = score(&[7, 4, 3], &diff(6)); // 1 success
    assert_eq!(contest(&strong, &weak), Contest::Win);
    assert_eq!(contest(&weak, &strong), Contest::Lose);
}

#[test]
fn contested_ties_deadlock() {
    let a = score(&[7, 8], &diff(6)); // 2
    let b = score(&[9, 6], &diff(6)); // 2
    assert_eq!(contest(&a, &b), Contest::Deadlock);
}

#[test]
fn contested_failures_are_zero_not_negative() {
    // two failures with different numbers of 1s both count as 0 successes.
    let a = score(&[7, 1], &diff(6)); // net 0, failure
    let b = score(&[8, 5], &diff(6)); // net 1, success
    assert_eq!(contest(&a, &b), Contest::Lose);
    let c = score(&[4, 3], &diff(6)); // 0 successes
    let d = score(&[5, 2], &diff(6)); // 0 successes
    assert_eq!(contest(&c, &d), Contest::Deadlock);
}

// ------------------------------------------------------------------- formatter

#[test]
fn summary_reads_like_a_roll_log() {
    assert_eq!(
        score(&[7, 8, 4], &diff(6)).summary(),
        "rolled 7 8 4 -> 2 successes"
    );
    assert_eq!(
        score(&[8, 4, 3], &diff(6)).summary(),
        "rolled 8 4 3 -> 1 success"
    );
    assert_eq!(
        score(&[4, 3, 2], &diff(6)).summary(),
        "rolled 4 3 2 -> failure"
    );
    assert_eq!(
        score(&[3, 2, 1], &diff(6)).summary(),
        "rolled 3 2 1 -> botch"
    );
    assert_eq!(
        score(&[1, 1, 2], &diff(6)).summary(),
        "rolled 1 1 2 -> botch (2)"
    );
}

#[test]
fn summary_notes_willpower() {
    let mut opts = diff(6);
    opts.willpower = true;
    assert_eq!(
        score(&[7, 7], &opts).summary(),
        "rolled 7 7 +WP -> 3 successes"
    );
}

#[test]
fn display_matches_summary() {
    let out = score(&[7, 8, 4], &diff(6));
    assert_eq!(format!("{out}"), out.summary());
}

// ---------------------------------------------------------------- seeded roll

#[test]
fn pool_roll_is_seeded_and_reproducible() {
    let opts = diff(6);
    let mk = || {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut c = RollCtx::new(&mut rng);
        Owod20Pool.roll(5, &opts, &mut c)
    };
    let a = mk();
    let b = mk();
    assert_eq!(a, b);
    // seed 42 d10 stream = [8,6,5,9,3,...]; at diff 6: 8,6,9 succeed = 3.
    assert_eq!(
        a.dice.iter().map(|d| d.face).collect::<Vec<_>>(),
        vec![8, 6, 5, 9, 3]
    );
    assert_eq!(a.result, RollResult::Success(3));
}

#[test]
fn pool_of_zero_or_less_still_rolls_one_die() {
    let opts = diff(6);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let mut c = RollCtx::new(&mut rng);
    let out = Owod20Pool.roll(0, &opts, &mut c);
    assert_eq!(out.dice.len(), 1);

    let mut rng2 = ChaCha8Rng::seed_from_u64(42);
    let mut c2 = RollCtx::new(&mut rng2);
    let out2 = Owod20Pool.roll(-5, &opts, &mut c2);
    assert_eq!(out2.dice.len(), 1);
}

#[test]
fn pool_roll_respects_dice_cap() {
    let opts = diff(6);
    let limits = mush_dice::RollLimits {
        max_dice: 4,
        ..Default::default()
    };
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let mut c = RollCtx::with_limits(&mut rng, limits);
    let out = Owod20Pool.roll(50, &opts, &mut c);
    assert_eq!(out.dice.len(), 4);
}

// -------------------------------------------------------------- serialization

#[test]
fn outcome_serializes_for_the_event_bus() {
    let out = score(&[10, 7, 1], &{
        let mut o = diff(6);
        o.specialty = true;
        o
    });
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["difficulty"], 6);
    assert_eq!(json["net"], out.net);
    assert_eq!(json["result"], serde_json::json!({ "Success": out.net }));
    assert_eq!(json["dice"][0]["doubled"], true);
    // round-trips
    let back: Owod20Outcome = serde_json::from_value(json).unwrap();
    assert_eq!(back, out);
}
