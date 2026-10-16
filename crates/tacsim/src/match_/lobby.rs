//! **The lobby: doc 16 §3.8's machine, as players walk it** (16b cut K.1; doc 16
//! §3.7 "Pods", §3.8 "Lifecycle", §5; 16b §8 cut K, K1).
//!
//! Cut T.3 built the two ends — [`crate::match_::start`], the `Boarding →
//! Running` check-out, and [`crate::match_::finish`], the check-in — and left
//! `Selection` and `Boarding` as slots for this cut. This is what fills them:
//! one **centre** per server (the proving ground, doc 16 §1), one match at a
//! time in it, and the five `SIM` verbs that move it.
//!
//! ```text
//! Idle → Selection → Boarding → Running → Ended → Idle
//!                  ↘        ↘        ↘
//!                   Aborted ← Aborted ← Aborted | Void → Idle
//! ```
//!
//! | edge | what moves it |
//! |---|---|
//! | `Idle → Selection` | the first `SIM TEAM red\|blue` |
//! | `Selection → Idle` | the last `SIM LEAVE` |
//! | `Selection → Boarding` | `SIM START`, by a team lead or staff, with a pilot a side |
//! | `Boarding → Running` | every claimed slot seated ([`slot_seated`]), **or** `match.board_timeout_ms` with the unseated dropped |
//! | `Running → Ended \| Void` | the arena's own `Ended` (elimination, `match.max_duration_ms`, a fault) |
//! | any → `Aborted` | `SIM ABORT` by a participant or staff |
//! | terminal → `Idle` | [`reset`], idempotent, re-runnable by staff as `@arena/reset` |
//!
//! # A slot belongs to an **account**, not to a character
//!
//! Doc 16 §3.8 says *one slot per account*, and it means it: two characters on
//! one account are one player with two bodies, and letting them take two pods is
//! letting one player fight both sides of a duel. So the slot is keyed by
//! [`mush_core::AccountId`] — opaque, immutable across a rename, and never
//! rendered — and records the character that claimed it beside it, which is what
//! every player-facing line names. The oracle answers `None` for an unmapped
//! character, and the claim then **refuses**: doc 10 R3's fail-closed rule, and
//! the same refusal `+focus` gives. **16b cut AH.1** keys the first-time hints by
//! the same account, for the same reason (`crate::match_::tutorial`).
//!
//! # What "released atomically" means here
//!
//! Every release in this file — a `SIM LEAVE`, a dropped slot at the boarding
//! deadline, an abort, the reset after an end — runs inside **one** command
//! handler on the single world task, beside the check-in that returns custody.
//! The pods are RAM rows on this module's own state slot and the seats, leases
//! and focus go through `mush_sim`'s one revocation seam, so either the whole
//! release rides the batch or none of it does.

use std::collections::BTreeMap;

use mush_core::{AccountId, Ctx, ObjId, World};
use mush_proto::{OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::core::CoreClass;
use crate::arena::state::{Designator, EndReason, MatchMode, StopReason, Team};
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::arena_task::MatchId;
use crate::config::{ArenaConfig, CommandConfig, MatchConfig, MindsConfig, SensorsConfig};
use crate::match_::centre::{self, CENTRE_ARENA, CentreRoom, PODS_PER_SIDE};
use crate::match_::holdings::{self, HeldMind, SealChoice};
use crate::match_::net;
use crate::match_::report::HandOver;
use crate::match_::{MatchBook, MatchOutcome, MatchState, Side};

/// The one line every out-of-place `SIM` refuses with. Until 16b cut F.4 a
/// command gate said it; [`crate::match_::sim::sim_command`] says it now.
pub const OUTSIDE_CENTRE: &str = "The simulator is through the lobby.";

/// One claimed slot in the centre (doc 16 §3.8's `Selection`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    /// Which side.
    pub team: Team,
    /// Which slot of that side, 1-based — the pod it locks and the arena unit
    /// it flies.
    pub index: u8,
    /// **The claim's owner.** Opaque and never rendered (doc 10 R3).
    pub account: AccountId,
    /// The character that claimed it, which is what every line names.
    pub pilot: ObjId,
    /// The pod locked to it at `SIM START`.
    pub pod: Option<ObjId>,
    /// The unit seated in it, once cut K.2's `BOARD` reports one.
    pub unit: Option<ObjId>,
    /// **Whether this pilot is out of their cockpit, by any route** (16b cut
    /// F.1; the meaning since 16b cut Y.3).
    ///
    /// A seat given up during `Running` hands the tank to its sealed mind (the
    /// reflex controller's until 17b cut M4.3; re-pointed by 17b cut M5.7).
    /// The slot stays on the board because the result record names its pilot,
    /// but `SIM STATUS` said `seated` about somebody standing in the team room,
    /// which is the one fact a spectator most wants right.
    ///
    /// Written by `slot_left`, from the one place every route to a
    /// `Control { Released }` passes through — `EXIT`, a `QUIT` and the linkdead
    /// grace running out all reach it, and the last two reach it more often in a
    /// playtest than the first. **16b cut Y.3** tells the routes apart with
    /// [`Slot::dropped_at_ms`] beside this: a drop can be taken back
    /// (`slot_reclaimed`, which is the one writer of `false` mid-match), an
    /// `EXIT` cannot.
    pub left: bool,
    /// **When a drop took this seat**, in world milliseconds (16b cut Y.3; 16c
    /// `N4`; doc 07 §1.3 "Reclaiming control").
    ///
    /// A drop is the linkdead grace running out, or a `QUIT` — both reach
    /// `SeatFacts::disconnected` in `arena_bridge`, which writes this through
    /// `slot_dropped` a moment before its `Released` marks the slot
    /// [`Slot::left`]. `Some` is what makes the slot **reclaimable**: `BOARD` in
    /// the running match takes the tank back once
    /// `[tacsim.match].reclaim_min_ms` has passed since this moment.
    ///
    /// **An `EXIT` never leaves it set** (doc 16 §11's N.1 amendment: a
    /// deliberate leave is one-way, whatever came before it). `slot_dropped`
    /// writes only on a slot still seated and not `left`, so an `EXIT` followed
    /// by a `QUIT` marks nothing; and `EXIT` itself clears it through
    /// `slot_exited`, so a pilot who dropped, reconnected inside their pod and
    /// climbed out has left for good, which is what `EXIT` tells them.
    pub dropped_at_ms: Option<i64>,
    /// **The unit type this pilot has chosen to field** (16b cut Q.8), as the
    /// definition's canonical id, or `None` for the pod's own authored type.
    ///
    /// Written by [`unit_type`] (`SIM UNIT <type>`) during `Selection`, read at
    /// `SIM START`, where the pod's tank is refitted to it through
    /// [`mush_tactics::module::refit_unit`] before the pods lock. It goes with
    /// the slot: every release clears `centre.slots`, so the choice never
    /// survives the return to `Idle` and the next match starts from the authored
    /// types again — which is also what the reset on that transition guarantees
    /// of the tanks themselves.
    pub unit_type: Option<SmolStr>,
    /// **Whether the boot's resume released this seat** (16b cut AF.3; 16c
    /// N16, `Q-AF4`): a restart of the server half resumed the match from its
    /// last snapshot and gave every seat to its sealed mind, this one included
    /// ("to doctrine" here until 17b cut M5.7: one word for every mind). It is
    /// what tells a pilot who dropped of their own accord (Y.3's line alone)
    /// from one the restart dropped, who is owed the resume line before it.
    /// Cleared by `slot_reclaimed`, and by `resume_line_said` once the line
    /// has been said, so it is said once.
    ///
    /// `#[serde(default)]`: a live record an AF.2 running half wrote has no
    /// such key, and it must decode under this build so that match resumes
    /// (the live record's own `LIVE_FORMAT` policy).
    #[serde(default)]
    pub released_by_resume: bool,
    /// **The mind this pilot has chosen for the hull** (17b cut M5.1; doc 17
    /// §12, D-UM-49), as `SIM MIND` wrote it, or `None` for the default every
    /// unchosen slot is sealed with — Standard Doctrine on Line (the owner's
    /// OQ-9 (i): an unchosen slot stays as built).
    ///
    /// Written by [`mind`] during `Selection` only, fitted to the slot's
    /// chassis at the choice ([`unit_type`] fits it again when the chassis
    /// changes), resolved against the holdings seam at the check-out
    /// ([`SimError::MindNotHeld`] when the pilot no longer holds it) into
    /// `crate::match_::Side::minds`. It goes with the slot, as
    /// [`Slot::unit_type`] does. **Since 17b cut M5.2 the check-out seals
    /// it** (`crate::match_::start`; Q-UM-15's substitution under
    /// [`Centre::doctrine_only`]).
    ///
    /// `#[serde(default)]` and skipped when `None`, under the live record's
    /// `LIVE_FORMAT` policy ([`Slot::released_by_resume`]'s precedent): a
    /// record written before it decodes, and a slot that chose nothing writes
    /// the bytes it always wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mind: Option<MindChoice>,
}

/// **A chosen mind and its profiles** (17b cut M5.1; doc 17 §11, §12; the
/// owner's OQ-3) — what `SIM MIND` stores on a slot and `SIM AI MIND` on a
/// side's fill: the mind's own name and the profiles in the order they were
/// chosen, each a canonical preset spelling. The first profile is the one
/// that flies at the release, the rest are switchable with `MIND PROFILE`
/// (17b cut M5.4). **Empty is Line alone** — `SIM MIND Standard` and `SIM MIND
/// Standard PROFILE Line` store the same choice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindChoice {
    /// The mind's name as its holding spells it (`Standard Doctrine`).
    pub mind: SmolStr,
    /// The profiles, canonical spellings, first flies first; empty is Line
    /// alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<SmolStr>,
}

impl MindChoice {
    /// **How every line spells it**: `Standard Doctrine (Sniper, Line)`, and
    /// `Standard Doctrine (Line)` for Line alone.
    #[must_use]
    pub fn words(&self) -> String {
        mind_words(&self.mind, &self.profiles)
    }
}

/// **Who set a side's fill choice** (17b cut M5.1; the design note's §2.4):
/// the side's lead, by account — the one who may read it back after the
/// start, and whose leaving clears it before the check-out — or staff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillSetter {
    /// The side's lead at the time of the choice. Opaque and never rendered.
    Lead(AccountId),
    /// Staff, who hold no minds and set Standard Doctrine's presets only
    /// (D9).
    Staff,
}

/// **One side's fill choice** (17b cut M5.1; doc 17 §12, §19) — what `SIM AI
/// MIND` stores: the side, who set it, and the mind with its ONE profile
/// (nobody aboard a fill hull switches it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillChoice {
    /// The side whose unclaimed pods it flies.
    pub team: Team,
    /// Who set it.
    pub by: FillSetter,
    /// The mind and its one profile (empty is Line).
    pub choice: MindChoice,
}

impl Slot {
    /// Whether this slot is seated — the question the `Boarding → Running`
    /// trigger asks of every one of them.
    #[must_use]
    pub const fn is_seated(&self) -> bool {
        self.unit.is_some()
    }

    /// **Whether this slot's pilot dropped and may take the tank back** (16b cut
    /// Y.3): out of the cockpit ([`Slot::left`]) by a drop
    /// ([`Slot::dropped_at_ms`]) and not by an `EXIT`. The board reads it as
    /// `flown by its mind` ([`board_standing_clause`]; `flown by doctrine`
    /// until 17b cut M5.3, which made it one word for every sealed mind).
    #[must_use]
    pub const fn is_dropped(&self) -> bool {
        self.left && self.dropped_at_ms.is_some()
    }
}

/// **The simulator centre**, one per server, one match at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Centre {
    /// Where it is in doc 16 §3.8's machine.
    pub state: MatchState,
    /// The match id, minted at `SIM START` so `Boarding` already has a name.
    pub match_id: Option<MatchId>,
    /// The arena the centre fights on.
    pub arena: SmolStr,
    /// The claimed slots, in claim order.
    pub slots: Vec<Slot>,
    /// The lead of each side — its first claimant, or once that account has
    /// given the slot up (`SIM LEAVE`), been dropped at boarding, or left its
    /// tank once the match is running (16b cut B.4), the side's first remaining
    /// claimant still in a tank ([`Centre::resettle_leads`]) — who may
    /// `SIM START` and `SIM CONCEDE`.
    pub leads: Vec<(Team, AccountId)>,
    /// When `Boarding` gives up on the unseated, in world milliseconds.
    pub board_deadline_ms: Option<i64>,
    /// How the last match ended, until staff reset the centre.
    pub outcome: Option<MatchOutcome>,
    /// **The arena's own reason for that outcome** (16b cut Q.7).
    ///
    /// [`MatchOutcome`] is the result document's six-word vocabulary (doc 16 §4)
    /// and two different ends map to `draw`: a mutual elimination, and
    /// [`EndReason::Spent`] — nobody left with a round to fire. The board is the
    /// one surface that wants to tell them apart, so it keeps the reason beside
    /// the word rather than widening the record's vocabulary for a sentence.
    pub outcome_reason: Option<EndReason>,
    /// **How many units each side fields with `[tacsim.match].ai_fill` on**
    /// (16b cut Q.7), or `None` for the default — which is the larger side's
    /// claimed slots, and never less than one.
    ///
    /// `SIM AI <n>` writes it, and it is cleared with the rest of the board at
    /// every return to `Idle`. It is **not** a per-side number: the two sides
    /// field the same count, because a fill that let one side ask for more
    /// machines than the other would be the lobby choosing who wins — which is
    /// why [`ai_side_size`] reads it as a **floor** under the larger side and
    /// not as a cap on it.
    pub ai_side_size: Option<u8>,
    /// **When every pilot of this match had left it**, in world milliseconds, or
    /// `None` while somebody is still aboard (16b cut D.3, `B-33`).
    ///
    /// Stamped by `slot_left` the moment the last un-`left` slot goes `left`
    /// during `Running`, read by [`unattended_deadline`] on the lobby's own 1 s
    /// tick, and cleared by exactly three things: a `Stop { Unattended }` the
    /// arena accepted, a [`crate::arena_bridge::ControlRefusal::NoMatch`] (the
    /// match is already gone), and `release_rows` (the end arrived by some
    /// other route). On this same `MatchBook` slot: no new state slot. It is
    /// carried by the live record (16b cut AF.2) but **not trusted across a
    /// restart**: a resumed centre (16b cut AF.3) comes back with it cleared,
    /// its seats are released to their sealed minds ("to doctrine" here until
    /// 17b cut M5.7) through `slot_left`, which stamps it once the last one is
    /// marked, and the resume restamps it at the boot's
    /// own moment when every slot was already left — so a match nobody comes
    /// back to ends `unattended_ms` after the restart, not an hour later.
    ///
    /// A centre with **no slots** — a staff `@arena/start`, which `adopt` binds
    /// — is never stamped: there was nobody to leave.
    ///
    /// **16b cut Y.3's reclaim un-stamps it** (`N4`): `slot_reclaimed` puts a
    /// dropped pilot back in their seat mid-match and clears this with the
    /// warning below (`Centre::clear_unattended`), or the match it just gave
    /// back would end under the returning pilot. An `EXIT` is still one-way
    /// (doc 16 §11's N.1 amendment), so nothing else un-stamps it.
    pub unattended_since_ms: Option<i64>,
    /// Whether the unattended stop has already been logged as undeliverable (16b
    /// cut D.3, `B-33`), so a lagging match warns once and not once a second.
    ///
    /// Cleared wherever [`Centre::unattended_since_ms`] is.
    pub unattended_warned: bool,
    /// **The rule the next match is fought under** (16b cut O.1; 16c N11), as
    /// `SIM MODE` set it — [`MatchMode::Skirmish`] by default.
    ///
    /// A match-wide setting on [`Centre::ai_side_size`]'s model: set by a side's
    /// lead or staff in `Selection` ([`mode`]), read by anybody, carried to the
    /// check-out by `ready`, and cleared with the rest of the board at every
    /// return to `Idle` (`release_rows`' `Idle` arm, and `to_idle`'s
    /// `Centre::default`). It is **public in selection**: the board prints the
    /// rule (`  mode: hold — …`) to every reader, and never who holds the spot.
    pub mode: MatchMode,
    /// **How many targets the range fields** (16b cut O.3; 16c N13), as `SIM
    /// RANGE <n>` set it, or `None` when the next match is not a range.
    ///
    /// **The invariant**: `mode == MatchMode::Range` exactly when this is
    /// `Some`. Every write keeps it: [`range`] writes both, [`mode`] setting
    /// `hold` or `skirmish` clears this, and both returns to `Idle` clear both
    /// (`release_rows`' `Idle` arm, and `to_idle`'s `Centre::default`). The
    /// targets stand on pods 1 to `n` of the side opposite every claimed slot
    /// ([`range_sides`]); which side that is is not stored, because under a range
    /// every claim is on one side by construction ([`range`] refuses two sides,
    /// [`claim`] refuses the target side).
    pub range_targets: Option<u8>,
    /// **What each side's AI fill is to carry** (17b cut M5.1; doc 17 §12,
    /// §19; the design note's §2.4), as `SIM AI MIND` set it — at most one
    /// entry a side, none for a side whose fill flies the default (Standard
    /// Doctrine on Line).
    ///
    /// Set by a side's lead (from the lead's own holdings) or by staff
    /// (Standard Doctrine's presets only), in `Selection` only ([`ai_mind`]).
    /// **A change of lead before the check-out clears a lead's entry**
    /// ([`Centre::resettle_leads`]), so no new lead reads or inherits another
    /// account's choice; a staff entry stays. After the check-out the entry is
    /// kept as it was resolved, read only by its setter and staff, and it is
    /// cleared with the rest of the board at the return to `Idle`.
    ///
    /// `#[serde(default)]` and skipped when empty, under the live record's
    /// `LIVE_FORMAT` policy: a centre nobody chose a fill for writes the bytes
    /// it always wrote.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fill_minds: Vec<FillChoice>,
    /// **Humans on both sides: only Standard Doctrine flies** (17b cut M5.2;
    /// doc 17 §21's Q-UM-15 at its default, the owner's OQ-5; the design
    /// note's C-7 and item 11). Decided ONCE, at `SIM START` ([`start`]),
    /// from the claimed slots: `true` when both sides hold one. Under it the
    /// check-out (`ready`) seals a pilot's non-Doctrine choice as Standard
    /// Doctrine on Line, and a lead's non-Doctrine fill as Standard Doctrine
    /// at the lead's chosen profile when it is one of Doctrine's six presets
    /// (else Line), each `substituted` and sealed with the fallback `humans
    /// on the other side`; Standard Doctrine at any preset is Doctrine and
    /// never substituted, so no production match substitutes anything before
    /// 17b cut M6 (production holds Doctrine alone, the owner's OQ-2).
    ///
    /// Cleared with the rest of the board at the return to `Idle`.
    /// `#[serde(default)]` and skipped when `false`, under the live record's
    /// `LIVE_FORMAT` policy: a centre with humans on one side writes the bytes
    /// it always wrote.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub doctrine_only: bool,
}

impl Centre {
    /// Whether the centre is holding a match: anything a second `SIM START` or
    /// a staff `@arena/start` must not walk into.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        matches!(
            self.state,
            MatchState::Selection
                | MatchState::Boarding
                | MatchState::Running
                | MatchState::Stopping
        )
    }

    /// The slot one account holds.
    #[must_use]
    pub fn slot_of_account(&self, account: AccountId) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.account == account)
    }

    /// The slot one character holds.
    #[must_use]
    pub fn slot_of_pilot(&self, pilot: ObjId) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.pilot == pilot)
    }

    /// How many slots a side holds.
    #[must_use]
    pub fn side_len(&self, team: Team) -> usize {
        self.slots.iter().filter(|slot| slot.team == team).count()
    }

    /// Whether `account` leads `team`.
    #[must_use]
    pub fn leads_side(&self, team: Team, account: AccountId) -> bool {
        self.leads
            .iter()
            .any(|(side, lead)| *side == team && *lead == account)
    }

    /// **Settle the side leads over the slots as they stand** (16b cut AA.2; 16c
    /// `B-78`) — the one rule for every site that removes slots. Per side: the
    /// recorded lead stays while that account still holds a slot on the side;
    /// otherwise the side's first remaining slot in claim order leads; a side
    /// with no slot has no entry. `SIM LEAVE` and the boarding deadline both
    /// call it after their `retain`, so the two cannot drift again: before this
    /// cut the deadline dropped a lead and kept its entry, and the side could not
    /// `SIM CONCEDE` for the rest of the match.
    ///
    /// **16b cut B.4** (cut AA's residual): a slot whose pilot has **left** the
    /// tank mid-match ([`Slot::left`]) neither keeps the lead nor inherits it —
    /// the slot stays on the board for the result record, but its pilot is in
    /// the team room and cannot speak for a side still fighting. A side whose
    /// every slot has left has no entry. `slot_left` calls this the moment it
    /// marks a slot, so the side's next claimant still in a tank leads at once.
    ///
    /// The vector is rebuilt in [`Team::ALL`] order. Nothing reads the order:
    /// every reader asks `any`.
    ///
    /// **17b cut M5.1** (the design note's §2.4): before the check-out — in
    /// `Selection` and `Boarding`, which is `SIM LEAVE` and the boarding
    /// deadline — a side whose lead account changes loses the fill choice that
    /// lead set ([`Centre::fill_minds`], [`FillSetter::Lead`]), so the fill
    /// reverts to Standard Doctrine on Line and no new lead reads or inherits
    /// another account's choice; a staff choice stays. From `Running` on the
    /// entry is what the check-out resolved and stays as it is.
    pub fn resettle_leads(&mut self) {
        let mut leads: Vec<(Team, AccountId)> = Vec::with_capacity(Team::ALL.len());
        for team in Team::ALL {
            let kept =
                self.leads
                    .iter()
                    .find(|(side, lead)| {
                        *side == team
                            && self.slots.iter().any(|slot| {
                                slot.team == team && slot.account == *lead && !slot.left
                            })
                    })
                    .map(|(_, lead)| *lead);
            let lead = kept.or_else(|| {
                self.slots
                    .iter()
                    .find(|slot| slot.team == team && !slot.left)
                    .map(|slot| slot.account)
            });
            if let Some(lead) = lead {
                leads.push((team, lead));
            }
        }
        self.leads = leads;
        if matches!(self.state, MatchState::Selection | MatchState::Boarding) {
            let leads = &self.leads;
            self.fill_minds.retain(|fill| match fill.by {
                FillSetter::Staff => true,
                FillSetter::Lead(account) => leads
                    .iter()
                    .any(|(side, lead)| *side == fill.team && *lead == account),
            });
        }
    }

    /// **Forget that every pilot had left** (16b cut D.3, `B-33`) — the stamp and
    /// the once-per-match warning together, so the two can never drift apart.
    fn clear_unattended(&mut self) {
        self.unattended_since_ms = None;
        self.unattended_warned = false;
    }

    /// The lowest unclaimed slot index on a side, or `None` when it is full.
    fn free_index(&self, team: Team, cap: u8) -> Option<u8> {
        (1..=cap).find(|index| {
            !self
                .slots
                .iter()
                .any(|slot| slot.team == team && slot.index == *index)
        })
    }
}

