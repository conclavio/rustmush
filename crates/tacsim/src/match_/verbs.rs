//! **`@arena`, the staff lever on a match** (16b cut T.3; doc 16 §3.7 "Verbs",
//! §3.8 "Lifecycle").
//!
//! Cut K gives players `SIM TEAM`, `SIM START`, `BOARD`, `STARTUP`, `HEADING`,
//! `SPEED`, `EXIT` and `SIM ABORT`. None of that exists yet, and the custody
//! transition underneath it is the thing this cut has to get right — so the
//! lifecycle gets a **staff** lever first, on the one verb cut T.2 already
//! registered:
//!
//! | switch | what it does |
//! |---|---|
//! | `@arena/status` | every match, its state, its tempo, its ledger counts (T.2's, extended with the state) |
//! | `@arena/start <definition>=<red>/<blue>` | `Selection → Boarding → Running` in one step: check every named unit out and open the match |
//! | `@arena/intent <unit>=<kind> [args]` | one order, as the unit's own seat |
//! | `@arena/abort [<match>]` | `Stop { Abort }` |
//! | `@arena/release <unit>` | **16b cut D.3**: clear one unit's stranded `deployed` marker and free its seat |
//! | `@arena/pose <callsign>=<x> <y>[ <heading>]` | **16b cut U.4**: the cell a centre slot's unit opens the next match on (`=off`, or bare to list) |
//! | `@arena/report [<match>] [<chunk>]` | **16b cut V.3**: one page of a match's tape, for staff ([`crate::match_::report::request_page`]) |
//! | `@arena/tempo <match>=<period_ms>` | **16b cut I.3**: floor a running match's wake interval (`=off` lifts it, bare reads it) |
//! | `@arena/place <unit>=<x>,<y>` | **16b cut I.3**: put a unit of a running match on a cell, as its own seat |
//! | `@arena/watch <match>` | **16b cut I.4**: watch a running match's whole field on `arena.observer` (`off` stops it, bare reads it) |
//! | `@arena/fault <match>` | **test builds only** — arm a panic inside the next `step` |
//!
//! `@arena/start` is deliberately the whole of `Selection → Boarding → Running`
//! rather than three verbs: the two states it passes through are slots cut K
//! fills with real player choices, and inventing a staff spelling for them now
//! would be a second lifecycle to retire later. What it does not skip is the
//! **trigger** — it calls the same [`crate::match_::start`] cut K's
//! `Boarding → Running` will.
//!
//! # `@arena/intent` and the focus check
//!
//! [`crate::arena_bridge::forward_intent`] validates focus, the live channel
//! grant, the control lease and the epoch. A staffer driving a unit from outside
//! its cockpit fails the *first* of those and nothing else, so this switch does
//! the same validation **minus focus**: the unit is in a running match, the order
//! is stamped with the epoch the world published, and the arena authorises it
//! again at apply. It is a staff tool and it says so; the player path is cut K's
//! and goes through `forward_intent` untouched.

use mush_core::{CmdError, Ctx, ObjId};
use smol_str::SmolStr;

use crate::arena::intents::IntentKind;
use crate::arena::state::{Designator, StopReason, Team};
use crate::arena_bridge::ArenaRegistry;
use crate::arena_task::{ArenaMsg, MatchId};

use super::{MatchBook, Side, allocate_id};

/// The switches cut T.3 adds to `@arena`, in help order (16b cut K.1 adds
/// `reset`; 16b cut D.3 adds `release`; 16b cut U.4 adds `pose`; 16b cut V.3
/// adds `report`; 16b cut I.3 adds `tempo` and `place`; 16b cut I.4 adds
/// `watch`).
#[must_use]
pub fn register_switches() -> Vec<SmolStr> {
    let mut switches: Vec<SmolStr> = [
        "status", "start", "intent", "abort", "reset", "release", "pose", "report", "tempo",
        "place", "watch",
    ]
    .iter()
    .copied()
    .map(SmolStr::new)
    .collect();
    if cfg!(any(test, feature = "fault-injection")) {
        switches.push(SmolStr::new("fault"));
    }
    switches
}

/// Why a staff lifecycle verb refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArenaVerbError {
    /// The argument shape was wrong.
    #[error("Usage: {0}")]
    Usage(&'static str),
    /// No such match.
    #[error("There is no match {0}.")]
    NoMatch(u64),
    /// The match is not in a state that admits this.
    #[error("Match {match_id} is {state}.")]
    WrongState {
        /// Which match.
        match_id: u64,
        /// What state it is in.
        state: &'static str,
    },
    /// The tacsim layer did not compose.
    #[error("The arena is not available on this server.")]
    Unavailable,
}

impl From<ArenaVerbError> for CmdError {
    fn from(error: ArenaVerbError) -> CmdError {
        CmdError::msg(error.to_string())
    }
}

