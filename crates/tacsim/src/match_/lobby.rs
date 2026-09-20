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
//! the same refusal `+focus` gives.
//!
//! # What "released atomically" means here
//!
//! Every release in this file — a `SIM LEAVE`, a dropped slot at the boarding
//! deadline, an abort, the reset after an end — runs inside **one** command
//! handler on the single world task, beside the check-in that returns custody.
//! The pods are RAM rows on this module's own state slot and the seats, leases
//! and focus go through `mush_sim`'s one revocation seam, so either the whole
//! release rides the batch or none of it does.

use mush_core::{AccountId, Ctx, ObjId, World};
use mush_proto::{OutputEvent, RichText};
use smol_str::SmolStr;

use crate::arena::state::{EndReason, StopReason, Team};
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::arena_task::MatchId;
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
use crate::match_::centre::{self, CENTRE_ARENA, CentreRoom, PODS_PER_SIDE};
use crate::match_::{MatchBook, MatchOutcome, MatchState, Side};

/// The one line every out-of-place `SIM` refuses with. Until 16b cut F.4 a
/// command gate said it; [`crate::match_::sim::sim_command`] says it now.
pub const OUTSIDE_CENTRE: &str = "The simulator is through the lobby.";

/// One claimed slot in the centre (doc 16 §3.8's `Selection`).
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// **Whether this pilot is out of their cockpit for good** (16b cut F.1).
    ///
    /// A seat given up during `Running` is one-way (doc 16 §11's N.1 amendment):
    /// the tank goes to the reflex controller and the seat cannot be retaken. The
    /// slot stays on the board because the result record names its pilot, but
    /// `SIM STATUS` said `seated` about somebody standing in the team room, which
    /// is the one fact a spectator most wants right.
    ///
    /// Written by [`slot_left`], from the one place every route to a
    /// `Control { Released }` passes through — `EXIT`, a `QUIT` and the linkdead
    /// grace running out all reach it, and the last two reach it more often in a
    /// playtest than the first.
    pub left: bool,
}

impl Slot {
    /// Whether this slot is seated — the question the `Boarding → Running`
    /// trigger asks of every one of them.
    #[must_use]
    pub const fn is_seated(&self) -> bool {
        self.unit.is_some()
    }
}

/// **The simulator centre**, one per server, one match at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Centre {
    /// Where it is in doc 16 §3.8's machine.
    pub state: MatchState,
    /// The match id, minted at `SIM START` so `Boarding` already has a name.
    pub match_id: Option<MatchId>,
    /// The arena the centre fights on.
    pub arena: SmolStr,
    /// The claimed slots, in claim order.
    pub slots: Vec<Slot>,
    /// The first claimant of each side — its lead, who may `SIM START`.
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
    /// whatever `[tacsim].enabled` says — but the arena, its domain and the eight
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
    /// The centre is not in a state that admits this.
    #[error("The simulator centre is {state}; that is not something you can do to it now.")]
    WrongState {
        /// What state it is in.
        state: &'static str,
    },
    /// Only a team lead or staff may start.
    #[error("Only a side's first pilot or staff can start the match.")]
    NotLead,
    /// A side has nobody.
    #[error("The {0} side has no pilots; a match takes at least one a side.")]
    EmptySide(&'static str),
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
    /// **16b cut F.4 made the advice performable.** It used to say "staff must
    /// re-apply the arena", and no staff verb can: the arena pack is a boot pack
    /// and is deliberately not on the `@atlas/pack` shelf (doc 16 §3.3's tanks
    /// are created once, by the centre's content, and the latch is what makes a
    /// second boot make no ninth tank). The layer-off case now refuses earlier
    /// with [`SimError::LayerOff`], so what is left here is K.1 deviation (1) —
    /// a world that applied the arena before its roster existed, or one whose
    /// tanks have since been destroyed — and the only repair is at boot.
    #[error(
        "The simulator has no unit for {team} slot {index}. The arena is applied at boot and no \
         verb re-applies it; staff should read the server's boot log."
    )]
    NoUnit {
        /// Which side.
        team: &'static str,
        /// Which slot.
        index: u8,
    },
    /// The check-out itself refused, with the reason cut T.3 gave.
    #[error("The match could not start: {0}")]
    Start(String),
}

