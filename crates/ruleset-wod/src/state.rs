//! The WoD module's in-RAM working state, living in a typed state slot on the
//! [`World`](mush_core::World).
//!
//! It carries the server dice RNG (seedable for deterministic tests), the
//! **transient** per-character roll-log ring (staff-queryable via
//! `+roll/last`, deliberately *not* persisted), any active extended rolls, open
//! chargen sessions, and the M11 concealment / M13 pending scene state. Sheets
//! themselves persist as object attributes through
//! [`SheetStore`](mush_ruleset::SheetStore).
//!
//! # M17 hot-restart persistence
//!
//! The **scene-invisible subset** — concealment (`concealed`) + its pierced
//! verdicts (`pierced`), accepted-but-unthrown pendings (`pending`), and
//! in-progress extended rolls (`extended`) — is mirrored whole-doc-per-subsystem
//! to the `mod:wod:*` documents on every mutation (the `*_persist_op` builders,
//! handed to [`Ctx::persist`](mush_core::Ctx::persist)) and re-hydrated at boot
//! by [`WodRuleset::on_world_loaded`](crate::WodRuleset), so a mid-scene hot
//! restart is invisible (a cloaked vampire stays hidden). The RNG (re-seeded on
//! boot), the roll-log ring (deliberately transient), and chargen (a login-flow
//! transient, re-derivable from the persisted sheet) are **not** persisted.

use std::collections::{BTreeMap, HashMap, VecDeque};

use mush_core::{ModuleOp, ModuleWrite, ObjId};
use mush_dice::owod20::{ExtendedRoll, Owod20Outcome};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// How many recent rolls to keep per character in the transient ring.
pub const ROLL_LOG_CAP: usize = 20;

/// The WoD module's persistence namespace (`mod:wod:*`) — the same routing name
/// [`WodRuleset`](crate::WodRuleset) registers under. The scene-invisible subset
/// of [`WodState`] (M17) is mirrored here whole-doc-per-subsystem, so a hot
/// restart re-hydrates it before the first re-attached `look` renders. The RNG,
/// the roll-log ring, and open chargen sessions are deliberately NOT persisted.
const NS: &str = "wod";

/// The `conceal` doc key: concealment ratings AND their pierced verdicts (they
/// are always cleared together, so co-locating keeps them consistent).
const KEY_CONCEAL: &str = "conceal";
/// The `pending` doc key: accepted-but-unthrown M13 actions, per actor.
const KEY_PENDING: &str = "pending";
/// The `extended` doc key: in-progress extended rolls, per actor.
const KEY_EXTENDED: &str = "extended";

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

/// A Storyteller-adjudicated action awaiting the player's dice throw (M13).
///
/// Written into [`WodState::pending`] by the ruling completion once the ST has
/// **accepted** a `+cast … proposing …` proposal and it has been re-validated
/// against the live sheet. The next `+roll`/`+roll/silent` the actor makes
/// consumes it exactly once: the roll is ST-authoritative — it runs the stored
/// `pool` at the stored `difficulty` regardless of what the player typed — and
/// then defers the interpretation pass. Expires; persisted across a hot restart
/// (M17) so an accepted-but-unthrown ruling survives and the next `+roll` still
/// consumes it as ST-authoritative.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingAction {
    /// The ST-ruled pool expression (a trait expr resolved against the sheet).
    pub pool: String,
    /// The ST-ruled difficulty (already clamped to `2..=10`).
    pub difficulty: u8,
    /// The Spheres the ST validated for this working.
    pub spheres: Vec<String>,
    /// The Paradox the ST assigned on a failed / vulgar working (clamped).
    pub paradox: i32,
    /// The difficulty an observer rolls to witness the working (M13 task 2).
    pub witness_difficulty: u8,
    /// The sense channel an observer notices the working on (M14).
    pub sense_channel: crate::perception::SenseChannel,
    /// The player's original freeform proposal (carried into interpretation).
    pub freeform: String,
    /// Whether the working was flagged vulgar (`+cast/vulgar`).
    pub vulgar: bool,
    /// Correlation id of the originating ruling (audit / dedupe).
    pub correlation: u64,
    /// Mage working vs generic action — selects the roll phrasing and the
    /// mechanical consequences applied at interpretation.
    pub kind: crate::st::ActionKind,
    /// Unix-seconds expiry: a `+roll` after this treats it as no pending.
    pub expires_at: i64,
}