/// `@arena/start <definition-id>=<red unit ids>/<blue unit ids>`.
///
/// Runs `Selection → Boarding → Running` in one authoritative action with the
/// named units and the pilots currently seated on them, so the custody trigger is
/// exercised without cut K's lobby.
///
/// **16b cut O.1**: always a skirmish — a mode is the lobby's `SIM MODE`, and a
/// `mode` argument here is a recorded residual.
pub fn start(
    ctx: &mut Ctx<'_>,
    arena_cfg: &crate::config::ArenaConfig,
    match_cfg: &crate::config::MatchConfig,
    sensors_cfg: &crate::config::SensorsConfig,
) -> Result<(), CmdError> {
    const USAGE: &str = "@arena/start <definition>=<red #ids>/<blue #ids>";
    let definition = ctx.args.lhs().trim().to_owned();
    if definition.is_empty() {
        return Err(ArenaVerbError::Usage(USAGE).into());
    }
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|rhs| !rhs.is_empty())
        .ok_or(ArenaVerbError::Usage(USAGE))?
        .to_owned();
    let (red, blue) = rhs.split_once('/').ok_or(ArenaVerbError::Usage(USAGE))?;
    // **16b cut Q.1**: the staff shortcut fields no pods, so every unit goes over
    // with slot `0` and `Arena::new` gives it the lowest free slot of its team in
    // unit-id order. A `@arena/start` match therefore has callsigns too, and they
    // are the same on a replay.
    let sides = vec![
        Side {
            team: crate::arena::state::Team::Red,
            units: parse_units(red)?
                .into_iter()
                .map(super::Fielded::unslotted)
                .collect(),
        },
        Side {
            team: crate::arena::state::Team::Blue,
            units: parse_units(blue)?
                .into_iter()
                .map(super::Fielded::unslotted)
                .collect(),
        },
    ];
    // **16b cut K.1.** The shortcut runs through the same machine the lobby
    // does, which here means one thing: a centre that is already holding a match
    // is not something a staff shortcut walks into. A *terminal* centre is,
    // because the shortcut is what an operator reaches for when the lobby is not
    // in play at all, and refusing until somebody reset it would be a chore with
    // no safety in it. Checked before anything is minted or written.
    if super::lobby::centre(ctx.w).is_some_and(super::Centre::is_busy) {
        return Err(CmdError::msg(
            "The simulator centre is holding a match; `@arena/abort` it, or `@arena/reset` the \
             centre, before starting another.",
        ));
    }
    let match_id = allocate_id(ctx).map_err(CmdError::msg)?;
    // The record exists in `Boarding` before the trigger fires, so the state
    // machine is walked rather than jumped: cut K's `SIM START` lands on exactly
    // this call with exactly this predecessor state.
    let arena = SmolStr::new(&definition);
    // **16b cut O.1**: a staff start is always a skirmish. `@arena/start` takes
    // no mode argument — the lobby's `SIM MODE` is where a mode is chosen — and
    // that is a residual recorded for the owner, not an oversight.
    super::start(
        ctx,
        match_id,
        &arena,
        &sides,
        arena_cfg,
        match_cfg,
        sensors_cfg,
        crate::arena::state::MatchMode::Skirmish,
    )
    .map_err(|error| CmdError::msg(error.to_string()))?;
    // It ran: the centre now says so, so `SIM STATUS` in the lobby and
    // `@arena/status` at a staff console are one answer and not two.
    super::lobby::adopt(ctx, match_id);
    let actor = ctx.actor;
    let units: usize = sides.iter().map(|side| side.units.len()).sum();
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        format!(
            "Match {match_id} is running on {arena} with {units} unit(s) checked out. \
             `@arena/abort {match_id}` ends it."
        ),
    );
    Ok(())
}

/// `@arena/intent <unit>=<kind> [args]` — one order, as the unit's own seat.
pub fn intent(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "@arena/intent <unit>=startup | heading <deg> | speed <cms>";
    let unit = parse_unit(ctx.args.lhs().trim()).ok_or(ArenaVerbError::Usage(USAGE))?;
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|rhs| !rhs.is_empty())
        .ok_or(ArenaVerbError::Usage(USAGE))?;
    let kind = parse_kind(rhs).ok_or(ArenaVerbError::Usage(USAGE))?;
    let match_id = offer_as_seat(
        ctx,
        unit,
        kind,
        "That unit already has as many orders queued as it may.",
        |_, _| Ok(()),
    )?;
    let actor = ctx.actor;
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        format!("Ordered #{} on match {match_id}.", unit.0),
    );
    Ok(())
}

/// **One order, stamped as the unit's own seat and offered to its match** —
/// the shared body of `@arena/intent` and (16b cut I.3) `@arena/place`.
///
/// The unit must be bound to a running match (`That unit is not in a running
/// match.`); `check` then runs against the registry with the match's id, which
/// is where a switch makes the refusals the world can make honestly; the order
/// is stamped with the epoch the world published
/// ([`ArenaRegistry::stamp_intent`]) and offered. A refused offer answers `busy`
/// while the task is live and `That match has ended.` after. Answers the match.
fn offer_as_seat(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    kind: IntentKind,
    busy: &'static str,
    check: impl FnOnce(&ArenaRegistry, MatchId) -> Result<(), CmdError>,
) -> Result<MatchId, CmdError> {
    let channel = kind.channel_required();
    let (match_id, intent, tasks) = {
        let registry = ctx
            .w
            .module_state_mut::<ArenaRegistry>()
            .ok_or(ArenaVerbError::Unavailable)?;
        let match_id = registry
            .match_of_unit(unit)
            .ok_or_else(|| CmdError::msg("That unit is not in a running match."))?;
        check(registry, match_id)?;
        let intent = registry.stamp_intent(unit, channel, kind);
        (match_id, intent, registry.tasks().clone())
    };
    match tasks.offer(match_id, ArenaMsg::Intent(intent)) {
        Ok(()) => Ok(match_id),
        Err(_) if tasks.is_live(match_id) => Err(CmdError::msg(busy)),
        Err(_) => Err(CmdError::msg("That match has ended.")),
    }
}

/// `@arena/place`'s usage, after the `Usage: ` prefix.
const PLACE_USAGE: &str = "@arena/place <unit>=<x>,<y>";

/// The sentence a refused offer of a staff lever answers while its match's task
/// is live (16b cut I.3).
const BUSY: &str = "That match is busy; try again in a moment.";

/// **`@arena/place <unit>=<x>,<y>`** — **16b cut I.3** (16c N10): put a unit of
/// a running match on an arena cell, as the unit's own seat.
///
/// The world refuses what it can refuse honestly, against the ground the
/// registry holds for the match ([`ArenaRegistry::arena_of`]): a cell off the
/// window and a cell that is impassable or has no way out, in `@arena/pose`'s
/// own sentences. Those refusals, and a refused offer, **reach no tape** — the
/// arena never received the place. Nor does a match the book no longer holds
/// `running` take one (`Match N is stopping.`, as `@arena/tempo` says it): a
/// `Stop` is already on its way, and a place sent after it would reach a task
/// that steps no more. Occupancy and a round in flight are the arena's to judge
/// at the moment it resolves the place; the order rides the intent path as an
/// [`IntentKind::Place`] on `helm`, and **every place the arena receives is
/// journalled** — `placed`, or `place_refused` with the reason, `stopped` for
/// one that reaches the pulse a `Stop` ends the match in — while nothing is
/// ever said to a pilot. The one place that leaves no line is one queued in the
/// pulse whose own substep ends the match (a kill or the duration bound): no
/// line follows `match_ended` (16b cut W.2). The reply is the staffer's alone.
///
/// A staff surface, so it names object ids (doc 16 §5's observer rule is about
/// pilots).
///
/// # Errors
///
/// One sentence per refusal, in the order they are asked.
pub fn place(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let unit = parse_unit(ctx.args.lhs().trim()).ok_or(ArenaVerbError::Usage(PLACE_USAGE))?;
    let (x, y) = ctx
        .args
        .rhs()
        .and_then(parse_cell)
        .ok_or(ArenaVerbError::Usage(PLACE_USAGE))?;
    // The book's word on the unit's match, read before the registry is borrowed
    // (a match opened with no lifecycle record around it has none, and runs
    // exactly while its task does).
    let booked = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.match_of_unit(unit))
        .and_then(|match_id| {
            ctx.w
                .module_state::<MatchBook>()
                .and_then(|book| book.get(match_id))
                .map(|record| (match_id, record.state))
        });
    offer_as_seat(
        ctx,
        unit,
        IntentKind::Place { x, y },
        BUSY,
        |registry, match_id| {
            if let Some((booked_id, state)) = booked
                && booked_id == match_id
                && state != super::MatchState::Running
            {
                return Err(ArenaVerbError::WrongState {
                    match_id: match_id.0,
                    state: state.as_str(),
                }
                .into());
            }
            let Some(ground) = registry.arena_of(match_id) else {
                return Err(CmdError::msg("That unit is not in a running match."));
            };
            if ground.terrain.get(x, y).is_none() {
                return Err(CmdError::msg(format!(
                    "Cell ({x}, {y}) is off the arena window."
                )));
            }
            // 16b cut AE.1: the tracked reading, on purpose — this world-side
            // precheck knows no unit's row; the arena refuses the row's own case
            // (`Arena::place_refusal`, `impassable`) when the place lands.
            if !ground.terrain.has_way_out(x, y) {
                return Err(CmdError::msg(format!(
                    "Cell ({x}, {y}) is impassable, or has no way out."
                )));
            }
            Ok(())
        },
    )?;
    let actor = ctx.actor;
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        format!(
            "Placing #{} at cell ({x}, {y}): the arena answers at its next pulse and the tape \
             records it.",
            unit.0
        ),
    );
    Ok(())
}

