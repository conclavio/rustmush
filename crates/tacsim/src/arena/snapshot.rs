//! **The snapshot** (16b cut AF.1; 16c N16; doc 16 §3.1 "Determinism",
//! "Durability" and "Clock and lag", §4 "Arena state", §11's first "Before IC
//! play" item; D-16-3) — one running [`Arena`], between two pulses, as one
//! document, and the same [`Arena`] back out of it.
//!
//! Doc 16 §3.1 "Durability" made an OOC match RAM-only: a crash voids it. This
//! module is the first third of the resume that item asks for. It turns an arena
//! into a self-describing, versioned document and back, and proves the round
//! trip three ways; it ships nothing and boots nothing (the message that carries
//! the document to the world is 16b cut AF.2's, the boot that decodes it AF.3's).
//! Like the rest of the library it has no I/O, no wall clock and no store.
//!
//! # The document
//!
//! `{"record": "tacsim.snapshot", "format": 3, "arena": <definition id>,
//! "arena_definition_digest": <64 lowercase base-16 characters>,
//! "physics_digest": <the compiled locomotion table's digest>, "at_ms",
//! "ledger_seq", "chunk_seq", "state_hash": <the u64 as a decimal string>,
//! "state_digest": <64 lowercase base-16 characters>, "state": <one JSON
//! string>}`. There is no match id: the arena library has none, and the key the
//! world stores the document under names the match (16b cut AF.2).
//!
//! **Why the state is a string and not a nested object.** The store's Redis seam
//! writes a module document through `JSON.SET`, which parses it and serialises it
//! again, and the arena's contact ids, its seed and its hashes are full-range
//! `u64`s the store does not promise to keep above `i64::MAX`. A string comes
//! back byte for byte on every backend; `state_digest` — SHA-256 over the
//! string's bytes, checked before the string is parsed — proves it did; and
//! [`Snapshot::bytes`], the string's bytes, is exactly what the harness's bound
//! is asserted on (16b cut AF.2). `state_hash` is a decimal string for the same
//! reason; the head's small integers stay numbers.
//!
//! **Why the ground's digest is not enough.** [`ArenaDefinition::digest`] folds
//! the cells' tracked price and deliberately omits the wheeled and legged rows,
//! which `ArenaTerrain::priced` computes from the compiled `LocomotionTable`
//! (`state.rs`, "Why the terrain's per-row class table is NOT in it"). A build
//! that re-priced a class would resume a match priced by the old table with every
//! other check passing, so the head also records
//! `mush_atlas::physics::LOCOMOTION_DIGEST`, and a decode under another table is
//! refused by name ([`SnapshotError::Physics`]).
//!
//! **Determinism.** Every collection in the tree is ordered (`BTreeMap`,
//! `BTreeSet`, `Vec`, `VecDeque`), so a map writes its keys in the key type's
//! own order; struct fields serialise in declaration order; the one
//! `serde_json::Value` in the tree — an opaque unit's rules blob — writes its
//! object keys in `serde_json::Map`'s order, which is sorted in this workspace
//! (no `preserve_order`); and nothing stored is a float. **The state string is
//! therefore deterministic but not key-sorted**: a struct writes its fields in
//! declaration order and a map keyed by an enum in the enum's order (the class
//! caps write `walker` before `drone`), so nothing may read it as a canonical
//! sorted form. `crates/tacsim/tests/arena.rs`'
//! `af1_the_encoding_is_deterministic_and_integer` is the proof: one arena
//! encoded twice is one string, encoded, decoded and encoded again it is still
//! that string, and a walk of the parsed state finds no float — with a direct
//! assertion that `serde_json::Map` sorts, the canary for a `preserve_order`
//! that would make the rules blob's bytes follow insertion order. The six maps
//! whose key JSON cannot write as an object key —
//! [`crate::arena::state::EpochTable`]'s `(unit, channel)`,
//! [`crate::arena::state::DesignatorBook`]'s `(viewer, target)`,
//! [`Arena::spent_said`]'s `(told, target, section)`, [`Arena::held_losses`]'
//! `(viewer, contact)`, [`Arena::squad_watch`]'s
//! [`crate::deliberation::SquadId`], a struct, and (17b cut M1b.1)
//! [`Arena::held_said`]'s `(unit, slot, contact)` — are written as arrays of pairs
//! through `pairs`. `serde_json` refuses a struct key only at run time, and
//! nothing refuses one at compile time: the unit tests below pin the key form of
//! every keyed map the tree holds today, and whoever adds a keyed map adds it
//! there by hand; the resume tests' encodes of populated arenas are the backstop,
//! and only for a map their fixtures fill.
//!
//! # The three lists
//!
//! [`Arena`] has 54 fields (55 until 17b cut M4.6 deleted `reflex`), and
//! every one is placed by name, twice: in the
//! exhaustive destructuring that builds the encode's borrowed `ArenaStateRef`,
//! and in the exhaustive literal the decode builds the arena with. A field a
//! later cut adds is a compile error in both until it is put in one of these:
//!
//! - **Carried** (36): `units`, `epochs`, `contacts`, `designators`,
//!   `spent_said`, `due`, `ledger` (its [`LedgerState`]: counters, open chunk,
//!   closed queue, both SHA-256 running states), `clock` (its [`ClockState`]),
//!   `seed`, `resolution`, `pilots`, `motors` (17b cut M1a.1: the
//!   [`MotorTable`] whole — its rows, the path budget's window and spend — and
//!   never its path scratch, which moved there from the reflex controller's
//!   memory, `reflex`, carried until 17b cut M4.6 deleted it), `minds` (17b
//!   cut M1a.5: every unit mind's row — its think counter, the think in
//!   flight, its memory, its notice buffer, its range memory, its order and
//!   its strikes, since 17b cut M1b.1 its card, and since 17b cut M3.2 the
//!   think in flight's ANSWER (D-UM-23 as amended: its epochs, intents and
//!   memory, so a resumed landing lands what the think answered and nothing
//!   is recomputed), its drop and its seal's digest), `conditions` (its rows
//!   sized by each hull's core since 17b cut M3.2),
//!   `held_said` and `odds_floor_permille` (17b cut M1b.1: every hull's
//!   condition slots, the held-shot rows and the Compact floor the arena
//!   captured at check-out — carried, so a config reload between a check-out
//!   and a resume moves nothing), `classes` (17b cut M3.2: the class table
//!   captured at check-out, on the floor's argument — a resumed match keeps
//!   the table it was checked out under), `pending`,
//!   `shots`, `next_shot_seq`, `stop`, `now_ms`, `at_release`, `pending_catchup`,
//!   `held_losses`, `ended`, `arena_cfg`, `match_cfg`, `sensors_cfg`,
//!   `alert_range_cm`, `jam_radius_cm`, `squads`, `mode`, `hold`, and the two
//!   views `squad_watch` and `squads_expired` — integers, carried so a resumed
//!   squad's `SignalReason` stays true (`no_order` is chosen exactly when a
//!   squad was never reported) and a resume does not re-ask every squad at once.
//! - **Re-derived** (7), from the caller's ground — whose digest must be the
//!   recorded one — and from compiled code: `definition` (and the horizon index
//!   inside its terrain), `index` (built and rebuilt over the carried units, as
//!   [`Arena::new`] builds it), `chassis` (16c `T-105`: each unit's
//!   chassis-static words, `crate::arena::state::ChassisWords::table` over
//!   the decoded units, as `Arena::new` reads them off the check-out records —
//!   a pure function of the carried `unit` and `profile`), `min_cost`
//!   (`Arena::new`'s own line),
//!   `jam_fold` (empty: a memo, exact by 16b cut U.2's rule), `mind_runner`
//!   (17b cut M1a.5: `crate::arena::mind::default_mind_runner`, the call
//!   [`Arena::new`] makes, so a resumed match thinks with the runner a fresh
//!   one would), and (17b cut M3.5) `runners`, each hull's own runner by
//!   its seal — empty at decode; `crate::minds::install_sealed_runners`
//!   refills it from the decoded seals (`crate::arena_task::resume_arena`
//!   calls it), so a resumed scripted hull thinks on the runner its seal
//!   names. (`classes` was re-derived stock here in 17b cut M3.1 and is
//!   carried from 17b cut M3.2; 17b cut M3.1's `rated_class`, a test's lever
//!   until 17b cut M3.3's seal, was re-derived here until 17b cut M4.1
//!   deleted it.)
//! - **Reset** (12), views and per-pulse scratch: (17b cut M4.1) `awaiting`,
//!   `collected` and `unsettled`, the "answered" fact beside each mind row
//!   and the thinks collected and not yet written — [`Snapshot::encode`]
//!   refuses an arena whose `awaiting` holds a think, by name, and the decode
//!   lays all three down empty; `pending_cards` (17b cut
//!   M1b.5: the card changes `@mind/live` offered, applied and emptied at the next pulse's start, so
//!   empty between two pulses; not carried, so `SNAPSHOT_FORMAT` does not
//!   move), `lost_designators`,
//!   `spent_hits` and `pending_notices` (empty between two pulses — [`Snapshot::encode`]
//!   refuses an arena that holds any, never a panic), `last_frames` (empty: a
//!   resumed pilot's first frame is a keyframe), `action_hash` (outside),
//!   `substep_hashes` (empty), `watched` (`false`) and `last_observer_s`
//!   (`None`). (`mind_landings`, the thinks waiting to land, was reset here
//!   until 17b cut M3.2 retired it: each think's answer rides its row's
//!   `pending` and is carried with the row — D-UM-23 as amended 2026-10-04,
//!   the external review's finding 1.)
//!
//! **[`UnitState`] is carried whole by its derive.** It has no re-derived and no
//! reset field: AE's fit constants `cost_row` and `class`, the traffic hold, the
//! motion carries, `held`, heat, the lock, the per-substep `blocked` flag,
//! `linkdead`, the band cache, the callsign, every counter and (17b cut M3.2)
//! the seal reference `seal` (laid down empty until 17b cut M3.3) all go. A
//! `#[serde(skip)]` on one of its fields is the only way to drop one, which is
//! grep-able, and the unit tests below destructure it exhaustively so a new
//! field is a decision somebody makes on purpose.
//!
//! # The proofs
//!
//! 1. **Every byte**: `state_digest` over the string, checked before it is
//!    parsed.
//! 2. **Every hashed field**: [`crate::arena::hash::state_hash`] of the decoded
//!    arena equals the hash recorded at encode, checked on **every** decode —
//!    there is no second entry point that skips it, no stand-in ground and no
//!    decode "for the boot"; 16b cut AF.3 reads [`Snapshot::head`] first and
//!    decodes once, against the real ground.
//! 3. **Everything that steers**: a byte-identical tape, hash sequence and final
//!    state from a resume at every pulse of the scripted match, at three
//!    cadences, under the task's own wake rule, across a drop and a reclaim and
//!    in a squadded arena, with the ledger's running digests and the squad
//!    signals held equal too (`crates/tacsim/tests/arena.rs`, the `af1_*`
//!    tests), and a round carried in flight on the shipped centre landing the
//!    same at two cadences (`crates/tacsim/tests/shipped_scale.rs`).
//!
//! # The format
//!
//! [`SNAPSHOT_FORMAT`] moves whenever the carried tree's **shape** changes — a
//! field added, removed, renamed or re-typed anywhere in it, a key added to one
//! of the three carried configuration blocks included — or a carried field's
//! **meaning** does. `af1_the_snapshot_shape_is_pinned` catches the first kind
//! mechanically **only on the paths its two unstepped fixtures fill** — not
//! inside a collection they leave empty or an option they leave `None` at pulse
//! zero, which its doc lists; there the bump is the author's and the reviewer's
//! to see, and `deny_unknown_fields` on the carried types turns a stale document
//! into a refusal rather than a silent drop. The pin moves only together with
//! the format. A snapshot of another format is refused by name
//! ([`SnapshotError::Format`]) before anything else is read of it, and never
//! partially applied.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_atlas::physics::LOCOMOTION_DIGEST;
use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;