/// Why a `SIM` verb refused. Every variant is exactly the line the player sees.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    /// This world has no simulator centre (no OOC pack at version 4).
    #[error("This server has no simulator centre.")]
    NoCentre,
    /// The actor is not in one of the centre's three rooms.
    #[error("{OUTSIDE_CENTRE}")]
    OutsideCentre,
    /// The tacsim layer did not compose.
    #[error("The arena is not available on this server.")]
    Unavailable,
    /// **The layer composed but is switched off** (16b cut F.4).
    ///
    /// The centre's rooms and pods come with the starter area, so they are here
    /// whatever `[tacsim].enabled` says — but the arena, its domain and the
    /// stock tanks are the game's own boot packs and ride on that flag. Before
    /// this variant, two players could claim slots in such a centre and only
    /// `SIM START` refused, with advice ("staff must re-apply the arena") that
    /// named a thing no verb can do. The refusal belongs at the front instead.
    #[error(
        "The TacSim layer is off on this server, so the simulator has no arena and no tanks; \
         staff must turn it on and restart."
    )]
    LayerOff,
    /// The account oracle does not know this character (fail closed, doc 10 R3).
    ///
    /// **16b cut F.4 made this sentence true.** It used to say "reconnect, or
    /// ask staff to relink the character", and both were wrong: the map was
    /// built once at boot, so reconnecting re-read nothing, and no staff verb
    /// relinks. The map now learns of every character as it is created, so
    /// reaching this means the character genuinely belongs to no account — a
    /// seeded `#1`, or an account document that names it twice — and there is
    /// nothing the player or staff can type to change that. So it says what is
    /// true and promises no repair.
    #[error("This character has no account on record, so the simulator cannot hold a slot for it.")]
    UnknownAccount,
    /// The account already holds a slot.
    #[error("You already hold {team} slot {index}. `SIM LEAVE` gives it up.")]
    AlreadyClaimed {
        /// Which side.
        team: &'static str,
        /// Which slot.
        index: u8,
    },
    /// That side has no free slot.
    #[error("The {team} side is full: it takes {max} pilot(s).")]
    SideFull {
        /// Which side.
        team: &'static str,
        /// How many it takes.
        max: u8,
    },
    /// The actor holds no slot.
    #[error("You hold no slot in the simulator.")]
    NoSlot,
    /// **`SIM UNIT` with no slot to put a unit in** (16b cut Q.8).
    ///
    /// [`SimError::NoSlot`] by another name, said as the thing to do next: the
    /// selector is the one a pilot is most likely to reach for before claiming,
    /// because the list of fieldable units is the reason to want a slot.
    #[error("Claim a slot first: SIM TEAM red|blue.")]
    NoSlotToField,
    /// **`SIM UNIT <word>` naming nothing the catalog carries** (16b cut Q.8).
    #[error("No such unit type: {word}. SIM UNIT lists them.")]
    NoSuchUnitType {
        /// What was typed, as it was typed.
        word: String,
    },
    /// **A chosen unit type that would not go onto the slot's tank** (16b cut
    /// Q.8), refusing the `SIM START` that tried to fit it.
    ///
    /// Everything [`mush_tactics::module::refit_unit`] refuses with: a roster
    /// tank that is not a unit, one still carrying a `deployed` marker, a frozen
    /// definition owner, a reference the registry does not hold. Every one of
    /// them is a broken world rather than a player's mistake, so the line names
    /// the slot and advises no repair.
    ///
    /// **The sentence does not quote the reason** (16b cut Q.8, fixer:
    /// correctness Q8-6). `refit_unit`'s reasons carry the tank's object number
    /// (`#57 is deployed; ...`), and this line is shown to whoever typed `SIM
    /// START` — who may be the *other* side's lead, and for whom an object
    /// number is a fact about a machine their platform has not detected (doc 16
    /// §1, §5). The slot's own public designator is already on the board, so it
    /// stays; the raw reason goes to the server log, which is where the staff
    /// who can act on it read. [`SimError::Refit::why`] carries it there and is
    /// deliberately not in the message.
    #[error(
        "Slot {team} {index} cannot field {unit_type}: the simulator refused the refit. Staff \
         should read the server log."
    )]
    Refit {
        /// Which side.
        team: &'static str,
        /// Which slot.
        index: u8,
        /// The unit type it asked for, as the board spells it.
        unit_type: String,
        /// **What refused, for the server log only** — never in the sentence,
        /// because it names the object.
        why: String,
    },
    /// The centre is not in a state that admits this.
    #[error("The simulator centre is {state}; that is not something you can do to it now.")]
    WrongState {
        /// What state it is in.
        state: &'static str,
    },
    /// Only a team lead or staff may start.
    #[error("Only a side's lead or staff can start the match.")]
    NotLead,
    /// **`SIM AI <n>` from somebody who leads no side** (16b cut D.1).
    ///
    /// [`SimError::NotLead`] is `SIM START`'s sentence and says so; `SIM AI`
    /// answered it too, so a player who asked the fill to be three was told they
    /// could not start the match. The bare read is now served to anybody in the
    /// centre and only the *set* is gated, so this is the sentence for the set.
    #[error("Only a side's lead or staff can set the AI fill.")]
    NotLeadForAi,
    /// **`SIM MODE <word>` from somebody who leads no side** (16b cut O.1): the
    /// mode is a match-wide setting, and a match-wide setting is the lead's or
    /// staff's, as `SIM AI`'s is.
    #[error("Only a side's lead or staff can set the mode.")]
    NotLeadForMode,
    /// **`SIM MODE <word>` naming no mode** (16b cut O.1).
    #[error("No such mode: {word}. SIM MODE lists them.")]
    NoSuchMode {
        /// What was typed, as it was typed.
        word: String,
    },
    /// **`SIM MODE <word>` naming a mode the ground does not declare** (16b cut
    /// O.1). No production composition reaches it — the catalog is compiled in
    /// and the centre's arena is always the proving ground, which declares
    /// `hold` — so its sentence is pinned in-source.
    #[error("{arena} does not support {mode}; SIM MODE lists what it does.")]
    ModeUnsupported {
        /// The mode's wire word.
        mode: &'static str,
        /// The ground's authored name ([`crate::match_::record::arena_label`]).
        arena: String,
    },
    /// **`SIM MODE range`** (16b cut O.3; plan item 21): the range is one set
    /// and one read, and both are `SIM RANGE`'s — a range needs a count, which
    /// `SIM MODE` has no word for.
    #[error("SIM RANGE <n> sets the range.")]
    RangeViaRange,
    /// **`SIM RANGE <n>` from somebody who leads no side** (16b cut O.3): a
    /// match-wide setting is the lead's or staff's, as `SIM AI`'s and `SIM
    /// MODE`'s are.
    #[error("Only a side's lead or staff can set the range.")]
    NotLeadForRange,
    /// **`SIM RANGE <n>` with slots claimed on both sides** (16b cut O.3): the
    /// range is the claimed side against a side of targets, so the targets need
    /// a side nobody claimed.
    #[error("The range is one side against the targets; every slot must be on one side.")]
    RangeTwoSides,
    /// **`SIM RANGE <n>` outside the side's pods** (16b cut O.3).
    #[error("The range fields 1 to {max} targets.")]
    RangeCount {
        /// The upper bound: [`slots_per_side`].
        max: u8,
    },
    /// **`SIM TEAM` onto the targets' side while the mode is a range** (16b cut
    /// O.3): those pods are the targets'.
    #[error("The range puts targets on the {team} side; claim a {other} slot.")]
    RangeSide {
        /// The targets' side.
        team: &'static str,
        /// The shooters' side.
        other: &'static str,
    },
    /// **`SIM AI <n>` while the mode is a range** (16b cut O.3; plan item 39):
    /// the fill does not run on a range — its pods are the targets' — so a set
    /// would be a number nothing reads, and D.1's rule is that what `SIM AI`
    /// says be true on the server it is said on.
    #[error("The range fields no machines; SIM RANGE sets its targets.")]
    AiOnRange,
    /// **`SIM CONCEDE` from somebody who is not their side's lead** (16b
    /// cut D.3, `B-33`).
    ///
    /// Conceding ends the match for everybody on that side, so it is the lead's
    /// alone — the same right that starts it. Staff get no special case here:
    /// `@arena/abort` is theirs, and an operator ending somebody's match as a
    /// *loss for one side* would be the console choosing who won.
    #[error("Only a side's lead can concede for it.")]
    NotLeadToConcede,
    /// **`SIM ABORT` in `Selection` by anyone but staff** (16b cut D.3, `T-60`).
    ///
    /// There is no match in `Selection`: the id is not minted, no pod is locked
    /// and nothing is checked out. One claimant's `SIM ABORT` nevertheless wiped
    /// every other claimant's slot, which is a lobby's one participant dissolving
    /// it for the rest — so the sentence names the verb that gives up the one
    /// thing they hold. Staff may still call a `Selection` off, because a stuck
    /// lobby is exactly the thing a staff lever is for.
    #[error("Nothing is running yet. `SIM LEAVE` gives your slot up.")]
    NothingToAbort,
    /// **`SIM ABORT` or `SIM CONCEDE` while the arena cannot take a stop** (16b
    /// cut M.1, `B-49`): its inbox is full, or its result is already on its way
    /// to the world ([`crate::match_::AbortOutcome::Busy`]). Nothing changed and
    /// nothing was announced; the match is still the arena's to finish, and the
    /// same verb a moment later lands.
    #[error("The arena is busy; try that again in a moment.")]
    ArenaBusy,
    /// A side has nobody.
    #[error("The {0} side has no pilots; a match takes at least one a side.")]
    EmptySide(&'static str),
    /// **`SIM UNIT <type>` past the class cap** (16b cut AE.3; doc 16 §2 W2:
    /// *no drone swarms; `arena.max_units_by_class`*).
    ///
    /// The caller's own side already fields `cap` of the chosen type's class in
    /// its other claimed slots (by their effective types: the choice on a slot,
    /// else its pod's authored type). The sentence names the class and the
    /// number and nothing about the slots that hold them, so it tells the caller
    /// nothing their own side's board does not. One variant, two sentences: a
    /// cap of zero is the operator forbidding the class, and reads `This server
    /// fields no drones; pick another type.` rather than a count of nought.
    #[error("{}", class_cap_line(*.class, *.cap))]
    ClassCap {
        /// The chosen type's class.
        class: crate::definitions::UnitClass,
        /// `[tacsim.arena].max_units_by_class` for that class.
        cap: u32,
    },
    /// **`SIM UNIT <type>` or `SIM MIND …` past the side's half of the
    /// centre's compute pool** (17b cut M3.5; doc 17 §9, D-UM-36: *`SIM MIND`
    /// and `SIM UNIT` check at choice time … refused with the arithmetic*),
    /// said in the pool's sentence ([`crate::match_::pool::PoolRefusal`], the
    /// owner's OQ-11 wording since 17b cut M5.1). **17b cut M5.1**: every
    /// claimed hull of the side loads its class — the class its chosen mind
    /// seals at, else `[tacsim.minds].fill_class`, seated or not — so the
    /// later of `SIM UNIT` and `SIM MIND` is the one refused (doc 17 §12).
    /// The question is the side AFTER the change — the hull's new class
    /// beside every other hull's — unless the change lowers the hull's load,
    /// which is never refused (17b cut M5.1's review): a side that an
    /// unchecked claim or `SIM AI <n>` has put past its half refuses every
    /// `SIM UNIT` and `SIM MIND` of it that keeps or raises a hull's load,
    /// `SIM MIND none` back to `fill_class` included, until a claim is given
    /// up or the fill lowered.
    /// At the stock constants it cannot refuse (twelve Class III a side fit,
    /// `crate::match_::pool`'s head); an operator's lowered pool reaches it.
    #[error("{0}")]
    Pool(crate::match_::pool::PoolRefusal),
    /// **`SIM MIND <word>` naming no mind the character holds** (17b cut
    /// M5.1; D-UM-49): a refusal that posts nothing.
    #[error("You hold no mind called {word}; SIM MIND lists the minds you hold.")]
    NoSuchMind {
        /// What was typed, as it was typed.
        word: String,
    },
    /// **`SIM MIND <mind> @<rev>`** (17b cut M5.1; the owner's OQ-2): a
    /// revision pin waits for 17b cut M6's workshop.
    #[error(
        "No revisions are kept before the workshop; SIM MIND Standard seals the Registry's own."
    )]
    NoRevisions,
    /// **A profile the mind does not carry** (17b cut M5.1; OQ-3): its
    /// presets, listed.
    #[error("{mind} has no profile called {word}: {presets}.")]
    NoSuchProfile {
        /// The mind's name.
        mind: String,
        /// What was typed, as it was typed.
        word: String,
        /// Its presets, as a sentence lists them (`Line, Sentry … or
        /// Rearguard`).
        presets: String,
    },
    /// **One profile named twice** (17b cut M5.1): a profile is sealed once.
    #[error("{profile} is named twice; a profile is sealed once.")]
    ProfileTwice {
        /// The profile's canonical spelling.
        profile: String,
    },
    /// **More profiles than the sealed class stores** (17b cut M5.1; OQ-3:
    /// the class row's `profiles`, Class I → 2, read at the class the mind
    /// would be sealed at on this chassis). A chassis with no core flies the
    /// ROM (C-24), Class 0's row, and the sentence says so rather than naming
    /// a core it does not have (17b cut M5.1's review): `This target_mk1 has
    /// no core and flies the ROM, which seals one profile at most.`
    #[error("{}", too_many_profiles_line(.mind, *.class, *.most, .rom.as_deref()))]
    TooManyProfiles {
        /// The mind's name.
        mind: String,
        /// The class it would be sealed at.
        class: crate::arena::core::CoreClass,
        /// The class row's `profiles`.
        most: u8,
        /// The chassis, as the board spells it, when it has no core and flies
        /// the ROM; `None` on a chassis with a core.
        rom: Option<String>,
    },
    /// **A profile whose odds are under the operator's Compact floor**
    /// (17b cut M5.1; D-UM-17: refused, never clamped) — on a chassis with a
    /// weapons station only (C-23: an unarmed hull never refuses for the
    /// floor).
    #[error(
        "{profile} fires at {}% and better; this server's Compact floor is {}%, so it cannot be \
         sealed.",
        .odds_permille / 10,
        .floor_permille / 10
    )]
    UnderFloor {
        /// The profile's canonical spelling.
        profile: String,
        /// Its `min_odds`, per mille.
        odds_permille: u32,
        /// `[tacsim.minds].odds_floor_permille`.
        floor_permille: u32,
    },
    /// **A profile whose survival pair does not hold on this server's
    /// doctrine** (17b cut M5.1; the card's `pair`, run on every chassis):
    /// Rearguard's `break_off_pct` 70 under a configured `fight_home_pct`
    /// above it.
    #[error(
        "{profile} cannot be sealed here: it would put fight_home_pct {fight_home} above \
         break_off_pct {break_off}."
    )]
    ProfilePair {
        /// The profile's canonical spelling.
        profile: String,
        /// `fight_home_pct` as the card prints it.
        fight_home: String,
        /// `break_off_pct` as the card prints it.
        break_off: String,
    },
    /// **A mind rated above a governed core's class** (17b cut M5.1;
    /// D-UM-41, the owner's OQ-10 (d); the design note's C-24): refused at
    /// the choice. A chassis with no core is never over-class (it flies the
    /// ROM), nor one whose governed core is Class 0 (17b cut M5.2's S1
    /// ruling: it flies the ROM, as no core does).
    #[error(
        "{mind} thinks at Class {rated_word}; this {chassis}'s core is Class {core_word} and is \
         governed.",
        rated_word = .rated.as_str(),
        core_word = .core.as_str()
    )]
    OverClass {
        /// The mind's name.
        mind: String,
        /// The class it is rated at.
        rated: crate::arena::core::CoreClass,
        /// The chassis, as the board spells it.
        chassis: String,
        /// The core's class.
        core: crate::arena::core::CoreClass,
    },
    /// **`SIM AI MIND` with more than one profile** (17b cut M5.1): nobody
    /// aboard a fill hull switches it.
    #[error("The fill flies one profile; nobody aboard switches it.")]
    FillOneProfile,
    /// **Staff naming a mind that is not Standard Doctrine for a fill** (17b
    /// cut M5.1; D9: staff hold no holdings, D-UM-49).
    #[error("Staff hold no minds; SIM AI MIND Standard sets the fill's doctrine and preset.")]
    StaffHoldNoMinds,
    /// **Staff setting a fill without naming its side** (17b cut M5.1): a
    /// lead's side is their own, staff lead none.
    #[error("Which side's fill? SIM AI MIND red Standard sets red's.")]
    FillWhichSide,
    /// **A chosen mind no longer held at the check-out** (17b cut M5.1; the
    /// design note's M5.1 item 6): `ready` resolves every fielded hull's
    /// choice against the holdings seam, and a holding the pilot (or, for a
    /// fill, the lead who set it) no longer holds calls the match off. The
    /// sentence names the slot or the fill and NEVER the mind, because the
    /// call-off is said to the whole centre (C-4). Unreachable in production
    /// (every character holds Standard Doctrine); reached with a test
    /// provider.
    #[error("{}", mind_not_held_line(.team, *.index))]
    MindNotHeld {
        /// The side's word.
        team: &'static str,
        /// The slot's index, or `None` for the side's fill.
        index: Option<u8>,
    },
    /// **A side that fields no gun** (16b cut AE.3; plan §6 AE-X3).
    ///
    /// Every machine the side would fly carries no weapon component — a lone
    /// drone — so the side could see and never shoot, and the match could only
    /// end on the clock. Bound in `ready()`, where the sides are final (the
    /// boarding deadline's drops and the fill are both behind it), so a drop
    /// that leaves a side a lone drone is called off with `Match N could not
    /// start: …`; and said early at `SIM START`, before anything is written,
    /// through the same `side_fields_a_gun`. Never for the range's target
    /// side, which is targets and shoots at nobody.
    #[error("The {team} side fields no gun; a side needs a machine that carries one.")]
    NoGun {
        /// The side's word.
        team: &'static str,
    },
    /// **Nobody at all is playing** (16b cut Q.7), with the AI fill on.
    ///
    /// The fill lets one human fight the machines; it does not let a centre run
    /// a match nobody is in. This is what is left of `EmptySide` when a side
    /// without a pilot is legal and a *centre* without one is not.
    #[error("Nobody is in the simulator; a match takes at least one pilot.")]
    NobodyPlaying,
    /// **`SIM AI` on a server whose fill is off** (16b cut Q.7).
    #[error("AI fill is off on this server.")]
    AiOff,
    /// **`SIM AI <n>` outside the centre's pods** (16b cut Q.7).
    #[error("AI fill is 1 to {max} a side.")]
    AiRange {
        /// The upper bound: [`slots_per_side`].
        max: u8,
    },
    /// **`SIM AI <n>` below the side that has already claimed more than that**
    /// (16b cut Q.7): the fill tops a side up, it never drops a pilot.
    #[error("AI fill cannot be below the larger side ({claimed}).")]
    AiBelowSide {
        /// How many slots the larger side holds.
        claimed: u8,
    },
    /// The centre's arena, or one of its units, is not in this world.
    ///
    /// **16b cut F.4 made the advice performable**, and **16b cut Z.3 made it
    /// true again.** F.4 said the arena was applied at boot and no verb could
    /// re-apply it, which was so while the arena pack was a boot pack and not on
    /// the `@atlas/pack` shelf. Z.3 shelves the game's two boot packs beside the
    /// starter packs — the latch is still what makes a second boot make no
    /// thirteenth tank, and the shelf is how a standing world takes a newer
    /// version by hand — so staff can re-apply the arena, with the centre idle
    /// (the applier refuses while a roster tank is under a match's custody).
    ///
    /// The layer-off case refuses earlier with [`SimError::LayerOff`], and since
    /// Z.3 the lobby's cap ([`slots_per_side`]) is the slots whose pod *and*
    /// tank exist on both sides, so a claim above the first missing tank is
    /// refused as a full side. Three paths still reach this refusal: the cap's
    /// floor of one, which leaves slot 1 claimable on a world whose pods stand
    /// and whose roster is empty; a tank destroyed or `@unit/remove`d under a
    /// slot that was already claimed; and K.1 deviation (1), a world that
    /// applied the arena before its roster existed.
    ///
    /// **The route in the sentence repairs only a world whose arena latch is
    /// behind the shipped version** (or one that never applied it): the shelf
    /// takes a *newer* version, and on a world already at the shipped one the
    /// plan is ` Nothing to do.` and the apply refuses `is applied at this
    /// version`. It does not re-make a stock tank lost on a current world; that
    /// has no verb yet, and `deploy/README.md` says so.
    #[error(
        "The simulator has no unit for {team} slot {index}. Staff apply the arena pack with \
         @atlas/pack plan tacsim.pack.proving_ground and @atlas/pack apply, while the centre is \
         idle."
    )]
    NoUnit {
        /// Which side.
        team: &'static str,
        /// Which slot.
        index: u8,
    },
    /// The check-out itself refused, with the reason cut T.3 gave — since 17b
    /// cut M5.1 (its review) in the call-off's own words
    /// (`crate::match_::StartError::call_off_words`), one sentence: what the
    /// last `BOARD`er reads, under 120 columns.
    #[error("The match could not start: {0}")]
    Start(String),
}

/// **[`SimError::ClassCap`]'s sentence** (16b cut AE.3): the count, in the
/// class's own noun, or — at a cap of zero — the class forbidden on this
/// server.
fn class_cap_line(class: crate::definitions::UnitClass, cap: u32) -> String {
    if cap == 0 {
        format!(
            "This server fields no {}; pick another type.",
            class.noun(0)
        )
    } else {
        format!(
            "A side fields at most {cap} {}; pick another type.",
            class.noun(cap)
        )
    }
}

/// **[`SimError::MindNotHeld`]'s sentence** (17b cut M5.1): the slot, or the
/// side's fill — never the mind.
fn mind_not_held_line(team: &str, index: Option<u8>) -> String {
    match index {
        Some(index) => {
            format!("Slot {team} {index}'s chosen mind is no longer held by its pilot.")
        }
        None => format!("The {team} fill's chosen mind is no longer held by its lead."),
    }
}

/// **A small count in words** (17b cut M5.1): `one` to `six`, the figure
/// past them — the lobby's mind lines never print more than a class row's
/// `profiles` (at most four) or a held mind's presets.
fn count_word(n: usize) -> String {
    match n {
        1 => "one".to_owned(),
        2 => "two".to_owned(),
        3 => "three".to_owned(),
        4 => "four".to_owned(),
        5 => "five".to_owned(),
        6 => "six".to_owned(),
        n => n.to_string(),
    }
}

/// `one profile`, `two profiles` — [`SimError::TooManyProfiles`]'s count.
fn profiles_counted(n: u8) -> String {
    if n == 1 {
        "one profile".to_owned()
    } else {
        format!("{} profiles", count_word(usize::from(n)))
    }
}

/// **[`SimError::TooManyProfiles`]'s sentence** (17b cut M5.1): the class
/// the mind thinks at on the chassis's core, or — on a chassis with no core,
/// `rom` naming it — that the hull flies the ROM (C-24; its review).
fn too_many_profiles_line(mind: &str, class: CoreClass, most: u8, rom: Option<&str>) -> String {
    match rom {
        None => format!(
            "{mind} thinks at Class {} on this core, which seals {} at most.",
            class.as_str(),
            profiles_counted(most)
        ),
        Some(chassis) => format!(
            "This {chassis} has no core and flies the ROM, which seals {} at most.",
            profiles_counted(most)
        ),
    }
}

/// **A list as a sentence says it**: `a`, `a or b`, `a, b or c` (with `or`),
/// or the same with `and`.
pub(crate) fn listed<S: AsRef<str>>(items: &[S], last: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.as_ref().to_owned(),
        [head @ .., tail] => format!(
            "{} {last} {}",
            head.iter()
                .map(AsRef::as_ref)
                .collect::<Vec<&str>>()
                .join(", "),
            tail.as_ref()
        ),
    }
}

/// **A chosen mind as every line spells it** (17b cut M5.1): `Standard
/// Doctrine (Sniper, Line)`; an empty profile list is Line alone, `Standard
/// Doctrine (Line)`.
#[must_use]
pub fn mind_words(mind: &str, profiles: &[SmolStr]) -> String {
    if profiles.is_empty() {
        format!("{mind} (Line)")
    } else {
        format!(
            "{mind} ({})",
            profiles
                .iter()
                .map(SmolStr::as_str)
                .collect::<Vec<&str>>()
                .join(", ")
        )
    }
}

/// **A side's word with a capital** (`Red`, `Blue`), as a sentence opens
/// with it.
fn side_word(team: Team) -> &'static str {
    match team {
        Team::Red => "Red",
        Team::Blue => "Blue",
    }
}

// ---------------------------------------------------------------------------
// Reading the centre
// ---------------------------------------------------------------------------

/// The centre, as it stands.
#[must_use]
pub fn centre(w: &World) -> Option<&Centre> {
    w.module_state::<MatchBook>().map(MatchBook::centre)
}

/// **The character leading `team` on the centre's board** (17b cut M3.3; doc
/// 17 §12, §19): the slot of the side's lead account ([`Centre::leads`]),
/// its pilot — the warrant principal of the side's AI fill, the one who set
/// what the fill flies. `None` unless the centre is `Boarding`: the
/// check-out (`crate::match_::start`) runs there for a lobby match and never
/// for a staff `@arena/start` (refused while the centre is busy), so a staff
/// match's fill, and a side with no lead, have no principal and nobody is
/// told.
#[must_use]
pub fn lead_character_of(w: &World, team: Team) -> Option<ObjId> {
    let centre = centre(w)?;
    if centre.state != MatchState::Boarding {
        return None;
    }
    let (_, account) = centre.leads.iter().find(|(side, _)| *side == team)?;
    centre
        .slots
        .iter()
        .find(|slot| slot.team == team && slot.account == *account)
        .map(|slot| slot.pilot)
}

/// The configured `[tacsim.arena]` and `[tacsim.match]` blocks, which every
/// transition past `Selection` needs and which cut K.2's `BOARD` does not carry.
///
/// **16b cut Q.1** made it `pub(crate)`: `RANGE` and `BEARING` read
/// `[tacsim.sensors].pass_ms` to decide whether a plot is stale enough to say so,
/// and reading the configuration through the one accessor is what keeps the
/// cockpit from growing a second copy of where the blocks live.
#[must_use]
pub(crate) fn configs(w: &World) -> Option<(ArenaConfig, MatchConfig, SensorsConfig)> {
    let cfg = w.module_state::<crate::TacsimStore>()?.config();
    Some((cfg.arena.clone(), cfg.match_.clone(), cfg.sensors.clone()))
}

/// The configured `[tacsim.command]` block (**16b cut Y.2**; doc 16 §2 W8):
/// the span and the rate `ORDER` is held to, read from the one place
/// [`configs`] reads, so the cockpit grows no second copy of where the blocks
/// live.
#[must_use]
pub(crate) fn command_config(w: &World) -> Option<CommandConfig> {
    let cfg = w.module_state::<crate::TacsimStore>()?.config();
    Some(cfg.command.clone())
}

/// The configured `[tacsim.minds]` block (**17b cut M1b.1**; doc 17 §5,
/// D-UM-17 as amended): the Compact floor `crate::arena_bridge::open_match`
/// hands to `crate::arena_task::spawn_arena`, which captures it into the arena
/// at check-out. Read from the one place [`configs`] reads; a running or a
/// resumed match never reads it again.
#[must_use]
pub(crate) fn minds_config(w: &World) -> Option<MindsConfig> {
    let cfg = w.module_state::<crate::TacsimStore>()?.config();
    Some(cfg.minds.clone())
}

/// How many slots one side of **this** centre takes: the configured cap, held
/// down to the pods that exist — `min(configured, PODS_PER_SIDE, fieldable)`,
/// never below one.
///
/// **16b cut Z.3 made the doc true.** It used to clamp to the compile-time
/// [`PODS_PER_SIDE`] alone, which was the same number as the pods in every
/// world while both were four. Z.3 moves the constant to six and ships the two
/// packs that make pods and tanks 5 and 6 — which a standing world takes by
/// hand — so the constant is now what the binary can seat and
/// [`centre::fieldable_per_side`] is what this world can: every pod and every
/// tank, on both sides, slot by slot. Without it a Z.3 binary on an un-migrated
/// world handed out slot 5 and every `SIM START` then refused with
/// [`SimError::NoCentre`], since the pod lookup runs before the unit check.
/// [`largest_side`]'s floor and [`ai_side_size`] follow through this cap.
#[must_use]
pub fn slots_per_side(w: &World, match_cfg: &MatchConfig) -> u8 {
    u8::try_from(match_cfg.max_slots_per_side)
        .unwrap_or(PODS_PER_SIDE)
        .min(PODS_PER_SIDE)
        .min(centre::fieldable_per_side(w))
        .max(1)
}

/// **The claimed slots of the larger side** (16b cut Q.7), never below one.
///
/// It is the floor under [`ai_side_size`] and the number `SIM AI` refuses to go
/// below: the fill tops a side up to a count, so a count under what somebody has
/// already claimed would be an instruction to un-claim a slot.
///
/// **16b cut D.3 (`T-51`)**: held inside `1..=PODS_PER_SIDE`, the way
/// [`ai_side_size`] holds its own answer. No path reaches it — `claim` refuses
/// past [`slots_per_side`], which is itself clamped to the pods that exist — but
/// a floor that could exceed the cap it is a floor under is a sum waiting to be
/// wrong, and a `u8::MAX` out of the `try_from` below would have been a fill of
/// 255 if anything ever fed it one.
#[must_use]
pub fn largest_side(centre: &Centre) -> u8 {
    Team::ALL
        .iter()
        .map(|team| u8::try_from(centre.side_len(*team)).unwrap_or(u8::MAX))
        .max()
        .unwrap_or(0)
        .clamp(1, PODS_PER_SIDE)
}

/// **How many units each side fields with the AI fill on** (16b cut Q.7): the
/// number `SIM AI` set, never below [`largest_side`], held inside
/// `1..=slots_per_side`.
///
/// **This departs from cut N.1's "its whole four pods"**, deliberately and in
/// the owner's own words: a solo player who typed `SIM START` against the fill
/// got a four-a-side battle with seven machines in it. The fill now answers the
/// size of the game the humans turned up for, and `SIM AI 4` is how somebody who
/// wants the old behaviour asks for it.
///
/// **The number `SIM AI` stores is a floor and not a cap.** `SIM AI` refuses a
/// size below what is already claimed ([`SimError::AiBelowSide`]) — but claiming
/// goes on for the whole of `Selection`, so a lead who typed `SIM AI 2` and then
/// watched two friends claim would otherwise have set up a 4 v 2 under a board
/// line reading `each side fields 2`. Reading the stored number against
/// `largest_side` on every read is what keeps the two sides the same size, which
/// is [`Centre::ai_side_size`]'s own rule: a fill that let one side ask for more
/// machines than the other would be the lobby choosing who wins.
#[must_use]
pub fn ai_side_size(w: &World, centre: &Centre, match_cfg: &MatchConfig) -> u8 {
    let cap = slots_per_side(w, match_cfg);
    centre
        .ai_side_size
        .unwrap_or(0)
        .max(largest_side(centre))
        .clamp(1, cap)
}

/// **The range's two sides** (16b cut O.3; 16c N13): `(shooters, targets)` —
/// the side every claimed slot is on, and the other one — or `None` when the
/// centre is not set to a range or holds no slot to read a side off.
///
/// Under a range every claim is on one side by construction ([`range`] refuses
/// a centre with slots on both, [`claim`] refuses the targets' side), so the
/// first slot's side is every slot's. The returns to `Idle` clear the range with
/// the slots, so the only states with a range and no slot are the terminal ones,
/// after `release_rows` has dropped the rows; there the board prints no range
/// line and the bare `SIM RANGE` read names no side.
#[must_use]
pub fn range_sides(centre: &Centre) -> Option<(Team, Team)> {
    if centre.mode != MatchMode::Range {
        return None;
    }
    let shooters = centre.slots.first()?.team;
    Some((shooters, shooters.other()))
}

/// **A count of targets in words** (16b cut O.3, fixer): `1 target`, `2
/// targets` — the report's own `1 shot, 1 hit` rule, because `SIM RANGE 1` is
/// the flow the help recommends and `1 targets` is the first line it prints.
fn targets_counted(count: u8) -> String {
    if count == 1 {
        "1 target".to_owned()
    } else {
        format!("{count} targets")
    }
}

/// **The pod one pilot holds** (cut K.2's `BOARD` asks this first).
#[must_use]
pub fn pod_of(w: &World, pilot: ObjId) -> Option<ObjId> {
    centre(w)?.slot_of_pilot(pilot)?.pod
}

/// **The pilot one pod is locked to** (cut K.2's `BOARD` refuses anyone else).
#[must_use]
pub fn pilot_of(w: &World, pod: ObjId) -> Option<ObjId> {
    centre(w)?
        .slots
        .iter()
        .find(|slot| slot.pod == Some(pod))
        .map(|slot| slot.pilot)
}

/// **The tank a pod flies** — re-exported from [`crate::match_::centre`], which
/// derives it from the arena pack's own slot ids.
#[must_use]
pub fn tank_of(w: &World, pod: ObjId) -> Option<ObjId> {
    centre::tank_of(w, pod)
}

/// **Refuse every lobby verb on a server with the layer switched off** (16b cut
/// F.4).
///
/// First, and before anything reads the centre: the whole of what the lobby does
/// ends in an arena that is not there, and a player who is going to be refused is
/// owed the refusal at the word they typed rather than three verbs later. The
/// `tacsim` module registers its verbs unconditionally so a disabled layer
/// "refuses out loud rather than hiding a verb" (E5c deviation 1) — this is the
/// half of that sentence the lobby did not have.
pub(crate) fn require_layer(w: &World) -> Result<(), SimError> {
    let store = w
        .module_state::<crate::TacsimStore>()
        .ok_or(SimError::Unavailable)?;
    if store.enabled() {
        Ok(())
    } else {
        Err(SimError::LayerOff)
    }
}

/// The account behind a character, failing closed on the unknown.
fn account_of(w: &World, actor: ObjId) -> Result<AccountId, SimError> {
    w.module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(actor))
        .ok_or(SimError::UnknownAccount)
}

/// Staff, on this crate's one test.
///
/// **16b cut V.3** made it `pub(crate)`: the report completion re-validates a
/// staff page's reader with it (`crate::match_::report::ReportRead`).
pub(crate) fn is_staff(w: &World, actor: ObjId) -> bool {
    mush_core::perm::has_power(w, actor, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, actor, mush_core::Role::Royalty)
}

/// **The `SIM STATUS` half of `in_centre`** (16b cut F.4): refuse an actor who
/// is not standing in the centre, without naming which room they are in.
///
/// It exists because the read used to lean on the `SIM` command gate for this,
/// and the gate is gone (see the comment above `sim::register`).
///
/// # Errors
///
/// [`SimError::NoCentre`] or [`SimError::OutsideCentre`], as the line the player
/// sees.
pub fn outside_centre(w: &World, actor: ObjId) -> Result<(), SimError> {
    in_centre(w, actor).map(|_| ())
}

/// Refuse an actor who is not standing in the centre.
fn in_centre(w: &World, actor: ObjId) -> Result<CentreRoom, SimError> {
    if centre::centre_rooms(w).is_none() {
        return Err(SimError::NoCentre);
    }
    centre::where_in_centre(w, actor).ok_or(SimError::OutsideCentre)
}

// ---------------------------------------------------------------------------
// `Idle -> Selection`: the claim
// ---------------------------------------------------------------------------

