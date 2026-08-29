//! Shadowrun 5th Edition (d6 hit pool) mechanic — rules-correctness golden tests.
//!
//! Most tests drive [`Sr5Pool::score`] with explicit faces so they assert the
//! rules directly, independent of the RNG. The Rule-of-Six explosion needs the
//! RNG, so those tests inject a [`SeqRng`] that yields a scripted face sequence;
//! a couple exercise the seeded `ChaCha` path for determinism.

use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use mush_dice::RollCtx;
use mush_dice::pool::PoolMechanic;
use mush_dice::sr5::{
    Sr5Contest, Sr5Die, Sr5Extended, Sr5ExtendedStatus, Sr5Opts, Sr5Outcome, Sr5Pool, Sr5Result,
    contest, opposed,
};

fn score(faces: &[u8], opts: &Sr5Opts) -> Sr5Outcome {
    Sr5Pool::score(faces, opts)
}

/// Default options: threshold 1, no limit, no edge.
fn opts() -> Sr5Opts {
    Sr5Opts::default()
}

/// A deterministic `RngCore` that replays a scripted d6 face sequence.
///
/// `roll_d6` computes `next_u32() % 6 + 1`, so yielding `face - 1` reproduces
/// exactly `face`. Panics if the roller draws more dice than were scripted,
/// which keeps the explosion tests honest about how many dice they roll.
struct SeqRng {
    vals: Vec<u32>,
    idx: usize,
}

impl SeqRng {
    fn faces(faces: &[u8]) -> Self {
        Self {
            vals: faces.iter().map(|&f| u32::from(f - 1)).collect(),
            idx: 0,
        }
    }
}

impl RngCore for SeqRng {
    fn next_u32(&mut self) -> u32 {
        let v = *self.vals.get(self.idx).expect("SeqRng exhausted");
        self.idx += 1;
        v
    }
    fn next_u64(&mut self) -> u64 {
        u64::from(self.next_u32())
    }
    fn fill_bytes(&mut self, dst: &mut [u8]) {
        for b in dst.iter_mut() {
            *b = self.next_u32() as u8;
        }
    }
}

// ------------------------------------------------------------------- hits (5,6)

#[test]
fn only_fives_and_sixes_are_hits() {
    let out = score(&[6, 5, 4, 3, 2, 1], &opts());
    assert_eq!(out.raw_hits, 2); // 6 and 5
    assert_eq!(out.hits, 2);
    assert!(out.dice[0].is_hit); // 6
    assert!(out.dice[1].is_hit); // 5
    assert!(!out.dice[2].is_hit); // 4
    assert!(!out.dice[3].is_hit); // 3
    assert!(!out.dice[4].is_hit); // 2
    assert!(!out.dice[5].is_hit); // 1
}

#[test]
fn no_hits_is_a_plain_failure() {
    let out = score(&[4, 3, 2], &opts());
    assert_eq!(out.hits, 0);
    assert!(!out.glitch);
    assert_eq!(out.result, Sr5Result::Failure);
    assert_eq!(out.net_hits, -1); // 0 - threshold 1
}

// ----------------------------------------------------------------------- glitch

#[test]
fn no_glitch_at_exactly_half_ones() {
    // SR5 RAW: glitch is MORE THAN half. 4 dice, 2 ones -> 2*2 == 4, NOT > 4
    // -> no glitch (a 5 gives one hit).
    let out = score(&[1, 1, 5, 3], &opts());
    assert!(!out.glitch);
    assert_eq!(out.ones, 2);
    assert_eq!(out.hits, 1);
    assert_eq!(out.result, Sr5Result::Success(0));
}

#[test]
fn glitch_above_half_ones() {
    // 4 dice, 3 ones -> 3*2 == 6 > 4 -> glitch. A 5 keeps it out of critical.
    let out = score(&[1, 1, 1, 5], &opts());
    assert!(out.glitch);
    assert_eq!(out.ones, 3);
    assert_eq!(out.hits, 1);
    assert_eq!(out.result, Sr5Result::Glitch(1));
}

#[test]
fn no_glitch_below_half_ones() {
    // 4 dice, 1 one -> 1*2 == 2 < 4 -> no glitch.
    let out = score(&[1, 3, 4, 5], &opts());
    assert!(!out.glitch);
    assert_eq!(out.ones, 1);
    assert_eq!(out.hits, 1);
    assert_eq!(out.result, Sr5Result::Success(0));
}

#[test]
fn odd_pool_needs_strictly_more_than_half_ones() {
    // 5 dice: 2 ones -> 4 < 5 no glitch; 3 ones -> 6 >= 5 glitch.
    let two = score(&[1, 1, 5, 5, 3], &opts());
    assert!(!two.glitch);
    assert_eq!(two.hits, 2);

    let three = score(&[1, 1, 1, 5, 3], &opts());
    assert!(three.glitch);
    assert_eq!(three.hits, 1);
    assert_eq!(three.result, Sr5Result::Glitch(1));
}