use crate::arena::catchup::{Catchup, ReleaseSnapshot};
use crate::arena::clock::{Clock, ClockState};
use crate::arena::command::{ExpiredSquads, SquadWatchTable};
use crate::arena::conditions::ConditionTable;
use crate::arena::core::ClassTable;
use crate::arena::hash::state_hash;
use crate::arena::index::SpatialIndex;
use crate::arena::intents::Intent;
use crate::arena::ledger::{Ledger, LedgerState};
use crate::arena::mind::{MindState, default_mind_runner};
use crate::arena::motor::MotorTable;
use crate::arena::state::{
    ActionHash, Arena, ArenaDefinition, ArenaError, ContactId, CostRow, Designator, DesignatorBook,
    DueHeap, EpochTable, HoldState, MatchEnd, MatchMode, Pilot, ShadowContacts, StopReason,
    UnitState, check_poses,
};
use crate::arena::step::FrameSlots;
use crate::arena::weapons::Shot;
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
use crate::definitions::resolution::ResolutionDef;
use crate::deliberation::squad::SquadRoster;

/// The document's `record` word: what a reader checks first, so a document of
/// any other kind is [`SnapshotError::Record`] and never a parse error.
pub const SNAPSHOT_RECORD: &str = "tacsim.snapshot";

/// Bumped whenever the state tree's SHAPE changes (a field added, removed,
/// renamed or re-typed anywhere in the carried tree — a key added to
/// `ArenaConfig`, `MatchConfig` or `SensorsConfig` included) or a carried field's
/// MEANING changes; the structural pin (`crates/tacsim/tests/arena.rs`,
/// `af1_the_snapshot_shape_is_pinned`) moves with it and never alone, and a
/// behaviour change moves neither.
///
/// **2 since 16b cut AI.1** (16c `B-84`): `ReflexUnit` gained `searched_from`
/// and `refused` (a `Refusal`), carried by the reflex rows' derive — a shape
/// change, so a format-1 document is refused by name rather than decoded with
/// the two fields defaulted and then voided on the hash.
///
/// **3 since 17b cut M1a.1** (doc 17 §4, D-UM-3): the driving fields left the
/// reflex rows for `Arena::motors` (a `MotorTable` carried whole, its scratch
/// skipped) and `ReflexMemory` lost `window`/`spent` — a shape change, so a
/// format-2 document is refused by name ([`SnapshotError::Format`]) before
/// any field of it is read, and a match resumed from one is voided by AF.3's
/// boot as every refused document is.
///
/// **4 since 17b cut M1a.4** (doc 17 §4, D-UM-3, D-UM-10): `DueKind::MotorTick`
/// on the heap and `IntentKind::Goal` in `pending` are carried variants a
/// format-3 reader cannot decode — a shape change of two carried enums, which
/// the two unstepped `af1` fixtures leave empty, so the pin's digest and line
/// count did not move and only the format did.
///
/// **5 since 17b cut M1a.5** (doc 17 §3, §6, D-UM-2, D-UM-23): the `minds`
/// section, `Controller::Mind` and the two mind due kinds (`MindThink`,
/// `MindLand`) — a carried section and three carried variants a format-4
/// reader cannot decode. The `af1` pin gains the one `state.minds: object`
/// line each unstepped fixture fills.
///
/// **6 since 17b cut L.1** (doc 17 §4, D-UM-52): `Order.by` on every carried
/// standing order (the reflex rows' `order`) — a shape change a format-5 reader
/// cannot decode; the two unstepped `af1` fixtures hold no order at pulse zero,
/// so the pin's digest and line count did not move and only the format did.
///
/// **7 since 17b cut M1b.1** (doc 17 §5, D-UM-16, D-UM-17, D-UM-43):
/// `odds_floor_permille`, the `conditions` and `held_said` sections,
/// `MindState.card`, `IntentKind::Fire.slot`, `IntentKind::Condition`, the two
/// slot notice variants (`FrameNotice::SlotFired`, `SlotHeld`) and
/// `RefusalDetail::Floor` — a carried scalar, two carried sections, a carried
/// field and four carried variants a format-6 reader cannot decode; the `af1`
/// pin gains the three lines each unstepped fixture fills
/// (`state.odds_floor_permille`, `state.conditions`, `state.held_said`).
///
/// **8 since 17b cut M3.2** (doc 17 §6, §9, §13; D-UM-16, D-UM-23 as amended
/// 2026-10-04, D-UM-31, D-UM-45): `PendingThink`'s answer (`epochs`,
/// `intents`, `memory`), `MindState.dropped` and `seal_digest`,
/// `UnitState.seal`, `Arena.classes`, the condition rows sized by the core;
/// a format-7 document is refused by name ([`SnapshotError::Format`]). The
/// `af1` pin gains `state.classes` with its four rows and `state.units[].seal`
/// with its seven fields; the mind row's and the condition row's new shapes
/// are under the pin on no fixture there (their round trips are
/// `tests/arena.rs`' `m3_a_snapshot_carries_the_minds_with_their_pending_answers`
/// and `tests/minds.rs`' pending-answer resume).
///
/// **9 since 17b cut M4.6** (the deletion of the reflex controller; 17b §3's
/// M4 row, finding (iv); the owner's rulings of 2026-10-09): the `reflex`
/// section (`Arena.reflex`, the reflex rows and their frozen doctrine) is
/// gone, and with it two carried variants (`Controller::Reflex`,
/// `DueKind::ReflexTick`); `UnitState.seal` gains `SealRef::doctrine`
/// (whether the hull is sealed on Standard Doctrine, derived by identity at
/// the seal); `MindState` gains `order_issuer` (the owner's STOP C: the
/// issuer beside the mind's order, carried and never folded); and the
/// carried `Refusal` word `not_reflex` is `not_machine`. A format-8 document
/// is refused by name ([`SnapshotError::Format`]) before any field of it is
/// read, so **the first reload onto this build voids every match running
/// across it** `aborted(restart)`, "snapshot format 8 is not 9" (AF.3's
/// boot, as every refused document is); nothing migrates. The `af1` pin
/// walks `tests/arena.rs`' `af1_mind_squad_arena` in place of the reflex
/// squad fixture (deleted), given one motor row and one squad order with an
/// issuer LANDED on its mover before it is encoded (a mind's check-out lays
/// no motor row, where the reflex's adoption laid one per machine), so it
/// holds the mind rows, their due thinks, `seal.doctrine`, a motor row's nine
/// carried fields, a standing order's `FrameOrder` fields and `order_issuer`
/// as a number, and was re-blessed once.
///
/// **10 since 17b cut M5.2** (doc 17 §11, §12, §24's STOP E; the owner's
/// OQ-1 and OQ-3 of 2026-10-10 — the one format move the M5 cuts make):
/// three carried additions a format-9 reader cannot decode — `UnitState.seal`
/// gains `SealRef::profiles` (the sealed set, preset codes, never folded),
/// `UnitState` gains `card_profile` (the active profile and its switch's
/// stamp, folded only off its default) and `MindState` gains `order_charge`
/// (the charge of a held `Support`, folded inside the order's `Support`
/// bytes) — and two carried enums a variant each: `IntentKind::Profile` (a
/// seat's `MIND PROFILE`; continuous, so never queued in `pending`, but the
/// enum is the carried one) and `Refusal::Profile` (its word, `profile`, on
/// a carried `FrameNotice::Refused`). A format-9 document is refused by name
/// ([`SnapshotError::Format`]) before any field of it is read, so **the first
/// reload onto this build voids every match running across it**
/// `aborted(restart)`, "snapshot format 9 is not 10" (AF.3's boot, as every
/// refused document is); nothing migrates. The `af1` pin was re-blessed once
/// with it (509 lines to 556): it walks the three additions on every unit and
/// row, its squad arena's mover — sealed with a chosen set and switched off
/// its first profile, holding a `Support` with a `RangeBand`, its charge's
/// row (`of`) and its charge, and a motor row whose route, avoided cell,
/// search origin and remembered stop are filled — and an order on each of
/// the other three minds (an `Advance`, an `Engage` with its `target` and a
/// `Screen`), so the inner shapes doc 17 §24's index named as unwalked —
/// the route and avoid elements, `Refusal`, `RangeBand`, the other
/// objectives — are under it.
///
/// **11 since M5rules:** condition rows contain ordered typed fire, hold and
/// Sensors rules, with stable slot identity and bounded native predicates.
/// Format 10 is refused rather than migrated; saved in-flight matches restart.
/// **12 since M5r:** motor rows retain requested goals, outcomes and entry-zone
/// formation state. **13 since M6:** canonical card catalogs and profile runtimes,
/// sealed bindings, pending rule samples and schedules, own-hit stamps, the tape
/// cursor and the captured snapshot byte limit replace the old fixed-card shape.
/// Earlier formats, including 12 and 11, are refused by name before state decode;
/// in-flight matches restart rather than migrating.
/// **14 since M7:** filed origins carry the exact immutable Registry certificate
/// and saved revision identity, including when the effective execution is ROM.
/// There is no default certificate or migration from an unbound filed origin.
pub const SNAPSHOT_FORMAT: u32 = 14;

