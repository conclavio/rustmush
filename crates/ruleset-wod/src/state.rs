//! The WoD module's in-RAM working state, living in a typed state slot on the
//! [`World`](mush_core::World).
//!
//! It carries the server dice RNG (seedable for deterministic tests), the
//! **transient** per-character roll-log ring (staff-queryable via
//! `+roll/last`, deliberately *not* persisted), any active extended rolls, and
//! open chargen sessions. Sheets themselves persist as object attributes
//! through [`SheetStore`](mush_ruleset::SheetStore); only this volatile
//! session/roll bookkeeping lives here.

use std::collections::{HashMap, VecDeque};

use mush_core::ObjId;
use mush_dice::owod20::{ExtendedRoll, Owod20Outcome};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;
use smol_str::SmolStr;

/// How many recent rolls to keep per character in the transient ring.
pub const ROLL_LOG_CAP: usize = 20;

/// One entry in a character's roll-log ring / the `wod.roll` panel feed.
#[derive(Clone, Debug, Serialize)]
pub struct RollLogEntry {
    /// The roller.
    pub who: ObjId,
    /// The pool expression as typed ("strength+brawl").
    pub expr: String,
    /// The one-line rendered summary.
    pub summary: String,
    /// Difficulty applied.
    pub difficulty: u8,
    /// Wall-clock timestamp (unix seconds).
    pub ts: i64,
    /// Whether the roll was private.
    pub private: bool,
    /// The full dice outcome (per-die provenance for the panel/audit).
    pub outcome: Owod20Outcome,
}

/// An open chargen session for one character (a transient; the sheet itself is
/// the persisted work-in-progress). Records the priority pools the player
/// assigned, so budget assessment knows each category's total.
#[derive(Clone, Debug, Default)]
pub struct ChargenSession {
    /// Attribute priority pools: `[(category, dots), …]` (physical/social/mental).
    pub attr_pools: Vec<(SmolStr, i32)>,
    /// Ability priority pools: `[(group, dots), …]` (talents/skills/knowledges).
    pub abil_pools: Vec<(SmolStr, i32)>,
}

impl ChargenSession {
    /// Whether both priority sets have been chosen.
    #[must_use]
    pub fn priorities_set(&self) -> bool {
        !self.attr_pools.is_empty() && !self.abil_pools.is_empty()
    }
}

/// The WoD module state slot. `Clone` (world snapshots clone their state), so
/// every field is cloneable — including the RNG (a snapshot shares the stream
/// position, which is fine for inspection).
#[derive(Clone, Debug)]
pub struct WodState {
    rng: ChaCha8Rng,
    logs: HashMap<ObjId, VecDeque<RollLogEntry>>,
    extended: HashMap<ObjId, ExtendedRoll>,
    chargen: HashMap<ObjId, ChargenSession>,
    /// M11 concealment (Obfuscate): subject → the Obfuscate rating in force
    /// when they cloaked (the difficulty basis for a piercing roll). A subject
    /// absent here is not concealed and is perceived by everyone (the free
    /// fast-path the `Perceive` hook takes). Volatile — never persisted.
    concealed: HashMap<ObjId, i32>,
    /// M11 pierced-verdict cache: `(observer, subject)` → whether `observer`
    /// perceives the concealed `subject`. Rolled **once** (in the mutable
    /// concealment paths — `+obfuscate` and the presence observer), never per
    /// event; the read-only `Perceive` hook is a pure lookup here. Absent for a
    /// concealed subject means *not yet rolled* and is treated as concealed
    /// (fail-safe: never leak). Volatile.
    pierced: HashMap<(ObjId, ObjId), bool>,
}

impl WodState {
    /// State with an OS-seeded RNG (production).
    #[must_use]
    pub fn from_os() -> WodState {
        WodState::with_rng(ChaCha8Rng::from_os_rng())
    }

    /// State with a fixed-seed RNG (deterministic tests).
    #[must_use]
    pub fn seeded(seed: u64) -> WodState {
        WodState::with_rng(ChaCha8Rng::seed_from_u64(seed))
    }

