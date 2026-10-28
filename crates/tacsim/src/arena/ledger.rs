//! **The match ledger** (16b cut A.1; doc 16 §4 "Ledger events", §3.1's egress
//! paragraph and "Durability"; D-16-3).
//!
//! One match's black box: JSON lines, chunked at [`CHUNK_BYTES`], **streamed out**
//! as they fill. The arena never holds a store handle, so the ledger is not a log
//! it writes — it is bytes it hands over. Cut T's world half appends the chunks to
//! `log:match.<id>` through `mush_core::Ledger` on its own writer lane and
//! acknowledges them with [`Ledger::mark_flushed`]. That is what keeps arena
//! memory flat over an hour of play while the record survives.
//!
//! # Three bounds, three different answers
//!
//! | Bound | Reached when | What happens |
//! |---|---|---|
//! | `ledger.max_bytes` at 90% | [`Ledger::summaries_enabled`] is `false` | the cadenced `position_summary` stops; events keep flowing |
//! | `ledger.max_bytes` at 100% | [`Ledger::append`] answers [`LedgerFull`] | doc 16 §4: the match is stopped with `Timeout` |
//! | `ledger.max_unflushed_chunks` | [`Ledger::lagged`] past half of it, [`Ledger::overflowed`] past it | cut A.2 journals `lagged`, then stops the match with `Void` |
//!
//! The first two protect the *record*; the third protects *memory*, and it is
//! deliberately the only one that voids a match — a world that cannot drain the
//! egress is a world whose arena is about to grow without limit, and doc 16 §3.1
//! would rather lose the match than the server.
//!
//! # A crash still voids the match
//!
//! Doc 16 §3.1 "Durability": for OOC matches the arena holds all state in memory
//! and there is no resume. The ledger's [`Ledger::digest`] and log id go on the
//! result record, not the ledger itself. Per-pulse state durability is a
//! requirement before IC play, not a cut-A one.
//!
//! **16b cut AF.1** gave the ledger the state a snapshot carries
//! ([`LedgerState`], [`Ledger::to_state`], [`Ledger::from_state`]); the message
//! that ships it is 16b cut AF.2's and the boot that resumes from it AF.3's, so
//! until AF.3 lands the sentence above is still what a crash does.

use std::collections::VecDeque;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use sha2::digest::common::hazmat::{SerializableState, SerializedState};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;

use crate::arena::clock::Tempo;
use crate::arena::core::CoreClass;
use crate::arena::mind::SortieId;
use crate::arena::snapshot::SnapshotError;
use crate::arena::state::{ContactId, EndReason, Team};
use crate::config::LedgerConfig;
use crate::deliberation::order::CharacterRef;

/// One chunk of the streamed ledger, in bytes (doc 16 §3.1: *`Ledger` chunks
/// (16 KiB) leave the arena as they fill*).
pub const CHUNK_BYTES: usize = 16 * 1024;