/// **One arena between two pulses, as a document** (16b cut AF.1).
///
/// The head's fields are copied out beside the document so a caller — the task
/// that sends it, the world that stores it — reads them without a parse.
pub struct Snapshot {
    /// The arena millisecond it was taken at: [`Arena::now_ms`].
    pub at_ms: i64,
    /// [`crate::arena::hash::state_hash`] of the arena at encode — what the
    /// decode's own hash must equal.
    pub state_hash: u64,
    /// The ledger's last event sequence ([`Ledger::seq`]).
    pub ledger_seq: u64,
    /// The ledger's last closed chunk ([`Ledger::last_closed_seq`]).
    pub chunk_seq: u64,
    /// The ground's id ([`ArenaDefinition::id`]).
    pub arena: SmolStr,
    /// [`digest_text`] of [`ArenaDefinition::digest`].
    pub definition_digest: String,
    /// The document itself (this module's head has its shape).
    pub document: serde_json::Value,
}

/// By hand: the head and the state's size, **never the document**. `ArenaOut`
/// derives `Debug`, and a test failure message must not dump a megabyte of state.
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot")
            .field("at_ms", &self.at_ms)
            .field("state_hash", &self.state_hash)
            .field("ledger_seq", &self.ledger_seq)
            .field("chunk_seq", &self.chunk_seq)
            .field("arena", &self.arena)
            .field("definition_digest", &self.definition_digest)
            .field("state_bytes", &self.bytes().len())
            .finish()
    }
}

/// **A snapshot document's head** (16b cut AF.1): every key but `state`, read
/// without parsing it ([`Snapshot::head`]). What 16b cut AF.3's boot decides
/// resumability on before it builds a ground.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotHead {
    /// The document's format ([`SNAPSHOT_FORMAT`]; a head of another is never
    /// answered).
    pub format: u32,
    /// The ground's id.
    pub arena: SmolStr,
    /// The ground's digest, 64 lowercase base-16 characters.
    pub definition_digest: String,
    /// The compiled locomotion table's digest the match was priced under.
    pub physics_digest: String,
    /// The arena millisecond it was taken at.
    pub at_ms: i64,
    /// The state hash recorded at encode.
    pub state_hash: u64,
    /// The ledger's last event sequence.
    pub ledger_seq: u64,
    /// The ledger's last closed chunk.
    pub chunk_seq: u64,
    /// SHA-256 over the state string's bytes, 64 lowercase base-16 characters.
    pub state_digest: String,
}

/// **Why a snapshot was refused** (16b cut AF.1). Every refusal is a value,
/// never a panic, and each names what it compared.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    /// Complete encoded document, including its escaped state string.
    #[error("snapshot needs at least {required_at_least} bytes; maximum is {limit}")]
    Capacity {
        /// Bytes counted before serialization stopped.
        required_at_least: u64,
        /// Effective encoded-document ceiling.
        limit: u64,
    },
    /// The document's `record` is not [`SNAPSHOT_RECORD`].
    #[error("not a snapshot document (record {found})")]
    Record {
        /// What the `record` key held, as JSON when it was not a string.
        found: String,
    },
    /// The document is a snapshot of another format.
    #[error("snapshot format {found} is not {expected}")]
    Format {
        /// The document's format.
        found: u32,
        /// [`SNAPSHOT_FORMAT`].
        expected: u32,
    },
    /// The ground given is not the ground the snapshot was taken on.
    #[error("the snapshot was taken on ground {recorded} and this ground is {given}")]
    Definition {
        /// The digest the head records.
        recorded: String,
        /// The digest of the ground given.
        given: String,
    },
    /// This build prices movement under another locomotion table.
    #[error("the ground's movement prices changed (recorded {recorded}, loaded {loaded})")]
    Physics {
        /// The table digest the head records.
        recorded: String,
        /// `mush_atlas::physics::LOCOMOTION_DIGEST` of this build.
        loaded: String,
    },
    /// The state string is not the one the head was written for.
    #[error("state digest mismatch: recorded {recorded}, computed {computed}")]
    DigestMismatch {
        /// The head's `state_digest`.
        recorded: String,
        /// SHA-256 over the state string as given.
        computed: String,
    },
    /// The document or the state does not decode into the shape this format
    /// writes (`serde`'s own message), or its head disagrees with its state.
    #[error("the snapshot does not decode: {0}")]
    Malformed(String),
    /// The decoded arena hashes to something else than the arena that was
    /// encoded.
    #[error("state hash mismatch: recorded {recorded}, decoded {decoded}")]
    HashMismatch {
        /// The head's `state_hash`.
        recorded: u64,
        /// [`crate::arena::hash::state_hash`] of the decoded arena.
        decoded: u64,
    },
    /// The decoded state breaks a refusal [`Arena::new`] makes of a check-out.
    #[error("the decoded state is not a match: {0}")]
    Refused(#[from] ArenaError),
    /// **A think awaits its write** (17b cut M4.1; doc 17 §6, D-UM-22): the
    /// arena holds a collected think whose answer nothing has delivered or
    /// settled yet, so a snapshot would carry its facts and no answer. The
    /// hulls, in id order.
    #[error("a think awaits its answer on {}: the batch was not delivered before the encode", awaiting_text(.units))]
    Awaiting {
        /// The hulls whose think is unwritten ([`Arena::awaiting`]).
        units: Vec<ObjId>,
    },
}

/// The hulls of [`SnapshotError::Awaiting`] as `#1, #2`.
fn awaiting_text(units: &[ObjId]) -> String {
    units
        .iter()
        .map(|unit| format!("#{}", unit.0))
        .collect::<Vec<_>>()
        .join(", ")
}

/// **A 32-byte digest as the tree renders every digest**: 64 lowercase base-16
/// characters (16b cut AF.1 moved it here from `crate::arena_bridge`, which
/// re-exports it, so the snapshot's head and the result record write one form).
#[must_use]
pub fn digest_text(digest: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(64);
    for byte in digest {
        // `write!` into a `String` is infallible; the result is discarded rather
        // than unwrapped, which is also what keeps this off the panic budget.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// **[`digest_text`] read back** (17b cut M3.3): exactly 64 lowercase
/// base-16 characters to their 32 bytes; anything else — a capital, a short
/// string, a stray character — is `None`, so the one form is the only form.
#[must_use]
pub fn digest_from_text(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (byte, pair) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let mut value = 0_u8;
        for digit in pair {
            let nibble = match digit {
                b'0'..=b'9' => digit - b'0',
                b'a'..=b'f' => digit - b'a' + 10,
                _ => return None,
            };
            value = value.wrapping_mul(16).wrapping_add(nibble);
        }
        *byte = value;
    }
    Some(out)
}

/// **A 32-byte digest field as the tree writes every digest** (17b cut M3.3):
/// `#[serde(with = "crate::arena::snapshot::digest_serde")]` writes
/// [`digest_text`] and reads [`digest_from_text`], refusing any other form —
/// the seal's two digests ([`crate::arena::mind::MindSeal`]) ride a record a
/// person may read, where a list of thirty-two numbers would not be.
pub mod digest_serde {
    use serde::{Deserialize as _, Deserializer, Serializer};

    /// Write `digest` as 64 lowercase base-16 characters.
    ///
    /// # Errors
    ///
    /// The serializer's own.
    pub fn serialize<S: Serializer>(digest: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::digest_text(digest))
    }

    /// Read 64 lowercase base-16 characters.
    ///
    /// # Errors
    ///
    /// A string of any other form, named.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let text = String::deserialize(deserializer)?;
        super::digest_from_text(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "a digest is 64 lowercase base-16 characters, not {text:?}"
            ))
        })
    }
}