    fn with_rng(rng: ChaCha8Rng) -> WodState {
        WodState {
            rng,
            logs: HashMap::new(),
            extended: HashMap::new(),
            chargen: HashMap::new(),
            concealed: HashMap::new(),
            pierced: HashMap::new(),
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
    pub fn extended(&self, who: ObjId) -> Option<&ExtendedRoll> {
        self.extended.get(&who)
    }

    /// Mutable access to `who`'s extended roll, creating one for `target`
    /// successes if none is active (or if the active one targets a different
    /// total).
    pub fn extended_for(&mut self, who: ObjId, target: i32) -> &mut ExtendedRoll {
        let reset = self
            .extended
            .get(&who)
            .is_none_or(|e| e.target != target || e.is_complete() || e.botched);
        if reset {
            self.extended.insert(who, ExtendedRoll::new(target));
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

    // ------------------------------------------------------------------
    // M11 concealment (Obfuscate) + pierced-verdict cache
    // ------------------------------------------------------------------

    /// Mark `subject` concealed at Obfuscate `rating` (the difficulty basis for
    /// a piercing roll). Idempotent: re-cloaking overwrites the rating. Does
    /// **not** clear stale verdicts — the caller re-rolls present observers.
    pub fn conceal(&mut self, subject: ObjId, rating: i32) {
        self.concealed.insert(subject, rating.max(0));
    }

    /// Lift `subject`'s concealment and drop every pierced verdict about them,
    /// so a later re-cloak starts from a clean slate (no stale verdict). Returns
    /// whether they were concealed.
    pub fn reveal(&mut self, subject: ObjId) -> bool {
        let was = self.concealed.remove(&subject).is_some();
        self.clear_perceptions_for_subject(subject);
        was
    }

    /// Whether `subject` is currently concealed.
    #[must_use]
    pub fn is_concealed(&self, subject: ObjId) -> bool {
        self.concealed.contains_key(&subject)
    }

    /// Whether anyone at all is concealed — the observer's free fast-path (no
    /// concealment ⇒ no cache to maintain, no RNG touched).
    #[must_use]
    pub fn has_concealment(&self) -> bool {
        !self.concealed.is_empty()
    }

    /// The Obfuscate rating `subject` cloaked at, if concealed.
    #[must_use]
    pub fn concealed_rating(&self, subject: ObjId) -> Option<i32> {
        self.concealed.get(&subject).copied()
    }

    /// Whether `observer` perceives concealed `subject`: `Some(true)` pierced,
    /// `Some(false)` failed, `None` not yet rolled.
    #[must_use]
    pub fn perception_verdict(&self, observer: ObjId, subject: ObjId) -> Option<bool> {
        self.pierced.get(&(observer, subject)).copied()
    }

    /// Cache one `(observer, subject)` verdict.
    pub fn set_perception(&mut self, observer: ObjId, subject: ObjId, pierced: bool) {
        self.pierced.insert((observer, subject), pierced);
    }

    /// Drop every verdict about `subject` (they de-cloaked or moved rooms).
    pub fn clear_perceptions_for_subject(&mut self, subject: ObjId) {
        self.pierced.retain(|(_, s), _| *s != subject);
    }

    /// Drop every verdict held by `observer` (they moved / are being
    /// re-evaluated in a new room).
    pub fn clear_perceptions_by_observer(&mut self, observer: ObjId) {
        self.pierced.retain(|(o, _), _| *o != observer);
    }

    /// Forget `who` entirely — as a concealed subject and as either side of any
    /// verdict (on disconnect / destruction).
    pub fn forget_actor(&mut self, who: ObjId) {
        self.concealed.remove(&who);
        self.pierced.retain(|(o, s), _| *o != who && *s != who);
    }
}

impl Default for WodState {
    fn default() -> WodState {
        WodState::from_os()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_log_ring_caps() {
        let mut st = WodState::seeded(1);
        let who = ObjId(5);
        for i in 0..(ROLL_LOG_CAP + 5) {
            st.push_log(
                who,
                RollLogEntry {
                    who,
                    expr: format!("roll {i}"),
                    summary: String::new(),
                    difficulty: 6,
                    ts: i as i64,
                    private: false,
                    outcome: mush_dice::owod20::Owod20Pool::score(&[6], &Default::default()),
                },
            );
        }
        let logs = st.recent_logs(who, 100);
        assert_eq!(logs.len(), ROLL_LOG_CAP);
        // Oldest dropped: first retained is entry #5.
        assert_eq!(logs[0].expr, "roll 5");
        assert_eq!(
            logs.last().unwrap().expr,
            format!("roll {}", ROLL_LOG_CAP + 4)
        );
    }

    #[test]
    fn seeded_rng_is_deterministic() {
        use rand::RngCore;
        let mut a = WodState::seeded(42);
        let mut b = WodState::seeded(42);
        assert_eq!(a.rng().next_u32(), b.rng().next_u32());
    }
}