/// **`SIM TEAM red|blue`** — claim one slot on a side (doc 16 §3.8,
/// `Selection`).
///
/// One slot per **account**, at most [`slots_per_side`] a side, no budgets,
/// terms or consent (16a Q28). The first claimant of a side becomes its lead,
/// which is the only extra right the claim carries: `SIM START`.
///
/// # Errors
///
/// [`SimError`], as the line the player sees.
pub fn claim(ctx: &mut Ctx<'_>, team: Team) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let account = account_of(ctx.w, actor)?;
    let (_, match_cfg, _) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    let cap = slots_per_side(ctx.w, &match_cfg);
    // **16b cut F.1**: the first claim on a **terminal** centre clears the board.
    //
    // The K row's deviation 3 keeps the outcome on the centre after a check-in so
    // `SIM STATUS` can say how the last match went, with `@arena/reset` the way
    // back to `Idle`. That is still what happens — right up to the moment
    // somebody wants to play again, and on a fresh world there is no front door
    // to a staff character at all, so two players alone could not start a second
    // match. Claiming a slot *is* the statement that the last match has been
    // read, so the claim performs the staff verb's own reset (the seat sweep, the
    // stock tanks back at spec and at muster, the centre to `Idle`) and then
    // claims. `@arena/reset` stays, staff-only and idempotent, for a centre
    // nobody is claiming into.
    if centre(ctx.w).is_some_and(|centre| centre.state.is_terminal()) {
        let outcome = reset(ctx);
        // **The line says only what the report says.** A tank the reset skipped
        // (still carrying a `deployed` marker) or could not write is one the next
        // match will fight on as it stands, and `@arena/reset` — the one surface
        // that names those rows — is behind a staff login these two players may
        // not have. So the claim never promises "back at muster" over a report
        // that does not, and when something is stuck it says so in a plain line
        // rather than leaving them to discover it as an unexplained loss.
        let stuck = outcome.tanks.deployed.len() + outcome.tanks.failed.len();
        if outcome.changed() {
            announce(
                ctx,
                if stuck == 0 {
                    "The last match is cleared from the board and the tanks are back at muster."
                        .to_owned()
                } else {
                    "The last match is cleared from the board.".to_owned()
                },
            );
        }
        if stuck > 0 {
            announce(
                ctx,
                format!(
                    "{stuck} tank(s) could not be put back and will fight the next match as they \
                     stand; ask staff to look at the simulator."
                ),
            );
        }
    }
    let index = {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let centre = book.centre_mut();
        if !matches!(centre.state, MatchState::Idle | MatchState::Selection) {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        if let Some(held) = centre.slot_of_account(account) {
            return Err(SimError::AlreadyClaimed {
                team: held.team.as_str(),
                index: held.index,
            });
        }
        // **16b cut O.3** (16c N13): under a range the other side's pods are the
        // targets', so a claim there is refused before the side's cap is read.
        if let Some((shooters, targets)) = range_sides(centre)
            && team == targets
        {
            return Err(SimError::RangeSide {
                team: targets.as_str(),
                other: shooters.as_str(),
            });
        }
        let index = centre.free_index(team, cap).ok_or(SimError::SideFull {
            team: team.as_str(),
            max: cap,
        })?;
        centre.state = MatchState::Selection;
        centre.arena = SmolStr::new_static(CENTRE_ARENA);
        centre.outcome = None;
        centre.slots.push(Slot {
            team,
            index,
            account,
            pilot: actor,
            pod: None,
            unit: None,
            left: false,
            dropped_at_ms: None,
            unit_type: None,
            released_by_resume: false,
            mind: None,
        });
        if !centre.leads.iter().any(|(side, _)| *side == team) {
            centre.leads.push((team, account));
        }
        index
    };
    let name = display_name(ctx.w, actor);
    // **16b cut D.1**: the advice has to be true on the server it is said on.
    // With the AI fill on, `SIM START` does not wait for a pilot on the other
    // side — it gives the machines the pods nobody claimed — so the old sentence
    // told a solo player to go and find an opponent they do not need.
    let fill = configs(ctx.w).is_some_and(|(_, match_cfg, _)| match_cfg.ai_fill);
    // **16b cut O.3**: and under a range neither of those is true — the targets
    // take the other side's pods and the machines take none, fill or no fill.
    let targets = centre(ctx.w)
        .and_then(range_sides)
        .map(|(_, targets)| targets);
    let advice = match targets {
        Some(targets) => format!(
            "`SIM START` begins the match; the targets stand on the {} side.",
            targets.as_str()
        ),
        None if fill => {
            "`SIM START` begins the match; the machines take the pods nobody claims.".to_owned()
        }
        None => "`SIM START` begins the match once both sides have a pilot.".to_owned(),
    };
    announce(
        ctx,
        format!("{name} takes {} slot {index}. {advice}", team.as_str()),
    );
    Ok(())
}

/// **`SIM LEAVE`** — give a slot up. The last release returns the centre to
/// `Idle`.
///
/// # Errors
///
/// [`SimError`], as the line the player sees.
pub fn leave(ctx: &mut Ctx<'_>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let account = account_of(ctx.w, actor)?;
    let (team, index, seated, emptied) = {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let centre = book.centre_mut();
        if !matches!(centre.state, MatchState::Selection | MatchState::Boarding) {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let held = centre
            .slot_of_account(account)
            .cloned()
            .ok_or(SimError::NoSlot)?;
        centre.slots.retain(|slot| slot.account != account);
        // The lead leaves: the next claimant of that side inherits it, so a
        // side is never left with no one who can start. 16b cut AA.2 (`B-78`):
        // through the one rule the boarding deadline runs as well.
        centre.resettle_leads();
        // 16b cut D.3 (`T-48`): the zeroing used to happen here, in place, which
        // made this the second writer of the `Idle` transition. It is [`to_idle`]
        // below now — the same pair [`reset`] runs, in the same order.
        let emptied = centre.slots.is_empty();
        (held.team, held.index, held.is_seated(), emptied)
    };
    // **16b cut F.1**: the seat goes with the slot.
    //
    // `EXIT` and `SIM LEAVE` were exact opposites and each kept half of what the
    // other gave back: `EXIT` released the seat and kept the slot, `SIM LEAVE`
    // dropped the slot and kept the seat — and a pilot who typed `SIM LEAVE` from
    // inside their pod (the centre counts a pod as its team room, so it is
    // accepted there) was left sitting in a pod holding a tank's billet with no
    // slot to give it back. The next pilot of that pod was then refused
    // `StationHeld`, and after the next reset moved the body out, `EXIT` itself
    // answered "You are not in a pod." Giving up the slot gives up the seat.
    if seated || crate::match_::cockpit::in_a_pod(ctx.w, actor) {
        crate::match_::cockpit::eject_pilot(ctx, actor);
    }
    // **16b cut Q.8 (fixer): the last leave is a return to `Idle`, and every
    // return to `Idle` puts the tanks back.**
    //
    // This is the third path to the line above and it was the only one that did
    // not run the tank reset: [`reset`] runs it (`@arena/reset`, and [`claim`]
    // on a terminal centre) and the boot pass runs it, but a lobby that simply
    // empties zeroed the centre in place and left whatever the tanks were. That
    // was invisible until `SIM UNIT`, and then it was not: a `SIM START` that
    // refits one slot and is refused by the next leaves a refitted tank standing
    // in `Selection`, and the pilots giving up on that lobby would have handed
    // the next pair a biped nobody asked for. It is [`reset_stock_tanks`]'s own
    // pass — idempotent, silent when nothing moved, `deployed` skipped — so on
    // every other leave in the project's life it writes nothing.
    //
    // [`reset_stock_tanks`]: crate::match_::reset_stock_tanks
    //
    // **And when it is not the last, the leaver's own pod is put back** (16b cut
    // Q.8, fixer). The whole-roster pass above would be wrong here: during
    // `Boarding` the other pilots' tanks have already been refitted to what they
    // chose, and a match is about to check them out. Only the slot that just went
    // is un-fitted, which is what makes the choice go with the pilot.
    if emptied {
        // 16b cut D.3 (`T-48`): the one writer of `Idle`, then the tank pass.
        to_idle(ctx);
        crate::match_::reset_stock_tanks(ctx, &std::collections::BTreeSet::new());
    } else {
        unfit_slot(ctx, team, index);
    }
    let name = display_name(ctx.w, actor);
    announce(
        ctx,
        format!("{name} gives up {} slot {index}.", team.as_str()),
    );
    // A leave during `Boarding` can be the last thing the trigger was waiting
    // for, or the thing that empties a side. Either way the answer is owed now
    // and not at the deadline.
    settle_boarding(ctx);
    Ok(())
}

// ---------------------------------------------------------------------------
// `SIM UNIT`: which machine the slot fields (16b cut Q.8)
// ---------------------------------------------------------------------------

/// The word the board spells one unit definition with: its **short** name — the
/// last segment of the canonical id, so `tacsim.unit.line_tank` is `line_tank` —
/// unless another loaded definition shares that segment, in which case there is
/// no short name to be had and the whole id is the word.
///
/// The collision is checked against every loaded unit definition and not only
/// the fieldable ones, so the word a slot's row prints cannot change when a
/// definition stops resolving as current.
fn short_name(id: &str) -> &str {
    id.rsplit('.').next().unwrap_or(id)
}

/// Whether `short_name(id)` is this world's name for exactly one loaded unit
/// definition.
fn short_name_is_unique(w: &World, id: &str) -> bool {
    let Some(store) = w.module_state::<crate::TacsimStore>() else {
        return true;
    };
    let short = short_name(id);
    store
        .definitions()
        .specs
        .keys()
        .filter(|other| short_name(other.as_str()).eq_ignore_ascii_case(short))
        .count()
        <= 1
}

/// How the board and every `SIM UNIT` sentence spell one definition id.
///
/// **16b cut D.3 (`T-60`)** made it `pub(crate)`: `STATUS`'s second line brackets
/// the fitted type when it is not the pod's authored one, and it has to spell it
/// the way the board does or a pilot would read two names for one chassis.
pub(crate) fn type_word(w: &World, id: &str) -> String {
    if short_name_is_unique(w, id) {
        short_name(id).to_owned()
    } else {
        id.to_owned()
    }
}

/// **Every unit type this world can field**, in the catalog's own canonical-id
/// order: the loaded unit definitions that resolve through
/// [`mush_tactics::UnitCatalog::get_current`] as *the* current definition of
/// their key.
///
/// That is the same resolution `@unit/create <obj>=<schema-id>` makes, which is
/// what makes "fieldable" mean "something the tactics crate will really
/// instantiate" rather than "something a pack mentioned". A definition that is
/// loaded but ambiguous across owners or versions is not offered, because
/// `refit_unit` could not act on the word either.
///
/// **Doc 18 cut IC2** (doc 18 §4.2 schema addition (1), D-18-29): nor is a unit
/// whose class no arena fields — the Dray, class `hauler`, is in the catalog for
/// the port code and never in a lobby, so `SIM UNIT` never lists it and
/// `SIM UNIT dray` answers the unknown-type line ([`resolve_unit_type`] reads
/// this list).
fn fieldable_ids(w: &World, reg: &mush_core::cmd::Registry) -> Vec<mush_core::CanonicalId> {
    let Some(definitions) = reg.definitions() else {
        return Vec::new();
    };
    let Some(store) = w.module_state::<crate::TacsimStore>() else {
        return Vec::new();
    };
    let catalog = store.definitions();
    catalog
        .specs
        .keys()
        .filter(|id| {
            catalog
                .units
                .get_current(id.as_str(), definitions)
                .is_some()
        })
        .filter(|id| {
            catalog
                .specs
                .get(*id)
                .is_some_and(|spec| spec.class_of().fieldable())
        })
        .cloned()
        .collect()
}

/// One typed word as a fieldable definition id, or `None`.
///
/// The whole canonical id first, then the short name — so a world where two
/// packs both end in `.line_tank` still answers to both full ids, and answers to
/// neither short one, which is what "list and accept the full id for those"
/// means.
fn resolve_unit_type(
    w: &World,
    reg: &mush_core::cmd::Registry,
    word: &str,
) -> Option<mush_core::CanonicalId> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let ids = fieldable_ids(w, reg);
    if let Some(exact) = ids.iter().find(|id| id.as_str().eq_ignore_ascii_case(word)) {
        return Some(exact.clone());
    }
    let mut matches = ids
        .iter()
        .filter(|id| short_name(id.as_str()).eq_ignore_ascii_case(word));
    let first = matches.next()?;
    matches.next().is_none().then(|| first.clone())
}

/// **The unit type one pod's tank is authored with** — the arena pack's own
/// `tanks[].unit` for side `team`'s slot `index`, resolved through the same slot
/// id [`crate::match_::tank_for_slot`] resolves the object through.
///
/// It is what `SIM UNIT` clears a choice back to, what the board means by
/// "Yours" on a slot that has chosen nothing, and what the `Idle` reset puts
/// back.
#[must_use]
pub fn authored_unit_type(w: &World, team: Team, index: u8) -> Option<SmolStr> {
    let arena = mush_core::CanonicalId::new(CENTRE_ARENA).ok()?;
    let slot = centre::slot_id(w, CENTRE_ARENA, team, index)?;
    let definitions = w.module_state::<crate::TacsimStore>()?.definitions();
    let def = definitions.arenas.get(&arena)?;
    def.tanks
        .iter()
        .find(|tank| tank.id.as_str() == slot.as_str())
        .map(|tank| SmolStr::new(tank.unit.as_str()))
}

/// **The class of one unit type** (16b cut AE.3), off the store's catalog:
/// [`crate::definitions::UnitSpec::class_of`] for the spec `id` names, or
/// `None` for an id the catalog does not carry.
fn class_of_type(w: &World, id: &str) -> Option<crate::definitions::UnitClass> {
    let key = mush_core::CanonicalId::new(id).ok()?;
    let store = w.module_state::<crate::TacsimStore>()?;
    store
        .definitions()
        .specs
        .get(&key)
        .map(crate::definitions::UnitSpec::class_of)
}

/// **Whether a side's machines carry a gun between them** (16b cut AE.3; plan
/// §6 AE-X3), or [`SimError::NoGun`] naming the side.
///
/// `types` is every unit type the side will fly, as definition ids: the
/// catalog spec each names is read for a weapon component
/// ([`crate::definitions::UnitSpec::weapons`]), which is how the check-out
/// itself reads a unit's weapons (`arena_def::weapon_summaries` off the spec
/// the unit's schema names), so a type the catalog does not carry reads as
/// carrying none, exactly as the arena would field it. An **empty** list
/// answers `Ok`: a side with no machine at all is `EmptySide`'s or `NoUnit`'s
/// refusal, not this one.
///
/// The one rule both callers use: [`ready`] over the final sides' fielded
/// tanks, where it binds, and [`start`] over the claimed slots' effective
/// types plus the pods the fill will add ([`fill_pods`]), as a courtesy before
/// anything is written.
fn side_fields_a_gun(w: &World, team: Team, types: &[SmolStr]) -> Result<(), SimError> {
    if types.is_empty() {
        return Ok(());
    }
    let Some(store) = w.module_state::<crate::TacsimStore>() else {
        return Ok(());
    };
    let catalog = store.definitions();
    let armed = types.iter().any(|id| {
        mush_core::CanonicalId::new(id.as_str())
            .ok()
            .and_then(|key| catalog.specs.get(&key))
            .is_some_and(|spec| spec.weapons().next().is_some())
    });
    if armed {
        Ok(())
    } else {
        Err(SimError::NoGun {
            team: team.as_str(),
        })
    }
}

/// **The unit type a tank is now fitted to**, off its M32 record's schema —
/// what `SIM START`'s refit left on it, which is what the check-out reads.
fn fitted_type(w: &World, tank: ObjId) -> Option<SmolStr> {
    mush_tactics::module::unit_of(w, tank).map(|unit| SmolStr::new(unit.schema().key().id.as_str()))
}

/// **The pods the AI fill adds to one side** (16b cut N.1, sized by Q.7;
/// factored by 16b cut AE.3 so [`ready`] and [`start`]'s courtesy cannot walk
/// it two ways): the lowest unclaimed pods of `team` whose tanks exist, in pod
/// order, until the side holds [`ai_side_size`] machines counting `fielded`
/// (the tanks already on it). A pod whose tank is already fielded is skipped.
/// The caller decides whether the fill runs at all (`[tacsim.match].ai_fill`,
/// and never on a range).
fn fill_pods(
    w: &World,
    centre: &Centre,
    match_cfg: &MatchConfig,
    team: Team,
    fielded: &[ObjId],
) -> Vec<(u8, ObjId)> {
    let fill_to = usize::from(ai_side_size(w, centre, match_cfg));
    let mut added: Vec<(u8, ObjId)> = Vec::new();
    for index in 1..=slots_per_side(w, match_cfg) {
        if fielded.len().saturating_add(added.len()) >= fill_to {
            break;
        }
        if centre
            .slots
            .iter()
            .any(|slot| slot.team == team && slot.index == index)
        {
            continue;
        }
        if let Some(tank) = centre::tank_for_slot(w, team, index)
            && !fielded.contains(&tank)
            && !added.iter().any(|(_, other)| *other == tank)
        {
            added.push((index, tank));
        }
    }
    added
}

/// **`SIM UNIT [<type>]`** — read or choose the unit type the caller's own slot
/// will field (16b cut Q.8).
///
/// **Choosing** is `Selection` only, and it acts on the **caller's** slot: there
/// is no arguing about somebody else's machine, and the choice is applied at
/// `SIM START` and not here, so a lobby that never starts a match writes nothing
/// to any tank.
///
/// **Listing is anybody's** (16b cut D.1). `None` prints `Fieldable units: …` to
/// anybody standing in the centre, in any state, slot or no slot, and appends
/// ` Yours: {type}.` only where the caller holds a slot. The catalog is the
/// reason to want a slot, so it is not gated behind holding one; the list is the
/// same one `Selection` printed, which is public.
///
/// Choosing the pod's **authored** type clears the choice rather than recording
/// it, so `SIM UNIT line_tank` and never having typed `SIM UNIT` at all are the
/// same state and the start does no work for either.
///
/// # Errors
///
/// [`SimError`], as the line the player sees. The bare list refuses only with
/// [`SimError::LayerOff`], [`SimError::OutsideCentre`],
/// [`SimError::UnknownAccount`] and [`SimError::Unavailable`]; a choice then adds
/// [`SimError::NoSlotToField`] for a caller holding no slot,
/// [`SimError::WrongState`] outside `Selection`,
/// [`SimError::NoSuchUnitType`] for a word the catalog does not carry, and
/// (16b cut AE.3; doc 16 §2 W2) [`SimError::ClassCap`] for a type whose class
/// the caller's own side already fields to `[tacsim.arena].max_units_by_class`
/// in its other claimed slots — `A side fields at most 1 drone; pick another
/// type.` at the shipped default, `This server fields no drones; pick another
/// type.` at a cap of zero. Re-choosing a type of the class the caller's slot
/// already holds is never refused: the count leaves the caller's own slot out.
/// `Arena::new` is the final word (`ArenaError::ClassCap`); this is the refusal
/// at the choice, where the pilot can still pick another type. **17b cut
/// M5.1**: then the slot's standing mind fitted to the new chassis — the
/// refusals of [`mind`]'s fit ([`SimError::OverClass`],
/// [`SimError::TooManyProfiles`], [`SimError::UnderFloor`],
/// [`SimError::ProfilePair`]) — and **17b cut M3.5** (doc 17 §9, D-UM-36)
/// [`SimError::Pool`], the centre's compute pool at the choice with the
/// arithmetic, the caller's hull at its own class: its mind's, else
/// `[tacsim.minds].fill_class` (17b cut M5.1; at the stock constants it
/// cannot refuse, `crate::match_::pool`'s head).
pub fn unit_type(ctx: &mut Ctx<'_>, choice: Option<&str>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let account = account_of(ctx.w, actor)?;
    let state = centre(ctx.w).ok_or(SimError::Unavailable)?.state;
    // **16b cut D.1: the bare list answers anybody in the centre, in any state,
    // slot or no slot.** The catalog is the reason to want a slot, so refusing to
    // print it until one is held is the wrong way round; and it is the same list
    // `SIM UNIT` prints in `Selection`, which is public. The caller's own choice
    // is appended only where there is a slot to have one.
    if choice.map(str::trim).is_none_or(str::is_empty) {
        let ids = fieldable_ids(ctx.w, ctx.reg);
        if ids.is_empty() {
            return Err(SimError::Unavailable);
        }
        let list: Vec<String> = ids.iter().map(|id| type_word(ctx.w, id.as_str())).collect();
        let held = centre(ctx.w)
            .and_then(|centre| centre.slot_of_account(account))
            .cloned();
        let mine = held
            .and_then(|slot| {
                slot.unit_type
                    .clone()
                    .or_else(|| authored_unit_type(ctx.w, slot.team, slot.index))
            })
            .map(|id| format!(" Yours: {}.", type_word(ctx.w, id.as_str())))
            .unwrap_or_default();
        // To the actor alone: a list is not news the room wants.
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("Fieldable units: {}.{mine}", list.join(", ")),
        );
        return Ok(());
    }
    // **The slot before the state, but only where the advice is true** (16b cut
    // Q.8, fixer: correctness Q8-5).
    //
    // [`SimError::NoSlotToField`] is [`SimError::NoSlot`] said as the thing to do
    // next — "claim a slot first: SIM TEAM red|blue" — and [`claim`] accepts
    // exactly three states: `Idle`, `Selection`, and a terminal centre, where the
    // claim clears the last match's board first. In `Boarding`, `Running` and
    // `Stopping` it is refused, so there that sentence advises a verb the caller
    // cannot type. In those three the state refusal is what is honest: for a
    // pilot who holds a slot, as it always was, and now for one who does not.
    if !matches!(state, MatchState::Idle | MatchState::Selection) && !state.is_terminal() {
        return Err(SimError::WrongState {
            state: state.as_str(),
        });
    }
    let held = centre(ctx.w)
        .and_then(|centre| centre.slot_of_account(account))
        .cloned()
        .ok_or(SimError::NoSlotToField)?;
    if state != MatchState::Selection {
        return Err(SimError::WrongState {
            state: state.as_str(),
        });
    }
    let authored = authored_unit_type(ctx.w, held.team, held.index);
    // The bare read has already answered above (16b cut D.1), so a word is what
    // is left.
    let word = choice.map(str::trim).unwrap_or_default();
    let chosen =
        resolve_unit_type(ctx.w, ctx.reg, word).ok_or_else(|| SimError::NoSuchUnitType {
            word: word.to_owned(),
        })?;
    // **16b cut AE.3** (doc 16 §2 W2): the class cap, at the choice, over the
    // caller's own side and nothing else — the other claimed slots of the side
    // by their **effective** type (the choice on the slot, else the pod's
    // authored type, which is what `fit_fielded_units` will field), so a refusal
    // tells the caller only what their own side's board already says.
    let class = class_of_type(ctx.w, chosen.as_str());
    let cap = class.and_then(|class| {
        configs(ctx.w).and_then(|(arena_cfg, _, _)| {
            arena_cfg
                .max_units_by_class
                .get(&class)
                .map(|cap| (class, *cap))
        })
    });
    if let Some((class, cap)) = cap {
        let others: Vec<(u8, Option<SmolStr>)> = centre(ctx.w)
            .map(|centre| {
                centre
                    .slots
                    .iter()
                    .filter(|slot| slot.team == held.team && slot.account != account)
                    .map(|slot| (slot.index, slot.unit_type.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let fielded = others
            .into_iter()
            .filter(|(index, unit_type)| {
                unit_type
                    .clone()
                    .or_else(|| authored_unit_type(ctx.w, held.team, *index))
                    .and_then(|id| class_of_type(ctx.w, id.as_str()))
                    == Some(class)
            })
            .count();
        if u32::try_from(fielded).unwrap_or(u32::MAX) >= cap {
            return Err(SimError::ClassCap { class, cap });
        }
    }
    // **17b cut M5.1** (doc 17 §12: *the later of `SIM UNIT` and `SIM MIND`
    // is the one refused*): the slot's standing mind, fitted to the NEW
    // chassis — over-class, the profile count at the class it would seal
    // at, the floor on an armed chassis — so the arithmetic printed below is
    // the new chassis's. A standing choice the pilot no longer holds is left
    // to the check-out (`SimError::MindNotHeld`) and loads `fill_class`
    // here, as an unchosen slot does.
    //
    // **17b cut M3.5** (doc 17 §9, D-UM-36): then the pool, at the choice —
    // the caller's hull at its OWN class (its mind's, else
    // `[tacsim.minds].fill_class`: 17b cut M5.1 counts a seated hull as
    // check-out admits it) against what the side's other hulls speak for
    // (`pool_at_slot`), under the pool of the most units this centre can
    // field. At the stock constants it cannot refuse; an operator's lowered
    // pool reaches it.
    if let Some(minds_cfg) = minds_config(ctx.w) {
        let class = match held.mind.as_ref() {
            Some(choice) => {
                let holdings = holdings::held_by(ctx.w, held.pilot);
                match holdings::resolve(&holdings, &choice.mind) {
                    Some(mind) => fit_on_chassis(ctx.w, chosen.as_str(), mind, &choice.profiles)?,
                    None => minds_cfg.fill_class,
                }
            }
            None => minds_cfg.fill_class,
        };
        pool_at_slot(ctx.w, held.team, held.index, class)?;
    }
    let spelling = type_word(ctx.w, chosen.as_str());
    let stored =
        (authored.as_deref() != Some(chosen.as_str())).then(|| SmolStr::new(chosen.as_str()));
    {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let centre = book.centre_mut();
        let Some(slot) = centre.slots.iter_mut().find(|slot| slot.account == account) else {
            return Err(SimError::NoSlotToField);
        };
        slot.unit_type = stored;
    }
    announce(
        ctx,
        format!(
            "Slot {} {} will field {spelling}.",
            held.team.as_str(),
            held.index
        ),
    );
    Ok(())
}

/// **The pool at the choice** (17b cut M3.5; doc 17 §9, D-UM-36) — what
/// [`unit_type`] and [`mind`] ask last ([`pool_at_slot`] gathers the loads):
/// the caller's hull's load at `mine` against what the side's other loads
/// speak for — its other claimed slots, each at its own class (17b cut M5.1:
/// its chosen mind's, else `[tacsim.minds].fill_class`, seated or not), AND,
/// since 17b cut M4.3, its unclaimed pods under the AI fill at the fill's
/// chosen class or `fill_class` — under the pool of `per_side` pods a side,
/// both sides (`centre::fieldable_per_side`: the most units the centre can
/// field, so the smallest pool any match of it runs under). **A choice that
/// adds nothing is never refused** (`would think 0` is never said): `None`
/// is admitted under any pool, which no lobby caller passes since 17b cut
/// M5.1; the arithmetic is the same one check-out's last gate does.
///
/// # Errors
///
/// [`SimError::Pool`] with the arithmetic.
fn pool_at_the_choice(
    cfg: &MindsConfig,
    per_side: u8,
    others: impl IntoIterator<Item = Option<crate::arena::core::CoreClass>>,
    mine: Option<crate::arena::core::CoreClass>,
) -> Result<(), SimError> {
    use crate::match_::pool::{admit, load_of, pool_for};
    let would = load_of(&cfg.classes, mine);
    if would == 0 {
        return Ok(());
    }
    let compute = pool_for(cfg, usize::from(per_side).saturating_mul(2));
    let spoken = others
        .into_iter()
        .map(|class| load_of(&cfg.classes, class))
        .fold(0_u64, u64::saturating_add);
    admit(compute, spoken, would).map_err(SimError::Pool)
}

// ---------------------------------------------------------------------------
// `SIM MIND` and `SIM AI MIND`: which mind a hull carries (17b cut M5.1)
// ---------------------------------------------------------------------------

/// **The bare `SIM MIND` line of a slot that chose nothing** (17b cut M5.1;
/// the owner's OQ-9 (i): an unchosen slot stays as built).
pub const MIND_UNCHOSEN: &str = "Yours: none; a tank you leave flies Standard Doctrine on Line.";

/// **`SIM MIND none`'s reply** (17b cut M5.1).
pub const MIND_CLEARED: &str =
    "Your slot carries no mind of your own; a tank you leave flies Standard Doctrine on Line.";

/// **Everybody else's `SIM AI MIND` from `SIM START` on** (17b cut M5.1; the
/// design note's §2.4): it names nothing and marks nothing.
pub const FILLS_HIDDEN: &str = "The fills are not shown once a match starts.";

/// **What one `SIM MIND` line (or `SIM AI MIND`'s, after its side word) asks
/// for** (17b cut M5.1; D8), read by [`MindRequest::parse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MindRequest {
    /// The bare word: a read, to the actor alone.
    Read,
    /// `none`: the choice cleared.
    Clear,
    /// `<mind> [@<rev>] [PROFILE <p>[, <p>]…]`.
    Choose {
        /// The mind's words, as typed.
        mind: String,
        /// Whether an `@<rev>` pin was written (refused: no revisions are
        /// kept before 17b cut M6).
        revision: bool,
        /// The profiles' words, as typed, in order; empty is Line alone.
        profiles: Vec<String>,
    },
}

/// **Why a mind line is not one** — the usage line `crate::match_::sim` says
/// for it (17b cut M5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MindUsage {
    /// No mind before `PROFILE`.
    Mind,
    /// `PROFILE` with no profile after it.
    Profiles,
}

impl MindRequest {
    /// **Read one line** (17b cut M5.1; D8): empty is [`MindRequest::Read`],
    /// `none` (case-folded) [`MindRequest::Clear`], anything else a choice —
    /// the mind's words up to an `@` or the word `PROFILE` (case-folded), and
    /// after `PROFILE` the profiles, comma-separated.
    ///
    /// # Errors
    ///
    /// [`MindUsage`] when the words are not a choice: no mind before
    /// `PROFILE`, or nothing after it.
    pub fn parse(rest: &str) -> Result<MindRequest, MindUsage> {
        let rest = rest.trim();
        if rest.is_empty() {
            return Ok(MindRequest::Read);
        }
        if rest.eq_ignore_ascii_case("none") {
            return Ok(MindRequest::Clear);
        }
        let (head, tail) = match split_profile(rest) {
            Some((head, tail)) => (head, Some(tail)),
            None => (rest, None),
        };
        let (mind, revision) = match head.split_once('@') {
            Some((mind, _)) => (mind.trim(), true),
            None => (head.trim(), false),
        };
        if mind.is_empty() {
            return Err(MindUsage::Mind);
        }
        let profiles = match tail {
            None => Vec::new(),
            Some(tail) => {
                let words: Vec<String> = tail
                    .split(',')
                    .map(str::trim)
                    .filter(|word| !word.is_empty())
                    .map(str::to_owned)
                    .collect();
                if words.is_empty() {
                    return Err(MindUsage::Profiles);
                }
                words
            }
        };
        Ok(MindRequest::Choose {
            mind: mind.to_owned(),
            revision,
            profiles,
        })
    }
}

/// **`rest` split at the word `PROFILE`**, case-folded and standing alone
/// between whitespace: the words before it and the words after it.
fn split_profile(rest: &str) -> Option<(&str, &str)> {
    let mut start: Option<usize> = None;
    for (at, ch) in rest
        .char_indices()
        .chain(std::iter::once((rest.len(), ' ')))
    {
        if ch.is_whitespace() {
            if let Some(from) = start.take()
                && rest.get(from..at)?.eq_ignore_ascii_case("profile")
            {
                return Some((rest.get(..from)?, rest.get(at..)?));
            }
        } else if start.is_none() {
            start = Some(at);
        }
    }
    None
}

/// **What one `SIM AI MIND` line asks for** (17b cut M5.1): an optional side
/// word — staff name the side they set, a lead's side is their own — then a
/// [`MindRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FillRequest {
    /// `red` or `blue`, when the line names one.
    pub side: Option<Team>,
    /// The rest.
    pub request: MindRequest,
}

impl FillRequest {
    /// **Read one line**: a leading `red` or `blue` (case-folded) is the
    /// side, and the rest is [`MindRequest::parse`]'s — so a side word alone
    /// is a bare read, never a mind called `red` (17b cut M5.1's review).
    ///
    /// # Errors
    ///
    /// [`MindRequest::parse`]'s.
    pub fn parse(rest: &str) -> Result<FillRequest, MindUsage> {
        let rest = rest.trim();
        let (head, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let side = match head.to_ascii_lowercase().as_str() {
            "red" => Some(Team::Red),
            "blue" => Some(Team::Blue),
            _ => None,
        };
        if let Some(side) = side {
            return Ok(FillRequest {
                side: Some(side),
                request: MindRequest::parse(tail)?,
            });
        }
        Ok(FillRequest {
            side: None,
            request: MindRequest::parse(rest)?,
        })
    }
}

/// **The core one unit type is built with, read without an instance** (17b
/// cut M5.1; R-9, D-UM-37: the pod emulates the chassis's content-declared
/// core and never reads owned IC hardware): the spec's
/// [`crate::definitions::unit::ComponentRole::Core`] and the store's
/// [`crate::definitions::core::CoreDef`] — `(class, governed)` — or `None` for
/// a type with no core (or one the catalog does not carry), which flies the
/// ROM. [`class_of_type`]'s neighbour.
pub(crate) fn core_of_type(w: &World, id: &str) -> Option<(CoreClass, bool)> {
    let key = mush_core::CanonicalId::new(id).ok()?;
    let catalog = w.module_state::<crate::TacsimStore>()?.definitions();
    let core = catalog.specs.get(&key)?.cores().next()?;
    catalog.cores.get(core).map(|def| (def.class, def.governed))
}

/// **Whether one unit type carries a weapon** (17b cut M5.1; C-23: the odds
/// floor binds only a hull with a weapons station) — the spec's weapon
/// components, read as [`side_fields_a_gun`] reads them.
pub(crate) fn armed_of_type(w: &World, id: &str) -> bool {
    let Ok(key) = mush_core::CanonicalId::new(id) else {
        return false;
    };
    w.module_state::<crate::TacsimStore>()
        .and_then(|store| store.definitions().specs.get(&key).cloned())
        .is_some_and(|spec| spec.weapons().next().is_some())
}

/// **The unit type one slot fields**: its choice, else its pod's authored
/// type.
fn effective_type(w: &World, slot: &Slot) -> Option<SmolStr> {
    slot.unit_type
        .clone()
        .or_else(|| authored_unit_type(w, slot.team, slot.index))
}

/// **The class `held` would be sealed at on `chassis`** — the seal's own rule
/// (`crate::match_::seal::seal_for`): `min(rated, core, fill_class)`, and
/// Class 0 on a chassis with no core.
fn sealed_class(
    w: &World,
    held: &HeldMind,
    chassis: Option<&str>,
    fill_class: CoreClass,
) -> CoreClass {
    match chassis.and_then(|id| core_of_type(w, id)) {
        Some((core, _)) => held.rated.min(core).min(fill_class),
        None => CoreClass::Zero,
    }
}

/// **One profile's card, held to the server** (17b cut M5.1): the card's
/// survival pair on every chassis, and on an `armed` one the Compact floor —
/// the seal's own questions (`crate::match_::seal::seal_for`,
/// [`crate::match_::seal::odds_are_lawful`]), refused, never clamped
/// (D-UM-17). A profile that is not a stock preset has no stock card and
/// nothing to hold.
fn profile_lawful(
    profile: &str,
    reflex: &crate::config::ReflexConfig,
    floor_permille: u32,
    armed: bool,
) -> Result<(), SimError> {
    let Some(named) = crate::arena::card::PRESETS
        .iter()
        .find(|preset| preset.eq_ignore_ascii_case(profile.trim()))
        .copied()
    else {
        return Ok(());
    };
    let Some(card) = crate::arena::card::preset(named, reflex) else {
        return Ok(());
    };
    if let Err(crate::arena::card::CardError::Pair {
        fight_home,
        break_off,
        ..
    }) = card.pair(named)
    {
        return Err(SimError::ProfilePair {
            profile: named.to_owned(),
            fight_home,
            break_off,
        });
    }
    if armed && !crate::match_::seal::odds_are_lawful(&card, floor_permille) {
        return Err(SimError::UnderFloor {
            profile: named.to_owned(),
            odds_permille: u32::from(card.min_odds_permille),
            floor_permille,
        });
    }
    Ok(())
}

/// **The fit at the choice, but the pool** (17b cut M5.1; doc 17 §9, §12;
/// the design note's M5.1 item 4): whether `held`, sealed with `profiles`,
/// may fly `chassis` — over-class (D-UM-41, C-24: a governed core below the
/// mind's rating; a chassis with no core never is, nor, since 17b cut M5.2's
/// S1 ruling, one whose governed core is Class 0, which flies the ROM — the
/// seal's own predicate, `crate::match_::seal::seal_for`), the profile count at the
/// class it would seal at (OQ-3), and each profile's card ([`profile_lawful`],
/// the floor on an armed chassis only, C-23) — answering that class.
/// [`fit_mind`] adds the pool; `SIM AI MIND` asks this alone for each pod its
/// side's fill may take.
fn fit_on_chassis(
    w: &World,
    chassis: &str,
    held: &HeldMind,
    profiles: &[SmolStr],
) -> Result<CoreClass, SimError> {
    let minds_cfg = minds_config(w).unwrap_or_default();
    let reflex = configs(w)
        .map(|(arena_cfg, _, _)| arena_cfg.reflex)
        .unwrap_or_default();
    let core = core_of_type(w, chassis);
    if let Some((core, true)) = core
        && held.rated > core
        && core > CoreClass::Zero
    {
        return Err(SimError::OverClass {
            mind: held.name().to_owned(),
            rated: held.rated,
            chassis: type_word(w, chassis),
            core,
        });
    }
    let class = sealed_class(w, held, Some(chassis), minds_cfg.fill_class);
    let most = minds_cfg.classes.row(class).profiles;
    if profiles.len() > usize::from(most) {
        return Err(SimError::TooManyProfiles {
            mind: held.name().to_owned(),
            class,
            most,
            rom: core.is_none().then(|| type_word(w, chassis)),
        });
    }
    let armed = armed_of_type(w, chassis);
    if profiles.is_empty() {
        profile_lawful("Line", &reflex, minds_cfg.odds_floor_permille, armed)?;
    }
    for profile in profiles {
        profile_lawful(profile, &reflex, minds_cfg.odds_floor_permille, armed)?;
    }
    Ok(class)
}

/// **The fit at the choice** (17b cut M5.1; the design note's M5.1 item 4):
/// [`fit_on_chassis`], then the pool for side `team`'s slot `index` at the
/// class it answered ([`pool_at_slot`]) — so `SIM MIND` and `SIM UNIT` ask
/// the same questions and "the later … is the one refused" (doc 17 §12).
fn fit_mind(
    w: &World,
    team: Team,
    index: u8,
    chassis: &str,
    held: &HeldMind,
    profiles: &[SmolStr],
) -> Result<CoreClass, SimError> {
    let class = fit_on_chassis(w, chassis, held, profiles)?;
    pool_at_slot(w, team, index, class)?;
    Ok(class)
}

/// **The presets of `held` that `chassis` may seal** (17b cut M5.1) — the
/// bare `SIM MIND`'s `Fits` line: how many it may seal there (the class
/// row's `profiles` at the class it would seal at, and never more than the
/// presets listed), and the presets [`fit_on_chassis`] takes there one at a
/// time — the choice's own questions, so the line and the refusal cannot
/// drift (17b cut M5.1's review); `None` when it is over-class there or no
/// preset is lawful.
fn fits(w: &World, chassis: &str, held: &HeldMind) -> Option<(u8, Vec<SmolStr>)> {
    let mut class = None;
    let lawful: Vec<SmolStr> = held
        .presets
        .iter()
        .filter(|preset| {
            fit_on_chassis(w, chassis, held, std::slice::from_ref(*preset))
                .map(|sealed| class = Some(sealed))
                .is_ok()
        })
        .cloned()
        .collect();
    let most = minds_config(w)
        .unwrap_or_default()
        .classes
        .row(class?)
        .profiles;
    let listed = u8::try_from(lawful.len()).unwrap_or(u8::MAX);
    Some((most.min(listed), lawful))
}

/// **The class one claimed slot's hull is admitted at** (17b cut M5.1): the
/// class its chosen mind would seal at on its chassis, else `fill_class` —
/// seated or not, the class check-out admits it at
/// (`crate::match_::pool::admitted_class`). A choice the pilot no longer holds
/// is left to the check-out and counts as unchosen.
fn slot_class(w: &World, slot: &Slot, fill_class: CoreClass) -> CoreClass {
    slot.mind
        .as_ref()
        .and_then(|choice| {
            let held = holdings::held_by(w, slot.pilot);
            let mind = holdings::resolve(&held, &choice.mind)?;
            Some(sealed_class(
                w,
                mind,
                effective_type(w, slot).as_deref(),
                fill_class,
            ))
        })
        .unwrap_or(fill_class)
}

/// **The mind a side's fill choice names, resolved** (17b cut M5.1): against
/// the holdings of the lead who set it (their own slot's pilot, F11), or
/// Standard Doctrine for staff (D9). `None` when it no longer resolves.
fn fill_held(w: &World, centre: &Centre, fill: &FillChoice) -> Option<HeldMind> {
    match fill.by {
        FillSetter::Staff => {
            let doctrine = HeldMind::doctrine();
            holdings::names(&doctrine, &fill.choice.mind).then_some(doctrine)
        }
        FillSetter::Lead(account) => {
            let pilot = centre
                .slots
                .iter()
                .find(|slot| slot.team == fill.team && slot.account == account)?
                .pilot;
            holdings::resolve(&holdings::held_by(w, pilot), &fill.choice.mind).cloned()
        }
    }
}

/// **What a side's fill is sealed with** (17b cut M5.1's resolution, and
/// 17b cut M5.2's Q-UM-15): the mind `held` resolved from `fill` at its one
/// profile — or, under [`Centre::doctrine_only`] with `held` not Standard
/// Doctrine, Standard Doctrine **at the lead's chosen profile** when it is
/// one of Doctrine's six presets, else Line (doc 17:1488; the design note's
/// C-7), `substituted`.
fn fill_seal_choice(held: &HeldMind, fill: &FillChoice, doctrine_only: bool) -> SealChoice {
    if doctrine_only && !held.is_doctrine() {
        let doctrine = HeldMind::doctrine();
        let profiles = fill
            .choice
            .profiles
            .first()
            .and_then(|name| doctrine.preset(name))
            .filter(|name| name != "Line")
            .into_iter()
            .collect();
        return SealChoice {
            held: doctrine,
            profiles,
            chose: true,
            substituted: true,
        };
    }
    SealChoice {
        held: held.clone(),
        profiles: fill.choice.profiles.clone(),
        chose: true,
        substituted: false,
    }
}

/// **The pilots Q-UM-15's sentence is owed to at `SIM START`** (17b cut
/// M5.2; the design note's item 11): under [`Centre::doctrine_only`], each
/// claimed slot's pilot whose own choice resolves to a mind that is not
/// Standard Doctrine, and each lead whose fill choice does (when the fill
/// fields a pod: `[tacsim.match].ai_fill` on, no range, and [`fill_pods`]
/// finds one) — the hulls [`ready`] will substitute — once each, in claim
/// order. Empty when the rule is off, and on every production server before
/// 17b cut M6 (production holds Standard Doctrine alone).
fn substituted_pilots(w: &World, centre: &Centre, match_cfg: &MatchConfig) -> Vec<ObjId> {
    let mut pilots: Vec<ObjId> = Vec::new();
    if !centre.doctrine_only {
        return pilots;
    }
    for slot in &centre.slots {
        let Some(choice) = slot.mind.as_ref() else {
            continue;
        };
        if holdings::resolve(&holdings::held_by(w, slot.pilot), &choice.mind)
            .is_some_and(|held| !held.is_doctrine())
            && !pilots.contains(&slot.pilot)
        {
            pilots.push(slot.pilot);
        }
    }
    if match_cfg.ai_fill && centre.mode != MatchMode::Range {
        for fill in &centre.fill_minds {
            let FillSetter::Lead(account) = fill.by else {
                continue;
            };
            let Some(lead) = centre
                .slots
                .iter()
                .find(|slot| slot.team == fill.team && slot.account == account)
            else {
                continue;
            };
            let fielded: Vec<ObjId> = centre
                .slots
                .iter()
                .filter(|slot| slot.team == fill.team)
                .filter_map(|slot| centre::tank_for_slot(w, fill.team, slot.index))
                .collect();
            if fill_held(w, centre, fill).is_some_and(|held| !held.is_doctrine())
                && !fill_pods(w, centre, match_cfg, fill.team, &fielded).is_empty()
                && !pilots.contains(&lead.pilot)
            {
                pilots.push(lead.pilot);
            }
        }
    }
    pilots
}

/// **Q-UM-15's sentence** (17b cut M5.2; doc 17:1490-1491; the owner's
/// OQ-5): said at `SIM START` to each pilot whose own choice (or, for a
/// lead, whose fill's) is substituted, to that pilot alone — never in a
/// production match of 17b cut M5. 80 columns.
pub const HUMANS_ON_THE_OTHER_SIDE: &str =
    "Humans on the other side: your fill and any released hull fly Standard Doctrine.";

/// **Every class side `team` would field, but slot `except`'s** (17b cut
/// M5.1): its claimed slots at their own class ([`slot_class`]) and, under
/// the AI fill and no range, the pods the fill would add ([`fill_pods`]) at
/// the fill's chosen class on each pod's chassis, else `fill_class`.
fn side_classes(
    w: &World,
    centre: &Centre,
    team: Team,
    except: Option<u8>,
    fill_class: CoreClass,
) -> Vec<CoreClass> {
    let mut classes: Vec<CoreClass> = centre
        .slots
        .iter()
        .filter(|slot| slot.team == team && Some(slot.index) != except)
        .map(|slot| slot_class(w, slot, fill_class))
        .collect();
    if let Some((_, match_cfg, _)) = configs(w)
        && match_cfg.ai_fill
        && centre.mode != MatchMode::Range
    {
        let fielded: Vec<ObjId> = centre
            .slots
            .iter()
            .filter(|slot| slot.team == team)
            .filter_map(|slot| centre::tank_for_slot(w, team, slot.index))
            .collect();
        let held = centre
            .fill_minds
            .iter()
            .find(|fill| fill.team == team)
            .and_then(|fill| fill_held(w, centre, fill));
        for (index, _) in fill_pods(w, centre, &match_cfg, team, &fielded) {
            classes.push(match &held {
                Some(held) => sealed_class(
                    w,
                    held,
                    authored_unit_type(w, team, index).as_deref(),
                    fill_class,
                ),
                None => fill_class,
            });
        }
    }
    classes
}

/// **The pool at the choice for one slot** (17b cut M5.1; doc 17 §9,
/// D-UM-36): side `team`'s slot `index` at `class` against every other hull
/// of the side ([`side_classes`]), through [`pool_at_the_choice`]. Nothing to
/// refuse on a world with no `[tacsim.minds]` or no centre. **A change that
/// lowers the slot's load is never refused** (17b cut M5.1's review, D-R2): a
/// side that an unchecked claim or `SIM AI <n>` has put past its half may
/// still move down — a mind chosen on a chassis with no core (400 for 1,500)
/// — while a change that keeps or raises the load (`SIM MIND none` back to
/// `fill_class` included) is asked of the side after it, so the later of `SIM
/// UNIT` and `SIM MIND` stays the one refused (doc 17 §12).
fn pool_at_slot(w: &World, team: Team, index: u8, class: CoreClass) -> Result<(), SimError> {
    use crate::match_::pool::load_of;
    let (Some(minds_cfg), Some(centre)) = (minds_config(w), centre(w)) else {
        return Ok(());
    };
    if let Some(slot) = centre
        .slots
        .iter()
        .find(|slot| slot.team == team && slot.index == index)
        && load_of(&minds_cfg.classes, Some(class))
            < load_of(
                &minds_cfg.classes,
                Some(slot_class(w, slot, minds_cfg.fill_class)),
            )
    {
        return Ok(());
    }
    let others = side_classes(w, centre, team, Some(index), minds_cfg.fill_class);
    pool_at_the_choice(
        &minds_cfg,
        centre::fieldable_per_side(w),
        others.into_iter().map(Some),
        Some(class),
    )
}

/// **The profiles of one choice, resolved against `held`** (17b cut M5.1;
/// OQ-3): each a preset it carries, case-folded, in its canonical spelling,
/// none twice; a list that is Line alone is stored empty.
fn resolve_profiles(held: &HeldMind, words: &[String]) -> Result<Vec<SmolStr>, SimError> {
    let mut profiles: Vec<SmolStr> = Vec::with_capacity(words.len());
    for word in words {
        let profile = held.preset(word).ok_or_else(|| SimError::NoSuchProfile {
            mind: held.name().to_owned(),
            word: word.clone(),
            presets: listed(&held.presets, "or"),
        })?;
        if profiles.contains(&profile) {
            return Err(SimError::ProfileTwice {
                profile: profile.to_string(),
            });
        }
        profiles.push(profile);
    }
    if let [only] = profiles.as_slice()
        && only == "Line"
    {
        profiles.clear();
    }
    Ok(profiles)
}

/// **A bare `SIM MIND`'s `Fits` line** (17b cut M5.1): the chassis and its
/// core's class, the mind, and how many of which presets it may seal there —
/// `Fits line_tank (II): Standard Doctrine, up to two of Line, Sentry, Sniper,
/// Skirmisher, Hunter or Rearguard.` A chassis with no core (`core` `None`)
/// flies the ROM (C-24), and its bracket says so — `Fits target_mk1 (ROM):
/// …` — rather than naming a core class it does not have (17b cut M5.1's
/// review; `(ROM)` is no wider than `(III)`).
#[must_use]
pub fn fits_line(
    chassis: &str,
    core: Option<CoreClass>,
    mind: &str,
    most: u8,
    presets: &[SmolStr],
) -> String {
    format!(
        "Fits {chassis} ({}): {mind}, up to {} of {}.",
        core.map_or("ROM", CoreClass::as_str),
        count_word(usize::from(most)),
        listed(presets, "or")
    )
}

/// **A bare `SIM MIND`'s pool line** (17b cut M5.1; doc 17 §9, C-8: the pool
/// "visible in the fiction") — `Red's half of the centre's pool is 244,000
/// ops a second and 3,000 are spoken for.`
#[must_use]
pub fn pool_line(team: Team, half: u64, spoken: u64) -> String {
    format!(
        "{}'s half of the centre's pool is {} ops a second and {} are spoken for.",
        side_word(team),
        crate::match_::pool::thousands(half),
        crate::match_::pool::thousands(spoken)
    )
}

/// **A holding list as a line says it** (17b cut M5.1): one mind with its
/// presets counted — `Standard Doctrine (six presets)`, production's every
/// character — and several by their names alone, joined with `and` (`Standard
/// Doctrine, Fixture I and Load III`), so the line stays under 120 columns
/// where a test provider holds three.
#[must_use]
pub fn holdings_words(held: &[HeldMind]) -> String {
    match held {
        [] => "no mind".to_owned(),
        [mind] => {
            let n = mind.presets.len();
            format!(
                "{} ({} preset{})",
                mind.name(),
                count_word(n),
                if n == 1 { "" } else { "s" }
            )
        }
        several => {
            let names: Vec<SmolStr> = several
                .iter()
                .map(|mind| mind.identity.mind.clone())
                .collect();
            listed(&names, "and")
        }
    }
}

/// **The bare `SIM MIND`'s lines** (17b cut M5.1; the design note's §2.4),
/// always the reader's own: in `Selection` their slot's choice, one `Fits`
/// line per held mind that fits the slot's chassis and the side's pool; from
/// `SIM START` on their choice and that selection is closed; and with no
/// slot what they hold.
fn mind_read_lines(w: &World, actor: ObjId, state: MatchState, slot: Option<&Slot>) -> Vec<String> {
    let character = slot.map_or(actor, |slot| slot.pilot);
    let held = holdings::held_by(w, character);
    let closed = matches!(
        state,
        MatchState::Boarding | MatchState::Running | MatchState::Stopping
    );
    let Some(slot) = slot else {
        let what = holdings_words(&held);
        return vec![if closed {
            format!("You hold {what}; selection is closed.")
        } else {
            format!("You hold {what}; claim a slot and SIM MIND lists what fits its core.")
        }];
    };
    if state != MatchState::Selection {
        return vec![match &slot.mind {
            Some(choice) => format!("Yours: {}; selection is closed.", choice.words()),
            None => "Yours: none; selection is closed.".to_owned(),
        }];
    }
    let mut lines = vec![match &slot.mind {
        Some(choice) => format!("Yours: {}.", choice.words()),
        None => MIND_UNCHOSEN.to_owned(),
    }];
    if let Some(chassis) = effective_type(w, slot) {
        let core = core_of_type(w, &chassis).map(|(class, _)| class);
        for mind in &held {
            if let Some((most, presets)) = fits(w, &chassis, mind) {
                lines.push(fits_line(
                    &type_word(w, &chassis),
                    core,
                    mind.name(),
                    most,
                    &presets,
                ));
            }
        }
    }
    if let (Some(minds_cfg), Some(centre)) = (minds_config(w), centre(w)) {
        use crate::match_::pool::{load_of, pool_for};
        let spoken = side_classes(w, centre, slot.team, None, minds_cfg.fill_class)
            .into_iter()
            .map(|class| load_of(&minds_cfg.classes, Some(class)))
            .fold(0_u64, u64::saturating_add);
        let units = usize::from(centre::fieldable_per_side(w)).saturating_mul(2);
        let half = pool_for(&minds_cfg, units) / 2;
        lines.push(pool_line(slot.team, half, spoken));
    }
    lines
}

/// **`SIM MIND [<mind> [PROFILE <p>[, <p>]…] | none]`** — read or choose the
/// mind the caller's own slot carries (17b cut M5.1; doc 17 §12, D-UM-49;
/// the owner's OQ-2, OQ-3; D8).
///
/// **The bare read** answers in any state, to the actor alone, about their
/// own slot (the design note's §2.4): in `Selection` their choice (or
/// [`MIND_UNCHOSEN`]), one `Fits` line per held mind that fits the slot's
/// chassis ([`fits_line`]) and the side's pool ([`pool_line`]); from `SIM
/// START` on their choice and `selection is closed`; with no slot what they
/// hold.
///
/// **A choice** is `Selection` only and the caller's own slot, in `SIM
/// UNIT`'s order: [`SimError::WrongState`], then [`SimError::NoSlotToField`]
/// — no new variant for either. Then the mind: an `@<rev>` pin is
/// [`SimError::NoRevisions`], a mind the character does not hold
/// [`SimError::NoSuchMind`] (posting nothing), a profile it does not carry
/// [`SimError::NoSuchProfile`], one named twice [`SimError::ProfileTwice`];
/// then the fit (`fit_mind`): [`SimError::OverClass`],
/// [`SimError::TooManyProfiles`], [`SimError::ProfilePair`],
/// [`SimError::UnderFloor`] (an armed chassis only) and [`SimError::Pool`].
/// `none` clears the choice, and is held to the pool too: the default loads
/// `fill_class`, which may be more than a chosen mind on a chassis with no
/// core.
///
/// **Every reply is the actor's alone** (`sys_to_actor`): nothing about a
/// choice is announced (doc 17 §12's fog; `SIM UNIT`'s announce is what this
/// must not copy). The choice is recorded on [`Slot::mind`], and the
/// check-out seals it (17b cut M5.2).
///
/// # Errors
///
/// [`SimError`], as the line the player sees.
pub fn mind(ctx: &mut Ctx<'_>, request: MindRequest) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let account = account_of(ctx.w, actor)?;
    let state = centre(ctx.w).ok_or(SimError::Unavailable)?.state;
    let slot = centre(ctx.w)
        .and_then(|centre| centre.slot_of_account(account))
        .cloned();
    let chosen = match request {
        MindRequest::Read => {
            for line in mind_read_lines(ctx.w, actor, state, slot.as_ref()) {
                ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
            }
            return Ok(());
        }
        MindRequest::Clear => None,
        MindRequest::Choose {
            mind,
            revision,
            profiles,
        } => Some((mind, revision, profiles)),
    };
    // `SIM UNIT`'s order: the state where the slot advice is not true, the
    // slot, then `Selection`.
    if !matches!(state, MatchState::Idle | MatchState::Selection) && !state.is_terminal() {
        return Err(SimError::WrongState {
            state: state.as_str(),
        });
    }
    let slot = slot.ok_or(SimError::NoSlotToField)?;
    if state != MatchState::Selection {
        return Err(SimError::WrongState {
            state: state.as_str(),
        });
    }
    let (stored, reply) = match chosen {
        None => {
            let fill_class = minds_config(ctx.w).map_or(CoreClass::I, |cfg| cfg.fill_class);
            pool_at_slot(ctx.w, slot.team, slot.index, fill_class)?;
            (None, MIND_CLEARED.to_owned())
        }
        Some((word, revision, profiles)) => {
            if revision {
                return Err(SimError::NoRevisions);
            }
            let held = holdings::held_by(ctx.w, slot.pilot);
            let mind = holdings::resolve(&held, &word).ok_or_else(|| SimError::NoSuchMind {
                word: word.trim().to_owned(),
            })?;
            let profiles = resolve_profiles(mind, &profiles)?;
            let chassis = effective_type(ctx.w, &slot).ok_or(SimError::NoUnit {
                team: slot.team.as_str(),
                index: slot.index,
            })?;
            fit_mind(ctx.w, slot.team, slot.index, &chassis, mind, &profiles)?;
            let choice = MindChoice {
                mind: mind.identity.mind.clone(),
                profiles,
            };
            let reply = format!("Your slot will carry {}.", choice.words());
            (Some(choice), reply)
        }
    };
    {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let Some(held) = book
            .centre_mut()
            .slots
            .iter_mut()
            .find(|held| held.account == account)
        else {
            return Err(SimError::NoSlotToField);
        };
        held.mind = stored;
    }
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, reply);
    Ok(())
}

/// **The fill read for one side, privileged** — the setter's, the lead's
/// and staff's line.
fn fill_line(team: Team, fill: Option<&FillChoice>) -> String {
    match fill {
        Some(fill) => format!("{}'s fill: {}.", side_word(team), fill.choice.words()),
        None => format!(
            "{}'s fill: nothing chosen; it flies Standard Doctrine on Line.",
            side_word(team)
        ),
    }
}

/// **The bare `SIM AI MIND`'s lines** (17b cut M5.1; the design note's
/// §2.4): before `SIM START`, one a side — the side's lead and staff read the
/// choice ([`fill_line`]), everyone else whether one was made; from `SIM
/// START` on, the account that set a fill reads its own and staff read both,
/// and everyone else reads [`FILLS_HIDDEN`] alone. `only` (`SIM AI MIND red`)
/// keeps that side's line alone.
fn fill_read_lines(
    centre: &Centre,
    account: Option<AccountId>,
    staff: bool,
    only: Option<Team>,
) -> Vec<String> {
    let fill_of = |team: Team| centre.fill_minds.iter().find(|fill| fill.team == team);
    let teams = Team::ALL
        .into_iter()
        .filter(|team| only.is_none_or(|only| only == *team));
    if matches!(centre.state, MatchState::Idle | MatchState::Selection) {
        return teams
            .map(|team| {
                let lead = account.is_some_and(|account| centre.leads_side(team, account));
                match (fill_of(team), staff || lead) {
                    (fill, true) => fill_line(team, fill),
                    (Some(_), false) => format!("{}'s fill carries a mind.", side_word(team)),
                    (None, false) => format!("{}'s fill: nothing chosen.", side_word(team)),
                }
            })
            .collect();
    }
    let lines: Vec<String> = teams
        .filter_map(|team| {
            let fill = fill_of(team);
            let setter = fill.is_some_and(|fill| {
                account.is_some_and(|account| fill.by == FillSetter::Lead(account))
            });
            (staff || setter).then(|| fill_line(team, fill))
        })
        .collect();
    if lines.is_empty() {
        vec![FILLS_HIDDEN.to_owned()]
    } else {
        lines
    }
}

/// **`SIM AI MIND [[red|blue] <mind> [PROFILE <p>] | none]`** — read or
/// choose the mind a side's AI fill carries (17b cut M5.1; doc 17 §12, §19;
/// D9, D-UM-49).
///
/// **In `SIM AI`'s order** (the design note's §2.4): every form is
/// [`SimError::AiOff`] on a server whose fill is off; on a range the bare read
/// is `AI fill: none on a range.` and a set [`SimError::AiOnRange`]. The bare
/// read then answers anybody in the centre (`fill_read_lines`). A set is
/// `Selection` only ([`SimError::WrongState`]) and a side's lead's, for their
/// own side, from their own holdings, or staff's, for the side they name,
/// Standard Doctrine's presets only ([`SimError::StaffHoldNoMinds`],
/// [`SimError::FillWhichSide`]); anyone else is [`SimError::NotLeadForAi`].
/// The fill flies **one** profile ([`SimError::FillOneProfile`]) — nobody
/// aboard switches it — and the choice is fitted, but for the pool, to every
/// pod of the side the fill may take (`fit_on_chassis`). `none` clears it.
///
/// It does not re-run the pool: the check-out's `StartError::Pool` is the
/// backstop, and no fill choice can refuse at the stock constants (the
/// design note's M5.1 item 4). Every reply is the actor's alone; nothing is
/// announced.
///
/// # Errors
///
/// [`SimError`], as the line the player sees.
pub fn ai_mind(ctx: &mut Ctx<'_>, request: FillRequest) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let staff = is_staff(ctx.w, actor);
    let account = match account_of(ctx.w, actor) {
        Ok(account) => Some(account),
        Err(_) if staff => None,
        Err(error) => return Err(error),
    };
    let (_, match_cfg, _) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    if !match_cfg.ai_fill {
        return Err(SimError::AiOff);
    }
    if centre(ctx.w).is_some_and(|centre| centre.mode == MatchMode::Range) {
        if request.request != MindRequest::Read {
            return Err(SimError::AiOnRange);
        }
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            "AI fill: none on a range.".to_owned(),
        );
        return Ok(());
    }
    let chosen = match request.request {
        MindRequest::Read => {
            let centre = centre(ctx.w).ok_or(SimError::Unavailable)?;
            for line in fill_read_lines(centre, account, staff, request.side) {
                ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
            }
            return Ok(());
        }
        MindRequest::Clear => None,
        MindRequest::Choose {
            mind,
            revision,
            profiles,
        } => Some((mind, revision, profiles)),
    };
    let (team, lead_pilot) = {
        let centre = centre(ctx.w).ok_or(SimError::Unavailable)?;
        if centre.state != MatchState::Selection {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let led = account.and_then(|account| {
            Team::ALL
                .into_iter()
                .find(|team| centre.leads_side(*team, account))
        });
        let team = match (request.side, led) {
            (Some(side), Some(led)) if side == led => side,
            (Some(side), _) if staff => side,
            (None, Some(led)) => led,
            (None, None) if staff => return Err(SimError::FillWhichSide),
            _ => return Err(SimError::NotLeadForAi),
        };
        let lead_pilot = (led == Some(team))
            .then(|| {
                account.and_then(|account| {
                    centre
                        .slots
                        .iter()
                        .find(|slot| slot.team == team && slot.account == account)
                        .map(|slot| slot.pilot)
                })
            })
            .flatten();
        (team, lead_pilot)
    };
    let (stored, reply) = match chosen {
        None => (None, fill_line(team, None)),
        Some((word, revision, profiles)) => {
            if revision {
                return Err(SimError::NoRevisions);
            }
            let held = match lead_pilot {
                Some(pilot) => holdings::resolve(&holdings::held_by(ctx.w, pilot), &word)
                    .cloned()
                    .ok_or_else(|| SimError::NoSuchMind {
                        word: word.trim().to_owned(),
                    })?,
                None => {
                    let doctrine = HeldMind::doctrine();
                    if !holdings::names(&doctrine, &word) {
                        return Err(SimError::StaffHoldNoMinds);
                    }
                    doctrine
                }
            };
            if profiles.len() > 1 {
                return Err(SimError::FillOneProfile);
            }
            let profiles = resolve_profiles(&held, &profiles)?;
            let per_side =
                configs(ctx.w).map_or(1, |(_, match_cfg, _)| slots_per_side(ctx.w, &match_cfg));
            for index in 1..=per_side {
                if let Some(chassis) = authored_unit_type(ctx.w, team, index) {
                    fit_on_chassis(ctx.w, &chassis, &held, &profiles)?;
                }
            }
            let choice = MindChoice {
                mind: held.identity.mind.clone(),
                profiles,
            };
            let reply = format!("{}'s fill will carry {}.", side_word(team), choice.words());
            let by = match (lead_pilot, account) {
                (Some(_), Some(account)) => FillSetter::Lead(account),
                _ => FillSetter::Staff,
            };
            (Some(FillChoice { team, by, choice }), reply)
        }
    };
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        let centre = book.centre_mut();
        centre.fill_minds.retain(|fill| fill.team != team);
        if let Some(fill) = stored {
            centre.fill_minds.push(fill);
        }
    }
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, reply);
    Ok(())
}