/// One arena cell, `<x>,<y>` or `<x> <y>` (16b cut I.3).
fn parse_cell(rhs: &str) -> Option<(u16, u16)> {
    let words: Vec<&str> = rhs
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .collect();
    match words.as_slice() {
        [x, y] => Some((x.parse().ok()?, y.parse().ok()?)),
        _ => None,
    }
}

/// `@arena/tempo`'s usage, after the `Usage: ` prefix.
const TEMPO_USAGE: &str = "@arena/tempo <match>=<period_ms> | <match>=off | <match>";

/// **`@arena/tempo`** — **16b cut I.3** (16c N10): a floor under a running
/// match's wake interval. Three forms:
///
/// * `@arena/tempo <match>=<period_ms>` — the task wakes once per
///   `max(period, floor)` of wall time and each wake moves at most
///   `max_catch_up_steps` periods of the adaptive tempo's own (the clamp stays
///   the tempo's), which is slow motion when the floor is longer than that. A
///   floor is `tempo.engaged_ms` to [`crate::arena::clock::MAX_TEMPO_FLOOR_MS`]
///   in whole multiples of `engaged_ms`, so every figure the reply prints is
///   exactly what an on-time wake does.
/// * `@arena/tempo <match>=off` — the match runs at its own tempo again.
/// * `@arena/tempo <match>` — what stands.
///
/// **Never a pause** (16c §6): every wake moves arena time, and the tempo never
/// decides an outcome, so a floor under it cannot either; it is not hashed and
/// the arena records each change on the tape (`tempo_floor`). Pilots' orders wait
/// for the next wake. The floor is said to nobody but the caller, and
/// [`crate::arena_bridge::MatchMirror::tempo_floor_ms`] is written only after the
/// task took the message, so `@arena/status` never claims a floor the arena does
/// not have. Only a `running` match takes one.
///
/// # Errors
///
/// One sentence per refusal: the usage, the bounds, `There is no match N.`,
/// `Match N is <state>.`, and a busy match.
pub fn tempo(ctx: &mut Ctx<'_>, arena_cfg: &crate::config::ArenaConfig) -> Result<(), CmdError> {
    let match_id = MatchId(
        ctx.args
            .lhs()
            .trim()
            .parse::<u64>()
            .map_err(|_| ArenaVerbError::Usage(TEMPO_USAGE))?,
    );
    let engaged = arena_cfg.tempo.engaged_ms;
    let ceiling = crate::arena::clock::MAX_TEMPO_FLOOR_MS;
    // `None` is the bare read; `Some(None)` the lift; `Some(Some(ms))` a floor.
    let asked: Option<Option<u32>> = match ctx.args.rhs().map(str::trim) {
        None | Some("") => None,
        Some(word) if word.eq_ignore_ascii_case("off") => Some(None),
        Some(word) => {
            let ms = word
                .parse::<u32>()
                .map_err(|_| ArenaVerbError::Usage(TEMPO_USAGE))?;
            if engaged == 0 || ms < engaged || ms > ceiling || ms % engaged != 0 {
                return Err(CmdError::msg(format!(
                    "A tempo floor is {engaged} to {ceiling} ms in steps of {engaged}, or off."
                )));
            }
            Some(Some(ms))
        }
    };
    // The match, and its state: only a running match has a wake to floor.
    let state = ctx
        .w
        .module_state::<MatchBook>()
        .and_then(|book| book.get(match_id))
        .map(|record| record.state);
    let live = ctx
        .w
        .module_state::<ArenaRegistry>()
        .is_some_and(|registry| registry.tasks().is_live(match_id));
    match state {
        Some(super::MatchState::Running) => {}
        Some(state) => {
            return Err(ArenaVerbError::WrongState {
                match_id: match_id.0,
                state: state.as_str(),
            }
            .into());
        }
        // A match opened straight through `open_match`, with no lifecycle
        // record around it, is running exactly while its task is.
        None if live => {}
        None => return Err(ArenaVerbError::NoMatch(match_id.0).into()),
    }
    let actor = ctx.actor;
    let Some(floor) = asked else {
        let standing = ctx
            .w
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.mirror(match_id))
            .and_then(|mirror| mirror.tempo_floor_ms);
        let line = match standing {
            Some(floor_ms) => floor_line(match_id, floor_ms, arena_cfg),
            None => format!("Match {match_id} has no tempo floor."),
        };
        ctx.out
            .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, line);
        return Ok(());
    };
    let tasks = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .ok_or(ArenaVerbError::Unavailable)?;
    if tasks
        .offer(match_id, ArenaMsg::TempoFloor { period_ms: floor })
        .is_err()
    {
        return Err(CmdError::msg(if tasks.is_live(match_id) {
            BUSY
        } else {
            "That match has ended."
        }));
    }
    // Only now, with the message in the task's inbox, does the mirror say so.
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.note_tempo_floor(match_id, floor);
    }
    let line = match floor {
        Some(floor_ms) => floor_line(match_id, floor_ms, arena_cfg),
        None => format!("Match {match_id} runs at its own tempo again."),
    };
    ctx.out
        .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, line);
    Ok(())
}

