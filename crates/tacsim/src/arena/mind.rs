//! **The mind seam** (17b cut M1a.5; doc 17 §3 "The view", §4 "Intents", §6
//! "When a mind thinks", §7 "The runner"; D-UM-1, D-UM-2, D-UM-4, D-UM-8,
//! D-UM-9, D-UM-13, D-UM-20, D-UM-21, D-UM-23).
//!
//! A [`Controller::Mind`] hull is flown by a **unit mind**: something that is
//! handed the hull's own view, its own memory and the map, and answers a short
//! list of [`IntentKind`]s and a new memory. This file is everything the arena
//! does around that call, and nothing of what answers it:
//!
//! * **The state** a mind carries between thinks ([`MindState`], one row per
//!   hull on [`Arena::minds`]): its think counter, the think in flight, its
//!   memory's bytes, its notice buffer, its range memory, the standing order a
//!   squad gave it, its strikes, its drop (`dropped`) and its seal's digest.
//!   Hashed (`crate::arena::hash`, item 3c) and carried by a snapshot, because
//!   every one of them decides a later intent (the pending think's answer,
//!   `dropped` and `seal_digest` carried since 17b cut M3.2 and folded since
//!   17b cut M3.6, `hash.rs`' item 3c).
//! * **The view** ([`Arena::mind_view`], [`MindView`]): the hull's keyframe and
//!   nothing finer (D-UM-4). It is built by the one builder a pilot's frame is
//!   built by (`Arena::frame_for`) and projected through the one row formatter
//!   a pilot's `CONTACTS` reads ([`crate::arena::view::frame_contacts_held`]),
//!   so a mind can never read what the keyframe does not carry.
//! * **The think** ([`DueKind::MindThink`]): a due event on the stagger grid,
//!   every [`STOCK_THINK_MS`] of arena time, resolved in the substep its
//!   timestamp falls in **after that millisecond's other due items** (D-UM-20;
//!   `Arena::substep`'s due loop holds the mind kinds back), against the state
//!   then. **Since 17b cut M4.1 the think is COLLECTED at its millisecond and
//!   ANSWERED at the head of the next substep** (doc 17 §6 "Execution policy",
//!   D-UM-22; 17b §3's M4 row, finding (1)): `Arena::collect_think` freezes
//!   the job ([`CollectedThink`] — the view, the memory, the card, the row's
//!   profile, the runner), writes the think's FACTS on the row (`pending`'s
//!   `think_seq`, `t_ms` and epochs with an empty answer, the hull named in
//!   the scratch set [`Arena::awaiting`]), advances `think_seq` and schedules
//!   the landing and the next think; the runner runs anywhere between that
//!   millisecond and the write — on the sim task's pool, or inline — and
//!   `Arena::settle_collected` writes its answer at ONE sim-time moment
//!   both paths share: before anything of the next substep, which is before
//!   the next pulse's phases on the pooled path ([`Arena::deliver_answers`])
//!   and the next substep's head inside `Arena::step` on the inline path
//!   ([`Arena::settle_inline`]). A think of more than [`MIND_INTENTS_MAX`]
//!   intents is a fault and lands nothing (D-UM-9); a fault's consequences
//!   and its lines follow the answer to that write, the lines carrying the
//!   think's OWN substep's end hash ([`CollectedThink::substep_hash`]) at the
//!   think's own `t_ms`, so the tape is no longer `t_ms`-monotonic across
//!   them (declared, doc 17 §24).
//! * **The landing** ([`DueKind::MindLand`]): one substep later, the think's
//!   intents go through the pilot's own door (`Arena::apply_machine_intents`,
//!   D-UM-12) at the epochs the think recorded (D-UM-13) under
//!   [`IntentSource::Mind`], so a seat that changed hands between the two
//!   refuses the stale thought `Epoch`, and each refusal is appended to the
//!   mind's own notice buffer (D-UM-8: *a refusal is never silent*).
//! * **The answer in flight** (D-UM-23 as amended 2026-10-04; 17b cut M3.2):
//!   between the think and its landing the think's ANSWER — the epochs it
//!   recorded, its intents and its memory — rides the row itself
//!   ([`MindState::pending`], a [`PendingThink`]), so a snapshot taken between
//!   the two carries it and the resumed landing lands exactly what the think
//!   answered. It is never recomputed: the view the think read is gone (its
//!   notices drained, its ranges settled), and a second think on the resumed
//!   view is another answer. The scratch table `Arena::mind_landings` that
//!   held the answer until 17b cut M3.2 — and lost it across a resume — is
//!   retired. **17b cut M4.1**: between the think's millisecond and its
//!   write the row carries the think's facts and NO answer, the hull stands
//!   in [`Arena::awaiting`], and `Snapshot::encode` refuses the arena by
//!   name — the sim task awaits every outstanding batch before it encodes
//!   (doc 17 §6 "Snapshot and resume"), so a snapshot always carries a
//!   written answer; the fold (`hash.rs`' item 3c) is unchanged, so the
//!   think's substep hash folds the facts and an empty answer.
//!
//! # What runs a think
//!
//! [`MindRunner`] is a trait and this library implements it twice: with
//! [`NoMind`], which thinks nothing, and — since 17b cut M1a.6 — with the
//! native stock mind ([`crate::arena::stock_mind::NativeStockMind`], T1 of doc
//! 17 §17), which [`default_mind_runner`] answers. It is a pure function of the
//! job, compiled in, and reads nothing the job does not hand it. The scripted
//! runners live outside the library (doc 17 §7, D-UM-1): the interpreter the
//! sim task drives — the Rhai engine of doc 17 §2 — which this file must never
//! name in code: the contract test
//! `crates/mush-contract-tests/tests/arena_library.rs` refuses the lowercase
//! crate name anywhere under `crates/tacsim/src/arena/`. The arena calls the
//! runner and reads its answer; it never times it (there is no wall clock
//! here: the twin lane times a view and a think from the test), and it never
//! decodes the memory it hands back (D-UM-30: canonical bytes, opaque).
//!
//! # Production
//!
//! No lobby, check-out or control sets [`Controller::Mind`] in this cut, so no
//! production match pops a mind kind, folds a mind row or calls a runner: the
//! two pins of 17b cut M1a.1 (`tests/reflex.rs`,
//! `m1a1_the_moved_motor_folds_the_same_bytes`; `tests/motor.rs`,
//! `m1a1_the_shipped_duel_hashes_as_before`) are re-asserted unchanged by
//! `tests/minds.rs`. 17b cut M4 is the flip.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::card::{CardChange, PRESETS, preset};
use crate::arena::conditions::{
    ASPECTS_ANY, ConditionTrigger, FireCondition, SlotTarget, condition_is_lawful,
};
use crate::arena::intents::{
    Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, apply_intent, resolve_emcon,
    resolve_startup,
};
use crate::arena::ledger::EventKind;
use crate::arena::motor::MotorCounters;
use crate::arena::perception::{
    Aspect, FrameCondition, FrameNotice, FrameOrder, MindObjective, Receiver,
};
use crate::arena::state::{
    Arena, ArenaDefinition, Channel, Classification, ContactId, Controller, Designator, DueKind,
    Power, PartyId,
};
use crate::arena::step::{FrameHeat, FrameSection};
use crate::arena::view::ContactRow;
use crate::arena::weapons::{FiredBy, FrameWeapon};
use crate::config::{ArenaConfig, ReflexConfig};

/// **How often a stock mind thinks**, arena milliseconds (17b cut M1a.5; doc
/// 17 §9, D-UM-35's class 0/I cadence). **The Class I row of
/// `ClassTable::STOCK`; production reads the profile, the tests' arithmetic
/// keeps the constant** (17b cut M3.1): the think arm and `adopt_mind` read
/// [`ThinkJob::profile`]'s `think_ms` ([`Arena::mind_profile`]) at the moment
/// they schedule.
pub const STOCK_THINK_MS: i64 = 1_000;

/// **The class the stock mind is rated at** (17b cut M3.1; doc 17 §9,
/// D-UM-35): Standard Doctrine's, Class I — the one mind there is. **17b cut
/// M3.3's seal records** `min(rated, core, cap)` as [`MindSeal::class`] under
/// `crate::match_::SealTerms::rated`, this constant on every production and
/// fixture seal; **since 17b cut M4.1 the think side reads the seal**
/// ([`Arena::mind_class`] off `UnitState::seal.class` against the core's
/// class now — the arena-wide `rated_class` and its test lever are gone). So
/// on every shipped chassis (cores I to III) a stock mind thinks at Class
/// I's row, today's figures. `tests/content.rs` pins it equal to the Standard
/// Doctrine manifest's own `rated_class`.
pub const STOCK_RATED_CLASS: crate::arena::core::CoreClass = crate::arena::core::CoreClass::I;

/// **How many notices a mind's buffer holds** (D-UM-8): the newest
/// `MIND_NOTICES`, the oldest dropped first.
pub const MIND_NOTICES: usize = 16;

/// **The most intents one think may answer** (D-UM-9). A think that answers
/// more is a fault: it lands **nothing** and keeps its old memory — never a
/// truncation, which would land a prefix of a plan its author never wrote.
pub const MIND_INTENTS_MAX: usize = 16;

/// **How many faulted thinks running drop a hull to doctrine** (17b cut M2.2;
/// D-UM-31, D-UM-32): the third consecutive fault drops it for the rest of
/// the sortie. `MindState.strikes` stops here ([`MindState::dropped`]).
pub const MIND_STRIKES_TO_DROP: u8 = 3;

/// **The cause word of D-UM-9's overflow** (17b cut M3.4; doc 17 §8): a think
/// answering more than [`MIND_INTENTS_MAX`] intents with no fault of its own
/// is journalled `mind_fault` with this word — the word the scripted
/// runner's own overflow flag answers (`intents`).
pub const OVERFLOW_CAUSE: &str = "intents";

/// **How many contacts a view carries** (doc 17 §3; D-UM-35's class I
/// `contact_rows`): twelve — the hull's lock and its order's contact
/// seated, then every contact not known friendly, then the friendly ones,
/// each group nearest-first by the held range then by [`ContactId`] — read
/// nearest-first (17b cut M4.5c; doc 17 §3 as amended 2026-10-09 by the
/// owner's two rulings; until then the nearest twelve). **The Class I row of
/// `ClassTable::STOCK`; production reads the profile, the tests' arithmetic
/// keeps the constant** (17b cut M3.1: [`Arena::mind_view`] truncates at the
/// mind's own `rows`).
pub const STOCK_CONTACT_ROWS: usize = 12;

/// **The think in flight and its answer** (D-UM-23 as amended 2026-10-04):
/// which think, the lattice millisecond it was taken at, and — since 17b cut
/// M3.2 — what it answered: the epochs of the hull's three channels at the
/// think, the intents in emission order and the memory the runner handed
/// back. `Some` from a clean think to its landing, which consumes it.
///
/// **Carried, never recomputed**: a snapshot between the think and its
/// landing writes the answer, and the resumed landing lands it at the
/// recorded epochs, so a resume between a think at `N` and its landing at
/// `N + 1` reproduces the hash sequence. **Folded whole since 17b cut
/// M3.6** (`crate::arena::hash`'s item 3c): the presence, `think_seq`, `t_ms`,
/// then the three epochs, the intents' count and each intent's canonical
/// bytes, the memory's length and digest — 17b cuts M3.2 to M3.5 folded the
/// first three alone, M2's bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingThink {
    /// The think's sequence number.
    pub think_seq: u64,
    /// The arena millisecond it was taken at — its lattice millisecond.
    pub t_ms: i64,
    /// **Each channel's epoch at the think**, in [`Channel::ALL`]'s order
    /// (helm, sensors, fire) — what the landing's intents are stamped with,
    /// so a seat that changed hands in between refuses them `Epoch`
    /// (D-UM-13).
    pub epochs: [u64; 3],
    /// Immutable program identity and fixed sample schedule from collection.
    pub profile_token: crate::arena::profile_runtime::ProfileToken,
    pub lands_ms: i64,
    pub period_ms: i64,
    pub profile_class: crate::arena::core::CoreClass,
    /// Complete metered sample, distinct from station intents.
    pub rule_sample: Option<crate::arena::profile_rules::RuleSample>,
    /// **The intents**, in the order the runner emitted them (at most
    /// [`MIND_INTENTS_MAX`]: a longer answer is a fault and is never
    /// pending).
    pub intents: Vec<IntentKind>,
    /// **The memory the runner handed back** (D-UM-30): canonical bytes,
    /// written to [`MindState::memory`] at the landing.
    #[serde(deserialize_with = "crate::arena::memory_bound::deserialize")]
    pub memory: Vec<u8>,
}

impl PendingThink {
    /// **The epoch `channel` was at when the think was taken** — the stamp
    /// the landing authorises the channel's intents against.
    #[must_use]
    pub fn epoch(&self, channel: Channel) -> u64 {
        Channel::ALL
            .iter()
            .position(|on| *on == channel)
            .and_then(|at| self.epochs.get(at))
            .copied()
            .unwrap_or(0)
    }
}

/// **Why a hull was dropped to the ROM** (17b cut M3.2; doc 17 §8, §13;
/// D-UM-31, D-UM-32): three faulted thinks running, a core the seal recorded
/// that is gone, or a mind this engine cannot run. Written to
/// [`MindState::dropped`] once a sortie and never cleared by a think — nor by
/// a possess and a release, which keep the row (17b cut M3.5:
/// [`MindState::dropped`]) — and journalled as
/// one [`EventKind::MindDropped`] line carrying [`DropCause::as_str`] (17b cut
/// M3.4):
///
/// * [`DropCause::Strikes`] by the think arm (`Arena::mind_think`) the think
///   the strikes reach [`MIND_STRIKES_TO_DROP`];
/// * [`DropCause::Core`] by the landing arm (`Arena::mind_land`), or the think
///   arm when no landing is pending, on a hull whose seal recorded a core
///   (`SealRef::core_class` above Class 0) that is gone;
/// * [`DropCause::Abi`] by NOTHING in the arena: a resume across a changed
///   `MIND_ABI` is refused by name before the arena exists
///   (`crate::match_::resume`'s `ResumeRefusal::MindAbi`, 17b cut M3.3). It is
///   carried, and the world's sentence words it
///   (`crate::arena_bridge::dropped_line`), for the region map that will run a
///   sortie across an engine change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropCause {
    /// Three faulted thinks running (D-UM-31).
    Strikes,
    /// The core the seal recorded is destroyed (D-UM-34).
    Core,
    /// The mind's ABI is not this engine's (D-UM-46). Written by nothing in
    /// the arena (the enum's head).
    Abi,
}

impl DropCause {
    /// The word: `strikes`, `core`, `abi`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DropCause::Strikes => "strikes",
            DropCause::Core => "core",
            DropCause::Abi => "abi",
        }
    }

    /// **The cause a `mind_dropped` line's word names** (17b cut M3.4): the
    /// inverse of [`DropCause::as_str`], `None` for any other word — what the
    /// world reads the tape's line back with (`crate::arena_bridge`).
    #[must_use]
    pub fn parse(word: &str) -> Option<DropCause> {
        [DropCause::Strikes, DropCause::Core, DropCause::Abi]
            .into_iter()
            .find(|cause| cause.as_str() == word)
    }
}

/// **A sortie's id** (17b cut M3.2; doc 17 §13, D-UM-45; doc 23 §7, D-23-10):
/// one hull's one seal epoch, deterministic — no clock read at the mint, no
/// counter. **Two kinds** (doc 23 cut S.1, the lead's L1): a match is a
/// simulator concept, not an IC one (D-23-10), so an IC epoch never shares the
/// arena's packing.
///
/// - [`SortieId::Packed`] — **the arena's**: one hull's one match, minted at
///   the seal as [`SortieId::in_match`], unique per (unit, match) by
///   construction. Serialised exactly as it always was, the bare number, so
///   every pinned tape, record and snapshot is byte-identical. The IC claim
///   book's transition ordinal (`crate::minds::ic_sortie`) rides this kind
///   until the cut that flies a mind IC (doc 23 cut S.4) makes the book carry
///   its host's id.
/// - [`SortieId::Ic`] — **an IC epoch** (doc 23 §6): the hull and the IC
///   instant its leaving began (a surface excursion's
///   `crate::surface::ExcursionId::launch_ic`; the voyage's undock in S.2) —
///   unique per hull by construction. Serialised as the tagged object
///   `{"ic":{"hull":<n>,"at_ic":<t>}}`.
///
/// The decode accepts either form. `Ord` puts every arena id before every IC
/// one; within a kind, the number, then the hull and the instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SortieId {
    /// The arena's packed `(match, unit)` number ([`SortieId::in_match`]).
    Packed(u64),
    /// An IC epoch: the hull, and the IC instant its leaving began.
    Ic {
        /// The hull whose epoch it is.
        hull: ObjId,
        /// The IC instant (IC seconds) the hull's leaving began.
        at_ic: i64,
    },
}

impl Default for SortieId {
    /// [`SortieId::none`].
    fn default() -> SortieId {
        SortieId::none()
    }
}

impl SortieId {
    /// **The sortie of `unit` in match `match_id`**: `(match_id << 32) |
    /// unit` — the match in the high word, the object number in the low.
    #[must_use]
    pub fn in_match(match_id: u64, unit: ObjId) -> SortieId {
        SortieId::Packed((match_id << 32) | u64::from(unit.0))
    }

    /// **The IC epoch of `hull` that began leaving at `at_ic`** (doc 23 §7,
    /// cut S.1's L1): the surface excursion mints it at the launch's end from
    /// `(tv, launch_ic)`.
    #[must_use]
    pub const fn ic(hull: ObjId, at_ic: i64) -> SortieId {
        SortieId::Ic { hull, at_ic }
    }

    /// **No sortie** — the arena kind at `0`, what [`SealRef::none`] carries
    /// until 17b cut M3.3 lays seals down.
    #[must_use]
    pub const fn none() -> SortieId {
        SortieId::Packed(0)
    }

    /// Whether this is an IC epoch's id ([`SortieId::Ic`]).
    #[must_use]
    pub const fn is_ic(self) -> bool {
        matches!(self, SortieId::Ic { .. })
    }

    /// **The arena's packed number**, `None` for an IC epoch — what every
    /// reader that once read the bare `.0` reads now (the arena's report
    /// lines; the IC claim book's transition order until S.4).
    #[must_use]
    pub const fn as_match_packed(self) -> Option<u64> {
        match self {
            SortieId::Packed(n) => Some(n),
            SortieId::Ic { .. } => None,
        }
    }
}

impl std::fmt::Display for SortieId {
    /// The arena's kind as its bare number (the report lines' `sortie=` word, unchanged); an IC
    /// epoch as `<hull>@<at_ic>`, `crate::surface::ExcursionId`'s shape.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortieId::Packed(n) => write!(f, "{n}"),
            SortieId::Ic { hull, at_ic } => write!(f, "{}@{at_ic}", hull.0),
        }
    }
}

/// The IC kind's wire: `{"ic":{"hull":<n>,"at_ic":<t>}}`, strict.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IcSortieWire {
    ic: IcEpochWire,
}

/// The IC kind's inner object.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IcEpochWire {
    hull: ObjId,
    at_ic: i64,
}

impl Serialize for SortieId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            SortieId::Packed(n) => serializer.serialize_u64(n),
            SortieId::Ic { hull, at_ic } => IcSortieWire {
                ic: IcEpochWire { hull, at_ic },
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for SortieId {
    /// Either form, through a JSON value: every store the id lives in is JSON, and the value
    /// route keeps the bare number exact under `serde_json/arbitrary_precision` too (the bot
    /// lab's build), where a buffered untagged decode would not.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<SortieId, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        if let Some(n) = value.as_u64() {
            return Ok(SortieId::Packed(n));
        }
        if value.is_object() {
            let wire: IcSortieWire = serde_json::from_value(value).map_err(D::Error::custom)?;
            return Ok(SortieId::Ic {
                hull: wire.ic.hull,
                at_ic: wire.ic.at_ic,
            });
        }
        Err(D::Error::custom(
            "a sortie id is the arena's number or {\"ic\":{\"hull\":..,\"at_ic\":..}}",
        ))
    }
}

/// **What flies a sealed hull** (17b cut M3.2; doc 17 §13, D-UM-45): the
/// reflex controller, the native stock mind, or a scripted runner. Its words
/// are `reflex`, `native` and `scripted` — never the scripted engine's own
/// name, which the arena library may not contain
/// (`crates/mush-contract-tests/tests/arena_library.rs`).
///
/// **[`SealEngine::Reflex`] is decode-only** since 17b cut M4.3 (no seal
/// computes it) and outlives the reflex controller 17b cut M4.6 deleted:
/// every live record an M3.3–M4.2 build wrote for a running match carries it
/// (`crate::match_::MatchRecord::minds`), and without the variant such a record
/// would fail to decode before its format-8 snapshot is refused by name. Two
/// in-memory defaults still write it, neither hashed nor read to decide
/// anything (the conventions' D8, recorded): [`SealRef::none`] and
/// [`MindSeal::placeholder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealEngine {
    /// The reflex controller — a fill, or a pilot's hull a release handed to
    /// it, in an M3.3–M4.2 seal. Decode-only (see the enum's doc): the
    /// controller is gone since 17b cut M4.6.
    Reflex,
    /// The native stock mind (T1 of doc 17 §17).
    Native,
    /// A scripted runner over the sealed source.
    Scripted,
}

impl SealEngine {
    /// The word: `reflex`, `native`, `scripted`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SealEngine::Reflex => "reflex",
            SealEngine::Native => "native",
            SealEngine::Scripted => "scripted",
        }
    }
}

/// **The seal a hull was checked out under, as the arena keeps it** (17b cut
/// M3.2; doc 17 §13, D-UM-45, D-UM-46): the sortie, the seal's digest, the
/// core class the seal recorded, the class the mind thinks at, the engine,
/// whether the core is governed, whether the sortie is closed, (17b cut
/// M4.6) whether the mind sealed is Standard Doctrine and (17b cut M5.2) the
/// profiles it was sealed with.
///
/// On [`crate::arena::state::UnitState::seal`]: laid down empty
/// ([`SealRef::none`]) in 17b cut M3.2; **since 17b cut M3.3** [`Arena::new`]
/// writes it from the custody's seal at check-out ([`MindSeal::seal_ref`]). Carried by the snapshot (the unit's
/// derive) and **NOT folded into `state_hash`**: a per-match constant, as
/// [`Arena::odds_floor_permille`] is, whose `closed` moves only with a ledger
/// line (17b cut M3.6's `sortie_closed`), which the hash folds through the
/// ledger's sequence.
pub use super::mind_seal::{Admission, MindSeal, SealRef};

/// **The word a sortie closes with at the match's end** (17b cut M3.6; doc 17
/// §12, §13): every sortie still open when the match ends, at any of its six
/// end sites.
pub const SORTIE_MATCH_END: &str = "match_end";

/// **The word a sortie closes with at its hull's destruction** (17b cut M3.6):
/// `Arena::destroy_unit` closes the wreck's sortie then, not at the end.
pub const SORTIE_DESTROYED: &str = "destroyed";

impl Arena {
    /// **Close one hull's sortie** (17b cut M3.6; doc 17 §13): the
    /// [`EventKind::SortieClosed`] line for `unit` with `word`, at `t_ms`, and
    /// [`SealRef::closed`] set — or `None` when the hull is not on the field or
    /// its sortie is already closed, so a sortie closes once (a resumed match
    /// carries `closed` and does not re-close). The line is the caller's to
    /// journal, under the hash of the moment it belongs to.
    pub(crate) fn close_sortie(
        &mut self,
        unit: ObjId,
        t_ms: i64,
        word: &'static str,
    ) -> Option<crate::arena::perception::Notice> {
        let state = self.unit_mut(unit)?;
        if state.seal.closed {
            return None;
        }
        state.seal.closed = true;
        Some((
            t_ms,
            EventKind::SortieClosed {
                unit,
                sortie: state.seal.sortie,
                word: SmolStr::new_static(word),
            },
            Vec::new(),
        ))
    }

    /// **Close every sortie still open** (17b cut M3.6; doc 17 §12, §13;
    /// D-UM-45, D-UM-46): one [`EventKind::SortieClosed`] per unit whose
    /// [`SealRef::closed`] is false, in unit-id order, at `t_ms`, each sortie
    /// marked closed.
    ///
    /// **Called at every one of the match's six end sites, immediately BEFORE
    /// the end's own line, at the end's millisecond and under the end's hash**
    /// with [`SORTIE_MATCH_END`] — the three in-substep ends (a hold won, an
    /// elimination or a `Spent`, the deadline: the closes pushed into the
    /// substep's notices ahead of `match_ended`), the pulse's backstop
    /// (elimination or `Spent`, and the duration), the egress overflow's
    /// `void`, and `Arena::stopped` (the world's `Abort`, `Timeout`, `Void`,
    /// `Concede`, `Unattended`) — so the end stays the tape's LAST line (16b
    /// cut W.2). A `void` closes too. Two ends close nothing, declared: the
    /// ledger-full end (the ledger is what ran out; it journals no end line
    /// either), and a match voided at boot
    /// (`crate::match_::resume::void_at_boot`, which writes no tape line). A
    /// wreck's sortie closed at its destruction ([`SORTIE_DESTROYED`]) is not
    /// closed again.
    pub fn close_open_sorties(
        &mut self,
        t_ms: i64,
        word: &'static str,
    ) -> Vec<crate::arena::perception::Notice> {
        let open: Vec<ObjId> = self
            .units
            .iter()
            .filter(|state| !state.seal.closed)
            .map(|state| state.id)
            .collect();
        open.into_iter()
            .filter_map(|unit| self.close_sortie(unit, t_ms, word))
            .collect()
    }
}

/// **What one unit mind holds between thinks** (17b cut M1a.5; doc 17 §3, §6,
/// §8), one row per [`Controller::Mind`] hull on [`Arena::minds`] — and,
/// **since 17b cut M3.5** (doc 17 §12 "Release and possess"; D-UM-2, D-UM-30),
/// one DORMANT row per hull a pilot possessed from its mind: a possess keeps
/// the row (`Arena::suspend_mind`), nothing thinks on it, stages a notice
/// into it or reads its order while the hull is piloted, and a release
/// adopts it again as it stood. The hash folds a dormant row as any row
/// (no production hull carried one before 17b cut M4: the fill was the
/// reflex's, and a piloted hull has no row at check-out).
///
/// Every field that decides a later intent is hashed (`crate::arena::hash`,
/// item 3c) and every field but the cache is carried by a snapshot — the
/// argument the reflex machine's own row was folded on (16b cut N.1). Two
/// fields are NOT hashed: [`MindState::doctrine_state`], a cache over the
/// memory bytes the hash folds (17b cut M4.3), and
/// [`MindState::order_issuer`], a record field (the owner's STOP C ruling,
/// 17b cut M4.6) that decides nothing after the order lands. The pending
/// think's answer ([`PendingThink`]'s `epochs`, `intents` and `memory`),
/// [`MindState::dropped`] and [`MindState::seal_digest`], carried since 17b
/// cut M3.2, are folded since 17b cut M3.6 (`hash.rs`' item 3c, the cut's one
/// re-bless).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindState {
    /// **The next think's sequence number**: a [`DueKind::MindThink`] whose
    /// `think_seq` is not this one is stale and does nothing.
    pub think_seq: u64,
    /// **The think waiting to land** (D-UM-23), or `None`.
    pub pending: Option<PendingThink>,
    /// **The mind's memory** (D-UM-30): canonical bytes the runner writes and
    /// the arena never decodes. Written at the landing, so a fault keeps the
    /// old bytes.
    #[serde(deserialize_with = "crate::arena::memory_bound::deserialize")]
    pub memory: Vec<u8>,
    /// **The notice buffer** (D-UM-8): every notice staged on the hull since
    /// the last think — the pilot's own notices, and the mind's own refusals
    /// — at most [`MIND_NOTICES`], the oldest dropped; drained into the next
    /// view. **17b cut M3.5** (doc 17 §12): filled only while the hull is the
    /// mind's ([`Controller::Mind`]) — a dormant row under a pilot is never
    /// pushed to, so the pilot's notices never reach the mind's buffer and
    /// the first think after a release drains none of them.
    pub notices: Vec<FrameNotice>,
    /// **The range memory** (doc 17 §3; 16b cut B.4's `T-60` hysteresis read
    /// for a mind): the figure each contact's `range_m` was last given as,
    /// settled at every think so a plot near a 50 m edge does not flip.
    pub ranges: BTreeMap<ContactId, u64>,
    /// **The standing order** (D-UM-15 as decided): a squad's order re-keyed
    /// at write time to this hull's own HELD contacts, or `None`.
    pub order: Option<FrameOrder>,
    /// **The issuer of the order this row holds** (the owner's ruling on STOP
    /// C, 2026-10-09; 17b cut M4.6): `Order.issuer` as it landed — the
    /// pilot's tank whose scope the hint was written in, `None` for the
    /// squad's commander. Written beside [`MindState::order`] where the order
    /// lands (`crate::arena::intents::apply_intent`'s Order arm, from the same
    /// `Order` the `ordered` line journals) and cleared wherever the order is
    /// (a release, an expiry, a possess, the arena's end of a `Support` whose
    /// charge left), so `None` whenever `order` is. **A record field, as
    /// `Order.by` is (D-UM-52)**: the hint is re-keyed once, at write time, in
    /// the issuer's scope (D-UM-15, `Arena::rekey_order`), so the issuer
    /// steers nothing after the landing — carried by the snapshot (format 9),
    /// **never folded into `state_hash`** (`crate::arena::hash`'s "Never
    /// folded"); read only by [`Arena::live_orders_by_issuer`]. It replaces
    /// the reflex rows' issuer, which 16b cut Y.1 folded because the reflex
    /// re-keyed its hint at every tick; that fold went with the reflex in
    /// 17b cut M4.6.
    pub order_issuer: Option<ObjId>,
    /// **The charge of the `Support` this row holds** (17b cut M5.2; doc 17
    /// §24's STOP E, the owner's remedy of 2026-10-09: "the charge's `ObjId`
    /// on the order"): `Objective::Support { of }`'s unit as the order
    /// landed, when it is not this hull (a self-charge is re-keyed to
    /// `Engage`, `Arena::rekey_order`); `None` for every other order and
    /// for none. Written beside [`MindState::order`] in
    /// `crate::arena::intents::apply_intent`'s Order arm and cleared wherever
    /// the order is (a release, an expiry, a possess, the arena's end of a
    /// `Support` whose charge left), so `None` whenever `order` is.
    ///
    /// **Decision state, unlike [`MindState::order_issuer`]**:
    /// `Arena::end_mind_support_whose_charge_left` ends the order when this
    /// unit is no longer in [`Arena::units`] — whether or not the supporter
    /// held its charge classified at the landing (`MindObjective::Support {
    /// of: None }`) or still holds a row for it — so it is carried by the
    /// snapshot (format 10) and **folded inside the order's own bytes**:
    /// item 3c's `Support` arm folds it after the charge's row
    /// (`crate::arena::hash`), so only a row holding a `Support` folds a new
    /// byte. Not on the view.
    pub order_charge: Option<ObjId>,
    /// **The faults counted against the mind** (D-UM-31; 17b cut M2.2): the
    /// faulted thinks running, at most [`MIND_STRIKES_TO_DROP`]. The think arm
    /// writes it — a faulted think `min(strikes + 1, 3)`, a clean think `0`
    /// only while the row is not dropped. **Since 17b cut M3.2 the drop is
    /// its own field, [`MindState::dropped`]**, which the think arm writes
    /// `Some(DropCause::Strikes)` the think the strikes reach three; the
    /// count itself no longer IS the drop. **Since 17b cut M3.5** the row is
    /// kept across a possess (`Arena::suspend_mind`, doc 17 §12), so a
    /// possess-then-release resumes the strikes and the drop where they stood.
    pub strikes: u8,
    /// **The drop to the ROM, and why** (17b cut M3.2; D-UM-31, D-UM-32, doc
    /// 17 §13): `None` while the mind flies, `Some(cause)` for the rest of the
    /// sortie once dropped — never cleared by a think, clean or faulted.
    /// Written by the think arm on the third strike (`Some(DropCause::Strikes)`)
    /// and, since 17b cut M3.4, by the landing arm (or the think arm with no
    /// landing pending) for a sealed core that is gone (`Some(DropCause::Core)`),
    /// each with one `mind_dropped` line ([`DropCause`]'s head).
    /// **17b cut M3.5 closed M3.2's gap**: a possess keeps the row DORMANT
    /// (`Arena::suspend_mind`) and the release adopts it again
    /// (`Arena::adopt_mind` keeps a standing row), so a dropped hull possessed
    /// and released stays dropped — the ROM thinks, and no second
    /// `mind_dropped` line is journalled. Nothing removes the row since 17b
    /// cut M4.6 deleted the `Mind → Reflex` transition (`Arena::release_mind`,
    /// a fixture's alone) with the reflex controller.
    /// Carried by the snapshot; **folded into `state_hash` since 17b cut
    /// M3.6** (its presence, then the cause's byte: strikes 1, core 2, abi 3;
    /// after `strikes`, before the card).
    pub dropped: Option<DropCause>,
    /// **The digest of the seal the mind was checked out under** (17b cut
    /// M3.2; doc 17 §13, D-UM-45): since 17b cut M3.3 the custody seal's
    /// [`MindSeal::digest`], read off [`crate::arena::state::UnitState::seal`]
    /// when the row is laid down (`Arena::adopt_mind`).
    /// Carried by the snapshot; **folded into `state_hash` since 17b cut
    /// M3.6** (§13's fold list: its 32 bytes, after the drop).
    pub seal_digest: [u8; 32],
    /// **The doctrine's state, cached** (17b cut M4.3; the M4 row's finding
    /// (4)): `StockMemory.state` as the last landed memory decodes it
    /// ([`doctrine_state_of`]), `Startup` for an empty, absent or undecodable
    /// `state` key. Scratch: never hashed (item 3c folds the bytes it is
    /// derived from), never carried (`SNAPSHOT_FORMAT` stays; re-derived at
    /// the decode), refreshed at every write of [`MindState::memory`]. Read by
    /// the tempo's `ai_approaching` (`Arena::any_mind_approaching`), the
    /// squad signal and the Y.4 catch-up through [`Arena::machine_state`],
    /// each gated on [`Controller::Mind`] — and, **since 17b cut M4.6**, on
    /// the hull's [`SealRef::doctrine`]: a mind sealed on anything but
    /// Standard Doctrine reads
    /// [`crate::arena::doctrine::MachineState::Unknown`], its cache unread.
    #[serde(skip)]
    pub doctrine_state: crate::arena::doctrine::MachineState,
}