/// Re-ask `Boarding`'s two questions after a slot moved: did a side empty, and
/// is everybody left seated?
///
/// Called wherever a boarding centre loses or gains a seat outside the deadline
/// — [`leave`] here, and cut K.2's `EXIT` through [`slot_vacated`]. Silent, and
/// a no-op on a centre that is not boarding.
fn settle_boarding(ctx: &mut Ctx<'_>) {
    let fill = configs(ctx.w).is_some_and(|(_, match_cfg, _)| match_cfg.ai_fill);
    let (match_id, empty, ready) = {
        let Some(centre) = centre(ctx.w) else {
            return;
        };
        if centre.state != MatchState::Boarding {
            return;
        }
        let Some(match_id) = centre.match_id else {
            return;
        };
        // **16b cut Q.7**: with the fill on, a side losing its last slot is not an
        // abort — the machines take that side. A centre losing its last slot is.
        //
        // **16b cut O.3** (plan item 13): and under a range the targets' side is
        // never "nobody" — it is targets — so the question is the shooters' side
        // alone, which is every slot of the centre, fill or no fill.
        let empty = if fill || centre.mode == MatchMode::Range {
            centre.slots.is_empty().then_some(None)
        } else {
            Team::ALL
                .iter()
                .find(|team| centre.side_len(**team) == 0)
                .copied()
                .map(Some)
        };
        let ready = !centre.slots.is_empty() && centre.slots.iter().all(Slot::is_seated);
        (match_id, empty, ready)
    };
    if let Some(team) = empty {
        // 16b cut D.3 (`B-32`): through the one pre-`Running` funnel.
        call_off(
            ctx,
            match team {
                Some(team) => format!(
                    "Match {match_id} is aborted: the {} side has nobody left to fly it.",
                    team.as_str()
                ),
                None => format!("Match {match_id} is aborted: nobody is left to fly it."),
            },
        );
        return;
    }
    if ready && let Err(error) = run(ctx, match_id) {
        tracing::warn!(
            match_id = match_id.0,
            %error,
            "tacsim: a boarding that lost a slot could not start its match"
        );
    }
}

// ---------------------------------------------------------------------------
// `Selection -> Boarding`: the pods lock
// ---------------------------------------------------------------------------

