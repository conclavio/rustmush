//! Classic World of Darkness 20th Anniversary (V20 / M20 / W20) pool mechanic —
//! the Storyteller System d10 pool.
//!
//! Rules (authoritative for rustmush; matches the milestone crib exactly):
//!
//! - Roll a pool of `N` d10 (`N >= 1`; a pool driven to `0` or below rolls a
//!   single die).
//! - **Difficulty** is a target number, default `6`, clamped to `2..=10`. A die
//!   is a success when `face >= difficulty`.
//! - Each **1** subtracts one success.
//! - **Botch**: *no* die reached the difficulty **and** at least one `1` shows
//!   (severity = number of `1`s). Rolling any success at all — including a
//!   Willpower automatic success — makes a botch impossible; the worst that can
//!   then happen is a plain failure.
//! - **Specialty** (a.k.a. 10-again): each `10` counts as **two** successes. It
//!   is not a reroll.
//! - **Willpower**: spending a point adds one automatic success and prevents a
//!   botch.
//! - `net = successes(counting doubled 10s) - ones + willpower + extra`. If a
//!   botch applies, that wins; otherwise `net <= 0` is a failure and `net > 0`
//!   is that many successes.
//!
//! Every die retains its provenance ([`Owod20Die`]) for the roll log and web
//! panel. [`Owod20Pool::score`] is a pure function of the rolled faces, so a
//! logged roll can be re-scored/replayed without the RNG.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::expr::RollCtx;
use crate::pool::PoolMechanic;

/// Options for an oWoD20 pool roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owod20Opts {
    /// ST target number; clamped to `2..=10` on use (default `6`).
    pub difficulty: u8,
    /// Roller has an applicable specialty: 10s count double.
    pub specialty: bool,
    /// A Willpower point is spent: +1 automatic success, no botch.
    pub willpower: bool,
    /// Automatic successes from other sources (merits, powers, ST fiat).
    pub extra_successes: i32,
    /// Explicit "10s count double" flag. Equivalent to `specialty`; kept
    /// separate so a caller can enable 10-again for a non-specialty reason.
    pub again_tens_double: bool,
}

impl Default for Owod20Opts {
    fn default() -> Self {
        Self {
            difficulty: 6,
            specialty: false,
            willpower: false,
            extra_successes: 0,
            again_tens_double: false,
        }
    }
}

impl Owod20Opts {
    /// Options at `difficulty`, everything else default.
    pub fn new(difficulty: u8) -> Self {
        Self {
            difficulty,
            ..Self::default()
        }
    }

    /// The difficulty actually used, clamped to the legal `2..=10` range.
    pub fn effective_difficulty(&self) -> u8 {
        self.difficulty.clamp(2, 10)
    }

    /// Whether 10s count as two successes for this roll.
    pub fn double_tens(&self) -> bool {
        self.specialty || self.again_tens_double
    }
}

/// One d10 in an oWoD20 roll, with why-it-counted provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owod20Die {
    pub face: u8,
    /// `face >= difficulty`.
    pub is_success: bool,
    /// `face == 1` (subtracts a success).
    pub is_one: bool,
    /// A `10` counted twice (specialty / 10-again).
    pub doubled: bool,
}

/// The classified result of an oWoD20 roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollResult {
    /// No successes and one or more 1s; payload is the number of 1s.
    Botch(u8),
    /// Zero net successes (but not a botch).
    Failure,
    /// This many net successes.
    Success(u8),
}

/// Full outcome of an oWoD20 pool roll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owod20Outcome {
    /// Difficulty actually applied (post-clamp).
    pub difficulty: u8,
    /// Every die, in roll order.
    pub dice: Vec<Owod20Die>,
    /// Successes counting doubled 10s, before subtracting 1s / adding bonuses.
    pub raw_successes: i32,
    /// Number of 1s rolled.
    pub ones: i32,
    /// Whether a Willpower automatic success was applied.
    pub willpower_success: bool,
    /// Automatic successes contributed from `extra_successes`.
    pub extra_successes: i32,
    /// `raw_successes - ones + willpower + extra_successes`.
    pub net: i32,
    /// Classified result.
    pub result: RollResult,
}

impl Owod20Outcome {
    /// Net successes for comparison purposes: `Success(n) -> n`, everything
    /// else `0`. A failure or botch never counts as negative in a contest.
    pub fn successes(&self) -> i32 {
        match self.result {
            RollResult::Success(n) => i32::from(n),
            _ => 0,
        }
    }

    /// Whether the roll botched.
    pub fn is_botch(&self) -> bool {
        matches!(self.result, RollResult::Botch(_))
    }