/// **What a floor does at each tempo**, as `@arena/tempo` says it (16b cut
/// I.3): per tempo, the wake is `max(period, floor)` of wall time and an
/// on-time wake moves `min(max_catch_up_steps × period, wake)` of arena time —
/// exact, because a floor is a whole multiple of the engaged period.
fn floor_line(match_id: MatchId, floor_ms: u32, arena_cfg: &crate::config::ArenaConfig) -> String {
    use crate::arena::clock::Tempo;
    let steps = arena_cfg.max_catch_up_steps.max(1);
    let at = |tempo: Tempo| -> (u32, u32) {
        let period = tempo.period_ms(&arena_cfg.tempo);
        let wake = period.max(floor_ms);
        (period.saturating_mul(steps).min(wake), wake)
    };
    let (engaged_moves, engaged_wake) = at(Tempo::Engaged);
    let (alert_moves, alert_wake) = at(Tempo::Alert);
    let (quiet_moves, quiet_wake) = at(Tempo::Quiet);
    format!(
        "Match {match_id} under a floor of {floor_ms} ms: engaged {engaged_moves} ms of arena \
         time a wake every {engaged_wake} ms, alert {alert_moves} every {alert_wake}, quiet \
         {quiet_moves} every {quiet_wake}; orders wait for the next wake. @arena/tempo \
         {match_id}=off lifts it."
    )
}

/// `@arena/watch`'s usage, after the `Usage: ` prefix.
const WATCH_USAGE: &str = "@arena/watch <match> | off";

/// **`@arena/watch`** — **16b cut I.4** (16c N10): the referee's third lever,
/// the field of a running match on the staffer's panel. Three forms:
///
/// * `@arena/watch <match>` — [`crate::arena_bridge::watch`]'s checks and
///   binding; the staffer's panel is sent the match's whole field on
///   `arena.observer` at most once an arena second, and screen 0 of the TUI
///   and the web draws it. The line client prints none of it (it has no
///   shape), and the reply says so.
/// * `@arena/watch off` — [`crate::arena_bridge::unwatch`].
/// * `@arena/watch` — what the staffer watches.
///
/// # Errors
///
/// The usage line, and [`crate::arena_bridge::watch`]'s refusals.
pub fn watch(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if ctx.args.rhs().is_some() {
        return Err(ArenaVerbError::Usage(WATCH_USAGE).into());
    }
    let word = ctx.args.lhs().trim().to_owned();
    let line = if word.is_empty() {
        match crate::arena_bridge::watching(ctx.w, actor) {
            Some(match_id) => watching_line(match_id),
            None => "You are watching nothing.".to_owned(),
        }
    } else if word.eq_ignore_ascii_case("off") {
        match crate::arena_bridge::unwatch(ctx, actor) {
            Some(match_id) => format!("You stop watching match {match_id}."),
            None => "You are watching nothing.".to_owned(),
        }
    } else {
        let match_id = MatchId(
            word.parse::<u64>()
                .map_err(|_| ArenaVerbError::Usage(WATCH_USAGE))?,
        );
        crate::arena_bridge::watch(ctx, actor, match_id)?;
        watching_line(match_id)
    };
    ctx.out
        .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, line);
    Ok(())
}

/// What a watching staffer is told, on the watch and on the bare read.
fn watching_line(match_id: MatchId) -> String {
    format!(
        "You are watching match {match_id}: its picture reaches your panel at most once an \
         arena second on arena.observer, and screen 0 draws it; the line client prints none \
         of it. @arena/watch off stops it."
    )
}

/// `@arena/abort [<match>]` — stop a match; with no argument, the only one
/// running.
pub fn abort(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let named = ctx.args.lhs().trim();
    let match_id = if named.is_empty() {
        let live: Vec<MatchId> = ctx
            .w
            .module_state::<MatchBook>()
            .map(|book| {
                book.iter()
                    .filter(|(_, record)| record.state.is_live())
                    .map(|(id, _)| id)
                    .collect()
            })
            .unwrap_or_default();
        match live.as_slice() {
            [one] => *one,
            [] => return Err(CmdError::msg("No match is running.")),
            _ => {
                return Err(CmdError::msg(
                    "More than one match is running; say which: @arena/abort <match>.",
                ));
            }
        }
    } else {
        MatchId(
            named
                .parse::<u64>()
                .map_err(|_| ArenaVerbError::Usage("@arena/abort [<match>]"))?,
        )
    };
    let state = ctx
        .w
        .module_state::<MatchBook>()
        .and_then(|book| book.get(match_id))
        .map(|record| record.state)
        .ok_or(ArenaVerbError::NoMatch(match_id.0))?;
    if !state.may_abort() {
        return Err(ArenaVerbError::WrongState {
            match_id: match_id.0,
            state: state.as_str(),
        }
        .into());
    }
    // 16b cut M.1 (`B-49`): a busy arena changes nothing; the operator asks again.
    if super::abort(ctx, match_id, StopReason::Abort) == super::AbortOutcome::Busy {
        return Err(CmdError::msg(format!(
            "Match {match_id} is busy; try @arena/abort again in a moment."
        )));
    }
    let actor = ctx.actor;
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        format!("Match {match_id} is stopping; its units check back in when it does."),
    );
    Ok(())
}

