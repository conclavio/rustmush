//! Shadowrun 5th Edition pool mechanic — the d6 hit pool.
//!
//! Rules (authoritative for rustmush; matches the milestone crib exactly):
//!
//! - Roll a pool of `N` d6 (`N = attribute + skill + modifiers`, plus any
//!   `edge_dice`). Each die showing **5 or 6 is a HIT**.
//! - **Glitch**: if *more than half* of the dice rolled show a `1`, the roll
//!   glitches (SR5 RAW) — `ones * 2 > dice_rolled`. A glitch with **zero
//!   hits** is a **critical glitch**.
//! - **Threshold**: the difficulty (default `1`). `hits >= threshold` succeeds;
//!   `net hits = hits - threshold`.
//! - **Limit** (SR5's signature): a test cannot count more hits than its Limit.
//!   Excess hitting dice are wasted and flagged [`Sr5Die::limit_capped`].
//! - **Edge — push the limit** ([`Sr5Opts::edge_push`]): ignore the Limit for
//!   the test *and* apply the **Rule of Six** — every `6` explodes into another
//!   d6 (which can hit and explode again), bounded by [`crate::RollLimits`] so a
//!   run of 6s can never loop forever. [`Sr5Opts::edge_dice`] models the simpler
//!   "spend Edge for extra dice" (added to the pool without the reroll).
//!
//! Every die keeps its provenance ([`Sr5Die`]) for the roll log and web panel.
//! [`Sr5Pool::score`] is a pure function of the rolled faces (RNG-free), so a
//! logged roll can be re-scored/replayed; the Rule-of-Six explosion is a
//! roll-time concern (it needs the RNG) and threads the `exploded_from`
//! linkage in through [`Sr5Pool::roll`].
//!
//! ## Glitch counting under the Rule of Six
//!
//! When `edge_push` explodes 6s, the exploded dice are ordinary members of the
//! pool: they count for hits **and** their `1`s and their count feed the glitch
//! test (`ones * 2 > dice.len()`), because `dice_rolled` is the whole rolled
//! pool. This keeps [`score`] a
//! pure function of the flat face list. (Some tables count only the *initial*
//! pool toward glitches; that is a documented house-rule variant, not the model
//! here.)
//!
//! [`score`]: Sr5Pool::score

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::expr::RollCtx;
use crate::pool::PoolMechanic;

/// Options for an SR5 pool roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sr5Opts {
    /// Difficulty: hits needed to succeed (default `1`). `net = hits - threshold`.
    pub threshold: u8,
    /// The test's Limit. `None` = unlimited; `Some(n)` caps counted hits at `n`
    /// (excess hitting dice are wasted). Ignored entirely when `edge_push`.
    pub limit: Option<u8>,
    /// Edge "push the limit": ignore the Limit for this test **and** apply the
    /// Rule of Six (6s explode into extra dice).
    pub edge_push: bool,
    /// Extra dice added to the pool (e.g. spending Edge for `+Edge` dice). These
    /// are ordinary dice; the Rule of Six only applies when `edge_push` is set.
    pub edge_dice: u8,
    /// Flat dice-pool modifier (wound penalties, situational mods). Added to the
    /// pool; a pool driven below `0` simply rolls no dice (0 hits).
    pub modifier: i32,
}

impl Default for Sr5Opts {
    fn default() -> Self {
        Self {
            threshold: 1,
            limit: None,
            edge_push: false,
            edge_dice: 0,
            modifier: 0,
        }
    }
}

impl Sr5Opts {
    /// Options at `threshold`, everything else default.
    pub fn new(threshold: u8) -> Self {
        Self {
            threshold,
            ..Self::default()
        }
    }

    /// Builder: set the Limit.
    pub fn with_limit(mut self, limit: u8) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Builder: spend Edge to *push the limit* (ignore Limit + Rule of Six).
    pub fn pushing_the_limit(mut self) -> Self {
        self.edge_push = true;
        self
    }

    /// Builder: add `n` Edge dice to the pool.
    pub fn with_edge_dice(mut self, n: u8) -> Self {
        self.edge_dice = n;
        self
    }

    /// Builder: set the dice-pool modifier.
    pub fn with_modifier(mut self, modifier: i32) -> Self {
        self.modifier = modifier;
        self
    }

    /// Threshold as `u32` (at least the configured value; `0` is legal and means
    /// any number of hits — including zero — succeeds unless a glitch intervenes).
    fn threshold_u32(&self) -> u32 {
        u32::from(self.threshold)
    }
}