/// SHA-256 over the state string's bytes, as [`digest_text`] renders it.
fn state_digest(state: &str) -> String {
    digest_text(&Sha256::digest(state.as_bytes()).into())
}

/// **The document, as `serde` reads and writes it** — every key of this module
/// head's shape, and no other (`deny_unknown_fields`). The state is borrowed
/// both ways: encoding does not copy it into the struct, and a head read or a
/// decode of a document in hand does not copy it out.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document<'a> {
    record: String,
    format: u32,
    arena: SmolStr,
    arena_definition_digest: String,
    physics_digest: String,
    at_ms: i64,
    ledger_seq: u64,
    chunk_seq: u64,
    /// A decimal string, for the reason the state is one (this module's head).
    state_hash: String,
    state_digest: String,
    #[serde(borrow)]
    state: Cow<'a, str>,
}

impl Snapshot {
    /// **Encode an arena between two pulses** (16b cut AF.1).
    ///
    /// The state is serialised from borrows (`ArenaStateRef`), because
    /// production pays this every `[tacsim.arena].snapshot_ms` on the arena
    /// task's thread; what it copies is the ledger's open chunk and closed queue,
    /// the clock's four fields, and **every unit's opaque
    /// `mush_tactics::TacticalUnit` once**: that type serialises through
    /// `#[serde(into = "UnitRepr")]` (mush-tactics' own contract, as its
    /// `MobilityProfile` does through `ProfileRepr`), and serde's `into` clones
    /// the value before converting it — the sections, cells, components,
    /// capabilities and rules blob of each unit. That is the encode's largest
    /// copy; 16b cut AF.2's off-and-on timing on the harness is what measures
    /// it, and a borrowing `UnitRepr` serialiser in mush-tactics is the lever if
    /// the measurement asks for one.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::Malformed`] when the arena is inside a pulse — the three
    /// per-pulse scratch tables are drained before `step` returns, so an arena
    /// holding any of them was not handed over between two pulses — or when a
    /// value does not serialise.
    pub fn encode(arena: &Arena) -> Result<Snapshot, SnapshotError> {
        Self::encode_with_limit(arena, arena.arena_cfg.snapshot_max_bytes)
    }

    /// A stricter caller budget; never raises the arena's captured limit.
    pub fn encode_with_limit(arena: &Arena, limit: u64) -> Result<Snapshot, SnapshotError> {
        let limit = limit
            .min(arena.arena_cfg.snapshot_max_bytes)
            .min(super::snapshot_capacity::SNAPSHOT_MAX_BYTES);
        if !arena.lost_designators.is_empty()
            || !arena.spent_hits.is_empty()
            || !arena.pending_notices.is_empty()
        {
            return Err(SnapshotError::Malformed("taken inside a pulse".to_owned()));
        }
        // **17b cut M4.1** (doc 17 §6 "Snapshot and resume"; D-UM-22,
        // D-UM-23): a think collected and not yet written would be carried
        // as its facts with an EMPTY answer and land as nothing on the
        // resumed arena — refused by name instead. The sim task awaits every
        // outstanding batch before it encodes (`arena_task`'s
        // `snapshot_if_due`), so production never reaches this arm; a rig
        // that steps and encodes settles first (`Arena::step_settled`).
        if !arena.awaiting.is_empty() {
            return Err(SnapshotError::Awaiting {
                units: arena.awaiting.iter().copied().collect(),
            });
        }
        let state = String::from_utf8(super::snapshot_capacity::json_bytes(
            &ArenaStateRef::from(arena),
            limit,
        )?)
        .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
        let hash = state_hash(arena);
        let definition_digest = digest_text(&arena.definition.digest());
        let encoded = Document {
            record: SNAPSHOT_RECORD.to_owned(),
            format: SNAPSHOT_FORMAT,
            arena: arena.definition.id.clone(),
            arena_definition_digest: definition_digest.clone(),
            physics_digest: LOCOMOTION_DIGEST.to_owned(),
            at_ms: arena.now_ms,
            ledger_seq: arena.ledger.seq(),
            chunk_seq: arena.ledger.last_closed_seq(),
            state_hash: hash.to_string(),
            state_digest: state_digest(&state),
            state: Cow::Borrowed(&state),
        };
        // Count the escaped outer document before allocating its state copy.
        super::snapshot_capacity::json_len(&encoded, limit)?;
        let document = serde_json::to_value(encoded)
            .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
        Ok(Snapshot {
            at_ms: arena.now_ms,
            state_hash: hash,
            ledger_seq: arena.ledger.seq(),
            chunk_seq: arena.ledger.last_closed_seq(),
            arena: arena.definition.id.clone(),
            definition_digest,
            document,
        })
    }

    /// **Read a document's head**, never its state (16b cut AF.1): the record
    /// and the format first, then every other key but `state`, whose string is
    /// neither parsed nor digested here.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::Record`], [`SnapshotError::Format`], or
    /// [`SnapshotError::Malformed`] with `serde`'s message — a missing or
    /// unknown key, or a `state_hash` that is not a decimal `u64`.
    pub fn head(document: &serde_json::Value) -> Result<SnapshotHead, SnapshotError> {
        super::snapshot_capacity::json_len(document, super::snapshot_capacity::SNAPSHOT_MAX_BYTES)?;
        read(document).map(|(head, _)| head)
    }

    /// **The arena back out of a document** (16b cut AF.1), on `definition` —
    /// the ground the caller built, which must be the one it was taken on.
    ///
    /// The checks, in this order and on every decode: the record; the format;
    /// the ground's digest (before anything is built); the locomotion table's
    /// digest; the state digest (before the string is parsed); the parse
    /// (`deny_unknown_fields` on every struct of this crate and of
    /// mush-tactics' unit form in the tree, with two exceptions: the standing
    /// order's `crate::deliberation::Order`, whose flattened objective serde
    /// will not combine with it — its own comment says so — and mush-sim's
    /// `Pose`, read through that crate's `PoseWire`, which is not this cut's to
    /// close — a key one of those two loses is caught by the pin or by nothing);
    /// the arena built by an exhaustive literal, its six re-derived fields
    /// derived and its twelve reset; the pose refusals [`Arena::new`] makes of a check-out
    /// (`BadHeading`, `PoseOffWindow`, `CellOccupied`); **the state hash**, which
    /// is never optional; and last, that the head agrees with the state it
    /// carries (the time, the two sequences and the ground's id).
    ///
    /// **`CellImpassable` is not re-applied**, on purpose. It is `Arena::new`'s
    /// rule for a pose that comes from **outside** — the world's hand-over at
    /// check-out — and no lane of a running arena breaks it (motion never
    /// enters such a cell, and since 16b cut AE.1 a staff `@arena/place` refuses
    /// one for the unit's own row as `impassable`). A snapshot's poses are the
    /// arena's own, and the hash check is the proof of that. The three refusals
    /// re-applied are the ones the decoded arena's own structures stand on — the
    /// spatial index keys a unit by one on-window cell with one occupant, and
    /// the facet arithmetic reads a heading in range — whereas a unit on a cell
    /// its row cannot enter is a state the step survives (motion caps its speed
    /// at nought), so it is left to the hash.
    ///
    /// The decoded arena's clock has no wall anchor: the caller resumes it with
    /// [`Arena::resume_at`] before its first pulse.
    ///
    /// # Errors
    ///
    /// Each check above refuses by its own [`SnapshotError`] variant.
    pub fn decode(
        document: &serde_json::Value,
        definition: ArenaDefinition,
    ) -> Result<Arena, SnapshotError> {
        Self::decode_with_limit(
            document,
            definition,
            super::snapshot_capacity::SNAPSHOT_MAX_BYTES,
        )
    }

    /// Apply the operator's current ceiling as well as the captured ceiling.
    /// The first state scan allocates no arena collections.
    pub fn decode_with_limit(
        document: &serde_json::Value,
        definition: ArenaDefinition,
        limit: u64,
    ) -> Result<Arena, SnapshotError> {
        let limit = limit.min(super::snapshot_capacity::SNAPSHOT_MAX_BYTES);
        super::snapshot_capacity::json_len(document, limit)?;
        let (head, state) = read(document)?;
        let given = digest_text(&definition.digest());
        if given != head.definition_digest {
            return Err(SnapshotError::Definition {
                recorded: head.definition_digest,
                given,
            });
        }
        if head.physics_digest != LOCOMOTION_DIGEST {
            return Err(SnapshotError::Physics {
                recorded: head.physics_digest,
                loaded: LOCOMOTION_DIGEST.to_owned(),
            });
        }
        let computed = state_digest(&state);
        if computed != head.state_digest {
            return Err(SnapshotError::DigestMismatch {
                recorded: head.state_digest,
                computed,
            });
        }
        #[derive(Deserialize)]
        struct RecordedLimit {
            arena_cfg: LimitField,
        }
        #[derive(Deserialize)]
        struct LimitField {
            snapshot_max_bytes: u64,
        }
        let recorded: RecordedLimit = serde_json::from_str(&state)
            .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
        let recorded = recorded.arena_cfg.snapshot_max_bytes;
        if !(super::snapshot_capacity::SNAPSHOT_MIN_BYTES
            ..=super::snapshot_capacity::SNAPSHOT_MAX_BYTES)
            .contains(&recorded)
        {
            return Err(SnapshotError::Malformed(
                "recorded snapshot byte limit is invalid".into(),
            ));
        }
        super::snapshot_capacity::json_len(document, limit.min(recorded))?;
        let parsed: ArenaState = serde_json::from_str(&state)
            .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
        let arena = parsed.into_arena(definition)?;
        super::memory_bound::validate_arena(&arena)?;
        // `Arena::new`'s pose refusals, not `CellImpassable` (see above).
        check_poses(&arena.definition.terrain, &arena.units)?;
        let decoded = state_hash(&arena);
        if decoded != head.state_hash {
            return Err(SnapshotError::HashMismatch {
                recorded: head.state_hash,
                decoded,
            });
        }
        // The head's own keys are outside the state digest, and the boot reads
        // them before it decodes (16b cut AF.3): a head that says one thing of a
        // state that says another is a moved field, refused like one.
        if head.at_ms != arena.now_ms
            || head.ledger_seq != arena.ledger.seq()
            || head.chunk_seq != arena.ledger.last_closed_seq()
            || head.arena != arena.definition.id
        {
            return Err(SnapshotError::Malformed(format!(
                "the head says {} at {} ms, event {} and chunk {}, and the state says {} at {} ms, \
                 event {} and chunk {}",
                head.arena,
                head.at_ms,
                head.ledger_seq,
                head.chunk_seq,
                arena.definition.id,
                arena.now_ms,
                arena.ledger.seq(),
                arena.ledger.last_closed_seq()
            )));
        }
        Ok(arena)
    }