/// `@arena/reset` — **16b cut K.1**: return the simulator centre to `Idle`.
///
/// Doc 16 §3.8 asks for a reset that is *idempotent and re-runnable by staff*,
/// and this is it. An ordinary end already released the seats, leases, focus and
/// pods in the check-in; what the centre keeps afterwards is the **terminal
/// row** — the match id and how it went — so an operator can see what happened
/// before the next crew walks in. This clears it, and clears anything a failure
/// left behind with it.
///
/// A centre that is still holding a live match is refused rather than cleared:
/// the units are checked out and only the check-in can bring them back, which is
/// `@arena/abort`'s job.
///
/// # Errors
///
/// A message when a match is still running.
pub fn reset(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if super::lobby::centre(ctx.w).is_some_and(super::Centre::is_busy) {
        return Err(CmdError::msg(
            "That centre is holding a match; `@arena/abort` it first — its units are checked out \
             and only the check-in gives them back.",
        ));
    }
    let outcome = super::lobby::reset(ctx);
    // **16b cut U.4 (16c `T-52`)**: and every `@arena/pose` override, whether a
    // match has used it or not. The reset is staff's "start from the pack
    // again", and an opening cell is the one thing staff set that neither the
    // centre's return to `Idle` nor a new match clears on its own. Counted
    // first, so that a reset whose only work was the overrides does not answer
    // "nothing changed" over a book it just emptied.
    let overrides = ctx.w.module_state_mut::<MatchBook>().map_or(0, |book| {
        let standing = book.opening_cells().len();
        book.clear_opening_cells();
        standing
    });
    let actor = ctx.actor;
    // **16b cut D.3 (`B-31`)**: three headlines, not two. A reset that wrote
    // nothing because every tank it wanted to write was still deployed, or
    // refused, said "already idle; nothing changed" and then printed a row per
    // stuck tank underneath it — the headline contradicting the report it was
    // the headline of. The middle answer is the honest one.
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        if outcome.changed() || (outcome.is_empty() && overrides > 0) {
            "The simulator centre is reset; its slots and pods are free."
        } else if outcome.is_empty() {
            "The simulator centre is already idle; nothing changed."
        } else {
            "The simulator centre is idle, but not clean:"
        },
    );
    // **16b cut F.1**: and what it actually found. The sweep now runs even on an
    // already-idle centre — which is exactly the state a restart mid-match leaves
    // behind, with a body still in a pod and a billet still held — so the verb has
    // to be able to say "idle, and I freed two pods" rather than "nothing
    // changed".
    if !outcome.ejected.is_empty() {
        let line = format!(
            "{} pod(s) opened and {} cockpit billet(s) given back.",
            outcome.ejected.bodies.len(),
            outcome.ejected.seats.len()
        );
        ctx.out
            .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, line);
    }
    if outcome.tanks.changed() {
        // **16b cut Q.8**: and how many were put back to their own TYPE. A refit
        // is a re-instantiation and not a plan, so it is in neither count above
        // — and without this the line could read "0 tank(s) back at spec, 0 back
        // at muster" over a pass that rewrote three documents. Empty, and the
        // sentence byte-identical, when nothing was refitted.
        let refits = match outcome.tanks.refitted {
            0 => String::new(),
            n => format!(", {n} back to their own type"),
        };
        let line = format!(
            "{} tank(s) back at spec, {} back at muster{refits}.",
            outcome.tanks.restored.len(),
            outcome.tanks.remustered.len()
        );
        ctx.out
            .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, line);
    }
    for (unit, why) in &outcome.tanks.failed {
        let line = format!("#{} could not be reset: {why}", unit.0);
        ctx.out
            .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, line);
    }
    for unit in &outcome.tanks.deployed {
        let line = format!(
            "#{} is still deployed and was left alone; `@arena/abort` its match first.",
            unit.0
        );
        ctx.out
            .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, line);
    }
    // **16b cut U.4**: and the `@arena/pose` overrides it dropped, said last so
    // the "idle, but not clean:" rows stay under their headline. Silent when
    // there were none.
    if overrides > 0 {
        ctx.out.sys_to_actor(
            actor,
            crate::arena_bridge::NOTICE_CATEGORY,
            format!("{overrides} opening cell override(s) cleared."),
        );
    }
    Ok(())
}

/// `@arena/pose`'s usage, after the `Usage: ` prefix [`ArenaVerbError::Usage`]
/// renders.
const POSE_USAGE: &str =
    "@arena/pose <callsign>=<x> <y>[ <heading>] | <callsign>=off | (bare) list";