/// **What a match records** (doc 16 §4 "Ledger events").
///
/// Cut A fills [`EventKind::MatchStarted`], [`EventKind::Tempo`],
/// [`EventKind::Startup`], [`EventKind::PositionSummary`],
/// [`EventKind::Blocked`], [`EventKind::Lagged`], [`EventKind::MatchEnded`],
/// [`EventKind::Aborted`] and [`EventKind::Void`]; **16b cut S.1** fills
/// [`EventKind::Detected`], [`EventKind::Classified`] and [`EventKind::Lost`],
/// which is what A.1's three placeholders were reserved for; **16b cut W.1** fills
/// [`EventKind::Locked`], [`EventKind::Fired`], [`EventKind::Impact`] and
/// [`EventKind::Miss`]; **16b cut I.3** adds the staff levers' three kinds,
/// [`EventKind::TempoFloor`], [`EventKind::Placed`] and
/// [`EventKind::PlaceRefused`]; **16b cut O.2** adds [`EventKind::Emcon`]
/// (twenty-nine kinds); **17b cut M1b.5** adds the card lever's two,
/// [`EventKind::MindCard`] and [`EventKind::MindCardRefused`] (thirty-one);
/// **17b cut M3.3** adds [`EventKind::MindSealed`],
/// [`EventKind::SortieClosed`], [`EventKind::MindFault`] and
/// [`EventKind::MindDropped`] (thirty-five) — defined in 17b cut M3.3 so the
/// kind vocabulary is one reviewed diff; `mind_fault` and `mind_dropped`
/// journalled since 17b cut M3.4 (`Arena::mind_think`, `Arena::mind_land`),
/// `mind_sealed` and `sortie_closed` since 17b cut M3.6 (`Arena::new`,
/// `Arena::close_open_sorties` at the six end sites and `Arena::destroy_unit`).
/// The rest are the
/// kinds doc 16 §4 names and later cuts give fields to; they are declared here so
/// that the kind vocabulary is fixed in one reviewed diff and a later cut adds
/// fields rather than a new event stream. **They still serialise**: an internally
/// tagged unit variant is `{"kind":"hit"}`, which is a valid, if uninformative,
/// line.
///
/// **The `kind` word of every variant is stable.** Cut S.1 gave three of them
/// fields and cut W.1 gave four more, which adds keys to those objects and changes
/// neither the tag nor any other kind's encoding, so every consumer keyed on `kind`
/// still matches and every kind an earlier cut actually wrote is byte-identical. The
/// ones that changed are exactly the ones nothing could yet write: no cut before S.1
/// could schedule a sensor pass and none before W.1 could accept a shot, so no
/// ledger anywhere holds a field-less `{"kind":"fired"}` line for the new shape to
/// fail to read.
///
/// **16b cut M.3 is the exception, and says so**: `blocked` gained `cause` (16c
/// `T-04`), `hit` gained `armor_section` (`T-13`), and every line's `draw` key
/// became `draws` (`T-07`). The kind words did not move. The project is
/// pre-launch, so this is a new shape and not a migration: the only readers are
/// this crate's own — the bridge reads each chunk's last `position_summary` and
/// its last `t_ms` as it arrives, with the shape it was built with — and the
/// tests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// The first event of every match.
    MatchStarted,
    /// The adaptive tempo changed (doc 16 §3.1: *tempo changes are ledger
    /// events*).
    Tempo {
        /// The tempo left.
        from: Tempo,
        /// The tempo entered.
        to: Tempo,
    },
    /// A unit accepted `STARTUP` and is now `starting`.
    Startup {
        /// The unit.
        unit: ObjId,
    },
    /// **A unit's startup finished and its power is up** (16b cut Q.1; doc 16
    /// §3.3 "Startup"), at the [`crate::arena::state::DueKind::StartupComplete`]
    /// event's own arena timestamp.
    ///
    /// Journalled only when the flip `starting → online` actually happens, so a
    /// unit whose startup was overtaken (a wreck, a second `STARTUP`) writes no
    /// line. It is what the pilot's `Power online.` is a projection of, exactly as
    /// every other notice is a projection of a journalled event.
    PowerOnline {
        /// The unit.
        unit: ObjId,
    },
    /// Every `ledger.summary_ms` of arena time, in every tempo: every unit as
    /// `(unit, x_cm, y_cm, z_cm, heading_mdeg)`. **There is no per-pulse `moved`
    /// event** (doc 16 §4) — this cadence is the position record.
    PositionSummary {
        /// Every unit, in unit-id order.
        units: Vec<(ObjId, i64, i64, i32, u32)>,
    },
    /// A unit was stopped at a cell boundary (doc 16 §3.4 step 4): one notice per
    /// stop, naming the cell it could not enter and why.
    ///
    /// **16b cut M.3** (16c `T-04`, §5's Q22): journalled on the new edge of
    /// [`crate::arena::state::UnitState::held`] — a stop at a cell or for a
    /// cause the unit was not already held against — and nothing else, so one
    /// stop is one line however the pulse is cut and however long a standing
    /// order pushes the unit at the obstacle. Until this cut it fired on a
    /// per-substep flag's rising edge, whose count was a function of the
    /// partition and was unbounded against a bank.
    Blocked {
        /// The unit.
        unit: ObjId,
        /// The refused column.
        x: u16,
        /// The refused row.
        y: u16,
        /// **Why** (16b cut M.3): the
        /// [`crate::arena::motion::BlockedCause::as_str`] word — `impassable`,
        /// `too_steep`, `occupied` or `edge`. It is what the reflex controller's
        /// avoid list is fed off (an `occupied` stop), and what the pilot's own
        /// sentence already said.
        cause: crate::arena::motion::BlockedCause,
    },
    /// **A viewer began holding a contact** (16b cut S.1; doc 16 §3.6 step 4).
    ///
    /// The contact id is the arena's own opaque identity
    /// ([`crate::arena::state::ContactId`], sixteen base-16 digits on the wire),
    /// and `target` is the object it is held on. Both are in the *record* and
    /// neither ever reaches the viewer who holds it: doc 16 §3.6 lets a pilot
    /// name the id and nothing else. The ledger is the black box, not a view.
    Detected {
        /// The platform that now holds it.
        viewer: ObjId,
        /// The opaque contact id.
        contact: ContactId,
        /// What is held.
        target: ObjId,
    },
    /// **A viewer told a contact apart** (16b cut S.1; doc 16 §3.6 step 4, W5).
    ///
    /// Carries the `classify:<observer>:<target>:<pass_seq>` draw and its value in
    /// [`LedgerEvent::draws`] when the classification was the viewer's own pass; a
    /// classification that arrived over the comms mesh (step 5) carries none,
    /// because no draw was made for it — the sharer's pass already recorded one.
    Classified {
        /// The platform.
        viewer: ObjId,
        /// The opaque contact id.
        contact: ContactId,
        /// What is held.
        target: ObjId,
        /// The side it was resolved to.
        team: Team,
    },
    /// **A viewer lost a contact** (16b cut S.1; doc 16 §3.6 step 4's TTL:
    /// `sensors.pass_ms × sensors.lost_after_passes` without a refresh).
    Lost {
        /// The platform.
        viewer: ObjId,
        /// The opaque contact id it held it under.
        contact: ContactId,
        /// What it was holding.
        target: ObjId,
    },
    /// **A platform's own receiver crossed into or out of interference** (16b cut
    /// D.4, `B-22`; doc 16 §9 items 4 and 5), at its sensor pass's own arena
    /// timestamp.
    ///
    /// One line per **transition** and none for a change of degree inside a
    /// bubble, which is what the pilot's notice is a projection of. The line says
    /// which platform and whether it is degraded, and — like every other line in
    /// here — it is the *record*: the black box may name the unit, and what the
    /// pilot is told carries no source at all
    /// ([`crate::arena::perception::Receiver`]).
    ///
    /// A pass that stops running altogether (a wreck, a sight shot off) writes no
    /// line: the receiver is off rather than clear.
    Receiver {
        /// The platform.
        unit: ObjId,
        /// Whether its receiver is now degraded.
        degraded: bool,
    },
    /// **A platform's jamming pod was switched** (16b cut O.2, 16c N12), at the
    /// arena millisecond phase 4 resolved the pilot's `EMCON` at.
    ///
    /// Journalled only on a real flip
    /// ([`crate::arena::intents::resolve_emcon`]): an order that finds the switch
    /// already where it puts it writes no line. What the sights inside the bubble
    /// made of it is each one's own `receiver` line, at its own next pass.
    Emcon {
        /// The platform whose pod it is.
        unit: ObjId,
        /// Whether the pod now radiates.
        on: bool,
    },
    /// **A platform locked a contact** (16b cut W.1; doc 16 §3.7's `LOCK`).
    ///
    /// The opaque contact id is what the pilot named; `target` is the object it is
    /// held on, which is in the *record* and never reaches the pilot — the same rule
    /// [`EventKind::Detected`] follows.
    Locked {
        /// The platform that locked.
        unit: ObjId,
        /// The contact it locked.
        contact: ContactId,
        /// What that contact is held on.
        target: ObjId,
    },
    /// **A shot was accepted** (16b cut W.1; doc 16 §3.5 "Acceptance and cost").
    ///
    /// One line per accepted shot, whatever it later resolves as: doc 16 §3.5's
    /// *there is no free probe of line of sight* means a shot that could never have
    /// hit is in the record beside one that does.
    Fired {
        /// Who fired.
        unit: ObjId,
        /// The weapon's component id.
        weapon: SmolStr,
        /// The shot's sequence within the match — the argument of its draw labels
        /// (doc 16 §3.5) and the key the `impact`, `hit` and `miss` lines carry.
        shot_seq: u64,
        /// What it was fired at.
        target: ObjId,
        /// The range at acceptance, centimetres.
        range_cm: u64,
        /// How many rounds left the weapon (the burst, or what was left of it).
        rounds: u32,
        /// **Who fired, where the law asks** (17b cut M1b.1; doc 17 §5,
        /// D-UM-14): `"cond"` for a condition slot's own shot, `"mind"` for a
        /// mind's cited shot, absent for a seat's and a machine's — so every
        /// line before this cut keeps its bytes, and a bare legacy line reads.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<SmolStr>,
        /// **The slot the shot was fired by or cites** (17b cut M1b.1), absent
        /// when `by` is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<u8>,
    },
    /// **A shot's flight time elapsed** (16b cut W.1; doc 16 §3.5 "Scheduled
    /// impact"). The outcome is the `hit` or `miss` line that follows it.
    Impact {
        /// The shot's sequence.
        shot_seq: u64,
        /// Who fired it.
        unit: ObjId,
        /// What it was fired at.
        target: ObjId,
    },
    /// **A shot struck** (16b cut W.2; doc 16 §3.5 "Damage": *one `hit` ledger
    /// event carrying the plan's cause*).
    ///
    /// One line per hit, carrying the whole of what the impact's single
    /// [`mush_tactics::UnitMutationPlan`] did: which section the `facet:<shot_seq>`
    /// draw chose, which facet's armour the round passed, how much damage arrived,
    /// what is left of both cells afterwards, and the consequences that fired. The
    /// draws ride [`LedgerEvent::draws`], as doc 16 §4 asks — **both of them**
    /// since 16b cut M.3 (16c `T-07`): the shot's own `hit:<seq>` or
    /// `burst:<seq>` first, then the `facet:<seq>` that chose the section, so the
    /// section a round found can be re-derived from the seed and the line.
    ///
    /// `armor_left` and `internals_left` are **after** the plan, so a reader of the
    /// black box never has to know a unit's authored maxima to see a tank coming
    /// apart.
    Hit {
        /// The shot's sequence.
        shot_seq: u64,
        /// Who fired it.
        unit: ObjId,
        /// What it struck.
        target: ObjId,
        /// The section the facet draw chose.
        section: SmolStr,
        /// The facet whose `armor` cell the round passed — the struck-through
        /// section's, for a section with no armour of its own (doc 16 §3.3).
        facet: SmolStr,
        /// **The section whose `facet` armour the round passed** (16b cut M.3,
        /// 16c `T-13`): the struck-through section's for a section with no armour
        /// of its own (the engine, through `hull`), else `section` itself. Until
        /// this cut a struck-through hit named the chosen section beside another
        /// section's facet, and `armor_left` could not be read without the
        /// content.
        armor_section: SmolStr,
        /// How many rounds of the shot landed: `1` for a single round, the
        /// binomial sample for a burst.
        rounds: u32,
        /// The damage they carried together.
        damage: u32,
        /// The armour left in that facet — `armor_section`'s — afterwards.
        armor_left: u32,
        /// The internals left in that section afterwards.
        internals_left: u32,
        /// The consequences that fired, by their authored words; empty unless the
        /// section's internals reached zero on this hit.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        consequences: Vec<SmolStr>,
        /// The mutation plan's own cause tag, which is what ties this line to the
        /// M32 journal entry the same plan wrote.
        cause: SmolStr,
    },
    /// **A shot did not strike, with its reason** (16b cut W.2; doc 16 §3.5: *a
    /// failed validation is a `miss` with its reason*).
    ///
    /// The vocabulary is [`crate::arena::damage::MISS_REASONS`] and nothing else:
    /// the four validations doc 16 names, the full cover a shot is accepted into
    /// anyway, the dispersion that is the draw's own answer, and the refusal M32
    /// cannot give for a plan this engine builds.
    Miss {
        /// The shot's sequence.
        shot_seq: u64,
        /// Why it missed — one word of [`crate::arena::damage::MISS_REASONS`].
        reason: SmolStr,
    },
    /// **A section reached zero internals** (16b cut W.2; doc 16 §3.3's
    /// *Consequence at zero internals* column), with the consequences that fired.
    ///
    /// It follows the `hit` that finished the section off and carries the same
    /// words, because the two answer different questions: the `hit` is what one
    /// round did and this is what the unit lost.
    SectionDestroyed {
        /// The unit.
        unit: ObjId,
        /// The section.
        section: SmolStr,
        /// **Who did it** (16b cut D.4, `T-08`): the shooter whose round finished
        /// the section off, and `None` for a section lost to anything else — a
        /// test rig's direct call, or a consequence that did not come off a round.
        ///
        /// [`EventKind::Impact`] has carried the pair since cut W.1 and this is
        /// the same fact on the line that needs it: the staging scan used to
        /// credit whichever impact happened to be in flight, which was a stale
        /// read the moment anything but a round could destroy a section.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<ObjId>,
        /// Its consequences, by their authored words.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        consequences: Vec<SmolStr>,
    },
    /// **A unit was disabled** — no weapons and no motion — which precedes
    /// destroyed (doc 10 3A; 16b cut W.2). Said once, on the transition.
    UnitDisabled {
        /// The unit.
        unit: ObjId,
    },
    /// **A unit became a wreck** (16b cut W.2; doc 16 §3.3 "A wreck"). Said once.
    UnitDestroyed {
        /// The unit.
        unit: ObjId,
        /// **Who did it** (16b cut D.4, `T-08`), on the rule
        /// [`EventKind::SectionDestroyed`]'s `by` follows: the shooter whose round
        /// finished the tank, and `None` for a hull destroyed by anything else.
        /// It is what the gunner's kill line is now gated on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<ObjId>,
    },
    /// **An order landed on a unit** (16b cut Y.1; doc 16 §6 "Deliberation": *an
    /// order (objective, target, horizon, constraints) with a time to live*).
    ///
    /// One line per **unit** the order landed on, at the arena millisecond
    /// `crate::arena::intents::apply_intent` took it at — a pilot's `ORDER ALL`
    /// over two squads of two is four lines at one `t_ms` under the phase's one
    /// hash. A refused order journals nothing: the line records what the arena
    /// accepted, not what anybody asked for.
    ///
    /// `target` is the order's preferred contact **as the order carries it**: the
    /// issuer's opaque contact id (a pilot's own, or the squad commander's when
    /// `issuer` is `None`). It is never a target object id — the black box records
    /// what was said, and which object that named is the reflex row's business.
    Ordered {
        /// The unit the order landed on.
        unit: ObjId,
        /// The squad the order was written for, rendered (`red-1`).
        squad: SmolStr,
        /// The unit whose scope the hint is written in, `None` for the squad's
        /// own commander (the LLM seam).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issuer: Option<ObjId>,
        /// The objective, by its word.
        objective: SmolStr,
        /// The issuer's opaque contact id the order points at, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<ContactId>,
        /// The commander's word for an `advance` cell (a key spot's name).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spot: Option<SmolStr>,
        /// Whether the order forbids shooting.
        hold_fire: bool,
        /// Its time to live, milliseconds of arena time.
        ttl_ms: u32,
    },
    /// **A squad's standing order ran out unrenewed** (16b cut N.2; doc 16 §6
    /// "Provider failure": *the visible in-world effect is a notice when an order
    /// expires unrenewed* — since 17b cut L.2 (D-UM-32 as amended by R-C) the
    /// leader's order lapsing, in the leader's name, never "the link dropped",
    /// which is M3's word for a mind's own failure).
    ///
    /// One line per **unit**, at the arena millisecond that unit's next mind
    /// landing reads the expiry (its next reflex tick until 17b cut M4.6) — not
    /// at the millisecond the order nominally ran out, and
    /// deliberately: the order stops steering when the machine next looks at it,
    /// and a black box that said otherwise would disagree with what the tank did.
    ///
    /// The line is the record; the sentence a person reads is the world's, said
    /// once per squad by the bridge that issued the order. Both carry the lapse's
    /// meaning — the order ran out and the unit is back on doctrine — and neither
    /// is derived from the other, which is the same split every notice in this
    /// engine has.
    ///
    /// **16b cut E.2 (16c `B-41`) gave it a second cause**, and the line is
    /// unchanged because what it records is still true of it: *the unit that
    /// dropped to doctrine*. An `Objective::Advance` whose cell no route reaches
    /// ends **for that unit** at the tick whose search failed, and journals this
    /// same line with the word `advance`. The bridge's own
    /// lapse sentence is said off its wall-clock watch of the
    /// order's expiry (`crate::arena_bridge::say_expired_orders`) and not off
    /// this line, so no pilot hears anything new or sooner; what the squad gets
    /// is `crate::arena::command::SignalReason::OrderExpired` on its next
    /// signal, and the commander decides again.
    OrderExpired {
        /// The unit that dropped to doctrine.
        unit: ObjId,
        /// The squad the order was written for, rendered (`red-1`).
        squad: SmolStr,
        /// The objective it was carrying, by its word.
        objective: SmolStr,
    },
    /// **A squad's standing order was handed back to doctrine** (16b cut Z.2;
    /// 16c Q44): a pilot's `ORDER <squad> RELEASE` landed on a unit whose order
    /// was written for that squad.
    ///
    /// One line per **unit** the release dropped, at the arena millisecond
    /// `crate::arena::intents::apply_intent` took it at, under phase 2's one
    /// hash exactly as the `ordered` line is. A release to a unit under no order,
    /// or under another squad's, journals nothing.
    ///
    /// It is its own kind and not an `order_expired` line on purpose: the debrief
    /// must not say an order ran out when a pilot ended it.
    OrderReleased {
        /// The unit that dropped to doctrine.
        unit: ObjId,
        /// The squad the order was written for, rendered (`red-1`).
        squad: SmolStr,
        /// The tank whose pilot released it, `None` for a release nobody seated
        /// gave.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issuer: Option<ObjId>,
    },
    /// **Staff set or lifted a tempo floor** (16b cut I.3; 16c N10:
    /// `@arena/tempo`): one line per change, at the pulse's end, beside the
    /// `tempo` line and under the same phase-5 hash.
    ///
    /// The floor is a view of the wake cadence and no hash folds it; this line
    /// moves the ledger's sequence like every line, which is why a floored run is
    /// only ever compared with another floored run.
    TempoFloor {
        /// The floor now standing, milliseconds; `None` (`null`) for a lift.
        period_ms: Option<u32>,
    },
    /// **Staff put a unit on a cell** (16b cut I.3; 16c N10: `@arena/place`),
    /// at the arena millisecond phase 4 resolved it, under phase 4's one hash.
    /// Never said to anybody: the pilot's next frame carries the new cell.
    Placed {
        /// The unit placed.
        unit: ObjId,
        /// The column it stands in now.
        x: u16,
        /// The row.
        y: u16,
        /// The column it stood in, `u16::MAX` when it stood off the window.
        from_x: u16,
        /// The row it stood in, `u16::MAX` likewise.
        from_y: u16,
    },
    /// **A staff place the arena refused** (16b cut I.3): every `Place` the
    /// arena receives leaves a line, this or [`EventKind::Placed`]. Journalled
    /// under phase 2's hash for the three gates it meets there and under phase
    /// 4's for the entry test; never staged to a pilot, because a refusal naming
    /// a round in flight would tell of an impact their sensors never saw.
    PlaceRefused {
        /// The unit named.
        unit: ObjId,
        /// The column asked for.
        x: u16,
        /// The row asked for.
        y: u16,
        /// Why: `impassable`, `occupied` or `in_flight` from the entry test, or
        /// phase 2's `unknown`, `epoch` or `channel`
        /// ([`crate::arena::intents::Refusal::as_str`]).
        reason: SmolStr,
    },
    /// **A mind's card changed** (17b cut M1b.5; doc 17 §11, §22's M1b,
    /// D-UM-43): a staff `@mind/live` landed on the arena's OWN row of a
    /// [`crate::arena::state::Controller::Mind`] hull at the start of the
    /// pulse after it was offered (`Arena::apply_mind_cards`, phase 2's head),
    /// under phase 2's one hash — or (**17b cut M5.2**; doc 17 §11, D-UM-44)
    /// a seat's `MIND PROFILE` ([`crate::arena::intents::IntentKind::Profile`])
    /// landing in phase 2 on a seated hull — its dormant row's card rewritten
    /// when one stands — under the same hash: a recorded intent, its `card` the
    /// switched-to profile's preset, the card that will fly at the release.
    /// Read back to staff, and to the hull's own pilot in their report (17b
    /// cut M5.6).
    ///
    /// It carries the exact immutable card/program key. The complete payload is
    /// shared in the captured arena catalog; the own-hull frame publishes the
    /// selected active card and program for authorized readback.
    /// Never said to a pilot as it lands.
    MindCard {
        /// The hull.
        unit: ObjId,
        /// The card's name after the change (`StockCard::name_of`: `Sniper`,
        /// `Sniper, edited (min_odds 70)`).
        name: SmolStr,
        /// The sliders the change moved, in doc 17 §11's order (empty for a
        /// change that moved none).
        changed: Vec<SmolStr>,
        /// Exact immutable card/program identity; payload is in the arena catalog.
        card: crate::arena::runtime_card::ProfileKey,
        /// Derived compatibility view for the staff stock-slider mirror. Custom
        /// cards without the complete stock fields have no such view. The key
        /// above remains the authoritative full card/program identity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stock: Option<super::card::StockCard>,
    },
    /// **A card change the arena refused** (17b cut M1b.5): every
    /// [`crate::arena::card::CardChange`] the arena takes leaves a line, this
    /// or [`EventKind::MindCard`], and a refused one changes nothing.
    MindCardRefused {
        /// The hull named.
        unit: ObjId,
        /// Why: `not_mind` (no `Controller::Mind` hull with a row),
        /// `unlawful` (the card's `min_odds` under the arena's own Compact
        /// floor), `no preset named …`, or a slider's own refusal
        /// ([`crate::arena::card::CardError`]'s sentence).
        reason: SmolStr,
    },
    /// **A sortie opened under its seal** (17b cut M3.3 defines it; doc 17
    /// §13, §19; D-UM-45, D-UM-46): one line per unit at the match's opening,
    /// so the ledger's digest attests which minds flew. **Journalled since 17b
    /// cut M3.6** (`Arena::new`, in unit-id order right after `match_started`,
    /// at 0 under the opening hash) — the line shifts the folded ledger
    /// sequence of every match, and that cut is the one that re-blessed.
    MindSealed {
        /// The hull.
        unit: ObjId,
        /// Its sortie ([`crate::arena::mind::SortieId::in_match`]).
        sortie: SortieId,
        /// The sealed mind's name (`Standard Doctrine`).
        mind: SmolStr,
        /// The first sixteen base-16 characters of the sealed source's
        /// SHA-256 (`crate::arena::mind::MindSeal::source_sha256`).
        source: SmolStr,
        /// All sixty-four of the card's digest.
        card_digest: SmolStr,
        /// The class the mind thinks at.
        class: CoreClass,
        /// Whether the core runs under the governor.
        governed: bool,
        /// The licence the core holds, or `doctrine`.
        licence: SmolStr,
        /// **The state that licence is read under** (doc 17h cut M8a; doc 17 §10): `doctrine`,
        /// `draft`, or a filed licence's `active` / `grace` / `pending` / `lapsed` / `revoked` /
        /// `recalled`, derived at the site that seals and passed to the line
        /// (`MindSeal::sealed_line`'s parameter, never a seal field). M8a re-blessed every tape
        /// that prints this line.
        licence_state: crate::minds::licence_state::SealLicenceState,
        /// The warrant principal, absent on the wire where nobody chose.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        principal: Option<CharacterRef>,
    },
    /// **A sortie closed** (17b cut M3.3 defines it; doc 17 §12, §13): at the
    /// match's end (`match_end`, every open sortie, immediately before the
    /// end's own line) or at the hull's destruction (`destroyed`). Journalled
    /// since 17b cut M3.6 (`Arena::close_open_sorties`, `Arena::destroy_unit`),
    /// once per sortie (`SealRef::closed`); a ledger-full end and a void at
    /// boot close nothing on the tape.
    SortieClosed {
        /// The hull.
        unit: ObjId,
        /// Its sortie.
        sortie: SortieId,
        /// `match_end` or `destroyed`.
        word: SmolStr,
    },
    /// **A mind's think faulted** (17b cut M3.3 defines it; doc 17 §8;
    /// D-UM-31): the think landed nothing and kept its old memory. Journalled
    /// since 17b cut M3.4 by the think arm (`Arena::mind_think`) at the
    /// think's millisecond. A think opens no action hash and fills none: the
    /// line carries the substep's hash when an earlier line of the substep
    /// has taken it, else a fresh state hash that stores nothing — even under
    /// a lazy slot a landing (or, until 17b cut M4.6, a reflex tick) opened and
    /// has not yet filled
    /// (`Arena::think_line_hash`), so no later line's hash moves.
    MindFault {
        /// The hull.
        unit: ObjId,
        /// The faulted think's sequence number.
        think_seq: u64,
        /// The fault's cause word.
        cause: SmolStr,
    },
    /// **A hull dropped to the ROM** (17b cut M3.3 defines it; doc 17 §8,
    /// §13; D-UM-31, D-UM-32): once per sortie. It carries what the
    /// principal's sentence needs, so the world tells it from the tape and
    /// reads no live config (16b cut V.3's rule). Journalled since 17b cut
    /// M3.4: on `strikes` by the think arm beside the third `mind_fault`,
    /// under its hash; on `core` by the landing arm under the landing's hash,
    /// or by the think arm when no landing is pending (under the think's, as
    /// `mind_fault`'s); on `abi` by nothing in the arena (a resume refuses by
    /// name).
    MindDropped {
        /// The hull.
        unit: ObjId,
        /// `strikes`, `core` or `abi`
        /// ([`crate::arena::mind::DropCause::as_str`]).
        cause: SmolStr,
        /// The last faulting think's cause word — on `strikes` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fault: Option<SmolStr>,
        /// The op budget the faulting thinks were charged against — on
        /// `strikes` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ops: Option<u64>,
    },
    /// A pulse absorbed more wall time than `max_catch_up_steps × period`, or the
    /// egress backed up past `ledger.max_unflushed_chunks`.
    Lagged {
        /// The clamped `dt` the pulse ran with, milliseconds.
        dt_ms: u32,
    },
    /// The match ended.
    MatchEnded {
        /// Why.
        reason: EndReason,
    },
    /// `SIM ABORT`.
    Aborted,
    /// The match is void: nothing it derived may be committed.
    Void,
}