#[test]
fn critical_glitch_is_glitch_with_zero_hits() {
    // 4 dice, 3 ones (more than half), zero hits -> critical glitch.
    let out = score(&[1, 1, 1, 3], &opts());
    assert!(out.glitch);
    assert_eq!(out.hits, 0);
    assert_eq!(out.result, Sr5Result::CriticalGlitch);
    assert!(out.is_critical_glitch());
    assert_eq!(out.net_hits, -1);
}

#[test]
fn glitch_with_enough_hits_is_still_a_glitch_not_success() {
    // 6 dice, 4 ones (more than half) -> glitch; two hits (5,6) meet threshold
    // 1 but the roll still glitches -> Glitch(2), never Success.
    let out = score(&[1, 1, 1, 1, 5, 6], &opts());
    assert!(out.glitch);
    assert_eq!(out.hits, 2);
    assert_eq!(out.result, Sr5Result::Glitch(2));
    assert!(!out.is_success());
    assert_eq!(out.net_hits, 1); // hits - threshold is still tracked
}

// ------------------------------------------------------------------------ limit

#[test]
fn limit_caps_hits_and_flags_the_wasted_dice() {
    // 5 raw hits, limit 3 -> 3 counted; the 4th and 5th hitting dice are wasted.
    let out = score(&[5, 5, 6, 6, 5], &Sr5Opts::new(1).with_limit(3));
    assert_eq!(out.raw_hits, 5);
    assert_eq!(out.hits, 3);
    assert_eq!(out.limit_applied, Some(3));
    assert!(!out.dice[0].limit_capped);
    assert!(!out.dice[1].limit_capped);
    assert!(!out.dice[2].limit_capped);
    assert!(out.dice[3].limit_capped);
    assert!(out.dice[4].limit_capped);
    assert_eq!(out.net_hits, 2);
    assert_eq!(out.result, Sr5Result::Success(2));
}

#[test]
fn limit_not_reached_caps_nothing() {
    let out = score(&[5, 6, 2, 3], &Sr5Opts::new(1).with_limit(4));
    assert_eq!(out.raw_hits, 2);
    assert_eq!(out.hits, 2);
    assert_eq!(out.limit_applied, Some(4));
    assert!(out.dice.iter().all(|d| !d.limit_capped));
}

// ------------------------------------------------------------- edge / rule of six

#[test]
fn edge_push_ignores_the_limit_and_explodes_sixes() {
    // Pool 2, pushing the limit, limit 1 (must be ignored). Scripted stream:
    // initial [6, 3]; the 6 explodes into a 5 (a hit, no further explosion).
    let mut rng = SeqRng::faces(&[6, 3, 5]);
    let mut ctx = RollCtx::new(&mut rng);
    let out = Sr5Pool.roll(
        2,
        &Sr5Opts::new(1).with_limit(1).pushing_the_limit(),
        &mut ctx,
    );

    // Three dice ended up in the pool: the extra one came from die 0.
    assert_eq!(out.dice.len(), 3);
    assert_eq!(out.dice[0].face, 6);
    assert_eq!(out.dice[1].face, 3);
    assert_eq!(out.dice[2].face, 5);
    assert_eq!(out.dice[2].exploded_from, Some(0));
    assert!(out.dice[2].is_hit); // the exploded die produced an extra hit

    // Limit is waived: both the 6 and the exploded 5 count.
    assert_eq!(out.raw_hits, 2);
    assert_eq!(out.hits, 2);
    assert_eq!(out.limit_applied, None);
    assert_eq!(out.result, Sr5Result::Success(1));
}

#[test]
fn rule_of_six_chains_and_records_provenance() {
    // Pool 1 pushing: 6 -> 6 -> 5. Each 6 spawns the next die, chained.
    let mut rng = SeqRng::faces(&[6, 6, 5]);
    let mut ctx = RollCtx::new(&mut rng);
    let out = Sr5Pool.roll(1, &Sr5Opts::new(1).pushing_the_limit(), &mut ctx);

    assert_eq!(out.dice.len(), 3);
    assert_eq!(out.dice[0].exploded_from, None);
    assert_eq!(out.dice[1].exploded_from, Some(0));
    assert_eq!(out.dice[2].exploded_from, Some(1));
    assert_eq!(out.raw_hits, 3); // 6, 6, 5 all hit
    assert_eq!(out.hits, 3);
}