impl Default for MindState {
    /// **[`MindState::fresh`] over the default doctrine** — kept for the
    /// fixtures that build a row by hand; the arena itself always lays a row
    /// down over its own `ReflexConfig`.
    fn default() -> Self {
        MindState::fresh(&ReflexConfig::default())
    }
}

impl MindState {
    /// **A fresh mind's row** (17b cut M1b.1): nothing thought, nothing
    /// remembered, nothing said, no order — and the Line card read off `cfg`,
    /// the arena's own doctrine (`crate::arena::card::line`).
    #[must_use]
    pub fn fresh(_cfg: &ReflexConfig) -> MindState {
        MindState {
            think_seq: 0,
            pending: None,
            memory: Vec::new(),
            notices: Vec::new(),
            ranges: BTreeMap::new(),
            order: None,
            order_issuer: None,
            order_charge: None,
            strikes: 0,
            dropped: None,
            seal_digest: [0; 32],
            doctrine_state: crate::arena::doctrine::MachineState::Startup,
        }
    }

    /// **Whether the hull is dropped to the ROM** (17b cut M2.2; D-UM-31,
    /// D-UM-32): since 17b cut M3.2 a read of [`MindState::dropped`]'s field —
    /// `Some` of any cause. Handed to every think as [`ThinkJob::dropped`].
    #[must_use]
    pub fn dropped(&self) -> bool {
        self.dropped.is_some()
    }

    /// **Append one notice to the buffer** (D-UM-8), dropping the oldest past
    /// [`MIND_NOTICES`].
    pub fn push_notice(&mut self, notice: FrameNotice) {
        self.notices.push(notice);
        if self.notices.len() > MIND_NOTICES {
            self.notices.remove(0);
        }
    }
}

/// **What runs a think** (doc 17 §7, D-UM-1): the seam the sim task plugs a
/// runner into. Called synchronously inside the think's due event; it must be
/// a pure function of the job for the cross-tempo property to hold.
pub trait MindRunner: std::fmt::Debug + Send + Sync {
    /// Think once.
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult;
}

/// **One think's input** (doc 17 §6).
///
/// **17b cut M2.2** (17b's settled question (ii); D-UM-29, D-UM-31): the job
/// carries the match `seed` (the key of a mind's `rand(k)` draws through
/// `DrawLabel::Mind`) and `dropped` (the hull's three-fault drop to doctrine),
/// and hands the view and the map behind the arena's own `Arc`s — the view
/// wrapped once per think after [`Arena::mind_view`], the map the arena's
/// [`Arena::definition`] itself — so a scripted runner can hold either for
/// the length of a think from inside its engine without copying a
/// sixty-five-thousand-cell terrain or reaching it through a raw pointer.
#[derive(Clone, Copy, Debug)]
pub struct ThinkJob<'a> {
    /// The hull thinking — its own object number, which is also the view's
    /// `unit` (doc 17 §3 reads "no kernel takes or returns an `ObjId`" as about
    /// other units: the M1a plan's departure (e)).
    pub unit: ObjId,
    /// The think's sequence number.
    pub think_seq: u64,
    /// **The think's lattice millisecond** — the [`DueKind::MindThink`]'s own
    /// timestamp. The view's `t_ms` is the end of the substep it fell in, the
    /// state's own clock (D-UM-20's *the view is the state then*).
    pub t_ms: i64,
    /// **The think's landing millisecond** (17b cut M1a.6; D-UM-21): `t_ms`
    /// plus the substep length read at collection — the
    /// [`DueKind::MindLand`]'s own timestamp, and the millisecond every
    /// discrete intent the think emits is resolved at
    /// (`Arena::apply_machine_intents`). A runner that plans a shot reads its
    /// weapon's readiness for THIS millisecond — the view's `ready_in_ms`
    /// against `lands_ms − view.t_ms` — as the reflex's `engage_orders` reads
    /// it at the millisecond `Arena::resolve_fire` is handed (16b cut M.2);
    /// read at the view's clock, a weapon coming ready inside the landing's
    /// substep is a shot declined until the next think, a whole `think_ms`
    /// lost per round where doc 17 §17 budgets at most one substep.
    pub lands_ms: i64,
    /// **The match seed** (17b cut M2.2; D-UM-29): the key of a mind's
    /// `rand(k)` draws, each drawn under `DrawLabel::Mind { unit, think_seq,
    /// n }` — the arena's own [`Arena::seed`].
    pub seed: u64,
    /// **Whether the hull is dropped to doctrine** (17b cut M2.2; D-UM-31,
    /// D-UM-32): [`MindState::dropped`] — three faults running — or, since
    /// 17b cut M3.1 (D-UM-33), a mind class of `Zero` ([`Arena::mind_class`]):
    /// a hull with no core, or one whose core reads Class 0, is handed to the
    /// ROM from its first think, with no drop line (17b cut M3.4 writes one
    /// only on strikes and on a sealed core that is gone). A scripted runner
    /// answers a dropped job with its native stand-in — the ROM, Standard
    /// Doctrine's native body at this job's Class 0 row; the native stock mind
    /// ignores it (it is the stand-in).
    pub dropped: bool,
    /// **The row the mind thinks at** (17b cut M3.1; doc 17 §9, D-UM-35,
    /// D-UM-4): [`Arena::mind_profile`] — the class table's row of
    /// `min(rated, core)`, Class 0's for a dropped job. The view was cut to
    /// its `rows` and `odds_rows`; a runner reads its op ceiling, its memory
    /// keys, its `line_to` cap and its `slots` here. Standard Doctrine on any
    /// shipped core reads Class I's row, today's constants.
    pub profile: &'a crate::arena::core::CoreProfile,
    /// The hull's view, behind the `Arc` the arena wrapped it in once for
    /// this think (17b cut M2.2).
    pub view: &'a Arc<MindView>,
    /// The memory the last landed think left.
    pub memory: &'a [u8],
    /// The map: the window, its zones, its spots and its waypoint graph —
    /// the arena's own [`Arena::definition`], behind its `Arc` (17b cut M2.2).
    pub map: &'a Arc<ArenaDefinition>,
    /// **The exact card captured for this think** (doc 17 §11): the active
    /// [`ProfileRuntime::card`](super::profile_runtime::ProfileRuntime::card),
    /// including its immutable key and hydrated schema, values and rules.
    /// Stock helpers read its optional
    /// [`RuntimeCard::stock`](super::runtime_card::RuntimeCard::stock) adapter;
    /// authored minds read their declared values. `NoMind` ignores it. A later
    /// profile change invalidates this job's token rather than replacing its card.
    pub card: &'a super::runtime_card::RuntimeCard,
    pub program: &'a super::profile_rules::BoundProgram,
    pub profile_token: super::profile_runtime::ProfileToken,
}

/// **Why a think faulted** (17b cut M2.2; doc 17 §8, D-UM-31): the cause
/// word a runner answers in [`ThinkResult::fault`] — one of `ops` (the
/// operation-and-fee budget ran out), `limit` (an engine limit: depth, size,
/// a kernel's per-think cap), `type` (a value of the wrong type, a refused
/// symbol), `memory` (a memory past its class's shape), `intents` (more than
/// [`MIND_INTENTS_MAX`] intents), `unknown` (an id the view does not hold)
/// or `panic` (the runner caught a panic). The arena reads that there IS a
/// fault — it lands nothing and counts a strike ([`Arena`]'s think arm) — and,
/// since 17b cut M3.4, journals the word on the think's `mind_fault` line (and
/// on the `mind_dropped` line of the third running).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinkFault {
    /// The cause word.
    pub cause: SmolStr,
}

/// **One think's answer** (doc 17 §6): the intents in emission order and the
/// new memory — and, since 17b cut M2.2 (D-UM-31), the fault that ended it
/// and the runner's trace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThinkResult {
    /// Bounded runtime facts only; private source is never copied into diagnostics.
    pub diagnostics: Option<super::author_trace::ThinkDiagnostics>,
    /// Complete metered think-term truth, published only on successful landing.
    pub rule_sample: Option<super::profile_rules::RuleSample>,
    /// What to do, at most [`MIND_INTENTS_MAX`].
    pub intents: Vec<IntentKind>,
    /// What to remember.
    pub memory: Vec<u8>,
    /// **The fault that ended the think** (17b cut M2.2; D-UM-31), or `None`
    /// for a clean think. A faulted think lands nothing whatever `intents`
    /// and `memory` hold.
    pub fault: Option<ThinkFault>,
    /// **The runner's trace** (17b cut M2.2; D-UM-31): the lines a script
    /// printed, at most eight. The arena drops it — never hashed, never
    /// journalled; a recording runner reads it in the tests.
    pub trace: Vec<String>,
}

/// **The runner that thinks nothing** (17b cut M1a.5): no intent, the memory
/// handed back as it came. What every arena ran until 17b cut M1a.6's native
/// stock mind replaced it in [`default_mind_runner`]; a test still sets it on
/// [`Arena::mind_runner`] for a hull no runner moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoMind;

impl MindRunner for NoMind {
    /// Nothing, the memory as it came, no fault and no trace.
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        ThinkResult {
            rule_sample: None,
            diagnostics: None,
            intents: Vec::new(),
            memory: job.memory.to_vec(),
            fault: None,
            trace: Vec::new(),
        }
    }
}

/// **The runner an arena is opened with** (17b cut M1a.5): the one function
/// both [`Arena::new`] and the snapshot's decode call, so a resumed match
/// thinks with the runner a fresh one does.
///
/// **17b cut M1a.6** (doc 17 §17, D-UM-3): the native stock mind,
/// [`crate::arena::stock_mind::NativeStockMind`], on the match's own frozen
/// doctrine (`reflex`) — T1, the oracle the Rhai port is held to. [`NoMind`]
/// stays for the tests that want a hull no runner moves.
#[must_use]
pub fn default_mind_runner(cfg: &ArenaConfig) -> Arc<dyn MindRunner> {
    Arc::new(crate::arena::stock_mind::NativeStockMind::with(&cfg.reflex))
}

// ---------------------------------------------------------------------------
// The collected think and the settle (17b cut M4.1; doc 17 §6, D-UM-22)
// ---------------------------------------------------------------------------

/// **One think, collected at its millisecond and not yet written** (17b cut
/// M4.1; doc 17 §6 "Execution policy", D-UM-22): the job's FROZEN inputs —
/// everything [`ThinkJob`] borrows, owned — beside what the write needs that
/// the job does not: the think's own substep's end hash for a fault's lines,
/// and the range memory a fault puts back. A pure function of these answers
/// the think wherever it runs (the sim task's pool, or inline at the next
/// substep's head), and `Arena::settle_collected` writes the answer.
///
/// `Clone` so the arena keeps its own copy on [`Arena`]'s scratch while the
/// task hands clones to the pool (`crate::arena::step::StepOutput::thinks`):
/// the `Arc`s are shared and the owned parts are small (a memory of at most a
/// few hundred bytes, a range memory of at most the row's contacts).
#[derive(Clone, Debug)]
pub struct CollectedThink {
    /// Station epochs captured with this job, even if its pending row is later discarded.
    pub epochs: [u64; 3],
    /// The hull thinking ([`ThinkJob::unit`]).
    pub unit: ObjId,
    /// The think's sequence number ([`ThinkJob::think_seq`]).
    pub think_seq: u64,
    /// The think's lattice millisecond ([`ThinkJob::t_ms`]).
    pub t_ms: i64,
    /// The landing millisecond ([`ThinkJob::lands_ms`]).
    pub lands_ms: i64,
    /// The match seed ([`ThinkJob::seed`]).
    pub seed: u64,
    /// Whether the job is the ROM's ([`ThinkJob::dropped`]).
    pub dropped: bool,
    /// The row the think is held to ([`ThinkJob::profile`]), by value.
    pub profile: crate::arena::core::CoreProfile,
    /// The view, behind the `Arc` the arena wrapped it in ([`ThinkJob::view`]).
    pub view: Arc<MindView>,
    /// The memory the last landed think left ([`ThinkJob::memory`]), by value.
    pub memory: Vec<u8>,
    /// The map ([`ThinkJob::map`]).
    pub map: Arc<ArenaDefinition>,
    /// The card the mind thinks under ([`ThinkJob::card`]), by value.
    pub card: super::runtime_card::RuntimeCard,
    pub program: super::profile_rules::BoundProgram,
    pub profile_token: super::profile_runtime::ProfileToken,
    /// **The runner the hull thinks on** ([`Arena::runner_for`] at
    /// collection): its own by its seal, else the arena-wide fallback.
    pub runner: Arc<dyn MindRunner>,
    /// **The think's own substep's end hash** — the `state_hash` the substep
    /// the think fell in recorded (`Arena::substep_hashes`), stamped by
    /// `Arena::stamp_collected` at that record; `None` only between the
    /// collection and the record. What a faulted think's `mind_fault` and
    /// `mind_dropped` lines carry (17b §3's M4 row, finding (1)): a fresh hash
    /// at the write would differ between the inline head and the step's end
    /// (notices are staged after the push, the drain records a hash and phase
    /// 5 folds the tempo), where this one is one function of sim time.
    pub substep_hash: Option<u64>,
    /// **The range memory as it stood before the view settled it** — what a
    /// fault puts back (D-UM-9: a think is atomic; the view's drained notices
    /// are the view's own `notices`).
    pub ranges_before: BTreeMap<ContactId, u64>,
}

impl CollectedThink {
    /// **The job, borrowed off the frozen inputs** — what the runner is
    /// handed, on whichever thread runs it.
    #[must_use]
    pub fn job(&self) -> ThinkJob<'_> {
        ThinkJob {
            unit: self.unit,
            think_seq: self.think_seq,
            t_ms: self.t_ms,
            lands_ms: self.lands_ms,
            seed: self.seed,
            dropped: self.dropped,
            profile: &self.profile,
            view: &self.view,
            memory: &self.memory,
            map: &self.map,
            card: &self.card,
            program: &self.program,
            profile_token: self.profile_token,
        }
    }

    /// **Run the think on its own runner, now, on this thread** — one
    /// [`ThinkAnswer`] addressed to the row. The pool's worker and
    /// [`run_batch_inline`] call exactly this; the result is a pure function
    /// of the frozen job (D-UM-22), so where it ran cannot change it.
    #[must_use]
    pub fn think(&self) -> ThinkAnswer {
        ThinkAnswer {
            unit: self.unit,
            think_seq: self.think_seq,
            result: self.runner.think(&self.job()),
        }
    }
}

/// **One think's answer, addressed** (17b cut M4.1): the row it is for and
/// the runner's [`ThinkResult`]. What a batch answers in job order, and what
/// [`Arena::deliver_answers`] writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinkAnswer {
    /// The hull.
    pub unit: ObjId,
    /// The think's sequence number — matched against the row's `pending`.
    pub think_seq: u64,
    /// The runner's answer.
    pub result: ThinkResult,
}

/// **What one settle wrote** (17b cut M4.1): the thinks written, the faults
/// among them, the overflows among the faults, and the answers dropped
/// because their row no longer awaited them (a hull wrecked, possessed or
/// released since the collection; a match since ended). The faults are what
/// `crate::arena::motor::MotorCounters::mind_faults` counts: a caller that
/// settles between two pulses (the sim task, [`Arena::step_settled`]) folds
/// them into the counters it reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settled {
    /// Thinks whose answer or fault was written.
    pub written: u32,
    /// Faulted thinks among them (`MotorCounters::mind_faults`).
    pub faults: u32,
    /// Overflows among the faults (`MotorCounters::mind_overflows`).
    pub overflows: u32,
    /// Answers dropped unwritten.
    pub dropped: u32,
}

impl Settled {
    /// Fold `other` into this one, saturating.
    fn absorb(&mut self, other: Settled) {
        self.written = self.written.saturating_add(other.written);
        self.faults = self.faults.saturating_add(other.faults);
        self.overflows = self.overflows.saturating_add(other.overflows);
        self.dropped = self.dropped.saturating_add(other.dropped);
    }
}

/// **Run a batch of collected thinks inline, in order, on this thread**
/// (17b cut M4.1): each on its own runner ([`CollectedThink::think`]), the
/// answers in job order — what the sim task's pool does on its workers, done
/// synchronously by a caller with no pool ([`Arena::step_settled`], every
/// fixture). A runner that panics unwinds through the caller: on the sim
/// task that is `Arena::step`'s unwind boundary and the match is void, as a
/// pool worker's panic voids it (doc 17 §24; never a fault).
#[must_use]
pub fn run_batch_inline(thinks: &[CollectedThink]) -> Vec<ThinkAnswer> {
    thinks.iter().map(CollectedThink::think).collect()
}

// ---------------------------------------------------------------------------
// Memory (17b cut M1a.6; doc 17 §8, D-UM-30 as amended; 17b cut M2.2)
// ---------------------------------------------------------------------------

/// **A mind's memory, decoded, and the codec's names** (17b cut M2.2; doc 17
/// §8, D-UM-30 as amended — 17b's settled question (i)): `mush_mind`'s ONE
/// codec, re-exported. [`Memory`] is a `BTreeMap<String, MemValue>`, so the
/// keys are sorted and one map has one encoding; [`MemValue`] is an INT, a
/// bool, unit, a printable-ASCII string or an INT array; [`MemError`] is the
/// shape check's refusal. Until 17b cut M2.2 this file kept its own map over
/// `serde_json` values and wrote it with `serde_json::to_vec`; the bytes
/// `mush_mind::memory::encode` writes are those bytes for every memory the
/// check accepts (pinned by `tests/minds.rs`'
/// `m2_the_memory_codec_writes_the_native_bytes`), so the native stock mind
/// and its scripted port hand back the same bytes (Gate A) through the one
/// writer.
pub use mush_mind::memory::{MemError, Memory, Value as MemValue};

/// **The stock class's memory limits** (doc 17 §8; D-UM-35's class I
/// `memory_keys`): at most 32 keys of 1 to 24 characters, strings of at most
/// 32 bytes, INT arrays of at most 32 — and, since 17b cut M2.2, keys and
/// strings in printable ASCII (`0x20..=0x7E`; D-UM-30 as amended). **The Class
/// I row of `ClassTable::STOCK`; production reads the profile, the tests'
/// arithmetic keeps the constant** (17b cut M3.1: the scripted runner holds a
/// think to `Limits { keys: profile.memory_keys, ..STOCK_MEMORY_LIMITS }` —
/// the key count is the class's, the shape bounds are every class's). The
/// ASCII rule changes nothing the native stock mind writes: its fifteen keys
/// are ASCII words and it writes no string value.
pub const STOCK_MEMORY_LIMITS: mush_mind::memory::Limits = mush_mind::memory::Limits {
    keys: 32,
    key_chars: 24,
    string_bytes: 32,
    array_len: 32,
};

/// **Whether a memory is one a stock mind may hold** (doc 17 §8): the one
/// shape check, `mush_mind::memory::check`, over [`STOCK_MEMORY_LIMITS`].
///
/// # Errors
///
/// The first refusal, in key order: the count before any key.
pub fn memory_check(memory: &Memory) -> Result<(), MemError> {
    mush_mind::memory::check(memory, &STOCK_MEMORY_LIMITS)
}

/// **The doctrine state a memory holds** (17b cut M4.3): its `state` key as
/// `StockMemory` reads it — `Startup` for an empty, absent or ill-shaped one
/// (the twin's read, made production).
#[must_use]
pub fn doctrine_state_of(memory: &[u8]) -> crate::arena::doctrine::MachineState {
    crate::arena::stock_mind::StockMemory::from(&decode_memory(memory)).state
}

/// **The Withdraw seed** (17b cut M4.3; the M4 row's finding (5); 16b cut
/// N.1): the bytes the ROM would write for a released hull — Standard
/// Doctrine's fifteen keys at their defaults with `state` `Withdraw` and
/// `ordered` false — so the mind's first think drives it home and rallies on
/// Q.7's clock (the expiry's restoration reads `ordered`), and the row's
/// first write changes no shape.
#[must_use]
pub fn withdraw_seed() -> Vec<u8> {
    encode_memory(&Memory::from(crate::arena::stock_mind::StockMemory {
        state: crate::arena::doctrine::MachineState::Withdraw,
        ordered: false,
        ..crate::arena::stock_mind::StockMemory::default()
    }))
}

/// **A standing memory seeded `Withdraw`** (17b cut M4.3): the dormant row a
/// possess left keeps what it saw, with the doctrine's state set `Withdraw`,
/// `ordered` cleared, and exactly the six keys the reflex's `adopt` cleared
/// — `stand_until_ms`, `quiet_since_ms`, `rallied`, `last_plot`,
/// `unheld_since_ms`, `hunt` (16b cuts Q.7 and E.4: a seat released into
/// `Withdraw` neither inherits its pilot's last contact as a place to drive
/// to nor has used its one return). An empty memory is [`withdraw_seed`].
#[must_use]
pub fn seed_withdraw(memory: &[u8]) -> Vec<u8> {
    if memory.is_empty() {
        return withdraw_seed();
    }
    let mut stock = crate::arena::stock_mind::StockMemory::from(&decode_memory(memory));
    stock.state = crate::arena::doctrine::MachineState::Withdraw;
    stock.ordered = false;
    stock.stand_until_ms = None;
    stock.quiet_since_ms = None;
    stock.rallied = false;
    stock.last_plot = None;
    stock.unheld_since_ms = None;
    stock.hunt = None;
    encode_memory(&Memory::from(stock))
}

/// **A memory's canonical bytes** (D-UM-30): `mush_mind::memory::encode` —
/// sorted compact JSON with only `"` and `\` escaped, what the arena stores
/// and hashes. An empty map is `{}`.
#[must_use]
pub fn encode_memory(memory: &Memory) -> Vec<u8> {
    mush_mind::memory::encode(memory)
}

/// **A memory read back off its bytes**: `mush_mind::memory::decode`, with
/// the arena's rule kept — empty bytes are the empty map (a fresh sortie),
/// and so are bytes that do not decode: nothing writes such bytes, and a
/// runner handed them starts over rather than faulting.
#[must_use]
pub fn decode_memory(bytes: &[u8]) -> Memory {
    if bytes.is_empty() {
        return Memory::new();
    }
    mush_mind::memory::decode(bytes).unwrap_or_default()
}

/// **What a mind sees** (17b cut M1a.5; doc 17 §3, D-UM-4 read strictly — the
/// M1a plan's departure (h)): the hull's keyframe, field by field, at the
/// keyframe's own resolution and nothing finer.
///
/// Every field is a key of `crate::match_::cockpit::status_frame_held` or a
/// [`ContactRow`] rounding of one: `unit`, `callsign`, `cell`, `x_cm`, `y_cm`,
/// `z_cm`, `heading_mdeg` (the keyframe carries the own pose at those
/// resolutions), `speed_cms`, `power`, `held` (the keyframe's `blocked`),
/// `wreck`, `immobile`, `sight_m`, `ecm`, `receiver`, `heat`, `sections`,
/// `weapons` (each row's keyframe keys, `bands_cm` included since 17b cut
/// M1a.3), `lock`, `order`, `t_ms`, `notices`, `contacts` and (since 17b cut
/// M1b.0) `scan_tables`, the tables the contacts' `condition.table` indexes,
/// and (since 17b cut M1b.3) `odds_floor_permille`; each weapon row carries its
/// `armed` slot since the same cut.
/// `team` is the keyframe's callsign letter read as a side. The only object
/// number in it is the hull's own.
///
/// **One field is not the keyframe's** (17b cut M4.6b; doc 17 §3 and D-UM-4
/// as amended 2026-10-09, the owner's STOP B ruling): [`MindView::immobile`]
/// reads a hull authored with no speed as immobile once its power is
/// `Online`, where the keyframe's `immobile` is its motion disabled alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MindView {
    /// The hull's own object number (the keyframe's `unit`).
    pub unit: u32,
    /// Its public name (`callsign`).
    pub callsign: Designator,
    /// Its party: what the entry-zone kernels read, never a mind (doc 32
    /// Q-CF-3: the mind API exposes relation, not party).
    pub party: PartyId,
    /// The cell it stands in (`cell`).
    pub cell: Option<(u16, u16)>,
    /// Public match mode (`mode`).
    pub mode: crate::arena::state::MatchMode,
    /// Last accepted hit on this hull in arena time; absent before any hit.
    pub own_last_hit_ms: Option<i64>,
    /// Public objective geometry (`objective`), never capture state.
    pub objective: Option<crate::arena::state::HoldObjective>,
    /// Its own requested drive and retained outcome (`motor`).
    pub motor: Option<crate::arena::step::FrameMotor>,
    /// Its own pose, centimetres (`x_cm`, `y_cm`, `z_cm`).
    pub x_cm: i64,
    /// North.
    pub y_cm: i64,
    /// Height.
    pub z_cm: i64,
    /// Its heading, millidegrees (`heading_mdeg`).
    pub heading_mdeg: u32,
    /// Its speed, cm/s (`speed_cms`).
    pub speed_cms: i32,
    /// Its power (`power`).
    pub power: Power,
    /// Whether motion is holding it against something (`blocked`).
    pub held: bool,
    /// Whether it is a wreck (`wreck`).
    pub wreck: bool,
    /// **Whether it cannot close** (`immobile`): its motion disabled — the
    /// keyframe's `immobile` — or, since 17b cut M4.6b (the owner's STOP B
    /// ruling, 2026-10-09, "power up, then park"), a hull authored with no
    /// speed (`max_speed_cms` nought, the range's `target_mk1`) whose power
    /// is `Online`. Cold or starting, such a hull reads as the keyframe does,
    /// so its first thinks take the power arm. The one field where the view
    /// is not the keyframe (doc 17 §3, D-UM-4 as amended); the pilot's frame
    /// is untouched.
    pub immobile: bool,
    /// Its own nominal sight, whole metres (`sight_m`).
    pub sight_m: u64,
    /// Its pod's switch, `None` without a pod (`ecm`).
    pub ecm: Option<bool>,
    /// Its own receiver (`receiver`).
    pub receiver: Option<Receiver>,
    /// Its heat (`heat`).
    pub heat: Option<FrameHeat>,
    /// Its own sections (`sections`).
    pub sections: Vec<FrameSection>,
    /// Its weapons (`weapons`), empty on a hull with no weapons station.
    pub weapons: Vec<FrameWeapon>,
    /// All active native rules, at the cockpit keyframe's resolution and order.
    pub rules: Vec<crate::arena::weapons::FrameRule>,
    /// The contact it is locked on (`lock`).
    pub lock: Option<ContactId>,
    /// Its standing order, re-keyed to its own rows (`order`).
    pub order: Option<FrameOrder>,
    /// **The arena millisecond the view was built at** (`t_ms`): the end of
    /// the substep the think fell in, the clock every age above is read on.
    pub t_ms: i64,
    /// The notice buffer, drained into this view (`notices`).
    pub notices: Vec<FrameNotice>,
    /// The class's `rows` of the held contacts (`contacts`;
    /// [`STOCK_CONTACT_ROWS`] at Class I), read nearest-first by the held
    /// range then by [`ContactId`] — cut keeping the hull's lock and its
    /// order's contact first (the seat), then every contact not known
    /// friendly, then the friendly ones (17b cut M4.5c; doc 17 §3 as amended
    /// 2026-10-09 by the owner's two rulings, [`Arena::mind_view`]).
    pub contacts: Vec<ViewContact>,
    /// **The section-word tables `ViewContact.condition.table` indexes**
    /// (`scan_tables`), the keyframe's own: a per-view index, rebuilt per frame
    /// by first appearance across the scope, never stable across thinks (doc
    /// 17 §3 as amended 2026-10-04; 17b cut M1b.0). Moved verbatim from the
    /// frame, every table of it — a table no row of `contacts` indexes
    /// included, so a row's index is the keyframe's index and a reader needs no
    /// remap (D-UM-4 as amended: a truncation is a row cap, never the silent
    /// omission of a field another field indexes).
    pub scan_tables: Vec<Vec<SmolStr>>,
    /// **The match's Compact floor, per mille** (`odds_floor_permille`; 17b
    /// cut M1b.3, D-UM-4, D-UM-17): the floor every arming and cited shot is
    /// held to, learned lawfully off the keyframe's own key and never off the
    /// config.
    pub odds_floor_permille: u32,
}

/// **One contact as a mind sees it** (17b cut M1a.5; doc 17 §3): a keyframe
/// contact's keys at the keyframe's resolution — degrees and the held metres,
/// never millidegrees or centimetres.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewContact {
    /// The row key (`id`).
    pub id: ContactId,
    /// Its short name (`designator`).
    pub designator: Designator,
    /// The number behind a renamed row (`alias`).
    pub alias: Option<u16>,
    /// What the platform knows of it (`class`, the word's source).
    pub class: Classification,
    /// Whether it is own or a friend (`friendly`).
    pub friendly: bool,
    /// **The hull's party's relation to it** (`relation`; doc 32 Q-CF-3: the
    /// mind API exposes relation, not party).
    pub relation: crate::arena::party::Relation,
    /// Whether the last earned plot showed motion (`moving` on the keyframe).
    pub moving: bool,
    /// The bearing to its plot, whole degrees (`bearing_deg`).
    pub bearing_deg: u32,
    /// The same off the hull's heading, whole degrees (`relative_deg`).
    pub relative_deg: u32,
    /// The held range to its plot, whole metres (`range_m`).
    pub range_m: u64,
    /// The band word (`band`).
    pub band: SmolStr,
    /// How old the plot is (`age_ms`).
    pub age_ms: i64,
    /// The plot's cell (`cell`).
    pub cell: Option<(u16, u16)>,
    /// The held reading (`condition`).
    pub condition: Option<FrameCondition>,
    /// The chassis's fatal section words beside a held reading (`fatal`).
    pub fatal: Option<Vec<SmolStr>>,
    /// The facet its plot was showing (`aspect`).
    pub aspect: Aspect,
    /// The odds per weapon, default weapon first (`odds`), as
    /// `(weapon, p_permille)`; empty for a row outside the odds rows and on a
    /// hull with no weapons station.
    pub odds: Vec<(SmolStr, u32)>,
    /// Whether it reads dead (`reads_dead`), `None` on a hull with no weapons
    /// station — the keyframe carries no key there.
    pub reads_dead: Option<bool>,
}

/// **What became of one machine intent** (17b cut M1a.5; doc 17 §4, D-UM-12):
/// `Arena::apply_machine_intents`' answer per kind, in emission order (the
/// mind seam's since 17b cut M4.6a, with the function).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// It took effect — applied, or queued and resolved.
    Applied,
    /// It was refused, at apply or by its resolver, for this reason. (The
    /// arena's order refusal — not `crate::arena::motor::Refusal`, the motor's
    /// remembered stop.)
    Refused(crate::arena::intents::Refusal),
}