// ---------------------------------------------------------------------------
// Reading the centre
// ---------------------------------------------------------------------------

/// The centre, as it stands.
#[must_use]
pub fn centre(w: &World) -> Option<&Centre> {
    w.module_state::<MatchBook>().map(MatchBook::centre)
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

/// How many slots one side of **this** centre takes: the configured cap, held
/// down to the pods that exist (doc 16 §3.7 authors four a side).
#[must_use]
pub fn slots_per_side(match_cfg: &MatchConfig) -> u8 {
    u8::try_from(match_cfg.max_slots_per_side)
        .unwrap_or(PODS_PER_SIDE)
        .clamp(1, PODS_PER_SIDE)
}

/// **The claimed slots of the larger side** (16b cut Q.7), never below one.
///
/// It is the floor under [`ai_side_size`] and the number `SIM AI` refuses to go
/// below: the fill tops a side up to a count, so a count under what somebody has
/// already claimed would be an instruction to un-claim a slot.
#[must_use]
pub fn largest_side(centre: &Centre) -> u8 {
    Team::ALL
        .iter()
        .map(|team| u8::try_from(centre.side_len(*team)).unwrap_or(u8::MAX))
        .max()
        .unwrap_or(0)
        .max(1)
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
pub fn ai_side_size(centre: &Centre, match_cfg: &MatchConfig) -> u8 {
    let cap = slots_per_side(match_cfg);
    centre
        .ai_side_size
        .unwrap_or(0)
        .max(largest_side(centre))
        .clamp(1, cap)
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
fn is_staff(w: &World, actor: ObjId) -> bool {
    mush_core::perm::has_power(w, actor, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, actor, mush_core::Role::Royalty)
}

/// **The `SIM STATUS` half of [`in_centre`]** (16b cut F.4): refuse an actor who
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
    let cap = slots_per_side(&match_cfg);
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
        });
        if !centre.leads.iter().any(|(side, _)| *side == team) {
            centre.leads.push((team, account));
        }
        index
    };
    let name = display_name(ctx.w, actor);
    announce(
        ctx,
        format!(
            "{name} takes {} slot {index}. `SIM START` begins the match once both sides have a \
             pilot.",
            team.as_str()
        ),
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
    let (team, index, seated) = {
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
        if centre.side_len(held.team) == 0 {
            centre.leads.retain(|(side, _)| *side != held.team);
        } else if centre.leads_side(held.team, account) {
            // The lead leaves: the next claimant of that side inherits it, so a
            // side is never left with no one who can start.
            let heir = centre
                .slots
                .iter()
                .find(|slot| slot.team == held.team)
                .map(|slot| slot.account);
            centre.leads.retain(|(side, _)| *side != held.team);
            if let Some(heir) = heir {
                centre.leads.push((held.team, heir));
            }
        }
        if centre.slots.is_empty() {
            *centre = Centre::default();
        }
        (held.team, held.index, held.is_seated())
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
        let empty = if fill {
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
        release(ctx, MatchState::Aborted, Some(MatchOutcome::Abort));
        announce(
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
        if match_cfg.ai_fill {
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
    }
    announce(
        ctx,
        format!(
            "Match {match_id} is boarding. Every pilot: BOARD your pod. The match starts when \
             everyone is seated, or in {} second(s).",
            match_cfg.board_timeout_ms / 1_000
        ),
    );
    Ok(())
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

/// **Mark one pilot's slot as out of its cockpit for good** (16b cut F.1).
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
/// Answers whether a slot moved, which is nothing anybody prints — it is here so
/// the caller pays for no borrow it does not need.
pub(crate) fn slot_left(ctx: &mut Ctx<'_>, pilot: ObjId) -> bool {
    let Some(book) = ctx.w.module_state_mut::<MatchBook>() else {
        return false;
    };
    let centre = book.centre_mut();
    if centre.state != MatchState::Running {
        return false;
    }
    match centre.slots.iter_mut().find(|slot| slot.pilot == pilot) {
        Some(slot) if !slot.left => {
            slot.left = true;
            true
        }
        _ => false,
    }
}

/// Fire the check-out with whatever is seated (doc 16 §3.8's
/// `Boarding → Running`).
fn run(ctx: &mut Ctx<'_>, match_id: MatchId) -> Result<(), SimError> {
    let (arena_cfg, match_cfg, sensors_cfg) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    let arena = centre(ctx.w)
        .map(|centre| centre.arena.clone())
        .filter(|arena| !arena.is_empty())
        .unwrap_or_else(|| SmolStr::new_static(CENTRE_ARENA));
    let mut sides: Vec<Side> = Vec::new();
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
        let fill_to = ai_side_size(centre, &match_cfg);
        for team in Team::ALL {
            // **16b cut Q.1**: the pod index travels with the unit. It is the
            // only place in the tree that knows which slot fielded which tank, so
            // `Arena::new` can resolve `RA`, `BC` and a pilot and their teammates
            // call one machine by one name.
            let mut units: Vec<crate::match_::Fielded> = centre
                .slots
                .iter()
                .filter(|slot| slot.team == team)
                .filter_map(|slot| {
                    slot.unit.map(|unit| crate::match_::Fielded {
                        unit,
                        slot: slot.index,
                    })
                })
                .collect();
            if !match_cfg.ai_fill && units.is_empty() {
                return Err(SimError::EmptySide(team.as_str()));
            }
            // **16b cut N.1** (doc 16 §6: *the AI fill of a later lobby*), behind
            // `[tacsim.match].ai_fill`, **sized by 16b cut Q.7**. With it on, a
            // side is topped up to [`ai_side_size`] with the tanks of the lowest
            // unclaimed pods: the pods nobody claimed, and the pods of the slots
            // dropped at the boarding deadline. They arrive at the check-out with
            // **no seated pilot**, which is the one thing that makes them the
            // reflex controller's (`crate::match_::start`) — the lobby names no
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
            if match_cfg.ai_fill {
                for index in 1..=slots_per_side(&match_cfg) {
                    if units.len() >= usize::from(fill_to) {
                        break;
                    }
                    if centre
                        .slots
                        .iter()
                        .any(|slot| slot.team == team && slot.index == index)
                    {
                        continue;
                    }
                    // 16b cut Q.1: an AI-filled pod carries its own index too, so
                    // a machine in red pod 3 is `RC`, exactly as a pilot in that
                    // pod would be.
                    if let Some(tank) = centre::tank_for_slot(ctx.w, team, index)
                        && !units.iter().any(|fielded| fielded.unit == tank)
                    {
                        units.push(crate::match_::Fielded {
                            unit: tank,
                            slot: index,
                        });
                    }
                }
            }
            // A side with no unit at all is still a side nobody can fight: with
            // the fill on it means the centre's tanks are missing (K.1 deviation
            // 1), which is the same broken world `SimError::NoUnit` names.
            if units.is_empty() {
                return Err(SimError::EmptySide(team.as_str()));
            }
            sides.push(Side { team, units });
        }
    }
    match crate::match_::start(
        ctx,
        match_id,
        &arena,
        &sides,
        &arena_cfg,
        &match_cfg,
        &sensors_cfg,
    ) {
        Ok(_) => {
            if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
                let centre = book.centre_mut();
                centre.state = MatchState::Running;
                centre.board_deadline_ms = None;
            }
            announce(ctx, format!("Match {match_id} is running. Good hunting."));
            Ok(())
        }
        Err(error) => {
            // Nothing is deployed (cut T.3's `start` rolls its own writes back),
            // so the centre gives the pods and the slots back and says why.
            release(ctx, MatchState::Aborted, Some(MatchOutcome::Abort));
            announce(
                ctx,
                format!("Match {match_id} could not start: {error}. The pods are free again."),
            );
            Err(SimError::Start(error.to_string()))
        }
    }
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
    }
    // **16b cut Q.7**: with the fill on the deadline asks about the centre and not
    // about a side — a side that boarded nobody is a side the machines fly, and
    // only a centre that boarded nobody is an abort.
    let fill = configs(ctx.w).is_some_and(|(_, match_cfg, _)| match_cfg.ai_fill);
    let empty = if fill {
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
        release(ctx, MatchState::Aborted, Some(MatchOutcome::Abort));
        announce(
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
/// A team lead or staff, in `Selection` only: the fill is decided before the
/// pods lock, because it is what the pods are locked *for*. `None` reads;
/// `Some(n)` sets, and is announced to the centre the way a claim is.
///
/// The number is a **side size**, not a machine count: `SIM AI 3` is a three a
/// side match, so a side with two pilots gets one machine and a side with none
/// gets three.
///
/// # Errors
///
/// [`SimError`], as the line the player sees:
/// [`SimError::AiOff`] on a server with `[tacsim.match].ai_fill` off,
/// [`SimError::WrongState`] outside `Selection`, [`SimError::NotLead`] for
/// anyone but a side's first pilot or staff, [`SimError::AiRange`] outside
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
    let cap = slots_per_side(&match_cfg);
    let (effective, claimed) = {
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
            return Err(SimError::NotLead);
        }
        (ai_side_size(centre, &match_cfg), largest_side(centre))
    };
    let Some(size) = size else {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            format!("AI fill: {effective} a side."),
        );
        return Ok(());
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
// The ends
// ---------------------------------------------------------------------------

/// **`SIM ABORT`** — by any participant or staff, from any live state (doc 16
/// §3.8).
///
/// `Selection` and `Boarding` release slots and pods only: there is no custody
/// to return. `Running` sends `Stop { Abort }`, and cut T.3's check-in follows
/// when the arena answers.
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
    let name = display_name(ctx.w, actor);
    match (state, match_id) {
        (MatchState::Running | MatchState::Stopping, Some(match_id)) => {
            crate::match_::abort(ctx, match_id, StopReason::Abort);
            announce(
                ctx,
                format!(
                    "{name} aborts match {match_id}. Its units check back in when the arena \
                     answers."
                ),
            );
        }
        _ => {
            release(ctx, MatchState::Aborted, Some(MatchOutcome::Abort));
            announce(ctx, format!("{name} calls the match off."));
        }
    }
    Ok(())
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
}

/// **The reset to `Idle`** — idempotent, and re-runnable by staff as
/// `@arena/reset` (doc 16 §3.8).
///
/// The **one funnel** the centre returns to `Idle` through, and therefore the one
/// place doc 16 §3.3's *at every `Idle` transition each is reset to its
/// `UnitSpec`* can live. Three things, in this order:
///
/// 1. the **seat sweep**, before anything else and **before the already-idle
///    early return**. A restart leaves the centre `Idle` by construction (the
///    match book is RAM) while a body is still standing in a pod and a billet is
///    still held on its tank, and the old `reset` returned on the first line in
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
    let tanks = crate::match_::reset_stock_tanks(ctx);
    if !was_idle {
        release_rows(ctx, MatchState::Idle, None);
        if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
            *book.centre_mut() = Centre::default();
        }
    }
    ResetOutcome {
        was_idle,
        ejected,
        tanks,
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
        }
    }
}