impl EventKind {
    /// The kind word this event serialises under — the `kind` field's value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            EventKind::MatchStarted => "match_started",
            EventKind::Tempo { .. } => "tempo",
            EventKind::Startup { .. } => "startup",
            EventKind::PowerOnline { .. } => "power_online",
            EventKind::PositionSummary { .. } => "position_summary",
            EventKind::Blocked { .. } => "blocked",
            EventKind::Detected { .. } => "detected",
            EventKind::Classified { .. } => "classified",
            EventKind::Lost { .. } => "lost",
            EventKind::Receiver { .. } => "receiver",
            EventKind::Emcon { .. } => "emcon",
            EventKind::Locked { .. } => "locked",
            EventKind::Fired { .. } => "fired",
            EventKind::Impact { .. } => "impact",
            EventKind::Hit { .. } => "hit",
            EventKind::Miss { .. } => "miss",
            EventKind::SectionDestroyed { .. } => "section_destroyed",
            EventKind::UnitDisabled { .. } => "unit_disabled",
            EventKind::UnitDestroyed { .. } => "unit_destroyed",
            EventKind::Ordered { .. } => "ordered",
            EventKind::OrderExpired { .. } => "order_expired",
            EventKind::OrderReleased { .. } => "order_released",
            EventKind::TempoFloor { .. } => "tempo_floor",
            EventKind::Placed { .. } => "placed",
            EventKind::PlaceRefused { .. } => "place_refused",
            EventKind::MindCard { .. } => "mind_card",
            EventKind::MindCardRefused { .. } => "mind_card_refused",
            EventKind::MindSealed { .. } => "mind_sealed",
            EventKind::SortieClosed { .. } => "sortie_closed",
            EventKind::MindFault { .. } => "mind_fault",
            EventKind::MindDropped { .. } => "mind_dropped",
            EventKind::Lagged { .. } => "lagged",
            EventKind::MatchEnded { .. } => "match_ended",
            EventKind::Aborted => "aborted",
            EventKind::Void => "void",
        }
    }
}