/// **`SIM START`** — lock the pods and open boarding (doc 16 §3.8, `Boarding`).
///
/// A team lead or staff, with at least one pilot a side. Slot *n* of a side
/// locks to pod *n* of that team room, the match id is minted here so `Boarding`
/// already has a name, and the deadline is `match.board_timeout_ms` from now.
///
/// # Errors
///
/// [`SimError`], as the line the player sees.
pub fn start(ctx: &mut Ctx<'_>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let staff = is_staff(ctx.w, actor);
    let account = match account_of(ctx.w, actor) {
        Ok(account) => Some(account),
        Err(_) if staff => None,
        Err(error) => return Err(error),
    };
    let (_, match_cfg, _) = configs(ctx.w).ok_or(SimError::Unavailable)?;

    // Every refusal before the first write, and before an id is minted.
    {
        let book = ctx
            .w
            .module_state::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let centre = book.centre();
        if centre.state != MatchState::Selection {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let lead = account.is_some_and(|account| {
            Team::ALL
                .iter()
                .any(|team| centre.leads_side(*team, account))
        });
        if !staff && !lead {
            return Err(SimError::NotLead);
        }
        // **16b cut Q.7**: with the fill on, one claimed slot **in the centre**
        // starts a match — the owner asked for exactly one human against the
        // machines. With it off, both sides still need a pilot, byte for byte.
        // `Selection` is only reachable with a slot in it, so the first arm is a
        // guard and not a path.
        //
        // **16b cut O.3** (16c N13; plan item 13): under a range the targets'
        // side is never empty — its pods are the targets' — so the question is
        // the shooters' side alone, which is every claimed slot: a range starts
        // on a server with the fill off, which is the shipped default.
        if match_cfg.ai_fill || centre.mode == MatchMode::Range {
            if centre.slots.is_empty() {
                return Err(SimError::NobodyPlaying);
            }
        } else {
            for team in Team::ALL {
                if centre.side_len(team) == 0 {
                    return Err(SimError::EmptySide(team.as_str()));
                }
            }
        }
    }
    // Every pod and every unit must exist before anything is locked: a match
    // that boards into a centre with no pods is a match nobody can seat.
    let mut locks: Vec<(usize, ObjId)> = Vec::new();
    {
        let Some(centre) = centre(ctx.w) else {
            return Err(SimError::Unavailable);
        };
        for (position, slot) in centre.slots.iter().enumerate() {
            let pod = centre::pod_object(ctx.w, slot.team, slot.index).ok_or(SimError::NoCentre)?;
            if centre::tank_for_slot(ctx.w, slot.team, slot.index).is_none() {
                return Err(SimError::NoUnit {
                    team: slot.team.as_str(),
                    index: slot.index,
                });
            }
            locks.push((position, pod));
        }
    }
    // **16b cut AE.3** (plan §6 AE-X3): a side that would field no gun, said
    // now, with the other refusals and before the first write — over the
    // claimed slots' effective types (their choice, else the pod's authored
    // type, which is what the refit below fields) plus, with the fill on and no
    // range, the authored types of the pods the fill will add ([`fill_pods`],
    // the walk [`ready`] makes). A fill-on lone drone pilot starts when the
    // fill brings its side a gun (`SIM AI 2`: a tank from the next pod); at the
    // fill's own size of one — the larger side — the drone is its side's one
    // machine and is refused here, as a fill-off one is. It is a courtesy:
    // [`ready`] is where it binds, because the boarding deadline can still drop
    // a slot and the fill tops the side up again there.
    {
        let Some(centre) = centre(ctx.w) else {
            return Err(SimError::Unavailable);
        };
        let targets = range_sides(centre).map(|(_, targets)| targets);
        for team in Team::ALL {
            if targets == Some(team) {
                continue;
            }
            let claimed: Vec<&Slot> = centre
                .slots
                .iter()
                .filter(|slot| slot.team == team)
                .collect();
            let mut types: Vec<SmolStr> = claimed
                .iter()
                .filter_map(|slot| {
                    slot.unit_type
                        .clone()
                        .or_else(|| authored_unit_type(ctx.w, team, slot.index))
                })
                .collect();
            if match_cfg.ai_fill && targets.is_none() {
                let fielded: Vec<ObjId> = claimed
                    .iter()
                    .filter_map(|slot| centre::tank_for_slot(ctx.w, team, slot.index))
                    .collect();
                types.extend(
                    fill_pods(ctx.w, centre, &match_cfg, team, &fielded)
                        .into_iter()
                        .filter_map(|(index, _)| authored_unit_type(ctx.w, team, index)),
                );
            }
            side_fields_a_gun(ctx.w, team, &types)?;
        }
    }
    // **16b cut Q.8**: the refits, after every refusal above and before a single
    // pod is locked or an id is minted. Every pod of the centre, not only the
    // ones that chose something — a pod with no choice on it is put back to its
    // authored type here, which is what stops a stale refit reaching the next
    // claimant or the AI fill.
    fit_fielded_units(ctx)?;
    let match_id = crate::match_::allocate_id(ctx).map_err(|_| SimError::Unavailable)?;
    let deadline = ctx
        .w
        .now()
        .saturating_mul(1_000)
        .saturating_add(i64::from(match_cfg.board_timeout_ms));
    {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let centre = book.centre_mut();
        for (position, pod) in locks {
            if let Some(slot) = centre.slots.get_mut(position) {
                slot.pod = Some(pod);
            }
        }
        centre.state = MatchState::Boarding;
        centre.match_id = Some(match_id);
        centre.board_deadline_ms = Some(deadline);
        // **17b cut M5.2** (Q-UM-15; the owner's OQ-5): decided once, here,
        // from the claims — humans on both sides.
        centre.doctrine_only = Team::ALL.iter().all(|team| centre.side_len(*team) > 0);
    }
    tracing::info!(
        match_id = match_id.0,
        actor = actor.0,
        "tacsim: SIM START opened boarding"
    );
    announce(
        ctx,
        format!(
            "Match {match_id} is boarding. Every pilot: BOARD your pod. The match starts when \
             everyone is seated, or in {} second(s).",
            match_cfg.board_timeout_ms / 1_000
        ),
    );
    // **16b cut AH.1** (16c N18): the boarding point, after the announce so the
    // hint reads under the line it follows, to every claimed slot's pilot —
    // `tutorial::hint` decides, per account, whether it is said.
    let pilots: Vec<ObjId> = centre(ctx.w)
        .map(|centre| centre.slots.iter().map(|slot| slot.pilot).collect())
        .unwrap_or_default();
    for pilot in pilots {
        crate::match_::tutorial::hint(ctx, pilot, crate::match_::tutorial::HintPoint::Boarding);
    }
    // **17b cut M5.2** (Q-UM-15; the design note's item 11): each pilot whose
    // own choice — or, for a lead, whose fill's — the check-out will seal as
    // Standard Doctrine is told so now, to that pilot alone (the actor of
    // `SIM START` is whoever typed it), once. Never in production before
    // 17b cut M6 (production holds Standard Doctrine alone).
    let told = centre(ctx.w)
        .map(|centre| substituted_pilots(ctx.w, centre, &match_cfg))
        .unwrap_or_default();
    for pilot in told {
        ctx.out
            .sys_to_actor(pilot, NOTICE_CATEGORY, HUMANS_ON_THE_OTHER_SIDE);
    }
    Ok(())
}

/// **Fit every fielded pod's tank to the type that pod is to fly** (16b cut Q.8,
/// walked over every slot by the fixer), the last thing [`start`] does before it
/// writes anything of its own.
///
/// The type one pod is to fly is `slot.unit_type.or(authored_unit_type(..))`:
/// the claimant's own choice if the slot holds one, and **otherwise the type the
/// arena pack authors that pod with**. That `or` is the whole of the fix.
/// Walking only the slots that chose something left a pod refitted by an earlier
/// refused `SIM START` — or by a pilot who chose and then left — standing as the
/// machine nobody in this lobby asked for: the next claimant of that slot boarded
/// it, and with `[tacsim.match].ai_fill` on the fill flew it, against the one
/// sentence `docs/help/simulator.md` makes about the machines ("the machines that
/// fill the unclaimed pods always fly the pod's own tank").
///
/// So **every** pod of the centre is walked, claimed or not, and the ones with no
/// choice on them are put back to their authored type before a single pod locks.
/// A tank already holding the type it is to fly is not touched, so the healthy
/// lobby — where nobody chose anything and the last `Idle` transition already put
/// every tank back — writes nothing at all.
///
/// **A refusal on a claimed slot refuses the START**, before the pods lock and
/// before an id is minted, so no partial lock is taken — but a tank already
/// refitted in this same call **stays refitted**. There is no rollback and there
/// does not need to be one: **every** return to `Idle` puts every roster tank
/// back to its authored type (`crate::match_::reset_stock_tanks`), and a
/// half-refitted centre that started no match is exactly the state that
/// transition exists to clear. All four paths there run it — `@arena/reset`, a
/// [`claim`] on a terminal centre, the boot pass, and the last [`leave`].
///
/// **A refusal on a pod nobody claimed is logged and skipped.** An unclaimed pod
/// whose tank cannot be put back is a broken world (a stale `deployed` marker, a
/// schema that stopped loading) and not something the pilots who are trying to
/// start a match can do anything about; refusing their START over a machine's
/// chassis would be a worse answer than letting the machine fly what it stands
/// in. The next `Idle` reports the same tank in `ResetReport::deployed` or
/// `failed`, which is the surface staff already read.
///
/// # Errors
///
/// [`SimError::Refit`], naming the slot and the type; and [`SimError::NoUnit`]
/// for a claimed slot whose pod has no tank at all, which [`start`] has already
/// refused above.
fn fit_fielded_units(ctx: &mut Ctx<'_>) -> Result<(), SimError> {
    let (_, match_cfg, _) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    let per_side = slots_per_side(ctx.w, &match_cfg);
    // Every pod of the centre in board order, each with the claim on it if there
    // is one — read in one borrow, because everything below needs `ctx` mutably.
    // A claimed slot whose index is outside the configured count (an operator
    // narrowed `max_slots_per_side` under a live lobby) is still walked: it is in
    // the match, and the fill's own loop is what the count bounds.
    //
    // **16b cut O.3** (16c N13): under a range the targets' pods 1 to `n` want
    // [`crate::definitions::content::TARGET_MK1`], and are walked as **claimed**
    // with a choice on them — a refusal on one refuses the start as a claimed
    // slot's does, with that pod's own index, because the range cannot run
    // without its targets. Every other pod wants what it wanted before.
    let pods: Vec<(Team, u8, bool, Option<SmolStr>)> = {
        let Some(centre) = centre(ctx.w) else {
            return Err(SimError::Unavailable);
        };
        let range = range_sides(centre)
            .zip(centre.range_targets)
            .map(|((_, targets), count)| (targets, count));
        let mut pods: Vec<(Team, u8, bool, Option<SmolStr>)> = Vec::new();
        for team in Team::ALL {
            let targets = range
                .filter(|(targets, _)| *targets == team)
                .map_or(0, |(_, count)| count);
            for index in 1..=per_side.max(targets) {
                if index <= targets {
                    pods.push((
                        team,
                        index,
                        true,
                        Some(SmolStr::new_static(crate::definitions::content::TARGET_MK1)),
                    ));
                    continue;
                }
                let claim = centre
                    .slots
                    .iter()
                    .find(|slot| slot.team == team && slot.index == index);
                pods.push((
                    team,
                    index,
                    claim.is_some(),
                    claim.and_then(|slot| slot.unit_type.clone()),
                ));
            }
        }
        for slot in &centre.slots {
            if slot.index > per_side {
                pods.push((slot.team, slot.index, true, slot.unit_type.clone()));
            }
        }
        pods
    };
    // The reads, then the writes: `refit_unit` takes `ctx` mutably, and the
    // binding it needs is cloned out first.
    let mut owed: Vec<(Team, u8, bool, ObjId, String, mush_tactics::UnitSpec)> = Vec::new();
    for (team, index, claimed, chosen) in pods {
        let had_choice = chosen.is_some();
        let Some(want) = chosen.or_else(|| authored_unit_type(ctx.w, team, index)) else {
            // No choice and no authored type to fall back on: there is nothing
            // this pass could put the tank back to, and the `Idle` reset has the
            // same answer.
            continue;
        };
        let Some(tank) = centre::tank_for_slot(ctx.w, team, index) else {
            if claimed {
                return Err(SimError::NoUnit {
                    team: team.as_str(),
                    index,
                });
            }
            // An index with no pod at all: a centre whose content authors fewer
            // tanks than the configured count. Nothing to fit.
            continue;
        };
        let word = type_word(ctx.w, want.as_str());
        let entry = match mush_core::CanonicalId::new(want.as_str()) {
            Ok(entry) => entry,
            Err(error) => {
                let why = format!("`{want}` is not a canonical id: {error}");
                if claimed {
                    return Err(refit_refusal(team, index, word, why));
                }
                skipped_pod(team, index, tank, &word, &why);
                continue;
            }
        };
        let binding = ctx
            .w
            .module_state::<crate::TacsimStore>()
            .and_then(|store| store.definitions().bindings.get(&entry).cloned());
        let Some(binding) = binding else {
            let why = format!("no unit schema `{want}` is loaded");
            // A *choice* the catalog has lost between `SIM UNIT` and `SIM START`
            // refuses the start; an authored type that is not loaded is the
            // world the reset already lives with, and the tank stands.
            if claimed && had_choice {
                return Err(refit_refusal(team, index, word, why));
            }
            skipped_pod(team, index, tank, &word, &why);
            continue;
        };
        // Already that type: the tank was refitted by an earlier `SIM START` this
        // selection, or it is authored as one and nothing has moved it. Nothing
        // is written, which is every pod of a healthy lobby.
        if mush_tactics::module::unit_of(ctx.w, tank)
            .is_some_and(|held| *held.schema() == binding.spec.definition)
        {
            continue;
        }
        owed.push((team, index, claimed, tank, word, binding.spec));
    }
    for (team, index, claimed, tank, word, spec) in owed {
        if let Err(why) = mush_tactics::module::refit_unit(ctx, tank, &spec) {
            if claimed {
                return Err(refit_refusal(team, index, word, why));
            }
            skipped_pod(team, index, tank, &word, &why);
        }
    }
    Ok(())
}

/// The refusal [`fit_fielded_units`] makes, with the raw reason put where staff
/// read it and out of the sentence the lead sees (16b cut Q.8, fixer).
fn refit_refusal(team: Team, index: u8, unit_type: String, why: String) -> SimError {
    tracing::warn!(
        team = team.as_str(),
        index,
        unit_type = %unit_type,
        why = %why,
        "tacsim: a claimed slot's unit could not be fitted; the start is refused"
    );
    SimError::Refit {
        team: team.as_str(),
        index,
        unit_type,
        why,
    }
}

/// A pod nobody claimed whose tank could not be put back: logged, and the match
/// goes on (16b cut Q.8, fixer).
fn skipped_pod(team: Team, index: u8, tank: ObjId, unit_type: &str, why: &str) {
    tracing::warn!(
        team = team.as_str(),
        index,
        unit = %tank,
        unit_type,
        why,
        "tacsim: an unclaimed pod's tank could not be fitted; it fields what it stands in"
    );
}

/// **Put one pod's tank back to the type its arena pack authors** (16b cut Q.8,
/// fixer) — what a slot's choice leaves behind when the slot itself goes.
///
/// [`Slot::unit_type`] is cleared with the slot, but the *tank* is not: after a
/// `SIM START` the machine standing in that pod is the one the departing pilot
/// chose, and during `Boarding` there is no return to `Idle` between their going
/// and the check-out. So the two paths that drop a slot outright — [`leave`] and
/// [`board_deadline`] — put that pod's tank back here, and the choice really does
/// go with the pilot: the next claimant of the slot boards the pod's own machine,
/// and with `[tacsim.match].ai_fill` on the fill flies the authored type, which is
/// what `docs/help/simulator.md` promises.
///
/// It refuses nothing. A leave and a deadline are both things that have already
/// happened, and neither can be undone over a tank; a failure is logged and the
/// next `Idle` reset reports the tank again.
///
/// Silent and idempotent: a pod already holding its authored type — every pod of
/// every lobby in which nobody chose anything — is compared and left alone.
fn unfit_slot(ctx: &mut Ctx<'_>, team: Team, index: u8) {
    let Some(want) = authored_unit_type(ctx.w, team, index) else {
        return;
    };
    let Some(tank) = centre::tank_for_slot(ctx.w, team, index) else {
        return;
    };
    let Ok(entry) = mush_core::CanonicalId::new(want.as_str()) else {
        return;
    };
    let binding = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .and_then(|store| store.definitions().bindings.get(&entry).cloned());
    let Some(binding) = binding else {
        return;
    };
    if mush_tactics::module::unit_of(ctx.w, tank)
        .is_some_and(|held| *held.schema() == binding.spec.definition)
    {
        return;
    }
    if let Err(why) = mush_tactics::module::refit_unit(ctx, tank, &binding.spec) {
        tracing::warn!(
            team = team.as_str(),
            index,
            unit = %tank,
            why,
            "tacsim: a vacated slot's tank could not be put back to its authored type"
        );
    }
}

// ---------------------------------------------------------------------------
// `Boarding -> Running`: the named trigger
// ---------------------------------------------------------------------------

/// **Report one pilot seated** — the trigger's input, and the one function cut
/// K.2's `BOARD` calls when it has put a character in a pod and activated the
/// seat.
///
/// Answers `Ok(true)` when *this* seating was the one that fired `Boarding →
/// Running` (doc 16 §3.8's named transition), `Ok(false)` when the match is
/// still waiting on somebody.
///
/// # Errors
///
/// [`SimError`]: the centre is not boarding this match, or the pilot holds no
/// slot in it, or the check-out itself refused.
pub fn slot_seated(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    pilot: ObjId,
    unit: ObjId,
) -> Result<bool, SimError> {
    {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let centre = book.centre_mut();
        if centre.state != MatchState::Boarding || centre.match_id != Some(match_id) {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let Some(slot) = centre.slots.iter_mut().find(|slot| slot.pilot == pilot) else {
            return Err(SimError::NoSlot);
        };
        slot.unit = Some(unit);
        if !centre.slots.iter().all(Slot::is_seated) {
            return Ok(false);
        }
    }
    run(ctx, match_id)?;
    Ok(true)
}

/// **Report one pilot out of their pod again** — the symmetric half of
/// [`slot_seated`], which cut K.2's `EXIT` owes the centre when it runs during
/// `Boarding`.
///
/// The slot keeps its place and its pod; what it loses is the seat, so the
/// `Boarding → Running` trigger stops counting it as ready and the deadline
/// drops it like any other slot nobody boarded. Without this, a pilot who
/// boards and steps back out is still "seated" and the match starts with an
/// empty pod.
///
/// If the vacancy empties a side the match aborts on the spot, which is the same
/// answer the deadline would give later.
///
/// # Errors
///
/// [`SimError`]: the centre is not boarding that match, or the pilot holds no
/// slot in it.
pub fn slot_vacated(ctx: &mut Ctx<'_>, match_id: MatchId, pilot: ObjId) -> Result<(), SimError> {
    {
        let book = ctx
            .w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let centre = book.centre_mut();
        if centre.state != MatchState::Boarding || centre.match_id != Some(match_id) {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let Some(slot) = centre.slots.iter_mut().find(|slot| slot.pilot == pilot) else {
            return Err(SimError::NoSlot);
        };
        slot.unit = None;
    }
    settle_boarding(ctx);
    Ok(())
}

/// **Mark one pilot's slot as out of its cockpit** (16b cut F.1).
///
/// **16b cut Y.3**: *for good* only by an `EXIT`. A drop reaches this line too,
/// but [`slot_dropped`] has stamped [`Slot::dropped_at_ms`] a moment before, so
/// the slot reads `flown by its mind` (`flown by doctrine` until 17b cut M5.3)
/// and [`slot_reclaimed`] can put the pilot back; this function is unchanged
/// and still knows no route.
///
/// [`Slot::left`] is what stops `SIM STATUS` calling somebody standing in the
/// team room `seated`, and it belongs **where the release is decided**, not in
/// `EXIT`. `EXIT` is one of three routes to the same `Control { Released }`: the
/// others are a `QUIT` (`SeatFacts::disconnected`) and the linkdead grace running
/// out, and both reach the pilot's standing in exactly the same way — the arena
/// drops them from its frame roster, the registry drops their binding, and a
/// relogin sends no `Live`. So [`crate::arena_bridge::send_control`] calls this
/// for every `Released` that names a pilot, and the two paths nobody thought of
/// are marked by the same line as the one that was.
///
/// Only during `Running`: a seat given up while the centre is `Boarding` is not
/// final (the pilot may board again), and [`slot_vacated`] is that story.
///
/// **And it is where the unattended clock starts** (16b cut D.3, `B-33`): when
/// the marking leaves a centre with at least one slot and every one of them
/// `left`, the moment is stamped on [`Centre::unattended_since_ms`] and
/// [`unattended_deadline`] ends the match once the configured interval has
/// passed. A centre with **no** slots — a staff `@arena/start` — is never
/// stamped, because nobody left it.
///
/// A destroyed pilot who stays in their pod is **attending**: nothing here reads
/// a wreck, only whether somebody gave their seat up.
///
/// **And it is where the mind's span opens** (17b cut M5.6; the owner's OQ-9
/// (ii); the design note's M5.6 item 1): a slot this marking moves opens a
/// span on its pilot's debrief fold ([`crate::match_::report::Fold::released`])
/// at the arena's own clock as the world last heard it
/// (`crate::arena_bridge::MatchMirror::now_ms`, the orchestrator's ruling at
/// 17b cut M5.4's close), `Dropped` when [`slot_dropped`] stamped the slot a
/// moment before (a drop, a `QUIT`, a resume) and `ClimbedOut` otherwise (an
/// `EXIT`) — the one fact of the `Your mind` block neither the tape nor the
/// check-out carries. A match whose mirror the registry no longer holds
/// opens none. A wreck's pilot who climbs out (`EXIT leaves the pod.`) or
/// drops opens one too, since nothing here reads a wreck: the fold reads the
/// hull's destruction off the tape and the block reads no span from it on
/// (the mind flies no wreck; 17b cut M5.6's review).
///
/// Answers whether a slot moved, which is nothing anybody prints — it is here so
/// the caller pays for no borrow it does not need.
pub(crate) fn slot_left(ctx: &mut Ctx<'_>, pilot: ObjId) -> bool {
    let now_ms = ctx.w.now().saturating_mul(1_000);
    // `MatchMirror::now_ms`, the arena's own clock — never the mirror's
    // `arena_ms`, the ledger's last stamp (the ruling at 17b cut M5.4's close).
    let mirror_now_ms = centre(ctx.w)
        .and_then(|centre| centre.match_id)
        .and_then(|match_id| arena_now_ms(ctx.w, match_id));
    let Some(book) = ctx.w.module_state_mut::<MatchBook>() else {
        return false;
    };
    let centre = book.centre_mut();
    if centre.state != MatchState::Running {
        return false;
    }
    let mut handed = None;
    let moved = match centre.slots.iter_mut().find(|slot| slot.pilot == pilot) {
        Some(slot) if !slot.left => {
            slot.left = true;
            handed = Some(if slot.dropped_at_ms.is_some() {
                HandOver::Dropped
            } else {
                HandOver::ClimbedOut
            });
            // **16b cut B.4** (cut AA's residual): a lead who leaves the tank
            // mid-match hands the side on at once — before this, `left` was
            // set and the lead kept, so no succession ran and the teammates
            // still fighting read `Only a side's lead can concede for it.`
            centre.resettle_leads();
            true
        }
        _ => false,
    };
    if centre.unattended_since_ms.is_none()
        && !centre.slots.is_empty()
        && centre.slots.iter().all(|slot| slot.left)
    {
        centre.unattended_since_ms = Some(now_ms);
    }
    let match_id = centre.match_id;
    if let (Some(by), Some(match_id), Some(at_ms)) = (handed, match_id, mirror_now_ms)
        && let Some(fold) = pilot_fold_mut(book, match_id, pilot)
    {
        fold.released(at_ms, by);
    }
    moved
}

/// **Record that a drop, and not an `EXIT`, is taking one pilot's seat** (16b
/// cut Y.3; 16c `N4`; doc 07 §1.3 "Reclaiming control").
///
/// Called by `arena_bridge`'s `SeatFacts::disconnected` — the linkdead grace
/// running out, or a clean `QUIT` — **before** its `Released` reaches
/// `send_control`, which marks the slot [`Slot::left`] through [`slot_left`]:
/// the slot has to know the route before it is marked, because `slot_left` is
/// one line for every route and knows none of them.
///
/// Writes [`Slot::dropped_at_ms`] on the pilot's slot during `Running`, and
/// **only on a slot still seated and not yet `left`**: a slot already out of its
/// cockpit — by an `EXIT` — is never turned into a drop. (An `EXIT` also takes
/// the registry binding with it, so the hook's seated arm never runs after one;
/// the guard is the second lock on the same door.) Nothing else moves here: the
/// succession and the unattended stamp are `slot_left`'s, a moment later.
///
/// Answers whether it wrote.
pub(crate) fn slot_dropped(ctx: &mut Ctx<'_>, pilot: ObjId, now_ms: i64) -> bool {
    let Some(book) = ctx.w.module_state_mut::<MatchBook>() else {
        return false;
    };
    let centre = book.centre_mut();
    if centre.state != MatchState::Running {
        return false;
    }
    match centre.slots.iter_mut().find(|slot| slot.pilot == pilot) {
        Some(slot) if !slot.left && slot.is_seated() => {
            slot.dropped_at_ms = Some(now_ms);
            true
        }
        _ => false,
    }
}

/// **Make one pilot's leave final** (16b cut Y.3; doc 16 §11's N.1 amendment).
///
/// `EXIT` during `Running` calls this after it has handed the seat back. A
/// pilot who dropped (their slot [`Slot::is_dropped`]), reconnected inside the
/// pod the drop left them in and typed `EXIT` still holds the billet, so `EXIT`
/// finds it and hands it back — but the `Released` that sends reaches
/// [`slot_left`] on a slot already `left`, which changes nothing, and the drop's
/// stamp would survive: the `EXIT` sentence *you cannot take it back* would be
/// false and a `BOARD` would take the tank back anyway. So `EXIT` clears the
/// stamp here, and a deliberate leave is one-way whatever came before it.
///
/// Answers whether it cleared anything.
pub(crate) fn slot_exited(ctx: &mut Ctx<'_>, pilot: ObjId) -> bool {
    let Some(book) = ctx.w.module_state_mut::<MatchBook>() else {
        return false;
    };
    let centre = book.centre_mut();
    if centre.state != MatchState::Running {
        return false;
    }
    centre
        .slots
        .iter_mut()
        .find(|slot| slot.pilot == pilot)
        .and_then(|slot| slot.dropped_at_ms.take())
        .is_some()
}

/// **Give a dropped pilot their seat back** (16b cut Y.3; 16c `N4`; doc 07 §1.3
/// "Reclaiming control") — the centre's half of a `BOARD` in a running match.
///
/// The slot is back in its cockpit: [`Slot::left`] `false`, [`Slot::dropped_at_ms`]
/// `None`, [`Slot::unit`] the tank. Then two things the drop set in motion are
/// undone:
///
/// - **the unattended clock** — a drop that left nobody aboard stamped
///   [`Centre::unattended_since_ms`], and the match just given back must not end
///   under the pilot who took it. The stamp and its once-per-match warning go
///   together, through the one helper that keeps them from drifting;
/// - **the leads**, resettled over the slots as they now stand. The recorded
///   heir keeps the lead (it is checked first), so a reclaimed pilot does **not**
///   take the side back — 16c's owner question Q42 at its default; a side whose
///   every claimant had left had no entry, and the first to come back leads it.
///
/// **17b cut M5.6**: and the mind's span the drop opened on the pilot's debrief
/// fold closes ([`crate::match_::report::Fold::reclaimed`]), at the arena's own
/// clock as the world last heard it — [`slot_left`]'s clock.
///
/// # Errors
///
/// [`SimError::WrongState`] unless the centre is `Running` on `match_id` **and**
/// the match book's record for it is `Running` too: `unattended_deadline` and
/// `SIM ABORT` park the record in `Stopping` while the centre is still `Running`,
/// and a pilot who took the tank back then would read *yours again* and, a
/// moment later, the end. [`SimError::NoSlot`] when the pilot holds no slot.
pub(crate) fn slot_reclaimed(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    pilot: ObjId,
    unit: ObjId,
) -> Result<(), SimError> {
    // `MatchMirror::now_ms`, as `slot_left` reads it.
    let mirror_now_ms = arena_now_ms(ctx.w, match_id);
    let book = ctx
        .w
        .module_state_mut::<MatchBook>()
        .ok_or(SimError::Unavailable)?;
    let record = book.get(match_id).map(|record| record.state);
    let centre = book.centre_mut();
    if centre.state != MatchState::Running || centre.match_id != Some(match_id) {
        return Err(SimError::WrongState {
            state: centre.state.as_str(),
        });
    }
    // The centre is running this match; the record says whether a stop is
    // already in flight, and its word is the one the pilot reads.
    if record != Some(MatchState::Running) {
        return Err(SimError::WrongState {
            state: record.unwrap_or(centre.state).as_str(),
        });
    }
    let Some(slot) = centre.slots.iter_mut().find(|slot| slot.pilot == pilot) else {
        return Err(SimError::NoSlot);
    };
    slot.left = false;
    slot.dropped_at_ms = None;
    slot.unit = Some(unit);
    // 16b cut AF.3: the pilot is back; a resume owes them nothing more.
    slot.released_by_resume = false;
    centre.clear_unattended();
    centre.resettle_leads();
    // **17b cut M5.6**: the mind's span closes, on the clock `slot_left`
    // opened it on.
    if let Some(at_ms) = mirror_now_ms
        && let Some(fold) = pilot_fold_mut(book, match_id, pilot)
    {
        fold.reclaimed(at_ms);
    }
    Ok(())
}

/// **`pilot`'s debrief fold in match `match_id`** (17b cut M5.6): the one
/// whose roster seats the pilot (`Fold::pilot`), where [`slot_left`] opens
/// and [`slot_reclaimed`] closes the `Your mind` block's spans.
fn pilot_fold_mut(
    book: &mut MatchBook,
    match_id: MatchId,
    pilot: ObjId,
) -> Option<&mut crate::match_::report::Fold> {
    book.get_mut(match_id)?
        .folds
        .values_mut()
        .find(|fold| fold.pilot() == Some(pilot))
}

/// **The resume line has been said to `pilot`** (16b cut AF.3; 16c N16,
/// `Q-AF4`): the slot's `released_by_resume` is cleared, so the pilot reads
/// `match_::RESUME_LINE` once, whichever path said it — the boarding tick to
/// a session that survived the restart, or `SeatFacts::connected` to a later
/// login — and a login after that reads Y.3's line alone, as any dropped
/// pilot's does. Nothing when the pilot holds no slot.
pub(crate) fn resume_line_said(ctx: &mut Ctx<'_>, pilot: ObjId) {
    if let Some(slot) = ctx.w.module_state_mut::<MatchBook>().and_then(|book| {
        book.centre_mut()
            .slots
            .iter_mut()
            .find(|slot| slot.pilot == pilot)
    }) {
        slot.released_by_resume = false;
    }
}

/// Everything [`run`] needs once it has decided the match can be attempted.
struct Ready {
    /// The arena definition's id.
    arena: SmolStr,
    /// The sides, each with its fielded units.
    sides: Vec<Side>,
    /// `[tacsim.arena]`.
    arena_cfg: ArenaConfig,
    /// `[tacsim.match]`.
    match_cfg: MatchConfig,
    /// `[tacsim.sensors]`.
    sensors_cfg: SensorsConfig,
    /// **The rule the match is fought under** (16b cut O.1), off
    /// [`Centre::mode`].
    mode: MatchMode,
}

/// Fire the check-out with whatever is seated (doc 16 §3.8's
/// `Boarding → Running`).
///
/// **16b cut D.3 (`B-32`, `T-50`): every refusal ends the lobby.** Before this
/// cut the three refusals [`ready`] makes — `Unavailable`, `NobodyPlaying`,
/// `EmptySide` — simply returned `Err` from here, and the two callers that reach
/// `run` without a player behind them ([`settle_boarding`] and
/// [`board_deadline`]) logged it at `warn` and went home. The centre was then
/// stuck in `Boarding` with its deadline already cleared, its pods locked and a
/// pilot sitting in one whose only way out was `SIM ABORT` — a state no tick
/// could leave. Both arms now go through [`call_off`], which is the one funnel
/// every pre-`Running` end uses, and `run` still hands the `Err` back to whoever
/// typed something.
fn run(ctx: &mut Ctx<'_>, match_id: MatchId) -> Result<(), SimError> {
    let Ready {
        arena,
        sides,
        arena_cfg,
        match_cfg,
        sensors_cfg,
        mode,
    } = match ready(ctx) {
        Ok(ready) => ready,
        Err(error) => {
            // **16b cut AE.3**: every `ready` refusal is a whole sentence with
            // its own full stop (the arena's check-out errors below are not), so
            // the stop is the sentence's and not doubled: `…carries one. The pods
            // are free again.`, never `one.. The pods`.
            let said = error.to_string();
            call_off(
                ctx,
                format!(
                    "Match {match_id} could not start: {}. The pods are free again.",
                    said.trim_end_matches('.')
                ),
            );
            return Err(error);
        }
    };
    // **17b cut M4.3**: the unseated hulls are minds (`crate::match_::start`).
    match crate::match_::start(
        ctx,
        match_id,
        &arena,
        &sides,
        &arena_cfg,
        &match_cfg,
        &sensors_cfg,
        mode,
    ) {
        Ok(_) => {
            if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
                // 16b cut U.4 (16c `T-52`): this check-out used whatever
                // `@arena/pose` had set, so this match's end is what clears it.
                // A check-out that refused used nothing, and the overrides stand
                // for the next attempt.
                book.consume_openings(match_id);
                let centre = book.centre_mut();
                centre.state = MatchState::Running;
                centre.board_deadline_ms = None;
            }
            tracing::info!(match_id = match_id.0, "tacsim: match entered Running");
            // **16b cut Z.4 (16c `N8`)**: the sides' nets, here and not in
            // `start` — staff `@arena/start` shares `start`, and nothing inside it
            // tells a lobby pilot from a staff-seated one — and only after its
            // `Ok`, so a refused check-out joins nobody. **Before** the
            // announcement, in the same batch: `Good hunting` stays the last line
            // of the check-out, which is the line every client and every wire test
            // waits on to know the match is theirs, so the join line is already in
            // hand by then and never lands in the answer to what they type next.
            join_the_nets(ctx);
            // **17b cut M5.2** (doc 17 §12; D7, C-18; the design note's item
            // 12): each seated pilot whose slot chose a mind, or whose choice
            // was substituted, reads what was sealed — to that pilot alone,
            // before `Good hunting.`, which stays the check-out's last line.
            tell_seal_lines(ctx, match_id, &sides);
            announce(ctx, format!("Match {match_id} is running. Good hunting."));
            Ok(())
        }
        Err(error) => {
            // Nothing is deployed (cut T.3's `start` rolls its own writes back),
            // so the centre gives the pods and the slots back and says why.
            // **17b cut M3.3**: `CardUnlawful` is a whole sentence with its own
            // full stop, so the stop is trimmed as AE.3 trims `ready`'s.
            // **17b cut M4.3**: the pool and a sealed mind the server cannot
            // run — the two refusals the flip made reachable here — in their
            // short form, under 120 columns (`StartError::call_off_words`).
            //
            // **17b cut M5.1** (its review): and the actor — the last
            // BOARDer, whose `BOARD` answers with this `Err` byte for byte,
            // or nobody on the deadline's tick — reads the same short form,
            // as its own sentence. `StartError::Pool`'s long form names a
            // hull and the class it is admitted at, which marks a chosen
            // mind to a pilot who may be on the other side, and runs past
            // 120 columns; it is the log's, here, and the fixtures'.
            // `CardUnlawful`'s short form is its own sentence, so its reply
            // is unchanged.
            tracing::warn!(
                match_id = match_id.0,
                %error,
                "tacsim: the check-out refused, and the match is called off"
            );
            let said = error.call_off_words(ctx.w, &sides);
            let said = said.trim_end_matches('.');
            call_off(
                ctx,
                format!("Match {match_id} could not start: {said}. The pods are free again."),
            );
            Err(SimError::Start(format!("{said}.")))
        }
    }
}

/// **The seal line** (17b cut M5.2; doc 17 §12 "The station view says so in
/// one line before `STARTUP`"; the design note's D7, C-18 and item 12):
/// `Sealed for this sortie: Standard Doctrine (Sniper, Line), Class I; MIND
/// reads it.` — the mind and its profiles as the seal names them (Line
/// alone when it names none) and the class the seal records — or, for a
/// choice Q-UM-15 substituted, `…, Class I, for humans on the other side;
/// MIND reads it.` No seal id (C-14). `None` for a seal that names no mind.
#[must_use]
pub fn seal_line(seal: &crate::arena::mind::MindSeal, substituted: bool) -> Option<String> {
    if seal.mind.is_empty() {
        return None;
    }
    let profiles: Vec<SmolStr> = seal
        .profiles
        .iter()
        .map(|profile| profile.name.clone())
        .collect();
    let why = if substituted {
        ", for humans on the other side"
    } else {
        ""
    };
    Some(format!(
        "Sealed for this sortie: {}, Class {}{why}; MIND reads it.",
        mind_words(&seal.mind, &profiles),
        seal.class.as_str()
    ))
}

/// **Tell each choosing pilot what was sealed** (17b cut M5.2; the design
/// note's item 12) — [`run`]'s success arm, after the nets and before `Good
/// hunting.`: per seated slot whose hull's [`SealChoice`] chose a mind or was
/// substituted, its [`seal_line`] off the seal the check-out recorded
/// (`MatchRecord::minds`), **to that slot's pilot alone** — an audience
/// naming the pilot's own object, never the actor of `run` (the last BOARDer,
/// or nobody on the deadline's tick: F2). An unchosen slot's seal is the
/// default the board, the help and `MIND` describe, and is not said (C-18).
/// The fill's choice is no seated pilot's hull, and is not said either.
fn tell_seal_lines(ctx: &mut Ctx<'_>, match_id: MatchId, sides: &[Side]) {
    let seated: Vec<(ObjId, ObjId)> = centre(ctx.w)
        .map(|centre| {
            centre
                .slots
                .iter()
                .filter_map(|slot| slot.unit.map(|unit| (slot.pilot, unit)))
                .collect()
        })
        .unwrap_or_default();
    let lines: Vec<(ObjId, String)> = {
        let Some(record) = ctx
            .w
            .module_state::<MatchBook>()
            .and_then(|book| book.matches.get(&match_id))
        else {
            return;
        };
        seated
            .iter()
            .filter_map(|(pilot, unit)| {
                let choice = sides.iter().find_map(|side| side.minds.get(unit))?;
                if !(choice.chose || choice.substituted) {
                    return None;
                }
                let sealed = record.minds.get(unit)?;
                seal_line(&sealed.seal, choice.substituted).map(|line| (*pilot, line))
            })
            .collect()
    };
    for (pilot, line) in lines {
        ctx.out.sys_to_actor(pilot, NOTICE_CATEGORY, line);
    }
}

/// **Put every human pilot of the match that just started on their side's net**
/// (16b cut Z.4; 16c `N8`; doc 16 §3.7 "Boarding") — [`run`]'s success arm, at
/// the `Boarding → Running` trigger: the last `BOARD` or the board deadline, and
/// never `SIM START`, which only locks the pods.
///
/// One join per slot of the centre, under the slot's callsign as
/// `resolve_callsigns`' first pass gives that pilot's unit —
/// `Designator::Slot { team, slot: index }`, so `RA`, `BE` — which needs no read
/// of the arena. The slots are the humans: a machine the AI fill fields has no
/// slot and no pilot, and never joins. Each side's roster is its own slots'
/// pilots, and [`net::join_pilot`] sweeps whoever else is on that side's net
/// first; a side with no human pilot has its net swept alone, so a stale member
/// cannot outlive a machine-only side either.
///
/// Every slot is seated by the time the centre is `Running` (the trigger waits
/// for all of them, the deadline drops the rest); the `is_seated` filter is the
/// second lock on that door, because a pilot who never boarded must not be on
/// the radio of a match they are not in.
fn join_the_nets(ctx: &mut Ctx<'_>) {
    let seated: Vec<(Team, u8, ObjId)> = centre(ctx.w)
        .map(|centre| {
            centre
                .slots
                .iter()
                .filter(|slot| slot.is_seated())
                .map(|slot| (slot.team, slot.index, slot.pilot))
                .collect()
        })
        .unwrap_or_default();
    for team in Team::ALL {
        let roster: Vec<ObjId> = seated
            .iter()
            .filter(|(side, _, _)| *side == team)
            .map(|(_, _, pilot)| *pilot)
            .collect();
        if roster.is_empty() {
            net::sweep(ctx, team, &roster);
            continue;
        }
        for (_, index, pilot) in seated.iter().filter(|(side, _, _)| *side == team) {
            let callsign = Designator::Slot {
                team,
                slot: u16::from(*index),
            }
            .to_string();
            if !net::join_pilot(ctx, *pilot, team, &callsign, &roster) {
                tracing::debug!(
                    pilot = %pilot,
                    callsign,
                    "tacsim: a pilot was not joined to their side's net (comsys absent, the net \
                     gone, or no free alias)"
                );
            }
        }
    }
}

/// **Test hook** (17b cut M5.1's review): the sides [`run`] would hand the
/// check-out — each hull's `Fielded::mind_class` and each side's
/// `Side::minds`, as [`ready`] resolves them — or its refusal, without
/// running the check-out or writing anything. Nothing in the shipped lane
/// calls it.
#[doc(hidden)]
pub fn ready_sides_for_tests(ctx: &mut Ctx<'_>) -> Result<Vec<Side>, SimError> {
    ready(ctx).map(|ready| ready.sides)
}

/// The sides [`run`] would check out, or the refusal that says it cannot.
fn ready(ctx: &mut Ctx<'_>) -> Result<Ready, SimError> {
    let (arena_cfg, match_cfg, sensors_cfg) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    let arena = centre(ctx.w)
        .map(|centre| centre.arena.clone())
        .filter(|arena| !arena.is_empty())
        .unwrap_or_else(|| SmolStr::new_static(CENTRE_ARENA));
    let mut sides: Vec<Side> = Vec::new();
    // 16b cut O.1: the mode `SIM MODE` chose, a skirmish by default.
    let mode = centre(ctx.w).map_or(MatchMode::Skirmish, |centre| centre.mode);
    // **16b cut U.4 (16c `T-52`)**: the opening cells staff set with
    // `@arena/pose`, which ride each slot's `Fielded` into the check-out — for a
    // claimed pod and an AI-filled one alike, since both are the slot's.
    let openings = ctx
        .w
        .module_state::<MatchBook>()
        .map(|book| book.opening_cells().clone())
        .unwrap_or_default();
    // 17b cut M5.1: the class a chosen mind is admitted at is capped here, as
    // the seal caps it (`seal_for`'s `min(rated, core, class_cap)`).
    let fill_class = minds_config(ctx.w).map_or(CoreClass::I, |cfg| cfg.fill_class);
    {
        let Some(centre) = centre(ctx.w) else {
            return Err(SimError::Unavailable);
        };
        // **16b cut Q.7**: with the fill on, the question is not "has each side a
        // pilot" but "is anybody playing at all". Asked once, over the whole
        // centre, before a single side is built.
        if match_cfg.ai_fill && !centre.slots.iter().any(Slot::is_seated) {
            return Err(SimError::NobodyPlaying);
        }
        // **16b cut O.3** (16c N13; plan item 13): a range with no slot to read
        // the shooters' side off is a range nobody is playing.
        let range = if centre.mode == MatchMode::Range {
            let (_, targets) = range_sides(centre).ok_or(SimError::NobodyPlaying)?;
            Some((targets, centre.range_targets.unwrap_or(0)))
        } else {
            None
        };
        for team in Team::ALL {
            // **16b cut O.3**: the targets' side is its pods 1 to `n`, each a
            // `Fielded` with **no seated pilot** — which is what makes a unit a
            // machine's (`crate::match_::start`: a unit mind's since 17b cut
            // M4.3, the reflex controller's before), so a target starts up and
            // then parks (17b cut M4.6b, the owner's STOP B ruling: once online
            // its mind's view reads a hull with no authored speed `immobile`,
            // and with no weapon that is Standard Doctrine's `Disabled` edge —
            // the reflex's inert rule's "start up, then stand") — and it is never empty:
            // a missing tank is the broken world `SimError::NoUnit` names, as for
            // a claimed slot. Neither `EmptySide` refusal below reads it.
            if let Some((targets, count)) = range
                && team == targets
            {
                let mut units: Vec<crate::match_::Fielded> = Vec::new();
                let mut minds: BTreeMap<ObjId, SealChoice> = BTreeMap::new();
                for index in 1..=count {
                    let tank =
                        centre::tank_for_slot(ctx.w, team, index).ok_or(SimError::NoUnit {
                            team: team.as_str(),
                            index,
                        })?;
                    units.push(crate::match_::Fielded {
                        unit: tank,
                        slot: index,
                        opening: openings.get(&(team, index)).copied(),
                        // 17b cut M5.1: a target chooses no mind, so the pool
                        // admits it by the fill rule (`fill_class`).
                        mind_class: None,
                    });
                    minds.insert(tank, SealChoice::unchosen());
                }
                if units.is_empty() {
                    return Err(SimError::EmptySide(team.as_str()));
                }
                sides.push(Side { team, units, minds });
                continue;
            }
            // **16b cut Q.1**: the pod index travels with the unit. It is the
            // only place in the tree that knows which slot fielded which tank, so
            // `Arena::new` can resolve `RA`, `BC` and a pilot and their teammates
            // call one machine by one name.
            //
            // **17b cut M5.1** (the design note's M5.1 item 6): each seated
            // slot's choice resolved against the holdings seam — a mind its
            // pilot no longer holds is `SimError::MindNotHeld`, naming the
            // slot and never the mind — into the side's `minds` and, for a
            // slot that chose, the class its mind seals at on the fitted
            // chassis (`Fielded::mind_class`); an unchosen hull is admitted
            // by the fill rule. Since 17b cut M5.2 the check-out seals each
            // hull with its entry (`crate::match_::start`).
            let mut units: Vec<crate::match_::Fielded> = Vec::new();
            let mut minds: BTreeMap<ObjId, SealChoice> = BTreeMap::new();
            for slot in centre.slots.iter().filter(|slot| slot.team == team) {
                let Some(unit) = slot.unit else {
                    continue;
                };
                let (mind_class, choice) = match &slot.mind {
                    None => (None, SealChoice::unchosen()),
                    Some(choice) => {
                        let held =
                            holdings::resolve(&holdings::held_by(ctx.w, slot.pilot), &choice.mind)
                                .cloned()
                                .ok_or(SimError::MindNotHeld {
                                    team: team.as_str(),
                                    index: Some(slot.index),
                                })?;
                        // **17b cut M5.2** (Q-UM-15, the owner's OQ-5): with
                        // humans on both sides a non-Doctrine choice is
                        // sealed as Standard Doctrine on Line (the fallback
                        // preset: no configuration exists, critic G10).
                        let choice = if centre.doctrine_only && !held.is_doctrine() {
                            SealChoice {
                                held: HeldMind::doctrine(),
                                profiles: Vec::new(),
                                chose: true,
                                substituted: true,
                            }
                        } else {
                            SealChoice {
                                held,
                                profiles: choice.profiles.clone(),
                                chose: true,
                                substituted: false,
                            }
                        };
                        let chassis =
                            fitted_type(ctx.w, unit).or_else(|| effective_type(ctx.w, slot));
                        let class =
                            sealed_class(ctx.w, &choice.held, chassis.as_deref(), fill_class);
                        (Some(class), choice)
                    }
                };
                units.push(crate::match_::Fielded {
                    unit,
                    slot: slot.index,
                    opening: openings.get(&(team, slot.index)).copied(),
                    mind_class,
                });
                minds.insert(unit, choice);
            }
            if (!match_cfg.ai_fill || range.is_some()) && units.is_empty() {
                return Err(SimError::EmptySide(team.as_str()));
            }
            // **16b cut N.1** (doc 16 §6: *the AI fill of a later lobby*), behind
            // `[tacsim.match].ai_fill`, **sized by 16b cut Q.7**. With it on, a
            // side is topped up to [`ai_side_size`] with the tanks of the lowest
            // unclaimed pods: the pods nobody claimed, and the pods of the slots
            // dropped at the boarding deadline. They arrive at the check-out with
            // **no seated pilot**, which is the one thing that makes them their
            // sealed minds' (`crate::match_::start`; the reflex controller's until
            // 17b cut M4.3, re-pointed by 17b cut M5.7) — the lobby names no
            // controller of its own, so the two halves cannot disagree about who
            // is flying what.
            //
            // **A side with nobody in it is no longer an abort on its own**, and
            // this comment used to say the opposite ("a match of eight machines is
            // a demonstration and not a match"). That ruling was written when no
            // human could be alone in a centre — both sides needed a pilot to
            // start at all — and the owner has now asked for exactly one human
            // against the fill. What replaces it is the same sentence read over
            // the *centre*: a match still needs somebody playing it, and `run`
            // refuses one that has nobody seated anywhere.
            //
            // Off by default, and `crate::config::MatchConfig::ai_fill` says why.
            //
            // **16b cut O.3**: and never on a range. The shooters' side is its
            // claimed slots ONLY — its unclaimed pods are left out whatever the
            // fill says, so a pilot on a range shoots at targets and nothing
            // shoots back.
            //
            // **16b cut AE.3**: the walk is [`fill_pods`], shared with `SIM
            // START`'s courtesy so the two cannot differ.
            //
            // **17b cut M5.1**: the side's fill choice, when it fields a pod,
            // resolved against the holdings of the lead who set it (staff's is
            // Standard Doctrine) — `SimError::MindNotHeld` naming the fill
            // when it no longer resolves — and each fill hull's class read on
            // its own chassis.
            if match_cfg.ai_fill && range.is_none() {
                let fielded: Vec<ObjId> = units.iter().map(|fielded| fielded.unit).collect();
                let pods = fill_pods(ctx.w, centre, &match_cfg, team, &fielded);
                let fill = match centre.fill_minds.iter().find(|fill| fill.team == team) {
                    Some(fill) if !pods.is_empty() => Some(fill_seal_choice(
                        &fill_held(ctx.w, centre, fill).ok_or(SimError::MindNotHeld {
                            team: team.as_str(),
                            index: None,
                        })?,
                        fill,
                        centre.doctrine_only,
                    )),
                    _ => None,
                };
                for (index, tank) in pods {
                    // 16b cut Q.1: an AI-filled pod carries its own index too, so
                    // a machine in red pod 3 is `RC`, exactly as a pilot in that
                    // pod would be.
                    let (mind_class, choice) = match &fill {
                        None => (None, SealChoice::unchosen()),
                        Some(choice) => {
                            let chassis = fitted_type(ctx.w, tank)
                                .or_else(|| authored_unit_type(ctx.w, team, index));
                            (
                                Some(sealed_class(
                                    ctx.w,
                                    &choice.held,
                                    chassis.as_deref(),
                                    fill_class,
                                )),
                                choice.clone(),
                            )
                        }
                    };
                    units.push(crate::match_::Fielded {
                        unit: tank,
                        slot: index,
                        opening: openings.get(&(team, index)).copied(),
                        mind_class,
                    });
                    minds.insert(tank, choice);
                }
            }
            // A side with no unit at all is still a side nobody can fight: with
            // the fill on it means the centre's tanks are missing (K.1 deviation
            // 1), which is the same broken world `SimError::NoUnit` names.
            if units.is_empty() {
                return Err(SimError::EmptySide(team.as_str()));
            }
            // **16b cut AE.3** (plan §6 AE-X3): and a side whose final machines
            // carry no gun between them — a lone drone, after the deadline dropped
            // its tank's pilot — is refused here, where the sides are final. `run`
            // calls it off: `Match N could not start: The red side fields no gun;
            // …`. The range's target side never reaches this line (`continue`
            // above).
            let types: Vec<SmolStr> = units
                .iter()
                .filter_map(|fielded| fitted_type(ctx.w, fielded.unit))
                .collect();
            side_fields_a_gun(ctx.w, team, &types)?;
            sides.push(Side { team, units, minds });
        }
    }
    Ok(Ready {
        arena,
        sides,
        arena_cfg,
        match_cfg,
        sensors_cfg,
        mode,
    })
}

/// **The boarding deadline** — `match.board_timeout_ms` elapsed with somebody
/// unseated (doc 16 §3.8).
///
/// The unseated slots are dropped and their pods freed; if that empties a side
/// the match aborts, and otherwise the check-out fires with whoever is in.
/// Driven by the module's coarse tick, so it is a due world tick and not a timer
/// inside the arena — there is no arena yet.
pub fn board_deadline(ctx: &mut Ctx<'_>, now_ms: i64) {
    // **16b cut F.4 residual**: the layer, before the clock. Every route into
    // `Boarding` asks [`require_layer`] first, so a centre on a server with the
    // layer off is `Idle` and this would find nothing anyway — but the tick that
    // calls it is registered unconditionally, and a deadline that ran the eject
    // and abort machinery of a layer nobody can reach would be a second opinion
    // about whether the simulator is on. There is one, and it is this.
    if require_layer(ctx.w).is_err() {
        return;
    }
    let Some(match_id) = ({
        let Some(centre) = centre(ctx.w) else {
            return;
        };
        if centre.state != MatchState::Boarding {
            return;
        }
        match centre.board_deadline_ms {
            Some(deadline) if deadline <= now_ms => centre.match_id,
            _ => return,
        }
    }) else {
        return;
    };
    let dropped = {
        let Some(book) = ctx.w.module_state_mut::<MatchBook>() else {
            return;
        };
        let centre = book.centre_mut();
        let dropped: Vec<(ObjId, Team, u8)> = centre
            .slots
            .iter()
            .filter(|slot| !slot.is_seated())
            .map(|slot| (slot.pilot, slot.team, slot.index))
            .collect();
        centre.slots.retain(Slot::is_seated);
        // 16b cut AA.2 (`B-78`): a dropped lead hands the side to its first
        // seated claimant, or `SIM CONCEDE` is nobody's for the whole match.
        centre.resettle_leads();
        centre.board_deadline_ms = None;
        dropped
    };
    for (pilot, team, index) in &dropped {
        let name = display_name(ctx.w, *pilot);
        announce(
            ctx,
            format!(
                "{name} never boarded; {} slot {index} is dropped.",
                team.as_str()
            ),
        );
        // **16b cut Q.8 (fixer): the choice goes with the slot.** A pilot dropped
        // here chose their machine at `Selection` and `SIM START` fitted it; the
        // pod they never boarded is about to be flown by the fill or left out of
        // the match entirely, and either way what stands in it should be the pod's
        // own tank again. Silent when they chose nothing, which is every drop in
        // the project's life so far.
        unfit_slot(ctx, *team, *index);
    }
    // **16b cut Q.7**: with the fill on the deadline asks about the centre and not
    // about a side — a side that boarded nobody is a side the machines fly, and
    // only a centre that boarded nobody is an abort.
    //
    // **16b cut O.3** (plan item 13): and under a range the targets' side never
    // boards anybody and is never empty, so the question is the shooters' side,
    // which is every slot of the centre, fill or no fill.
    let fill = configs(ctx.w).is_some_and(|(_, match_cfg, _)| match_cfg.ai_fill);
    let range = centre(ctx.w).is_some_and(|centre| centre.mode == MatchMode::Range);
    let empty = if fill || range {
        centre(ctx.w)
            .is_some_and(|centre| centre.slots.is_empty())
            .then_some(None)
    } else {
        Team::ALL
            .iter()
            .find(|team| centre(ctx.w).is_some_and(|centre| centre.side_len(**team) == 0))
            .copied()
            .map(Some)
    };
    if let Some(team) = empty {
        // 16b cut D.3 (`B-32`): through the one pre-`Running` funnel.
        call_off(
            ctx,
            match team {
                Some(team) => format!(
                    "Match {match_id} is aborted: the {} side boarded nobody.",
                    team.as_str()
                ),
                None => format!("Match {match_id} is aborted: nobody boarded."),
            },
        );
        return;
    }
    if let Err(error) = run(ctx, match_id) {
        tracing::warn!(match_id = match_id.0, %error, "tacsim: a boarding deadline could not start its match");
    }
}

// ---------------------------------------------------------------------------
// `SIM AI` — how many machines a side fields (16b cut Q.7)
// ---------------------------------------------------------------------------

/// **`SIM AI [<n>]`** — read or set how many units each side fields with the AI
/// fill on (16b cut Q.7).
///
/// **Setting** it is a team lead's or staff's, in `Selection` only: the fill is
/// decided before the pods lock, because it is what the pods are locked *for*.
/// `Some(n)` sets, and is announced to the centre the way a claim is.
///
/// **Reading it is anybody's** (16b cut D.1). `None` answers `AI fill: {n} a
/// side.` at once, in any state, to anybody standing in the centre, and only the
/// layer, the location and the server's own `ai_fill` switch come first. It is
/// the number the board already prints to the whole room.
///
/// The number is a **side size**, not a machine count: `SIM AI 3` is a three a
/// side match, so a side with two pilots gets one machine and a side with none
/// gets three.
///
/// # Errors
///
/// [`SimError`], as the line the player sees:
/// [`SimError::AiOff`] on a server with `[tacsim.match].ai_fill` off (both the
/// read and the set); **under a range** (16b cut O.3) the read answers `AI
/// fill: none on a range.` and the set is [`SimError::AiOnRange`]; and then, for
/// the set alone,
/// [`SimError::WrongState`] outside `Selection`, [`SimError::NotLeadForAi`] for
/// anyone but a side's lead or staff, [`SimError::AiRange`] outside
/// `1..=slots_per_side`, and [`SimError::AiBelowSide`] under what a side has
/// already claimed.
pub fn ai(ctx: &mut Ctx<'_>, size: Option<u8>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let staff = is_staff(ctx.w, actor);
    let account = match account_of(ctx.w, actor) {
        Ok(account) => Some(account),
        Err(_) if staff => None,
        Err(error) => return Err(error),
    };
    let (_, match_cfg, _) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    // The server fact first: a player on a box with no fill is owed that and not
    // a lecture about who may set it.
    if !match_cfg.ai_fill {
        return Err(SimError::AiOff);
    }
    let cap = slots_per_side(ctx.w, &match_cfg);
    // **16b cut O.3** (16c N13; plan item 39): a range fields no machines — its
    // other side's pods are the targets' — so the number the fill would read is
    // not the number any match here will field. D.1's rule is that the text be
    // true on the server it is said on: the read says so, and a set is refused
    // rather than stored for nothing.
    if centre(ctx.w).is_some_and(|centre| centre.mode == MatchMode::Range) {
        if size.is_some() {
            return Err(SimError::AiOnRange);
        }
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            "AI fill: none on a range.".to_owned(),
        );
        return Ok(());
    }
    // **16b cut D.1**: the bare read answers anybody standing in the centre, in
    // any state, lead or not. It is one number about the server's own settings —
    // the same number the board's own `AI fill:` line prints to everyone in the
    // room — and refusing it with `SIM START`'s sentence told a player who asked
    // a question that they could not start a match.
    let Some(size) = size else {
        let centre = centre(ctx.w).ok_or(SimError::Unavailable)?;
        let effective = ai_side_size(ctx.w, centre, &match_cfg);
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("AI fill: {effective} a side."),
        );
        return Ok(());
    };
    // Setting it is what the state and the lead gate are for, and they are
    // unchanged.
    let claimed = {
        let centre = centre(ctx.w).ok_or(SimError::Unavailable)?;
        if centre.state != MatchState::Selection {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let lead = account.is_some_and(|account| {
            Team::ALL
                .iter()
                .any(|team| centre.leads_side(*team, account))
        });
        if !staff && !lead {
            // 16b cut D.1: `SIM AI`'s own sentence. `NotLead` is `SIM START`'s.
            return Err(SimError::NotLeadForAi);
        }
        largest_side(centre)
    };
    if size < 1 || size > cap {
        return Err(SimError::AiRange { max: cap });
    }
    if size < claimed {
        return Err(SimError::AiBelowSide { claimed });
    }
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.centre_mut().ai_side_size = Some(size);
    }
    announce(ctx, format!("AI fill set to {size} a side."));
    Ok(())
}