/// **`@arena/pose`** — **16b cut U.4 (16c `T-52`)**: choose the cell a centre
/// slot's unit opens the **next match** on.
///
/// Three forms, staff like every `@arena` switch:
///
/// * `@arena/pose <callsign>=<x> <y>[ <heading>]` — the slot's callsign as the
///   board prints it (`RA`, `BC`: the team letter and the slot letter,
///   case-folded), an arena cell on the centre's own arena, and a heading in
///   whole degrees (the side's facing when left out: red 90, blue 270, the entry
///   zone's own).
/// * `@arena/pose <callsign>=off` — the slot opens on its authored cell again.
/// * bare `@arena/pose` — the overrides standing, one line each.
///
/// Before it, the one way to open a match anywhere but the authored cells was a
/// raw-JSON `@spatial/place` of the tank, which every return to `Idle` undid —
/// and a lone claimant's `SIM TEAM` on a terminal centre *is* such a return. So
/// the override lives on the match book beside the centre
/// ([`MatchBook::opening_cells`]), rides the slot's [`super::Fielded`] into the
/// check-out, and is offered to [`super::opening_pose`] as the world pose — held
/// to exactly what a staff placement is held to. It is cleared by the end of the
/// match that used it, by `@arena/reset`, and per slot by `off`.
///
/// Settable in every state but `Running` and `Stopping`: `Idle`, `Selection`,
/// `Boarding`, and the terminal states staff stand in between matches. Checked
/// against [`super::CENTRE_ARENA`], because a centre in `Idle` names no arena at
/// all. The other overrides are checked for the same cell here; the authored
/// cells are the check-out's business.
///
/// A staff surface naming a slot and a cell staff chose; nothing reaches a pilot.
///
/// # Errors
///
/// One sentence per refusal, in the order they are asked.
pub fn pose(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let word = ctx.args.lhs().trim().to_owned();
    let rhs = ctx.args.rhs().map(str::trim).map(str::to_owned);
    let actor = ctx.actor;
    // The bare read.
    if word.is_empty() && rhs.as_deref().is_none_or(str::is_empty) {
        let rows: Vec<String> = ctx
            .w
            .module_state::<MatchBook>()
            .map(|book| {
                book.opening_cells()
                    .iter()
                    .map(|((team, slot), cell)| {
                        format!(
                            "  {}: cell ({}, {}) heading {}",
                            callsign(*team, *slot),
                            cell.x,
                            cell.y,
                            cell.heading_mdeg / 1_000
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if rows.is_empty() {
            ctx.out.sys_to_actor(
                actor,
                crate::arena_bridge::NOTICE_CATEGORY,
                "No opening cell is overridden.",
            );
        } else {
            ctx.out.sys_to_actor(
                actor,
                crate::arena_bridge::NOTICE_CATEGORY,
                "Opening cells overridden for the next match:",
            );
            for row in rows {
                ctx.out
                    .sys_to_actor(actor, crate::arena_bridge::NOTICE_CATEGORY, row);
            }
        }
        return Ok(());
    }
    let Some(rhs) = rhs.filter(|rhs| !rhs.is_empty()) else {
        return Err(ArenaVerbError::Usage(POSE_USAGE).into());
    };
    if word.is_empty() {
        return Err(ArenaVerbError::Usage(POSE_USAGE).into());
    }
    let named = word.to_ascii_uppercase();
    // `off`, before the numbers are parsed.
    if rhs.eq_ignore_ascii_case("off") {
        let (team, slot) = slot_of_callsign(ctx.w, &named)?;
        refuse_while_running(ctx.w)?;
        if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
            book.clear_opening_cell(team, slot);
        }
        ctx.out.sys_to_actor(
            actor,
            crate::arena_bridge::NOTICE_CATEGORY,
            format!("{} opens at its authored cell again.", callsign(team, slot)),
        );
        return Ok(());
    }
    // `<x> <y>[ <heading>]`: whole numbers, or it is not this form at all.
    let numbers: Vec<&str> = rhs.split_whitespace().collect();
    let (x, y, heading) = match numbers.as_slice() {
        [x, y] => (x.parse::<u16>().ok(), y.parse::<u16>().ok(), None),
        [x, y, heading] => (
            x.parse::<u16>().ok(),
            y.parse::<u16>().ok(),
            Some(heading.parse::<i64>().ok()),
        ),
        _ => (None, None, None),
    };
    let (Some(x), Some(y)) = (x, y) else {
        return Err(ArenaVerbError::Usage(POSE_USAGE).into());
    };
    let heading = match heading {
        None => None,
        Some(Some(degrees)) => Some(degrees),
        Some(None) => return Err(ArenaVerbError::Usage(POSE_USAGE).into()),
    };
    // (1) The slot.
    let (team, slot) = slot_of_callsign(ctx.w, &named)?;
    // (2) and (3) The cell, on the centre's own arena.
    let definition = super::definition_for(ctx.w, &SmolStr::new_static(super::CENTRE_ARENA))
        .map_err(CmdError::msg)?
        .ok_or(ArenaVerbError::Unavailable)?;
    if definition.terrain.get(x, y).is_none() {
        return Err(CmdError::msg(format!(
            "Cell ({x}, {y}) is off the arena window."
        )));
    }
    // 16b cut AE.1: the tracked reading, on purpose — this precheck knows no
    // unit's row; `opening_pose` judges the override on the unit's own row at
    // the start and falls back to the zone when that row cannot use it.
    if !definition.terrain.has_way_out(x, y) {
        return Err(CmdError::msg(format!(
            "Cell ({x}, {y}) is impassable, or has no way out."
        )));
    }
    // (4) Another override on the same cell.
    let clash = ctx.w.module_state::<MatchBook>().and_then(|book| {
        book.opening_cells()
            .iter()
            .find(|(key, cell)| **key != (team, slot) && cell.x == x && cell.y == y)
            .map(|((other_team, other_slot), _)| callsign(*other_team, *other_slot))
    });
    if let Some(other) = clash {
        return Err(CmdError::msg(format!(
            "Cell ({x}, {y}) is already {other}'s opening cell."
        )));
    }
    // (5) The heading.
    let degrees = match heading {
        Some(degrees) => u32::try_from(degrees)
            .ok()
            .filter(|degrees| *degrees < 360)
            .ok_or_else(|| CmdError::msg("Heading is 0 to 359."))?,
        // Facing the other side, as the entry zone's own walk does.
        None => match team {
            Team::Red => 90,
            Team::Blue => 270,
        },
    };
    // (6) The state.
    refuse_while_running(ctx.w)?;
    let cell = super::OpeningCell {
        x,
        y,
        heading_mdeg: degrees.saturating_mul(1_000),
    };
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.set_opening_cell(team, slot, cell);
    }
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        format!(
            "{} opens at cell ({x}, {y}) heading {degrees} for the next match.",
            callsign(team, slot)
        ),
    );
    Ok(())
}

/// A centre slot's callsign as the board prints it: the team letter and the slot
/// letter ([`Designator::Slot`]'s own rendering, so the two cannot differ).
fn callsign(team: Team, slot: u8) -> String {
    Designator::Slot {
        team,
        slot: u16::from(slot),
    }
    .to_string()
}

/// The `(team, slot)` a typed callsign names on this centre — a pod the centre's
/// content actually made — or the one refusal for any other word.
fn slot_of_callsign(w: &mush_core::World, named: &str) -> Result<(Team, u8), CmdError> {
    super::POD_ENTRIES
        .iter()
        .map(|(team, slot, _)| (*team, *slot))
        .find(|(team, slot)| {
            callsign(*team, *slot) == named && super::pod_object(w, *team, *slot).is_some()
        })
        .ok_or_else(|| CmdError::msg(format!("There is no slot {named} on this centre.")))
}

/// Refuse a change to the overrides while the centre holds a running match: its
/// units are checked out, and a cell chosen now would apply to nothing.
fn refuse_while_running(w: &mush_core::World) -> Result<(), CmdError> {
    if super::lobby::centre(w).is_some_and(|centre| centre.state.is_live()) {
        return Err(CmdError::msg(
            "The simulator centre is running; set opening cells before the match starts.",
        ));
    }
    Ok(())
}

/// `@arena/release <unit>` — **16b cut D.3 (`T-49`)**: clear one unit's stranded
/// `deployed` marker and give its cockpit seat back.
///
/// Until this cut nothing did. A check-in that the strict encoder refused, or a
/// rolled-back check-out that could not be undone, left the marker on the
/// record — and the marker refuses every ordinary mutation, so that tank could
/// not be reset, refitted, fielded or boarded again. The only advice the logs
/// could honestly give was *restart the server half*, because the boot
/// reconciliation was the one thing in the tree that cleared one.
///
/// It is the void check-in: the unit is restored from the snapshot its own
/// marker carries and the marker is cleared, in the same authoritative action as
/// [`crate::match_::release_seat`], which is what stops the repair leaving a
/// half-released pod behind.
///
/// **Refused while the unit's match is live**, because a checked-out unit's
/// marker is not stranded — the arena is its single writer, and `@arena/abort` is
/// the verb that ends that. Refused for a unit that carries no marker at all,
/// because there is nothing to repair and answering "released" would be a lie.
///
/// Staff surfaces may name object ids (doc 16 §5's observer rule is about
/// pilots), so every sentence here does.
///
/// # Errors
///
/// A message naming the unit: a live match, no marker, or whatever the check-in
/// itself refused.
pub fn release(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "@arena/release <unit>";
    let unit = parse_unit(ctx.args.lhs().trim()).ok_or(ArenaVerbError::Usage(USAGE))?;
    if let Some(match_id) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.match_of_unit(unit))
    {
        return Err(CmdError::msg(format!(
            "#{} is in match {match_id}, which is still live; `@arena/abort` it first.",
            unit.0
        )));
    }
    if !mush_sim::deployed::is_deployed(ctx.w, unit) {
        return Err(CmdError::msg(format!(
            "#{} carries no deployed marker.",
            unit.0
        )));
    }
    mush_tactics::custody::check_in(ctx, unit, None)
        .map_err(|error| CmdError::msg(error.to_string()))?;
    super::release_seat(ctx, unit);
    let actor = ctx.actor;
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        format!(
            "#{} is released: its check-out snapshot is restored and its seat is free.",
            unit.0
        ),
    );
    Ok(())
}