    /// Refuse oversized raw envelopes before allocating their JSON value.
    /// Value-based module boot has already allocated that envelope; its entrypoint
    /// still bounds the inner state parse before arena collections grow.
    pub fn decode_bytes(
        bytes: &[u8],
        definition: ArenaDefinition,
        limit: u64,
    ) -> Result<Arena, SnapshotError> {
        let limit = limit.min(super::snapshot_capacity::SNAPSHOT_MAX_BYTES);
        if bytes.len() as u64 > limit {
            return Err(SnapshotError::Capacity {
                required_at_least: bytes.len() as u64,
                limit,
            });
        }
        let document = serde_json::from_slice(bytes)
            .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
        Self::decode_with_limit(&document, definition, limit)
    }

    /// **The state string's bytes** — what the snapshot costs to store, and what
    /// the harness's bound is asserted on (16b cut AF.2). Empty for a document
    /// with no state string, which [`Snapshot::encode`] never builds.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.document
            .get("state")
            .and_then(serde_json::Value::as_str)
            .map_or(&[], str::as_bytes)
    }
}

/// **The head and the borrowed state of a document**: the record, then the
/// format, off the raw value — so a document of another kind or another format
/// is refused by name whatever else its shape is — then the whole [`Document`].
fn read(document: &serde_json::Value) -> Result<(SnapshotHead, Cow<'_, str>), SnapshotError> {
    let record = document.get("record");
    if record.and_then(serde_json::Value::as_str) != Some(SNAPSHOT_RECORD) {
        return Err(SnapshotError::Record {
            found: match record {
                Some(serde_json::Value::String(word)) => word.clone(),
                Some(other) => other.to_string(),
                None => "none".to_owned(),
            },
        });
    }
    let format = document
        .get("format")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| SnapshotError::Malformed("the format is not a number".to_owned()))?;
    if format != u64::from(SNAPSHOT_FORMAT) {
        return Err(SnapshotError::Format {
            found: u32::try_from(format).unwrap_or(u32::MAX),
            expected: SNAPSHOT_FORMAT,
        });
    }
    let Document {
        record: _,
        format,
        arena,
        arena_definition_digest,
        physics_digest,
        at_ms,
        ledger_seq,
        chunk_seq,
        state_hash,
        state_digest,
        state,
    } = Document::deserialize(document)
        .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
    let state_hash = state_hash.parse::<u64>().map_err(|_| {
        SnapshotError::Malformed(format!(
            "state_hash {state_hash:?} is not a decimal 64-bit number"
        ))
    })?;
    Ok((
        SnapshotHead {
            format,
            arena,
            definition_digest: arena_definition_digest,
            physics_digest,
            at_ms,
            state_hash,
            ledger_seq,
            chunk_seq,
            state_digest,
        },
        state,
    ))
}

// ---------------------------------------------------------------------------
// The state tree: two structs, one field list
// ---------------------------------------------------------------------------

/// **What the encode serialises**: the 36 carried fields of [`Arena`], in its
/// own order, every one a borrow but the two small mirrors ([`LedgerState`] and
/// [`ClockState`]) — so an encode clones no contact and no shot, and of a unit
/// only the opaque `mush_tactics::TacticalUnit`, which its own
/// `#[serde(into = "UnitRepr")]` clones on the way out ([`Snapshot::encode`]
/// says what that costs and who measures it).
/// [`ArenaState`] is its owned twin, field for field; the encode → decode →
/// encode round trip (`af1_the_encoding_is_deterministic_and_integer`)
/// proves the two agree byte for byte.
#[derive(Serialize)]
struct ArenaStateRef<'a> {
    units: &'a [UnitState],
    epochs: &'a EpochTable,
    contacts: &'a ShadowContacts,
    designators: &'a DesignatorBook,
    #[serde(serialize_with = "pairs::serialize_ref")]
    spent_said: &'a BTreeMap<(ObjId, ObjId, SmolStr), i64>,
    due: &'a DueHeap,
    ledger: LedgerState,
    clock: ClockState,
    tape_cursor: Option<crate::arena::input_tape::TapeCursor>,
    seed: &'a u64,
    resolution: &'a ResolutionDef,
    pilots: &'a [Pilot],
    motors: &'a MotorTable,
    minds: &'a BTreeMap<ObjId, MindState>,
    cards: &'a crate::arena::runtime_card::CardCatalog,
    profiles: &'a BTreeMap<ObjId, crate::arena::profile_runtime::ProfileRuntime>,
    conditions: &'a ConditionTable,
    #[serde(serialize_with = "pairs::serialize_ref")]
    held_said: &'a BTreeMap<(ObjId, u8, ContactId), i64>,
    odds_floor_permille: &'a u32,
    classes: &'a ClassTable,
    pending: &'a BTreeMap<ObjId, VecDeque<Intent>>,
    shots: &'a BTreeMap<u64, Shot>,
    next_shot_seq: &'a u64,
    stop: &'a Option<StopReason>,
    now_ms: &'a i64,
    at_release: &'a BTreeMap<ObjId, ReleaseSnapshot>,
    pending_catchup: &'a BTreeMap<ObjId, Catchup>,
    #[serde(serialize_with = "pairs::serialize_ref")]
    held_losses: &'a BTreeMap<(ObjId, ContactId), (i64, Designator)>,
    ended: &'a Option<MatchEnd>,
    arena_cfg: &'a ArenaConfig,
    match_cfg: &'a MatchConfig,
    sensors_cfg: &'a SensorsConfig,
    alert_range_cm: &'a u32,
    jam_radius_cm: &'a u32,
    squads: &'a [SquadRoster],
    #[serde(serialize_with = "pairs::serialize_ref")]
    squad_watch: &'a SquadWatchTable,
    squads_expired: &'a ExpiredSquads,
    mode: &'a MatchMode,
    hold: &'a Option<HoldState>,
}

impl<'a> From<&'a Arena> for ArenaStateRef<'a> {
    /// **Every field of [`Arena`] by name, and no `..`**: a field a later cut adds
    /// does not compile here until it is carried, re-derived or reset.
    fn from(arena: &'a Arena) -> ArenaStateRef<'a> {
        let Arena {
            // Re-derived: the caller's ground, checked against the head's digest.
            definition: _definition,
            units,
            epochs,
            contacts,
            designators,
            // Reset: drained inside the substep that fills it.
            lost_designators: _lost_designators,
            // Reset: drained inside the substep that fills it.
            spent_hits: _spent_hits,
            spent_said,
            due,
            ledger,
            // Re-derived: built and rebuilt over the carried units.
            index: _index,
            // Re-derived (16c T-105): built over the carried units' chassis.
            chassis: _chassis,
            clock,
            tape_cursor,
            tape_witnesses: _tape_witnesses,
            seed,
            resolution,
            pilots,
            // Carried (17b cut M1a.1): the motor rows and the path budget; the
            // table's scratch is `#[serde(skip)]`, as the reflex memory's was
            // (that memory, `Arena.reflex`, was deleted in 17b cut M4.6).
            motors,
            // Carried (17b cut M1a.5): every unit mind's row.
            minds,
            cards,
            profiles,
            // Carried (17b cut M1b.1): every hull's condition slots, the
            // held-shot rows (through `pairs`) and the floor captured at
            // check-out.
            conditions,
            held_said,
            odds_floor_permille,
            // Carried (17b cut M3.2): the class table captured at check-out.
            classes,
            // Reset (17b cut M4.1): the "answered" fact beside each row, and
            // the thinks collected and not yet written — `encode` refuses an
            // arena that holds any (`awaiting`, below), and the decode lays
            // all three down empty.
            awaiting: _awaiting,
            traces: _traces,
            collected: _collected,
            unsettled: _unsettled,
            // Reset (17b cut M1b.5): the card changes waiting for the next
            // pulse are scratch, emptied at its start.
            pending_cards: _pending_cards,
            // Re-derived (17b cut M1a.5): compiled code, `default_mind_runner`.
            mind_runner: _mind_runner,
            // Re-derived (17b cut M3.5): compiled code, empty at decode —
            // `crate::minds::install_sealed_runners` refills it off the seals.
            runners: _runners,
            // Re-derived: `Arena::new`'s line over the definition.
            min_cost: _min_cost,
            pending,
            shots,
            // Reset: report-only observations never enter saved execution.
            acceptance: _acceptance,
            next_shot_seq,
            // Reset: a slot of one phase, outside between two pulses.
            action_hash: _action_hash,
            stop,
            now_ms,
            // Reset: a view; a resumed pilot's first frame is a keyframe.
            last_frames: _last_frames,
            at_release,
            pending_catchup,
            // Reset: a view of who is watching, set by the task.
            watched: _watched,
            // Reset: the watch's once-a-second gate, a view.
            last_observer_s: _last_observer_s,
            // Reset: cleared by the frames of the pulse that staged them.
            pending_notices: _pending_notices,
            held_losses,
            // Reset: cleared at the start of every pulse.
            substep_hashes: _substep_hashes,
            ended,
            arena_cfg,
            match_cfg,
            sensors_cfg,
            alert_range_cm,
            jam_radius_cm,
            // Re-derived: a memo, empty, exact by 16b cut U.2's rule.
            jam_fold: _jam_fold,
            squads,
            squad_watch,
            squads_expired,
            mode,
            hold,
        } = arena;
        ArenaStateRef {
            units,
            epochs,
            contacts,
            designators,
            spent_said,
            due,
            ledger: ledger.to_state(),
            clock: clock.to_state(),
            tape_cursor: *tape_cursor,
            seed,
            resolution,
            pilots,
            motors,
            minds,
            cards,
            profiles,
            conditions,
            held_said,
            odds_floor_permille,
            classes,
            pending,
            shots,
            next_shot_seq,
            stop,
            now_ms,
            at_release,
            pending_catchup,
            held_losses,
            ended,
            arena_cfg,
            match_cfg,
            sensors_cfg,
            alert_range_cm,
            jam_radius_cm,
            squads,
            squad_watch,
            squads_expired,
            mode,
            hold,
        }
    }
}