/// One d6 in an SR5 roll, with why-it-counted provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sr5Die {
    pub face: u8,
    /// `face >= 5` (a hit).
    pub is_hit: bool,
    /// `face == 1` (feeds the glitch test).
    pub is_one: bool,
    /// A hitting die whose hit was discarded by the Limit.
    pub limit_capped: bool,
    /// If this die was rolled by the Rule of Six, the index (in `dice`) of the
    /// `6` that spawned it. `None` for an original pool die.
    pub exploded_from: Option<u16>,
}

/// The classified result of an SR5 roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sr5Result {
    /// A glitch with zero hits — disaster.
    CriticalGlitch,
    /// A glitch with hits; payload is the (post-Limit) hit count. Something goes
    /// wrong even though the dice may have met the threshold.
    Glitch(u32),
    /// Not a glitch, hits below the threshold.
    Failure,
    /// Not a glitch, hits met the threshold; payload is the net hits.
    Success(u32),
}

/// Full outcome of an SR5 pool roll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sr5Outcome {
    /// Every die, in roll order (exploded dice appended after their pool).
    pub dice: Vec<Sr5Die>,
    /// Hits before the Limit is applied (`face >= 5`, all dice).
    pub raw_hits: u32,
    /// Number of `1`s rolled (all dice).
    pub ones: u32,
    /// The Limit actually in effect (`None` when unlimited or when `edge_push`
    /// waived it).
    pub limit_applied: Option<u8>,
    /// Counted hits after the Limit (`min(raw_hits, limit)`, or `raw_hits` when
    /// unlimited / pushing).
    pub hits: u32,
    /// `hits - threshold` (may be negative on a miss).
    pub net_hits: i32,
    /// Whether the roll glitched (`ones * 2 > dice.len()`; SR5 RAW = more than half).
    pub glitch: bool,
    /// Classified result.
    pub result: Sr5Result,
}

impl Sr5Outcome {
    /// Post-Limit hits, for opposed comparisons.
    pub fn hits(&self) -> u32 {
        self.hits
    }

    /// Whether the roll met its threshold (a plain success — a glitch that also
    /// met the threshold is *not* counted here).
    pub fn is_success(&self) -> bool {
        matches!(self.result, Sr5Result::Success(_))
    }

    /// Whether the roll glitched (either flavour).
    pub fn is_glitch(&self) -> bool {
        self.glitch
    }

    /// Whether the roll critically glitched.
    pub fn is_critical_glitch(&self) -> bool {
        matches!(self.result, Sr5Result::CriticalGlitch)
    }

    /// The dice as thrown — `"5 6 3 1"`, or `"(no dice)"` for an empty
    /// pool: the left-hand side of [`Sr5Outcome::summary`] and of the
    /// one-line room narration (interface plan I3).
    pub fn faces_text(&self) -> String {
        if self.dice.is_empty() {
            "(no dice)".to_string()
        } else {
            self.dice
                .iter()
                .map(|d| d.face.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        }
    }

    /// `"limit 4"` when a Limit was in effect (`limit_applied`), else `None`.
    pub fn limit_text(&self) -> Option<String> {
        self.limit_applied.map(|n| format!("limit {n}"))
    }

    /// The classified result in words — `"2 hits"`, `"1 hit"`,
    /// `"glitch (1 hit)"`, `"critical glitch"`.
    pub fn result_text(&self) -> String {
        match self.result {
            Sr5Result::CriticalGlitch => "critical glitch".to_string(),
            Sr5Result::Glitch(h) => format!("glitch ({} {})", h, hits_word(h)),
            Sr5Result::Failure | Sr5Result::Success(_) => {
                format!("{} {}", self.hits, hits_word(self.hits))
            }
        }
    }

    /// A classic one-line summary for the roll log / ANSI, e.g.
    /// `"rolled 5 6 3 1 (limit 4) -> 2 hits"` or
    /// `"rolled 1 1 3 -> critical glitch"`.
    pub fn summary(&self) -> String {
        let limit = self
            .limit_text()
            .map_or_else(String::new, |l| format!(" ({l})"));
        format!(
            "rolled {}{limit} -> {}",
            self.faces_text(),
            self.result_text()
        )
    }
}

impl fmt::Display for Sr5Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

/// `"hit"` for exactly one, `"hits"` otherwise.
fn hits_word(n: u32) -> &'static str {
    if n == 1 { "hit" } else { "hits" }
}

/// The SR5 d6 hit-pool mechanic (unit type).
#[derive(Debug, Clone, Copy, Default)]
pub struct Sr5Pool;