#[test]
fn edge_dice_add_to_the_pool_without_exploding() {
    // 3 edge dice on a pool of 2, no push -> 5 dice, no explosion even on a 6.
    let mut rng = SeqRng::faces(&[6, 5, 6, 2, 1]);
    let mut ctx = RollCtx::new(&mut rng);
    let out = Sr5Pool.roll(2, &Sr5Opts::new(1).with_edge_dice(3), &mut ctx);
    assert_eq!(out.dice.len(), 5); // no extra dice from the 6s
    assert!(out.dice.iter().all(|d| d.exploded_from.is_none()));
    assert_eq!(out.raw_hits, 3); // 6, 5, 6
}

// ----------------------------------------------------------------- threshold/net

#[test]
fn threshold_and_net_hits() {
    let win = score(&[6, 6, 6, 5], &Sr5Opts::new(2));
    assert_eq!(win.hits, 4);
    assert_eq!(win.net_hits, 2);
    assert_eq!(win.result, Sr5Result::Success(2));

    let lose = score(&[6, 3, 2], &Sr5Opts::new(2));
    assert_eq!(lose.hits, 1);
    assert_eq!(lose.net_hits, -1);
    assert_eq!(lose.result, Sr5Result::Failure);

    let exact = score(&[6, 5], &Sr5Opts::new(2));
    assert_eq!(exact.net_hits, 0);
    assert_eq!(exact.result, Sr5Result::Success(0));
}

// ------------------------------------------------------------------- provenance

#[test]
fn per_die_flags_are_recorded() {
    let out = score(&[6, 1, 3], &opts());
    assert_eq!(
        out.dice[0],
        Sr5Die {
            face: 6,
            is_hit: true,
            is_one: false,
            limit_capped: false,
            exploded_from: None,
        }
    );
    assert_eq!(
        out.dice[1],
        Sr5Die {
            face: 1,
            is_hit: false,
            is_one: true,
            limit_capped: false,
            exploded_from: None,
        }
    );
    assert_eq!(
        out.dice[2],
        Sr5Die {
            face: 3,
            is_hit: false,
            is_one: false,
            limit_capped: false,
            exploded_from: None,
        }
    );
}

// -------------------------------------------------------------------- contested

#[test]
fn opposed_reports_net_hit_difference() {
    let actor = score(&[6, 6, 6], &opts()); // 3 hits
    let opp = score(&[6, 3, 2], &opts()); // 1 hit
    assert_eq!(opposed(&actor, &opp), 2);
    assert_eq!(opposed(&opp, &actor), -2);
    assert_eq!(contest(&actor, &opp), Sr5Contest::ActorWins(2));
    assert_eq!(contest(&opp, &actor), Sr5Contest::OpponentWins(2));
}

#[test]
fn opposed_tie_favours_no_one() {
    let a = score(&[6, 5], &opts()); // 2
    let b = score(&[6, 6], &opts()); // 2
    assert_eq!(opposed(&a, &b), 0);
    assert_eq!(contest(&a, &b), Sr5Contest::Tie);
}

// -------------------------------------------------------------------- extended

#[test]
fn extended_accumulates_toward_threshold() {
    let mut ext = Sr5Extended::new(5, 4);
    assert_eq!(
        ext.add(score(&[6, 6, 3], &opts())),
        Sr5ExtendedStatus::Continuing
    ); // +2 = 2
    assert_eq!(
        ext.add(score(&[6, 3, 2], &opts())),
        Sr5ExtendedStatus::Continuing
    ); // +1 = 3
    assert_eq!(
        ext.add(score(&[6, 6, 6], &opts())),
        Sr5ExtendedStatus::Complete
    ); // +3 = 6
    assert_eq!(ext.accumulated, 6);
    assert!(ext.is_complete());
    assert_eq!(ext.rolls.len(), 3);
    assert_eq!(ext.next_pool, 1); // decremented once per interval from 4
}

#[test]
fn extended_runs_out_of_dice() {
    let mut ext = Sr5Extended::new(10, 2);
    assert_eq!(
        ext.add(score(&[6, 6], &opts())),
        Sr5ExtendedStatus::Continuing
    ); // +2, pool ->1
    assert_eq!(
        ext.add(score(&[6, 3], &opts())),
        Sr5ExtendedStatus::Exhausted
    ); // +1, pool ->0
    assert_eq!(ext.accumulated, 3);
    assert!(!ext.is_complete());
}

#[test]
fn extended_critical_glitch_ends_the_test() {
    let mut ext = Sr5Extended::new(5, 4);
    ext.add(score(&[6, 6], &opts())); // +2
    let status = ext.add(score(&[1, 1], &opts())); // critical glitch
    assert_eq!(status, Sr5ExtendedStatus::CriticalGlitch);
    assert!(ext.critical_glitched);
    assert!(!ext.is_complete());
    assert_eq!(ext.accumulated, 2); // no auto-undo of prior progress
}