impl Arena {
    /// **The first [`DueKind::MindThink`] of every unit the check-out handed a
    /// mind** (17b cut M1a.5; doc 17 §6), in unit-id order, each with a fresh
    /// [`MindState`] — the shape `Arena::schedule_first_reflex_ticks` had
    /// (deleted with the reflex in 17b cut M4.6). Only a fixture fielded one in
    /// this cut; since 17b cut M4.3 every AI-filled hull is one. Since 17b
    /// cut M5.2 each row flies its hull's first sealed profile
    /// (`Arena::adopt_mind`), so a side's fill flies its lead's preset.
    pub fn schedule_first_mind_thinks(&mut self) {
        let minds: Vec<ObjId> = self
            .units
            .iter()
            .filter(|unit| unit.controller == Controller::Mind)
            .map(|unit| unit.id)
            .collect();
        for id in minds {
            self.adopt_mind(id, None);
        }
    }

    /// **Hand a hull to a mind** (17b cut M1a.5): a [`MindState`] row if it
    /// has none, and its first think at its next slot of the stagger grid if
    /// the heap holds none for it — the grid of the mind's own `think_ms`
    /// (17b cut M3.1: [`Arena::mind_profile`], read now; Class I's 1,000 ms,
    /// [`STOCK_THINK_MS`], on every shipped chassis).
    ///
    /// **17b cut M3.5** (doc 17 §12 "Release and possess"; D-UM-2, D-UM-30,
    /// D-UM-32): a row that STANDS is kept — the dormant row a possess left
    /// ([`Arena::suspend_mind`]) — so a release adopts the mind again with its
    /// memory, its strikes, its drop, its seal digest, its card, its range
    /// memory and its notice buffer as the possess found them (its held
    /// order cleared at the suspend, 17b cut M4.3), and its first think after
    /// the release is `think_seq` continued (the think the possess took off
    /// the heap, scheduled again on the grid). Only a hull with no row is laid
    /// a fresh one.
    ///
    /// **17b cut M4.3** (the M4 row's finding (5); 16b cut N.1): `seed` is
    /// the doctrine state a `Released` seat hands the hull over in —
    /// `Withdraw`, the only seed there is — written into the row's memory
    /// (fresh or dormant, [`seed_withdraw`]) and the cache, so the mind's
    /// first think drives the tank home as the reflex machine did; `None`
    /// at check-out, and any other state (a fixture's `set_controller` naming
    /// the state it would have given the reflex machine), keeps the memory
    /// as it is: a mind's state is its memory's, and only a release seeds it.
    ///
    /// **17b cut M5.2** (doc 17 §11, §12; the design note's items 5 and 6): a
    /// row laid here flies the card of the hull's active profile
    /// ([`Arena::active_card`]: Line for every hull sealed on the Line card
    /// alone, so every match that chose nothing lays the bytes it laid), and
    /// the `Withdraw` seed is written only over a Standard Doctrine seal
    /// ([`SealRef::doctrine`]): any other mind keeps its own memory.
    pub(crate) fn adopt_mind(
        &mut self,
        unit: ObjId,
        seed: Option<crate::arena::doctrine::MachineState>,
    ) {
        // **17b cut M3.3** (doc 17 §13): a row laid down here carries the
        // digest of the seal the hull was checked out under, read off the
        // unit's own seal reference — at check-out and at any later adoption
        // alike; a standing row keeps its own (17b cut M3.5: the dormant row's
        // digest is the one it was laid with — a re-seat never re-seals).
        let (sealed, doctrine) = self.unit(unit).map_or(([0; 32], false), |state| {
            (state.seal.digest, state.seal.doctrine)
        });
        let row = self.minds.entry(unit).or_insert_with(|| MindState {
            seal_digest: sealed,
            ..MindState::fresh(&self.arena_cfg.reflex)
        });
        // **17b cut M5.2** (the design note's item 6): the `Withdraw` seed is
        // Standard Doctrine's — a mind sealed on anything else
        // ([`SealRef::doctrine`] false) is handed its own memory untouched,
        // since `seed_withdraw` reads any memory as the stock mind's and
        // drops what it does not know. Reachable only through a fixture mind
        // (the owner's OQ-2): every production seal is Standard Doctrine.
        if seed == Some(crate::arena::doctrine::MachineState::Withdraw) && doctrine {
            row.memory = seed_withdraw(&row.memory);
            row.doctrine_state = doctrine_state_of(&row.memory);
        }
        let think_seq = row.think_seq;
        if self
            .due
            .iter()
            .any(|event| matches!(event.kind, DueKind::MindThink { unit: on, .. } if on == unit))
        {
            return;
        }
        let at = self.grid_at(unit, self.mind_profile(unit).think_ms);
        self.due
            .schedule(at, DueKind::MindThink { unit, think_seq });
    }

    /// The exact active own-hull card from the admitted shared catalog. No
    /// current preset or mutable library lookup occurs during execution.
    #[must_use]
    pub fn active_card(
        &self,
        state: &crate::arena::state::UnitState,
    ) -> Option<&crate::arena::runtime_card::RuntimeCard> {
        self.profiles.get(&state.id).map(|runtime| &runtime.card)
    }

    /// **What the machine flying `unit` is doing, as a doctrine state** (17b
    /// cut M4.3; the M4 row's finding (4)): `Disabled` first, for a wreck or
    /// a disabled hull a mind flies (the twin's rule: a mind stops thinking at
    /// its wreck with its last state in memory); then, under
    /// [`Controller::Mind`], **[`MachineState::Unknown`] for a hull whose seal
    /// is not Standard Doctrine** ([`SealRef::doctrine`], 17b cut M4.6: such a
    /// mind keeps no doctrine state of its own in its memory, so its cache
    /// means nothing)
    /// and the row's cached state ([`MindState::doctrine_state`]) for one that
    /// is; `None` for a pilot's hull (a dormant row under a pilot is never
    /// read). The reflex row's state was read here under the reflex
    /// controller until 17b cut M4.6 deleted it.
    ///
    /// [`MachineState::Unknown`]: crate::arena::doctrine::MachineState::Unknown
    #[must_use]
    pub fn machine_state(&self, unit: ObjId) -> Option<crate::arena::doctrine::MachineState> {
        use crate::arena::doctrine::MachineState;
        let state = self.unit(unit)?;
        if state.limits.wreck || state.disabled {
            return state
                .controller
                .is_machine()
                .then_some(MachineState::Disabled);
        }
        match state.controller {
            Controller::Mind if !state.seal.doctrine => Some(MachineState::Unknown),
            Controller::Mind => Some(
                self.minds
                    .get(&unit)
                    .map_or(MachineState::Startup, |row| row.doctrine_state),
            ),
            Controller::Pilot => None,
        }
    }

    /// **A mind closing on a contact** (17b cut M4.3): any hull a mind flies
    /// whose cached doctrine state is `Approach` — the state its last think
    /// WROTE, so the tempo's Alert follows a mind entering `Approach` one
    /// substep after the reflex's tick would have (the landing); declared in
    /// doc 16 §11. A wreck reads nothing, and (17b cut M4.6) neither does a
    /// hull whose seal is not Standard Doctrine ([`SealRef::doctrine`]): it
    /// keeps no doctrine state of its own in its memory, so the cache is not read —
    /// the gate [`Arena::machine_state`] applies, read here off the cache
    /// directly.
    #[must_use]
    pub(crate) fn any_mind_approaching(&self) -> bool {
        self.units.iter().any(|unit| {
            unit.controller == Controller::Mind
                && !unit.limits.wreck
                && unit.seal.doctrine
                && self.minds.get(&unit.id).is_some_and(|row| {
                    row.doctrine_state == crate::arena::doctrine::MachineState::Approach
                })
        })
    }

    /// **The squads under a live order, grouped by who gave it** (16b cut Y.2;
    /// doc 16 §2 W8's *orders per commander bounded*; over the mind rows since
    /// 17b cut M4.6, the owner's STOP C ruling): every hull a mind flies whose
    /// held order has not expired at arena millisecond `now_ms`, its order's
    /// squad filed under the order's issuer ([`MindState::order_issuer`]).
    ///
    /// Grouped exactly as the scale harness's own tape join
    /// (`crates/tacsim/tests/harness.rs`' `live_orders_by_issuer`, the W8
    /// check, which asserts the two agree): only a hull whose controller is
    /// [`Controller::Mind`] (a dormant row under a pilot is skipped), **a
    /// wreck counted** (a wreck never lands, so its order never expires
    /// through the landing and only this deadline retires it), an order live
    /// while `expires_ms > now_ms`, and an order with no issuer — the LLM
    /// seam's — filed under **the squad commander of the unit it steers**
    /// ([`Arena::squad_of`]), so the machines' own orders count against the
    /// commander that asked for them; a unit in no squad files it under
    /// `None`. The squad is the row's rendered word, parsed
    /// ([`crate::deliberation::SquadId::parse`]; the arena writes only words
    /// it rendered, so a word that does not parse is skipped).
    ///
    /// A pure read for the scale harness's W8 assertion: nothing in the arena
    /// or the world decides anything on it, and nothing but the answer
    /// allocates. It is an **arena-time** figure (`accepted_ms + ttl_ms`), so it
    /// can lead the world's own span — which is counted on the world's watch in
    /// world time ([`crate::arena_bridge::ArenaRegistry::live_orders_of`]) — by
    /// the arena's lag. The reflex rows' version (over `Order.issuer`, folded
    /// there) was deleted with them in 17b cut M4.6.
    #[must_use]
    pub fn live_orders_by_issuer(
        &self,
        now_ms: i64,
    ) -> BTreeMap<Option<ObjId>, std::collections::BTreeSet<crate::deliberation::SquadId>> {
        let mut grouped: BTreeMap<
            Option<ObjId>,
            std::collections::BTreeSet<crate::deliberation::SquadId>,
        > = BTreeMap::new();
        for (unit, row) in &self.minds {
            if self
                .unit(*unit)
                .is_none_or(|state| state.controller != Controller::Mind)
            {
                continue;
            }
            let Some(order) = row.order.as_ref() else {
                continue;
            };
            if order.expires_ms <= now_ms {
                continue;
            }
            let Some(squad) = crate::deliberation::SquadId::parse(&order.squad) else {
                continue;
            };
            let issuer = row
                .order_issuer
                .or_else(|| self.squad_of(*unit).map(|roster| roster.commander));
            grouped.entry(issuer).or_default().insert(squad);
        }
        grouped
    }

    /// **The runner `unit` thinks on** (17b cut M3.5; doc 17 §13, D-UM-1):
    /// its own on [`Arena::runners`] — installed by its seal — else the
    /// arena-wide fallback, [`Arena::mind_runner`].
    #[must_use]
    pub fn runner_for(&self, unit: ObjId) -> &Arc<dyn MindRunner> {
        self.runners.get(&unit).unwrap_or(&self.mind_runner)
    }

    /// **Put a hull's mind to sleep under a pilot** (17b cut M3.5; doc 17 §12
    /// "Release and possess"; D-UM-2, D-UM-30, D-UM-32) — the `Mind → Pilot`
    /// transition of [`Arena::set_controller`]. The row STAYS, DORMANT: its
    /// memory, strikes, drop, seal digest, card, range memory and notice
    /// buffer are kept as they stand, for the release that adopts the mind
    /// again ([`Arena::adopt_mind`]); **its held order is cleared** (17b cut
    /// M4.3): an order given to the mind is not the pilot's and does not
    /// outlive the seat — the hand-back starts under doctrine, as 16b cut
    /// N.1's release does. Every think and landing of the hull
    /// leaves the heap, and the think waiting to land is discarded (`pending`
    /// cleared): its landing would be refused [`crate::arena::Refusal::Epoch`]
    /// at the reclaim's raised epochs anyway, so the answer is dropped here
    /// rather than carried to a landing that never comes (declared, doc 17
    /// §24). While the hull is piloted nothing reaches the row: a staged
    /// notice is the pilot's alone (`Arena::stage_notice` gates its buffer
    /// push on [`Controller::Mind`]) and the row's held order is not the
    /// pilot's (`holds_order_for`, the same gate). A re-seat is a seat
    /// activation inside the same sortie, never a new seal: nothing here
    /// touches [`crate::arena::state::UnitState::seal`].
    pub(crate) fn suspend_mind(&mut self, unit: ObjId) {
        self.trace_cancel_pending(unit, super::author_trace::Cancelled::Controller);
        self.invalidate_rule_sample(unit);
        if let Some(state) = self.minds.get_mut(&unit) {
            state.pending = None;
            // 17b cut M4.3: the held order goes with the seat — and (17b cut
            // M4.6, the owner's STOP C) its issuer with it, and (17b cut M5.2,
            // STOP E) its charge.
            state.order = None;
            state.order_issuer = None;
            state.order_charge = None;
        }
        // 17b cut M4.1: a think collected and not yet written is dropped at
        // its write (`Arena::settle_collected` reads this set).
        self.clear_order_watch_member(unit);
        self.awaiting.remove(&unit);
        self.forget_mind_events(unit);
    }

    /// Every [`DueKind::MindThink`] and [`DueKind::MindLand`] of `unit` off
    /// the heap — [`Arena::suspend_mind`]'s half (shared with
    /// `Arena::release_mind`, the `Mind → Reflex` transition, until 17b cut
    /// M4.6 deleted that with the reflex controller).
    fn forget_mind_events(&mut self, unit: ObjId) {
        self.due.retain(|kind| match kind {
            DueKind::MindThink { unit: on, .. } | DueKind::MindLand { unit: on, .. } => *on != unit,
            _ => true,
        });
    }

    /// **Hand a unit to a controller, or take it back** (doc 16 §3.8
    /// "Disconnects"), which is the one thing that moves
    /// [`crate::arena::state::UnitState::controller`] after check-out — since
    /// 17b cut M4.6 between the two controllers there are, a pilot and a mind
    /// (it lived in the reflex controller's module, with that controller's two
    /// arms, until that cut deleted it).
    ///
    /// **17b cut M1a.5: the outgoing controller is released on every
    /// transition**, read off the unit before anything is written. From
    /// [`Controller::Mind`] (to a pilot, the only other controller): every
    /// `MindThink` and `MindLand` of the unit and its think waiting to land,
    /// and — **since 17b cut M3.5** (doc 17 §12 "Release and possess") — the
    /// row STAYS, dormant (`Arena::suspend_mind`, its held order and issuer
    /// cleared), for the release that hands the hull back. From **either**:
    /// the hull's motor row and every `MotorTick` — a pilot's `DRIVE` dies
    /// with the seat, and a mind's goal with the mind — and the condition
    /// slots (17b cut M1b.1; doc 17 §12's "conditions stay dormant" is NOT
    /// built: the slots are the outgoing principal's on every transition,
    /// declared). Then the incoming one is installed: `Mind` keeps a standing
    /// row or lays a fresh one down, and schedules its first think; `Pilot`
    /// installs nothing. The row is seeded only when `state` is `Withdraw` —
    /// the only seed `Arena::adopt_mind` honours (17b cut M4.3: a
    /// `Released` hands the hull over in `Withdraw`, as 16b cut N.1 seeded
    /// the reflex), written into its memory and its cache; any other `state`
    /// keeps the row's memory as it stands (a fresh row's reads `Startup`).
    /// Nothing here touches the unit's seal: a re-seat is a seat activation
    /// inside the same sortie. Since 17b cut M1a.1 the motor's readers
    /// (`Arena::hold_routeless`, `Arena::note_blocked`) gate on the motor row
    /// and not on the controller, which is why no path that moves the
    /// controller may leave one behind.
    ///
    /// `state` is the seed (the conventions' D7: kept a parameter, typed
    /// [`crate::arena::doctrine::MachineState`], so no caller's seed is
    /// hard-coded here); a hand-over to a pilot reads nothing of it. A unit
    /// already under `controller` keeps everything it has.
    pub fn set_controller(
        &mut self,
        unit: ObjId,
        controller: Controller,
        state: crate::arena::doctrine::MachineState,
    ) {
        let Some(was) = self.unit(unit).map(|slot| slot.controller) else {
            return;
        };
        if was == controller {
            return;
        }
        // The outgoing controller's rows and cadence: 17b cut M3.5 (doc 17
        // §12), a possess keeps the mind's row dormant, with its held order
        // cleared (17b cut M4.3).
        if was == Controller::Mind {
            self.suspend_mind(unit);
        }
        // **17b cut M1b.1** (doc 17 §5): the condition slots are the outgoing
        // principal's decisions, so they go with the principal on EVERY
        // transition — a pilot's slots do not fire for the mind that takes
        // the tank over, nor a mind's for the pilot who reclaims it — and with
        // them the held-shot rows. Beside the motor row's release, for the
        // motor row's reason.
        self.clear_conditions(unit);
        self.motors.release(unit);
        self.due
            .retain(|kind| !matches!(kind, DueKind::MotorTick { unit: on } if *on == unit));
        if let Some(slot) = self.unit_mut(unit) {
            slot.controller = controller;
        }
        // The incoming one. 17b cut M4.3: a release (`Pilot → Mind`, the only
        // hand-over to a mind there is) seeds the doctrine's state as N.1
        // seeded the reflex's.
        if controller == Controller::Mind {
            // M6 ownership handback restores the selected immutable program,
            // while preserving dormant memory and the selected profile.
            if self.restore_profile_bindings(unit).is_err() {
                self.stop(crate::arena::state::StopReason::Void);
                return;
            }
            self.adopt_mind(unit, Some(state));
        }
    }

    /// **One hull's view, as its next think would read it** (17b cut M1a.5;
    /// doc 17 §3, D-UM-4), or `None` for a unit this arena does not hold or one
    /// with no mind row.
    ///
    /// Built by `Arena::frame_for` — the builder a pilot's keyframe is built by,
    /// which leaves the pilots' notices where they are — and projected through
    /// [`crate::arena::view::frame_contacts_held`] against the mind's own range
    /// memory, which it then settles ([`crate::arena::view::settle_ranges`]):
    /// the rows read nearest-first by the held `range_m` then by
    /// [`ContactId`], `rows` of them. It **drains** the mind's notice buffer
    /// into the view. Two writes, both hashed state, which is why it takes
    /// `&mut self` and why only the think arm calls it in a match.
    ///
    /// **The cut** (17b cut M4.5c; doc 17 §3 as amended 2026-10-09, the
    /// owner's ruling on STOP L, and as amended 2026-10-09 again, the owner's
    /// second ruling): first **the seat** — the held contacts the hull's own
    /// state names, its lock ([`crate::arena::view::frame_lock`]) and its
    /// standing order's contact (the view's own `order`, `frame.order`:
    /// `Support { of }`'s charge or `Engage { target }`'s target; no other
    /// objective names one) — then every contact NOT KNOWN FRIENDLY
    /// ([`ContactRow::friendly`] false — a hostile, or a return whose side is
    /// `unknown`), then the friendly ones, each group nearest-first by the
    /// held `range_m` then by [`ContactId`]; the first `rows` of that; then
    /// the kept rows re-read nearest-first by `(range_m, id)`. So a view
    /// holding no more contacts than its rows is the plain nearest-first cut
    /// it was, and only a view that truncates changes — in which rows it
    /// keeps, never in their order. Only a contact the frame holds is seated
    /// (the rows are the held contacts), at most two rows are, and every
    /// stock class carries at least eight, so the seat never fills a stock
    /// view (`[tacsim.minds].classes` may configure as few as one row; the
    /// seated rows are then themselves cut by the rest of the key). The
    /// order seated is the one the view presents, an order past its
    /// deadline that its landing has not yet cleared included (the view and
    /// its seat agree), and so is an order the mind has ended on its own
    /// (its memory's `ignore_until_ms`): the seat reads the view, not the
    /// mind's obedience. Until this cut the rows were cut nearest-first
    /// alone, and a hull mustered among more friends than its rows saw no
    /// enemy at all; with the not-known-friendly priority alone, a supporter
    /// crowded by hostiles lost its charge (the seat). The pilot's frame is
    /// not cut here (parity: a mind still reads no row the frame does not
    /// hold).
    ///
    /// **17b cut M3.1** (doc 17 §9, D-UM-35, D-UM-4): the cut is the mind's
    /// own row ([`Arena::mind_profile`]) — `rows` contacts (Class I's 12,
    /// [`STOCK_CONTACT_ROWS`]), and the odds kept for the first `odds_rows`
    /// contacts the FRAME carries odds for, in the frame's order (a hostile
    /// lock first, then nearest). M7 completes the advertised Class II/III
    /// ceilings in that shared frame: the actual carried, sealed and capped
    /// class extends the human picture beyond its native/Class I two-row floor.
    /// The mind still only truncates that picture (D-UM-4); an authored class
    /// declaration grants no rows. This changes the frame/replay compatibility,
    /// while the existing API4 getter and configured class ceilings remain. At
    /// `odds_rows == 0` (Class 0, the ROM) only the hull's lock row keeps its
    /// odds.
    ///
    /// **`immobile`** (17b cut M4.6b; doc 17 §3 and D-UM-4 as amended
    /// 2026-10-09, the owner's STOP B ruling in its "power up, then park"
    /// form): `frame.immobile || (no authored speed && power Online)` — the
    /// keyframe's motion-disabled flag, or a hull whose `max_speed_cms` is
    /// nought once its power is `Online`. A cold `target_mk1` (no weapon, no
    /// speed, power `off` at check-out) therefore reads mobile while `Off`
    /// and `Starting`, takes the stock mind's power arm (`STARTUP`) and comes
    /// online; its first think after reads `immobile` with no weapon rows,
    /// which is the `Disabled` edge, and it parks — the reflex's inert rule's
    /// "start up, then park". An armed hull authored with no speed reads
    /// `cannot_close` once online, so its edge into `Engage` is `release_cm`
    /// (16b cut AA.1). The frame (`Arena::frame_for`), the keyframe and the
    /// pilot are untouched: this is the view's one field that is not the
    /// keyframe's, read here and nowhere else.
    pub fn mind_view(&mut self, unit: ObjId) -> Option<MindView> {
        let profile = self.mind_profile(unit);
        let frame = self.frame_for(unit, self.clock.tempo())?;
        // 17b cut M4.6b (the owner's STOP B ruling, 2026-10-09, in its "power
        // up, then park" form): the MIND reads `immobile` for a hull authored
        // with no speed too — once its power is `Online`, so a cold range
        // target's first thinks still take the power arm (`STARTUP`) and only
        // then read it immobile (a weaponless one: `Disabled`, parked). The
        // pilot's frame keeps `limits.motion_disabled` alone (`frame_for`);
        // this is the one view field that is not the keyframe's.
        let immobile = frame.immobile
            || (frame.power == Power::Online
                && self
                    .unit(unit)
                    .is_some_and(|state| state.mobility.max_speed_cms == 0));
        let lock = crate::arena::view::frame_lock(&frame);
        let state = self.minds.get_mut(&unit)?;
        let mut rows: Vec<ContactRow> =
            crate::arena::view::frame_contacts_held(&frame, &state.ranges);
        state.ranges = crate::arena::view::settle_ranges(&frame, &state.ranges);
        let notices = std::mem::take(&mut state.notices);
        // 17b cut M4.5c (doc 17 §3 as amended 2026-10-09 twice, the owner's
        // two rulings): the cut SEATS the contacts the hull's own state names
        // — its lock and its standing order's contact, the order exactly as
        // the view presents it (`frame.order`, an order past its deadline
        // that the landing has not yet cleared included, so the view and its
        // seat agree, and an order the mind has ended on its own — its
        // memory's `ignore_until_ms` — included too: the seat reads the view,
        // not the mind's obedience) — then keeps every contact not known
        // friendly before any friendly one, each group nearest-first, so a
        // hull mustered among more friends than its rows still sees the enemy
        // and a supporter crowded by hostiles still sees its charge; the kept
        // rows then read nearest-first as before. At most two rows are seated
        // and every stock class carries at least eight, so the seat never
        // fills a stock view (a configured table may carry fewer: the seated
        // rows are then cut by the rest of the key, not known friendly first).
        // The match names every objective: only `Engage` and `Support` name
        // a contact.
        let ordered = frame
            .order
            .as_ref()
            .and_then(|order| match order.objective {
                MindObjective::Engage { target } => target,
                MindObjective::Support { of } => of,
                MindObjective::Hold
                | MindObjective::Advance { .. }
                | MindObjective::Withdraw
                | MindObjective::Screen { .. } => None,
            });
        let seated = |id: ContactId| Some(id) == lock || Some(id) == ordered;
        rows.sort_by_key(|row| (!seated(row.id), row.friendly, row.range_m, row.id));
        rows.truncate(profile.rows);
        rows.sort_by_key(|row| (row.range_m, row.id));
        // The contacts whose odds the view keeps: the frame's own odds rows,
        // in the frame's order, the first `odds_rows` of them — or the lock's
        // alone at nought.
        let mut odds_rows: Vec<ContactId> = Vec::new();
        for odds in &frame.odds {
            if !odds_rows.contains(&odds.contact) {
                odds_rows.push(odds.contact);
            }
        }
        if profile.odds_rows == 0 {
            odds_rows.retain(|id| Some(*id) == lock);
        } else {
            odds_rows.truncate(profile.odds_rows);
        }
        let contacts = rows
            .into_iter()
            .filter_map(|row| {
                let held = frame.contacts.iter().find(|held| held.id == row.id)?;
                Some(ViewContact {
                    id: row.id,
                    designator: row.designator,
                    alias: row.alias,
                    class: held.class,
                    friendly: row.friendly,
                    relation: row.relation,
                    moving: held.moving,
                    bearing_deg: row.bearing_deg,
                    relative_deg: row.relative_deg,
                    range_m: row.range_m,
                    band: row.band,
                    age_ms: row.age_ms,
                    cell: row.cell,
                    condition: held.condition,
                    fatal: held.fatal.clone(),
                    aspect: held.aspect,
                    odds: frame
                        .odds
                        .iter()
                        .filter(|odds| odds.contact == row.id && odds_rows.contains(&row.id))
                        .map(|odds| (odds.weapon.clone(), odds.p_permille))
                        .collect(),
                    reads_dead: held.reads_dead,
                })
            })
            .collect();
        Some(MindView {
            unit: frame.unit.0,
            callsign: frame.callsign,
            party: frame.party,
            cell: frame.cell,
            mode: frame.mode,
            own_last_hit_ms: frame.own_last_hit_ms,
            objective: frame.objective,
            motor: frame.motor,
            x_cm: frame.pose.x_cm,
            y_cm: frame.pose.y_cm,
            z_cm: i64::from(frame.pose.z_cm),
            heading_mdeg: frame.pose.heading_mdeg,
            speed_cms: frame.speed_cms,
            power: frame.power,
            held: frame.held,
            wreck: frame.wreck,
            immobile,
            sight_m: u64::from(frame.sight_cm / 100),
            ecm: frame.ecm,
            receiver: frame.receiver,
            heat: frame.heat,
            sections: frame.sections,
            lock,
            weapons: frame.weapons,
            rules: frame.rules,
            order: frame.order,
            t_ms: frame.t_ms,
            notices,
            contacts,
            scan_tables: frame.scan_tables,
            odds_floor_permille: frame.odds_floor_permille,
        })
    }

    /// **The think arm's first half: collect** (17b cut M4.1; doc 17 §6
    /// "Execution policy", D-UM-22 — until this cut the whole arm,
    /// `mind_think`, ran the runner here): one [`DueKind::MindThink`], at its
    /// lattice millisecond `t_ms`, with every other kind due at that
    /// millisecond already applied.
    ///
    /// In order: nothing after the match ended; nothing for a unit that is
    /// gone, a wreck or not a mind's, or for a stale `think_seq` — and none of
    /// those reschedules (an adoption schedules its own); with no landing
    /// pending, the drop for a sealed core that is gone (17b cut M3.4,
    /// `Arena::core_lost`, `Arena::drop_for_core`, its line under
    /// `Arena::think_line_hash`), so this think is the ROM's; the view
    /// ([`Arena::mind_view`]); the three epochs recorded; `think_seq`
    /// advanced; **the think's FACTS written on the row** — `pending` as a
    /// [`PendingThink`] of this `think_seq`, `t_ms` and the epochs with an
    /// EMPTY answer, and the hull named in [`Arena::awaiting`] — so the
    /// substep's hash folds the facts and no answer (`hash.rs`' item 3c,
    /// unchanged); the landing ([`DueKind::MindLand`], one substep on) and
    /// the next think (the mind's `think_ms` after this one's own
    /// millisecond, [`Arena::mind_profile`] read now — Class I's
    /// [`STOCK_THINK_MS`] on every shipped chassis) scheduled, so phase 5's
    /// `next_due_ms` wakes the task for them; and the job frozen on
    /// [`Arena`]'s scratch as a [`CollectedThink`] — the row the think is held
    /// to ([`ThinkJob::profile`]), `dropped` for a drop or a mind class of
    /// `Zero`, the hull's own runner ([`Arena::runner_for`]) — for
    /// `Arena::settle_collected` to write at the head of the next substep.
    /// The runner is NOT called here.
    pub(crate) fn collect_think(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        t_ms: i64,
        counters: &mut MotorCounters,
    ) {
        if self.ended.is_some()
            || !self
                .unit(unit)
                .is_some_and(|state| state.controller == Controller::Mind && !state.limits.wreck)
            || !self
                .minds
                .get(&unit)
                .is_some_and(|state| state.think_seq == think_seq)
        {
            return;
        }
        // 17b cut M3.4 (D-UM-32, D-UM-34): a core the seal recorded that is
        // gone drops the hull to the ROM at its next LANDING — and here, at its
        // next think, when no landing is pending (the core died with no think
        // in flight), before the view is taken, so this think is the ROM's.
        // The line takes `Arena::think_line_hash`: a think fills no lazy slot.
        if self
            .minds
            .get(&unit)
            .is_some_and(|state| state.pending.is_none())
            && self.core_lost(unit)
        {
            let hash = self.think_line_hash();
            self.drop_for_core(unit, t_ms, hash);
        }
        // D-UM-9: the two writes `mind_view` makes — the range memory settled
        // and the notice buffer drained — are undone if the think faults, so the
        // range memory as it stood is kept to put back (the drained notices are
        // the view's own).
        let ranges_before = self
            .minds
            .get(&unit)
            .map(|state| state.ranges.clone())
            .unwrap_or_default();
        let Some(view) = self.mind_view(unit) else {
            return;
        };
        // 17b cut M2.2: the view behind an `Arc`, wrapped once, so a scripted
        // runner shares it for the think instead of copying it.
        let view = Arc::new(view);
        // D-UM-13: the three epochs at the think, in `Channel::ALL`'s order —
        // carried on the pending answer (17b cut M3.2) to the landing.
        let epochs = Channel::ALL.map(|channel| self.epochs.current(unit, channel));
        counters.thinks = counters.thinks.saturating_add(1);
        // D-UM-21: the landing millisecond, read at collection — scheduled
        // below and handed to the runner (17b cut M1a.6), which plans its
        // shots for the millisecond they are resolved at.
        let lands_ms = t_ms.saturating_add(self.substep_len_ms());
        // 17b cut M3.5 (doc 17 §13): the hull's own runner, by its seal, else
        // the arena-wide fallback.
        let runner = Arc::clone(self.runner_for(unit));
        // 17b cut M3.1 (doc 17 §9, D-UM-35): the row this think is held to,
        // read now — the view above was cut to it, and the next think is
        // scheduled at its `think_ms` below. A mind class of `Zero` (no core,
        // a core at Class 0, or the drop) hands the job to the ROM. 17b cut
        // M3.4 cuts the condition row to Class 0's one slot at every drop
        // (`Arena::truncate_slots_to`) and holds a dropped hull's arming to it
        // (`Arena::arming_capacity`), so the ROM's one slot is the hull's.
        let class = self.mind_class(unit);
        let profile = self.classes.row(class);
        let period = profile.think_ms;
        let seed = self.seed;
        let map = Arc::clone(&self.definition);
        let Some(runtime) = self.profiles.get(&unit) else {
            return;
        };
        let card = runtime.card.clone();
        let program = runtime.program.clone();
        let profile_token = runtime.token();
        self.trace_cancel_pending(unit, super::author_trace::Cancelled::Superseded);
        let Some(state) = self.minds.get_mut(&unit) else {
            return;
        };
        let dropped = state.dropped() || class == crate::arena::core::CoreClass::Zero;
        let memory = state.memory.clone();
        state.think_seq = think_seq.saturating_add(1);
        // **17b cut M4.1**: the facts, and no answer — written at the settle
        // (`Arena::settle_collected`); a fault clears them there.
        state.pending = Some(PendingThink {
            think_seq,
            t_ms,
            epochs,
            profile_token,
            lands_ms,
            period_ms: period,
            profile_class: class,
            rule_sample: None,
            intents: Vec::new(),
            memory: Vec::new(),
        });
        self.awaiting.insert(unit);
        // The landing and the next think, scheduled NOW so phase 5's
        // `next_due_ms` wakes the task for them (a think in a pulse's last
        // substep is settled at the next step's head, after phase 5 ran): the
        // settle retracts the landing on a fault, and moves the next think to
        // Class 0's period on the strike that drops the hull (17b cut M3.1:
        // the cadence of a hull that has just dropped is Class 0's from its
        // next think).
        self.due
            .schedule(lands_ms, DueKind::MindLand { unit, think_seq });
        self.due.schedule(
            t_ms.saturating_add(period),
            DueKind::MindThink {
                unit,
                think_seq: think_seq.saturating_add(1),
            },
        );
        self.traces.begin(unit, think_seq, t_ms, dropped);
        self.collected.push(CollectedThink {
            epochs,
            unit,
            think_seq,
            t_ms,
            lands_ms,
            seed,
            dropped,
            profile,
            view,
            memory,
            map,
            card,
            program,
            profile_token,
            runner,
            substep_hash: None,
            ranges_before,
        });
    }