/// `@arena/fault <match>` — **test builds only**: make the next `step` of that
/// match's arena panic.
///
/// Doc 16 §3.1 "Failure" promises that a panic inside `step` voids the match, no
/// cell the faulted pulse touched reaches the store, and the world keeps serving
/// commands. That is a property of the *running server*, and the only honest way
/// to show it on one is to make a running server's arena panic — which no legal
/// input can do.
///
/// So this is the same shape `mush-server`'s `process_fault` uses for the K8 cuts:
/// a seam behind a cargo feature that is off by default, that no shipped binary
/// turns on, and that `mushd` and `mush-gateway` enable through a
/// `[dev-dependencies]` edge for the two tests that need it.
///
/// It reaches the arena through cut T.2's own hook, which is armed **by seed**
/// (`Arena::panic_on_next_step_for_tests` stores `self.seed` in a static). The
/// running arena lives inside its task and no reference to it crosses the seam —
/// deliberately, D-16-1 — so this arms the seed from a throwaway arena built on
/// the same seed, which is the one thing the hook's own shape allows without
/// opening a second door into a live match.
#[cfg(any(test, feature = "fault-injection"))]
pub fn fault(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let named = ctx.args.lhs().trim();
    let match_id = MatchId(
        named
            .parse::<u64>()
            .map_err(|_| ArenaVerbError::Usage("@arena/fault <match>"))?,
    );
    let (seed, unit) = ctx
        .w
        .module_state::<MatchBook>()
        .and_then(|book| book.get(match_id))
        .filter(|record| record.state.is_live())
        .and_then(|record| record.units().first().map(|unit| (record.seed, *unit)))
        .ok_or(ArenaVerbError::NoMatch(match_id.0))?;
    let cells = mush_tactics::custody::marker_of(ctx.w, unit)
        .map(|marker| marker.snapshot.clone())
        .ok_or(ArenaVerbError::NoMatch(match_id.0))?;
    arm_panic_for_seed(unit, cells, seed).map_err(CmdError::msg)?;
    let actor = ctx.actor;
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        format!("Match {match_id}'s next arena pulse will panic."),
    );
    Ok(())
}

/// Arm cut T.2's panic hook for one match seed.
///
/// The hook takes `&mut Arena` and reads `self.seed`, and nothing under
/// `crates/tacsim/src/arena/` may change, so the arming goes through a throwaway
/// arena carrying the same seed: a one-cell window holding **the match's own
/// first unit**, taken from the `deployed` marker's check-out snapshot, built
/// and dropped inside this function. Using the real unit rather than an invented
/// one is what keeps this off the list of things that drift: there is no second
/// spec here to go stale. It touches no state and no store, and it is compiled
/// only into test and fault-injection builds.
#[cfg(any(test, feature = "fault-injection"))]
fn arm_panic_for_seed(
    unit: ObjId,
    cells: mush_tactics::TacticalUnit,
    seed: u64,
) -> Result<(), String> {
    use crate::arena::state::{
        Arena, ArenaDefinition, ArenaTerrain, CustodyOut, CustodyUnit, Team, TerrainCell,
        TrackedRules,
    };
    let terrain = ArenaTerrain::uniform(
        1,
        1,
        1_000,
        TerrainCell {
            class: mush_atlas::TerrainClass::Ground,
            elev_cm: 0,
            move_cost: 100,
        },
    )
    .map_err(|error| error.to_string())?;
    let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
        .map_err(|error| error.to_string())?;
    let mut arena = Arena::new(
        CustodyOut {
            definition: ArenaDefinition {
                id: SmolStr::new_static("tacsim.arena.fault"),
                terrain,
                entry_zones: Vec::new(),
                key_spots: Vec::new(),
                waypoints: crate::arena::state::Waypoints::default(),
                modes: Vec::new(),
            },
            units: vec![CustodyUnit {
                id: unit,
                team: Team::Red,
                pose: mush_sim::data::Pose::at(500, 500, 0, posture),
                mobility: TrackedRules::default(),
                sensor_height_cm: 0,
                sensor: None,
                weapons: Vec::new(),
                profile: crate::arena::resolution::UnitProfile::default(),
                gunnery: 0,
                controller: crate::arena::state::Controller::Pilot,
                // 16b cut Q.1: no pod behind it.
                slot: 0,
                unit: cells,
            }],
            pilots: Vec::new(),
            resolution: crate::match_::stock_resolution(),
            seed,
            mode: crate::arena::state::MatchMode::Skirmish,
        },
        &crate::config::ArenaConfig::default(),
        &crate::config::MatchConfig::default(),
        &crate::config::SensorsConfig::default(),
    )
    .map_err(|error| error.to_string())?;
    arena.panic_on_next_step_for_tests();
    Ok(())
}

/// `@arena/status`'s per-match lifecycle word, folded into cut T.2's row.
///
/// `None` when the book holds no record for that match — which is not the same
/// as `Idle`. A match opened straight through
/// [`crate::arena_bridge::open_match`], without the lifecycle around it, is
/// running with no record; the caller then falls back to what the shared task
/// table says, which is the only thing that is true about it.
#[must_use]
pub fn state_word(w: &mush_core::World, match_id: MatchId) -> Option<&'static str> {
    w.module_state::<MatchBook>()
        .and_then(|book| book.get(match_id))
        .map(|record| record.state.as_str())
}

/// Parse a `/`-side's unit list: dbrefs, `#`-prefixed or not, comma or space
/// separated.
///
/// Deliberately **dbref only**, never a name match: a name lookup leaks
/// visibility, and this is a staff verb whose whole job is to name objects
/// exactly.
fn parse_units(list: &str) -> Result<Vec<ObjId>, CmdError> {
    let mut units = Vec::new();
    for word in list
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|word| !word.is_empty())
    {
        units.push(
            parse_unit(word).ok_or_else(|| CmdError::msg(format!("`{word}` is not a dbref.")))?,
        );
    }
    Ok(units)
}

/// One dbref.
fn parse_unit(word: &str) -> Option<ObjId> {
    word.trim()
        .trim_start_matches('#')
        .parse::<u32>()
        .ok()
        .map(ObjId)
}