// ----------------------------------------------------------------------
// M17 persisted docs (the scene-invisible subset, whole-doc-per-subsystem)
// ----------------------------------------------------------------------

/// The `conceal` doc: every concealed subject with its Obfuscate rating and the
/// per-observer pierced verdicts about them. Object ids are the map keys (JSON
/// stringifies them). Rating and verdicts are always mutated and cleared as a
/// unit, so persisting them as one document keeps them consistent on reload.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ConcealDoc {
    #[serde(default)]
    subjects: BTreeMap<u32, ConcealEntry>,
}

/// One concealed subject's persisted state: the rating they cloaked at, and the
/// `(observer → pierced?)` verdicts rolled about them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ConcealEntry {
    rating: i32,
    #[serde(default)]
    verdicts: BTreeMap<u32, bool>,
}

/// A `JsonSet` op for a whole subsystem document (WoD data is plain — it
/// serializes infallibly).
fn json_set(key: &'static str, value: serde_json::Value) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new_static(key),
            value,
        },
    }
}

/// The WoD module state slot. `Clone` (world snapshots clone their state), so
/// every field is cloneable — including the RNG (a snapshot shares the stream
/// position, which is fine for inspection).
#[derive(Clone, Debug)]
pub struct WodState {
    /// The server dice RNG. NOT persisted — re-seeded from the OS on boot
    /// (persisting a stream position is undesirable).
    rng: ChaCha8Rng,
    /// The transient per-character roll-log ring. NOT persisted (deliberately
    /// transient; it re-hydrates empty on a restart).
    logs: HashMap<ObjId, VecDeque<RollLogEntry>>,
    /// In-progress extended rolls. Persisted across a hot restart (M17, the
    /// `extended` doc) so an accumulating roll keeps its successes.
    extended: HashMap<ObjId, ExtendedRoll>,
    /// Open chargen sessions. NOT persisted (a login-flow transient, re-derivable
    /// from the persisted sheet).
    chargen: HashMap<ObjId, ChargenSession>,
    /// M11 concealment (Obfuscate): subject → the Obfuscate rating in force
    /// when they cloaked (the difficulty basis for a piercing roll). A subject
    /// absent here is not concealed and is perceived by everyone (the free
    /// fast-path the `Perceive` hook takes). Persisted across a hot restart
    /// (M17, the `conceal` doc) so a cloaked vampire does not pop into view.
    concealed: HashMap<ObjId, i32>,
    /// M11 pierced-verdict cache: `(observer, subject)` → whether `observer`
    /// perceives the concealed `subject`. Rolled **once** (in the mutable
    /// concealment paths — `+obfuscate` and the presence observer), never per
    /// event; the read-only `Perceive` hook is a pure lookup here. Absent for a
    /// concealed subject means *not yet rolled* and is treated as concealed
    /// (fail-safe: never leak). Persisted WITH `concealed` (the `conceal` doc)
    /// so who-sees-whom does not flip on reload.
    pierced: HashMap<(ObjId, ObjId), bool>,
    /// M13 Storyteller adjudication: actor → the accepted, re-validated action
    /// awaiting their dice throw. At most one per actor; a new proposal replaces
    /// any prior; the actor's next `+roll` consumes it. Persisted across a hot
    /// restart (M17, the `pending` doc); an entry past its `expires_at` is
    /// dropped on reload.
    pending: HashMap<ObjId, PendingAction>,
    /// M13: actor → the correlation id of their most recently PROPOSED ruling
    /// (`+cast … proposing …` / `+action`). A ruling completion may only mutate
    /// `pending` when it carries this correlation, so a stale or out-of-order
    /// ruling (an older proposal finishing after a newer one was proposed, or an
    /// interpret completion arriving after a fresh proposal) cannot clobber the
    /// newer pending. Volatile.
    latest_proposal: HashMap<ObjId, u64>,
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
            pending: HashMap::new(),
            latest_proposal: HashMap::new(),
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
        self.pending.remove(&who);
        self.latest_proposal.remove(&who);
    }

    // ------------------------------------------------------------------
    // M13 Storyteller adjudication: pending actions
    // ------------------------------------------------------------------

    /// Stamp `who`'s accepted, re-validated pending action, replacing any prior
    /// (a new proposal supersedes an old one).
    pub fn set_pending(&mut self, who: ObjId, action: PendingAction) {
        self.pending.insert(who, action);
    }

    /// Peek at `who`'s pending action (tests / re-entry guards); does not
    /// consume or expire it.
    #[must_use]
    pub fn pending(&self, who: ObjId) -> Option<&PendingAction> {
        self.pending.get(&who)
    }

    /// Consume `who`'s pending action if one is present and not past its
    /// `expires_at` (relative to `now`, unix seconds). An expired pending is
    /// dropped and `None` returned — so a stale `+roll` is a normal roll.
    /// Consumes exactly once: the entry is removed either way.
    pub fn take_pending(&mut self, who: ObjId, now: i64) -> Option<PendingAction> {
        let action = self.pending.remove(&who)?;
        if now > action.expires_at {
            return None;
        }
        Some(action)
    }

    /// Drop `who`'s pending action, if any (replace-on-new-proposal, or the
    /// completion's clear-before-decode guard).
    pub fn clear_pending(&mut self, who: ObjId) {
        self.pending.remove(&who);
    }

    /// Record `correlation` as `who`'s most recently proposed ruling. Called by
    /// the `+cast … proposing` / `+action` handlers as they defer the ruling, so
    /// only the ruling completion carrying this id may later mutate the pending.
    pub fn note_proposal(&mut self, who: ObjId, correlation: u64) {
        self.latest_proposal.insert(who, correlation);
    }

    /// The correlation id of `who`'s most recently proposed ruling, if any. A
    /// ruling completion whose correlation differs is stale / out-of-order and
    /// must not touch the pending.
    #[must_use]
    pub fn latest_proposal(&self, who: ObjId) -> Option<u64> {
        self.latest_proposal.get(&who).copied()
    }

    // ------------------------------------------------------------------
    // M17 write-through: whole-doc persist ops per subsystem
    //
    // Each mutation call site re-serializes the affected subsystem's WHOLE
    // document and hands the op to `ctx.persist(...)`, which rides the same
    // atomic per-command `WriteBatch` as the object writes. Small, low-frequency
    // maps: the whole-doc rewrite avoids per-key Del bookkeeping / drift.
    // ------------------------------------------------------------------

    /// A `JsonSet` mirroring the concealment set AND the pierced verdicts (one
    /// document; they are always mutated and cleared together). Emit after every
    /// change to `concealed` / `pierced` — the `+obfuscate` cloak/reveal, the
    /// presence-observer maintenance, and the move-driven verdict clears.
    #[must_use]
    pub fn conceal_persist_op(&self) -> ModuleOp {
        let mut subjects: BTreeMap<u32, ConcealEntry> = BTreeMap::new();
        for (&subject, &rating) in &self.concealed {
            subjects.insert(subject.0, ConcealEntry {
                rating,
                verdicts: BTreeMap::new(),
            });
        }
        // Attach each verdict to its (concealed) subject. Verdicts only ever
        // exist for a concealed subject, so any without a subject entry — there
        // should be none — are simply not carried, matching the RAM invariant.
        for (&(observer, subject), &pierced) in &self.pierced {
            if let Some(entry) = subjects.get_mut(&subject.0) {
                entry.verdicts.insert(observer.0, pierced);
            }
        }
        let doc = ConcealDoc { subjects };
        json_set(KEY_CONCEAL, serde_json::to_value(&doc).expect("conceal doc serializes"))
    }

    /// A `JsonSet` mirroring every actor's [`PendingAction`]. Emit after every
    /// `set_pending` / `clear_pending` / `take_pending`.
    #[must_use]
    pub fn pending_persist_op(&self) -> ModuleOp {
        let doc: BTreeMap<u32, &PendingAction> =
            self.pending.iter().map(|(k, v)| (k.0, v)).collect();
        json_set(KEY_PENDING, serde_json::to_value(&doc).expect("pending doc serializes"))
    }

    /// A `JsonSet` mirroring every actor's in-progress [`ExtendedRoll`]. Emit
    /// after every `extended_for` / `clear_extended`.
    #[must_use]
    pub fn extended_persist_op(&self) -> ModuleOp {
        let doc: BTreeMap<u32, &ExtendedRoll> =
            self.extended.iter().map(|(k, v)| (k.0, v)).collect();
        json_set(KEY_EXTENDED, serde_json::to_value(&doc).expect("extended doc serializes"))
    }

    // ------------------------------------------------------------------
    // M17 rehydrate: a two-phase decode → install so the world (validation) and
    // the state slot (install) borrows never overlap. `decode_*` is pure and
    // validates each referenced object against a liveness predicate; `install_*`
    // writes the decoded maps into the slot's fields ONLY (the register-time
    // RNG, the transient roll-log ring, and chargen are never touched). A decode
    // that dropped an orphan/expired entry flags it so the caller re-persists.
    // ------------------------------------------------------------------

    /// Decode + validate a persisted `conceal` doc against a liveness predicate,
    /// returning the restored `(concealed, pierced)` maps and whether any orphan
    /// was dropped (⇒ the caller re-persists so the store matches RAM). Takes no
    /// `self`, so the caller validates against the world *before* borrowing the
    /// state slot to [`install_conceal`](WodState::install_conceal) — keeping the
    /// world and slot borrows disjoint.
    #[allow(clippy::type_complexity)]
    pub fn decode_conceal(
        value: &serde_json::Value,
        live: impl Fn(ObjId) -> bool,
    ) -> Result<(HashMap<ObjId, i32>, HashMap<(ObjId, ObjId), bool>, bool), String> {
        let doc: ConcealDoc =
            serde_json::from_value(value.clone()).map_err(|e| format!("bad conceal doc: {e}"))?;
        let mut concealed = HashMap::new();
        let mut pierced = HashMap::new();
        let mut dropped = false;
        for (sid, entry) in doc.subjects {
            let subject = ObjId(sid);
            if !live(subject) {
                dropped = true;
                continue;
            }
            concealed.insert(subject, entry.rating.max(0));
            for (oid, verdict) in entry.verdicts {
                let observer = ObjId(oid);
                if live(observer) {
                    pierced.insert((observer, subject), verdict);
                } else {
                    dropped = true;
                }
            }
        }
        Ok((concealed, pierced, dropped))
    }

    /// Decode + validate a persisted `pending` doc, dropping any whose actor is
    /// gone or whose `expires_at` is already past `now`. Pure (see
    /// [`decode_conceal`](WodState::decode_conceal)).
    pub fn decode_pending(
        value: &serde_json::Value,
        now: i64,
        live: impl Fn(ObjId) -> bool,
    ) -> Result<(HashMap<ObjId, PendingAction>, bool), String> {
        let doc: BTreeMap<u32, PendingAction> =
            serde_json::from_value(value.clone()).map_err(|e| format!("bad pending doc: {e}"))?;
        let mut pending = HashMap::new();
        let mut dropped = false;
        for (aid, action) in doc {
            let actor = ObjId(aid);
            if !live(actor) || now > action.expires_at {
                dropped = true;
                continue;
            }
            pending.insert(actor, action);
        }
        Ok((pending, dropped))
    }

    /// Decode + validate a persisted `extended` doc, dropping any whose actor is
    /// gone. Pure (see [`decode_conceal`](WodState::decode_conceal)).
    pub fn decode_extended(
        value: &serde_json::Value,
        live: impl Fn(ObjId) -> bool,
    ) -> Result<(HashMap<ObjId, ExtendedRoll>, bool), String> {
        let doc: BTreeMap<u32, ExtendedRoll> =
            serde_json::from_value(value.clone()).map_err(|e| format!("bad extended doc: {e}"))?;
        let mut extended = HashMap::new();
        let mut dropped = false;
        for (aid, roll) in doc {
            let actor = ObjId(aid);
            if live(actor) {
                extended.insert(actor, roll);
            } else {
                dropped = true;
            }
        }
        Ok((extended, dropped))
    }

    /// Install a decoded `conceal` doc into the slot (fields only — the
    /// register-time RNG, the roll-log ring, and chargen are never touched).
    pub fn install_conceal(
        &mut self,
        concealed: HashMap<ObjId, i32>,
        pierced: HashMap<(ObjId, ObjId), bool>,
    ) {
        self.concealed = concealed;
        self.pierced = pierced;
    }

    /// Install a decoded `pending` doc into the slot (fields only).
    pub fn install_pending(&mut self, pending: HashMap<ObjId, PendingAction>) {
        self.pending = pending;
    }

    /// Install a decoded `extended` doc into the slot (fields only).
    pub fn install_extended(&mut self, extended: HashMap<ObjId, ExtendedRoll>) {
        self.extended = extended;
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