    /// **Stamp this substep's collected thinks with its end hash** (17b cut
    /// M4.1): called by `Arena::substep` right after it pushed the substep's
    /// `state_hash`, so every think collected in the substep carries THE
    /// SUBSTEP'S OWN hash ([`CollectedThink::substep_hash`]) and moves to the
    /// unsettled list the next settle point writes.
    pub(crate) fn stamp_collected(&mut self, hash: u64) {
        for mut think in self.collected.drain(..) {
            think.substep_hash = Some(hash);
            self.unsettled.push(think);
        }
    }

    /// **The thinks collected and not yet written** (17b cut M4.1): what
    /// `crate::arena::step::StepOutput::thinks` carries out to the sim task,
    /// and what the next `Arena::step` settles inline at its head if nothing
    /// delivered them.
    #[must_use]
    pub fn thinks_outstanding(&self) -> &[CollectedThink] {
        &self.unsettled
    }

    /// **Discard every think not yet written** (17b cut M4.1): what the sim
    /// task does before the stopping pulse that voids a match, so that
    /// pulse's head runs no runner (the one that just panicked on the pool
    /// among them). The rows keep their facts; a voided match is never
    /// encoded.
    pub fn discard_outstanding(&mut self) {
        self.trace_cancel_all(super::author_trace::Cancelled::Ended);
        self.unsettled.clear();
        self.collected.clear();
    }