impl Sr5Pool {
    /// Score a known set of d6 faces under `opts`.
    ///
    /// Pure and RNG-free — scores a live roll and re-scores a logged one. Every
    /// die is treated as an original pool die (`exploded_from = None`); the
    /// Rule-of-Six linkage is only available through [`Sr5Pool::roll`].
    pub fn score(faces: &[u8], opts: &Sr5Opts) -> Sr5Outcome {
        Self::score_with_origins(faces, &[], opts)
    }

    /// Scoring core. `origins[i]` (when present) is the `exploded_from` of face
    /// `i`; an empty `origins` slice means every die is an original.
    fn score_with_origins(faces: &[u8], origins: &[Option<u16>], opts: &Sr5Opts) -> Sr5Outcome {
        let mut dice = Vec::with_capacity(faces.len());
        let mut raw_hits = 0u32;
        let mut ones = 0u32;

        for (i, &face) in faces.iter().enumerate() {
            let is_hit = face >= 5;
            let is_one = face == 1;
            if is_hit {
                raw_hits += 1;
            }
            if is_one {
                ones += 1;
            }
            dice.push(Sr5Die {
                face,
                is_hit,
                is_one,
                limit_capped: false,
                exploded_from: origins.get(i).copied().flatten(),
            });
        }

        let dice_rolled = dice.len() as u32;
        // SR5 RAW: a glitch is MORE THAN half the dice showing a 1 (strictly
        // greater), not half-or-more. With 4 dice you need 3 ones, not 2.
        let glitch = dice_rolled > 0 && ones.saturating_mul(2) > dice_rolled;

        // Apply the Limit (waived while pushing the limit).
        let (hits, limit_applied) = if opts.edge_push {
            (raw_hits, None)
        } else {
            match opts.limit {
                Some(lim) => {
                    let cap = u32::from(lim);
                    if raw_hits > cap {
                        // Keep the first `cap` hitting dice; flag the rest.
                        let mut kept = 0u32;
                        for d in dice.iter_mut() {
                            if d.is_hit {
                                if kept < cap {
                                    kept += 1;
                                } else {
                                    d.limit_capped = true;
                                }
                            }
                        }
                        (cap, Some(lim))
                    } else {
                        (raw_hits, Some(lim))
                    }
                }
                None => (raw_hits, None),
            }
        };

        let threshold = opts.threshold_u32();
        let net_hits = hits as i32 - threshold as i32;

        let result = if hits == 0 && glitch {
            Sr5Result::CriticalGlitch
        } else if glitch {
            Sr5Result::Glitch(hits)
        } else if hits >= threshold {
            Sr5Result::Success(hits - threshold)
        } else {
            Sr5Result::Failure
        };

        Sr5Outcome {
            dice,
            raw_hits,
            ones,
            limit_applied,
            hits,
            net_hits,
            glitch,
            result,
        }
    }

    /// Effective number of pool dice to roll: `(pool + modifier)` floored at 0,
    /// plus `edge_dice`, capped by the DoS dice limit.
    fn dice_to_roll(pool: i32, opts: &Sr5Opts, ctx: &RollCtx<'_>) -> usize {
        let base = (i64::from(pool) + i64::from(opts.modifier)).max(0);
        let want = base + i64::from(opts.edge_dice);
        let max = i64::from(ctx.limits.max_dice.max(1));
        want.clamp(0, max) as usize
    }
}

impl PoolMechanic for Sr5Pool {
    type Opts = Sr5Opts;
    type Outcome = Sr5Outcome;

    fn roll(&self, pool: i32, opts: &Self::Opts, ctx: &mut RollCtx<'_>) -> Self::Outcome {
        let n = Self::dice_to_roll(pool, opts, ctx);
        if opts.edge_push {
            let (faces, origins) = roll_exploding(n, ctx);
            Self::score_with_origins(&faces, &origins, opts)
        } else {
            let faces = roll_plain(n, ctx);
            Self::score(&faces, opts)
        }
    }
}

/// Roll a single d6 (`1..=6`).
fn roll_d6(ctx: &mut RollCtx<'_>) -> u8 {
    (ctx.rng.next_u32() % 6) as u8 + 1
}

/// Roll `n` plain d6.
fn roll_plain(n: usize, ctx: &mut RollCtx<'_>) -> Vec<u8> {
    (0..n).map(|_| roll_d6(ctx)).collect()
}

/// Roll `n` d6 applying the Rule of Six: every `6` (original or exploded)
/// spawns another d6, bounded by the total-dice limit so a run of 6s halts.
fn roll_exploding(n: usize, ctx: &mut RollCtx<'_>) -> (Vec<u8>, Vec<Option<u16>>) {
    let budget = ctx.limits.max_dice.max(1) as usize;
    let mut faces = Vec::with_capacity(n);
    let mut origins = Vec::with_capacity(n);
    for _ in 0..n {
        faces.push(roll_d6(ctx));
        origins.push(None);
    }
    // Scan the growing pool; each 6 appends one more die (which is scanned in
    // turn, so a 6-on-a-6 explodes again) until the dice budget is exhausted.
    let mut i = 0;
    while i < faces.len() {
        if faces[i] == 6 && faces.len() < budget {
            let f = roll_d6(ctx);
            faces.push(f);
            origins.push(Some(i as u16));
        }
        i += 1;
    }
    (faces, origins)
}