/// One order, from the staff spelling.
fn parse_kind(rhs: &str) -> Option<IntentKind> {
    let mut words = rhs.split_whitespace();
    let verb = words.next()?.to_ascii_lowercase();
    let argument = words.next();
    match verb.as_str() {
        "startup" => Some(IntentKind::Startup),
        "heading" => {
            let degrees = argument?.parse::<u32>().ok()?;
            (degrees < 360).then(|| IntentKind::Heading {
                mdeg: degrees.saturating_mul(1_000),
            })
        }
        "speed" => Some(IntentKind::Speed {
            cms: argument?.parse::<i32>().ok()?,
        }),
        _ => None,
    }
}

/// The whole `@arena` verb: cut T.2's read plus this cut's three lifecycle
/// switches.
///
/// # Errors
///
/// Whatever the switch refused, rendered.
pub fn arena_command(
    ctx: &mut Ctx<'_>,
    arena_cfg: &crate::config::ArenaConfig,
    match_cfg: &crate::config::MatchConfig,
    sensors_cfg: &crate::config::SensorsConfig,
) -> Result<(), CmdError> {
    if ctx.switches.has("start") {
        start(ctx, arena_cfg, match_cfg, sensors_cfg)
    } else if ctx.switches.has("intent") {
        intent(ctx)
    } else if ctx.switches.has("abort") {
        abort(ctx)
    } else if ctx.switches.has("reset") {
        reset(ctx)
    } else if ctx.switches.has("pose") {
        // 16b cut U.4 (`T-52`).
        pose(ctx)
    } else if ctx.switches.has("release") {
        // 16b cut D.3 (`T-49`).
        release(ctx)
    } else if ctx.switches.has("report") {
        // 16b cut V.3 (16c N2): a page of the tape, for staff. The verb's own
        // staff gate has already run; no centre gate applies.
        let args = ctx.args.raw().to_owned();
        super::report::request_page(ctx, &args)
    } else if ctx.switches.has("tempo") {
        // 16b cut I.3 (16c N10): the floor under the wake interval.
        tempo(ctx, arena_cfg)
    } else if ctx.switches.has("place") {
        // 16b cut I.3 (16c N10): a unit moved by staff, as its own seat.
        place(ctx)
    } else if ctx.switches.has("watch") {
        // 16b cut I.4 (16c N10): the field of a running match, on the panel.
        watch(ctx)
    } else if cfg!(any(test, feature = "fault-injection")) && ctx.switches.has("fault") {
        #[cfg(any(test, feature = "fault-injection"))]
        {
            fault(ctx)
        }
        #[cfg(not(any(test, feature = "fault-injection")))]
        {
            Err(CmdError::msg("Permission denied."))
        }
    } else {
        crate::arena_bridge::status(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_staff_order_spellings_are_the_three_doc_16_names() {
        assert_eq!(parse_kind("startup"), Some(IntentKind::Startup));
        assert_eq!(
            parse_kind("heading 90"),
            Some(IntentKind::Heading { mdeg: 90_000 })
        );
        assert_eq!(
            parse_kind("speed -400"),
            Some(IntentKind::Speed { cms: -400 })
        );
        assert_eq!(parse_kind("heading 360"), None, "a bearing is 0..359");
        assert_eq!(parse_kind("fire cannon"), None, "cut W's, not this cut's");
        assert_eq!(parse_kind(""), None);
    }

    /// **16b cut I.3**: a cell is two whole numbers, by a comma or a space.
    #[test]
    fn i3_a_place_cell_is_two_numbers_by_a_comma_or_a_space() {
        assert_eq!(parse_cell("120,97"), Some((120, 97)));
        assert_eq!(parse_cell("120, 97"), Some((120, 97)));
        assert_eq!(parse_cell(" 120 97 "), Some((120, 97)));
        assert_eq!(parse_cell("120"), None);
        assert_eq!(parse_cell("120,97,3"), None);
        assert_eq!(parse_cell("-1,4"), None);
        assert_eq!(parse_cell("a,b"), None);
        assert_eq!(parse_cell(""), None);
    }

    /// **16b cut I.3**: the floor's reply is exact at the shipped tempo table —
    /// a 5,000 ms floor is slow motion at `Engaged` and `Alert` and nothing at
    /// `Quiet`.
    #[test]
    fn i3_the_floor_reply_says_all_three_tempos() {
        let cfg = crate::config::ArenaConfig::default();
        assert_eq!(
            floor_line(MatchId(12), 5_000, &cfg),
            "Match 12 under a floor of 5000 ms: engaged 2000 ms of arena time a wake every 5000 \
             ms, alert 4000 every 5000, quiet 5000 every 5000; orders wait for the next wake. \
             @arena/tempo 12=off lifts it."
        );
        // A floor under a tempo's own period coarsens nothing there.
        assert_eq!(
            floor_line(MatchId(3), 500, &cfg),
            "Match 3 under a floor of 500 ms: engaged 500 ms of arena time a wake every 500 ms, \
             alert 1000 every 1000, quiet 5000 every 5000; orders wait for the next wake. \
             @arena/tempo 3=off lifts it."
        );
    }

    /// **16b cut I.4**: the watching reply, verbatim.
    #[test]
    fn i4_the_watching_reply_says_where_the_picture_goes() {
        assert_eq!(
            watching_line(MatchId(12)),
            "You are watching match 12: its picture reaches your panel at most once an arena \
             second on arena.observer, and screen 0 draws it; the line client prints none of \
             it. @arena/watch off stops it."
        );
    }

    #[test]
    fn unit_lists_are_dbrefs_and_nothing_else() {
        assert_eq!(
            parse_units("#3, 4  5").ok(),
            Some(vec![ObjId(3), ObjId(4), ObjId(5)])
        );
        assert!(parse_units("Tank").is_err(), "a name is not a dbref");
        assert_eq!(parse_units("").ok(), Some(Vec::new()));
    }

    #[test]
    fn the_fault_switch_is_only_registered_where_it_is_compiled() {
        let switches = register_switches();
        assert!(switches.iter().any(|switch| switch == "start"));
        assert!(switches.iter().any(|switch| switch == "intent"));
        assert!(switches.iter().any(|switch| switch == "abort"));
        // 16b cut V.3: the staff page of the tape.
        assert!(switches.iter().any(|switch| switch == "report"));
        // 16b cut I.3: the two referee's levers, after it, in help order;
        // 16b cut I.4: and the third, `watch`, after them.
        let order: Vec<&str> = switches.iter().map(SmolStr::as_str).collect();
        assert_eq!(
            order.get(..11),
            Some(
                &[
                    "status", "start", "intent", "abort", "reset", "release", "pose", "report",
                    "tempo", "place", "watch"
                ][..]
            )
        );
        assert_eq!(
            switches.iter().any(|switch| switch == "fault"),
            cfg!(any(test, feature = "fault-injection"))
        );
    }
}