    /// **The think arm's second half: settle** (17b cut M4.1; doc 17 §6,
    /// D-UM-9, D-UM-22, D-UM-23 as amended, D-UM-31, D-UM-32): write ONE
    /// collected think's answer on its row — at the head of the substep after
    /// the think's, which is before anything of the next substep (the inline
    /// path, [`Arena::settle_inline`]) and before the next pulse's phases on
    /// the pooled path ([`Arena::deliver_answers`]): one sim-time moment,
    /// whichever thread ran the runner.
    ///
    /// Nothing is written — the answer DROPPED, counted — when the match has
    /// ended, when the hull no longer awaits a think (wrecked or possessed
    /// since the collection: `destroy_unit` and [`Arena::suspend_mind`] clear
    /// [`Arena::awaiting`], as `Arena::release_mind` did for a hull handed to
    /// the reflex until 17b cut M4.6), or when the
    /// row's `pending` no longer names THIS think. Otherwise **D-UM-9,
    /// atomic**: a FAULT — since 17b cut M2.2 a runner's
    /// [`ThinkResult::fault`] or, as before, more than [`MIND_INTENTS_MAX`]
    /// intents — lands nothing, keeps the old memory, puts back what the view
    /// drained and settled (the notices in front of the buffer, oldest
    /// dropped past [`MIND_NOTICES`]; the range memory as it was), clears
    /// `pending` (counted `faults`, and `overflows` when the length tripped),
    /// writes a strike (`strikes = min(strikes + 1,` [`MIND_STRIKES_TO_DROP`]`)`,
    /// D-UM-31 — the third running writes the drop, [`MindState::dropped`]
    /// `= Some(DropCause::Strikes)`, 17b cut M3.2), **retracts the landing**
    /// the collection scheduled, because there is nothing to land, and —
    /// **17b cut M3.4** — journals one `mind_fault` line (the runner's cause
    /// word, or [`OVERFLOW_CAUSE`]) and, on the dropping strike, one
    /// `mind_dropped` line (`strikes`, that word, the row's `ops`) **under the
    /// think's own substep's end hash** ([`CollectedThink::substep_hash`]) at
    /// the think's own `t_ms`, cuts the condition row to the ROM's one slot
    /// and moves the next think to Class 0's period; otherwise the strikes
    /// are cleared unless the hull is dropped and the answer — the intents,
    /// the memory — is written into the row's [`PendingThink`] (17b cut M3.2,
    /// D-UM-23 as amended: carried by the snapshot from here to the landing).
    /// The runner's trace is dropped: never hashed, never journalled. No
    /// action hash is opened or filled: the lines take the stamped hash.
    pub(crate) fn settle_collected(
        &mut self,
        think: &CollectedThink,
        result: ThinkResult,
    ) -> Settled {
        self.tape_witnesses.observe(think, &result, think.epochs);
        self.trace_complete(think, &result);
        let unit = think.unit;
        let dropped = Settled {
            dropped: 1,
            ..Settled::default()
        };
        if self.ended.is_some()
            || !self.awaiting.contains(&unit)
            || !self
                .profiles
                .get(&unit)
                .is_some_and(|runtime| runtime.accepts(think.profile_token))
        {
            return dropped;
        }
        let Some(state) = self.minds.get_mut(&unit) else {
            return dropped;
        };
        if !state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.think_seq == think.think_seq)
        {
            return dropped;
        }
        self.awaiting.remove(&unit);
        // Stamped at the think's substep's record on every path through
        // `Arena::step`; the fallback is for a think settled by a test rig
        // before any substep recorded it.
        let hash = think
            .substep_hash
            .unwrap_or_else(|| crate::arena::hash::state_hash(self));
        // 17b cut M2.2 (D-UM-31): a runner's fault and D-UM-9's overflow are
        // one path. `result.trace` is dropped here: never hashed, never
        // journalled. 17b cut M3.4: the fault's cause word — the runner's
        // own, or `intents` for an overflow with no fault — is what the
        // `mind_fault` line carries.
        let overflowed = result.intents.len() > MIND_INTENTS_MAX;
        let sample_bad = result.rule_sample.as_ref().is_some_and(|sample| {
            let mut checked = crate::arena::profile_rules::RuleLease::default();
            // Due events retain their scheduled millisecond, while the public
            // picture is captured after motion at the substep boundary. An
            // off-grid think therefore legitimately has view.t_ms > t_ms.
            // Bind the sample to the picture it actually read, not its timer.
            sample.sampled_ms != think.view.t_ms
                || checked
                    .land(
                        Some(sample.clone()),
                        &think.program,
                        think.profile_token.generation,
                        think.lands_ms,
                        think.lands_ms.saturating_add(think.profile.think_ms),
                    )
                    .is_err()
        });
        let cause: Option<SmolStr> = result
            .fault
            .as_ref()
            .map(|fault| fault.cause.clone())
            .or_else(|| overflowed.then(|| SmolStr::new_static(OVERFLOW_CAUSE)))
            .or_else(|| sample_bad.then(|| SmolStr::new_static("rules")));
        if let Some(cause) = cause.as_deref() {
            self.traces.mark_fault(unit, think.think_seq, cause);
        }
        let Some(state) = self.minds.get_mut(&unit) else {
            return dropped;
        };
        let Some(cause) = cause else {
            // D-UM-31's reset: a clean think clears the strikes — but never a
            // drop, which is for the rest of the sortie (D-UM-32).
            if !state.dropped() {
                state.strikes = 0;
            }
            // D-UM-23 as amended (17b cut M3.2): the answer rides the row to
            // its landing, so a snapshot between the two carries it.
            if let Some(pending) = state.pending.as_mut() {
                pending.intents = result.intents;
                pending.memory = result.memory;
                pending.rule_sample = result.rule_sample;
            }
            return Settled {
                written: 1,
                ..Settled::default()
            };
        };
        // D-UM-9: a fault lands nothing — not a prefix — and the memory the
        // last landed think left is the memory the next think is handed.
        // The think is atomic for the mind's own row too: what the view
        // drained goes back in front of the buffer and the range memory is
        // the one it was, so the notices the faulted think was handed —
        // its own refusals among them (D-UM-8, *a refusal is never
        // silent*) — reach the next think, whose memory never saw them.
        // 17b cut M2.2 (D-UM-31, D-UM-32): a strike, held at three.
        state.strikes = state.strikes.saturating_add(1).min(MIND_STRIKES_TO_DROP);
        // 17b cut M3.2: the third running writes the drop on its own
        // field — once, a drop is never re-caused (17b cut M3.4 journals
        // it below).
        let dropped_now = state.strikes >= MIND_STRIKES_TO_DROP && state.dropped.is_none();
        if dropped_now {
            state.dropped = Some(DropCause::Strikes);
        }
        state.pending = None;
        state.ranges = think.ranges_before.clone();
        let mut notices = think.view.notices.clone();
        notices.append(&mut state.notices);
        let excess = notices.len().saturating_sub(MIND_NOTICES);
        notices.drain(..excess);
        state.notices = notices;
        self.invalidate_rule_sample(unit);
        // The landing the collection scheduled: nothing lands.
        let think_seq = think.think_seq;
        self.due.retain(|kind| {
            !matches!(kind, DueKind::MindLand { unit: on, think_seq: seq } if *on == unit && *seq == think_seq)
        });
        // **17b cut M3.4** (doc 17 §8, §19; D-UM-31, D-UM-32): the fault is
        // journalled — one `mind_fault` line with its cause word at the think's
        // millisecond — and, on the strike that drops the hull, ONE
        // `mind_dropped` line beside it carrying what the principal's sentence
        // needs (the last fault's word and the op budget the faulting thinks
        // ran under, this think's row's), so the world tells it from the tape.
        // Both under ONE hash — since 17b cut M4.1 the think's own substep's
        // end hash, at the think's own `t_ms`, written here at the next
        // substep's head: the tape is not `t_ms`-monotonic across these lines
        // (declared). No action hash is opened for a think, and the lines fill
        // none. At the drop the condition row is cut to Class 0's one slot:
        // the ROM's.
        let _ = self.ledger.append(
            think.t_ms,
            EventKind::MindFault {
                unit,
                think_seq,
                cause: cause.clone(),
            },
            Vec::new(),
            hash,
        );
        if dropped_now {
            self.traces
                .drop_to_rom(unit, think_seq, super::author_trace::RomDrop::Strikes);
            let _ = self.ledger.append(
                think.t_ms,
                EventKind::MindDropped {
                    unit,
                    cause: SmolStr::new_static(DropCause::Strikes.as_str()),
                    fault: Some(cause),
                    ops: Some(think.profile.ops),
                },
                Vec::new(),
                hash,
            );
            if self.drop_runtime_to_rom(unit).is_err() {
                self.stop(crate::arena::state::StopReason::Void);
            }
            // 17b cut M3.1: a hull that has just dropped is on Class 0's
            // period from its next think — the one the collection scheduled
            // on the class it then had is moved when the period differs.
            self.reschedule_next_think(unit, think_seq.saturating_add(1), think.t_ms);
        }
        Settled {
            written: 1,
            faults: 1,
            overflows: u32::from(overflowed),
            dropped: 0,
        }
    }

    /// **Move `unit`'s next think to its row's period now** (17b cut M4.1,
    /// for the settle's drop): the scheduled [`DueKind::MindThink`] of
    /// `think_seq`, if its millisecond is not `t_ms` plus the period
    /// [`Arena::mind_profile`] reads now, is retracted and scheduled there —
    /// always later (a drop lowers the class, and no class's period is under
    /// Class III's one substep), so never before the settle's own moment.
    fn reschedule_next_think(&mut self, unit: ObjId, think_seq: u64, t_ms: i64) {
        let at = t_ms.saturating_add(self.mind_profile(unit).think_ms);
        let scheduled = self.due.iter().find_map(|event| match event.kind {
            DueKind::MindThink {
                unit: on,
                think_seq: seq,
            } if on == unit && seq == think_seq => Some(event.t_ms),
            _ => None,
        });
        if scheduled.is_none_or(|was| was == at) {
            return;
        }
        self.due.retain(|kind| {
            !matches!(kind, DueKind::MindThink { unit: on, think_seq: seq } if *on == unit && *seq == think_seq)
        });
        self.due
            .schedule(at, DueKind::MindThink { unit, think_seq });
    }

    /// **Settle every outstanding think inline, now, on this thread** (17b
    /// cut M4.1; doc 17 §6 "Execution policy": *when a landing's substep
    /// falls inside the same wake as its think … `Arena::step` calls the
    /// runner synchronously*): each unsettled think's runner called
    /// ([`CollectedThink::think`]) and its answer written
    /// (`Arena::settle_collected`), in collection order. Called at the head
    /// of every substep and at the head of `Arena::step` — so a caller that
    /// never delivers a batch (every fixture driving `step` alone) still has
    /// every think answered at the one sim-time moment, with the runner's
    /// wall time inside the step. The faults are answered in the [`Settled`]
    /// for the caller's counters.
    pub fn settle_inline(&mut self) -> Settled {
        let thinks = std::mem::take(&mut self.unsettled);
        let mut settled = Settled::default();
        for think in &thinks {
            let answer = think.think();
            settled.absorb(self.settle_collected(think, answer.result));
        }
        settled
    }

    /// **Write a batch's answers** (17b cut M4.1; doc 17 §6 "Execution
    /// policy", D-UM-22): what the sim task calls with the pool's answers
    /// before its next `Arena::step` and before it encodes a snapshot — each
    /// answer matched to the outstanding think of its `(unit, think_seq)`
    /// and written (`Arena::settle_collected`), in the order given (the
    /// batch's job order); an answer no outstanding think matches is dropped
    /// and counted. A think the batch did not answer stays outstanding and
    /// the next step settles it inline ([`Arena::settle_inline`]). A caller
    /// with no pool runs the batch itself: [`run_batch_inline`] over
    /// `StepOutput::thinks`, or simply [`Arena::step_settled`].
    pub fn deliver_answers(&mut self, answers: Vec<ThinkAnswer>) -> Settled {
        let mut settled = Settled::default();
        for answer in answers {
            let Some(at) = self
                .unsettled
                .iter()
                .position(|think| think.unit == answer.unit && think.think_seq == answer.think_seq)
            else {
                self.trace_complete_late(answer.unit, answer.think_seq, &answer.result);
                settled.dropped = settled.dropped.saturating_add(1);
                continue;
            };
            let think = self.unsettled.remove(at);
            settled.absorb(self.settle_collected(&think, answer.result));
        }
        settled
    }

    /// **The think arm as it stood before 17b cut M4.1 — collect, stamp and
    /// settle in one call** — kept for this file's own tests, which drive a
    /// think outside any substep: the stamp is [`Arena::think_line_hash`]
    /// (the substep's hash under a `Taken` slot, else a fresh one), the
    /// hash the arm's lines took until this cut.
    #[cfg(test)]
    pub(crate) fn mind_think(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        t_ms: i64,
        counters: &mut MotorCounters,
    ) {
        self.collect_think(unit, think_seq, t_ms, counters);
        let hash = self.think_line_hash();
        self.stamp_collected(hash);
        let settled = self.settle_inline();
        counters.mind_faults = counters.mind_faults.saturating_add(settled.faults);
        counters.mind_overflows = counters.mind_overflows.saturating_add(settled.overflows);
    }

    /// **Put a machine's intents through the pilot's own door** (17b cut M1a.5;
    /// doc 17 §4, D-UM-12) — `Arena::order`'s body, factored out so the reflex
    /// controller (until 17b cut M4.6 deleted it), the motor and a mind's
    /// landing are one function.
    ///
    /// The kinds go in emission order, `seq` ascending from `1`, each an
    /// [`Intent`] on the channel that carries it at `epoch_of(channel)`
    /// (`counters.intents` counted per intent), through [`apply_intent`] under
    /// `source`. One answer per kind, in order:
    ///
    /// * refused at apply — [`Landed::Refused`] with `apply_intent`'s word;
    /// * a continuous kind applied — [`Landed::Applied`]; a
    ///   [`MotorGoal::Cell`] that applied runs the motor once here
    ///   (`Arena::motor_step` at `t_ms`), the one follow at landing (D-UM-10);
    /// * a discrete kind queued — popped straight back out of `pending` and
    ///   resolved at `t_ms`: `Startup` by [`resolve_startup`], `Lock` by
    ///   `Arena::resolve_lock`, `Fire` by `Arena::resolve_fire` (`shots`
    ///   counted on `Ok`), and — new in this cut — `Emcon` by
    ///   [`resolve_emcon`]. **Each resolver's `Err` is the answer**
    ///   ([`Landed::Refused`]) and its `Ok` is [`Landed::Applied`]; until this
    ///   cut `Arena::order` dropped them with `let _`, and since it still does
    ///   not read the vector no byte of the reflex controller's moved (the M1a.1
    ///   pins). `Order`, `ReleaseOrder`, `Place` and `Goal` queue nothing.
    ///
    /// **17b cut M1b.1**: a `Fire` resolves through `Arena::resolve_fire_by` as
    /// [`crate::arena::weapons::FiredBy::Mind`] with the slot it cites for an
    /// [`IntentSource::Mind`] and as `Machine` otherwise (T0 is never gated); a
    /// mind's arming (`IntentKind::Condition`) lands through
    /// `Arena::resolve_arming`, in emission order, with NO evaluation here —
    /// `Arena::mind_land` evaluates the hull's slots once after every intent.
    ///
    /// **17b cut M4.6a**: here, beside the mind's landing that calls it, out of
    /// the reflex controller's module — it reads and writes no reflex state.
    pub(crate) fn apply_machine_intents(
        &mut self,
        unit: ObjId,
        kinds: &[IntentKind],
        t_ms: i64,
        epoch_of: &dyn Fn(Channel) -> u64,
        source: IntentSource,
        counters: &mut MotorCounters,
    ) -> Vec<Landed> {
        let mut landed: Vec<Landed> = Vec::with_capacity(kinds.len());
        let mut seq = 0_u64;
        for kind in kinds {
            let channel = kind.channel_required();
            seq = seq.saturating_add(1);
            let intent = Intent {
                unit,
                channel,
                epoch: epoch_of(channel),
                kind: kind.clone(),
                seq,
            };
            counters.intents = counters.intents.saturating_add(1);
            // **16b cut M.1** (`T-82`): the machine's own order, which the
            // per-seat rate never counts — see `IntentSource`; a mind's
            // (17b cut M1a.5, D-UM-14) neither.
            match apply_intent(self, &intent, source) {
                IntentOutcome::Refused(refusal) => {
                    landed.push(Landed::Refused(refusal));
                    continue;
                }
                IntentOutcome::AppliedContinuous => {
                    // **17b cut M1a.4** (D-UM-10): a goal takes effect at the
                    // landing millisecond, as phase 2's does for a seat's.
                    if matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })) {
                        let _ = self.motor_step(unit, t_ms, counters);
                    }
                    landed.push(Landed::Applied);
                    continue;
                }
                IntentOutcome::Queued => {}
            }
            // `apply_intent` pushed exactly this order to the back of the unit's
            // queue. It comes straight back out: leaving it there would hand it to
            // phase 4, at the pulse's end, which is the arena millisecond a
            // machine's intents must not land at (doc 16 §3.1's D-16-2: a
            // machine's intent lands at its own millisecond, `DueKind::MindLand`).
            if let Some(queue) = self.pending.get_mut(&unit) {
                queue.pop_back();
                if queue.is_empty() {
                    self.pending.remove(&unit);
                }
            }
            let resolved = match &intent.kind {
                IntentKind::Startup => resolve_startup(self, unit, t_ms),
                IntentKind::Lock { contact } => self.resolve_lock(unit, *contact, t_ms),
                // **17b cut M1b.1** (D-UM-14, D-UM-17): who fired — a mind's
                // shot with the slot it cites (gated from 17b cut M1b.2), a
                // machine's never gated.
                IntentKind::Fire { weapon, slot } => self
                    .resolve_fire_by(
                        unit,
                        weapon,
                        t_ms,
                        match source {
                            IntentSource::Mind => FiredBy::Mind { slot: *slot },
                            IntentSource::Seat => FiredBy::Seat,
                            IntentSource::Machine => FiredBy::Machine,
                            IntentSource::Cond => slot.map_or(FiredBy::Mind { slot: None }, |slot| FiredBy::Cond { slot }),
                        },
                    )
                    .map(|_| {
                        counters.shots = counters.shots.saturating_add(1);
                    }),
                // **17b cut M1b.1** (D-UM-10): a mind's arming lands here, in
                // emission order, through the one resolver a seat's goes
                // through in phase 4. NO evaluation here: the landing's one
                // evaluation is `mind_land`'s, after every intent (17b's
                // adversarial F5).
                IntentKind::Condition(arming) => {
                    self.resolve_machine_arming(unit, arming, t_ms).map(|_| ())
                }
                // **17b cut M1a.5** (D-UM-12): the arm `Arena::order` never
                // had. The reflex controller never switched its pod (16b cut
                // O.2's residual: a machine kept the switch where it found it),
                // so it never reached here; a mind's `emcon` does.
                IntentKind::Emcon { on } => resolve_emcon(self, unit, *on, t_ms),
                IntentKind::Heading { .. }
                | IntentKind::Speed { .. }
                | IntentKind::Order(_)
                | IntentKind::DetachmentCommand(_)
                | IntentKind::ReleaseOrder { .. }
                // 16b cut I.3: a staff act; a machine never issues one, and a
                // mind's is refused `Channel` at apply.
                | IntentKind::Place { .. }
                // 17b cut M1a.4: continuous, so never queued.
                | IntentKind::Goal(_)
                // 17b cut M5.2: a mind's is refused `Channel` at apply.
                | IntentKind::Profile { .. } => Ok(()),
            };
            landed.push(match resolved {
                Ok(()) => Landed::Applied,
                Err(refusal) => Landed::Refused(refusal),
            });
        }
        landed
    }

    /// **The landing arm** (17b cut M1a.5; doc 17 §6, D-UM-21, D-UM-13): one
    /// [`DueKind::MindLand`], one substep after its think, at `t_ms`.
    ///
    /// In order:
    ///
    /// 1. nothing unless the hull's row holds a [`PendingThink`] for THIS
    ///    think (a stale landing — a think since faulted, a row since laid
    ///    down afresh — lands nothing);
    /// 2. nothing after the match ended, nor for a unit gone, a wreck or no
    ///    longer a mind's (a reclaimed seat: 17b cut M3.5's
    ///    `Arena::suspend_mind` already cleared `pending` and took this
    ///    landing off the heap, and kept the row dormant) — the row's
    ///    `pending` is left as it stands;
    /// 3. **the answer is taken off the row** (17b cut M3.2; D-UM-23 as
    ///    amended): `pending` cleared, its memory written — the answer the
    ///    think gave, carried across a snapshot if one fell between the two,
    ///    and never recomputed;
    /// 4. a standing order whose deadline has come is cleared, with one
    ///    `order_expired` line journalled by the arena and the squad marked for
    ///    its next signal, as the reflex controller's expiry did — and
    ///    (17b cut M4.6b, the owner's STOP E ruling) so is a `Support` whose
    ///    charge has left the match ([`Arena::end_mind_support_whose_charge_left`]),
    ///    as the reflex controller's end for a charge gone did;
    /// 5. the intents go through `Arena::apply_machine_intents` under
    ///    [`IntentSource::Mind`] at the epochs the think recorded — a channel
    ///    raised since refuses its stale thoughts `Epoch` with no code here —
    ///    and every refusal is appended to the notice buffer as a
    ///    [`FrameNotice::Refused`] in `stage_refusal`'s shape, with no
    ///    `phase_two_tells` filter: *a refusal is never silent* (D-UM-8);
    /// 6. **17b cut M3.4**: a core the hull's seal recorded
    ///    (`SealRef::core_class` above Class 0) that is gone drops the hull to
    ///    the ROM — `dropped = Some(DropCause::Core)`, one `mind_dropped` line
    ///    (`core`, no fault, no budget) at `t_ms` under the landing's hash, the
    ///    runtime replaced with the captured Class 0 ROM, clearing the failed
    ///    program's armings (`Arena::core_lost`, `Arena::drop_for_core`) — the
    ///    think in flight having landed as thought;
    /// 7. **17b cut M1b.1**: the hull's condition slots are evaluated ONCE, at
    ///    `t_ms`, after every intent has landed (`ConditionTrigger::Landing`),
    ///    so an arming and a lock of one think are both in force when the
    ///    slots are read.
    pub(crate) fn mind_land(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        t_ms: i64,
        counters: &mut MotorCounters,
    ) {
        if !self.minds.get(&unit).is_some_and(|state| {
            state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.think_seq == think_seq)
        }) {
            return;
        }
        // **17b cut M4.1**: a landing never lands an UNWRITTEN answer. By
        // construction it cannot meet one — the think is settled at the head
        // of the substep after its own, and its landing is due one substep
        // on, in that substep or later — so this gate is the invariant made
        // explicit, not a path.
        if self.awaiting.contains(&unit) {
            // A missing completion cannot retain sampled authority or keep
            // condition evaluation deferred past this landing boundary.
            self.cancel_profile_think(unit);
            let _ = self.evaluate_conditions(unit, t_ms, ConditionTrigger::Landing, None, counters);
            return;
        }
        if self.ended.is_some()
            || !self
                .unit(unit)
                .is_some_and(|state| state.controller == Controller::Mind && !state.limits.wreck)
        {
            return;
        }
        let Some(state) = self.minds.get_mut(&unit) else {
            return;
        };
        let Some(mut answer) = state.pending.take() else {
            return;
        };
        if !self
            .profiles
            .get(&unit)
            .is_some_and(|runtime| runtime.accepts(answer.profile_token))
        {
            self.traces.cancel(
                unit,
                think_seq,
                t_ms,
                super::author_trace::Cancelled::Superseded,
            );
            self.invalidate_rule_sample(unit);
            let _ = self.evaluate_conditions(unit, t_ms, ConditionTrigger::Landing, None, counters);
            return;
        }
        // A sampled program spans stations, so any changed station invalidates
        // the complete sample. Ordinary intents still carry independent channel
        // epochs: a stale helm answer must not suppress a current Fire/Sensors
        // intent or silently discard the mind's otherwise valid memory landing.
        if Channel::ALL
            .iter()
            .any(|channel| self.epochs.current(unit, *channel) != answer.epoch(*channel))
        {
            answer.rule_sample = None;
        }
        if self
            .land_rule_sample(unit, answer.rule_sample.take(), t_ms, answer.period_ms)
            .is_err()
        {
            self.traces.mark_fault(unit, think_seq, "rules");
            let _ = self.evaluate_conditions(unit, t_ms, ConditionTrigger::Landing, None, counters);
            return;
        }
        let Some(state) = self.minds.get_mut(&unit) else {
            return;
        };
        // 17b cut M4.2: the landing reached its apply — the written answer
        // lands, clean or refused intent by intent. A counter, never hashed.
        counters.landings = counters.landings.saturating_add(1);
        state.memory = std::mem::take(&mut answer.memory);
        // 17b cut M4.3: the doctrine's state, cached off the bytes just
        // written (never hashed; the bytes are).
        state.doctrine_state = doctrine_state_of(&state.memory);
        let intents = std::mem::take(&mut answer.intents);
        self.expire_mind_order(unit, t_ms, counters);
        // 17b cut M4.6b (the owner's STOP E): a `Support` whose charge has
        // left the match, ended beside the expiry — after it, so one landing
        // journals at most one end for its one order.
        self.end_mind_support_whose_charge_left(unit, t_ms, counters);
        let epoch_of = |channel: Channel| answer.epoch(channel);
        let landed = self.apply_machine_intents(
            unit,
            &intents,
            t_ms,
            &epoch_of,
            IntentSource::Mind,
            counters,
        );
        self.traces.land(unit, think_seq, t_ms, &landed);
        for (at, outcome) in landed.into_iter().enumerate() {
            let Landed::Refused(refusal) = outcome else {
                continue;
            };
            let Some(kind) = intents.get(at) else {
                continue;
            };
            let notice = match kind {
                IntentKind::Fire { weapon, .. } => {
                    self.refusal_notice(unit, kind.as_str(), refusal, Some(weapon.clone()), t_ms)
                }
                // 17b cut M1b.1: a refused arming names its weapon, as a refused
                // `FIRE` does, in the one shape phase 4 stages a seat's in.
                IntentKind::Condition(arming) => {
                    self.arming_refusal_notice(unit, arming, refusal, t_ms)
                }
                _ => self.refusal_notice(unit, kind.as_str(), refusal, None, t_ms),
            };
            if let Some(state) = self.minds.get_mut(&unit) {
                state.push_notice(notice);
            }
        }
        // 6. **17b cut M3.4** (doc 17 §8, §9; D-UM-32, D-UM-34): a core the
        // seal recorded that is gone drops the hull to the ROM HERE — after the
        // think in flight has landed as thought (D-UM-9: a think is atomic, and
        // it was taken while the core stood) and before the slots are read.
        // M6 clears the failed program; a fresh ROM think arms its own gun.
        // Under the landing's own hash:
        // `run_mind_landings` opened the lazy slot, and the drop's line takes
        // it (`Arena::event_hash`) as every line of the landing does.
        if self.core_lost(unit) {
            let hash = self.event_hash();
            self.drop_for_core(unit, t_ms, hash);
            self.traces
                .drop_to_rom(unit, think_seq, super::author_trace::RomDrop::Core);
        }
        // 7. **17b cut M1b.1** (doc 17 §5, D-UM-16): the hull's condition slots,
        // ONCE, after every intent of the landing has landed — never after each
        // landed `Arm` inside it: a mind lands its `Condition` BEFORE its `Lock`
        // in emission order, so a per-arming evaluation would read the old lock
        // and fire at a target the think had just dropped (17b's adversarial
        // F5). A hull with no slot returns at once.
        let _ = self.evaluate_conditions(unit, t_ms, ConditionTrigger::Landing, None, counters);
    }

    /// **The hash a think's own line carries** (17b cut M3.4; doc 16 §4's
    /// one hash per moment): the substep's hash when an earlier line of the
    /// substep has already TAKEN it ([`crate::arena::state::ActionHash::Taken`]),
    /// else a fresh state hash that stores nothing — under `Outside` AND under
    /// `Pending`. [`Arena::event_hash`] would fill a `Pending` slot (one a
    /// landing — or, until 17b cut M4.6, a reflex tick — earlier in the substep
    /// opened and journalled nothing under), and every later line of the substep would then carry
    /// the think's moment where it carried its own: a think opens no action
    /// hash ([`Arena::run_mind_thinks`]) and fills none either.
    fn think_line_hash(&self) -> u64 {
        match self.action_hash {
            crate::arena::state::ActionHash::Taken(hash) => hash,
            crate::arena::state::ActionHash::Pending | crate::arena::state::ActionHash::Outside => {
                crate::arena::hash::state_hash(self)
            }
        }
    }

    /// **Whether `unit`'s sealed core is gone** (17b cut M3.4; doc 17 §8, §9,
    /// §13; D-UM-32, D-UM-33, D-UM-34): the hull's seal recorded a core
    /// (`UnitState::seal`'s `core_class` above Class 0), the unit carries
    /// none now ([`Arena::core_of`]: the capability revoked by a
    /// `destroy_core`), and its mind row stands and is not already dropped.
    /// It reads a DORMANT row (a possessed hull's, 17b cut M3.5) as it reads
    /// a live one and does not look at the controller: its callers do.
    ///
    /// **The seal's `core_class` is the gate**: a hull sealed WITHOUT a core —
    /// the target, a coreless chassis, a never-cored fixture hull — runs the
    /// ROM from its first think as a dropped job (its mind class is `Zero`)
    /// and journals no drop, ever: its seal's `fallback` already says so. So
    /// does a core `degrade_core` lowered to Class 0 (it is present, and its
    /// class is `Zero`; declared).
    fn core_lost(&self, unit: ObjId) -> bool {
        self.unit(unit)
            .is_some_and(|state| state.seal.core_class > crate::arena::core::CoreClass::Zero)
            && self.core_of(unit).is_none()
            && self.minds.get(&unit).is_some_and(|state| !state.dropped())
    }

    /// **Drop a hull whose sealed core is gone** ([`Arena::core_lost`]; 17b
    /// cut M3.4; D-UM-32, D-UM-34), at `t_ms` under `hash` —
    /// `dropped = Some(DropCause::Core)`, ONE `mind_dropped` line (`core`, no
    /// fault word, no budget), and the captured ROM replaces the complete card
    /// and rule program, clearing old armings. Its next think may arm one slot.
    /// The landing arm hands it the landing's hash
    /// ([`Arena::event_hash`] under the lazy slot `run_mind_landings`
    /// opened), the think arm [`Arena::think_line_hash`]. Continuous orders
    /// hold — the motor's goal and the desired heading and speed are not
    /// touched. A possessed hull keeps its DORMANT row (17b cut M3.5,
    /// `Arena::suspend_mind`), so what keeps a pilot's hull out of here is
    /// its two callers, the think and landing arms, both gated on
    /// `Controller::Mind`: a dormant row is never dropped under its pilot,
    /// and a caller that is not so gated must gate itself.
    fn drop_for_core(&mut self, unit: ObjId, t_ms: i64, hash: u64) {
        if let Some(state) = self.minds.get_mut(&unit) {
            state.dropped = Some(DropCause::Core);
        }
        let _ = self.ledger.append(
            t_ms,
            EventKind::MindDropped {
                unit,
                cause: SmolStr::new_static(DropCause::Core.as_str()),
                fault: None,
                ops: None,
            },
            Vec::new(),
            hash,
        );
        if self.drop_runtime_to_rom(unit).is_err() {
            self.stop(crate::arena::state::StopReason::Void);
        }
    }

    /// **A mind's standing order that has run out** (17b cut M1a.5; D-UM-15):
    /// cleared at the landing whose millisecond reaches its deadline, with the
    /// reflex controller's own expiry's shape — one `order_expired` line under the
    /// substep's lazy hash, `orders_expired` counted, the squad marked
    /// (`Arena::note_order_expired`, the squad parsed back from the order's
    /// rendered word by `SquadId::parse`).
    fn expire_mind_order(&mut self, unit: ObjId, t_ms: i64, counters: &mut MotorCounters) {
        let Some(order) = self
            .minds
            .get(&unit)
            .and_then(|state| state.order.as_ref())
            .filter(|order| order.expires_ms <= t_ms)
            .cloned()
        else {
            return;
        };
        self.end_mind_order(unit, &order, t_ms, counters);
    }

    /// **A mind's `Support` whose charge has left the match** (17b cut M4.6b;
    /// the owner's STOP E ruling, 2026-10-09, "the arena ends it"): ended at
    /// the supporter's landing, beside [`Arena::expire_mind_order`] and after
    /// it, with the reflex controller's own end for a charge gone — ONE
    /// `order_expired` line of that line's exact shape (`support`, the
    /// order's squad) under the substep's lazy hash, `orders_expired`
    /// counted, the squad marked.
    ///
    /// **17b cut M5.2** (STOP E's remedy, the charge's `ObjId` on the order):
    /// the charge is "gone" when the row's [`MindState::order_charge`] — the
    /// unit the `Support` named as it landed — is no longer in
    /// [`Arena::units`]. It reads only hashed state (the charge is folded
    /// inside the order's `Support` bytes) and writes only when it ends: the
    /// filter runs FIRST and [`Arena::event_hash`] is taken only once it has
    /// passed — taken unconditionally it would fill the landing's lazy
    /// `Pending` slot and move every later line's hash in the substep. A
    /// charge that is a wreck, or that the supporter's own row reads dead,
    /// ends nothing here: a mind stops obeying that order on its own (the
    /// `order_end` divergence, doc 17 §17), and the arena keeps it standing.
    ///
    /// **The two residuals closed** (doc 17 §24, 2026-10-10; 17b cut M5.2):
    /// until this cut the order named its charge only as the supporter's own
    /// row (`MindObjective::Support { of }`, re-keyed when the order landed),
    /// and the filter read that row of the supporter's contact store — so a
    /// `Support` landed with `of: None` (the supporter did not hold its
    /// charge classified at the landing) named no row and stood blind until
    /// `expires_ms` when its charge left, and a supporter blind to its charge
    /// for a contact TTL (the row aged out of its store) was not ended either
    /// — reachable at the stock figures, the blind stand of
    /// `t1_a_support_keeps_station_and_stands_blind`. Reading the charge off
    /// the order ends both, as the reflex's order, which named the object,
    /// did. Latent in production, where no path removes a unit from the
    /// match.
    fn end_mind_support_whose_charge_left(
        &mut self,
        unit: ObjId,
        t_ms: i64,
        counters: &mut MotorCounters,
    ) {
        let Some(order) = self
            .minds
            .get(&unit)
            .filter(|state| {
                state
                    .order_charge
                    .is_some_and(|charge| self.unit(charge).is_none())
                    && state.order.as_ref().is_some_and(|order| {
                        matches!(order.objective, MindObjective::Support { .. })
                    })
            })
            .and_then(|state| state.order.clone())
        else {
            return;
        };
        self.end_mind_order(unit, &order, t_ms, counters);
    }

    /// **The arena's end of a mind's standing order** — the half
    /// [`Arena::expire_mind_order`] and
    /// [`Arena::end_mind_support_whose_charge_left`] share, each calling it
    /// only once its own filter has passed: the substep's lazy hash taken
    /// here and not before, the row's order cleared, `orders_expired`
    /// counted, ONE `order_expired` line at `t_ms` naming the order's squad
    /// and objective word, and the squad marked for its next signal.
    fn end_mind_order(
        &mut self,
        unit: ObjId,
        order: &FrameOrder,
        t_ms: i64,
        counters: &mut MotorCounters,
    ) {
        let hash = self.event_hash();
        if let Some(state) = self.minds.get_mut(&unit) {
            state.order = None;
            // 17b cut M4.6 (the owner's STOP C): the issuer goes with the
            // order — unfolded, so the hash taken above is unmoved by it —
            // and (17b cut M5.2, STOP E) the charge, folded only inside the
            // order's own bytes, which the hash above already read.
            state.order_issuer = None;
            state.order_charge = None;
        }
        self.clear_order_watch_member(unit);
        counters.orders_expired = counters.orders_expired.saturating_add(1);
        let _ = self.ledger.append(
            t_ms,
            EventKind::OrderExpired {
                unit,
                squad: order.squad.clone(),
                objective: SmolStr::new_static(order.objective.as_str()),
            },
            Vec::new(),
            hash,
        );
        if let Some(squad) = crate::deliberation::squad::SquadId::parse(&order.squad) {
            self.note_order_expired(squad);
        }
    }

    /// **Stage a card change for the next pulse** (17b cut M1b.5; doc 17 §11,
    /// §22's M1b): the task hands every `ArenaMsg::MindCard` it buffered to
    /// the arena BEFORE the step, in arrival order, and phase 2's head applies
    /// them (`Arena::apply_mind_cards`). Staging decides nothing: any hull
    /// may be named, and the arena answers it at the pulse.
    pub fn stage_card(&mut self, unit: ObjId, change: CardChange) {
        self.pending_cards.entry(unit).or_default().push(change);
    }

    /// **The staged card changes, applied to the arena's OWN rows** (17b cut
    /// M1b.5; doc 17 §11, D-UM-17 as amended, D-UM-43) at phase 2's head, at
    /// the pulse's start millisecond, under phase 2's one lazy hash — so the
    /// line is on the tape at the same millisecond at every cadence (the task
    /// stages a change before the pulse that reaches it), and every think from
    /// then on reads the new card.
    ///
    /// Per hull in unit-id order, per change in arrival order:
    ///
    /// 1. a hull that is not a [`Controller::Mind`] hull with a row is refused
    ///    `not_mind`;
    /// 2. the change is applied to a copy of the row's card — `Preset(name)`
    ///    is [`preset`] over `arena_cfg.reflex` (a name that is none is
    ///    refused `no preset named {name}`), `Set` is [`crate::arena::card::StockCard::set`],
    ///    validated again here (the world validated it on a scratch card; a
    ///    refusal is the `CardError`'s own sentence);
    /// 3. **the arena's own floor**: `condition_is_lawful` over a probe
    ///    `FireCondition` carrying the card's `min_odds_permille` (target
    ///    `BestHostile`, every aspect, unbounded) against
    ///    [`Arena::odds_floor_permille`] — a breach is refused `unlawful` and
    ///    the row keeps its old card. Vacuous under 17b cut M1b's bound (the
    ///    floor at most 100 per mille, a slider's `min_odds` at least 100) and
    ///    written anyway: a resumed arena's floor is its own, and 17b cut M3
    ///    widens the bound;
    /// 4. the card is written to the row and ONE `mind_card` line journalled
    ///    with its name, the sliders it moved and the whole card.
    ///
    /// A refusal is a `mind_card_refused` line and nothing else. The map is
    /// emptied whatever happens. Nothing a pilot reads changes here: the
    /// mind's next think reads the card (its slots re-arm on the new fire
    /// discipline then, `crate::arena::stock_mind::arming_for`).
    pub(crate) fn apply_mind_cards(&mut self) {
        if self.pending_cards.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_cards);
        let at = self.now_ms;
        for (unit, changes) in pending {
            for change in changes {
                let kind = self.apply_mind_card(unit, &change);
                let hash = self.event_hash();
                let _ = self.ledger.append(at, kind, Vec::new(), hash);
            }
        }
    }

    /// One change of `Arena::apply_mind_cards`: the line it journals, with
    /// the row written on success.
    fn apply_mind_card(&mut self, unit: ObjId, change: &CardChange) -> EventKind {
        let refused = |reason: String| EventKind::MindCardRefused {
            unit,
            reason: SmolStr::new(reason),
        };
        let minded = self
            .unit(unit)
            .is_some_and(|state| state.controller == Controller::Mind);
        let Some(runtime) = self.profiles.get(&unit).filter(|_| minded) else {
            return refused("not_mind".to_owned());
        };
        let Some(before) = runtime.card.stock().cloned() else {
            return refused("not_stock_card".to_owned());
        };
        let after = match change {
            CardChange::Preset(name) => match preset(name, &self.arena_cfg.reflex) {
                Some(card) => {
                    // 17b cut M2.2, ruling (b): a preset is pair-checked as a
                    // slider is — Rearguard's break_off_pct over a doctrine
                    // whose fight_home_pct reads above it is refused by name.
                    let shown = PRESETS
                        .iter()
                        .find(|preset| preset.eq_ignore_ascii_case(name))
                        .copied()
                        .unwrap_or("preset");
                    if let Err(error) = card.pair(shown) {
                        return refused(error.to_string());
                    }
                    card
                }
                None => return refused(format!("no preset named {name}")),
            },
            CardChange::Set { slider, value } => {
                let mut card = before.clone();
                if let Err(error) = card.set(slider, value) {
                    return refused(error.to_string());
                }
                card
            }
        };
        let probe = FireCondition {
            weapon: SmolStr::default(),
            target: SlotTarget::BestHostile,
            min_permille: after.min_odds_permille,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        };
        if condition_is_lawful(&probe, self.odds_floor_permille).is_err() {
            return refused("unlawful".to_owned());
        }
        let name = SmolStr::new(after.name_of(&self.arena_cfg.reflex));
        let changed = after
            .changed_from(&before)
            .into_iter()
            .map(SmolStr::new_static)
            .collect();
        let Some(own) = self.unit(unit) else {
            return refused("missing_hull".to_owned());
        };
        let ids = own
            .seal
            .weapons
            .iter()
            .map(|weapon| weapon.id.clone())
            .collect::<Vec<_>>();
        let channels = own.seal.channels.clone();
        let profile = self.classes.row(own.seal.class);
        // A staff stock-slider change is an adapter over the active authored
        // card. Preserve its schema, extra fields and ordered programs.
        let active = runtime.card.key();
        let Some(mut definition) = self.cards.definition(active.definition).cloned() else {
            return refused("missing_card_definition".to_owned());
        };
        let Some(mut selected) = definition.profiles.get(usize::from(active.index)).cloned() else {
            return refused("missing_card_profile".to_owned());
        };
        let values = match crate::arena::runtime_card::stock_values(&after) {
            Ok(values) => values,
            Err(error) => return refused(error.to_string()),
        };
        // The journal's human-readable stock label may include punctuation.
        // Keep the authored profile identifier stable while changing its values.
        selected.values.extend(values);
        definition.profiles = vec![selected];
        let mut catalog = self.cards.clone();
        let key = match catalog.admit(definition) {
            Ok(definition) => crate::arena::runtime_card::ProfileKey {
                definition,
                index: 0,
            },
            Err(error) => return refused(error.to_string()),
        };
        let card = match catalog.resolve(key) {
            Ok(card) => card,
            Err(error) => return refused(error.to_string()),
        };
        let Some(runtime) = self.profiles.get(&unit) else {
            return refused("missing_runtime".to_owned());
        };
        let prepared = match runtime.prepare_card(
            &catalog,
            unit,
            card,
            &profile,
            &ids,
            &channels,
            self.odds_floor_permille,
        ) {
            Ok(prepared) => prepared,
            Err(error) => return refused(error.to_string()),
        };
        let Some(runtime) = self.profiles.get_mut(&unit) else {
            return refused("missing_runtime".to_owned());
        };
        if let Err(error) = runtime.commit(&mut self.conditions, prepared) {
            return refused(error.to_string());
        }
        self.cards = catalog;
        self.constrain_profile_hardware(unit);
        self.cancel_profile_think(unit);
        EventKind::MindCard {
            unit,
            name,
            changed,
            card: key,
            stock: Some(after),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::test_support::{arena_of, pose, tank};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    /// One mind (red) and one pilot (blue) on the eight-cell window.
    fn minded() -> Arena {
        let mut red = tank(RED, PartyId::RED, pose(1_500, 4_500));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        arena_of(vec![red, tank(BLUE, PartyId::BLUE, pose(6_500, 4_500))])
    }

    /// Seal the weapon facts before arena admission, as real custody does.
    fn minded_with_weapons(weapons: Vec<crate::arena::state::WeaponSummary>) -> Arena {
        let mut red = tank(RED, PartyId::RED, pose(1_500, 4_500));
        red.controller = Controller::Mind;
        red.weapons = weapons;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("armed fixture terms seal");
        arena_of(vec![red, tank(BLUE, PartyId::BLUE, pose(6_500, 4_500))])
    }

    #[test]
    fn m6_staff_slider_preserves_authored_schema_extra_values_and_whole_rules() {
        use crate::arena::conditions::Rule;
        use crate::arena::profile_runtime::ProfileRuntime;
        use crate::arena::runtime_card::RuntimeProfiles;
        use crate::arena::schema::{AuthoredRule, CardField, CardValue, ThinkPredicate, Widget};

        let mut arena = minded_with_weapons(vec![crate::arena::test_support::cannon()]);
        let previous = arena.profiles[&RED].clone();
        let mut definition = arena
            .cards
            .definition(previous.card.key().definition)
            .expect("opening definition")
            .clone();
        definition.schema.sections[0].fields.push(CardField {
            id: "author_flag".into(),
            label: "Author flag".into(),
            widget: Widget::Toggle {
                default: true,
                help: "Preserved custom value".into(),
            },
        });
        definition.profiles[0]
            .values
            .insert("author_flag".into(), CardValue::Boolean(true));
        definition.profiles[0].rules = vec![AuthoredRule {
            rule: Rule::hold_rule(previous.weapons[0].clone(), Vec::new()),
            think: vec![ThinkPredicate::Hostiles { at_least: 2 }],
        }];
        let expected_schema = definition.schema.clone();
        let expected_name = definition.profiles[0].name.clone();
        let expected_rules = definition.profiles[0].rules.clone();
        let digest = arena.cards.admit(definition).expect("authored definition");
        let selected = RuntimeProfiles::bind(&arena.cards, digest, &[]).expect("selection");
        let class = arena.classes.row(arena.unit(RED).expect("hull").seal.class);
        let runtime = ProfileRuntime::new(
            &arena.cards,
            RED,
            selected.clone(),
            previous.rom.clone(),
            &class,
            &previous.weapons,
            &previous.channels,
            arena.odds_floor_permille,
        )
        .expect("authored runtime");
        arena
            .conditions
            .replace_profile_rules(RED, &runtime.program.native_rows(), class.slots)
            .expect("authored rule slots");
        arena.unit_mut(RED).expect("hull").seal.profiles = selected.clone();
        let before = runtime.token();
        let expected_program = runtime.program.rows.clone();
        arena.profiles.insert(RED, runtime);
        let memory = br#"{"author_flag":true}"#.to_vec();
        arena.minds.get_mut(&RED).expect("mind").memory = memory.clone();

        let event = arena.apply_mind_card(
            RED,
            &CardChange::Set {
                slider: "min_odds".into(),
                value: "70".into(),
            },
        );
        let EventKind::MindCard {
            card,
            stock: Some(stock),
            ..
        } = event
        else {
            panic!("staff edit should accept a complete stock subset: {event:?}");
        };
        let after = &arena.profiles[&RED];
        assert_eq!(
            arena
                .cards
                .definition(after.card.key().definition)
                .expect("edited definition")
                .profiles[0]
                .name,
            expected_name
        );
        assert_ne!(after.token(), before);
        assert_eq!(card, after.card.key());
        assert_eq!(stock.min_odds_permille, 700);
        assert_eq!(after.card.stock(), Some(&stock));
        assert_eq!(after.card.schema(), Some(&expected_schema));
        assert_eq!(
            after.card.get("author_flag"),
            Some(&CardValue::Boolean(true))
        );
        assert_eq!(after.card.rules(), Some(expected_rules.as_slice()));
        assert_eq!(after.program.rows, expected_program);
        assert_eq!(after.profiles, selected);
        assert_eq!(after.rom, previous.rom);
        assert_eq!(arena.minds[&RED].memory, memory);
        assert_eq!(
            arena.conditions.slots(RED)[0],
            after.program.native_rows()[0]
        );
    }

    /// **`NoMind` thinks nothing** and hands the memory back as it came.
    #[test]
    fn m1a5_no_mind_answers_nothing_and_echoes_the_memory() {
        let mut arena = minded();
        let Some(view) = arena.mind_view(RED) else {
            panic!("a mind has a view");
        };
        let result = NoMind.think(&ThinkJob {
            unit: RED,
            think_seq: 0,
            t_ms: 0,
            lands_ms: arena.substep_len_ms(),
            seed: arena.seed,
            dropped: false,
            profile: &crate::arena::core::ClassTable::STOCK.row(crate::arena::core::CoreClass::I),
            view: &Arc::new(view),
            memory: &[1, 2, 3],
            map: &arena.definition,
            card: &arena.profiles[&RED].card,
            program: &arena.profiles[&RED].program,
            profile_token: arena.profiles[&RED].token(),
        });
        assert_eq!(
            result,
            ThinkResult {
                diagnostics: None,
                rule_sample: None,
                intents: Vec::new(),
                memory: vec![1, 2, 3],
                fault: None,
                trace: Vec::new(),
            }
        );
        // 17b cut M1a.6: the default runner is the native stock mind now.
        assert!(
            format!("{:?}", arena.mind_runner).contains("NativeStockMind"),
            "Arena::new opens with the default runner"
        );
    }

    /// **The check-out schedules one think per mind** on its grid, and lays a
    /// row down; a pilot's hull gets neither.
    #[test]
    fn m1a5_the_check_out_schedules_one_think_per_mind() {
        let arena = minded();
        let thinks: Vec<(i64, DueKind)> = arena
            .due
            .iter()
            .filter(|event| event.kind.is_mind())
            .map(|event| (event.t_ms, event.kind.clone()))
            .collect();
        assert_eq!(
            thinks,
            vec![(
                arena.grid_at(RED, STOCK_THINK_MS),
                DueKind::MindThink {
                    unit: RED,
                    think_seq: 0
                }
            )]
        );
        assert_eq!(arena.minds.keys().copied().collect::<Vec<_>>(), vec![RED]);
    }

    /// **The buffer is bounded**: seventeen notices keep the newest sixteen.
    #[test]
    fn m1a5_the_buffer_drops_the_oldest_past_sixteen() {
        let mut state = MindState::default();
        for radius_m in 0..17_u32 {
            state.push_notice(FrameNotice::Emcon { on: true, radius_m });
        }
        assert_eq!(state.notices.len(), MIND_NOTICES);
        assert_eq!(
            state.notices.first(),
            Some(&FrameNotice::Emcon {
                on: true,
                radius_m: 1
            })
        );
    }

    /// **The hash probe** (`crate::arena::hash`, item 3c): every field of a
    /// mind row moves the state hash on its own — `think_seq`, `pending` (its
    /// presence, its seq and its millisecond), the memory (its bytes at one
    /// length), the notices (one more, and two `Miss`es that differ only in
    /// their reason), the range memory (an id and a figure), the order (its
    /// presence, each objective's payload, `hold_fire`, the band, the deadline
    /// and the squad) and `strikes` — and, **since 17b cut M3.6** (D-UM-23 as
    /// amended), the pending ANSWER part by part (each epoch, the intents'
    /// count and an intent's bytes, the memory's length and its bytes),
    /// `dropped` (its presence and its cause) and `seal_digest` (its bytes);
    /// 17b cut M3.2 carried them unfolded.
    /// One change to a mind row.
    type Probe = Box<dyn Fn(&mut MindState)>;

    /// A pending think with an empty answer — its presence, `think_seq` and
    /// `t_ms`, and (folded since 17b cut M3.6) no epoch, intent or memory.
    fn pending_at(think_seq: u64, t_ms: i64) -> PendingThink {
        PendingThink {
            think_seq,
            t_ms,
            epochs: [0; 3],
            profile_token: crate::arena::profile_runtime::ProfileToken {
                unit: RED,
                generation: 0,
                key: crate::arena::runtime_card::ProfileKey {
                    definition: mush_core::DefinitionDigest::from_bytes([1; 32]),
                    index: 0,
                },
            },
            lands_ms: t_ms + 250,
            period_ms: 1000,
            profile_class: crate::arena::core::CoreClass::I,
            rule_sample: None,
            intents: Vec::new(),
            memory: Vec::new(),
        }
    }

    #[test]
    fn m1a5_every_mind_field_moves_the_state_hash() {
        use crate::arena::perception::MindObjective;
        use crate::deliberation::order::RangeBand;
        let base = minded();
        let before = crate::arena::hash::state_hash(&base);
        let order = FrameOrder {
            identity: None,
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Engage {
                target: Some(ContactId(7)),
            },
            hold_fire: false,
            max_range_band: None,
            expires_ms: 30_000,
        };
        let miss = |reason: &str| FrameNotice::Miss {
            launch_cell: None,
            contact: None,
            reason: SmolStr::new(reason),
        };
        let probes: Vec<(&str, Probe)> = vec![
            ("think_seq", Box::new(|s| s.think_seq = 1)),
            (
                "pending",
                Box::new(|s| {
                    s.pending = Some(pending_at(0, 0));
                }),
            ),
            ("memory", Box::new(|s| s.memory = vec![1])),
            (
                "notices",
                Box::new(move |s| s.notices = vec![miss("moved")]),
            ),
            (
                "ranges",
                Box::new(|s| {
                    s.ranges.insert(ContactId(7), 450);
                }),
            ),
            ("order", {
                let order = order.clone();
                Box::new(move |s| s.order = Some(order.clone()))
            }),
            ("strikes", Box::new(|s| s.strikes = 1)),
            // 17b cut M3.6 (D-UM-23 as amended; doc 17 §13's list): the drop
            // and the seal's digest, folded after `strikes`.
            (
                "dropped",
                Box::new(|s| s.dropped = Some(DropCause::Strikes)),
            ),
            ("seal_digest", Box::new(|s| s.seal_digest = [0xa5; 32])),
        ];
        let mut seen = vec![before];
        for (field, probe) in &probes {
            let mut arena = base.clone();
            if let Some(state) = arena.minds.get_mut(&RED) {
                probe(state);
            }
            let after = crate::arena::hash::state_hash(&arena);
            assert!(!seen.contains(&after), "`{field}` did not move the hash");
            seen.push(after);
        }
        // Within a field: each part of it is folded, not only its presence.
        let with = |change: &dyn Fn(&mut MindState)| {
            let mut arena = base.clone();
            if let Some(state) = arena.minds.get_mut(&RED) {
                change(state);
            }
            crate::arena::hash::state_hash(&arena)
        };
        let pairs: Vec<(&str, u64, u64)> = vec![
            (
                "pending.t_ms",
                with(&|s| {
                    s.pending = Some(pending_at(0, 0));
                }),
                with(&|s| {
                    s.pending = Some(pending_at(0, 250));
                }),
            ),
            (
                "pending.think_seq",
                with(&|s| {
                    s.pending = Some(pending_at(0, 0));
                }),
                with(&|s| {
                    s.pending = Some(pending_at(1, 0));
                }),
            ),
            (
                "memory bytes",
                with(&|s| s.memory = vec![1]),
                with(&|s| s.memory = vec![2]),
            ),
            // 17b cut M3.6: every part of the pending answer, not only the
            // presence of a think.
            (
                "pending.epochs (helm)",
                with(&|s| s.pending = Some(pending_at(0, 0))),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        epochs: [1, 0, 0],
                        ..pending_at(0, 0)
                    });
                }),
            ),
            (
                "pending.epochs (sensors)",
                with(&|s| s.pending = Some(pending_at(0, 0))),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        epochs: [0, 1, 0],
                        ..pending_at(0, 0)
                    });
                }),
            ),
            (
                "pending.epochs (fire)",
                with(&|s| s.pending = Some(pending_at(0, 0))),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        epochs: [0, 0, 1],
                        ..pending_at(0, 0)
                    });
                }),
            ),
            (
                "pending.intents (count)",
                with(&|s| s.pending = Some(pending_at(0, 0))),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        intents: vec![IntentKind::Lock {
                            contact: ContactId(7),
                        }],
                        ..pending_at(0, 0)
                    });
                }),
            ),
            (
                "pending.intents (an intent's bytes)",
                with(&|s| {
                    s.pending = Some(PendingThink {
                        intents: vec![IntentKind::Lock {
                            contact: ContactId(7),
                        }],
                        ..pending_at(0, 0)
                    });
                }),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        intents: vec![IntentKind::Lock {
                            contact: ContactId(8),
                        }],
                        ..pending_at(0, 0)
                    });
                }),
            ),
            (
                "pending.memory (length)",
                with(&|s| s.pending = Some(pending_at(0, 0))),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        memory: vec![1],
                        ..pending_at(0, 0)
                    });
                }),
            ),
            (
                "pending.memory (bytes)",
                with(&|s| {
                    s.pending = Some(PendingThink {
                        memory: vec![1],
                        ..pending_at(0, 0)
                    });
                }),
                with(&|s| {
                    s.pending = Some(PendingThink {
                        memory: vec![2],
                        ..pending_at(0, 0)
                    });
                }),
            ),
            (
                "dropped (its cause)",
                with(&|s| s.dropped = Some(DropCause::Strikes)),
                with(&|s| s.dropped = Some(DropCause::Core)),
            ),
            (
                "seal_digest (its bytes)",
                with(&|s| s.seal_digest = [0xa5; 32]),
                with(&|s| s.seal_digest = [0x5a; 32]),
            ),
            (
                "a miss's reason",
                with(&|s| s.notices = vec![miss("moved")]),
                with(&|s| s.notices = vec![miss("cover")]),
            ),
            (
                "one notice more",
                with(&|s| s.notices = vec![miss("moved")]),
                with(&|s| s.notices = vec![miss("moved"), miss("moved")]),
            ),
            (
                "a range figure",
                with(&|s| {
                    s.ranges.insert(ContactId(7), 450);
                }),
                with(&|s| {
                    s.ranges.insert(ContactId(7), 500);
                }),
            ),
            (
                "a range id",
                with(&|s| {
                    s.ranges.insert(ContactId(7), 450);
                }),
                with(&|s| {
                    s.ranges.insert(ContactId(8), 450);
                }),
            ),
        ];
        for (part, a, b) in pairs {
            assert_ne!(a, b, "{part} is not folded");
        }
        let ordered = |change: &dyn Fn(&mut FrameOrder)| {
            let mut order = order.clone();
            change(&mut order);
            with(&move |s| s.order = Some(order.clone()))
        };
        let stock = ordered(&|_| {});
        for (part, change) in [
            (
                "the engage target",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Engage {
                        target: Some(ContactId(8)),
                    };
                }) as Box<dyn Fn(&mut FrameOrder)>,
            ),
            (
                "no target",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Engage { target: None };
                }),
            ),
            (
                "an advance",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Advance { to: (3, 4) };
                }),
            ),
            (
                "a screen",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Screen {
                        facing_mdeg: 90_000,
                    };
                }),
            ),
            (
                "a support",
                Box::new(|o: &mut FrameOrder| {
                    o.objective = MindObjective::Support {
                        of: Some(ContactId(7)),
                    };
                }),
            ),
            (
                "hold_fire",
                Box::new(|o: &mut FrameOrder| o.hold_fire = true),
            ),
            (
                "the band",
                Box::new(|o: &mut FrameOrder| o.max_range_band = Some(RangeBand::Close)),
            ),
            (
                "the deadline",
                Box::new(|o: &mut FrameOrder| o.expires_ms = 31_000),
            ),
            (
                "the squad",
                Box::new(|o: &mut FrameOrder| o.squad = SmolStr::new("red-2")),
            ),
        ] {
            assert_ne!(ordered(&change), stock, "{part} is not folded");
        }
        let advance =
            |to: (u16, u16)| ordered(&move |o| o.objective = MindObjective::Advance { to });
        assert_ne!(advance((3, 4)), advance((4, 3)), "an advance's cell");
        let screen = |facing_mdeg: u32| {
            ordered(&move |o| o.objective = MindObjective::Screen { facing_mdeg })
        };
        assert_ne!(screen(90_000), screen(91_000), "a screen's facing");
        let support =
            |of: Option<ContactId>| ordered(&move |o| o.objective = MindObjective::Support { of });
        assert_ne!(
            support(Some(ContactId(7))),
            support(None),
            "a support's charge"
        );
    }

    /// **`stage_notice`'s mind branch** (D-UM-8): a notice staged on a hull a
    /// mind flies reaches its buffer as well as the platform's pulse queue; a
    /// pilot's hull's buffer does not exist, and nothing is written for it.
    #[test]
    fn m1a5_a_staged_notice_reaches_the_minds_buffer() {
        let mut arena = minded();
        arena.stage_notice(RED, FrameNotice::PowerOnline);
        arena.stage_notice(BLUE, FrameNotice::PowerOnline);
        assert_eq!(
            arena.minds.get(&RED).map(|state| state.notices.clone()),
            Some(vec![FrameNotice::PowerOnline])
        );
        assert_eq!(
            arena.pending_notices.get(&RED),
            Some(&vec![FrameNotice::PowerOnline]),
            "the pulse queue still gets it"
        );
        assert!(!arena.minds.contains_key(&BLUE));
        let Some(view) = arena.mind_view(RED) else {
            panic!("a mind has a view");
        };
        assert_eq!(view.notices, vec![FrameNotice::PowerOnline]);
        assert!(
            arena
                .minds
                .get(&RED)
                .is_some_and(|state| state.notices.is_empty()),
            "drained into the view"
        );
    }

    /// **The wide window** the view tests field on: 56 by 56 cells of ground,
    /// the test window's zones, spots and graph — room for a contact 500 m off,
    /// ten steps of `CONTACT_RANGE_STEP_M`, where the eight-cell window rounds
    /// every plot to the one figure of 50 m.
    fn wide(units: Vec<crate::arena::state::CustodyUnit>) -> Arena {
        use crate::arena::state::{ArenaTerrain, CustodyOut, MatchMode};
        use crate::arena::test_support::{definition, ground, resolution};
        use crate::config::{MatchConfig, SensorsConfig};
        Arena::new(
            CustodyOut {
                definition: ArenaDefinition {
                    terrain: ArenaTerrain::uniform(56, 56, 1_000, ground())
                        .unwrap_or_else(|error| panic!("the wide window: {error:?}")),
                    ..definition()
                },
                units,
                pilots: Vec::new(),
                resolution: resolution(),
                seed: 2_350,
                mode: MatchMode::Skirmish,
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap_or_else(|error| panic!("the wide arena: {error:?}"))
    }

    /// The row id the view fixtures hold contact `k` under: descending with
    /// the range, so the nearest row has the **highest** id and the frame's own
    /// row order (by id) is the reverse of nearest-first; and every id written
    /// with a letter in it, so a reader that took the keyframe's sixteen
    /// figures for decimal would read nothing.
    fn view_row_id(k: u16) -> ContactId {
        ContactId(0xAF0 - 0x10 * u64::from(k))
    }

    /// **The view is the keyframe** (17b cut M1a.5's observer-rule assertion
    /// for the new surface; doc 17 §3, D-UM-4 read strictly). A mind flying the
    /// laser tank — sections with facets, heat, a pod and a receiver — with a
    /// held standing order holds fourteen contacts, seven of each side, a few
    /// of them unclassified, one of them read (a held condition and the fatal
    /// words beside it), and a lock; its view is compared, field by field,
    /// with the keyframe `status_frame_held` writes off the same
    /// `Arena::frame_for` against the same range memory.
    ///
    /// The contacts sit at whole steps of 50 m from 50 to 500 m, two pairs and
    /// a third pair tied — **the third at the twelfth and thirteenth rows**, so
    /// the truncation is decided by the `ContactId` tie-break — and their ids
    /// fall as their range grows ([`view_row_id`]), so the keyframe's own row
    /// order is the reverse of nearest-first: a view that took the frame's
    /// first twelve rows, or sorted by id alone, would hold the farthest. The
    /// expected side sorts the keyframe's rows by `(range_m, id)` with the id
    /// read back as the sixteen hex figures it is written as. (**Since 17b
    /// cut M4.5c** the cut seats the lock and the order's contact — rows 0
    /// and 3, among the nearest anyway — then keeps every row not known
    /// friendly before any friendly one: row 12, a blue, is kept as not
    /// known friendly, no longer by the tie-break, and rows 11 and 13 are
    /// dropped as friends, the hull's own side. So the same twelve rows are
    /// kept and every assertion holds unmodified, but the tie at the cut no
    /// longer decides it; the tie-break at a cut is pinned since by
    /// `m4_5c_the_rom_s_eight_rows_keep_every_contact_not_known_friendly_first`,
    /// whose fifth friend is a tie.)
    ///
    /// The own fields are the keyframe's keys, `ecm`, `receiver`, `heat`,
    /// `order` and every section's keys included; each contact is at the
    /// keyframe's resolution with its `aspect`, `odds`, `reads_dead`,
    /// `condition` and `fatal` the frame's. The view names no object but its
    /// own number, and a hidden unit's live move changes nothing in it until a
    /// pass. (**Since 17b cut M4.6b** one own field is not the keyframe's on
    /// a hull authored with no speed: `immobile`, once online — the owner's
    /// STOP B ruling, pinned by
    /// `m4_6b_a_hull_authored_with_no_speed_reads_immobile_on_the_view_once_online`.
    /// The laser tank here is authored with speed, so its `immobile`
    /// assertion holds unmodified.)
    #[test]
    fn m1a5_the_view_is_the_keyframe() {
        use crate::arena::condition::{Condition, ConditionBand, Reading};
        use crate::arena::perception::{Jammer, MindObjective};
        use crate::arena::state::{SeenPose, ShadowContact};
        use crate::arena::test_support::{laser_tank, line_tank};
        use crate::deliberation::order::RangeBand;
        use crate::match_::cockpit::status_frame_held;
        // (dx, dy) from the mind in centimetres, nearest first: 50, 100, 100,
        // 150, 200, 200, 250, 300, 300, 350, 400, 450, 450 and 500 m.
        let offsets: [(i64, i64); 14] = [
            (0, 5_000),
            (0, 10_000),
            (10_000, 0),
            (0, 15_000),
            (0, 20_000),
            (20_000, 0),
            (15_000, 20_000),
            (0, 30_000),
            (30_000, 0),
            (0, 35_000),
            (0, 40_000),
            (0, 45_000),
            (45_000, 0),
            (0, 50_000),
        ];
        let (x0, y0) = (1_500_i64, 1_500_i64);
        let mut red = laser_tank(RED, PartyId::RED, pose(x0, y0));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let mut units = vec![red];
        let mut others: Vec<ObjId> = Vec::new();
        for (k, (dx, dy)) in (0_u16..).zip(offsets) {
            let id = ObjId(10 + u32::from(k));
            let team = if k % 2 == 0 { PartyId::BLUE } else { PartyId::RED };
            units.push(line_tank(id, team, pose(x0 + dx, y0 + dy)));
            others.push(id);
        }
        let mut arena = wide(units);
        for (k, id) in (0_u16..).zip(&others) {
            let Some(target) = arena.unit(*id) else {
                panic!("a fielded unit");
            };
            let class = if k % 5 == 4 {
                Classification::Unknown
            } else {
                Classification::Classified { party: target.team }
            };
            let plot = SeenPose::of(target);
            arena.contacts.insert(
                RED,
                view_row_id(k),
                ShadowContact::sighted(*id, class, 0, plot),
            );
        }
        // Row 2 (100 m, a classified blue) read with its hull destroyed.
        if let Some(held) = arena.contacts.get_mut(RED, view_row_id(2)) {
            held.seen_condition = Some(Reading {
                at_ms: 0,
                condition: Condition::default().with_band(1, ConditionBand::Destroyed),
            });
        }
        let engaged = view_row_id(3);
        if let Some(state) = arena.unit_mut(RED) {
            state.power = Power::Online;
            state.lock = Some(view_row_id(0));
            state.jammer = Some(Jammer {
                radius_cm: 10_000,
                attenuation_cost: 25,
                signature_permille: 150,
            });
            state.receiver = Some(Receiver {
                budget: 30,
                budget_max: 40,
                reach_permille: 750,
            });
        }
        let order = FrameOrder {
            identity: None,
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Engage {
                target: Some(engaged),
            },
            hold_fire: true,
            max_range_band: Some(RangeBand::Close),
            expires_ms: 30_000,
        };
        if let Some(state) = arena.minds.get_mut(&RED) {
            state.order = Some(order.clone());
        }
        let ranges = arena
            .minds
            .get(&RED)
            .map(|state| state.ranges.clone())
            .unwrap_or_default();
        let Some(frame) = arena.frame_for(RED, arena.clock.tempo()) else {
            panic!("a frame");
        };
        let keyframe = status_frame_held(crate::arena_task::MatchId(1), &frame, &ranges);
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        let key = |name: &str| keyframe.get(name).cloned().unwrap_or_default();

        // The own fields.
        assert_eq!(key("unit"), serde_json::json!(view.unit));
        assert_eq!(
            key("callsign"),
            serde_json::json!(view.callsign.to_string())
        );
        assert_eq!(
            key("cell"),
            serde_json::json!(view.cell.map(|(x, y)| [x, y]))
        );
        assert_eq!(key("x_cm"), serde_json::json!(view.x_cm));
        assert_eq!(key("y_cm"), serde_json::json!(view.y_cm));
        assert_eq!(key("z_cm"), serde_json::json!(view.z_cm));
        assert_eq!(key("heading_mdeg"), serde_json::json!(view.heading_mdeg));
        assert_eq!(key("speed_cms"), serde_json::json!(view.speed_cms));
        assert_eq!(key("power"), serde_json::json!(view.power.as_str()));
        assert_eq!(key("blocked"), serde_json::json!(view.held));
        assert_eq!(key("wreck"), serde_json::json!(view.wreck));
        assert_eq!(key("immobile"), serde_json::json!(view.immobile));
        assert_eq!(key("sight_m"), serde_json::json!(view.sight_m));
        assert_eq!(key("t_ms"), serde_json::json!(view.t_ms));
        // The pod, the receiver and the heat: present, and the keyframe's.
        assert_eq!(view.ecm, Some(true), "the fixture's pod, switched on");
        assert_eq!(
            key("ecm"),
            serde_json::json!(view.ecm.map(|on| if on { "on" } else { "off" }))
        );
        let Some(receiver) = view.receiver else {
            panic!("the fixture's receiver");
        };
        assert_eq!(
            key("receiver"),
            serde_json::json!({
                "budget": receiver.budget,
                "budget_max": receiver.budget_max,
                "reach_percent": receiver.reach_permille / 10,
            })
        );
        let Some(heat) = view.heat else {
            panic!("the laser tank carries heat");
        };
        assert_eq!(
            key("heat"),
            serde_json::json!({
                "heat": heat.heat,
                "max": heat.max,
                "overheated": heat.overheated(),
            })
        );
        // The standing order: the mind's own, and the keyframe's object of it
        // — the target the designator of the mind's own row for it.
        assert_eq!(view.order, Some(order.clone()), "the mind's held order");
        let target = view
            .contacts
            .iter()
            .find(|contact| contact.id == engaged)
            .map(|contact| contact.designator.to_string());
        assert!(target.is_some(), "the engaged row is in the view");
        assert_eq!(
            key("order"),
            serde_json::json!({
                "squad": "red-1",
                "objective": order.objective.as_str(),
                "target": target,
                "to": null,
                "facing_mdeg": null,
                "of": null,
                "hold_fire": true,
                "max_range_band": "close",
                "expires_in_ms": order.expires_ms - view.t_ms,
                "identity": order.identity.as_deref(),
            })
        );
        assert_eq!(
            key("lock"),
            serde_json::json!(view.lock.map(|id| id.to_string()))
        );
        assert_eq!(view.lock, Some(view_row_id(0)), "the fixture locks");
        // Every section, key by key.
        let sections = key("sections").as_array().cloned().unwrap_or_default();
        assert_eq!(sections.len(), view.sections.len());
        assert!(view.sections.len() > 1, "the laser tank's five sections");
        for (row, section) in sections.iter().zip(&view.sections) {
            assert_eq!(
                row,
                &serde_json::json!({
                    "id": section.id.as_str(),
                    "condition": section.condition.as_str(),
                    "internals": section.internals.map(|(current, _)| current),
                    "internals_max": section.internals.map(|(_, max)| max),
                    "facets": section.facets.iter().map(|facet| serde_json::json!({
                        "id": facet.id.as_str(),
                        "armor": facet.armor,
                        "armor_max": facet.armor_max,
                    })).collect::<Vec<serde_json::Value>>(),
                }),
                "section {}",
                section.id
            );
        }
        let weapons = key("weapons");
        let rows = weapons.as_array().cloned().unwrap_or_default();
        assert_eq!(rows.len(), view.weapons.len());
        assert!(!view.weapons.is_empty(), "the laser tank is armed");
        for (row, weapon) in rows.iter().zip(&view.weapons) {
            assert_eq!(row.get("id"), Some(&serde_json::json!(weapon.id.as_str())));
            assert_eq!(
                row.get("bands_cm"),
                Some(&serde_json::json!(weapon.bands_cm))
            );
            assert_eq!(row.get("ready"), Some(&serde_json::json!(weapon.ready)));
            assert_eq!(row.get("rounds"), Some(&serde_json::json!(weapon.rounds)));
            // 17b cut M1b.3: the slot, `null` here — nothing is armed.
            assert_eq!(
                row.get("armed"),
                Some(
                    &weapon
                        .armed
                        .as_ref()
                        .map_or(serde_json::Value::Null, crate::match_::cockpit::arming_json)
                )
            );
        }
        // 17b cut M1b.3: the match's floor, the keyframe's own key.
        assert_eq!(
            key("odds_floor_permille"),
            serde_json::json!(view.odds_floor_permille)
        );

        // The contacts: the keyframe's fourteen, nearest first, the first
        // twelve — the twelve the cut keeps here (the doc's parenthesis).
        let mut rows = key("contacts").as_array().cloned().unwrap_or_default();
        assert_eq!(rows.len(), 14, "the keyframe carries every held contact");
        let range = |row: &serde_json::Value| {
            row.get("range_m")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(u64::MAX)
        };
        // `ContactId`'s `Display` is sixteen hex figures.
        let id = |row: &serde_json::Value| {
            let text = row
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| panic!("a row id: {row}"));
            u64::from_str_radix(text, 16).unwrap_or_else(|_| panic!("a hex row id: {text}"))
        };
        assert_eq!(
            rows.iter().map(id).collect::<Vec<_>>(),
            (0_u16..14)
                .rev()
                .map(|k| view_row_id(k).0)
                .collect::<Vec<_>>(),
            "the precondition: the keyframe's rows come farthest first"
        );
        rows.sort_by_key(|row| (range(row), id(row)));
        assert_eq!(
            rows.iter().map(range).collect::<Vec<_>>(),
            vec![
                50, 100, 100, 150, 200, 200, 250, 300, 300, 350, 400, 450, 450, 500
            ],
            "the precondition: whole steps, a tie at the twelfth and thirteenth rows"
        );
        rows.truncate(STOCK_CONTACT_ROWS);
        assert_eq!(view.contacts.len(), STOCK_CONTACT_ROWS);
        // Of the pair at 450 m, row 12 (due east, a blue) is kept and row 11
        // (due north, a red) dropped with the farthest, row 13 (a red). Until
        // 17b cut M4.5c the tie-break decided it (row 12 the lower id); since
        // then the cut keeps row 12 as not known friendly and drops rows 11
        // and 13 as friends, and the tie-break at a cut is pinned by
        // `m4_5c_the_rom_s_eight_rows_keep_every_contact_not_known_friendly_first`.
        let held: Vec<ContactId> = view.contacts.iter().map(|contact| contact.id).collect();
        assert!(held.contains(&view_row_id(12)), "{held:?}");
        assert!(!held.contains(&view_row_id(11)), "{held:?}");
        assert!(!held.contains(&view_row_id(13)), "{held:?}");
        assert!(
            view.contacts.windows(2).all(|pair| match pair {
                [a, b] => (a.range_m, a.id) < (b.range_m, b.id),
                _ => true,
            }),
            "nearest first, then by id"
        );
        let mut read = 0_usize;
        for (row, contact) in rows.iter().zip(&view.contacts) {
            let field = |name: &str| row.get(name).cloned().unwrap_or_default();
            assert_eq!(field("id"), serde_json::json!(contact.id.to_string()));
            assert_eq!(
                field("designator"),
                serde_json::json!(contact.designator.to_string())
            );
            assert_eq!(
                field("alias"),
                serde_json::json!(
                    contact
                        .alias
                        .map(|n| Designator::Provisional(n).to_string())
                )
            );
            assert_eq!(
                field("class"),
                serde_json::json!(crate::arena::view::class_word(contact.class))
            );
            assert_eq!(field("bearing_deg"), serde_json::json!(contact.bearing_deg));
            assert_eq!(
                field("relative_deg"),
                serde_json::json!(contact.relative_deg)
            );
            assert_eq!(field("band"), serde_json::json!(contact.band.as_str()));
            assert_eq!(field("range_m"), serde_json::json!(contact.range_m));
            assert_eq!(field("age_ms"), serde_json::json!(contact.age_ms));
            assert_eq!(
                field("cell"),
                serde_json::json!(contact.cell.map(|(x, y)| [x, y]))
            );
            assert_eq!(field("friendly"), serde_json::json!(contact.friendly));
            assert_eq!(
                field("condition"),
                serde_json::json!(contact.condition.map(|reading| serde_json::json!({
                    "table": reading.table,
                    "bands": reading.bands.letters(),
                    "age_ms": reading.age_ms,
                })))
            );
            assert_eq!(field("aspect"), serde_json::json!(contact.aspect.as_str()));
            assert_eq!(field("fatal"), serde_json::json!(contact.fatal));
            assert_eq!(contact.fatal.is_some(), contact.condition.is_some());
            if contact.condition.is_some() {
                read += 1;
                assert_eq!(contact.id, view_row_id(2), "the one read row");
                assert_eq!(contact.reads_dead, Some(true), "its hull read destroyed");
            }
            assert_eq!(
                field("reads_dead"),
                serde_json::json!(contact.reads_dead.unwrap_or(false))
            );
            assert!(contact.reads_dead.is_some(), "an armed hull reads it");
            let odds: Vec<(String, u64)> = field("odds")
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|odds| {
                    (
                        odds.get("weapon")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        odds.get("p_permille")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or_default(),
                    )
                })
                .collect();
            let mine: Vec<(String, u64)> = contact
                .odds
                .iter()
                .map(|(weapon, p)| (weapon.to_string(), u64::from(*p)))
                .collect();
            assert_eq!(odds, mine, "the odds of {}", contact.id);
        }
        assert_eq!(read, 1, "the read row is in the view, with its words");
        // **17b cut M1b.0** (doc 17 §3 and D-UM-4 as amended 2026-10-04): the
        // section-word tables the rows' `condition.table` indexes travel with
        // them, the keyframe's own, and the read row's index resolves in them
        // to its chassis's words.
        assert_eq!(
            key("scan_tables"),
            serde_json::json!(view.scan_tables),
            "the view's scan tables are the keyframe's"
        );
        assert_eq!(view.scan_tables.len(), 1, "the one read row's chassis");
        let table = view
            .contacts
            .iter()
            .find_map(|contact| contact.condition)
            .and_then(|reading| view.scan_tables.get(usize::from(reading.table)));
        assert!(
            table.is_some_and(|words| words.len() > 1),
            "the read row's table resolves to its chassis's words: {table:?}"
        );
        assert!(
            view.contacts
                .iter()
                .filter(|contact| !contact.odds.is_empty())
                .count()
                >= 1,
            "the nearest classified rows carry odds"
        );
        // Nothing finer than the keyframe, and no object but its own.
        let text = format!("{view:?}");
        assert!(!text.contains("ObjId"), "the view names no object: {text}");
        let rows = format!("{:?}", view.contacts);
        assert!(
            !rows.contains("mdeg") && !rows.contains("_cm"),
            "a contact at a resolution finer than the keyframe's: {rows}"
        );

        // A hidden unit's live move changes nothing until a pass.
        let moved = others.get(3).copied().unwrap_or(ObjId(13));
        if let Some(state) = arena.unit_mut(moved) {
            state.pose.x_cm += 2_000;
            state.pose.y_cm += 2_000;
        }
        let Some(again) = arena.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(
            again.contacts, view.contacts,
            "a live pose leaked into the view"
        );
    }

    /// **17b cut M4.6b — the one view field that is not the keyframe's** (doc
    /// 17 §3 and D-UM-4 as amended 2026-10-09, the owner's STOP B ruling in
    /// its "power up, then park" form): a mind on a hull authored with no
    /// speed (`max_speed_cms` nought, its motion NOT disabled — the range's
    /// `target_mk1` shape) reads `immobile` on its view once its power is
    /// `Online`, and not while it is `Off` or `Starting`, so its first thinks
    /// take the power arm; the keyframe built off the same frame reads it
    /// mobile at every power — the pilot's picture is untouched. A hull whose
    /// motion is disabled reads `immobile` on both at every power, and a hull
    /// with speed and motion reads mobile on both.
    #[test]
    fn m4_6b_a_hull_authored_with_no_speed_reads_immobile_on_the_view_once_online() {
        use crate::arena::test_support::laser_tank;
        use crate::match_::cockpit::status_frame_held;
        let mut red = laser_tank(RED, PartyId::RED, pose(1_500, 1_500));
        red.controller = Controller::Mind;
        red.mobility.max_speed_cms = 0;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let mut arena = wide(vec![red]);
        // (the keyframe's `immobile`, the view's) at the arena as it stands.
        let read = |arena: &mut Arena| -> (bool, bool) {
            let ranges = arena
                .minds
                .get(&RED)
                .map(|state| state.ranges.clone())
                .unwrap_or_default();
            let Some(frame) = arena.frame_for(RED, arena.clock.tempo()) else {
                panic!("a frame");
            };
            let keyframe = status_frame_held(crate::arena_task::MatchId(1), &frame, &ranges);
            let Some(view) = arena.mind_view(RED) else {
                panic!("a view");
            };
            let Some(keyed) = keyframe
                .get("immobile")
                .and_then(serde_json::Value::as_bool)
            else {
                panic!("the keyframe's `immobile`: {keyframe:?}");
            };
            (keyed, view.immobile)
        };
        let powers = [Power::Off, Power::Starting, Power::Online];
        for power in powers {
            if let Some(state) = arena.unit_mut(RED) {
                state.power = power;
            }
            assert_eq!(
                read(&mut arena),
                (false, power == Power::Online),
                "no authored speed at {power:?}: the keyframe mobile, the view immobile once online"
            );
        }
        // Motion disabled: immobile on both, at every power.
        for power in powers {
            if let Some(state) = arena.unit_mut(RED) {
                state.power = power;
                state.limits.motion_disabled = true;
            }
            assert_eq!(
                read(&mut arena),
                (true, true),
                "motion disabled at {power:?}"
            );
        }
        // Speed and motion: mobile on both, online.
        if let Some(state) = arena.unit_mut(RED) {
            state.power = Power::Online;
            state.limits.motion_disabled = false;
            state.mobility.max_speed_cms = 1_000;
        }
        assert_eq!(read(&mut arena), (false, false), "a hull that can close");
    }

    /// **A Class 0 mind among a held field** (17b cut M4.5c's fixture): the
    /// line tank `RED` at (15 m, 15 m) on [`wide`], a mind sealed under the
    /// fixture terms capped at Class 0 — the ROM's eight rows — holding one
    /// contact per entry `(dx, dy, team, classified)`: a line tank of `team`
    /// at that offset in centimetres, classified on its own side or
    /// `unknown`, held under [`view_row_id`]`(k)` for the `k`th entry, so the
    /// frame's own row order (by id) is the reverse of the entries'.
    fn rom_among(field: &[(i64, i64, PartyId, bool)]) -> Arena {
        use crate::arena::core::CoreClass;
        use crate::arena::state::{SeenPose, ShadowContact};
        use crate::arena::test_support::line_tank;
        let (x0, y0) = (1_500_i64, 1_500_i64);
        let mut red = line_tank(RED, PartyId::RED, pose(x0, y0));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms {
            class_cap: CoreClass::Zero,
            ..crate::match_::SealTerms::fixture()
        })
        .expect("the capped terms seal");
        let mut units = vec![red];
        for (k, (dx, dy, team, _)) in (0_u32..).zip(field) {
            units.push(line_tank(ObjId(10 + k), *team, pose(x0 + dx, y0 + dy)));
        }
        let mut arena = wide(units);
        for (k, (_, _, _, classified)) in (0_u16..).zip(field) {
            let id = ObjId(10 + u32::from(k));
            let Some(target) = arena.unit(id) else {
                panic!("a fielded unit");
            };
            let class = if *classified {
                Classification::Classified { party: target.team }
            } else {
                Classification::Unknown
            };
            let plot = SeenPose::of(target);
            arena.contacts.insert(
                RED,
                view_row_id(k),
                ShadowContact::sighted(id, class, 0, plot),
            );
        }
        assert_eq!(arena.mind_class(RED), CoreClass::Zero, "the ROM");
        assert_eq!(arena.mind_profile(RED).rows, 8, "Class 0's eight rows");
        arena
    }

    /// **`RED`'s frame rows**, held against its mind's range memory as
    /// [`Arena::mind_view`] holds them, in the frame's own order.
    fn held_rows(arena: &Arena) -> Vec<ContactRow> {
        let ranges = arena
            .minds
            .get(&RED)
            .map(|state| state.ranges.clone())
            .unwrap_or_default();
        let Some(frame) = arena.frame_for(RED, arena.clock.tempo()) else {
            panic!("a frame");
        };
        crate::arena::view::frame_contacts_held(&frame, &ranges)
    }

    /// **The cut as it stood until 17b cut M4.5c**, for the comparison: the
    /// held `rows` nearest-first by `(range_m, id)`, the first `keep` of
    /// them.
    fn nearest_cut(mut rows: Vec<ContactRow>, keep: usize) -> Vec<ContactRow> {
        rows.sort_by_key(|row| (row.range_m, row.id));
        rows.truncate(keep);
        rows
    }

    /// **A mind's row cut keeps every contact not known friendly first**
    /// (17b cut M4.5c; doc 17 §3 as amended 2026-10-09, the owner's ruling on
    /// STOP L: the lance lane's hulls, mustered among more friends than their
    /// rows, held every hostile classified and saw none). A Class 0 mind
    /// ([`rom_among`], eight rows) holds nine classified friends at 50 to
    /// 400 m, the fifth and sixth tied at 250 m, then two classified hostiles
    /// at 450 and 500 m and, farthest, a red hull whose side is `unknown` at
    /// 550 m — every id falling as the range grows, so the frame's own order
    /// is the reverse of nearest-first.
    ///
    /// * **The finding, as a precondition**: the old nearest-first cut keeps
    ///   eight friends and nothing else.
    /// * **The cut**: the view keeps the two hostiles, the unknown — a red
    ///   hull, so the cut reads the row's `friendly` (false while the side is
    ///   unresolved), never the truth — and the five nearest friends, the
    ///   fifth of them the tie at 250 m decided by the lower [`ContactId`]
    ///   (entry 5, due east; entry 4, due north, is cut with the three
    ///   farthest friends).
    /// * **The presentation**: the kept rows read nearest-first, strictly
    ///   ascending by `(range_m, id)`, each the frame's own row.
    /// * **No more contacts than rows**: a second field of six — a friend and
    ///   a hostile at 100 m, a hostile and a friend at 200 m (the ids set so
    ///   the hostile reads first at 100 m and the friend first at 200 m), an
    ///   `unknown` at 300 m and a friend at 400 m — gives the view the old
    ///   nearest-first list row for row: a view that does not truncate is
    ///   unchanged, and a friend and a hostile at one range read by range
    ///   then id, never friends last.
    ///
    /// `m1a5_the_view_is_the_keyframe` passes unmodified: the two rows its
    /// Class I cut drops are the hull's own side.
    #[test]
    fn m4_5c_the_rom_s_eight_rows_keep_every_contact_not_known_friendly_first() {
        // (dx, dy) in centimetres from the mind, nearest first: nine friends
        // at 50, 100, 150, 200, 250, 250, 300, 350 and 400 m; two hostiles at
        // 450 and 500 m; a red hull unresolved at 550 m.
        let field: [(i64, i64, PartyId, bool); 12] = [
            (0, 5_000, PartyId::RED, true),
            (10_000, 0, PartyId::RED, true),
            (0, 15_000, PartyId::RED, true),
            (20_000, 0, PartyId::RED, true),
            (0, 25_000, PartyId::RED, true),
            (25_000, 0, PartyId::RED, true),
            (0, 30_000, PartyId::RED, true),
            (35_000, 0, PartyId::RED, true),
            (0, 40_000, PartyId::RED, true),
            (45_000, 0, PartyId::BLUE, true),
            (30_000, 40_000, PartyId::BLUE, true),
            (33_000, 44_000, PartyId::RED, false),
        ];
        let mut arena = rom_among(&field);
        let held = held_rows(&arena);
        assert_eq!(held.len(), 12, "the frame holds every contact");
        let mut by_range: Vec<(u64, bool)> =
            held.iter().map(|row| (row.range_m, row.friendly)).collect();
        by_range.sort_unstable();
        assert_eq!(
            by_range,
            vec![
                (50, true),
                (100, true),
                (150, true),
                (200, true),
                (250, true),
                (250, true),
                (300, true),
                (350, true),
                (400, true),
                (450, false),
                (500, false),
                (550, false),
            ],
            "the precondition: nine friends nearer than three rows not known friendly"
        );
        let old = nearest_cut(held.clone(), 8);
        assert_eq!(
            old.iter().map(|row| row.id).collect::<Vec<_>>(),
            [0, 1, 2, 3, 5, 4, 6, 7].map(view_row_id).to_vec(),
            "the precondition: the old cut, the tie at 250 m by id"
        );
        assert!(
            old.iter().all(|row| row.friendly),
            "the finding: the nearest-first cut keeps eight friends and nothing else"
        );

        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        let kept: Vec<ContactId> = view.contacts.iter().map(|contact| contact.id).collect();
        assert_eq!(
            kept,
            [0, 1, 2, 3, 5, 9, 10, 11].map(view_row_id).to_vec(),
            "the two hostiles, the unknown and the five nearest friends, nearest-first"
        );
        assert_eq!(
            view.contacts
                .iter()
                .filter(|contact| !contact.friendly)
                .count(),
            3,
            "every row not known friendly is kept"
        );
        assert!(
            view.contacts
                .iter()
                .any(|contact| !contact.friendly && contact.class == Classification::Unknown),
            "the unresolved red hull is not known friendly"
        );
        assert!(
            view.contacts.windows(2).all(|pair| match pair {
                [a, b] => (a.range_m, a.id) < (b.range_m, b.id),
                _ => true,
            }),
            "nearest first, then by id"
        );
        for contact in &view.contacts {
            let Some(row) = held.iter().find(|row| row.id == contact.id) else {
                panic!("a kept row the frame does not hold: {}", contact.id);
            };
            assert_eq!(
                (
                    contact.designator,
                    contact.friendly,
                    contact.bearing_deg,
                    contact.range_m,
                    contact.band.as_str(),
                    contact.cell,
                ),
                (
                    row.designator,
                    row.friendly,
                    row.bearing_deg,
                    row.range_m,
                    row.band.as_str(),
                    row.cell,
                ),
                "the frame's own row"
            );
        }

        // No more contacts than rows: the old cut, row for row.
        let field: [(i64, i64, PartyId, bool); 6] = [
            (0, 10_000, PartyId::RED, true),
            (10_000, 0, PartyId::BLUE, true),
            (0, 20_000, PartyId::BLUE, true),
            (20_000, 0, PartyId::RED, true),
            (0, 30_000, PartyId::BLUE, false),
            (0, 40_000, PartyId::RED, true),
        ];
        let mut arena = rom_among(&field);
        let old = nearest_cut(held_rows(&arena), 8);
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        let kept: Vec<ContactId> = view.contacts.iter().map(|contact| contact.id).collect();
        assert_eq!(
            kept,
            [1, 0, 3, 2, 4, 5].map(view_row_id).to_vec(),
            "by range then id: the hostile first at 100 m, the friend first at 200 m"
        );
        assert_eq!(
            view.contacts
                .iter()
                .map(|contact| (contact.range_m, contact.friendly))
                .collect::<Vec<_>>(),
            vec![
                (100, false),
                (100, true),
                (200, true),
                (200, false),
                (300, false),
                (400, true),
            ],
            "a friend and a hostile at one range, either order by id"
        );
        assert_eq!(
            view.contacts
                .iter()
                .map(|contact| (
                    contact.id,
                    contact.designator,
                    contact.friendly,
                    contact.range_m
                ))
                .collect::<Vec<_>>(),
            old.iter()
                .map(|row| (row.id, row.designator, row.friendly, row.range_m))
                .collect::<Vec<_>>(),
            "a view that does not truncate is the old nearest-first list"
        );
    }

    /// **The cut without the seat** (17b cut M4.5c's first ruling alone),
    /// for the comparison: the held `rows` not known friendly first,
    /// each group nearest-first by `(range_m, id)`, the first `keep` of them.
    fn unseated_cut(mut rows: Vec<ContactRow>, keep: usize) -> Vec<ContactRow> {
        rows.sort_by_key(|row| (row.friendly, row.range_m, row.id));
        rows.truncate(keep);
        rows
    }

    /// `RED`'s standing order of `objective`, held on its mind's row as a
    /// squad's order is written (`Arena::rekey_order`'s product), so the
    /// frame — and the view — present it.
    fn order_on(arena: &mut Arena, objective: MindObjective) {
        let Some(state) = arena.minds.get_mut(&RED) else {
            panic!("the mind's row");
        };
        state.order = Some(FrameOrder {
            identity: None,
            squad: SmolStr::new("red-1"),
            objective,
            hold_fire: false,
            max_range_band: None,
            expires_ms: 30_000,
        });
    }

    /// **The kept rows' ids** of `view`, in the view's order.
    fn kept_ids(view: &MindView) -> Vec<ContactId> {
        view.contacts.iter().map(|contact| contact.id).collect()
    }

    /// **The cut seats the hull's own named contacts** (17b cut M4.5c; doc
    /// 17 §3 as amended 2026-10-09 by the owner's second ruling, "keep the
    /// charge first": the first ruling's cut alone, reviewed, left a
    /// supporter whose view held its rows of contacts not known friendly
    /// without its charge, a friend, and on the leash's blind arm). A Class
    /// 0 mind
    /// ([`rom_among`], eight rows):
    ///
    /// * **The lock and the charge**: a friend at 50 m, nine classified
    ///   hostiles at 100 to 500 m, and a friend at 550 m; the hull locks the
    ///   hostile at 500 m (eight hostiles nearer) and stands under
    ///   `Support { of }` the friend at 550 m (nine hostiles nearer). As a
    ///   precondition both the old nearest-first cut and the cut without
    ///   the seat ([`unseated_cut`]) drop both. The view keeps the lock and the charge
    ///   and, for the rest, the six nearest hostiles; the nearest friend, not
    ///   seated, is cut with the two farther hostiles.
    /// * **The Engage target**, separately (no lock): a friend at 50 m, nine
    ///   hostiles at 100 to 500 m and the ordered target, a tenth hostile, at
    ///   550 m — kept, with the seven nearest hostiles.
    /// * **The presentation**: each view reads nearest-first, strictly
    ///   ascending by `(range_m, id)`, and holds what the frame holds.
    /// * **No more contacts than rows**: five contacts with the farthest
    ///   locked and the next ordered — the old nearest-first list row for
    ///   row; the seat changes only which rows a view that truncates keeps.
    #[test]
    fn m4_5c_the_rom_seats_its_lock_and_its_order_s_contact_before_the_cut() {
        // (a) and (b): the lock and the Support charge, in one view.
        let field: [(i64, i64, PartyId, bool); 11] = [
            (0, 5_000, PartyId::RED, true),
            (10_000, 0, PartyId::BLUE, true),
            (0, 15_000, PartyId::BLUE, true),
            (20_000, 0, PartyId::BLUE, true),
            (0, 25_000, PartyId::BLUE, true),
            (30_000, 0, PartyId::BLUE, true),
            (0, 35_000, PartyId::BLUE, true),
            (40_000, 0, PartyId::BLUE, true),
            (0, 45_000, PartyId::BLUE, true),
            (30_000, 40_000, PartyId::BLUE, true),
            (33_000, 44_000, PartyId::RED, true),
        ];
        let mut arena = rom_among(&field);
        let (locked, charge) = (view_row_id(9), view_row_id(10));
        if let Some(state) = arena.unit_mut(RED) {
            state.lock = Some(locked);
        }
        order_on(&mut arena, MindObjective::Support { of: Some(charge) });
        let held = held_rows(&arena);
        assert_eq!(held.len(), 11, "the frame holds every contact");
        assert_eq!(
            held.iter()
                .filter(|row| !row.friendly && row.range_m < 500)
                .count(),
            8,
            "the precondition: eight hostiles nearer than the lock"
        );
        let old = nearest_cut(held.clone(), 8);
        let unseated = unseated_cut(held.clone(), 8);
        for cut in [&old, &unseated] {
            assert!(
                !cut.iter().any(|row| row.id == locked || row.id == charge),
                "the precondition: a cut without the seat drops the lock and the charge"
            );
        }
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(view.lock, Some(locked), "the view's lock");
        assert_eq!(
            view.order.as_ref().map(|order| order.objective),
            Some(MindObjective::Support { of: Some(charge) }),
            "the view's order"
        );
        assert_eq!(
            kept_ids(&view),
            [1, 2, 3, 4, 5, 6, 9, 10].map(view_row_id).to_vec(),
            "the six nearest hostiles, the lock and the charge, nearest-first"
        );
        assert!(
            view.contacts
                .iter()
                .any(|contact| contact.id == charge && contact.friendly),
            "the charge, a friend, is seated"
        );
        assert!(
            !kept_ids(&view).contains(&view_row_id(0)),
            "the nearest friend, not seated, is cut"
        );
        // (c): the Engage target, separately.
        let field: [(i64, i64, PartyId, bool); 11] = [
            (0, 5_000, PartyId::RED, true),
            (10_000, 0, PartyId::BLUE, true),
            (0, 15_000, PartyId::BLUE, true),
            (20_000, 0, PartyId::BLUE, true),
            (0, 25_000, PartyId::BLUE, true),
            (30_000, 0, PartyId::BLUE, true),
            (0, 35_000, PartyId::BLUE, true),
            (40_000, 0, PartyId::BLUE, true),
            (0, 45_000, PartyId::BLUE, true),
            (30_000, 40_000, PartyId::BLUE, true),
            (33_000, 44_000, PartyId::BLUE, true),
        ];
        let mut engaged = rom_among(&field);
        let target = view_row_id(10);
        order_on(
            &mut engaged,
            MindObjective::Engage {
                target: Some(target),
            },
        );
        let engaged_held = held_rows(&engaged);
        assert_eq!(
            engaged_held
                .iter()
                .filter(|row| !row.friendly && row.id != target)
                .count(),
            9,
            "the precondition: nine hostiles nearer than the target"
        );
        assert!(
            !unseated_cut(engaged_held.clone(), 8)
                .iter()
                .any(|row| row.id == target),
            "the precondition: the cut without the seat drops the target"
        );
        let Some(engaged_view) = engaged.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(engaged_view.lock, None, "no lock here");
        assert_eq!(
            kept_ids(&engaged_view),
            [1, 2, 3, 4, 5, 6, 7, 10].map(view_row_id).to_vec(),
            "the seven nearest hostiles and the ordered target, nearest-first"
        );
        // The presentation, both views.
        for (view, held) in [(&view, &held), (&engaged_view, &engaged_held)] {
            assert_eq!(view.contacts.len(), 8, "Class 0's eight rows");
            assert!(
                view.contacts.windows(2).all(|pair| match pair {
                    [a, b] => (a.range_m, a.id) < (b.range_m, b.id),
                    _ => true,
                }),
                "nearest first, then by id"
            );
            for contact in &view.contacts {
                let Some(row) = held.iter().find(|row| row.id == contact.id) else {
                    panic!("a kept row the frame does not hold: {}", contact.id);
                };
                assert_eq!(
                    (contact.designator, contact.friendly, contact.range_m),
                    (row.designator, row.friendly, row.range_m),
                    "the frame's own row"
                );
            }
        }

        // No more contacts than rows: the old cut, row for row, whatever is
        // seated.
        let field: [(i64, i64, PartyId, bool); 5] = [
            (0, 10_000, PartyId::RED, true),
            (10_000, 0, PartyId::BLUE, true),
            (0, 20_000, PartyId::BLUE, false),
            (20_000, 0, PartyId::RED, true),
            (0, 30_000, PartyId::BLUE, true),
        ];
        let mut few = rom_among(&field);
        if let Some(state) = few.unit_mut(RED) {
            state.lock = Some(view_row_id(4));
        }
        order_on(
            &mut few,
            MindObjective::Support {
                of: Some(view_row_id(3)),
            },
        );
        let old = nearest_cut(held_rows(&few), 8);
        let Some(view) = few.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(
            view.contacts
                .iter()
                .map(|contact| (contact.id, contact.friendly, contact.range_m))
                .collect::<Vec<_>>(),
            old.iter()
                .map(|row| (row.id, row.friendly, row.range_m))
                .collect::<Vec<_>>(),
            "a view that does not truncate is the old nearest-first list"
        );
    }

    /// **The seat's edges** (17b cut M4.5c, the owner's second ruling): what
    /// the seat does when the hull's own state names a contact it does not
    /// hold, names one contact twice, or names two where the class keeps one
    /// row. A Class 0 mind ([`rom_among`]) among a friend at 50 m and ten
    /// classified hostiles at 100 to 550 m:
    ///
    /// * **An order naming a contact the hull does not hold** seats nothing:
    ///   under `Engage { target }` and under `Support { of }` an id no row
    ///   carries — the view presents the order, and its rows are the cut
    ///   without the seat ([`unseated_cut`]) row for row (the eight nearest
    ///   hostiles; the friend cut). Only a held contact is seated.
    /// * **A lock that is the order's contact** is one seated row: the hull
    ///   locks the 550 m hostile and is ordered to engage it — the view keeps
    ///   it and, for the other seven rows, the seven nearest contacts not
    ///   known friendly, exactly the cut without the seat over the rest.
    /// * **One row** (a configured class table, `[tacsim.minds].classes`
    ///   lawfully as low as one row; the stock table's least is eight), on a
    ///   field of its own: a friendly `Support` charge at 50 m, three
    ///   hostiles at 100 to 200 m and a hostile lock at 300 m — the lock is
    ///   kept, not the nearer charge nor the nearest hostile: between seated
    ///   rows the rest of the key decides (not known friendly first, then
    ///   nearest). With two seated hostiles (the lock at 300 m, an `Engage`
    ///   target at 200 m) the nearer is kept.
    #[test]
    fn m4_5c_the_seat_holds_only_held_contacts_once_each_and_cuts_by_the_rest_of_the_key() {
        // A friend at 50 m, then ten hostiles at 100, 150, 200, 250, 300,
        // 350, 400, 450, 500 and 550 m.
        let field: [(i64, i64, PartyId, bool); 11] = [
            (0, 5_000, PartyId::RED, true),
            (10_000, 0, PartyId::BLUE, true),
            (0, 15_000, PartyId::BLUE, true),
            (20_000, 0, PartyId::BLUE, true),
            (0, 25_000, PartyId::BLUE, true),
            (30_000, 0, PartyId::BLUE, true),
            (0, 35_000, PartyId::BLUE, true),
            (40_000, 0, PartyId::BLUE, true),
            (0, 45_000, PartyId::BLUE, true),
            (30_000, 40_000, PartyId::BLUE, true),
            (33_000, 44_000, PartyId::BLUE, true),
        ];

        // (i) An order naming a contact the hull does not hold: the id a
        // twelfth entry would carry, which no row carries.
        let unheld = view_row_id(11);
        for objective in [
            MindObjective::Engage {
                target: Some(unheld),
            },
            MindObjective::Support { of: Some(unheld) },
        ] {
            let mut arena = rom_among(&field);
            order_on(&mut arena, objective);
            let held = held_rows(&arena);
            assert!(
                !held.iter().any(|row| row.id == unheld),
                "the precondition: the ordered contact is not held"
            );
            let Some(view) = arena.mind_view(RED) else {
                panic!("a view");
            };
            assert_eq!(view.lock, None, "no lock here");
            assert_eq!(
                view.order.as_ref().map(|order| order.objective),
                Some(objective),
                "the view presents the order"
            );
            assert_eq!(
                kept_ids(&view),
                unseated_cut(held, 8)
                    .iter()
                    .map(|row| row.id)
                    .collect::<Vec<_>>(),
                "an unheld contact seats nothing: the cut without the seat"
            );
            assert_eq!(
                kept_ids(&view),
                [1, 2, 3, 4, 5, 6, 7, 8].map(view_row_id).to_vec(),
                "the eight nearest hostiles; the friend cut"
            );
        }

        // (ii) A lock that is the order's contact: one seated row.
        let mut arena = rom_among(&field);
        let named = view_row_id(10);
        if let Some(state) = arena.unit_mut(RED) {
            state.lock = Some(named);
        }
        order_on(
            &mut arena,
            MindObjective::Engage {
                target: Some(named),
            },
        );
        let held = held_rows(&arena);
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(view.lock, Some(named), "the view's lock");
        assert_eq!(
            view.contacts
                .iter()
                .filter(|contact| contact.id == named)
                .count(),
            1,
            "the contact named twice is one seated row"
        );
        let rest: Vec<ContactRow> = held.into_iter().filter(|row| row.id != named).collect();
        let mut expected: Vec<ContactId> = unseated_cut(rest, 7).iter().map(|row| row.id).collect();
        expected.push(named);
        assert_eq!(
            kept_ids(&view),
            expected,
            "the seated row and, for the rest, the cut without the seat over seven rows"
        );
        assert_eq!(
            kept_ids(&view),
            [1, 2, 3, 4, 5, 6, 7, 10].map(view_row_id).to_vec(),
            "the seven nearest hostiles and the named one, nearest-first"
        );

        // (iii) One row: between seated rows the rest of the key decides.
        let field: [(i64, i64, PartyId, bool); 5] = [
            (0, 5_000, PartyId::RED, true),
            (10_000, 0, PartyId::BLUE, true),
            (0, 15_000, PartyId::BLUE, true),
            (20_000, 0, PartyId::BLUE, true),
            (0, 30_000, PartyId::BLUE, true),
        ];
        let one_row = || {
            let mut arena = rom_among(&field);
            arena.classes = serde_json::from_value(serde_json::json!({"0": {"rows": 1}}))
                .expect("a lawful table");
            assert_eq!(arena.mind_profile(RED).rows, 1, "the configured row");
            arena
        };
        let (charge, lock) = (view_row_id(0), view_row_id(4));
        let mut arena = one_row();
        if let Some(state) = arena.unit_mut(RED) {
            state.lock = Some(lock);
        }
        order_on(&mut arena, MindObjective::Support { of: Some(charge) });
        let held = held_rows(&arena);
        assert_eq!(
            (
                nearest_cut(held.clone(), 1)
                    .iter()
                    .map(|row| row.id)
                    .collect::<Vec<_>>(),
                unseated_cut(held, 1)
                    .iter()
                    .map(|row| row.id)
                    .collect::<Vec<_>>(),
            ),
            (vec![charge], vec![view_row_id(1)]),
            "the precondition: nearest keeps the charge, the cut without the seat the nearest hostile"
        );
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(
            kept_ids(&view),
            vec![lock],
            "the hostile lock, not the nearer friendly charge"
        );
        let mut arena = one_row();
        if let Some(state) = arena.unit_mut(RED) {
            state.lock = Some(lock);
        }
        order_on(
            &mut arena,
            MindObjective::Engage {
                target: Some(view_row_id(3)),
            },
        );
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        assert_eq!(
            kept_ids(&view),
            vec![view_row_id(3)],
            "two seated hostiles: the nearer"
        );
    }

    /// **17b cut M1b.3** (doc 17 §3's parity rule, D-UM-4): a mind reads its
    /// own armed slots and the match's floor exactly as its pilot would — the
    /// view's `weapons[i].armed` is the keyframe's `armed` object for an armed
    /// row (the arena's slot number, the target's kind and every term) and its
    /// `null` for an unarmed one, and `odds_floor_permille` is the keyframe's
    /// top-level key, the arena's captured floor and not the default.
    #[test]
    fn m1b3_the_view_carries_the_arming_and_the_floor_as_the_keyframe_does() {
        use crate::arena::conditions::{Arming, FireCondition, SlotTarget};
        use crate::arena::test_support::{cannon, line_tank};
        use crate::match_::cockpit::{arming_json, status_frame_held};
        let mut red = line_tank(RED, PartyId::RED, pose(1_500, 1_500));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let mut mg = cannon();
        mg.id = SmolStr::new("machine_gun");
        mg.mount_section = SmolStr::new("hull");
        red.weapons.push(mg);
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("seal the complete initial weapon rack");
        let mut arena = wide(vec![red, line_tank(BLUE, PartyId::BLUE, pose(1_500, 21_500))]);
        arena.odds_floor_permille = 50;
        let arm = Arming::Arm(FireCondition {
            weapon: SmolStr::new("machine_gun"),
            target: SlotTarget::BestHostile,
            min_permille: 100,
            aspects: 0b0110,
            min_rounds_left: 4,
            min_range_m: 0,
            max_range_m: 420,
        });
        assert_eq!(arena.resolve_arming(RED, &arm, 0), Ok(Some(1)));
        let ranges = arena
            .minds
            .get(&RED)
            .map(|state| state.ranges.clone())
            .unwrap_or_default();
        let Some(frame) = arena.frame_for(RED, arena.clock.tempo()) else {
            panic!("a frame");
        };
        let keyframe = status_frame_held(crate::arena_task::MatchId(1), &frame, &ranges);
        let Some(view) = arena.mind_view(RED) else {
            panic!("a view");
        };
        let rows = keyframe
            .get("weapons")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(rows.len(), 2);
        assert_eq!(view.weapons.len(), 2);
        for (row, weapon) in rows.iter().zip(&view.weapons) {
            assert_eq!(
                row.get("armed"),
                Some(
                    &weapon
                        .armed
                        .as_ref()
                        .map_or(serde_json::Value::Null, arming_json)
                ),
                "{}",
                weapon.id
            );
        }
        assert_eq!(
            view.weapons.first().map(|gun| gun.armed.clone()),
            Some(None)
        );
        assert_eq!(
            view.weapons
                .get(1)
                .and_then(|gun| gun.armed.as_ref())
                .map(|armed| (
                    armed.slot,
                    armed.target,
                    armed.min_permille,
                    armed.aspects,
                    armed.keeps,
                    armed.min_range_m,
                    armed.max_range_m
                )),
            Some((1, SlotTarget::BestHostile, 100, 0b0110, 4, 0, 420)),
            "the arena's own slot number and terms"
        );
        assert_eq!(
            rows.get(1).and_then(|row| row.pointer("/armed/target")),
            Some(&serde_json::json!("best"))
        );
        assert_eq!(
            rows.first().and_then(|row| row.get("armed")),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(view.odds_floor_permille, 50);
        assert_eq!(
            keyframe.get("odds_floor_permille"),
            Some(&serde_json::json!(view.odds_floor_permille))
        );
    }

    /// **The view holds its ranges as a pilot's rows do** (doc 17 §3; 16b cut
    /// B.4's `T-60` hysteresis on the mind's own memory). One contact due north
    /// of the mind: first plotted at 420 m, which rounds to 400 and settles
    /// the memory at 400; then at 428 m, which rounds to 450 with no memory and
    /// sits within five eighths of a step of the held 400 — the view keeps 400
    /// and so does the memory; then at 440 m, past the tolerance — the view
    /// reads 450 and the memory settles there. A view read against an empty
    /// memory, or one that never settled it, would read 450 at 428 m.
    #[test]
    fn m1a5_the_view_holds_its_ranges_against_the_minds_own_memory() {
        use crate::arena::state::{SeenPose, ShadowContact};
        use crate::arena::test_support::line_tank;
        use crate::arena::view::range_m_rounded;
        let mut red = crate::arena::test_support::scout(RED, PartyId::RED, pose(1_500, 1_500));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let mut arena = wide(vec![
            red,
            line_tank(BLUE, PartyId::BLUE, pose(1_500, 1_500 + 42_000)),
        ]);
        let Some(target) = arena.unit(BLUE) else {
            panic!("blue");
        };
        let plot = SeenPose::of(target);
        let row = ContactId(0xB1);
        arena.contacts.insert(
            RED,
            row,
            ShadowContact::sighted(
                BLUE,
                Classification::Classified { party: PartyId::BLUE },
                0,
                plot,
            ),
        );
        let replot = |arena: &mut Arena, north_cm: i64| {
            if let Some(held) = arena.contacts.get_mut(RED, row) {
                held.seen_at.y_cm = 1_500 + north_cm;
            }
        };
        let read = |arena: &mut Arena| {
            let Some(view) = arena.mind_view(RED) else {
                panic!("a view");
            };
            let shown = view
                .contacts
                .iter()
                .find(|contact| contact.id == row)
                .map(|contact| contact.range_m);
            let held = arena
                .minds
                .get(&RED)
                .and_then(|state| state.ranges.get(&row).copied());
            (shown, held)
        };
        assert_eq!(read(&mut arena), (Some(400), Some(400)), "420 m");
        replot(&mut arena, 42_800);
        assert_eq!(
            range_m_rounded(42_800, 50),
            450,
            "the precondition: 428 m rounds to 450 with no memory"
        );
        assert_eq!(read(&mut arena), (Some(400), Some(400)), "428 m, held");
        replot(&mut arena, 44_000);
        assert_eq!(read(&mut arena), (Some(450), Some(450)), "440 m, let go");
    }

    // -----------------------------------------------------------------------
    // 17b cut M1a.6: the memory (doc 17 §8, D-UM-30); 17b cut M2.2: over
    // `mush_mind`'s one codec
    // -----------------------------------------------------------------------

    /// A memory of `pairs`.
    fn memory_of(pairs: &[(&str, MemValue)]) -> Memory {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    /// **`memory_check` refuses by key, by count and by value**, and takes
    /// every shape doc 17 §8 allows at its bounds ([`STOCK_MEMORY_LIMITS`]): a
    /// 24-character key, 32 keys, `i64::MIN` and `i64::MAX`, a bool, unit, a
    /// 32-byte string and an array of 32 integers. Refused: an empty key and
    /// a 25-character one; 33 keys; a 33-byte string and an array of 33
    /// integers. **17b cut M2.2** (D-UM-30 as amended): the shapes a float,
    /// an object or a nested array once needed refusing are no `MemValue` at
    /// all, and the printable-ASCII rule refuses a key or a string holding a
    /// byte outside `0x20..=0x7E` — the variants are `mush_mind`'s.
    #[test]
    fn m1a6_memory_check_refuses_by_key_count_and_value() {
        let limits = STOCK_MEMORY_LIMITS;
        assert_eq!(
            (
                limits.keys,
                limits.key_chars,
                limits.string_bytes,
                limits.array_len
            ),
            (32, 24, 32, 32),
            "the stock class's limits"
        );
        let edge = memory_of(&[
            ("k".repeat(limits.key_chars).as_str(), MemValue::Int(1)),
            ("low", MemValue::Int(i64::MIN)),
            ("high", MemValue::Int(i64::MAX)),
            ("flag", MemValue::Bool(true)),
            ("none", MemValue::Unit),
            ("word", MemValue::Str("w".repeat(limits.string_bytes))),
            ("cells", MemValue::Ints(vec![7_i64; limits.array_len])),
        ]);
        assert_eq!(memory_check(&edge), Ok(()));
        let full: Memory = (0..limits.keys)
            .map(|k| {
                (
                    format!("k{k}"),
                    MemValue::Int(i64::try_from(k).unwrap_or(0)),
                )
            })
            .collect();
        assert_eq!(memory_check(&full), Ok(()));
        let mut over = full.clone();
        over.insert("one_more".to_owned(), MemValue::Unit);
        assert_eq!(memory_check(&over), Err(MemError::Count(limits.keys + 1)));
        let long = "k".repeat(limits.key_chars + 1);
        for key in [long.as_str(), "", "caf\u{e9}", "tab\there"] {
            assert_eq!(
                memory_check(&memory_of(&[(key, MemValue::Int(1))])),
                Err(MemError::Key(key.to_owned())),
                "{key:?}"
            );
        }
        for bad in [
            MemValue::Str("w".repeat(limits.string_bytes + 1)),
            MemValue::Str("caf\u{e9}".to_owned()),
            MemValue::Str("line\nbreak".to_owned()),
            MemValue::Ints(vec![7_i64; limits.array_len + 1]),
        ] {
            assert_eq!(
                memory_check(&memory_of(&[("bad", bad.clone())])),
                Err(MemError::Value("bad".to_owned())),
                "{bad:?}"
            );
        }
    }

    /// **The encoding is canonical and round-trips** (D-UM-30): the keys sorted
    /// whatever order they were written in, no whitespace; `decode(encode(m))
    /// == m`; empty bytes and bytes that do not decode are the empty map, which
    /// encodes as `{}`. 17b cut M2.2: the bytes are `mush_mind`'s encoder's,
    /// which also refuses (as the empty map, the arena's rule) the whitespace
    /// `serde_json` read.
    #[test]
    fn m1a6_a_memory_encodes_sorted_and_round_trips() {
        let mut one = Memory::new();
        one.insert("zeta".to_owned(), MemValue::Ints(vec![3, 4]));
        one.insert("alpha".to_owned(), MemValue::Int(-1));
        one.insert("mid".to_owned(), MemValue::Unit);
        let mut two = Memory::new();
        two.insert("mid".to_owned(), MemValue::Unit);
        two.insert("alpha".to_owned(), MemValue::Int(-1));
        two.insert("zeta".to_owned(), MemValue::Ints(vec![3, 4]));
        let bytes = encode_memory(&one);
        assert_eq!(bytes, encode_memory(&two));
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap_or_default(),
            r#"{"alpha":-1,"mid":null,"zeta":[3,4]}"#
        );
        assert_eq!(decode_memory(&bytes), one);
        assert_eq!(encode_memory(&decode_memory(&bytes)), bytes);
        assert_eq!(decode_memory(&[]), Memory::new());
        assert_eq!(decode_memory(b"not json"), Memory::new());
        assert_eq!(decode_memory(b"[1, 2]"), Memory::new());
        assert_eq!(decode_memory(br#"{"alpha": -1}"#), Memory::new());
        assert_eq!(encode_memory(&Memory::new()), b"{}".to_vec());
    }

    /// **17b cut M3.2: the seal's and the drop's values and words are pinned**
    /// (doc 17 §13, D-UM-45, D-UM-31, D-UM-32) — what 17b cut M3.3 builds the
    /// seal and the record on. [`SortieId::in_match`] puts the match in the
    /// high word and the object number in the low; [`SealRef::none`] is sortie
    /// 0, a zero digest, Class 0 recorded and thought at, the reflex (its
    /// engine word, decode-only: the conventions' D8), governed, open and
    /// (17b cut M4.6) not doctrine, and is the `Default`; every [`DropCause`]
    /// and [`SealEngine`] serialises as its `as_str` word and reads back from
    /// it; [`Arena::new`] lays each unit's custody seal's reference (`none()`
    /// until 17b cut M3.3), which reads doctrine on the fixture seal; and a
    /// FILLED seal (Class III recorded, Class II thought at, scripted,
    /// ungoverned, closed, not doctrine) on a unit survives a snapshot round
    /// trip field for field.
    #[test]
    fn m3_the_seal_and_the_drop_carry_their_values_and_words() {
        use crate::arena::core::CoreClass;
        use crate::arena::snapshot::Snapshot;
        assert_eq!(
            SortieId::in_match(7, ObjId(4)),
            SortieId::Packed((7_u64 << 32) | 4)
        );
        assert_eq!(
            SortieId::in_match(7, ObjId(4)).as_match_packed(),
            Some(30_064_771_076)
        );
        assert_eq!(
            SortieId::in_match(1, ObjId(u32::MAX)).as_match_packed(),
            Some((1_u64 << 32) | u64::from(u32::MAX)),
            "the object number fills the low word and never carries"
        );
        assert_eq!(SortieId::none(), SortieId::Packed(0));
        assert_eq!(SortieId::default(), SortieId::none());
        assert_eq!(
            serde_json::to_value(SortieId::Packed(9)).ok(),
            Some(serde_json::json!(9)),
            "transparent: the number"
        );
        let none = SealRef::none();
        assert_eq!(none.sortie, SortieId::Packed(0));
        assert_eq!(none.digest, [0_u8; 32]);
        assert_eq!(none.core_class, CoreClass::Zero);
        assert_eq!(none.class, CoreClass::Zero);
        assert_eq!(none.engine, SealEngine::Reflex);
        assert!(none.governed, "none() is governed");
        assert!(!none.closed, "none() is open");
        // 17b cut M4.6: no seal is no doctrine.
        assert!(!none.doctrine, "none() is not doctrine");
        assert_eq!(SealRef::default(), none);
        for cause in [DropCause::Strikes, DropCause::Core, DropCause::Abi] {
            assert_eq!(
                serde_json::to_value(cause).ok(),
                Some(serde_json::json!(cause.as_str()))
            );
            assert_eq!(
                serde_json::from_value::<DropCause>(serde_json::json!(cause.as_str())).ok(),
                Some(cause)
            );
        }
        assert_eq!(
            [DropCause::Strikes, DropCause::Core, DropCause::Abi].map(DropCause::as_str),
            ["strikes", "core", "abi"]
        );
        for engine in [SealEngine::Reflex, SealEngine::Native, SealEngine::Scripted] {
            assert_eq!(
                serde_json::to_value(engine).ok(),
                Some(serde_json::json!(engine.as_str()))
            );
            assert_eq!(
                serde_json::from_value::<SealEngine>(serde_json::json!(engine.as_str())).ok(),
                Some(engine)
            );
        }
        assert_eq!(
            [SealEngine::Reflex, SealEngine::Native, SealEngine::Scripted].map(SealEngine::as_str),
            ["reflex", "native", "scripted"]
        );

        // `Arena::new` lays `none()` on every unit — **until 17b cut M3.3**,
        // which writes each unit's custody seal's reference instead (the
        // fixture seal: sortie `in_match(0, unit)`, never `none()`), and a
        // mind's row carries that seal's digest.
        let mut arena = minded();
        assert!(arena.units.iter().all(|unit| unit.seal != SealRef::none()));
        assert!(
            arena
                .units
                .iter()
                .all(|unit| unit.seal.sortie == SortieId::in_match(0, unit.id))
        );
        assert_eq!(
            arena.minds.get(&RED).map(|row| row.seal_digest),
            arena.unit(RED).map(|unit| unit.seal.digest)
        );
        // 17b cut M4.6: the fixture seal is Standard Doctrine's identity, so
        // every unit's reference reads doctrine.
        assert!(arena.units.iter().all(|unit| unit.seal.doctrine));
        // A filled seal round-trips through a snapshot — its doctrine bit the
        // opposite of the fixture's, so the carry is what the decode reads.
        let original = arena.unit(RED).expect("own sealed hull").seal.clone();
        let filled = SealRef {
            sortie: SortieId::in_match(3, RED),
            digest: [0x5a; 32],
            engine: SealEngine::Scripted,
            governed: false,
            closed: true,
            doctrine: false,
            name: "Authored fixture".into(),
            ..original
        };
        let Some(red) = arena.units.iter_mut().find(|unit| unit.id == RED) else {
            panic!("red is fielded");
        };
        red.seal = filled.clone();
        arena.minds.get_mut(&RED).expect("mind").seal_digest = filled.digest;
        let Ok(snapshot) = Snapshot::encode(&arena) else {
            panic!("an arena between pulses encodes");
        };
        // The state is one JSON string (`snapshot.rs`' head): parse it.
        let state: Option<serde_json::Value> = snapshot
            .document
            .get("state")
            .and_then(serde_json::Value::as_str)
            .and_then(|text| serde_json::from_str(text).ok());
        let red_seal = state
            .as_ref()
            .and_then(|state| state.get("units"))
            .and_then(serde_json::Value::as_array)
            .and_then(|units| {
                units
                    .iter()
                    .find(|unit| unit.get("id") == serde_json::to_value(RED).ok().as_ref())
            })
            .and_then(|unit| unit.get("seal"))
            .cloned();
        assert_eq!(
            red_seal.as_ref().and_then(|seal| seal.get("engine")),
            Some(&serde_json::json!("scripted")),
            "{red_seal:?}"
        );
        assert_eq!(
            red_seal.as_ref().and_then(|seal| seal.get("doctrine")),
            Some(&serde_json::json!(false)),
            "{red_seal:?}"
        );
        assert_eq!(
            red_seal.as_ref().and_then(|seal| seal.get("profiles")),
            Some(&serde_json::to_value(&filled.profiles).expect("profiles encode")),
            "{red_seal:?}"
        );
        let Ok(back) = Snapshot::decode(&snapshot.document, (*arena.definition).clone()) else {
            panic!("and decodes");
        };
        assert_eq!(back.unit(RED).map(|unit| unit.seal.clone()), Some(filled));
        // 17b cut M3.3: blue carries its own custody seal (no longer `none()`),
        // whole, through the round trip.
        assert_eq!(
            back.unit(BLUE).map(|unit| unit.seal.clone()),
            arena.unit(BLUE).map(|unit| unit.seal.clone())
        );
    }

    /// **Doc 23 cut S.1 (the lead's L1; D-23-10): an IC epoch's id never collides with the
    /// arena's.** `ic(#4, 0)` and `in_match(0, #4)` — the very collision doc 23 §7 names, the
    /// claim book's ordinal 4 against match 0 — are different ids; each round-trips; the arena
    /// kind is still the bare number on the wire and the IC kind the tagged object; either form
    /// decodes, and nothing else does (an unknown field, a negative number, a string).
    #[test]
    fn s1_an_ic_epoch_id_never_collides_with_the_arenas() {
        let ic = SortieId::ic(ObjId(4), 0);
        let arena = SortieId::in_match(0, ObjId(4));
        assert_ne!(ic, arena, "the kind separates them");
        assert!(ic.is_ic() && !arena.is_ic());
        assert_eq!(ic.as_match_packed(), None);
        assert_eq!(arena.as_match_packed(), Some(4));
        assert_eq!(serde_json::to_value(arena).ok(), Some(serde_json::json!(4)));
        assert_eq!(
            serde_json::to_value(ic).ok(),
            Some(serde_json::json!({"ic": {"hull": 4, "at_ic": 0}}))
        );
        let later = SortieId::ic(ObjId(4), 1_234_567);
        for id in [
            ic,
            arena,
            later,
            SortieId::none(),
            SortieId::Packed(u64::MAX),
        ] {
            let text = serde_json::to_string(&id).expect("a sortie id encodes");
            assert_eq!(
                serde_json::from_str::<SortieId>(&text).ok(),
                Some(id),
                "{text}"
            );
            let value = serde_json::to_value(id).expect("a sortie id encodes");
            assert_eq!(serde_json::from_value::<SortieId>(value).ok(), Some(id));
        }
        assert_eq!(
            serde_json::from_str::<SortieId>(r#"{"ic":{"hull":9,"at_ic":-5}}"#).ok(),
            Some(SortieId::ic(ObjId(9), -5))
        );
        for bad in [
            r#"{"ic":{"hull":9,"at_ic":1,"x":0}}"#,
            r#"{"ic":{"hull":9},"x":0}"#,
            r#"{"arena":4}"#,
            "-4",
            r#""4""#,
        ] {
            assert!(serde_json::from_str::<SortieId>(bad).is_err(), "{bad}");
        }
        // Every arena id orders before every IC one, so a book keyed by the id lists the
        // arena's first and nothing about the arena's order moves.
        assert!(SortieId::Packed(u64::MAX) < ic);
        assert!(SortieId::in_match(0, ObjId(3)) < arena);
        assert!(ic < later);
    }

    // -----------------------------------------------------------------------
    // 17b cut M3.4 — the faults and the drop, journalled
    // -----------------------------------------------------------------------

    /// **A runner that faults every think** with the word its row names for
    /// the think's `think_seq` (the last word for any later one) — and thinks
    /// clean, answering nothing, with no words — recording whether each job
    /// was handed as dropped and at which class.
    #[derive(Debug)]
    struct Faulting {
        words: Vec<&'static str>,
        seen: std::sync::Mutex<Vec<(u64, bool, crate::arena::core::CoreClass)>>,
    }

    impl MindRunner for Faulting {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            if let Ok(mut seen) = self.seen.lock() {
                seen.push((job.think_seq, job.dropped, job.profile.class));
            }
            let at = usize::try_from(job.think_seq).unwrap_or(usize::MAX);
            let word = self.words.get(at).or_else(|| self.words.last()).copied();
            ThinkResult {
                diagnostics: None,
                rule_sample: None,
                intents: Vec::new(),
                memory: job.memory.to_vec(),
                fault: word.map(|word| ThinkFault {
                    cause: SmolStr::new_static(word),
                }),
                trace: Vec::new(),
            }
        }
    }

    /// A slot on `weapon` that never fires.
    fn never(weapon: &str) -> FireCondition {
        FireCondition {
            weapon: SmolStr::new(weapon),
            target: SlotTarget::Lock,
            min_permille: 1_000,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        }
    }

    /// Every line the arena has journalled, flushed.
    fn journal(arena: &mut Arena) -> Vec<crate::arena::ledger::LedgerEvent> {
        arena.ledger.flush();
        arena
            .ledger
            .take_chunks()
            .into_iter()
            .flat_map(|chunk| {
                String::from_utf8(chunk.bytes)
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect::<Vec<crate::arena::ledger::LedgerEvent>>()
            })
            .collect()
    }

    /// **17b cut M3.4: a faulted think journals its word, opens no action
    /// hash, and the third running drops the hull once** (doc 17 §8; D-UM-31,
    /// D-UM-32): red — a Class I core, two guns armed in S1 and S2, a
    /// held-shot stamp on S2 — faults `unknown`, `type`, `ops` through
    /// `Arena::mind_think`:
    ///
    /// * after every think the arena's action hash is still `Outside` (no
    ///   action hash was opened for a think);
    /// * three `mind_fault` lines at the thinks' milliseconds with their
    ///   `think_seq` and words in order, and ONE `mind_dropped { strikes,
    ///   fault: ops, ops: 1500 }` beside the third, under the third's hash;
    /// * `dropped` is `Some(Strikes)` and M6 replaces the failed program with
    ///   the captured empty ROM program, every held stamp gone; the next job
    ///   is handed dropped at Class 0 with a one-slot arming capacity;
    /// * a fourth fault is journalled and writes no second drop; a clean
    ///   think after it never un-drops the hull (nor clears the strikes), and
    ///   an arming of the second gun is refused `NoSlot` (the ROM's one);
    /// * an overflow with no fault is `intents`.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn m3_a_faulted_think_journals_its_word_opens_no_hash_and_the_third_drops() {
        use crate::arena::core::CoreClass;
        use crate::arena::state::ActionHash;
        let mut arena = minded_with_weapons(vec![
            crate::arena::test_support::cannon(),
            crate::arena::state::WeaponSummary {
                id: SmolStr::new("gun2"),
                ..crate::arena::test_support::cannon()
            },
        ]);
        for (n, weapon) in [(1_u8, "cannon"), (2, "gun2")] {
            assert_eq!(arena.conditions.arm(RED, never(weapon), 2), Ok(n));
        }
        arena.held_said.insert((RED, 2, ContactId(3)), 10);
        let runner = Arc::new(Faulting {
            words: vec!["unknown", "type", "ops"],
            seen: std::sync::Mutex::new(Vec::new()),
        });
        arena.mind_runner = runner.clone();
        let mut counters = MotorCounters::default();
        for n in 0..3_u64 {
            let t_ms = 1_000 * (i64::try_from(n).unwrap_or(0) + 1);
            arena.mind_think(RED, n, t_ms, &mut counters);
            assert_eq!(
                arena.action_hash,
                ActionHash::Outside,
                "think {n} opened a hash"
            );
        }
        let lines: Vec<(i64, EventKind, u64)> = journal(&mut arena)
            .into_iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    EventKind::MindFault { .. } | EventKind::MindDropped { .. }
                )
            })
            .map(|event| (event.t_ms, event.kind, event.hash))
            .collect();
        let fault = |think_seq: u64, cause: &'static str| EventKind::MindFault {
            unit: RED,
            think_seq,
            cause: SmolStr::new_static(cause),
        };
        assert_eq!(
            lines
                .iter()
                .map(|(t_ms, kind, _)| (*t_ms, kind.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1_000, fault(0, "unknown")),
                (2_000, fault(1, "type")),
                (3_000, fault(2, "ops")),
                (
                    3_000,
                    EventKind::MindDropped {
                        unit: RED,
                        cause: SmolStr::new_static("strikes"),
                        fault: Some(SmolStr::new_static("ops")),
                        ops: Some(1_500),
                    }
                ),
            ]
        );
        assert_eq!(
            lines.get(2).map(|line| line.2),
            lines.get(3).map(|line| line.2),
            "the drop rides the third fault's hash"
        );
        let row = arena.minds.get(&RED).cloned().unwrap_or_default();
        assert_eq!(row.dropped, Some(DropCause::Strikes));
        assert!(
            arena.conditions.slots(RED).is_empty(),
            "M6 installs the captured empty ROM program; no failed-mind rule survives"
        );
        assert_eq!(arena.conditions.slot_of(RED, "cannon"), None);
        assert_eq!(arena.conditions.slot_of(RED, "gun2"), None);
        assert!(
            arena.held_said.is_empty(),
            "all failed-mind held stamps are gone"
        );
        assert_eq!(arena.arming_capacity(RED), 1, "the ROM may arm one slot");
        assert_eq!(
            arena.profiles[&RED].card.key(),
            arena.profiles[&RED].rom.key()
        );
        assert!(arena.profiles[&RED].program.rows.is_empty());
        assert_eq!(arena.slot_capacity(RED), 2, "the hardware still holds two");
        assert_eq!(arena.mind_class(RED), CoreClass::Zero);
        assert_eq!(counters.mind_faults, 3);

        // A fourth fault: its line, and no second drop.
        arena.mind_think(RED, 3, 4_000, &mut counters);
        assert_eq!(
            runner
                .seen
                .lock()
                .map(|seen| seen.last().copied())
                .ok()
                .flatten(),
            Some((3, true, CoreClass::Zero)),
            "the next job is the ROM's, at Class 0"
        );
        let more: Vec<EventKind> = journal(&mut arena)
            .into_iter()
            .map(|event| event.kind)
            .filter(|kind| {
                matches!(
                    kind,
                    EventKind::MindFault { .. } | EventKind::MindDropped { .. }
                )
            })
            .collect();
        assert_eq!(more, vec![fault(3, "ops")], "dropped once");
        // A clean think never un-drops.
        arena.mind_runner = Arc::new(NoMind);
        arena.mind_think(RED, 4, 5_000, &mut counters);
        let row = arena.minds.get(&RED).cloned().unwrap_or_default();
        assert_eq!(row.dropped, Some(DropCause::Strikes));
        assert_eq!(row.strikes, MIND_STRIKES_TO_DROP);
        // Its captured ROM starts unarmed. Once it arms its first gun, the
        // second cannot widen the one-slot row, even though hardware holds two.
        assert_eq!(
            arena.resolve_arming(
                RED,
                &crate::arena::conditions::Arming::Arm(never("cannon")),
                5_000,
            ),
            Ok(Some(1))
        );
        assert_eq!(
            arena.resolve_arming(
                RED,
                &crate::arena::conditions::Arming::Arm(never("gun2")),
                5_000
            ),
            Err(crate::arena::intents::Refusal::NoSlot)
        );
        assert_eq!(arena.conditions.slots(RED).len(), 1);

        // An overflow with no fault is `intents`.
        #[derive(Debug)]
        struct Overflowing;
        impl MindRunner for Overflowing {
            fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
                ThinkResult {
                    diagnostics: None,
                    rule_sample: None,
                    intents: vec![IntentKind::Startup; MIND_INTENTS_MAX + 1],
                    memory: job.memory.to_vec(),
                    fault: None,
                    trace: Vec::new(),
                }
            }
        }
        let mut arena = minded();
        arena.mind_runner = Arc::new(Overflowing);
        arena.mind_think(RED, 0, 1_000, &mut counters);
        assert_eq!(
            journal(&mut arena)
                .into_iter()
                .map(|event| event.kind)
                .filter(|kind| matches!(kind, EventKind::MindFault { .. }))
                .collect::<Vec<_>>(),
            vec![EventKind::MindFault {
                unit: RED,
                think_seq: 0,
                cause: SmolStr::new_static(OVERFLOW_CAUSE),
            }]
        );
    }

    /// **17b cut M3.4: a sealed core that is gone drops the hull at its next
    /// think when no landing is pending; a hull sealed without a core never**
    /// (doc 17 §9; D-UM-32, D-UM-33, D-UM-34): red's core capability revoked
    /// (what `destroy_core`'s plan does) — its next think journals ONE
    /// `mind_dropped { core }` at the think's millisecond with no fault and no
    /// budget, opens no action hash, and is handed to the runner dropped at
    /// Class 0; a later think journals nothing. A red fielded with NO core is
    /// sealed `core_class` Zero: every think dropped at Class 0 from the
    /// first, and no drop line, ever.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn m3_a_sealed_core_gone_drops_at_the_next_think_and_a_coreless_seal_never() {
        use crate::arena::core::CoreClass;
        use crate::arena::state::ActionHash;
        let recording = || {
            Arc::new(Faulting {
                words: Vec::new(),
                seen: std::sync::Mutex::new(Vec::new()),
            })
        };
        let mut arena = minded();
        assert!(
            arena
                .unit(RED)
                .is_some_and(|state| state.seal.core_class == CoreClass::I)
        );
        let key = arena.unit(RED).and_then(|state| {
            state
                .unit
                .capabilities()
                .find(|key| key.id.as_str() == crate::definitions::unit::CORE_CAPABILITY)
                .cloned()
        });
        let Some(key) = key else {
            panic!("red's core");
        };
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::RevokeCapability { key }],
            None,
            None,
        )
        .expect("a plan");
        if let Some(state) = arena.unit_mut(RED) {
            state.unit = mush_tactics::apply(&state.unit, &plan, 0)
                .expect("the revoke applies")
                .0;
        }
        assert_eq!(arena.core_of(RED), None);
        let runner = recording();
        arena.mind_runner = runner.clone();
        let mut counters = MotorCounters::default();
        arena.mind_think(RED, 0, 1_000, &mut counters);
        assert_eq!(arena.action_hash, ActionHash::Outside);
        arena.mind_think(RED, 1, 2_000, &mut counters);
        let drops: Vec<(i64, EventKind)> = journal(&mut arena)
            .into_iter()
            .filter(|event| matches!(event.kind, EventKind::MindDropped { .. }))
            .map(|event| (event.t_ms, event.kind))
            .collect();
        assert_eq!(
            drops,
            vec![(
                1_000,
                EventKind::MindDropped {
                    unit: RED,
                    cause: SmolStr::new_static("core"),
                    fault: None,
                    ops: None,
                }
            )]
        );
        assert_eq!(
            arena.minds.get(&RED).and_then(|state| state.dropped),
            Some(DropCause::Core)
        );
        assert_eq!(
            runner
                .seen
                .lock()
                .map(|seen| seen.clone())
                .unwrap_or_default(),
            vec![(0, true, CoreClass::Zero), (1, true, CoreClass::Zero)],
            "the dropping think is already the ROM's"
        );

        // Sealed without a core: the ROM from the first think, no line.
        let mut red = tank(RED, PartyId::RED, pose(1_500, 4_500));
        red.controller = Controller::Mind;
        red.unit = crate::arena::test_support::tactical_unit(RED);
        let red = red.sealed();
        assert_eq!(red.seal.core_class, CoreClass::Zero);
        assert!(red.seal.fallback.is_some(), "its seal says so");
        let mut arena = arena_of(vec![red, tank(BLUE, PartyId::BLUE, pose(6_500, 4_500))]);
        let runner = recording();
        arena.mind_runner = runner.clone();
        for n in 0..3_u64 {
            let t_ms = 1_000 * (i64::try_from(n).unwrap_or(0) + 1);
            arena.mind_think(RED, n, t_ms, &mut counters);
        }
        assert!(
            journal(&mut arena)
                .iter()
                .all(|event| !matches!(event.kind, EventKind::MindDropped { .. })),
            "a hull sealed without a core never drops"
        );
        assert!(
            runner
                .seen
                .lock()
                .map(|seen| seen
                    .iter()
                    .all(|(_, dropped, class)| *dropped && *class == CoreClass::Zero))
                .unwrap_or(false),
            "every think the ROM's, at Class 0"
        );
        assert_eq!(arena.minds.get(&RED).and_then(|state| state.dropped), None);
    }

    /// `unit`'s core capability revoked — what `destroy_core`'s plan does.
    fn revoke_core(arena: &mut Arena, unit: ObjId) {
        let key = arena.unit(unit).and_then(|state| {
            state
                .unit
                .capabilities()
                .find(|key| key.id.as_str() == crate::definitions::unit::CORE_CAPABILITY)
                .cloned()
        });
        let Some(key) = key else {
            panic!("the hull's core");
        };
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::RevokeCapability { key }],
            None,
            None,
        )
        .expect("a plan");
        if let Some(state) = arena.unit_mut(unit) {
            state.unit = mush_tactics::apply(&state.unit, &plan, 0)
                .expect("the revoke applies")
                .0;
        }
        assert_eq!(arena.core_of(unit), None);
    }

    /// **17b cut M3.4: a think's lines fill no lazy slot** (doc 16 §4's one
    /// hash per moment): `Arena::mind_think` under a `Pending` slot — what a
    /// landing (or, until 17b cut M4.6, a reflex tick) earlier in the substep
    /// leaves when it journalled nothing — journals its `mind_fault` (and, for a sealed
    /// core gone, its `mind_dropped`) under a fresh state hash, the one an
    /// `Outside` twin's line carries, and leaves the slot `Pending`, so the
    /// substep's next line takes its own moment; under a `Taken` slot the
    /// line carries the substep's hash and the slot is untouched.
    /// (`Arena::event_hash` would have filled the `Pending` slot with the
    /// think's moment, and every later line of the substep would carry it.)
    #[test]
    fn m3_a_think_s_lines_fill_no_lazy_slot_and_share_a_taken_one() {
        use crate::arena::state::ActionHash;
        let faulting = || {
            Arc::new(Faulting {
                words: vec!["ops"],
                seen: std::sync::Mutex::new(Vec::new()),
            })
        };
        let line_hashes = |arena: &mut Arena| -> Vec<(String, u64)> {
            journal(arena)
                .into_iter()
                .filter(|event| {
                    matches!(
                        event.kind,
                        EventKind::MindFault { .. } | EventKind::MindDropped { .. }
                    )
                })
                .map(|event| (event.kind.as_str().to_owned(), event.hash))
                .collect()
        };
        let mut counters = MotorCounters::default();
        // The twin under `Outside`: the fresh hash the line takes.
        let mut outside = minded();
        outside.mind_runner = faulting();
        outside.mind_think(RED, 0, 1_000, &mut counters);
        assert_eq!(outside.action_hash, ActionHash::Outside);
        let fresh = line_hashes(&mut outside);
        assert_eq!(fresh.len(), 1);
        // Under `Pending`: the same hash, and the slot left `Pending`.
        let mut pending = minded();
        pending.mind_runner = faulting();
        pending.action_hash = ActionHash::Pending;
        pending.mind_think(RED, 0, 1_000, &mut counters);
        assert_eq!(
            pending.action_hash,
            ActionHash::Pending,
            "a faulted think filled the lazy slot"
        );
        assert_eq!(line_hashes(&mut pending), fresh);
        // Under `Taken`: the substep's hash, the slot untouched.
        let mut taken = minded();
        taken.mind_runner = faulting();
        taken.action_hash = ActionHash::Taken(0xA5A5);
        taken.mind_think(RED, 0, 1_000, &mut counters);
        assert_eq!(taken.action_hash, ActionHash::Taken(0xA5A5));
        assert_eq!(
            line_hashes(&mut taken),
            vec![("mind_fault".to_owned(), 0xA5A5)]
        );
        // The core drop at a think's head, under `Pending`: its line, the
        // slot left `Pending`, the hash an `Outside` twin's line carries.
        let core_drop = |slot: ActionHash| {
            let mut arena = minded();
            revoke_core(&mut arena, RED);
            arena.mind_runner = Arc::new(NoMind);
            arena.action_hash = slot;
            arena.mind_think(RED, 0, 1_000, &mut MotorCounters::default());
            let slot = arena.action_hash;
            (slot, line_hashes(&mut arena))
        };
        let (slot, dropped) = core_drop(ActionHash::Pending);
        assert_eq!(
            slot,
            ActionHash::Pending,
            "a core drop at a think filled the lazy slot"
        );
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped, core_drop(ActionHash::Outside).1);
    }

    /// **17b cut M3.4: the ROM takes back its one slot** (D-UM-32): red —
    /// Class I, the cannon its FIRST weapon and a second gun — has the
    /// failed mind's second gun in S1 and the cannon in S2 when three faults
    /// drop it. M6 clears both old rules and their held stamps at the drop.
    /// If a later mutable row occupies S1, the ROM's arming of the hull's
    /// first weapon takes it back — the second gun's slot and its held-shot stamp
    /// gone — where `ConditionTable::arm` alone would refuse `NoSlot` at
    /// every ROM think. An arming of the second gun (not a ROM gun) is still
    /// refused `NoSlot`, and a seated pilot's hull is never the ROM's (its
    /// `NoSlot` stands).
    #[test]
    fn m3_the_rom_takes_back_a_slot_the_failed_mind_left_on_another_gun() {
        use crate::arena::conditions::Arming;
        use crate::arena::intents::Refusal;
        let gun = |id: &str| crate::arena::state::WeaponSummary {
            id: SmolStr::new(id),
            ..crate::arena::test_support::cannon()
        };
        let mut arena =
            minded_with_weapons(vec![crate::arena::test_support::cannon(), gun("gun2")]);
        for (n, weapon) in [(1_u8, "gun2"), (2, "cannon")] {
            assert_eq!(arena.conditions.arm(RED, never(weapon), 2), Ok(n));
        }
        arena.held_said.insert((RED, 1, ContactId(3)), 10);
        arena.mind_runner = Arc::new(Faulting {
            words: vec!["ops"],
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let mut counters = MotorCounters::default();
        for n in 0..3_u64 {
            let t_ms = 1_000 * (i64::try_from(n).unwrap_or(0) + 1);
            arena.mind_think(RED, n, t_ms, &mut counters);
        }
        assert_eq!(
            arena.minds.get(&RED).and_then(|state| state.dropped),
            Some(DropCause::Strikes)
        );
        assert!(
            arena.conditions.slots(RED).is_empty(),
            "drop clears the failed program"
        );
        assert!(
            arena.held_said.is_empty(),
            "drop clears failed-program held stamps"
        );
        // A later mutable row may occupy the ROM's one slot (for example, a
        // direct diagnostic arming). Its own first gun must still reclaim it.
        assert_eq!(arena.conditions.arm(RED, never("gun2"), 1), Ok(1));
        arena.held_said.insert((RED, 1, ContactId(3)), 4_000);
        assert_eq!(arena.conditions.slot_of(RED, "gun2"), Some(1));
        // The ROM's arming of its first weapon takes S1 back.
        let rom = FireCondition {
            min_permille: 600,
            ..never("cannon")
        };
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(rom.clone()), 4_000),
            Ok(Some(1))
        );
        assert_eq!(
            arena
                .conditions
                .slots(RED)
                .iter()
                .flatten()
                .filter_map(crate::arena::conditions::Rule::fire)
                .collect::<Vec<_>>(),
            vec![&rom]
        );
        assert!(arena.held_said.is_empty(), "the second gun's stamp went");
        // The second gun is not the ROM's: it takes no slot from it.
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(never("gun2")), 4_000),
            Err(Refusal::NoSlot)
        );
        // A seated pilot's hull is not the ROM's: a full row refuses.
        let mut piloted = minded_with_weapons(vec![
            crate::arena::test_support::cannon(),
            gun("gun2"),
            gun("gun3"),
        ]);
        piloted.unit_mut(RED).expect("pilot hull").controller = Controller::Pilot;
        for weapon in ["gun2", "gun3"] {
            assert!(
                piloted
                    .resolve_arming(RED, &Arming::Arm(never(weapon)), 0)
                    .is_ok()
            );
        }
        assert_eq!(
            piloted.resolve_arming(RED, &Arming::Arm(never("cannon")), 0),
            Err(Refusal::NoSlot)
        );
    }

    /// **The Withdraw seed** (17b cut M4.3): `withdraw_seed` is Standard
    /// Doctrine's fifteen keys at their defaults with `state` `Withdraw` and
    /// `ordered` false — the bytes the ROM would write — and `doctrine_state_of`
    /// reads it back; an empty, absent or ill-shaped memory reads `Startup`.
    #[test]
    fn m4_3_the_withdraw_seed_is_the_stock_keys_in_withdraw_under_no_order() {
        use crate::arena::doctrine::MachineState;
        use crate::arena::stock_mind::{STOCK_MEMORY_KEY_NAMES, StockMemory};
        let seed = withdraw_seed();
        let stock = StockMemory::from(&decode_memory(&seed));
        assert_eq!(stock.state, MachineState::Withdraw);
        assert!(!stock.ordered);
        assert_eq!(
            stock,
            StockMemory {
                state: MachineState::Withdraw,
                ..StockMemory::default()
            }
        );
        let keys = decode_memory(&seed);
        assert_eq!(keys.len(), STOCK_MEMORY_KEY_NAMES.len(), "fifteen keys");
        for key in STOCK_MEMORY_KEY_NAMES {
            assert!(keys.contains_key(key), "{key}");
        }
        assert_eq!(doctrine_state_of(&seed), MachineState::Withdraw);
        assert_eq!(doctrine_state_of(&[]), MachineState::Startup);
        assert_eq!(doctrine_state_of(b"{}"), MachineState::Startup);
        assert_eq!(doctrine_state_of(&[3, 1, 4]), MachineState::Startup);
        assert_eq!(
            doctrine_state_of(&encode_memory(&Memory::from(StockMemory {
                state: MachineState::Engage,
                ..StockMemory::default()
            }))),
            MachineState::Engage
        );
    }

    /// **A standing memory seeded `Withdraw`** (17b cut M4.3): `seed_withdraw`
    /// keeps what the row saw and clears exactly the reflex's six (16b cuts
    /// Q.7 and E.4) with the state and `ordered`; an empty memory is the seed;
    /// a memory of another shape becomes the seed (nothing of it decodes).
    #[test]
    fn m4_3_seed_withdraw_keeps_the_row_but_for_the_doctrine_s_six_and_the_state() {
        use crate::arena::doctrine::MachineState;
        use crate::arena::stock_mind::StockMemory;
        let standing = StockMemory {
            state: MachineState::Engage,
            ordered: true,
            target: Some(crate::arena::state::ContactId(0x11)),
            stand_until_ms: Some(9_000),
            quiet_since_ms: Some(8_000),
            rallied: true,
            last_plot: Some(((1_500, 2_500), 5_000)),
            unheld_since_ms: Some(7_000),
            hunt: Some((3, 4)),
            ignore_until_ms: Some(6_000),
            ..StockMemory::default()
        };
        let seeded = StockMemory::from(&decode_memory(&seed_withdraw(&encode_memory(
            &Memory::from(standing.clone()),
        ))));
        assert_eq!(
            seeded,
            StockMemory {
                state: MachineState::Withdraw,
                ordered: false,
                stand_until_ms: None,
                quiet_since_ms: None,
                rallied: false,
                last_plot: None,
                unheld_since_ms: None,
                hunt: None,
                ..standing
            },
            "the state, `ordered` and the six cleared; the target and the ignore kept"
        );
        assert_eq!(seed_withdraw(&[]), withdraw_seed());
        assert_eq!(seed_withdraw(&[3, 1, 4]), withdraw_seed());
        assert_eq!(
            doctrine_state_of(&seed_withdraw(&[3, 1, 4])),
            MachineState::Withdraw
        );
    }

    /// **`Arena::machine_state`** (17b cut M4.3): a mind's hull reads the
    /// row's cached doctrine state (fresh: `Startup`; whatever its last
    /// landing wrote); a pilot's hull reads nothing, its dormant row never; a
    /// wreck under a machine reads `Disabled`. `adopt_mind` seeds only a
    /// release's `Withdraw`: a fixture's other state keeps the memory as it is.
    #[test]
    fn m4_3_machine_state_reads_the_cache_the_row_or_nothing() {
        use crate::arena::doctrine::MachineState;
        use crate::arena::stock_mind::StockMemory;
        let mut arena = minded();
        assert_eq!(arena.machine_state(RED), Some(MachineState::Startup));
        assert_eq!(arena.machine_state(BLUE), None, "a pilot's hull");
        if let Some(state) = arena.minds.get_mut(&RED) {
            state.memory = encode_memory(&Memory::from(StockMemory {
                state: MachineState::Engage,
                ..StockMemory::default()
            }));
            state.doctrine_state = doctrine_state_of(&state.memory);
        }
        assert_eq!(arena.machine_state(RED), Some(MachineState::Engage));
        arena.set_controller(RED, Controller::Pilot, MachineState::Patrol);
        assert_eq!(
            arena.machine_state(RED),
            None,
            "a dormant row is never read"
        );
        arena.set_controller(RED, Controller::Mind, MachineState::Patrol);
        assert_eq!(
            arena.machine_state(RED),
            Some(MachineState::Engage),
            "a fixture's hand-over seeds nothing"
        );
        arena.set_controller(RED, Controller::Pilot, MachineState::Patrol);
        arena.set_controller(RED, Controller::Mind, MachineState::Withdraw);
        assert_eq!(
            arena.machine_state(RED),
            Some(MachineState::Withdraw),
            "a release's Withdraw seeds the dormant row"
        );
        assert_eq!(
            arena
                .minds
                .get(&RED)
                .map(|state| doctrine_state_of(&state.memory)),
            Some(MachineState::Withdraw),
            "the cache is the memory's"
        );
        if let Some(unit) = arena.unit_mut(RED) {
            unit.limits.wreck = true;
        }
        assert_eq!(arena.machine_state(RED), Some(MachineState::Disabled));
        if let Some(unit) = arena.unit_mut(BLUE) {
            unit.limits.wreck = true;
        }
        assert_eq!(
            arena.machine_state(BLUE),
            None,
            "a pilot's wreck is nobody's"
        );
    }

    /// **17b cut M4.6c: the doctrine bit** (the conventions' D2 and D3). The
    /// bit is derived from typed immutable origin in M6; neither display text
    /// nor the runner engine grants or revokes Standard Doctrine provenance. A hull a mind flies whose seal
    /// is not Standard Doctrine's (here an explicit authored origin, rather
    /// than display-name text) reads
    /// [`crate::arena::doctrine::MachineState::Unknown`] off
    /// `Arena::machine_state` whatever its cache says, raises no tempo
    /// (`any_mind_approaching` ignores its cache) and is counted in the
    /// leader's `doing:` histogram under `unknown` (slot 6); the same row on a
    /// doctrine seal reads its cache, raises the tempo and is counted under
    /// `approach`. A disabled hull and a wreck read `Disabled` before the
    /// gate.
    #[test]
    fn m4_6c_a_mind_not_sealed_on_doctrine_reads_unknown_raises_no_tempo_and_counts_unknown() {
        use crate::arena::doctrine::MachineState;
        let sealed = || {
            let mut red = tank(RED, PartyId::RED, pose(1_500, 4_500));
            red.controller = Controller::Mind;
            red.reseal(&crate::match_::SealTerms::fixture())
                .expect("the fixture terms seal");
            red.seal
        };
        // The identity, field by field; the engine is not part of it.
        assert!(sealed().seal_ref().doctrine, "the fixture seal is doctrine");
        let renamed = |change: &dyn Fn(&mut MindSeal)| {
            let mut seal = sealed();
            change(&mut seal);
            seal.seal_ref().doctrine
        };
        assert!(
            renamed(&|s| s.mind = SmolStr::new("Display name")),
            "display text grants no provenance"
        );
        assert!(!renamed(&|s| s.origin =
            crate::arena::profile_seal::MindOrigin::Draft {
                character: RED,
                mind: 1,
                revision: 1,
                digest: mush_core::DefinitionDigest::from_bytes([9; 32]),
            }));
        assert!(renamed(&|s| s.engine = SealEngine::Scripted));
        assert!(renamed(&|s| s.engine = SealEngine::Reflex));
        // A row whose cache reads `Approach`, on a doctrine seal: read.
        let approaching = || {
            let mut arena = minded();
            if let Some(state) = arena.minds.get_mut(&RED) {
                state.doctrine_state = MachineState::Approach;
            }
            arena
        };
        let mut doctrine = approaching();
        assert_eq!(doctrine.machine_state(RED), Some(MachineState::Approach));
        assert!(doctrine.any_mind_approaching());
        let signal = doctrine
            .take_deliberation_signals()
            .into_iter()
            .find(|signal| signal.commander == RED)
            .expect("the mind's squad is reported");
        assert_eq!(
            signal.states,
            [0, 0, 1, 0, 0, 0, 0],
            "counted under approach"
        );
        // A display rename alone remains doctrine; tagged provenance decides.
        let mut other = approaching();
        let mut seal = sealed();
        seal.mind = SmolStr::new("Load III");
        seal.origin = crate::arena::profile_seal::MindOrigin::Draft {
            character: RED,
            mind: 1,
            revision: 1,
            digest: mush_core::DefinitionDigest::from_bytes([9; 32]),
        };
        if let Some(unit) = other.unit_mut(RED) {
            unit.seal = seal.seal_ref();
        }
        assert_eq!(other.machine_state(RED), Some(MachineState::Unknown));
        assert!(
            !other.any_mind_approaching(),
            "the cache of a mind not on doctrine raises no tempo"
        );
        let signal = other
            .take_deliberation_signals()
            .into_iter()
            .find(|signal| signal.commander == RED)
            .expect("the mind's squad is reported");
        assert_eq!(
            signal.states,
            [0, 0, 0, 0, 0, 0, 1],
            "counted under unknown"
        );
        assert_eq!(
            signal
                .members
                .iter()
                .find(|member| member.unit == RED)
                .map(|member| member.state),
            Some(MachineState::Unknown)
        );
        assert_eq!(crate::arena::command::state_words()[6], "unknown");
        // A disabled hull (not a wreck) is `Disabled` first, doctrine or not;
        // set back, it reads `Unknown` again.
        if let Some(unit) = other.unit_mut(RED) {
            unit.disabled = true;
        }
        assert_eq!(other.machine_state(RED), Some(MachineState::Disabled));
        if let Some(unit) = other.unit_mut(RED) {
            unit.disabled = false;
        }
        assert_eq!(other.machine_state(RED), Some(MachineState::Unknown));
        // A wreck is `Disabled` first, doctrine or not.
        if let Some(unit) = other.unit_mut(RED) {
            unit.limits.wreck = true;
        }
        assert_eq!(other.machine_state(RED), Some(MachineState::Disabled));
    }
}
