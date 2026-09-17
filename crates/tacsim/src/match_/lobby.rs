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

use crate::arena::state::{StopReason, Team};
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::arena_task::MatchId;
use crate::config::{ArenaConfig, MatchConfig};
use crate::match_::centre::{self, CENTRE_ARENA, CentreRoom, PODS_PER_SIDE};
use crate::match_::{MatchBook, MatchOutcome, MatchState, Side};

/// The one line the gate and every out-of-place `SIM` refuses with.
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
    /// The account oracle does not know this character (fail closed, doc 10 R3).
    #[error(
        "Your account is not mapped yet, so the simulator cannot hold a slot for you; reconnect, \
         or ask staff to relink the character."
    )]
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
    /// The centre's arena, or one of its units, is not in this world.
    #[error("The simulator has no unit for {team} slot {index}; staff must re-apply the arena.")]
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
#[must_use]
fn configs(w: &World) -> Option<(ArenaConfig, MatchConfig)> {
    let cfg = w.module_state::<crate::TacsimStore>()?.config();
    Some((cfg.arena.clone(), cfg.match_.clone()))
}

/// How many slots one side of **this** centre takes: the configured cap, held
/// down to the pods that exist (doc 16 §3.7 authors four a side).
#[must_use]
pub fn slots_per_side(match_cfg: &MatchConfig) -> u8 {
    u8::try_from(match_cfg.max_slots_per_side)
        .unwrap_or(PODS_PER_SIDE)
        .clamp(1, PODS_PER_SIDE)
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
    in_centre(ctx.w, actor)?;
    let account = account_of(ctx.w, actor)?;
    let (_, match_cfg) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    let cap = slots_per_side(&match_cfg);
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
    in_centre(ctx.w, actor)?;
    let account = account_of(ctx.w, actor)?;
    let (team, index) = {
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
        (held.team, held.index)
    };
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
        let empty = Team::ALL
            .iter()
            .find(|team| centre.side_len(**team) == 0)
            .copied();
        let ready = !centre.slots.is_empty() && centre.slots.iter().all(Slot::is_seated);
        (match_id, empty, ready)
    };
    if let Some(team) = empty {
        release(ctx, MatchState::Aborted, Some(MatchOutcome::Abort));
        announce(
            ctx,
            format!(
                "Match {match_id} is aborted: the {} side has nobody left to fly it.",
                team.as_str()
            ),
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
    in_centre(ctx.w, actor)?;
    let staff = is_staff(ctx.w, actor);
    let account = match account_of(ctx.w, actor) {
        Ok(account) => Some(account),
        Err(_) if staff => None,
        Err(error) => return Err(error),
    };
    let (_, match_cfg) = configs(ctx.w).ok_or(SimError::Unavailable)?;

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
        for team in Team::ALL {
            if centre.side_len(team) == 0 {
                return Err(SimError::EmptySide(team.as_str()));
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

/// Fire the check-out with whatever is seated (doc 16 §3.8's
/// `Boarding → Running`).
fn run(ctx: &mut Ctx<'_>, match_id: MatchId) -> Result<(), SimError> {
    let (arena_cfg, match_cfg) = configs(ctx.w).ok_or(SimError::Unavailable)?;
    let arena = centre(ctx.w)
        .map(|centre| centre.arena.clone())
        .filter(|arena| !arena.is_empty())
        .unwrap_or_else(|| SmolStr::new_static(CENTRE_ARENA));
    let mut sides: Vec<Side> = Vec::new();
    {
        let Some(centre) = centre(ctx.w) else {
            return Err(SimError::Unavailable);
        };
        for team in Team::ALL {
            let units: Vec<ObjId> = centre
                .slots
                .iter()
                .filter(|slot| slot.team == team)
                .filter_map(|slot| slot.unit)
                .collect();
            if units.is_empty() {
                return Err(SimError::EmptySide(team.as_str()));
            }
            sides.push(Side { team, units });
        }
    }
    match crate::match_::start(ctx, match_id, &arena, &sides, &arena_cfg, &match_cfg) {
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
    let empty = Team::ALL
        .iter()
        .find(|team| centre(ctx.w).is_some_and(|centre| centre.side_len(**team) == 0))
        .copied();
    if let Some(team) = empty {
        release(ctx, MatchState::Aborted, Some(MatchOutcome::Abort));
        announce(
            ctx,
            format!(
                "Match {match_id} is aborted: the {} side boarded nobody.",
                team.as_str()
            ),
        );
        return;
    }
    if let Err(error) = run(ctx, match_id) {
        tracing::warn!(match_id = match_id.0, %error, "tacsim: a boarding deadline could not start its match");
    }
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

/// **The reset to `Idle`** — idempotent, and re-runnable by staff as
/// `@arena/reset` (doc 16 §3.8).
///
/// Answers whether anything changed, which is what lets the verb say "nothing
/// changed" on a second run rather than pretending it did something.
pub fn reset(ctx: &mut Ctx<'_>) -> bool {
    let idle = centre(ctx.w).is_some_and(|centre| *centre == Centre::default());
    if idle {
        return false;
    }
    release(ctx, MatchState::Idle, None);
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        *book.centre_mut() = Centre::default();
    }
    true
}

/// **The centre's half of an ended match** — called from
/// [`crate::match_::finish`] once custody is back.
///
/// The seats, leases and focus are released by the check-in itself (they belong
/// to the units, not to the lobby); what this releases is the lobby's own half,
/// the pods and the slots, in the same authoritative action. The centre is left
/// in its terminal state carrying the outcome so `SIM STATUS` can say how it
/// went, and [`reset`] is what returns it to `Idle`.
pub(crate) fn on_match_finished(ctx: &mut Ctx<'_>, match_id: MatchId, outcome: MatchOutcome) {
    if centre(ctx.w).and_then(|centre| centre.match_id) != Some(match_id) {
        return;
    }
    let state = match outcome {
        MatchOutcome::Abort => MatchState::Aborted,
        MatchOutcome::Void => MatchState::Void,
        _ => MatchState::Ended,
    };
    release(ctx, state, Some(outcome));
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

/// Drop every slot and pod and move the centre to `state`.
fn release(ctx: &mut Ctx<'_>, state: MatchState, outcome: Option<MatchOutcome>) {
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        let centre = book.centre_mut();
        centre.slots.clear();
        centre.leads.clear();
        centre.board_deadline_ms = None;
        centre.state = state;
        centre.outcome = outcome;
        if state == MatchState::Idle {
            centre.match_id = None;
            centre.arena = SmolStr::default();
        }
    }
}

// ---------------------------------------------------------------------------
// `SIM STATUS`
// ---------------------------------------------------------------------------

/// **`SIM STATUS`** — the centre, rendered (doc 16 §5).
#[must_use]
pub fn status_lines(w: &World) -> Vec<String> {
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
            let seated = if slot.is_seated() { ", seated" } else { "" };
            pilots.push(format!(
                "{}: {}{pod}{seated}",
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
    if let Some(deadline) = centre.board_deadline_ms {
        let left = deadline
            .saturating_sub(w.now().saturating_mul(1_000))
            .max(0);
        rows.push(format!("  boarding closes in {} second(s)", left / 1_000));
    }
    if let Some(arena_ms) = centre.match_id.and_then(|match_id| arena_ms(w, match_id)) {
        rows.push(format!("  arena time {arena_ms} ms"));
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

/// Say one line in all three of the centre's rooms.
///
/// The centre is three rooms and one conversation: a pilot in the red room has
/// to hear that blue started, and a pilot in the lobby has to hear that boarding
/// closed. Nothing else in the world hears any of it.
fn announce(ctx: &mut Ctx<'_>, text: String) {
    let Some(rooms) = centre::centre_rooms(ctx.w) else {
        let actor = ctx.actor;
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, text);
        return;
    };
    for room in rooms {
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