/// One ledger line (doc 16 §4: `{seq, t_ms, kind, ...}` *with the substep state
/// hash*, *carrying the draw labels and values where draws occurred*).
///
/// The kind is **flattened**, so the wire form is the flat object doc 16 names —
/// `{"seq":1,"t_ms":0,"kind":"match_started","hash":123}` — rather than a nested
/// one. `draws` is omitted entirely when no draw was made, which is most events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEvent {
    /// The event's sequence number within the match, from `1`.
    pub seq: u64,
    /// Arena milliseconds.
    pub t_ms: i64,
    /// What happened.
    #[serde(flatten)]
    pub kind: EventKind,
    /// **Every draw the event made**, in the order it made them: each draw's
    /// encoded label ([`crate::arena::rng::DrawLabel::encode`]) and its value.
    ///
    /// A list since 16b cut M.3 (16c `T-07`), because a `hit` makes two — the
    /// shot's `hit:<seq>` or `burst:<seq>`, then the `facet:<seq>` that chose the
    /// section — and the second never reached the black box while this was one
    /// `Option`. So does a `miss` whose reason is `refused` when it is written
    /// after the section was chosen (the plan could not be built or applied): the
    /// facet draw was made, so the line carries the shot's draw and then
    /// `facet:<seq>`, exactly as a `hit` does. A `miss` for dispersion carries
    /// one — its shot draw, including a round that arrived at a band with no
    /// facet table, where no facet was drawn — and so does a sensor `classified`,
    /// its `classify:` draw. Everything else carries none. With the seed on
    /// the result document (`crate::match_::record::write_result`), a reader can
    /// re-derive each value with [`crate::arena::rng::draw`], decoding the label
    /// with [`crate::arena::rng::DrawLabel::decode`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub draws: Vec<(String, u64)>,
    /// The substep state hash this event was recorded against
    /// ([`crate::arena::hash::state_hash`]).
    pub hash: u64,
}

/// A closed chunk of the streamed ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerChunk {
    /// The chunk's sequence number within the match, from `1`. It is what
    /// [`Ledger::mark_flushed`] acknowledges.
    pub seq: u64,
    /// Whole JSON lines, each newline-terminated.
    pub bytes: Vec<u8>,
}

/// `ledger.max_bytes` is reached: the event was **not** recorded.
///
/// Doc 16 §4: at 100% of `max_bytes` the match is stopped with `Timeout`. Cut
/// A.2's step turns this into that stop; a caller that cannot stop (cut A's own
/// two appends) drops the event and carries on, because a black box that has run
/// out of tape is not a reason to lose the match in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "the match ledger has reached its {max_bytes}-byte bound at {bytes_total} bytes: no further \
     event is recorded"
)]
pub struct LedgerFull {
    /// Bytes recorded so far.
    pub bytes_total: u64,
    /// The configured bound.
    pub max_bytes: u64,
}

/// **The streamed match ledger and its bounded egress.**
///
/// Chunks are closed on whole lines: a line that would carry the open chunk past
/// [`CHUNK_BYTES`] closes it first, so a chunk is never more than `CHUNK_BYTES`
/// and never holds a partial JSON line. A single line longer than `CHUNK_BYTES`
/// (which no shipped event produces) gets a chunk of its own.
pub struct Ledger {
    cfg: LedgerConfig,
    /// The next event's sequence, minus one: the last appended sequence.
    seq: u64,
    /// The chunk being filled.
    open: Vec<u8>,
    /// Chunks closed and not yet handed to the egress.
    closed: VecDeque<LedgerChunk>,
    /// The sequence of the most recently closed chunk; `0` before the first.
    last_closed_seq: u64,
    /// The highest chunk sequence the world has acknowledged.
    flushed_upto: u64,
    /// Every byte appended, in order.
    bytes_total: u64,
    /// The running digest over every appended byte.
    digest: Sha256,
    /// **The running digest over every byte handed out in a chunk** (16b cut
    /// M.1, `B-48`), updated by [`Ledger::take_chunks`] over each chunk's
    /// `bytes` in chunk order as it leaves. It is what the world's
    /// `log:match.<id>` can reproduce: see [`Ledger::shipped_digest`].
    shipped: Sha256,
}

/// **What a snapshot carries of the ledger** (16b cut AF.1; doc 16 §3.1
/// "Durability"): the counters, the chunk being filled, the closed chunks not
/// yet handed to the egress, and the two SHA-256 running states — everything
/// [`Ledger`] holds but its configuration, which the snapshot carries beside it
/// in the frozen `[tacsim.arena.ledger]` block.
///
/// The open buffer and each closed chunk are `String`s because they are JSON
/// lines, which are UTF-8 by construction; a snapshot whose text is not decodes
/// nowhere (the state's own parse refuses it). The two digests are the
/// `SerializableState` form of `sha2`'s running state, so a resumed ledger's
/// [`Ledger::digest`] and [`Ledger::shipped_digest`] are the digests of every
/// byte the match has appended and shipped since it opened, and not only of
/// the ones this process saw.
///
/// It carries **the open chunk, not the tape**: the chunks already handed to
/// the egress are the world's to keep (16b cut AF.2), and `flushed_upto` is
/// carried as it stood (16b cut AF.3 sets it on the resume).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerState {
    seq: u64,
    open: String,
    closed: Vec<(u64, String)>,
    last_closed_seq: u64,
    flushed_upto: u64,
    bytes_total: u64,
    digest: Vec<u8>,
    shipped: Vec<u8>,
}

impl Clone for Ledger {
    fn clone(&self) -> Ledger {
        Ledger {
            cfg: self.cfg.clone(),
            seq: self.seq,
            open: self.open.clone(),
            closed: self.closed.clone(),
            last_closed_seq: self.last_closed_seq,
            flushed_upto: self.flushed_upto,
            bytes_total: self.bytes_total,
            digest: self.digest.clone(),
            shipped: self.shipped.clone(),
        }
    }
}

impl std::fmt::Debug for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger")
            .field("seq", &self.seq)
            .field("bytes_total", &self.bytes_total)
            .field("open", &self.open.len())
            .field("closed", &self.closed.len())
            .field("last_closed_seq", &self.last_closed_seq)
            .field("flushed_upto", &self.flushed_upto)
            .finish()
    }
}

impl Ledger {
    /// An empty ledger under `cfg`.
    #[must_use]
    pub fn new(cfg: &LedgerConfig) -> Ledger {
        Ledger {
            cfg: cfg.clone(),
            seq: 0,
            open: Vec::new(),
            closed: VecDeque::new(),
            last_closed_seq: 0,
            flushed_upto: 0,
            bytes_total: 0,
            digest: Sha256::new(),
            shipped: Sha256::new(),
        }
    }

    /// **The ledger as a snapshot carries it** (16b cut AF.1): see
    /// [`LedgerState`]. It copies the open chunk and the closed queue, which
    /// between two pulses is at most one chunk's worth each (the task takes the
    /// closed ones after every pulse).
    #[must_use]
    pub fn to_state(&self) -> LedgerState {
        LedgerState {
            seq: self.seq,
            // Every byte in here is a `serde_json` line or the `encode_failed`
            // line `append` writes, all UTF-8, so the lossy branch is
            // unreachable; it is spelled lossy only so that this is not a panic.
            open: String::from_utf8_lossy(&self.open).into_owned(),
            closed: self
                .closed
                .iter()
                .map(|chunk| {
                    (
                        chunk.seq,
                        String::from_utf8_lossy(&chunk.bytes).into_owned(),
                    )
                })
                .collect(),
            last_closed_seq: self.last_closed_seq,
            flushed_upto: self.flushed_upto,
            bytes_total: self.bytes_total,
            digest: self.digest.serialize().to_vec(),
            shipped: self.shipped.serialize().to_vec(),
        }
    }