#[test]
fn extended_plain_glitch_is_surfaced_but_hits_still_count() {
    let mut ext = Sr5Extended::new(5, 4);
    let status = ext.add(score(&[1, 1, 1, 5, 5], &opts())); // glitch (3 of 5 ones), 2 hits
    assert_eq!(status, Sr5ExtendedStatus::Glitch);
    assert!(ext.glitched);
    assert_eq!(ext.accumulated, 2);
    assert!(!ext.is_complete());
}

// ------------------------------------------------------------------- formatter

#[test]
fn summary_reads_like_a_roll_log() {
    assert_eq!(
        score(&[5, 6, 3, 1], &Sr5Opts::new(1).with_limit(4)).summary(),
        "rolled 5 6 3 1 (limit 4) -> 2 hits"
    );
    assert_eq!(
        score(&[5, 2, 3], &opts()).summary(),
        "rolled 5 2 3 -> 1 hit"
    );
    assert_eq!(
        score(&[3, 2, 4], &opts()).summary(),
        "rolled 3 2 4 -> 0 hits"
    );
    assert_eq!(
        score(&[1, 1, 3], &opts()).summary(),
        "rolled 1 1 3 -> critical glitch"
    );
    assert_eq!(
        score(&[1, 1, 1, 5], &opts()).summary(),
        "rolled 1 1 1 5 -> glitch (1 hit)"
    );
    assert_eq!(
        score(&[1, 1, 1, 5, 6], &opts()).summary(),
        "rolled 1 1 1 5 6 -> glitch (2 hits)"
    );
}

#[test]
fn display_matches_summary() {
    let out = score(&[5, 6, 3], &opts());
    assert_eq!(format!("{out}"), out.summary());
}

// ---------------------------------------------------------------- seeded roll

#[test]
fn pool_roll_is_seeded_and_reproducible() {
    let o = opts();
    let mk = || {
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let mut c = RollCtx::new(&mut rng);
        Sr5Pool.roll(8, &o, &mut c)
    };
    let a = mk();
    let b = mk();
    assert_eq!(a, b);
    assert_eq!(a.dice.len(), 8); // non-edge: no explosion
    // Re-scoring the logged faces reproduces the outcome exactly.
    let faces: Vec<u8> = a.dice.iter().map(|d| d.face).collect();
    assert_eq!(score(&faces, &o), a);
}

#[test]
fn pool_of_zero_rolls_no_dice() {
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut c = RollCtx::new(&mut rng);
    let out = Sr5Pool.roll(0, &opts(), &mut c);
    assert!(out.dice.is_empty());
    assert_eq!(out.hits, 0);
    assert!(!out.glitch);
    assert_eq!(out.result, Sr5Result::Failure);
}

#[test]
fn negative_pool_floors_at_zero_dice() {
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut c = RollCtx::new(&mut rng);
    let out = Sr5Pool.roll(3, &Sr5Opts::new(1).with_modifier(-5), &mut c);
    assert!(out.dice.is_empty());
}

#[test]
fn edge_dice_can_rescue_a_zero_pool() {
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut c = RollCtx::new(&mut rng);
    let out = Sr5Pool.roll(0, &Sr5Opts::new(1).with_edge_dice(3), &mut c);
    assert_eq!(out.dice.len(), 3);
}

#[test]
fn pool_roll_respects_dice_cap() {
    let limits = mush_dice::RollLimits {
        max_dice: 10,
        ..Default::default()
    };
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut c = RollCtx::with_limits(&mut rng, limits);
    let out = Sr5Pool.roll(500, &opts(), &mut c);
    assert_eq!(out.dice.len(), 10);
}

#[test]
fn rule_of_six_explosion_is_bounded_by_the_dice_cap() {
    // A stream of all-6s would explode forever; the cap halts it.
    let limits = mush_dice::RollLimits {
        max_dice: 12,
        ..Default::default()
    };
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let mut c = RollCtx::with_limits(&mut rng, limits);
    let out = Sr5Pool.roll(20, &Sr5Opts::new(1).pushing_the_limit(), &mut c);
    assert!(out.dice.len() <= 12);
}

// -------------------------------------------------------------- serialization

#[test]
fn outcome_serializes_for_the_event_bus() {
    let out = score(&[5, 5, 6, 6, 5], &Sr5Opts::new(1).with_limit(3));
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hits"], 3);
    assert_eq!(json["raw_hits"], 5);
    assert_eq!(json["limit_applied"], 3);
    assert_eq!(json["result"], serde_json::json!({ "Success": 2 }));
    assert_eq!(json["dice"][3]["limit_capped"], true);
    // round-trips
    let back: Sr5Outcome = serde_json::from_value(json).unwrap();
    assert_eq!(back, out);
}

#[test]
fn critical_glitch_serializes_as_a_unit_variant() {
    let out = score(&[1, 1, 2], &opts());
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["result"], serde_json::json!("CriticalGlitch"));
}