    /// The dice as thrown — `"7 8 2 1"` (`"(no dice)"` for an empty pool),
    /// followed by ` +WP` when a Willpower success was applied: the left-hand
    /// side of [`Owod20Outcome::summary`] and of the one-line room narration
    /// (interface plan I3).
    pub fn dice_text(&self) -> String {
        let faces = if self.dice.is_empty() {
            "(no dice)".to_string()
        } else {
            self.dice
                .iter()
                .map(|d| d.face.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let wp = if self.willpower_success { " +WP" } else { "" };
        format!("{faces}{wp}")
    }

    /// The classified result in words — `"1 success"`, `"2 successes"`,
    /// `"failure"`, `"botch"`, `"botch (2)"`.
    pub fn result_text(&self) -> String {
        match self.result {
            RollResult::Success(1) => "1 success".to_string(),
            RollResult::Success(n) => format!("{n} successes"),
            RollResult::Failure => "failure".to_string(),
            RollResult::Botch(1) => "botch".to_string(),
            RollResult::Botch(n) => format!("botch ({n})"),
        }
    }

    /// A classic one-line summary for the roll log / ANSI, e.g.
    /// `"rolled 7 8 2 1 -> 1 success"` or `"rolled 3 2 1 -> botch"`.
    pub fn summary(&self) -> String {
        format!("rolled {} -> {}", self.dice_text(), self.result_text())
    }
}

impl fmt::Display for Owod20Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

/// The oWoD20 Storyteller-System pool mechanic (unit type).
#[derive(Debug, Clone, Copy, Default)]
pub struct Owod20Pool;

impl Owod20Pool {
    /// Score a known set of d10 faces under `opts`.
    ///
    /// Pure and RNG-free — the same function scores a live roll and re-scores a
    /// logged one. This is where the entire rules table lives.
    pub fn score(faces: &[u8], opts: &Owod20Opts) -> Owod20Outcome {
        let difficulty = opts.effective_difficulty();
        let double_tens = opts.double_tens();

        let mut dice = Vec::with_capacity(faces.len());
        let mut raw_successes = 0i32;
        let mut ones = 0i32;
        let mut any_success = false;

        for &face in faces {
            let is_success = face >= difficulty;
            let is_one = face == 1;
            let doubled = is_success && double_tens && face == 10;
            if is_success {
                any_success = true;
                raw_successes += if doubled { 2 } else { 1 };
            }
            if is_one {
                ones += 1;
            }
            dice.push(Owod20Die {
                face,
                is_success,
                is_one,
                doubled,
            });
        }

        let willpower = i32::from(opts.willpower);
        let net = raw_successes - ones + willpower + opts.extra_successes;

        let result = if !any_success && ones >= 1 && !opts.willpower {
            RollResult::Botch(clamp_u8(ones))
        } else if net <= 0 {
            RollResult::Failure
        } else {
            RollResult::Success(clamp_u8(net))
        };

        Owod20Outcome {
            difficulty,
            dice,
            raw_successes,
            ones,
            willpower_success: opts.willpower,
            extra_successes: opts.extra_successes,
            net,
            result,
        }
    }

    /// Roll `n` d10 from `ctx`, returning the faces (`1..=10`).
    fn roll_faces(n: usize, ctx: &mut RollCtx<'_>) -> Vec<u8> {
        let mut faces = Vec::with_capacity(n);
        for _ in 0..n {
            faces.push((ctx.rng.next_u32() % 10) as u8 + 1);
        }
        faces
    }
}

impl PoolMechanic for Owod20Pool {
    type Opts = Owod20Opts;
    type Outcome = Owod20Outcome;

    fn roll(&self, pool: i32, opts: &Self::Opts, ctx: &mut RollCtx<'_>) -> Self::Outcome {
        // Pool >= 1; a pool driven to 0 or below still rolls a single die. Cap
        // against the DoS limit so a bogus huge pool can't allocate unbounded.
        let want = if pool < 1 { 1 } else { pool as usize };
        let n = want.min(ctx.limits.max_dice.max(1) as usize);
        let faces = Self::roll_faces(n, ctx);
        Self::score(&faces, opts)
    }
}

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, i32::from(u8::MAX)) as u8
}

/// An accumulator for an **extended** roll: repeated rolls building toward a
/// success total.
///
/// The botch *policy* (undo, reset, end) is the caller's — this only records
/// that a botch happened and stops counting it as progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedRoll {
    /// Successes required to complete.
    pub target: i32,
    /// Cumulative net successes so far (failures add 0, botches add 0).
    pub accumulated: i32,
    /// Every roll made, in order.
    pub rolls: Vec<Owod20Outcome>,
    /// Whether any roll in the sequence botched.
    pub botched: bool,
}

/// Status of an [`ExtendedRoll`] after adding a roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtendedStatus {
    /// Not yet at target, no botch — roll again.
    Continuing,
    /// Accumulated successes reached the target.
    Complete,
    /// A roll botched; the caller decides the consequence.
    Botched,
}

impl ExtendedRoll {
    /// A fresh extended roll needing `target` successes.
    pub fn new(target: i32) -> Self {
        Self {
            target,
            accumulated: 0,
            rolls: Vec::new(),
            botched: false,
        }
    }

    /// Record one roll and return the resulting status.
    pub fn add(&mut self, outcome: Owod20Outcome) -> ExtendedStatus {
        match outcome.result {
            RollResult::Success(n) => self.accumulated += i32::from(n),
            RollResult::Failure => {}
            RollResult::Botch(_) => self.botched = true,
        }
        self.rolls.push(outcome);
        self.status()
    }

    /// Current status without adding a roll.
    pub fn status(&self) -> ExtendedStatus {
        if self.botched {
            ExtendedStatus::Botched
        } else if self.accumulated >= self.target {
            ExtendedStatus::Complete
        } else {
            ExtendedStatus::Continuing
        }
    }

    /// Whether the target has been met (and no botch intervened).
    pub fn is_complete(&self) -> bool {
        !self.botched && self.accumulated >= self.target
    }
}

/// Result of a contested / resisted roll from the acting side's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Contest {
    Win,
    Lose,
    /// Equal successes — no clear winner (V20 deadlock).
    Deadlock,
}

/// Resolve a contested roll: compare net successes, ties deadlock.
pub fn contest(actor: &Owod20Outcome, opponent: &Owod20Outcome) -> Contest {
    use std::cmp::Ordering;
    match actor.successes().cmp(&opponent.successes()) {
        Ordering::Greater => Contest::Win,
        Ordering::Less => Contest::Lose,
        Ordering::Equal => Contest::Deadlock,
    }
}