/// **What the decode parses**: [`ArenaStateRef`]'s fields, owned, with the same
/// names in the same order, and no key it does not name.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArenaState {
    units: Vec<UnitState>,
    epochs: EpochTable,
    contacts: ShadowContacts,
    designators: DesignatorBook,
    #[serde(with = "pairs")]
    spent_said: BTreeMap<(ObjId, ObjId, SmolStr), i64>,
    due: DueHeap,
    ledger: LedgerState,
    clock: ClockState,
    tape_cursor: Option<crate::arena::input_tape::TapeCursor>,
    seed: u64,
    resolution: ResolutionDef,
    pilots: Vec<Pilot>,
    motors: MotorTable,
    minds: BTreeMap<ObjId, MindState>,
    cards: crate::arena::runtime_card::CardCatalog,
    profiles: BTreeMap<ObjId, crate::arena::profile_runtime::ProfileRuntime>,
    conditions: ConditionTable,
    #[serde(with = "pairs")]
    held_said: BTreeMap<(ObjId, u8, ContactId), i64>,
    odds_floor_permille: u32,
    classes: ClassTable,
    pending: BTreeMap<ObjId, VecDeque<Intent>>,
    shots: BTreeMap<u64, Shot>,
    next_shot_seq: u64,
    stop: Option<StopReason>,
    now_ms: i64,
    at_release: BTreeMap<ObjId, ReleaseSnapshot>,
    pending_catchup: BTreeMap<ObjId, Catchup>,
    #[serde(with = "pairs")]
    held_losses: BTreeMap<(ObjId, ContactId), (i64, Designator)>,
    ended: Option<MatchEnd>,
    arena_cfg: ArenaConfig,
    match_cfg: MatchConfig,
    sensors_cfg: SensorsConfig,
    alert_range_cm: u32,
    jam_radius_cm: u32,
    squads: Vec<SquadRoster>,
    #[serde(with = "pairs")]
    squad_watch: SquadWatchTable,
    squads_expired: ExpiredSquads,
    mode: MatchMode,
    hold: Option<HoldState>,
}

impl ArenaState {
    /// **The arena, by an exhaustive literal** over every field of [`Arena`]:
    /// the 36 carried placed, the 6 re-derived derived and the 12 reset reset.
    fn into_arena(self, definition: ArenaDefinition) -> Result<Arena, SnapshotError> {
        let ArenaState {
            units,
            epochs,
            contacts,
            designators,
            spent_said,
            due,
            ledger,
            clock,
            tape_cursor,
            seed,
            resolution,
            pilots,
            motors,
            minds,
            cards,
            profiles,
            conditions,
            held_said,
            odds_floor_permille,
            classes,
            pending,
            shots,
            next_shot_seq,
            stop,
            now_ms,
            at_release,
            pending_catchup,
            held_losses,
            ended,
            arena_cfg,
            match_cfg,
            sensors_cfg,
            alert_range_cm,
            jam_radius_cm,
            squads,
            squad_watch,
            squads_expired,
            mode,
            hold,
        } = self;
        // M5rules carries bounded typed rows. A digest authenticates bytes;
        // it does not substitute for validating their supported shape.
        for (unit, slots) in conditions.rows() {
            if slots.len() > crate::arena::core::MAX_CONDITION_SLOTS
                || !units.iter().any(|state| state.id == *unit)
            {
                return Err(SnapshotError::Malformed(
                    "invalid native rule row".to_owned(),
                ));
            }
            for rule in slots.iter().flatten() {
                crate::arena::conditions::validate_rule_shape(rule)
                    .map_err(|_| SnapshotError::Malformed("invalid native rule".to_owned()))?;
            }
        }
        // 17b cut M4.3: the doctrine's cached state, re-derived from each
        // row's memory (never carried).
        let mut minds = minds;
        for row in minds.values_mut() {
            row.doctrine_state = crate::arena::mind::doctrine_state_of(&row.memory);
        }
        let ledger = Ledger::from_state(&arena_cfg.ledger, ledger)?;
        // Re-derived exactly as `Arena::new` derives them: the index over the
        // units it now holds, and the cheapest passable cell of each row.
        let mut index = SpatialIndex::new(&definition.terrain);
        index.rebuild(&definition.terrain, &units);
        // 16c T-105: the chassis words, re-derived over the decoded units as
        // `Arena::new` reads them off the check-out records.
        let chassis = crate::arena::state::ChassisWords::table(&units);
        let min_cost = CostRow::ALL.map(|row| definition.terrain.min_cost(row));
        // 17b cut M1a.5: the runner `Arena::new` opens with, by the same call.
        let mind_runner = default_mind_runner(&arena_cfg);
        let mut arena = Arena {
            // 17b cut M2.2: wrapped once, as `Arena::new` wraps it.
            definition: std::sync::Arc::new(definition),
            units,
            chassis,
            epochs,
            contacts,
            designators,
            lost_designators: BTreeMap::new(),
            spent_hits: BTreeSet::new(),
            spent_said,
            due,
            ledger,
            index,
            clock: Clock::from_state(clock),
            tape_cursor,
            tape_witnesses: crate::arena::input_tape::WitnessBuffer::default(),
            seed,
            resolution,
            pilots,
            motors,
            minds,
            cards,
            profiles,
            conditions,
            held_said,
            odds_floor_permille,
            // 17b cut M3.2: carried — the table the match was checked out
            // under.
            classes,
            // Reset (17b cut M4.1): laid down empty — no think awaits across
            // a snapshot (`encode` refused while one did).
            awaiting: BTreeSet::new(),
            traces: super::author_trace::TraceStore::default(),
            collected: Vec::new(),
            unsettled: Vec::new(),
            pending_cards: BTreeMap::new(),
            mind_runner,
            // 17b cut M3.5: re-derived empty, as `Arena::new` lays it; the
            // sealed installer refills it (`crate::arena_task::resume_arena`).
            runners: BTreeMap::new(),
            min_cost,
            pending,
            shots,
            acceptance: None,
            next_shot_seq,
            action_hash: ActionHash::Outside,
            stop,
            now_ms,
            last_frames: FrameSlots::new(),
            at_release,
            pending_catchup,
            watched: false,
            last_observer_s: None,
            pending_notices: BTreeMap::new(),
            held_losses,
            substep_hashes: Vec::new(),
            ended,
            arena_cfg,
            match_cfg,
            sensors_cfg,
            alert_range_cm,
            jam_radius_cm,
            jam_fold: BTreeMap::new(),
            squads,
            squad_watch,
            squads_expired,
            mode,
            hold,
        };
        arena
            .hydrate_profiles()
            .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
        Ok(arena)
    }
}

// ---------------------------------------------------------------------------
// The pairs helper
// ---------------------------------------------------------------------------

