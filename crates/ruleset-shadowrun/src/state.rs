//! The SR5 module's in-RAM working state (a typed [`Registrar::state`] slot):
//! the seedable server dice RNG, the transient per-character roll-log ring
//! (staff-queryable via `+roll/last`, deliberately **not** persisted), any
//! active extended rolls, and open chargen sessions.
//!
//! Sheets persist as object attributes through
//! [`SheetStore`](mush_ruleset::SheetStore); only this volatile session/roll
//! bookkeeping lives here (mirroring `ruleset-wod`'s `WodState`).

use std::collections::{HashMap, VecDeque};

use mush_core::ObjId;
use mush_dice::sr5::{Sr5Extended, Sr5Outcome};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

use crate::data::Priority;

/// How many recent rolls to keep per character in the transient ring.
pub const ROLL_LOG_CAP: usize = 20;

/// One entry in a character's roll-log ring / the `sr.roll` panel feed.
#[derive(Clone, Debug, Serialize)]
pub struct RollLogEntry {
    /// The roller.
    pub who: ObjId,
    /// The pool expression as typed ("body+pistols").
    pub expr: String,
    /// The one-line rendered summary.
    pub summary: String,
    /// The threshold applied.
    pub threshold: u8,
    /// Wall-clock timestamp (unix seconds).
    pub ts: i64,
    /// Whether the roll was private.
    pub private: bool,
    /// The full dice outcome (per-die provenance for the panel/audit).
    pub outcome: Sr5Outcome,
}

/// An open chargen session for one character (a transient; the sheet itself is
/// the persisted work-in-progress). Records the A–E priority assignment the
/// player chose, so budget assessment knows each category's allotment.
///
/// The letters are re-hydrated from the sheet's staff-only `chargen_priorities`
/// map on resume (reboot-safe), exactly like `ruleset-wod`.
#[derive(Clone, Debug, Default)]
pub struct ChargenSession {
    /// The five priority-category assignments: `[(category, letter), …]`
    /// (metatype / attributes / magic / skills / resources).
    pub priorities: Vec<(String, Priority)>,
}

impl ChargenSession {
    /// The letter assigned to `category`, if any.
    #[must_use]
    pub fn priority(&self, category: &str) -> Option<Priority> {
        self.priorities
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(category))
            .map(|(_, p)| *p)
    }

    /// Whether all five categories have a letter (a full A–E permutation).
    #[must_use]
    pub fn priorities_set(&self) -> bool {
        crate::chargen::PRIORITY_CATEGORIES
            .iter()
            .all(|c| self.priority(c).is_some())
    }
}

/// The SR5 module state slot. `Clone` (world snapshots clone their state), so
/// every field is cloneable — including the RNG (a snapshot shares the stream
/// position, which is fine for inspection).
#[derive(Clone, Debug)]
pub struct Sr5State {
    rng: ChaCha8Rng,
    logs: HashMap<ObjId, VecDeque<RollLogEntry>>,
    extended: HashMap<ObjId, Sr5Extended>,
    chargen: HashMap<ObjId, ChargenSession>,
}

impl Sr5State {
    /// State seeded from the OS (production).
    #[must_use]
    pub fn from_os() -> Sr5State {
        Sr5State::with_rng(ChaCha8Rng::from_os_rng())
    }

    /// State with a fixed RNG seed (deterministic tests).
    #[must_use]
    pub fn seeded(seed: u64) -> Sr5State {
        Sr5State::with_rng(ChaCha8Rng::seed_from_u64(seed))
    }

    fn with_rng(rng: ChaCha8Rng) -> Sr5State {
        Sr5State {
            rng,
            logs: HashMap::new(),
            extended: HashMap::new(),
            chargen: HashMap::new(),
        }
    }

    /// Mutable access to the dice RNG (for a `RollCtx`).
    pub fn rng(&mut self) -> &mut ChaCha8Rng {
        &mut self.rng
    }

    /// Record a roll in `who`'s transient ring (capped at [`ROLL_LOG_CAP`]).
    pub fn push_log(&mut self, who: ObjId, entry: RollLogEntry) {
        let ring = self.logs.entry(who).or_default();
        if ring.len() >= ROLL_LOG_CAP {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    /// The most recent `n` roll-log entries for `who` (newest last).
    #[must_use]
    pub fn recent_logs(&self, who: ObjId, n: usize) -> Vec<&RollLogEntry> {
        self.logs
            .get(&who)
            .map(|r| r.iter().rev().take(n).collect::<Vec<_>>())
            .map(|mut v| {
                v.reverse();
                v
            })
            .unwrap_or_default()
    }

    /// The active extended roll for `who`, if any.
    #[must_use]
    pub fn extended(&self, who: ObjId) -> Option<&Sr5Extended> {
        self.extended.get(&who)
    }

    /// Mutable access to `who`'s extended roll, creating one for `threshold`
    /// hits starting at `pool` dice if none is active (or if the active one
    /// targets a different total or has finished).
    pub fn extended_for(&mut self, who: ObjId, threshold: u32, pool: i32) -> &mut Sr5Extended {
        let reset = self.extended.get(&who).is_none_or(|e| {
            e.threshold != threshold || e.is_complete() || e.critical_glitched || e.next_pool < 1
        });
        if reset {
            self.extended.insert(who, Sr5Extended::new(threshold, pool));
        }
        self.extended.get_mut(&who).expect("just inserted")
    }

    /// Clear `who`'s extended roll.
    pub fn clear_extended(&mut self, who: ObjId) {
        self.extended.remove(&who);
    }

    /// The open chargen session for `who`, if any.
    #[must_use]
    pub fn chargen(&self, who: ObjId) -> Option<&ChargenSession> {
        self.chargen.get(&who)
    }

    /// Mutable access to `who`'s chargen session, if open.
    pub fn chargen_mut(&mut self, who: ObjId) -> Option<&mut ChargenSession> {
        self.chargen.get_mut(&who)
    }

    /// Whether `who` has an open chargen session.
    #[must_use]
    pub fn in_chargen(&self, who: ObjId) -> bool {
        self.chargen.contains_key(&who)
    }

    /// Open (or reset) a chargen session for `who`.
    pub fn open_chargen(&mut self, who: ObjId) {
        self.chargen.insert(who, ChargenSession::default());
    }

    /// Close `who`'s chargen session.
    pub fn close_chargen(&mut self, who: ObjId) {
        self.chargen.remove(&who);
    }
}

impl Default for Sr5State {
    fn default() -> Sr5State {
        Sr5State::from_os()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_dice::sr5::{Sr5Opts, Sr5Pool};

    #[test]
    fn roll_log_ring_caps() {
        let mut st = Sr5State::seeded(1);
        let who = ObjId(5);
        for i in 0..(ROLL_LOG_CAP + 5) {
            st.push_log(
                who,
                RollLogEntry {
                    who,
                    expr: format!("roll {i}"),
                    summary: String::new(),
                    threshold: 1,
                    ts: i as i64,
                    private: false,
                    outcome: Sr5Pool::score(&[5], &Sr5Opts::default()),
                },
            );
        }
        let logs = st.recent_logs(who, 100);
        assert_eq!(logs.len(), ROLL_LOG_CAP);
        assert_eq!(logs[0].expr, "roll 5");
    }

    #[test]
    fn seeded_rng_is_deterministic() {
        use rand::RngCore;
        let mut a = Sr5State::seeded(42);
        let mut b = Sr5State::seeded(42);
        assert_eq!(a.rng().next_u32(), b.rng().next_u32());
    }
}