// ---------------------------------------------------------------------------
// `SIM MODE` — which rule the match is fought under (16b cut O.1; 16c N11)
// ---------------------------------------------------------------------------

/// **The words of a ground's Hold rule** (16b cut O.1): the key spot's authored
/// name and map letter, the radius and the time — everything `SIM MODE`, the
/// board and the terminal head say about it, and all of it public ground (doc
/// 16 §3.2: the spot's name and letter are on every pilot's map legend).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoldWords {
    /// The key spot's authored name (`The Crossroads`).
    pub spot: SmolStr,
    /// Its map letter (`B`).
    pub letter: char,
    /// The half-side of the square, cells.
    pub radius_cells: u16,
    /// The time a side must be alone on it, milliseconds.
    pub hold_ms: u32,
}

impl HoldWords {
    /// The time in whole seconds — the loader holds `hold_ms` to whole seconds.
    #[must_use]
    pub const fn seconds(&self) -> u32 {
        self.hold_ms / 1_000
    }
}

/// **The centre's ground's Hold rule, in words** (16b cut O.1), read off the
/// loaded `ArenaDef` in `TacsimStore.definitions().arenas` — the compiled-in
/// catalog, with no window built — and [`crate::match_::cockpit::spot_letter`].
/// `None` for a world whose catalog has no centre arena or whose arena declares
/// no Hold.
#[must_use]
pub fn hold_words(w: &World) -> Option<HoldWords> {
    let arena = mush_core::CanonicalId::new(CENTRE_ARENA).ok()?;
    let def = w
        .module_state::<crate::TacsimStore>()?
        .definitions()
        .arenas
        .get(&arena)?;
    def.arena_modes()
        .into_iter()
        .find_map(|declared| match declared {
            crate::arena::state::ArenaMode::Hold(rule) => {
                let index = usize::from(rule.spot);
                let spot = def.key_spots.get(index)?;
                Some(HoldWords {
                    spot: SmolStr::new(&spot.name),
                    letter: crate::match_::cockpit::spot_letter(index)?,
                    radius_cells: rule.radius_cells,
                    hold_ms: rule.hold_ms,
                })
            }
        })
}

/// The words of the modes the centre's ground declares, in authored order —
/// `skirmish` first, which every ground supports and none declares.
fn declared_mode_words(w: &World) -> Vec<&'static str> {
    let mut words = vec![MatchMode::Skirmish.as_str()];
    let declared = mush_core::CanonicalId::new(CENTRE_ARENA)
        .ok()
        .and_then(|arena| {
            w.module_state::<crate::TacsimStore>()?
                .definitions()
                .arenas
                .get(&arena)
                .map(crate::definitions::arena::ArenaDef::arena_modes)
        })
        .unwrap_or_default();
    for mode in declared {
        let word = mode.mode().as_str();
        if !words.contains(&word) {
            words.push(word);
        }
    }
    words
}