/// Net-hits difference of an opposed test, from the actor's perspective:
/// `actor.hits - opponent.hits`. Positive = the actor wins by that many net
/// hits, negative = the opponent wins, `0` = a tie. Post-Limit hits are used;
/// each side's own Limit already shaped its `hits`.
pub fn opposed(actor: &Sr5Outcome, opponent: &Sr5Outcome) -> i32 {
    actor.hits as i32 - opponent.hits as i32
}

/// Winner of an opposed test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sr5Contest {
    /// Actor won by this many net hits (`>= 1`).
    ActorWins(u32),
    /// Opponent won by this many net hits (`>= 1`).
    OpponentWins(u32),
    /// Equal hits — the defender/status quo prevails (SR5: ties go to the
    /// defender), but no margin either way.
    Tie,
}

/// Classify an opposed test from the actor's perspective.
pub fn contest(actor: &Sr5Outcome, opponent: &Sr5Outcome) -> Sr5Contest {
    use std::cmp::Ordering;
    match actor.hits.cmp(&opponent.hits) {
        Ordering::Greater => Sr5Contest::ActorWins(actor.hits - opponent.hits),
        Ordering::Less => Sr5Contest::OpponentWins(opponent.hits - actor.hits),
        Ordering::Equal => Sr5Contest::Tie,
    }
}

/// An accumulator for an **extended test**: the same pool rolled repeatedly,
/// hits accumulating toward a threshold, with the pool losing one die each
/// interval.
///
/// A critical glitch ends the test in disaster; a plain glitch is surfaced (the
/// caller decides the fallout — typically lost time or progress) but hits are
/// still recorded. Running the pool out of dice before reaching the threshold is
/// [`Sr5ExtendedStatus::Exhausted`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sr5Extended {
    /// Total hits required to complete.
    pub threshold: u32,
    /// Cumulative hits so far (each roll adds its post-Limit hits).
    pub accumulated: u32,
    /// Dice the *next* interval will roll (decrements by one each `add`).
    pub next_pool: i32,
    /// Every interval roll, in order.
    pub rolls: Vec<Sr5Outcome>,
    /// Whether any interval glitched.
    pub glitched: bool,
    /// Whether any interval critically glitched (terminal).
    pub critical_glitched: bool,
}

/// Status of an [`Sr5Extended`] test after adding an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sr5ExtendedStatus {
    /// Below threshold, no glitch, dice remain — roll again.
    Continuing,
    /// Accumulated hits reached the threshold.
    Complete,
    /// This interval glitched (not critical); caller decides the consequence.
    Glitch,
    /// This interval critically glitched — the test fails.
    CriticalGlitch,
    /// The pool ran out of dice before reaching the threshold.
    Exhausted,
}

impl Sr5Extended {
    /// A fresh extended test needing `threshold` hits, starting at `pool` dice.
    pub fn new(threshold: u32, pool: i32) -> Self {
        Self {
            threshold,
            accumulated: 0,
            next_pool: pool,
            rolls: Vec::new(),
            glitched: false,
            critical_glitched: false,
        }
    }

    /// Record one interval roll and return the resulting status.
    pub fn add(&mut self, outcome: Sr5Outcome) -> Sr5ExtendedStatus {
        let crit = matches!(outcome.result, Sr5Result::CriticalGlitch);
        let glitch = outcome.glitch;
        self.accumulated = self.accumulated.saturating_add(outcome.hits);
        if glitch {
            self.glitched = true;
        }
        if crit {
            self.critical_glitched = true;
        }
        self.rolls.push(outcome);
        self.next_pool -= 1;

        if crit {
            Sr5ExtendedStatus::CriticalGlitch
        } else if self.accumulated >= self.threshold {
            Sr5ExtendedStatus::Complete
        } else if glitch {
            Sr5ExtendedStatus::Glitch
        } else if self.next_pool < 1 {
            Sr5ExtendedStatus::Exhausted
        } else {
            Sr5ExtendedStatus::Continuing
        }
    }

    /// Whether the threshold has been met (and no critical glitch intervened).
    pub fn is_complete(&self) -> bool {
        !self.critical_glitched && self.accumulated >= self.threshold
    }
}