/// **A map whose key JSON cannot write as an object key, as an array of
/// `[key, value]` pairs** (16b cut AF.1), in the map's own key order.
///
/// `serde_json` writes a map key only when it is a string, an integer or a unit
/// variant, and refuses a tuple or a struct **at run time** with "key must be a
/// string" — the derive compiles, because `BTreeMap<K, V>: Serialize` asks only
/// `K: Serialize`. So the six such maps in the carried tree are written through
/// here, as `#[serde(with = "crate::arena::snapshot::pairs")]` on a field (or
/// [`pairs::serialize_ref`] on a borrowed one). Nothing catches the next such
/// key at compile time: the unit tests below pin the key form of every keyed
/// map the tree holds today, and whoever adds one adds it there by hand.
pub(crate) mod pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// The map as a sequence of `(key, value)`, key ascending.
    ///
    /// # Errors
    ///
    /// Whatever the serializer refuses.
    pub(crate) fn serialize<K, V, S>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        serializer.collect_seq(map.iter())
    }

    /// [`serialize`] for a borrowed map, as the encode's borrowed state holds
    /// one.
    ///
    /// # Errors
    ///
    /// Whatever the serializer refuses.
    pub(crate) fn serialize_ref<K, V, S>(
        map: &&BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        serialize(map, serializer)
    }

    /// The pairs back into a map. **A key that appears twice is refused**: a
    /// map cannot have written it, so the document is not one this format
    /// wrote.
    ///
    /// # Errors
    ///
    /// The deserializer's own, or the duplicate key.
    pub(crate) fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let pairs = Vec::<(K, V)>::deserialize(deserializer)?;
        let mut map = BTreeMap::new();
        for (key, value) in pairs {
            if map.insert(key, value).is_some() {
                return Err(serde::de::Error::custom(
                    "a key appears twice in a table written as pairs",
                ));
            }
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};

    use mush_core::ObjId;
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use smol_str::SmolStr;

    use super::{Snapshot, SnapshotError, pairs};
    use crate::arena::catchup::{Catchup, ReleaseSnapshot, SectionWords};
    use crate::arena::command::SquadWatch;
    use crate::arena::intents::{Intent, IntentKind};
    use crate::arena::motor::MotorRow;
    use crate::arena::resolution::Resolution;
    use crate::arena::state::{
        Channel, Classification, ContactId, Designator, Power, SeenPose, ShadowContact, Team,
        UnitState,
    };
    use crate::arena::test_support::{arena_of, pose, scout, tank};
    use crate::arena::weapons::Shot;
    use crate::definitions::UnitClass;
    use crate::deliberation::SquadId;

    /// **Every field here is carried; a new field is carried by the derive —
    /// decide on purpose whether it should be, and if not, `#[serde(skip)]` it
    /// and put it in the reset list.**
    ///
    /// [`UnitState`] destructured with every field named and no `..`, answering
    /// the names, so the test below can hold the encoded unit's keys to them.
    fn every_unit_field(unit: &UnitState) -> [&'static str; 45] {
        let UnitState {
            id: _,
            team: _,
            callsign: _,
            pose: _,
            speed_cms: _,
            desired_heading_mdeg: _,
            desired_speed_cms: _,
            power: _,
            mobility: _,
            slope_limit_pct: _,
            cost_row: _,
            class: _,
            fatal_sections: _,
            grade_pct: _,
            occupied_hold: _,
            carry: _,
            limits: _,
            sensor_height_cm: _,
            sensor: _,
            thermal: _,
            jammer: _,
            ecm_on: _,
            eccm: _,
            receiver: _,
            weapons: _,
            profile: _,
            gunnery: _,
            heat: _,
            controller: _,
            seal: _,
            card_profile: _,
            unit: _,
            condition: _,
            ready_at_ms: _,
            ammo: _,
            lock: _,
            blocked: _,
            held: _,
            linkdead: _,
            disabled: _,
            shots_fired: _,
            hits: _,
            orders_window_s: _,
            orders_in_window: _,
            own_last_hit_ms: _,
        } = unit;
        [
            "id",
            "team",
            "callsign",
            "pose",
            "speed_cms",
            "desired_heading_mdeg",
            "desired_speed_cms",
            "power",
            "mobility",
            "slope_limit_pct",
            "cost_row",
            "class",
            "fatal_sections",
            "grade_pct",
            "occupied_hold",
            "carry",
            "limits",
            "sensor_height_cm",
            "sensor",
            "thermal",
            "jammer",
            "ecm_on",
            "eccm",
            "receiver",
            "weapons",
            "profile",
            "gunnery",
            "heat",
            "controller",
            "seal",
            // 17b cut M5.2: carried on purpose — the active profile and its
            // stamp decide a later card and a refusal.
            "card_profile",
            "unit",
            "condition",
            "ready_at_ms",
            "ammo",
            "lock",
            "blocked",
            "held",
            "linkdead",
            "disabled",
            "shots_fired",
            "hits",
            "orders_window_s",
            "orders_in_window",
            "own_last_hit_ms",
        ]
    }

    /// **The unit is carried whole**: an encoded unit's keys are exactly the
    /// names [`every_unit_field`] destructures — no field dropped by a skip that
    /// nobody listed, none added that nobody decided on.
    #[test]
    fn af1_a_unit_is_carried_whole_by_its_derive() {
        let arena = arena_of(vec![scout(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        let snapshot = Snapshot::encode(&arena).expect("an arena between pulses encodes");
        let state: serde_json::Value = serde_json::from_slice(snapshot.bytes()).expect("a state");
        let keys: BTreeSet<&str> = state
            .get("units")
            .and_then(|units| units.get(0))
            .and_then(serde_json::Value::as_object)
            .expect("one unit")
            .keys()
            .map(String::as_str)
            .collect();
        let named: BTreeSet<&str> = arena
            .units
            .first()
            .map(every_unit_field)
            .expect("one unit")
            .into_iter()
            .collect();
        assert_eq!(keys, named);
    }

    /// Encode and decode one value, and hold the answer to the original.
    fn round_trip<T>(value: &T) -> String
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let text = serde_json::to_string(value).expect("the value encodes");
        let back: T = serde_json::from_str(&text).expect("and decodes");
        assert_eq!(&back, value, "{text}");
        text
    }

    /// A map written through [`pairs`], as a field is.
    #[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
    #[serde(bound(
        serialize = "K: Serialize, V: Serialize",
        deserialize = "K: serde::Deserialize<'de> + Ord, V: serde::Deserialize<'de>"
    ))]
    struct Paired<K: Ord, V> {
        #[serde(with = "pairs")]
        map: BTreeMap<K, V>,
    }

    /// **The six tables written as pairs round-trip, and a duplicate key is
    /// refused** — the `SquadId`-keyed squad watch first, which `serde_json`
    /// would refuse as an object key at run time and every match with a
    /// machine squad fills at its first pulse.
    #[test]
    fn af1_the_pairs_round_trip_and_refuse_a_duplicate_key() {
        let squad = |team, index| SquadId { team, index };
        let watch = Paired {
            map: BTreeMap::from([
                (squad(Team::Red, 1), SquadWatch::default()),
                (squad(Team::Blue, 2), SquadWatch::default()),
            ]),
        };
        let text = round_trip(&watch);
        assert!(
            text.contains(r#"[[{"team":"red","index":1},"#),
            "a pair of the squad and its watch: {text}"
        );
        let epochs = Paired {
            map: BTreeMap::from([
                ((ObjId(4), Channel::Helm), 3_u64),
                ((ObjId(4), Channel::Fire), 2),
                ((ObjId(9), Channel::Sensors), 1),
            ]),
        };
        assert_eq!(
            round_trip(&epochs),
            r#"{"map":[[[4,"helm"],3],[[4,"fire"],2],[[9,"sensors"],1]]}"#
        );
        round_trip(&Paired {
            map: BTreeMap::from([((ObjId(4), ObjId(9)), 2_u16)]),
        });
        round_trip(&Paired {
            map: BTreeMap::from([((ObjId(4), ObjId(9), SmolStr::new("turret")), 5_000_i64)]),
        });
        round_trip(&Paired {
            map: BTreeMap::from([(
                (ObjId(4), ContactId(u64::MAX)),
                (
                    9_000_i64,
                    Designator::Slot {
                        team: Team::Blue,
                        slot: 1,
                    },
                ),
            )]),
        });

        // 17b cut M1b.1: `Arena.held_said`, keyed `(unit, slot, contact)` with
        // the contact id at the top of `u64`.
        assert_eq!(
            round_trip(&Paired {
                map: BTreeMap::from([((ObjId(4), 1_u8, ContactId(u64::MAX)), 9_000_i64)]),
            }),
            r#"{"map":[[[4,1,18446744073709551615],9000]]}"#
        );
        let twice = r#"{"map":[[[4,"helm"],3],[[4,"helm"],5]]}"#;
        let refused = serde_json::from_str::<Paired<(ObjId, Channel), u64>>(twice)
            .expect_err("a key twice is not a map");
        assert!(refused.to_string().contains("appears twice"), "{refused}");
    }

    /// **One of every keyed map the carried tree holds, through `serde_json`**
    /// (item 2's list; the report of 16b cut AF.1 has it with the file each
    /// lives in). The maps keyed by an integer, a string or a unit variant are
    /// written as JSON objects and must stay writable as ones; the six keyed by
    /// a tuple or a struct are [`pairs`]' and are round-tripped above.
    ///
    /// **This pins today's key types; it does not find tomorrow's.** Each map
    /// below is built by hand from item 2's grep, so a seventh struct- or
    /// tuple-keyed map added to the tree compiles (`BTreeMap<K, V>: Serialize`
    /// asks only `K: Serialize`) and passes here: whoever adds a keyed map adds
    /// it to this list by hand, as a pair table or an object. The mechanical
    /// backstop is weaker and later — `crates/tacsim/tests/arena.rs`' resume
    /// tests encode populated arenas, and fail on such a map only if their
    /// fixtures fill it.
    #[test]
    fn af1_every_keyed_map_in_the_tree_is_one_json_writes() {
        let unit = ObjId(4);
        // `DesignatorBook.next`.
        round_trip(&BTreeMap::from([(unit, 3_u16)]));
        // `ShadowContacts.by_viewer`, two levels: the viewer, then the contact id
        // at the top of `u64`.
        round_trip(&BTreeMap::from([(
            unit,
            BTreeMap::from([(
                ContactId(u64::MAX),
                ShadowContact::sighted(
                    ObjId(9),
                    Classification::Classified { team: Team::Blue },
                    5_000,
                    SeenPose::default(),
                ),
            )]),
        )]));
        // `Arena.pending` — since 17b cut M1b.1 with a cited `Fire`, a disarm
        // and an arming (a `Contact` target, every term set) in it, the carried
        // shapes that cut added to the queue.
        round_trip(&BTreeMap::from([(
            unit,
            VecDeque::from([
                Intent {
                    unit,
                    channel: Channel::Fire,
                    epoch: 2,
                    kind: IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: Some(1),
                    },
                    seq: 7,
                },
                Intent {
                    unit,
                    channel: Channel::Fire,
                    epoch: 2,
                    kind: IntentKind::Condition(crate::arena::conditions::Arming::Disarm {
                        weapon: None,
                    }),
                    seq: 8,
                },
                Intent {
                    unit,
                    channel: Channel::Fire,
                    epoch: 2,
                    kind: IntentKind::Condition(crate::arena::conditions::Arming::Arm(
                        crate::arena::conditions::FireCondition {
                            weapon: SmolStr::new("cannon"),
                            target: crate::arena::conditions::SlotTarget::Contact {
                                id: ContactId(u64::MAX),
                            },
                            min_permille: 600,
                            aspects: 0b0110,
                            min_rounds_left: 4,
                            min_range_m: 200,
                            max_range_m: u32::MAX,
                        },
                    )),
                    seq: 9,
                },
            ]),
        )]));
        // `Arena.shots`, by shot sequence.
        round_trip(&BTreeMap::from([(
            u64::MAX,
            Shot {
                seq: u64::MAX,
                shooter: unit,
                weapon: SmolStr::new("cannon"),
                contact: ContactId(1),
                target: ObjId(9),
                rounds: 1,
                fired_at_ms: 1_000,
                impact_at_ms: 1_500,
                range_cm: 70_000,
                aim_x_cm: 5_500,
                aim_y_cm: 75_500,
                aim_cell: Some((5, 75)),
                from_x_cm: 5_500,
                from_y_cm: 5_500,
                from_cell: Some((5, 5)),
                from_height_cm: 150,
                damage: 120,
                resolution: Resolution::blind("near", 3, mush_atlas::Cover::Partial),
            },
        )]));
        // `Arena.at_release` and `Arena.pending_catchup`.
        let words = SectionWords {
            condition: SmolStr::new("intact"),
            armour: 10,
            internals: 8,
        };
        round_trip(&BTreeMap::from([(
            unit,
            ReleaseSnapshot {
                at_ms: 35_000,
                cell: Some((5, 12)),
                power: Power::Online,
                contacts: vec![(ContactId(1), Designator::Provisional(1))],
                sections: vec![(SmolStr::new("hull"), words.clone())],
                rounds: vec![(SmolStr::new("cannon"), 40)],
            },
        )]));
        round_trip(&BTreeMap::from([(
            unit,
            Catchup {
                away_ms: 20_000,
                cell_then: Some((5, 12)),
                cell_now: None,
                power_then: Power::Online,
                power_now: Power::Off,
                wreck: false,
                contacts_new: vec![Designator::Provisional(2)],
                contacts_lost: Vec::new(),
                contacts_more: 0,
                sections: vec![(SmolStr::new("hull"), words.clone(), words)],
                sections_more: 0,
                rounds_spent: Vec::new(),
                rounds_more: 0,
                machine: None,
            },
        )]));
        // `MotorTable.rows` (17b cut M1a.1): keyed by the unit, an integer key.
        round_trip(&BTreeMap::from([(unit, MotorRow::default())]));
        round_trip(&BTreeMap::from([(
            unit,
            MotorRow {
                requested_goal: Some((6, 3)),
                outcome: Some(crate::arena::perception::DriveOutcome::Arrived),
                fan: crate::arena::motor::GoalFan::between((6, 3), (6, 6)),
                ..MotorRow::default()
            },
        )]));
        // `Arena.minds` (17b cut M1a.5), keyed by the unit, and inside each row
        // `MindState.ranges`, keyed by a contact id at the top of `u64` — and
        // (17b cut M3.2) a pending answer with an intent of each carried
        // shape the stock mind emits, a drop and a seal digest.
        round_trip(&BTreeMap::from([(
            unit,
            crate::arena::mind::MindState {
                ranges: BTreeMap::from([(ContactId(u64::MAX), 450_u64)]),
                pending: Some(crate::arena::mind::PendingThink {
                    think_seq: u64::MAX,
                    t_ms: 2_482,
                    epochs: [u64::MAX, 1, 2],
                    profile_token: crate::arena::profile_runtime::ProfileToken {
                        unit,
                        generation: 3,
                        key: crate::arena::runtime_card::ProfileKey {
                            definition: mush_core::DefinitionDigest::from_bytes([1; 32]),
                            index: 0,
                        },
                    },
                    lands_ms: 2732,
                    period_ms: 1000,
                    profile_class: crate::arena::core::CoreClass::I,
                    rule_sample: None,
                    intents: vec![
                        IntentKind::Lock {
                            contact: ContactId(u64::MAX),
                        },
                        IntentKind::Fire {
                            weapon: SmolStr::new("cannon"),
                            slot: None,
                        },
                        IntentKind::Condition(crate::arena::conditions::Arming::Disarm {
                            weapon: Some(SmolStr::new("cannon")),
                        }),
                    ],
                    memory: b"{\"k\":1}".to_vec(),
                }),
                dropped: Some(crate::arena::mind::DropCause::Strikes),
                seal_digest: [0xab; 32],
                ..crate::arena::mind::MindState::default()
            },
        )]));
        // 17b cut M1b.1: `ConditionTable.rows`, transparent and keyed by the
        // unit — an integer key, written as an object.
        let mut slots = crate::arena::conditions::ConditionTable::default();
        let _ = slots.arm(
            unit,
            crate::arena::conditions::FireCondition {
                weapon: SmolStr::new("cannon"),
                target: crate::arena::conditions::SlotTarget::Contact {
                    id: ContactId(u64::MAX),
                },
                min_permille: 100,
                aspects: crate::arena::conditions::ASPECTS_ANY,
                min_rounds_left: 0,
                min_range_m: 0,
                max_range_m: u32::MAX,
            },
            crate::arena::core::ClassTable::STOCK
                .row(crate::arena::core::CoreClass::I)
                .slots,
        );
        let text = round_trip(&slots);
        assert!(
            text.starts_with(r#"{"4":[{"action":"fire","condition":{"weapon":"cannon""#),
            "{text}"
        );
        // `UnitState.ready_at_ms`, `UnitState.ammo`, `HeatState.per_shot`.
        round_trip(&BTreeMap::from([(SmolStr::new("cannon"), -1_i64)]));
        round_trip(&BTreeMap::from([(SmolStr::new("cannon"), 40_u32)]));
        // `ArenaConfig.max_units_by_class`: a unit variant is a legal key.
        let caps = round_trip(&BTreeMap::from([
            (UnitClass::Drone, 1_u32),
            (UnitClass::Walker, 2),
        ]));
        assert_eq!(caps, r#"{"walker":2,"drone":1}"#);
        // The opaque unit's own maps (`SectionId`, `ComponentId`), through the
        // whole `TacticalUnit` — its `UnitRepr` already writes its two tuple- and
        // struct-keyed maps as pairs.
        let arena = arena_of(vec![tank(unit, Team::Red, pose(1_500, 1_500))]);
        let tactical = &arena.units.first().expect("one unit").unit;
        let text = serde_json::to_string(tactical).expect("the unit encodes");
        let back: mush_tactics::TacticalUnit = serde_json::from_str(&text).expect("and decodes");
        assert_eq!(&back, tactical);
        // And the squad watch and the expired set as the arena holds them:
        // the set is an array; the table is pairs (above).
        let expired: BTreeSet<SquadId> = BTreeSet::from([SquadId {
            team: Team::Red,
            index: 1,
        }]);
        assert_eq!(round_trip(&expired), r#"[{"team":"red","index":1}]"#);
    }

    /// **The head never parses the state**: a document whose `state` is the
    /// string `x` still answers its head, and only the decode's digest check
    /// finds out.
    #[test]
    fn af1_the_head_is_read_without_the_state() {
        let arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        let snapshot = Snapshot::encode(&arena).expect("an arena between pulses encodes");
        let mut document = snapshot.document.clone();
        document["state"] = serde_json::Value::String("x".to_owned());
        let head = Snapshot::head(&document).expect("the head reads without the state");
        assert_eq!(head.state_hash, snapshot.state_hash);
        assert_eq!(head.at_ms, 0);
        assert_eq!(head.format, super::SNAPSHOT_FORMAT);
        assert_eq!(head.arena, arena.definition.id);
        assert!(matches!(
            Snapshot::decode(&document, (*arena.definition).clone()),
            Err(SnapshotError::DigestMismatch { .. })
        ));
        // And its debug form is the head, never the document.
        let debug = format!("{snapshot:?}");
        assert!(debug.contains("state_bytes"), "{debug}");
        assert!(!debug.contains("units"), "{debug}");
    }

    /// **An arena inside a pulse is refused, never a panic**: the three
    /// per-pulse scratch tables are drained before `step` returns, so a caller
    /// holding one full was not between two pulses.
    #[test]
    fn af1_an_arena_inside_a_pulse_is_refused() {
        let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        arena.spent_hits.insert(1);
        assert_eq!(
            Snapshot::encode(&arena).map(|_| ()),
            Err(SnapshotError::Malformed("taken inside a pulse".to_owned()))
        );
    }
}