/// **`SIM MODE [skirmish|hold]`** — read or set which rule the next match is
/// fought under (16b cut O.1; 16c N11).
///
/// **16b cut O.3**: the range is not one of its words. `SIM MODE range` is
/// refused [`SimError::RangeViaRange`] (a range needs a count, which is
/// `SIM RANGE <n>`'s), the read ends ` SIM RANGE sets a range.`, and setting
/// `hold` or `skirmish` leaves a range, clearing [`Centre::range_targets`] with
/// the mode.
///
/// **Reading it is anybody's**, in any state, slot or no slot (the `SIM AI` and
/// `SIM UNIT` precedent, 16b cut D.1): `Mode: {current}. {label} supports:
/// {words}.`, to the actor alone — the current mode, the ground's authored name
/// and the words it supports, `skirmish` first — and, while the mode is Hold,
/// ` Hold: keep {spot} ({letter}) uncontested for {s} seconds to win.`
///
/// **Setting it** is a side's lead's or staff's, in `Selection` only, to a mode
/// the ground declares; it is announced to every centre room the way `SIM UNIT`
/// is: `Mode set to hold: keep {spot} ({letter}) uncontested for {s} seconds to
/// win; elimination still ends it.` or `Mode set to skirmish: eliminate the other
/// side to win.` The mode is public in selection; who holds the spot during a
/// match is told to nobody.
///
/// # Errors
///
/// [`SimError`], as the line the player sees. The read refuses only with
/// [`SimError::LayerOff`], [`SimError::OutsideCentre`] and
/// [`SimError::Unavailable`]; a set adds [`SimError::UnknownAccount`] for a
/// non-staff character with no account, [`SimError::WrongState`] outside
/// `Selection`, [`SimError::NotLeadForMode`], [`SimError::NoSuchMode`] and
/// [`SimError::ModeUnsupported`].
pub fn mode(ctx: &mut Ctx<'_>, choice: Option<&str>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let current = centre(ctx.w).ok_or(SimError::Unavailable)?.mode;
    let label = crate::match_::record::arena_label(ctx.w, CENTRE_ARENA);
    let hold = hold_words(ctx.w);
    // The bare read: anybody in the centre, in any state.
    let Some(word) = choice.map(str::trim).filter(|word| !word.is_empty()) else {
        let words = declared_mode_words(ctx.w).join(", ");
        let mut line = format!("Mode: {}. {label} supports: {words}.", current.as_str());
        if current == MatchMode::Hold
            && let Some(hold) = &hold
        {
            line.push_str(&format!(
                " Hold: keep {} ({}) uncontested for {} seconds to win.",
                hold.spot,
                hold.letter,
                hold.seconds()
            ));
        }
        // 16b cut O.3: the range is the one mode this list does not carry.
        line.push_str(" SIM RANGE sets a range.");
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        return Ok(());
    };
    // Setting it: `Selection`, a lead or staff, a mode word, a mode the ground
    // declares — in that order, `SIM AI`'s.
    let staff = is_staff(ctx.w, actor);
    let account = match account_of(ctx.w, actor) {
        Ok(account) => Some(account),
        Err(_) if staff => None,
        Err(error) => return Err(error),
    };
    {
        let centre = centre(ctx.w).ok_or(SimError::Unavailable)?;
        if centre.state != MatchState::Selection {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let lead = account.is_some_and(|account| {
            Team::ALL
                .iter()
                .any(|team| centre.leads_side(*team, account))
        });
        if !staff && !lead {
            return Err(SimError::NotLeadForMode);
        }
    }
    let chosen = MatchMode::parse(word).ok_or_else(|| SimError::NoSuchMode {
        word: word.to_owned(),
    })?;
    // The announcement first, because its one refusal comes before the support
    // test: **16b cut O.3**, the range is `SIM RANGE <n>`'s alone.
    let announcement = match (chosen, hold) {
        (MatchMode::Range, _) => return Err(SimError::RangeViaRange),
        (MatchMode::Hold, Some(hold)) => format!(
            "Mode set to hold: keep {} ({}) uncontested for {} seconds to win; elimination \
             still ends it.",
            hold.spot,
            hold.letter,
            hold.seconds()
        ),
        (MatchMode::Hold, None) => "Mode set to hold; elimination still ends it.".to_owned(),
        (MatchMode::Skirmish, _) => {
            "Mode set to skirmish: eliminate the other side to win.".to_owned()
        }
    };
    if !declared_mode_words(ctx.w).contains(&chosen.as_str()) {
        return Err(SimError::ModeUnsupported {
            mode: chosen.as_str(),
            arena: label,
        });
    }
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        let centre = book.centre_mut();
        centre.mode = chosen;
        // 16b cut O.3: leaving a range takes its count with it (the invariant on
        // `Centre::range_targets`).
        centre.range_targets = None;
    }
    announce(ctx, announcement);
    Ok(())
}

// ---------------------------------------------------------------------------
// `SIM RANGE` — one side against targets (16b cut O.3; 16c N13)
// ---------------------------------------------------------------------------

/// **`SIM RANGE [<n>]`** — read or set up a range: the claimed side against
/// `n` targets that neither move nor shoot, standing on the other side's pods 1
/// to `n` (16b cut O.3; 16c N13).
///
/// **The bare word always reads** — anybody standing in the centre, in any
/// state, slot or no slot, to the actor alone (D.1's rule for `SIM AI`):
/// `Range: {n} targets on the {team} side.` while the mode is a range (`Range:
/// {n} targets.` in a terminal state, where no slot is left to read a side
/// off), else `The range is not set; SIM RANGE <n> sets it in selection.`
/// Every count reads `1 target` for one (`targets_counted`), and the
/// announcement's verb with it (`1 target stands`).
///
/// **A number sets it**: in `Selection`, by a side's lead or staff, with every
/// claimed slot on one side, `n` from 1 to [`slots_per_side`]. It writes
/// [`Centre::mode`] and [`Centre::range_targets`] together and announces to
/// every centre room `Range set: {n} targets stand on the {other} side; the
/// match ends when every target is out of action, or on SIM ABORT.` `SIM MODE
/// hold` or `SIM MODE skirmish` leaves it.
///
/// What a range is, once set: the targets' side is never empty for the four
/// one-sided gates (`start`, `settle_boarding`, `board_deadline`, `ready`), so
/// it starts on a server with the fill off; `SIM START` refits the targets'
/// pods to `tacsim.unit.target_mk1` (`fit_fielded_units`); the shooters' own
/// unclaimed pods are never filled; and the match ends by the ordinary
/// elimination rule or on `SIM ABORT` — `spent` never ends it.
///
/// # Errors
///
/// [`SimError`], as the line the player sees. The read refuses only with
/// [`SimError::LayerOff`], [`SimError::OutsideCentre`] and
/// [`SimError::Unavailable`]; a set adds [`SimError::UnknownAccount`] for a
/// non-staff character with no account, [`SimError::WrongState`] outside
/// `Selection`, [`SimError::NotLeadForRange`], [`SimError::RangeTwoSides`] and
/// [`SimError::RangeCount`], in that order.
pub fn range(ctx: &mut Ctx<'_>, count: Option<u8>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    // The bare read: anybody in the centre, in any state.
    let Some(count) = count else {
        let centre = centre(ctx.w).ok_or(SimError::Unavailable)?;
        let line = match centre
            .range_targets
            .filter(|_| centre.mode == MatchMode::Range)
        {
            Some(count) => match range_sides(centre) {
                Some((_, targets)) => format!(
                    "Range: {} on the {} side.",
                    targets_counted(count),
                    targets.as_str()
                ),
                // A terminal centre: the rows are released and no side is left
                // to read the targets' off, so the line names none.
                None => format!("Range: {}.", targets_counted(count)),
            },
            None => "The range is not set; SIM RANGE <n> sets it in selection.".to_owned(),
        };
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        return Ok(());
    };
    // Setting it: `Selection`, a lead or staff, one side, a count — in that
    // order, `SIM AI`'s and `SIM MODE`'s.
    let staff = is_staff(ctx.w, actor);
    let account = match account_of(ctx.w, actor) {
        Ok(account) => Some(account),
        Err(_) if staff => None,
        Err(error) => return Err(error),
    };
    let (_, match_cfg, _) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    let cap = slots_per_side(ctx.w, &match_cfg);
    let targets = {
        let centre = centre(ctx.w).ok_or(SimError::Unavailable)?;
        if centre.state != MatchState::Selection {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let lead = account.is_some_and(|account| {
            Team::ALL
                .iter()
                .any(|team| centre.leads_side(*team, account))
        });
        if !staff && !lead {
            return Err(SimError::NotLeadForRange);
        }
        if Team::ALL.iter().all(|team| centre.side_len(*team) > 0) {
            return Err(SimError::RangeTwoSides);
        }
        // `Selection` is only reachable with a slot in it, so there is always a
        // side to read; the fallback is a guard and not a path.
        let shooters = centre
            .slots
            .first()
            .map(|slot| slot.team)
            .ok_or(SimError::NobodyPlaying)?;
        shooters.other()
    };
    if count < 1 || count > cap {
        return Err(SimError::RangeCount { max: cap });
    }
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        let centre = book.centre_mut();
        centre.mode = MatchMode::Range;
        centre.range_targets = Some(count);
    }
    announce(
        ctx,
        format!(
            "Range set: {} {} on the {} side; the match ends when every target is out of \
             action, or on SIM ABORT.",
            targets_counted(count),
            if count == 1 { "stands" } else { "stand" },
            targets.as_str()
        ),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The ends
// ---------------------------------------------------------------------------

/// **`SIM ABORT`** — by any participant or staff, from any live state (doc 16
/// §3.8).
///
/// `Selection` and `Boarding` release slots and pods only: there is no custody
/// to return. `Running` sends `Stop { Abort }`, and cut T.3's check-in follows
/// when the arena answers.
///
/// **16b cut D.3 (`T-60`) took `Selection` away from the participants.** There is
/// no match in `Selection` — no id, no pod lock, nothing checked out — and one
/// claimant's `SIM ABORT` wiped every other claimant's slot with it. The lobby's
/// own way out is `SIM LEAVE`, which gives up the one thing a claimant holds and
/// (when they are the last) returns the centre to `Idle` anyway; so a non-staff
/// `SIM ABORT` in `Selection` is [`SimError::NothingToAbort`]. `Boarding`,
/// `Running` and `Stopping` are unchanged and stay any participant's
/// (doc 16 §3.8), because in all three there is a match with a name.
///
/// **16b cut M.1** (`B-49`): an arena that cannot take the stop this instant is
/// [`SimError::ArenaBusy`], with nothing changed and nothing announced.
///
/// # Errors
///
/// [`SimError`], as the line the player sees.
pub fn abort(ctx: &mut Ctx<'_>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let staff = is_staff(ctx.w, actor);
    let participant = centre(ctx.w).is_some_and(|centre| centre.slot_of_pilot(actor).is_some());
    let (state, match_id) = {
        let Some(centre) = centre(ctx.w) else {
            return Err(SimError::Unavailable);
        };
        (centre.state, centre.match_id)
    };
    if !state.may_abort() {
        return Err(SimError::WrongState {
            state: state.as_str(),
        });
    }
    if !staff && !participant {
        return Err(SimError::NoSlot);
    }
    // 16b cut D.3 (`T-60`): the state check, after the slot check, so a bystander
    // in `Selection` still reads the refusal that is about them.
    if !staff && state == MatchState::Selection {
        return Err(SimError::NothingToAbort);
    }
    let name = display_name(ctx.w, actor);
    match (state, match_id) {
        (MatchState::Running | MatchState::Stopping, Some(match_id)) => {
            // 16b cut M.1 (`B-49`): a busy arena changes nothing and says nothing
            // to the centre; the one who typed it is told to try again.
            if crate::match_::abort(ctx, match_id, StopReason::Abort)
                == crate::match_::AbortOutcome::Busy
            {
                return Err(SimError::ArenaBusy);
            }
            announce(
                ctx,
                format!(
                    "{name} aborts match {match_id}. Its units check back in when the arena \
                     answers."
                ),
            );
        }
        // 16b cut D.3 (`B-32`): through the one pre-`Running` funnel, so a lobby
        // that chose a machine with `SIM UNIT` does not leave it standing.
        _ => call_off(ctx, format!("{name} calls the match off.")),
    }
    Ok(())
}

/// **`SIM CONCEDE`** — a side gives the match up (16b cut D.3, `B-33`; doc 16
/// §3.8).
///
/// The eighth selector of the one `SIM` verb, and the answer to a match that
/// cannot end: doc 16 §3.8's elimination is *a wreck, or disabled with no
/// weapons*, so a side that is beaten but not dead has nothing to do but drive
/// about until `match.max_duration_ms`. `SIM ABORT` is the wrong word for it —
/// an abort is a match that did not happen, and this one did — so a concede is a
/// **result**: [`crate::arena::state::EndReason::Conceded`], the other side's
/// own outcome word on the record, and the cells committed like any other end.
///
/// Guards in this order: the layer, the room, `Running`, a slot, and the side's
/// **lead**. Staff have `@arena/abort` and get no special case here (see
/// [`SimError::NotLeadToConcede`]).
///
/// The stop goes through [`crate::match_::abort`] — this crate's one stop funnel,
/// whatever the reason — and the check-in, the result document and the terminal
/// board then follow exactly as they do for an elimination. A match whose task is
/// already gone is finished there and then as an `Abort`, and the concede sentence
/// is **not** said over it (see the body). An arena that cannot take the stop this
/// instant is [`SimError::ArenaBusy`] (16b cut M.1, `B-49`): no concede is
/// recorded and nothing is announced.
///
/// # Errors
///
/// [`SimError`], as the line the player sees.
pub fn concede(ctx: &mut Ctx<'_>) -> Result<(), SimError> {
    let actor = ctx.actor;
    require_layer(ctx.w)?;
    in_centre(ctx.w, actor)?;
    let (team, match_id) = {
        let Some(centre) = centre(ctx.w) else {
            return Err(SimError::Unavailable);
        };
        if centre.state != MatchState::Running {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            });
        }
        let slot = centre.slot_of_pilot(actor).ok_or(SimError::NoSlot)?;
        if !centre.leads_side(slot.team, slot.account) {
            return Err(SimError::NotLeadToConcede);
        }
        // A `Running` centre the lobby started always has one; the `?` is the
        // broken-world arm and not a path.
        (slot.team, centre.match_id.ok_or(SimError::Unavailable)?)
    };
    // 16b cut M.1 (`B-49`): a busy arena records no concede and announces
    // nothing; the lead is told to try again, and the match is still running.
    if crate::match_::abort(ctx, match_id, StopReason::Concede { by: team })
        == crate::match_::AbortOutcome::Busy
    {
        return Err(SimError::ArenaBusy);
    }
    // **The sentence belongs to a stop that is really in flight** (16b cut D.3).
    // The funnel finishes a match whose task is already gone **on the spot**, as
    // an `EndReason::Abort`, and [`on_match_finished`] then says `Match {id} is
    // over (abort).` to the centre and leaves it terminal. Announcing the concede
    // over the top of that would tell everybody a side had won a match whose
    // board, record and result document all say `abort`, with no arena left to
    // come back and correct it. So the centre is asked whether it is still
    // running: a match with no arena has no result to concede, and nothing more
    // to say. (The funnel's own answer could say so too — `Finished` — but the
    // centre's state is the fact the sentence is about, so it is what is read.)
    if centre(ctx.w).is_none_or(|centre| centre.state != MatchState::Running) {
        return Ok(());
    }
    let name = display_name(ctx.w, actor);
    announce(
        ctx,
        format!(
            "{name} concedes for the {} side. Match {match_id} goes to {} when the arena answers.",
            team.as_str(),
            team.other().as_str()
        ),
    );
    Ok(())
}

/// **The one pre-`Running` end** (16b cut D.3, `B-32`, `T-50`).
///
/// Cut F.1 gave [`release`] the seat sweep and made it the one funnel into
/// `Aborted`; this is the rest of that sentence. Three things, in this order:
///
/// 1. [`release`] — the pods, the bodies, the billets and the rows, and the
///    centre into `Aborted` carrying [`MatchOutcome::Abort`];
/// 2. [`crate::match_::reset_stock_tanks`] — because a `SIM UNIT` refit is fitted
///    at `SIM START` and a lobby that ends before `Running` would otherwise leave
///    a biped standing in a pod nobody chose it for. Idempotent and silent when
///    nothing moved, which is every call in a lobby where nobody chose anything.
///    **The evidence argument that keeps this reset away from a finished match
///    does not apply here**: nothing was fought, so there is nothing to read;
/// 3. the sentence, said to the whole centre.
///
/// Every pre-`Running` end is one of these: [`abort`]'s `Selection`/`Boarding`
/// arm, [`settle_boarding`]'s empty arm, [`board_deadline`]'s empty arm, and both
/// of [`run`]'s refusal arms.
fn call_off(ctx: &mut Ctx<'_>, why: String) {
    release(ctx, MatchState::Aborted, Some(MatchOutcome::Abort));
    crate::match_::reset_stock_tanks(ctx, &std::collections::BTreeSet::new());
    announce(ctx, why);
}

/// **The unattended deadline** — `[tacsim.match].unattended_ms` of world time has
/// passed since every pilot of a `Running` match left it (16b cut D.3, `B-33`).
///
/// Driven by the lobby's own 1 s tick beside [`board_deadline`] and behind the
/// same `require_layer` test, for the same reason: a simulator that is switched
/// off does not stop matches.
///
/// **The stamp is cleared, and the sentence said, only when the offer lands.**
/// Nothing ever re-stamps, because `EXIT` is one-way — so clearing on a failed
/// send would disarm this path for the rest of the match, in exactly the long
/// lagging fight that most needs it. A
/// [`crate::arena_bridge::ControlRefusal::Busy`] — a momentarily full inbox, or
/// an `Ended` already on its way (16b cut M.1, `B-49`) — therefore leaves the
/// stamp for the next tick exactly as a `Gone` or an `Unavailable` does, and
/// warns **once** per match; a `NoMatch` clears it silently, because the match it
/// names is already over.
pub fn unattended_deadline(ctx: &mut Ctx<'_>, now_ms: i64) {
    if require_layer(ctx.w).is_err() {
        return;
    }
    let Some((match_id, since, warned)) = ({
        let Some(centre) = centre(ctx.w) else {
            return;
        };
        if centre.state != MatchState::Running {
            return;
        }
        match (centre.match_id, centre.unattended_since_ms) {
            (Some(match_id), Some(since)) => Some((match_id, since, centre.unattended_warned)),
            _ => None,
        }
    }) else {
        return;
    };
    let Some((_, match_cfg, _)) = configs(ctx.w) else {
        return;
    };
    // `0` is off, and it is read here rather than at the stamp so an operator who
    // turns the key off under a live match turns this off with it.
    if match_cfg.unattended_ms == 0
        || now_ms.saturating_sub(since) < i64::from(match_cfg.unattended_ms)
    {
        return;
    }
    match crate::arena_bridge::send_stop(ctx, match_id, StopReason::Unattended) {
        Ok(()) => {
            // The record parks in `Stopping` while the stop is in flight, exactly
            // as `crate::match_::abort` parks it, so `@arena/status` says what is
            // happening and a second stop is harmless.
            if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
                book.centre_mut().clear_unattended();
                if let Some(record) = book.get_mut(match_id) {
                    record.state = MatchState::Stopping;
                }
            }
            // **16b cut AF.2 (16c N16)**: and the live record says so at once,
            // as `crate::match_::abort` rewrites it — an unattended stop in
            // flight at a crash is found by the boot as `stopping` and voided,
            // never undone by a resume that would hand a match every pilot had
            // left back to nobody (16b cut AF.3).
            crate::match_::write_live(ctx, match_id);
            announce(ctx, unattended_line(match_id));
        }
        // The match is already gone: there is nothing to stop and nothing to say.
        Err(crate::arena_bridge::ControlRefusal::NoMatch) => {
            if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
                book.centre_mut().clear_unattended();
            }
        }
        Err(error) => {
            if !warned {
                tracing::warn!(
                    match_id = match_id.0,
                    %error,
                    "tacsim: a match every pilot has left would not take its stop; the lobby \
                     retries every second"
                );
                if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
                    book.centre_mut().unattended_warned = true;
                }
            }
        }
    }
}

/// **The sentence an unattended end says to the whole centre** (16b cut D.3,
/// `B-33`).
///
/// A function rather than a `format!` at the one call site so the observer rule's
/// own lens (`crates/mushd/tests/observer_rule.rs`) can sweep the **production**
/// sentence for a unit, a position or a contact instead of a copy of it kept in a
/// test file, which is a copy that can silently stop matching what the centre
/// really says. Doc 16 §1: it is built from the match id and nothing else, and
/// every board already publishes that.
#[must_use]
pub fn unattended_line(match_id: MatchId) -> String {
    format!("Match {match_id} ends: every pilot has left it.")
}

/// **What one reset did** (16b cut F.1).
#[derive(Clone, Debug, Default)]
pub struct ResetOutcome {
    /// Whether the centre was already `Idle` when the reset ran.
    pub was_idle: bool,
    /// The pods opened and the billets given back.
    pub ejected: crate::match_::cockpit::EjectReport,
    /// The stock tanks put back at their spec and at their authored slot.
    pub tanks: crate::match_::ResetReport,
}

impl ResetOutcome {
    /// Whether anything at all changed — what lets `@arena/reset` say "nothing
    /// changed" on a second run rather than pretending it did something.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.was_idle || !self.ejected.is_empty() || self.tanks.changed()
    }

    /// **Whether the reset found nothing to say at all** (16b cut D.3, `B-31`) —
    /// nothing changed **and** nothing was skipped or refused.
    ///
    /// It is [`ResetOutcome::changed`] with
    /// [`crate::match_::ResetReport::deployed`] and
    /// [`crate::match_::ResetReport::failed`] folded in, through
    /// `ResetReport::is_empty`, which existed with no caller. The two questions
    /// are not the same one: a pass over a roster tank that is still deployed
    /// writes nothing and has a great deal to say, and `@arena/reset`'s headline
    /// used to answer the first question while printing the answer to the second
    /// underneath it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.was_idle && self.ejected.is_empty() && self.tanks.is_empty()
    }
}

/// **The reset to `Idle`** — idempotent, and re-runnable by staff as
/// `@arena/reset` (doc 16 §3.8).
///
/// The **one funnel** the centre returns to `Idle` through, and therefore the one
/// place doc 16 §3.3's *at every `Idle` transition each is reset to its
/// `UnitSpec`* can live. Three things, in this order:
///
/// 1. the **seat sweep**, before anything else and **before the already-idle
///    early return**. A restart that could not resume the match (16b cut AF.3
///    voids it by name; a resumed match's centre comes back `Running` from its
///    live record) leaves the centre `Idle` while a body may still be standing
///    in a pod and a billet still held on its tank, and the old `reset`
///    returned on the first line in
///    exactly that case — so the staff repair the design pointed at was a no-op
///    on the one path that most needed it;
/// 2. the **tank reset** ([`crate::match_::reset_stock_tanks`]), which is what
///    makes a second match a second match rather than a fight between last
///    match's wreck and last match's survivor;
/// 3. the centre's own rows, when it was not already idle.
///
/// Answers [`ResetOutcome`], so a caller can say what it freed.
pub fn reset(ctx: &mut Ctx<'_>) -> ResetOutcome {
    let was_idle = centre(ctx.w).is_some_and(|centre| *centre == Centre::default());
    let ejected = crate::match_::cockpit::eject_all(ctx);
    let tanks = crate::match_::reset_stock_tanks(ctx, &std::collections::BTreeSet::new());
    if !was_idle {
        to_idle(ctx);
    }
    ResetOutcome {
        was_idle,
        ejected,
        tanks,
    }
}

/// **The one writer of `Idle`** (16b cut D.3, `T-48`).
///
/// [`release_rows`] moves the centre to the state it is given and clears what
/// belongs to a match; the assignment behind it is what clears the rest, so the
/// centre really equals [`Centre::default`] and not merely something close to it.
/// Both halves are needed and neither is anybody's on its own, so they are one
/// function.
///
/// Two callers: [`reset`], and the last [`leave`] — which used to zero the struct
/// in place, a second writer of the same transition that could (and, as fields
/// were added to [`Centre`], silently did) mean something different from this
/// one.
fn to_idle(ctx: &mut Ctx<'_>) {
    release_rows(ctx, MatchState::Idle, None);
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        *book.centre_mut() = Centre::default();
    }
}

/// **The centre's half of an ended match** — called from
/// [`crate::match_::finish`] once custody is back.
///
/// The seats, leases and focus are released by the check-in itself (they belong
/// to the units, not to the lobby); what this releases is the lobby's own half,
/// the pods and the slots, in the same authoritative action. The centre is left
/// in its terminal state carrying the outcome so `SIM STATUS` can say how it
/// went, and [`reset`] is what returns it to `Idle`.
pub(crate) fn on_match_finished(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    outcome: MatchOutcome,
    reason: EndReason,
) {
    // **16b cut U.4 (16c `T-52`)**: the `@arena/pose` overrides were for the
    // next match, and when that match is the one ending they have been honoured.
    // Asked before the centre test below, and by match id rather than by
    // centre: an override set after this match started is refused, one set
    // while it is `Ended` belongs to the match after, and a staff `@arena/start`
    // match never consumed any.
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.release_openings(match_id);
    }
    if centre(ctx.w).and_then(|centre| centre.match_id) != Some(match_id) {
        return;
    }
    let state = match outcome {
        MatchOutcome::Abort => MatchState::Aborted,
        MatchOutcome::Void => MatchState::Void,
        _ => MatchState::Ended,
    };
    // 16b cut K.2: the pods have bodies in them now, and "released" has to mean
    // the hatches as well as the rows — which is [`release`]'s own first half
    // since 16b cut F.1.
    release(ctx, state, Some(outcome));
    // 16b cut Q.7: the reason the board renders the outcome with.
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.centre_mut().outcome_reason = Some(reason);
    }
    announce(
        ctx,
        format!(
            "Match {match_id} is over ({}). Its units are checked in.",
            outcome.as_str()
        ),
    );
}

/// **Bind the centre to a match staff started outside the lobby**
/// (`@arena/start`).
///
/// The shortcut has no slots and no pods — it names units directly — so what it
/// binds is the *state*: the centre is running that match, `SIM STATUS` and
/// `@arena/status` agree, and a `SIM TEAM` in the lobby is refused while it
/// does.
pub(crate) fn adopt(ctx: &mut Ctx<'_>, match_id: MatchId) {
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        let centre = book.centre_mut();
        *centre = Centre {
            state: MatchState::Running,
            match_id: Some(match_id),
            ..Centre::default()
        };
    }
}

/// **Release everything the centre holds and move it to `state`.**
///
/// **16b cut F.1 gave it the seat half.** Before this cut `release` cleared four
/// fields on a struct, and every pre-`Running` end — an abort from `Boarding`,
/// the board deadline emptying a side, a check-out that refused at the trigger —
/// went through it and left the pilot sitting in the pod with the tank's billet
/// in hand and the account's focus on it. Doc 16 §3.8's *an abort from
/// `Selection` or `Boarding` releases slots and pods only* was written before
/// `BOARD` made a real seat during `Boarding` (cut K.2), and a pod with somebody
/// in it is not released.
///
/// The sweep runs **first**, so the pilot is standing in the team room by the
/// time the caller announces why.
fn release(
    ctx: &mut Ctx<'_>,
    state: MatchState,
    outcome: Option<MatchOutcome>,
) -> crate::match_::cockpit::EjectReport {
    let ejected = crate::match_::cockpit::eject_all(ctx);
    release_rows(ctx, state, outcome);
    ejected
}

/// Drop every slot and pod and move the centre to `state` — the **row** half of
/// [`release`], which [`reset`] calls on its own because it has already swept.
fn release_rows(ctx: &mut Ctx<'_>, state: MatchState, outcome: Option<MatchOutcome>) {
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        let centre = book.centre_mut();
        centre.slots.clear();
        centre.leads.clear();
        centre.board_deadline_ms = None;
        centre.state = state;
        centre.outcome = outcome;
        // 16b cut D.3 (`B-33`): the third and last thing that clears the stamp.
        // Whatever ended the match, it is over, and the clock that would have
        // ended it goes with the rows it was counting.
        centre.clear_unattended();
        // 16b cut Q.7: the reason is written by `on_match_finished`, straight
        // after this; every other release has an outcome with no arena reason
        // behind it (an abort before the arena opened), so it is cleared here.
        centre.outcome_reason = None;
        if state == MatchState::Idle {
            centre.match_id = None;
            centre.arena = SmolStr::default();
            // 16b cut Q.7: the fill size belongs to the match that was set up,
            // so the next `SIM TEAM` starts from the default again.
            centre.ai_side_size = None;
            // 16b cut O.1: and so does the mode.
            centre.mode = MatchMode::Skirmish;
            // 16b cut O.3: and the range's count with it (the invariant on
            // `Centre::range_targets`).
            centre.range_targets = None;
            // 17b cut M5.1: and the fills' mind choices, which belonged to the
            // match that was set up (a slot's choice goes with `slots`).
            centre.fill_minds.clear();
            // 17b cut M5.2: and Q-UM-15's rule, decided at that match's
            // `SIM START`.
            centre.doctrine_only = false;
        }
    }
}

// ---------------------------------------------------------------------------
// `SIM STATUS`
// ---------------------------------------------------------------------------

/// **Who is reading the board** (16b cut D.1; doc 16 §1, §5).
///
/// `SIM STATUS` is the one selector that answers in every state and from every
/// room of the centre **including the pods**, so a seated pilot mid-match used to
/// read the other side's whole roster with `, seated` / `, left` on every row:
/// they learned that the human on `RA` had dropped and a machine had the tank,
/// which no sensor of theirs told them. Cut Q.8 bracketed the chassis word to
/// `Selection` for exactly this reason and left the standing words unbracketed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardReader {
    /// `@arena/status`: the whole board in every state, byte for byte.
    Staff,
    /// A player, by the character that typed the verb. Their own side is their
    /// own knowledge; every other side is reduced while a match runs.
    Player(ObjId),
}