// ---------------------------------------------------------------------------
// `SIM STATUS`
// ---------------------------------------------------------------------------

/// **`SIM STATUS`** — the centre, rendered (doc 16 §5).
#[must_use]
pub fn status_lines(w: &World) -> Vec<String> {
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
    match centre.outcome {
        // **16b cut Q.7**: `Spent` is a draw, and a draw for a reason nobody can
        // read off the word — so the board says it, in the em-dash style it has
        // always used for an outcome.
        Some(_) if centre.outcome_reason == Some(EndReason::Spent) => {
            rows.push(format!("{head} — draw: both sides are out of ammunition"));
        }
        Some(outcome) => rows.push(format!("{head} — {}", outcome.as_str())),
        None => rows.push(head),
    }
    for team in Team::ALL {
        let mut pilots: Vec<String> = Vec::new();
        for slot in centre.slots.iter().filter(|slot| slot.team == team) {
            let pod = match slot.pod {
                Some(pod) => format!(", pod {}", display_name(w, pod)),
                None => String::new(),
            };
            // 16b cut F.1: `left` outranks `seated` — the slot still carries the
            // unit it was seated on, and the pilot is not in it.
            let seated = if slot.left {
                ", left"
            } else if slot.is_seated() {
                ", seated"
            } else {
                ""
            };
            // **16b cut Q.1**: the callsign the tank in that pod will answer to,
            // beside the index the board has always printed. It withholds
            // nothing — the letter is one piece of arithmetic on an index that is
            // already here — and it is what lets a pilot who hears `BA` on the
            // radio find out who that is.
            let callsign = crate::arena::state::Designator::Slot {
                team,
                slot: u16::from(slot.index),
            };
            pilots.push(format!(
                "{} ({callsign}): {}{pod}{seated}",
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
    // **16b cut Q.7**: what the machines are doing here.
    //
    // An AI-filled unit never enters `centre.slots`, so the board said nothing at
    // all about it — and Q.1's slot designators made that a real gap rather than a
    // tidy omission, because a pilot who hears `BC` on the radio can read "blue's
    // third" off the letter and would otherwise find blue's third missing from the
    // board. The line publishes no order of battle the letters do not already
    // give: how many a side, and that the machines take the pods nobody claimed.
    if let Some((_, match_cfg, _)) = configs(w)
        && match_cfg.ai_fill
        && matches!(
            centre.state,
            MatchState::Selection | MatchState::Boarding | MatchState::Running
        )
    {
        rows.push(format!(
            "  AI fill: each side fields {}; machines take the unclaimed pods.",
            ai_side_size(centre, &match_cfg)
        ));
    }
    if let Some(deadline) = centre.board_deadline_ms {
        let left = deadline
            .saturating_sub(w.now().saturating_mul(1_000))
            .max(0);
        rows.push(format!("  boarding closes in {} second(s)", left / 1_000));
    }
    if let Some(arena_ms) = centre.match_id.and_then(|match_id| arena_ms(w, match_id)) {
        rows.push(format!("  arena time {arena_ms} ms"));
    }
    // **16b cut F.1**: the way out of a terminal centre, said where somebody
    // reads it. The board still carries the last outcome above; this is the one
    // line that says the board is not a dead end.
    if centre.state.is_terminal() {
        rows.push("  `SIM TEAM red` or `SIM TEAM blue` opens the next match.".to_owned());
    }
    rows
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
        Some(centre) if *centre != Centre::default() => status_lines(w),
        _ => Vec::new(),
    }
}

/// The arena's own clock for one match, as the registry's mirror holds it.
fn arena_ms(w: &World, match_id: MatchId) -> Option<i64> {
    w.module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .map(|mirror| mirror.arena_ms)
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Say one line everywhere in the centre: its three rooms **and its eight
/// pods**.
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
fn display_name(w: &World, object: ObjId) -> String {
    w.get(object).map_or_else(
        || format!("#{}", object.0),
        |object| object.name.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let cfg = MatchConfig::default();
        assert_eq!(cfg.max_slots_per_side, 12, "doc 16 section 1's 12 v 12");
        assert_eq!(
            slots_per_side(&cfg),
            PODS_PER_SIDE,
            "but the centre has four pods a side"
        );
        let two = MatchConfig {
            max_slots_per_side: 2,
            ..MatchConfig::default()
        };
        assert_eq!(slots_per_side(&two), 2);
        let none = MatchConfig {
            max_slots_per_side: 0,
            ..MatchConfig::default()
        };
        assert_eq!(slots_per_side(&none), 1, "never zero");
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
}