    /// **A ledger back from a snapshot** (16b cut AF.1), under `cfg` — the
    /// frozen `[tacsim.arena.ledger]` block the snapshot carries beside it.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::Malformed`] when either digest state is not one
    /// `sha2`'s SHA-256 can resume from.
    pub fn from_state(cfg: &LedgerConfig, state: LedgerState) -> Result<Ledger, SnapshotError> {
        let LedgerState {
            seq,
            open,
            closed,
            last_closed_seq,
            flushed_upto,
            bytes_total,
            digest,
            shipped,
        } = state;
        Ok(Ledger {
            cfg: cfg.clone(),
            seq,
            open: open.into_bytes(),
            closed: closed
                .into_iter()
                .map(|(seq, bytes)| LedgerChunk {
                    seq,
                    bytes: bytes.into_bytes(),
                })
                .collect(),
            last_closed_seq,
            flushed_upto,
            bytes_total,
            digest: resume_sha256(&digest, "digest")?,
            shipped: resume_sha256(&shipped, "shipped")?,
        })
    }

    /// **Record one event**, answering its sequence number.
    ///
    /// The line is `serde_json`'s encoding of a [`LedgerEvent`] plus a newline.
    /// An encoding failure — which none of the shipped kinds can produce, all of
    /// them being integers, strings and enums — is recorded as a
    /// `{"kind":"encode_failed"}` line of the same sequence rather than dropped
    /// silently or escalated into an error the caller cannot act on: the black
    /// box records that something happened even when it cannot say what.
    ///
    /// # Errors
    ///
    /// [`LedgerFull`] when the line would carry the ledger past
    /// `ledger.max_bytes`. Nothing is recorded and the sequence is not consumed,
    /// so a caller that stops the match and appends `match_ended` will find no
    /// gap in the numbering.
    pub fn append(
        &mut self,
        t_ms: i64,
        kind: EventKind,
        draws: Vec<(String, u64)>,
        hash: u64,
    ) -> Result<u64, LedgerFull> {
        let seq = self.seq.saturating_add(1);
        let event = LedgerEvent {
            seq,
            t_ms,
            kind,
            draws,
            hash,
        };
        let mut line = serde_json::to_vec(&event).unwrap_or_else(|_| {
            format!("{{\"seq\":{seq},\"t_ms\":{t_ms},\"kind\":\"encode_failed\",\"hash\":{hash}}}")
                .into_bytes()
        });
        line.push(b'\n');
        let width = line.len() as u64;
        if self.bytes_total.saturating_add(width) > self.cfg.max_bytes {
            return Err(LedgerFull {
                bytes_total: self.bytes_total,
                max_bytes: self.cfg.max_bytes,
            });
        }
        // Close on whole lines: a chunk never holds a partial JSON object.
        if !self.open.is_empty() && self.open.len().saturating_add(line.len()) > CHUNK_BYTES {
            self.close_open();
        }
        self.digest.update(&line);
        self.bytes_total = self.bytes_total.saturating_add(width);
        self.open.extend_from_slice(&line);
        self.seq = seq;
        if self.open.len() >= CHUNK_BYTES {
            self.close_open();
        }
        Ok(seq)
    }

    /// **The egress**: every chunk that has closed and not yet been handed over.
    ///
    /// The open chunk stays open — call [`Ledger::flush`] first to close it, which
    /// is what the end of a match does.
    ///
    /// **16b cut M.1** (`B-48`): every chunk handed out is folded into the
    /// shipped digest here, in chunk order, as it leaves — so
    /// [`Ledger::shipped_digest`] is always the SHA-256 of exactly the bytes the
    /// egress has been given.
    pub fn take_chunks(&mut self) -> Vec<LedgerChunk> {
        let chunks: Vec<LedgerChunk> = self.closed.drain(..).collect();
        for chunk in &chunks {
            self.shipped.update(&chunk.bytes);
        }
        chunks
    }

    /// Close the chunk being filled, if it holds anything, so the next
    /// [`Ledger::take_chunks`] hands over everything recorded so far.
    pub fn flush(&mut self) {
        if !self.open.is_empty() {
            self.close_open();
        }
    }

    /// **How many closed chunks the world has not acknowledged** — the bounded
    /// egress's depth, whether or not [`Ledger::take_chunks`] has handed them out.
    #[must_use]
    pub fn unflushed_chunks(&self) -> u64 {
        self.last_closed_seq.saturating_sub(self.flushed_upto)
    }

    /// **The world's acknowledgement**: every chunk up to and including
    /// `upto_chunk_seq` is durable.
    ///
    /// Never lowers, and never runs ahead of what has actually closed — an ack
    /// for a chunk that does not exist yet acknowledges only what does.
    pub fn mark_flushed(&mut self, upto_chunk_seq: u64) {
        let acked = upto_chunk_seq.min(self.last_closed_seq);
        if acked > self.flushed_upto {
            self.flushed_upto = acked;
        }
    }

    /// The highest acknowledged chunk sequence.
    #[must_use]
    pub const fn flushed_upto(&self) -> u64 {
        self.flushed_upto
    }

    /// The most recently closed chunk's sequence; `0` before the first closes.
    #[must_use]
    pub const fn last_closed_seq(&self) -> u64 {
        self.last_closed_seq
    }

    /// The egress is backing up: more than **half** of `ledger.max_unflushed_chunks`
    /// closed chunks are waiting for the world's acknowledgement. Doc 16 §3.1: the
    /// arena marks itself `Lagged` when the world cannot keep up, and cut A.2
    /// journals one `lagged` event. The halfway mark is the warning before the
    /// bound itself voids the match ([`Ledger::overflowed`]).
    #[must_use]
    pub fn lagged(&self) -> bool {
        self.unflushed_chunks() > u64::from(self.cfg.max_unflushed_chunks / 2)
    }

    /// The egress is past `ledger.max_unflushed_chunks`: the world cannot drain
    /// it and the match is stopped with `Void` (doc 16 §3.1, the documented bound
    /// itself, ruled at review of cut A.2).
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.unflushed_chunks() > u64::from(self.cfg.max_unflushed_chunks)
    }

    /// Every byte appended, including the newlines.
    #[must_use]
    pub const fn bytes_total(&self) -> u64 {
        self.bytes_total
    }

    /// Whether the cadenced `position_summary` is still being written: `false`
    /// past **90%** of `ledger.max_bytes` (doc 16 §4).
    #[must_use]
    pub fn summaries_enabled(&self) -> bool {
        u128::from(self.bytes_total).saturating_mul(10)
            < u128::from(self.cfg.max_bytes).saturating_mul(9)
    }

    /// The SHA-256 of **every byte appended, in order** — the digest that goes on
    /// the result record (doc 16 §4), so a ledger read back out of the world's log
    /// can be checked against the match that produced it.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest.clone().finalize().into()
    }

    /// **The SHA-256 of every byte handed out in a chunk**, in chunk order (16b
    /// cut M.1, `B-48`) — the concatenation of every [`LedgerChunk::bytes`]
    /// [`Ledger::take_chunks`] has returned.
    ///
    /// It equals [`Ledger::digest`] whenever nothing is left in the open chunk or
    /// the closed queue, which is every end whose step ran to completion: the
    /// ending step calls [`Ledger::flush`] and then [`Ledger::take_chunks`]. It
    /// differs where bytes were appended and never left — the open chunk of a
    /// pulse that panicked before it flushed — and there it is the only digest
    /// the world's `log:match.<id>` can reproduce, which is why the task reports
    /// this one on its `Ended`.
    #[must_use]
    pub fn shipped_digest(&self) -> [u8; 32] {
        self.shipped.clone().finalize().into()
    }

    /// The last appended event's sequence; `0` before the first.
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Arena milliseconds between `position_summary` events.
    #[must_use]
    pub const fn summary_ms(&self) -> u32 {
        self.cfg.summary_ms
    }

    /// The configured byte bound.
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.cfg.max_bytes
    }

    /// Move the open chunk into the egress under the next chunk sequence.
    fn close_open(&mut self) {
        let seq = self.last_closed_seq.saturating_add(1);
        self.last_closed_seq = seq;
        self.closed.push_back(LedgerChunk {
            seq,
            bytes: std::mem::take(&mut self.open),
        });
    }
}