/// **`SIM STATUS`** — the centre, rendered (doc 16 §5).
///
/// **16b cut D.1 gave it a reader.** While `centre.state` is `Running` or
/// `Stopping`, and only for a [`BoardReader::Player`], the reader's **own** side
/// — the team of their slot — prints in full and every other side prints
/// **reduced**: `{index} ({callsign}): {pilot name}` and nothing else. No pod, no
/// `, seated`, no `, left`, no `, flown by its mind` (16b cut Y.3's drop clause,
/// reworded by 17b cut M5.3), no `[type]`.
/// A player holding no slot reads both sides reduced. [`BoardReader::Staff`],
/// and every other state — `Idle`, `Selection`, `Boarding` and the three
/// terminal states — get the whole board exactly as before; `Boarding`'s `seated` is lobby knowledge, because there is
/// no arena yet and nothing has detected anything.
///
/// Nothing in this crate writes `Stopping` to a **centre** — that state lives on
/// the [`crate::match_::MatchRecord`], which is where an abort parks a match
/// while its stop is in flight — so the second arm of the gate is for the day a
/// centre takes it and the reachable window (a stop sent, every slot still on
/// the board) is read under `Running`.
///
/// **The pilot's name stays on a reduced row on purpose.** It was public through
/// the whole of `Selection` and cannot change while a match runs, so printing it
/// publishes nothing that was not already known. Q1's "callsigns alone" is read
/// as *no standing*: what the row withholds is whether that pilot is still in the
/// tank, which is the only part a sensor would have had to earn.
///
/// The `AI fill:` line, the boarding clock, the arena clock and the terminal hint
/// are the same for every reader.
///
/// **17b cut M5.1 adds the minds, fogged** (the design note's §2.4; doc 17
/// §12, D10): in `Selection` only, a slot's row ends `, mind Standard Doctrine
/// (Sniper, Line)` for its own character and for staff and `, a mind` for
/// everyone else, and a slot that chose nothing prints nothing; a side's fill
/// choice is one row after both sides' — `  red fill: Standard Doctrine
/// (Sniper)` for the side's lead and staff, `  red fill: a mind` for everyone
/// else, no row for an unchosen fill. From `SIM START` on no row names or marks
/// a mind for any reader, staff included.
#[must_use]
pub fn status_lines(w: &World, reader: BoardReader) -> Vec<String> {
    // **16b cut F.4**: the board of a centre that cannot run a match is a
    // healthy-looking screen over nothing, which is how the layer-off world read
    // as working right up to `SIM START`. The read says the true thing instead.
    if let Err(error) = require_layer(w) {
        return vec![error.to_string()];
    }
    let Some(centre) = centre(w) else {
        return vec!["The arena is not available on this server.".to_owned()];
    };
    // A world the centre's content has never reached: say so, rather than
    // rendering an idle lobby nobody can walk into.
    if centre::centre_rooms(w).is_none() {
        return vec![SimError::NoCentre.to_string()];
    }
    let mut rows = Vec::new();
    let head = match centre.match_id {
        Some(match_id) => format!(
            "Simulator centre: {} (match {match_id})",
            centre.state.as_str()
        ),
        None => format!("Simulator centre: {}", centre.state.as_str()),
    };
    rows.push(match (centre.outcome, centre.outcome_reason) {
        // **16b cut Q.7**: `Spent` is a draw, and a draw for a reason nobody can
        // read off the word — so the board says it, in the em-dash style it has
        // always used for an outcome.
        (Some(_), Some(EndReason::Spent)) => {
            format!("{head} — draw: both sides are out of ammunition")
        }
        // **16b cut D.3 (`B-33`)**: the same argument for the two new ends. The
        // outcome word is the winner's, which says who won and not how.
        (Some(_), Some(EndReason::Conceded { winner })) => format!(
            "{head} — {} wins: {} conceded",
            winner.as_str(),
            winner.other().as_str()
        ),
        (Some(_), Some(EndReason::Unattended)) => {
            format!("{head} — timeout: every pilot had left")
        }
        // **16b cut O.1** (16c N11): a hold, and where — the spot's name is
        // public ground, and the match is over.
        (Some(_), Some(EndReason::Hold { winner })) => format!(
            "{head} — {} wins: held {}",
            winner.as_str(),
            hold_words(w).map_or_else(|| "the key spot".to_owned(), |hold| hold.spot.to_string())
        ),
        // **16b cut D.3 (`T-60`)**: no dash where the outcome only repeats the
        // state. `aborted (match 3) — abort` and `void — void` told the reader
        // the same thing twice; the states that carry information keep theirs.
        (Some(MatchOutcome::Abort), _) if centre.state == MatchState::Aborted => head,
        (Some(MatchOutcome::Void), _) if centre.state == MatchState::Void => head,
        (Some(outcome), _) => format!("{head} — {}", outcome.as_str()),
        (None, _) => head,
    });
    // **16b cut D.1**: the side this reader is entitled to read in full while a
    // match runs. `None` for staff (who read every side in full), and `None` for
    // a player who holds no slot (who reads every side reduced).
    let running = matches!(centre.state, MatchState::Running | MatchState::Stopping);
    let own_side = match reader {
        BoardReader::Staff => None,
        BoardReader::Player(actor) => centre.slot_of_pilot(actor).map(|slot| slot.team),
    };
    for team in Team::ALL {
        // Staff read every side in full in every state; a player reads a side in
        // full only when the match is not running or the side is their own.
        let in_full = match reader {
            BoardReader::Staff => true,
            BoardReader::Player(_) => !running || own_side == Some(team),
        };
        let mut pilots: Vec<String> = Vec::new();
        for slot in centre.slots.iter().filter(|slot| slot.team == team) {
            // **16b cut D.1**: the reduced row. The index, the callsign and the
            // pilot's name, and not one byte about where that pilot is or what
            // the slot is flying.
            if !in_full {
                let callsign = crate::arena::state::Designator::Slot {
                    team,
                    slot: u16::from(slot.index),
                };
                pilots.push(format!(
                    "{} ({callsign}): {}",
                    slot.index,
                    display_name(w, slot.pilot)
                ));
                continue;
            }
            let pod = match slot.pod {
                Some(pod) => format!(", pod {}", display_name(w, pod)),
                None => String::new(),
            };
            // 16b cut F.1: `left` outranks `seated` — the slot still carries the
            // unit it was seated on, and the pilot is not in it. **16b cut Y.3**:
            // and a drop is said apart from an `EXIT`, because a dropped pilot
            // can take the tank back and the machine is only holding it for them.
            let standing = board_standing_clause(slot.is_dropped(), slot.left, slot.is_seated());
            // **16b cut Q.1**: the callsign the tank in that pod will answer to,
            // beside the index the board has always printed. It withholds
            // nothing — the letter is one piece of arithmetic on an index that is
            // already here — and it is what lets a pilot who hears `BA` on the
            // radio find out who that is.
            let callsign = crate::arena::state::Designator::Slot {
                team,
                slot: u16::from(slot.index),
            };
            // **16b cut Q.8**: the machine this slot has chosen, when it is not
            // the pod's own. A slot that chose nothing prints nothing, so the
            // row a stock lobby renders is byte for byte the one Q.1 wrote.
            //
            // **And only during `Selection`** (16b cut Q.8, fixer). The board is
            // read from the lobby, the two team rooms AND the pods, and
            // `SIM STATUS` is the one selector that answers in every state — so
            // an unbracketed rule here would hand a seated pilot mid-match the
            // chassis of the contact their own sight still renders `X1`, which
            // is the single fact Q.1's provisional designators and Q.2's
            // classification clock exist to withhold (doc 16 §1, §5). Before the
            // match there is no arena and no sensor to go round: the choice is a
            // lobby negotiation and both sides may read it, which is what the
            // callsign note above says of the letter. Afterwards the choice has
            // already been fitted to the tank, so the row carries nothing the
            // lobby still needs and staff read the real thing with `@unit/show`.
            let fielding = match centre.state {
                MatchState::Selection => slot
                    .unit_type
                    .as_deref()
                    .map(|id| format!(" [{}]", type_word(w, id)))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            // **17b cut M5.1** (the design note's §2.4; doc 17 §12's fog):
            // the slot's mind, after the bracket and on its model, in
            // `Selection` only — named to the slot's own character and to
            // staff (`, mind Standard Doctrine (Sniper, Line)`), marked to
            // everyone else (`, a mind`), and nothing against a slot that
            // chose nothing, so a stock lobby's row is byte for byte what it
            // was. From `SIM START` on no row names or marks a mind for any
            // reader, staff included (doc 17:937).
            let mind = match centre.state {
                MatchState::Selection => board_mind_clause(
                    slot.mind.as_ref(),
                    match reader {
                        BoardReader::Staff => true,
                        BoardReader::Player(actor) => actor == slot.pilot,
                    },
                ),
                _ => String::new(),
            };
            pilots.push(format!(
                "{} ({callsign}): {}{fielding}{mind}{pod}{standing}",
                slot.index,
                display_name(w, slot.pilot)
            ));
        }
        if pilots.is_empty() {
            rows.push(format!("  {}: nobody", team.as_str()));
        } else {
            rows.push(format!("  {}: {}", team.as_str(), pilots.join("; ")));
        }
    }
    // **17b cut M5.1** (the design note's §2.4): each side's fill choice, in
    // `Selection` only and only where the fill runs, after both sides' rows —
    // named to the side's lead and to staff (`  red fill: Standard Doctrine
    // (Sniper)`), marked to everyone else (`  red fill: a mind`), and no row
    // for a fill nobody chose for.
    if centre.state == MatchState::Selection
        && centre.mode != MatchMode::Range
        && configs(w).is_some_and(|(_, match_cfg, _)| match_cfg.ai_fill)
    {
        for team in Team::ALL {
            let Some(fill) = centre.fill_minds.iter().find(|fill| fill.team == team) else {
                continue;
            };
            let reads = match reader {
                BoardReader::Staff => true,
                BoardReader::Player(actor) => {
                    account_of(w, actor).is_ok_and(|account| centre.leads_side(team, account))
                }
            };
            rows.push(board_fill_row(team, &fill.choice, reads));
        }
    }
    // **16b cut Q.7**: what the machines are doing here.
    //
    // An AI-filled unit never enters `centre.slots`, so the board said nothing at
    // all about it — and Q.1's slot designators made that a real gap rather than a
    // tidy omission, because a pilot who hears `BC` on the radio can read "blue's
    // third" off the letter and would otherwise find blue's third missing from the
    // board. The line publishes no order of battle the letters do not already
    // give: how many a side, and that the machines take the pods nobody claimed.
    //
    // **16b cut O.3** (plan item 21): and not under a range, where it would be
    // false — the fill takes no pod on a range.
    if let Some((_, match_cfg, _)) = configs(w)
        && match_cfg.ai_fill
        && centre.mode != MatchMode::Range
        && matches!(
            centre.state,
            MatchState::Selection | MatchState::Boarding | MatchState::Running
        )
    {
        rows.push(format!(
            "  AI fill: each side fields {}; machines take the unclaimed pods.",
            ai_side_size(w, centre, &match_cfg)
        ));
    }
    // **16b cut O.1** (16c N11; plan items 10, 17): the rule, to every reader,
    // while a match is set up, boarding or running — and only for a mode other
    // than the skirmish, so a stock board is byte for byte what it was. It says
    // the RULE and never who holds the spot: the clock reaches no pilot.
    if centre.mode == MatchMode::Hold
        && matches!(
            centre.state,
            MatchState::Selection | MatchState::Boarding | MatchState::Running
        )
        && let Some(hold) = hold_words(w)
    {
        rows.push(format!(
            "  mode: hold — keep {} ({}) uncontested for {} s",
            hold.spot,
            hold.letter,
            hold.seconds()
        ));
    }
    // **16b cut O.3** (16c N13): the range, in the same shape and the same
    // states — how many targets, and on which side, which is the whole of what
    // `SIM RANGE` set and is public in selection.
    if matches!(
        centre.state,
        MatchState::Selection | MatchState::Boarding | MatchState::Running
    ) && let Some(count) = centre.range_targets
        && let Some((_, targets)) = range_sides(centre)
    {
        rows.push(format!(
            "  mode: range — {} on the {} side",
            targets_counted(count),
            targets.as_str()
        ));
    }
    if let Some(deadline) = centre.board_deadline_ms {
        let left = deadline
            .saturating_sub(w.now().saturating_mul(1_000))
            .max(0);
        rows.push(format!("  boarding closes in {} second(s)", left / 1_000));
    }
    // **16b cut D.1**: the arena's own clock, in minutes and seconds. It used to
    // print the last closed ledger chunk's timestamp in raw milliseconds, which is
    // `0 ms` for the first minutes of a match and a minute or more stale after
    // that; the mirror now carries the clock off every `Frames` message, so this
    // is at worst one pulse behind. `0:00` until the first message arrives.
    if let Some(now_ms) = centre
        .match_id
        .and_then(|match_id| arena_now_ms(w, match_id))
    {
        rows.push(format!("  arena time {}", clock_text(now_ms)));
    }
    // **16b cut F.1**: the way out of a terminal centre, said where somebody
    // reads it. The board still carries the last outcome above; this is the one
    // line that says the board is not a dead end.
    if centre.state.is_terminal() {
        rows.push("  `SIM TEAM red` or `SIM TEAM blue` opens the next match.".to_owned());
    }
    rows
}

/// **The board's drop clause** (17b cut M5.3; `, flown by doctrine` until that
/// cut): a dropped pilot's row ends with it while the tank's sealed mind holds
/// the tank for them. One word for every mind, so it names none (the design
/// note's D11). [`board_standing_clause`] returns it for a drop.
pub const DROP_CLAUSE: &str = ", flown by its mind";

/// **The board's standing clause for one slot's row** (16b cut F.1, Y.3):
/// `, flown by its mind` for a pilot who dropped (the linkdead grace run out,
/// or a `QUIT`) and may take the tank back with `BOARD`; `, left` for an
/// `EXIT` (one-way); `, seated` for a pilot in the cockpit; nothing for a slot
/// not yet seated. A drop outranks `left` ([`Slot::is_dropped`] is `left` and
/// a drop), and `left` outranks `seated`.
///
/// **17b cut M5.3** (the design note's C-11 and D11): the drop clause was
/// `, flown by doctrine`. It is now one word for every mind a hull may be
/// sealed with, so the row tells nobody which mind flies the tank. An `EXIT`
/// keeps `, left`, the reader's (and the bot's) "cannot take it back".
/// [`status_lines`] prints it with [`Slot::is_dropped`], [`Slot::left`] and
/// [`Slot::is_seated`] (`dropped` without `left` is no slot's state); the bot's
/// contract test reads what it prints, and its tests take the drop clause
/// from [`DROP_CLAUSE`].
#[must_use]
pub const fn board_standing_clause(dropped: bool, left: bool, seated: bool) -> &'static str {
    if dropped {
        DROP_CLAUSE
    } else if left {
        ", left"
    } else if seated {
        ", seated"
    } else {
        ""
    }
}

/// **The board's mind clause for one slot's row** (17b cut M5.1; the design
/// note's §2.4) — `, mind Standard Doctrine (Sniper, Line)` for a reader who
/// may read it (the slot's own character, staff), `, a mind` for everyone
/// else, and nothing for a slot that chose nothing. [`status_lines`] prints
/// it in `Selection` only; the bot's contract test reads what it prints.
#[must_use]
pub fn board_mind_clause(choice: Option<&MindChoice>, reads: bool) -> String {
    match choice {
        Some(choice) if reads => format!(", mind {}", choice.words()),
        Some(_) => ", a mind".to_owned(),
        None => String::new(),
    }
}

/// **The board's row for one side's fill choice** (17b cut M5.1; the design
/// note's §2.4) — `  red fill: Standard Doctrine (Sniper)` for a reader who
/// may read it (the side's lead, staff), `  red fill: a mind` for everyone
/// else. [`status_lines`] prints it in `Selection` only, for a chosen fill.
#[must_use]
pub fn board_fill_row(team: Team, choice: &MindChoice, reads: bool) -> String {
    if reads {
        format!("  {} fill: {}", team.as_str(), choice.words())
    } else {
        format!("  {} fill: a mind", team.as_str())
    }
}

/// The centre's rows for `@arena/status`: the same lines `SIM STATUS` renders,
/// and **nothing at all** for a centre nobody has touched, so a staff read of a
/// quiet server is the one line it has always been.
#[must_use]
pub fn staff_status_rows(w: &World) -> Vec<String> {
    // A world with no centre in it — a composition without the OOC pack at
    // version 4 — reads exactly as it did before this cut: the per-match rows
    // and nothing else, with no line about a lobby that is not there.
    if centre::centre_rooms(w).is_none() {
        return Vec::new();
    }
    match centre(w) {
        // 16b cut D.1: staff read every side in full, in every state.
        Some(centre) if *centre != Centre::default() => status_lines(w, BoardReader::Staff),
        _ => Vec::new(),
    }
}

/// The arena's own clock for one match, as the registry's mirror holds it (16b
/// cut D.1: the per-pulse `now_ms`, not the ledger chunk's timestamp).
fn arena_now_ms(w: &World, match_id: MatchId) -> Option<i64> {
    w.module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .map(|mirror| mirror.now_ms)
}

/// **A stretch of arena time as `m:ss`** (16b cut D.1).
///
/// Whole minutes, then two digits of seconds. The board and `@arena/status` both
/// print it, so the two clocks are one spelling. A negative reading — which
/// nothing produces — floors at `0:00`.
#[must_use]
pub fn clock_text(now_ms: i64) -> String {
    let total_s = now_ms.max(0) / 1_000;
    format!("{}:{:02}", total_s / 60, total_s % 60)
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Say one line everywhere in the centre: its three rooms **and its pods**
/// (twelve since 16b cut Z.3; every one of [`centre::POD_ENTRIES`] this world
/// has).
///
/// The centre is three rooms and one conversation: a pilot in the red room has
/// to hear that blue started, and a pilot in the lobby has to hear that boarding
/// closed. Nothing else in the world hears any of it.
///
/// **16b cut K.2 added the pods.** A room fan-out reaches whoever is *located*
/// in that room, and `BOARD` puts the body inside the pod — so the moment the
/// cut that seats pilots landed, the pilots were the one audience the centre
/// could not reach, and "Match 1 is running. Good hunting." was said to an empty
/// lobby. A pod is where a pilot is, so a pod is one of the places the centre
/// speaks.
fn announce(ctx: &mut Ctx<'_>, text: String) {
    let Some(rooms) = centre::centre_rooms(ctx.w) else {
        let actor = ctx.actor;
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, text);
        return;
    };
    let pods: Vec<ObjId> = centre::POD_ENTRIES
        .iter()
        .filter_map(|(team, index, _)| centre::pod_object(ctx.w, *team, *index))
        .collect();
    for room in rooms.into_iter().chain(pods) {
        ctx.out.to_room(
            room,
            Vec::new(),
            OutputEvent::Sys(RichText::tag(NOTICE_CATEGORY, text.clone())),
        );
    }
}

/// One object's name, or its dbref when it has none.
///
/// **16b cut V.3** made it `pub(crate)`: `@arena/report` names the pilots on its
/// header with it.
pub(crate) fn display_name(w: &World, object: ObjId) -> String {
    w.get(object).map_or_else(
        || format!("#{}", object.0),
        |object| object.name.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The pool at the choice** (17b cut M3.5; doc 17 §9, D-UM-36), as
    /// `SIM UNIT` and `SIM MIND` ask it since 17b cut M5.1, with a class
    /// named on every hull:
    ///
    /// * a call whose every load is `None` — what `SIM UNIT` passed before
    ///   17b cut M5.1, and nothing passes since — is admitted under any pool,
    ///   the smallest legal one (1/1/1, a half of nothing) included;
    /// * under base 9,000, per unit 1, floor 1 and six pods a side, the pool
    ///   is over TWELVE units (8,988, a half of 4,494 — over six it would be
    ///   4,497): eleven Class 0 slots speak for 4,400 and the caller's Class
    ///   0 is refused with `SimError::Pool`, said in the pool's sentence (the
    ///   owner's OQ-11 wording since 17b cut M5.1: `This side would think
    ///   400 ops a second; its half of the centre's pool is 4,494 and 4,400
    ///   are spoken for.` was this pin at 17b cut M3.5); ten slots and the
    ///   caller fit;
    /// * a Class III choice under the 1/1/1 pool is refused against a half of
    ///   0 with nothing spoken for.
    #[test]
    fn m3_the_pool_at_the_choice_refuses_with_the_arithmetic() {
        use crate::arena::core::CoreClass;
        use crate::match_::pool::PoolRefusal;
        let smallest = MindsConfig {
            pool_base_ops_s: 1,
            pool_per_unit_ops_s: 1,
            pool_floor_ops_s: 1,
            ..MindsConfig::default()
        };
        for cfg in [MindsConfig::default(), smallest.clone()] {
            assert_eq!(
                pool_at_the_choice(&cfg, PODS_PER_SIDE, std::iter::repeat_n(None, 5), None),
                Ok(())
            );
        }

        let tiny = MindsConfig {
            pool_base_ops_s: 9_000,
            pool_per_unit_ops_s: 1,
            pool_floor_ops_s: 1,
            ..MindsConfig::default()
        };
        let refused = pool_at_the_choice(
            &tiny,
            6,
            std::iter::repeat_n(Some(CoreClass::Zero), 11),
            Some(CoreClass::Zero),
        );
        assert_eq!(
            refused,
            Err(SimError::Pool(PoolRefusal {
                would: 400,
                half: 4_494,
                spoken: 4_400,
            }))
        );
        assert_eq!(
            refused.map_err(|error| error.to_string()),
            Err(
                "This tank would think 400 ops a second; its side's half of the centre's pool is \
                 4,494 and 4,400 are spoken for."
                    .to_owned()
            )
        );
        assert_eq!(
            pool_at_the_choice(
                &tiny,
                6,
                std::iter::repeat_n(Some(CoreClass::Zero), 10),
                Some(CoreClass::Zero),
            ),
            Ok(())
        );

        assert_eq!(
            pool_at_the_choice(&smallest, 6, [], Some(CoreClass::III)),
            Err(SimError::Pool(PoolRefusal {
                would: 18_000,
                half: 0,
                spoken: 0,
            }))
        );
    }

    fn account(n: u8) -> AccountId {
        AccountId::mint(&format!("pilot-{n}"), i64::from(n)).expect("an account id mints")
    }

    fn slot(team: Team, index: u8, who: u8) -> Slot {
        Slot {
            team,
            index,
            account: account(who),
            pilot: ObjId(u32::from(who)),
            pod: None,
            unit: None,
            left: false,
            dropped_at_ms: None,
            unit_type: None,
            released_by_resume: false,
            mind: None,
        }
    }

    #[test]
    fn a_side_fills_the_lowest_free_slot_and_then_refuses() {
        let mut centre = Centre::default();
        assert_eq!(centre.free_index(Team::Red, 2), Some(1));
        centre.slots.push(slot(Team::Red, 1, 1));
        assert_eq!(centre.free_index(Team::Red, 2), Some(2));
        centre.slots.push(slot(Team::Red, 2, 2));
        assert_eq!(centre.free_index(Team::Red, 2), None, "a full side");
        assert_eq!(
            centre.free_index(Team::Blue, 2),
            Some(1),
            "the other side is its own"
        );
        // And a released slot is the one the next claim takes.
        centre.slots.retain(|held| held.index != 1);
        assert_eq!(centre.free_index(Team::Red, 2), Some(1));
    }

    #[test]
    fn the_cap_is_the_configured_one_held_down_to_the_pods_that_exist() {
        // 16b cut Z.3: the pods that exist are the ones **this world** has, pod
        // and tank on both sides, and not the binary's constant.
        let whole = centre::tests::whole_centre();
        let old = centre::tests::twelve_pods_and_eight_tanks();
        let cfg = MatchConfig::default();
        assert_eq!(cfg.max_slots_per_side, 12, "doc 16 section 1's 12 v 12");
        assert_eq!(PODS_PER_SIDE, 6);
        assert_eq!(
            slots_per_side(whole.world(), &cfg),
            PODS_PER_SIDE,
            "but the centre has six pods a side"
        );
        assert_eq!(
            slots_per_side(old.world(), &cfg),
            4,
            "and a world that has not taken the arena's version 3 has four tanks a side"
        );
        let two = MatchConfig {
            max_slots_per_side: 2,
            ..MatchConfig::default()
        };
        assert_eq!(slots_per_side(whole.world(), &two), 2);
        assert_eq!(slots_per_side(old.world(), &two), 2);
        let five = MatchConfig {
            max_slots_per_side: 5,
            ..MatchConfig::default()
        };
        assert_eq!(slots_per_side(whole.world(), &five), 5);
        assert_eq!(slots_per_side(old.world(), &five), 4);
        let none = MatchConfig {
            max_slots_per_side: 0,
            ..MatchConfig::default()
        };
        assert_eq!(slots_per_side(whole.world(), &none), 1, "never zero");
        let bare = mush_testkit::TestWorld::new();
        assert_eq!(
            slots_per_side(bare.world(), &cfg),
            1,
            "never zero, even with no centre"
        );
    }

    #[test]
    fn a_busy_centre_is_every_live_state_and_no_terminal_one() {
        for state in [
            MatchState::Selection,
            MatchState::Boarding,
            MatchState::Running,
            MatchState::Stopping,
        ] {
            let centre = Centre {
                state,
                ..Centre::default()
            };
            assert!(centre.is_busy(), "{}", state.as_str());
        }
        for state in [
            MatchState::Idle,
            MatchState::Ended,
            MatchState::Aborted,
            MatchState::Void,
        ] {
            let centre = Centre {
                state,
                ..Centre::default()
            };
            assert!(!centre.is_busy(), "{}", state.as_str());
        }
    }

    #[test]
    fn one_account_holds_one_slot_however_many_characters_it_has() {
        let mut centre = Centre::default();
        centre.slots.push(slot(Team::Red, 1, 1));
        let mut second = slot(Team::Blue, 1, 1);
        second.pilot = ObjId(99);
        assert!(
            centre.slot_of_account(second.account).is_some(),
            "the alt's account already holds one"
        );
        assert!(centre.slot_of_pilot(ObjId(99)).is_none());
    }

    /// **16b cut AA.2 (`B-78`)**: the one succession rule, without a world. The
    /// recorded lead stays while it holds a slot on its side; a lead whose slot
    /// is gone hands the side to its first remaining claimant; a side with no
    /// slot has no entry. Read through `leads_side` and `len`, never as a
    /// vector: nothing depends on the entry order.
    #[test]
    fn resettle_leads_keeps_promotes_and_drops() {
        let (a, b, c) = (account(1), account(2), account(3));
        let mut centre = Centre::default();
        centre.slots.push(slot(Team::Red, 1, 1));
        centre.slots.push(slot(Team::Red, 2, 2));
        centre.slots.push(slot(Team::Blue, 1, 3));
        centre.leads = vec![(Team::Red, a), (Team::Blue, c)];

        // Keep: every slot is still held, so nothing moves.
        centre.resettle_leads();
        assert!(
            centre.leads_side(Team::Red, a),
            "red keeps its first claimant"
        );
        assert!(centre.leads_side(Team::Blue, c), "blue keeps its own");
        assert_eq!(centre.leads.len(), 2);

        // Promote: red's lead is gone, and red's next claimant leads it.
        centre.slots.retain(|held| held.account != a);
        centre.resettle_leads();
        assert!(centre.leads_side(Team::Red, b), "red's heir leads it");
        assert!(
            !centre.leads_side(Team::Red, a),
            "the dropped lead does not"
        );
        assert!(centre.leads_side(Team::Blue, c), "blue is untouched");
        assert_eq!(centre.leads.len(), 2);

        // Drop: blue has no slot left, so it has no lead at all.
        centre.slots.retain(|held| held.account != c);
        centre.resettle_leads();
        assert!(
            !centre.leads.iter().any(|(side, _)| *side == Team::Blue),
            "an empty side has no entry"
        );
        assert!(centre.leads_side(Team::Red, b));
        assert_eq!(centre.leads.len(), 1);
    }

    /// **16b cut B.4** (cut AA's residual): a slot whose pilot has left the tank
    /// mid-match is still on the board but neither keeps the lead nor inherits
    /// it. Keep (the lead is in its tank), promote past a left slot to the next
    /// claimant still in one, and an all-left side has no entry.
    #[test]
    fn resettle_leads_skips_a_slot_that_has_left_its_tank() {
        let (a, b, c, d) = (account(1), account(2), account(3), account(4));
        let mut centre = Centre::default();
        // Claim order is the board's order: `a`, then `c`, then `b` — so `c`
        // stands between the lead and `b`, and an heir search that skipped only
        // the outgoing lead would land on `c`.
        centre.slots.push(slot(Team::Red, 1, 1));
        centre.slots.push(slot(Team::Red, 2, 3));
        centre.slots.push(slot(Team::Red, 3, 2));
        centre.slots.push(slot(Team::Blue, 1, 4));
        centre.leads = vec![(Team::Red, a), (Team::Blue, d)];

        // Keep: a non-lead leaving moves nothing.
        if let Some(held) = centre.slots.iter_mut().find(|held| held.account == c) {
            held.left = true;
        }
        centre.resettle_leads();
        assert!(centre.leads_side(Team::Red, a), "red keeps its lead");
        assert!(centre.leads_side(Team::Blue, d));

        // Promote past a left slot: the lead leaves; the next claimant in claim
        // order is `c`, who has left and is still on the board, so it is skipped
        // and `b` — the first claimant still in a tank — inherits.
        if let Some(held) = centre.slots.iter_mut().find(|held| held.account == a) {
            held.left = true;
        }
        centre.resettle_leads();
        assert!(centre.leads_side(Team::Red, b), "red's heir is in a tank");
        assert!(
            !centre.leads_side(Team::Red, a),
            "the lead who left does not lead"
        );
        assert!(
            !centre.leads_side(Team::Red, c),
            "nor does the earlier claimant who left"
        );
        assert_eq!(centre.slots.len(), 4, "the slots stay on the board");

        // All left: the side has no lead at all.
        if let Some(held) = centre.slots.iter_mut().find(|held| held.account == b) {
            held.left = true;
        }
        centre.resettle_leads();
        assert!(
            !centre.leads.iter().any(|(side, _)| *side == Team::Red),
            "a side whose every slot has left has no entry"
        );
        assert!(centre.leads_side(Team::Blue, d), "blue is untouched");
        assert_eq!(centre.leads.len(), 1);
    }

    /// **16b cut O.1**: the three `SIM MODE` refusals' sentences, pinned here —
    /// `ModeUnsupported` above all, which no production-composed world reaches
    /// (the catalog is compiled in and the centre's arena is always the proving
    /// ground, which declares `hold`).
    #[test]
    fn o1_the_mode_refusals_read_as_their_sentences() {
        assert_eq!(
            SimError::ModeUnsupported {
                mode: "hold",
                arena: "The Stand-in".to_owned(),
            }
            .to_string(),
            "The Stand-in does not support hold; SIM MODE lists what it does."
        );
        assert_eq!(
            SimError::NotLeadForMode.to_string(),
            "Only a side's lead or staff can set the mode."
        );
        assert_eq!(
            SimError::NoSuchMode {
                word: "capture".to_owned(),
            }
            .to_string(),
            "No such mode: capture. SIM MODE lists them."
        );
        // A new centre is a skirmish.
        assert_eq!(Centre::default().mode, MatchMode::Skirmish);
    }

    /// **16b cut O.3**: the range's six refusals read as their sentences, and a
    /// new centre holds no range — the invariant's `Idle` half.
    #[test]
    fn o3_the_range_refusals_read_as_their_sentences() {
        for (error, line) in [
            (SimError::RangeViaRange, "SIM RANGE <n> sets the range."),
            (
                SimError::NotLeadForRange,
                "Only a side's lead or staff can set the range.",
            ),
            (
                SimError::RangeTwoSides,
                "The range is one side against the targets; every slot must be on one side.",
            ),
            (
                SimError::RangeCount { max: 6 },
                "The range fields 1 to 6 targets.",
            ),
            (
                SimError::RangeSide {
                    team: "blue",
                    other: "red",
                },
                "The range puts targets on the blue side; claim a red slot.",
            ),
            (
                SimError::AiOnRange,
                "The range fields no machines; SIM RANGE sets its targets.",
            ),
        ] {
            assert_eq!(error.to_string(), line);
        }
        let idle = Centre::default();
        assert_eq!(idle.range_targets, None);
        assert_eq!(range_sides(&idle), None, "not a range");
        // The sides are read off the first slot: the shooters, then the other.
        let mut centre = Centre {
            mode: MatchMode::Range,
            range_targets: Some(2),
            ..Centre::default()
        };
        assert_eq!(range_sides(&centre), None, "no slot to read a side off");
        centre.slots.push(slot(Team::Blue, 1, 1));
        assert_eq!(range_sides(&centre), Some((Team::Blue, Team::Red)));
        centre.mode = MatchMode::Hold;
        assert_eq!(range_sides(&centre), None, "a hold is not a range");
        // One target is one target (the fixer's pluralisation).
        assert_eq!(targets_counted(1), "1 target");
        assert_eq!(targets_counted(2), "2 targets");
        assert_eq!(targets_counted(6), "6 targets");
    }

    /// **16b cut AE.3**: `SimError::ClassCap`'s one variant says two sentences
    /// — a count in the class's own noun, and a cap of nought as the class
    /// forbidden — and `SimError::NoGun` names the side and nothing in it.
    #[test]
    fn ae3_the_class_cap_and_no_gun_sentences() {
        use crate::definitions::UnitClass;
        let line = |class, cap| SimError::ClassCap { class, cap }.to_string();
        assert_eq!(
            line(UnitClass::Drone, 1),
            "A side fields at most 1 drone; pick another type."
        );
        assert_eq!(
            line(UnitClass::Drone, 3),
            "A side fields at most 3 drones; pick another type."
        );
        assert_eq!(
            line(UnitClass::Walker, 2),
            "A side fields at most 2 walkers; pick another type."
        );
        assert_eq!(
            line(UnitClass::Armour, 1),
            "A side fields at most 1 armoured machine; pick another type."
        );
        assert_eq!(
            line(UnitClass::Drone, 0),
            "This server fields no drones; pick another type."
        );
        assert_eq!(
            line(UnitClass::Armour, 0),
            "This server fields no armoured machines; pick another type."
        );
        assert_eq!(
            SimError::NoGun { team: "red" }.to_string(),
            "The red side fields no gun; a side needs a machine that carries one."
        );
    }
}