/// **A SHA-256 running state back from its serialised bytes** (16b cut AF.1):
/// `sha2`'s own `SerializableState`, so the digest the snapshot resumes is the
/// digest the match had been folding, byte for byte. `which` names the field
/// in the refusal.
fn resume_sha256(bytes: &[u8], which: &str) -> Result<Sha256, SnapshotError> {
    let serialized: &SerializedState<Sha256> = bytes.try_into().map_err(|_| {
        SnapshotError::Malformed(format!(
            "the ledger's {which} state is {} bytes, not a SHA-256 state",
            bytes.len()
        ))
    })?;
    Sha256::deserialize(serialized).map_err(|_| {
        SnapshotError::Malformed(format!("the ledger's {which} state is not a SHA-256 state"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Team;

    fn ledger(max_bytes: u64, max_unflushed_chunks: u32) -> Ledger {
        Ledger::new(&LedgerConfig {
            summary_ms: 5_000,
            max_bytes,
            max_unflushed_chunks,
        })
    }

    fn summary(units: usize) -> EventKind {
        EventKind::PositionSummary {
            units: (0..units)
                .map(|n| {
                    let id = u32::try_from(n).unwrap_or(0);
                    (ObjId(id), i64::from(id) * 100, i64::from(id) * 200, 0, 0)
                })
                .collect(),
        }
    }

    #[test]
    fn a_line_is_the_flat_object_doc_16_names() {
        let mut log = ledger(1 << 20, 256);
        assert_eq!(log.seq(), 0);
        assert_eq!(log.append(0, EventKind::MatchStarted, Vec::new(), 7), Ok(1));
        assert_eq!(log.seq(), 1);
        log.flush();
        let chunks = log.take_chunks();
        let bytes = chunks
            .first()
            .map(|chunk| chunk.bytes.clone())
            .unwrap_or_default();
        let text = String::from_utf8(bytes).expect("the ledger is utf-8");
        assert_eq!(
            text,
            "{\"seq\":1,\"t_ms\":0,\"kind\":\"match_started\",\"hash\":7}\n"
        );
        // And it reads back as the same event.
        let read: LedgerEvent = serde_json::from_str(text.trim_end()).expect("one line, one event");
        assert_eq!(read.kind, EventKind::MatchStarted);
        assert_eq!(
            (read.seq, read.t_ms, read.hash, read.draws),
            (1, 0, 7, Vec::new())
        );
    }

    /// **16b cut M.3** (16c `T-07`, `T-04`): `draws` is a list that keeps its
    /// order through the wire, and a line that drew nothing carries no `draws`
    /// key at all; the `blocked` line's cause is the pilot's own word.
    #[test]
    fn m3_draws_round_trip_in_order_and_an_undrawn_line_has_no_key() {
        let mut log = ledger(1 << 20, 256);
        let hit = EventKind::Hit {
            shot_seq: 3,
            unit: ObjId(4),
            target: ObjId(9),
            section: SmolStr::new_static("engine"),
            facet: SmolStr::new_static("rear"),
            armor_section: SmolStr::new_static("hull"),
            rounds: 1,
            damage: 120,
            armor_left: 0,
            internals_left: 40,
            consequences: Vec::new(),
            cause: SmolStr::new_static("shot:3"),
        };
        let draws = vec![("hit:3".to_owned(), 17), ("facet:3".to_owned(), 99)];
        let _ = log.append(10, hit.clone(), draws.clone(), 5);
        let blocked = EventKind::Blocked {
            unit: ObjId(4),
            x: 20,
            y: 30,
            cause: crate::arena::motion::BlockedCause::Impassable,
        };
        let _ = log.append(20, blocked.clone(), Vec::new(), 6);
        log.flush();
        let text = String::from_utf8(
            log.take_chunks()
                .into_iter()
                .flat_map(|chunk| chunk.bytes)
                .collect(),
        )
        .expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        let [drawn, undrawn] = lines.as_slice() else {
            panic!("two lines: {lines:?}");
        };
        assert!(
            drawn.contains("\"draws\":[[\"hit:3\",17],[\"facet:3\",99]]"),
            "{drawn}"
        );
        assert!(drawn.contains("\"armor_section\":\"hull\""), "{drawn}");
        let read: LedgerEvent = serde_json::from_str(drawn).expect("a line");
        assert_eq!(read.draws, draws);
        assert_eq!(read.kind, hit);
        assert!(!undrawn.contains("draws"), "{undrawn}");
        assert!(undrawn.contains("\"cause\":\"impassable\""), "{undrawn}");
        let read: LedgerEvent = serde_json::from_str(undrawn).expect("a line");
        assert!(read.draws.is_empty());
        assert_eq!(read.kind, blocked);
    }

    /// **16b cut M.3**: the `blocked` line's cause serialises as exactly the word
    /// `BlockedCause::as_str` gives — the one vocabulary, two readers.
    #[test]
    fn m3_the_blocked_cause_on_the_line_is_the_as_str_word() {
        use crate::arena::motion::BlockedCause;
        for cause in [
            BlockedCause::Impassable,
            BlockedCause::Occupied,
            BlockedCause::Edge,
            BlockedCause::TooSteep,
        ] {
            assert_eq!(
                serde_json::to_value(cause).expect("a cause serialises"),
                serde_json::json!(cause.as_str())
            );
            let back: BlockedCause =
                serde_json::from_value(serde_json::json!(cause.as_str())).expect("and reads back");
            assert_eq!(back, cause);
        }
    }

    #[test]
    fn every_kind_serialises_including_the_placeholders() {
        let kinds = [
            EventKind::MatchStarted,
            EventKind::Tempo {
                from: Tempo::Quiet,
                to: Tempo::Engaged,
            },
            EventKind::Startup { unit: ObjId(4) },
            summary(2),
            EventKind::Blocked {
                unit: ObjId(4),
                x: 3,
                y: 7,
                cause: crate::arena::motion::BlockedCause::TooSteep,
            },
            EventKind::Detected {
                viewer: ObjId(4),
                contact: ContactId(0x0123_4567_89ab_cdef),
                target: ObjId(9),
            },
            EventKind::Classified {
                viewer: ObjId(4),
                contact: ContactId(0x0123_4567_89ab_cdef),
                target: ObjId(9),
                team: Team::Blue,
            },
            EventKind::Lost {
                viewer: ObjId(4),
                contact: ContactId(0x0123_4567_89ab_cdef),
                target: ObjId(9),
            },
            // 16b cut D.4: the platform's own receiver, on the transition.
            EventKind::Receiver {
                unit: ObjId(4),
                degraded: true,
            },
            // 16b cut O.2: the pod's switch, each way.
            EventKind::Emcon {
                unit: ObjId(4),
                on: false,
            },
            EventKind::Emcon {
                unit: ObjId(4),
                on: true,
            },
            EventKind::Locked {
                unit: ObjId(4),
                contact: ContactId(0x0123_4567_89ab_cdef),
                target: ObjId(9),
            },
            EventKind::Fired {
                unit: ObjId(4),
                weapon: SmolStr::new_static("cannon"),
                shot_seq: 1,
                target: ObjId(9),
                range_cm: 4_000,
                rounds: 1,
                by: None,
                slot: None,
            },
            // 17b cut M1b.1 (D-UM-14): a condition slot's own shot, attributed.
            EventKind::Fired {
                unit: ObjId(4),
                weapon: SmolStr::new_static("cannon"),
                shot_seq: 2,
                target: ObjId(9),
                range_cm: 4_000,
                rounds: 1,
                by: Some(SmolStr::new_static("cond")),
                slot: Some(1),
            },
            EventKind::Impact {
                shot_seq: 1,
                unit: ObjId(4),
                target: ObjId(9),
            },
            EventKind::Hit {
                shot_seq: 1,
                unit: ObjId(4),
                target: ObjId(9),
                section: SmolStr::new_static("turret"),
                facet: SmolStr::new_static("front"),
                armor_section: SmolStr::new_static("turret"),
                rounds: 1,
                damage: 120,
                armor_left: 230,
                internals_left: 150,
                consequences: Vec::new(),
                cause: SmolStr::new_static("shot:1"),
            },
            EventKind::Miss {
                shot_seq: 1,
                reason: SmolStr::new_static("blocked"),
            },
            EventKind::SectionDestroyed {
                unit: ObjId(9),
                section: SmolStr::new_static("turret"),
                // 16b cut D.4: the shooter, where a round did it.
                by: Some(ObjId(4)),
                consequences: vec![SmolStr::new_static("remove_weapons")],
            },
            EventKind::UnitDisabled { unit: ObjId(9) },
            EventKind::UnitDestroyed {
                unit: ObjId(9),
                by: Some(ObjId(4)),
            },
            // 16b cut I.3: the two staff levers, a set and a lift.
            EventKind::TempoFloor {
                period_ms: Some(5_000),
            },
            EventKind::TempoFloor { period_ms: None },
            EventKind::Placed {
                unit: ObjId(9),
                x: 12,
                y: 7,
                from_x: u16::MAX,
                from_y: u16::MAX,
            },
            EventKind::PlaceRefused {
                unit: ObjId(9),
                x: 12,
                y: 7,
                reason: SmolStr::new_static("in_flight"),
            },
            // 17b cut M1b.5: the card lever's two kinds, the whole card riding
            // the first.
            EventKind::MindCard {
                unit: ObjId(4),
                name: SmolStr::new("Sniper, edited (min_odds 70)"),
                changed: vec![SmolStr::new_static("min_odds")],
                card: crate::arena::runtime_card::ProfileKey {
                    definition: mush_core::DefinitionDigest::from_bytes([0xab; 32]),
                    index: 0,
                },
                stock: None,
            },
            EventKind::MindCardRefused {
                unit: ObjId(9),
                reason: SmolStr::new_static("not_mind"),
            },
            // 17b cut M3.3: the four mind kinds (journalled since 17b cuts M3.4
            // and M3.6).
            EventKind::MindSealed {
                unit: ObjId(4),
                sortie: SortieId::in_match(7, ObjId(4)),
                mind: SmolStr::new_static("Standard Doctrine"),
                source: SmolStr::new_static("8b3ec78c9d957865"),
                card_digest: SmolStr::new("a".repeat(64)),
                class: CoreClass::I,
                governed: true,
                licence: SmolStr::new_static("doctrine"),
                licence_state: crate::minds::licence_state::SealLicenceState::Doctrine,
                principal: Some(CharacterRef::Character { id: ObjId(77) }),
            },
            EventKind::SortieClosed {
                unit: ObjId(4),
                sortie: SortieId::in_match(7, ObjId(4)),
                word: SmolStr::new_static("match_end"),
            },
            EventKind::MindFault {
                unit: ObjId(4),
                think_seq: 3,
                cause: SmolStr::new_static("ops"),
            },
            EventKind::MindDropped {
                unit: ObjId(4),
                cause: SmolStr::new_static("strikes"),
                fault: Some(SmolStr::new_static("ops")),
                ops: Some(1_500),
            },
            EventKind::Lagged { dt_ms: 20_000 },
            EventKind::MatchEnded {
                reason: EndReason::Elimination {
                    winner: Some(Team::Red),
                },
            },
            EventKind::Aborted,
            EventKind::Void,
        ];
        let mut log = ledger(1 << 20, 256);
        for (n, kind) in kinds.iter().enumerate() {
            let word = kind.as_str();
            let seq = log
                .append(
                    i64::try_from(n).unwrap_or(0),
                    kind.clone(),
                    vec![(format!("hit:{n}"), 42), (format!("facet:{n}"), 43)],
                    0,
                )
                .expect("room for every kind");
            assert_eq!(seq, u64::try_from(n).unwrap_or(0) + 1);
            assert_eq!(word, kind.as_str());
        }
        log.flush();
        let text = String::from_utf8(
            log.take_chunks()
                .into_iter()
                .flat_map(|chunk| chunk.bytes)
                .collect(),
        )
        .expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), kinds.len());
        for (line, kind) in lines.iter().zip(kinds.iter()) {
            let read: LedgerEvent = serde_json::from_str(line).expect("a line reads back");
            assert_eq!(&read.kind, kind);
            assert!(line.contains(&format!("\"kind\":\"{}\"", kind.as_str())));
            assert_eq!(
                read.draws.len(),
                2,
                "both draws survive the round trip, in order"
            );
        }
    }

    /// **17b cut M3.3** (doc 17 §19's reviewed diff): the four mind kinds'
    /// words and their exact wire shapes — `principal`, `fault` and `ops`
    /// absent on the wire when `None` — and each round-trips, the `None`s
    /// included.
    #[test]
    fn m3_the_four_mind_kinds_have_their_words_and_shapes() {
        let sortie = SortieId::in_match(7, ObjId(4));
        let sealed = |principal: Option<CharacterRef>| EventKind::MindSealed {
            unit: ObjId(4),
            sortie,
            mind: SmolStr::new_static("Standard Doctrine"),
            source: SmolStr::new_static("8b3ec78c9d957865"),
            card_digest: SmolStr::new("a".repeat(64)),
            class: CoreClass::I,
            governed: true,
            licence: SmolStr::new_static("doctrine"),
            licence_state: crate::minds::licence_state::SealLicenceState::Doctrine,
            principal,
        };
        let cases = [
            (
                sealed(Some(CharacterRef::Character { id: ObjId(77) })),
                serde_json::json!({
                    "kind": "mind_sealed", "unit": 4, "sortie": 30_064_771_076_u64,
                    "mind": "Standard Doctrine", "source": "8b3ec78c9d957865",
                    "card_digest": "a".repeat(64), "class": "I", "governed": true,
                    "licence": "doctrine", "licence_state": "doctrine",
                    "principal": {"kind": "character", "id": 77},
                }),
            ),
            (
                sealed(None),
                serde_json::json!({
                    "kind": "mind_sealed", "unit": 4, "sortie": 30_064_771_076_u64,
                    "mind": "Standard Doctrine", "source": "8b3ec78c9d957865",
                    "card_digest": "a".repeat(64), "class": "I", "governed": true,
                    "licence": "doctrine", "licence_state": "doctrine",
                }),
            ),
            (
                EventKind::SortieClosed {
                    unit: ObjId(4),
                    sortie,
                    word: SmolStr::new_static("destroyed"),
                },
                serde_json::json!({
                    "kind": "sortie_closed", "unit": 4, "sortie": 30_064_771_076_u64,
                    "word": "destroyed",
                }),
            ),
            (
                EventKind::MindFault {
                    unit: ObjId(4),
                    think_seq: 3,
                    cause: SmolStr::new_static("ops"),
                },
                serde_json::json!({
                    "kind": "mind_fault", "unit": 4, "think_seq": 3, "cause": "ops",
                }),
            ),
            (
                EventKind::MindDropped {
                    unit: ObjId(4),
                    cause: SmolStr::new_static("strikes"),
                    fault: Some(SmolStr::new_static("ops")),
                    ops: Some(1_500),
                },
                serde_json::json!({
                    "kind": "mind_dropped", "unit": 4, "cause": "strikes", "fault": "ops",
                    "ops": 1_500,
                }),
            ),
            (
                EventKind::MindDropped {
                    unit: ObjId(4),
                    cause: SmolStr::new_static("core"),
                    fault: None,
                    ops: None,
                },
                serde_json::json!({"kind": "mind_dropped", "unit": 4, "cause": "core"}),
            ),
        ];
        for (kind, wire) in cases {
            assert_eq!(
                serde_json::to_value(&kind).ok(),
                Some(wire.clone()),
                "{kind:?}"
            );
            assert_eq!(
                serde_json::from_value::<EventKind>(wire).ok(),
                Some(kind.clone()),
                "{kind:?} reads back"
            );
        }
        assert_eq!(
            [
                sealed(None).as_str(),
                EventKind::SortieClosed {
                    unit: ObjId(4),
                    sortie,
                    word: SmolStr::new_static("match_end"),
                }
                .as_str(),
                EventKind::MindFault {
                    unit: ObjId(4),
                    think_seq: 0,
                    cause: SmolStr::default(),
                }
                .as_str(),
                EventKind::MindDropped {
                    unit: ObjId(4),
                    cause: SmolStr::default(),
                    fault: None,
                    ops: None,
                }
                .as_str(),
            ],
            ["mind_sealed", "sortie_closed", "mind_fault", "mind_dropped"]
        );
    }

    #[test]
    fn chunks_close_on_whole_lines_at_sixteen_kibibytes() {
        let mut log = ledger(1 << 24, 4_096);
        // Enough summaries to fill several chunks.
        for n in 0..400_u32 {
            log.append(i64::from(n), summary(12), Vec::new(), u64::from(n))
                .expect("room");
        }
        assert!(log.last_closed_seq() > 1, "more than one chunk closed");
        let chunks = log.take_chunks();
        let mut total = 0_usize;
        for (n, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.seq, u64::try_from(n).unwrap_or(0) + 1);
            assert!(
                chunk.bytes.len() <= CHUNK_BYTES,
                "chunk {} is {} bytes",
                chunk.seq,
                chunk.bytes.len()
            );
            // Whole lines only: the last byte is a newline and every line parses.
            assert_eq!(chunk.bytes.last(), Some(&b'\n'));
            let text = String::from_utf8(chunk.bytes.clone()).expect("utf-8");
            for line in text.lines() {
                let _: LedgerEvent = serde_json::from_str(line).expect("a whole line");
            }
            total = total.saturating_add(chunk.bytes.len());
        }
        // A closed chunk is within one line of full.
        let longest = chunks
            .iter()
            .flat_map(|chunk| {
                String::from_utf8(chunk.bytes.clone())
                    .unwrap_or_default()
                    .lines()
                    .map(str::len)
                    .collect::<Vec<_>>()
            })
            .max()
            .unwrap_or(0);
        for chunk in &chunks {
            assert!(
                chunk.bytes.len() + longest + 1 > CHUNK_BYTES,
                "chunk {} closed early at {} bytes",
                chunk.seq,
                chunk.bytes.len()
            );
        }
        // Everything handed over, plus whatever is still open, is everything
        // appended.
        log.flush();
        let tail: usize = log
            .take_chunks()
            .iter()
            .map(|chunk| chunk.bytes.len())
            .sum();
        assert_eq!(
            u64::try_from(total.saturating_add(tail)).unwrap_or(0),
            log.bytes_total()
        );
    }

    #[test]
    fn the_egress_reports_lagged_then_overflowed_and_an_ack_drains_it() {
        let mut log = ledger(1 << 24, 2);
        assert_eq!(log.unflushed_chunks(), 0);
        assert!(!log.lagged());
        assert!(!log.overflowed());
        // Fill chunks until two have closed: past half of `max_unflushed_chunks`
        // (2), the warning, but not past the bound itself.
        let mut n = 0_i64;
        while log.last_closed_seq() < 2 {
            log.append(n, summary(12), Vec::new(), 0).expect("room");
            n += 1;
        }
        assert_eq!(log.unflushed_chunks(), 2);
        assert!(
            log.lagged(),
            "two closed against a bound of two: past half of it"
        );
        assert!(!log.overflowed(), "two is not past two");
        // Past the bound: doc 16 §3.1 voids the match here.
        while log.last_closed_seq() < 5 {
            log.append(n, summary(12), Vec::new(), 0).expect("room");
            n += 1;
        }
        assert_eq!(log.unflushed_chunks(), 5);
        assert!(log.overflowed());
        // Taking the chunks is not acknowledging them: the world must ack.
        let _ = log.take_chunks();
        assert_eq!(log.unflushed_chunks(), 5);
        assert!(log.overflowed());
        // The ack drains the depth, never lowers, and never runs ahead.
        log.mark_flushed(4);
        assert_eq!(log.unflushed_chunks(), 1);
        assert!(!log.lagged());
        log.mark_flushed(2);
        assert_eq!(log.flushed_upto(), 4, "an ack never lowers");
        log.mark_flushed(u64::MAX);
        assert_eq!(log.flushed_upto(), log.last_closed_seq());
        assert_eq!(log.unflushed_chunks(), 0);
    }

    #[test]
    fn summaries_stop_at_ninety_per_cent_and_appends_refuse_at_a_hundred() {
        // A small bound so the test is a handful of lines rather than 64 MiB.
        let mut log = ledger(1_000, 256);
        assert!(log.summaries_enabled());
        let mut refusals = 0_usize;
        let mut summaries_off_at: Option<u64> = None;
        for n in 0..200_i64 {
            if !log.summaries_enabled() && summaries_off_at.is_none() {
                summaries_off_at = Some(log.bytes_total());
            }
            match log.append(n, EventKind::MatchStarted, Vec::new(), 0) {
                Ok(_) => {}
                Err(full) => {
                    refusals += 1;
                    assert_eq!(full.max_bytes, 1_000);
                    assert!(full.bytes_total <= 1_000);
                }
            }
        }
        assert!(refusals > 0, "the bound was never reached");
        // Summaries stopped at 90%, before the refusals began.
        let off_at = summaries_off_at.expect("summaries stopped");
        assert!(off_at >= 900, "summaries stopped at {off_at} bytes");
        assert!(!log.summaries_enabled());
        // The bound is never exceeded, and the sequence has no gap: a refused
        // event does not consume a number.
        assert!(log.bytes_total() <= 1_000);
        log.flush();
        let text = String::from_utf8(
            log.take_chunks()
                .into_iter()
                .flat_map(|chunk| chunk.bytes)
                .collect(),
        )
        .expect("utf-8");
        let seqs: Vec<u64> = text
            .lines()
            .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
            .map(|event| event.seq)
            .collect();
        assert_eq!(
            seqs,
            (1..=log.seq()).collect::<Vec<u64>>(),
            "the sequence is contiguous"
        );
    }

    #[test]
    fn the_digest_is_stable_over_a_fixed_event_sequence() {
        let script = |log: &mut Ledger| {
            let _ = log.append(0, EventKind::MatchStarted, Vec::new(), 1);
            let _ = log.append(
                100,
                EventKind::Tempo {
                    from: Tempo::Quiet,
                    to: Tempo::Alert,
                },
                Vec::new(),
                2,
            );
            let _ = log.append(200, EventKind::Startup { unit: ObjId(4) }, Vec::new(), 3);
            let _ = log.append(5_000, summary(3), Vec::new(), 4);
            let _ = log.append(
                6_000,
                EventKind::Hit {
                    shot_seq: 1,
                    unit: ObjId(4),
                    target: ObjId(9),
                    section: SmolStr::new_static("hull"),
                    facet: SmolStr::new_static("front"),
                    armor_section: SmolStr::new_static("hull"),
                    rounds: 1,
                    damage: 120,
                    armor_left: 280,
                    internals_left: 200,
                    consequences: Vec::new(),
                    cause: SmolStr::new_static("shot:1"),
                },
                vec![("hit:1".to_string(), 12_072_725_193_963_834_385)],
                5,
            );
            let _ = log.append(
                7_000,
                EventKind::MatchEnded {
                    reason: EndReason::Abort,
                },
                Vec::new(),
                6,
            );
        };
        let mut first = ledger(1 << 20, 256);
        script(&mut first);
        let mut second = ledger(1 << 20, 256);
        script(&mut second);
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.seq(), 6);
        // The digest covers every appended byte, so it moves when one event does
        // and it does not depend on where the chunk boundaries fell.
        let mut altered = ledger(1 << 20, 256);
        script(&mut altered);
        let _ = altered.append(8_000, EventKind::Void, Vec::new(), 7);
        assert_ne!(first.digest(), altered.digest());
        // Draining and acknowledging changes nothing about the digest.
        let before = first.digest();
        first.flush();
        let _ = first.take_chunks();
        first.mark_flushed(u64::MAX);
        assert_eq!(first.digest(), before);
        // And the digest of the empty ledger is SHA-256 of nothing, pinned.
        let empty = ledger(1 << 20, 256);
        assert_eq!(
            empty.digest(),
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55
            ]
        );
    }

    #[test]
    fn flush_closes_the_open_chunk_and_take_chunks_alone_does_not() {
        let mut log = ledger(1 << 20, 256);
        let _ = log.append(0, EventKind::MatchStarted, Vec::new(), 0);
        assert!(log.take_chunks().is_empty(), "the open chunk stays open");
        assert_eq!(log.last_closed_seq(), 0);
        log.flush();
        assert_eq!(log.last_closed_seq(), 1);
        assert_eq!(log.take_chunks().len(), 1);
        // Flushing nothing closes nothing.
        log.flush();
        assert_eq!(log.last_closed_seq(), 1);
        assert!(log.take_chunks().is_empty());
    }

    /// **16b cut M.1** (`B-48`): the shipped digest is the digest of what left in
    /// chunks, so it equals the full one exactly when nothing is left behind.
    #[test]
    fn the_shipped_digest_covers_what_left_and_equals_the_digest_once_flushed() {
        let mut log = ledger(1 << 20, 256);
        let empty = log.shipped_digest();
        assert_eq!(empty, log.digest(), "nothing appended, nothing shipped");
        let _ = log.append(0, EventKind::MatchStarted, Vec::new(), 0);
        // An open chunk: appended and never handed out.
        assert_ne!(log.shipped_digest(), log.digest());
        assert_eq!(log.shipped_digest(), empty);
        // Closed but not yet taken is still not shipped.
        log.flush();
        assert_ne!(log.shipped_digest(), log.digest());
        let first = log.take_chunks();
        assert_eq!(first.len(), 1);
        assert_eq!(log.shipped_digest(), log.digest());
        // A closed chunk taken while a later one is still open: they differ, and
        // the shipped one is SHA-256 of the taken bytes alone.
        let _ = log.append(1_000, EventKind::Void, Vec::new(), 1);
        log.flush();
        let _ = log.append(2_000, EventKind::Void, Vec::new(), 2);
        let second = log.take_chunks();
        assert_eq!(second.len(), 1);
        assert_ne!(log.shipped_digest(), log.digest());
        let mut expected = Sha256::new();
        for chunk in first.iter().chain(&second) {
            expected.update(&chunk.bytes);
        }
        let expected: [u8; 32] = expected.finalize().into();
        assert_eq!(log.shipped_digest(), expected);
        // Flushed and taken, the two agree again.
        log.flush();
        let _ = log.take_chunks();
        assert_eq!(log.shipped_digest(), log.digest());
    }

    /// **16b cut AF.1**: a ledger carried through its state across two chunk
    /// closes — one taken and acknowledged, one closed and still waiting, and an
    /// open chunk part-filled — is the ledger that was never carried. The state
    /// crosses JSON on the way, as the snapshot's does; afterwards both are
    /// given the same appends, and every counter, both digests and every chunk
    /// the egress is handed after agree.
    #[test]
    fn af1_a_ledger_through_its_state_is_the_uninterrupted_ledger() {
        let append = |log: &mut Ledger, n: i64| {
            log.append(
                n,
                summary(12),
                vec![(format!("hit:{n}"), 7)],
                u64::try_from(n).unwrap_or(0),
            )
            .expect("room");
        };
        let mut straight = ledger(1 << 24, 4_096);
        let mut carried = ledger(1 << 24, 4_096);
        let mut n = 0_i64;
        while carried.last_closed_seq() < 1 {
            append(&mut straight, n);
            append(&mut carried, n);
            n += 1;
        }
        assert_eq!(straight.take_chunks(), carried.take_chunks());
        straight.mark_flushed(1);
        carried.mark_flushed(1);
        while carried.last_closed_seq() < 2 {
            append(&mut straight, n);
            append(&mut carried, n);
            n += 1;
        }
        // Two lines into the next chunk, so the open buffer is under test too.
        for _ in 0..2 {
            append(&mut straight, n);
            append(&mut carried, n);
            n += 1;
        }

        let state = carried.to_state();
        assert!(!state.open.is_empty(), "the open chunk is carried");
        assert_eq!(
            state.closed.len(),
            1,
            "one closed chunk waits for the egress"
        );
        assert_eq!((state.last_closed_seq, state.flushed_upto), (2, 1));
        let text = serde_json::to_string(&state).expect("the state encodes");
        let back: LedgerState = serde_json::from_str(&text).expect("and decodes");
        assert_eq!(back, state);
        let defaulted = Ledger::from_state(&LedgerConfig::default(), back.clone())
            .expect("the digest states resume");
        // Under the configuration it is given, not one it carried.
        assert_eq!(defaulted.max_bytes(), LedgerConfig::default().max_bytes);
        let mut resumed = Ledger::from_state(
            &LedgerConfig {
                summary_ms: 5_000,
                max_bytes: 1 << 24,
                max_unflushed_chunks: 4_096,
            },
            back,
        )
        .expect("the digest states resume");

        let same = |a: &Ledger, b: &Ledger| {
            assert_eq!(a.digest(), b.digest());
            assert_eq!(a.shipped_digest(), b.shipped_digest());
            assert_eq!(a.seq(), b.seq());
            assert_eq!(a.bytes_total(), b.bytes_total());
            assert_eq!(a.last_closed_seq(), b.last_closed_seq());
            assert_eq!(a.flushed_upto(), b.flushed_upto());
        };
        same(&straight, &resumed);
        while straight.last_closed_seq() < 4 {
            append(&mut straight, n);
            append(&mut resumed, n);
            n += 1;
        }
        same(&straight, &resumed);
        assert_eq!(straight.take_chunks(), resumed.take_chunks());
        straight.flush();
        resumed.flush();
        assert_eq!(straight.take_chunks(), resumed.take_chunks());
        same(&straight, &resumed);
        assert_eq!(resumed.digest(), resumed.shipped_digest(), "all of it left");

        // A digest state of the wrong length is refused by name, never a panic.
        let mut broken = carried.to_state();
        broken.shipped.pop();
        assert!(matches!(
            Ledger::from_state(&LedgerConfig::default(), broken),
            Err(SnapshotError::Malformed(message)) if message.contains("shipped")
        ));
    }

    #[test]
    fn the_configured_cadence_and_bound_are_readable() {
        let log = ledger(4_096, 8);
        assert_eq!(log.summary_ms(), 5_000);
        assert_eq!(log.max_bytes(), 4_096);
    }

    /// **17b cut M1b.1** (D-UM-14): a `fired` line carries `by` and `slot` only
    /// when the shot was a slot's or a cited mind's; a bare line — every line
    /// before this cut — is byte for byte what it was and still decodes.
    #[test]
    fn m1b1_a_fired_line_carries_its_attribution_only_when_there_is_one() {
        let bare = EventKind::Fired {
            unit: ObjId(4),
            weapon: SmolStr::new_static("cannon"),
            shot_seq: 1,
            target: ObjId(9),
            range_cm: 4_000,
            rounds: 1,
            by: None,
            slot: None,
        };
        let text = serde_json::to_string(&bare).expect("serialises");
        assert_eq!(
            text,
            "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":1,\"target\":9,\
             \"range_cm\":4000,\"rounds\":1}"
        );
        let cond = EventKind::Fired {
            unit: ObjId(4),
            weapon: SmolStr::new_static("cannon"),
            shot_seq: 1,
            target: ObjId(9),
            range_cm: 4_000,
            rounds: 1,
            by: Some(SmolStr::new_static("cond")),
            slot: Some(2),
        };
        let text = serde_json::to_string(&cond).expect("serialises");
        assert!(text.ends_with(",\"by\":\"cond\",\"slot\":2}"), "{text}");
        let back: EventKind = serde_json::from_str(&text).expect("reads back");
        assert_eq!(back, cond);
        // A legacy line, written before the keys existed, decodes with neither.
        let legacy: EventKind = serde_json::from_str(
            "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":1,\"target\":9,\
             \"range_cm\":4000,\"rounds\":1}",
        )
        .expect("a legacy line");
        assert_eq!(legacy, bare);
    }
}
