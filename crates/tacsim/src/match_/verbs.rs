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
use crate::arena::state::StopReason;
use crate::arena_bridge::ArenaRegistry;
use crate::arena_task::{ArenaMsg, MatchId};

use super::{MatchBook, Side, allocate_id};

/// The switches cut T.3 adds to `@arena`, in help order (16b cut K.1 adds
/// `reset`).
#[must_use]
pub fn register_switches() -> Vec<SmolStr> {
    let mut switches: Vec<SmolStr> = ["status", "start", "intent", "abort", "reset"]
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
    let sides = vec![
        Side {
            team: crate::arena::state::Team::Red,
            units: parse_units(red)?,
        },
        Side {
            team: crate::arena::state::Team::Blue,
            units: parse_units(blue)?,
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
    super::start(
        ctx,
        match_id,
        &arena,
        &sides,
        arena_cfg,
        match_cfg,
        sensors_cfg,
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
    let channel = kind.channel_required();
    let (match_id, intent, tasks) = {
        let registry = ctx
            .w
            .module_state_mut::<ArenaRegistry>()
            .ok_or(ArenaVerbError::Unavailable)?;
        let match_id = registry
            .match_of_unit(unit)
            .ok_or_else(|| CmdError::msg("That unit is not in a running match."))?;
        let intent = registry.stamp_intent(unit, channel, kind);
        (match_id, intent, registry.tasks().clone())
    };
    match tasks.offer(match_id, ArenaMsg::Intent(intent)) {
        Ok(()) => {
            let actor = ctx.actor;
            ctx.out.sys_to_actor(
                actor,
                crate::arena_bridge::NOTICE_CATEGORY,
                format!("Ordered #{} on match {match_id}.", unit.0),
            );
            Ok(())
        }
        Err(_) if tasks.is_live(match_id) => Err(CmdError::msg(
            "That unit already has as many orders queued as it may.",
        )),
        Err(_) => Err(CmdError::msg("That match has ended.")),
    }
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
    super::abort(ctx, match_id, StopReason::Abort);
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
    let changed = super::lobby::reset(ctx);
    let actor = ctx.actor;
    ctx.out.sys_to_actor(
        actor,
        crate::arena_bridge::NOTICE_CATEGORY,
        if changed {
            "The simulator centre is reset; its slots and pods are free."
        } else {
            "The simulator centre is already idle; nothing changed."
        },
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
            },
            units: vec![CustodyUnit {
                id: unit,
                team: Team::Red,
                pose: mush_sim::data::Pose::at(500, 500, 0, posture),
                mobility: TrackedRules::default(),
                sensor_height_cm: 0,
                sensor: None,
                weapons: Vec::new(),
                unit: cells,
            }],
            pilots: Vec::new(),
            seed,
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
        assert_eq!(
            switches.iter().any(|switch| switch == "fault"),
            cfg!(any(test, feature = "fault-injection"))
        );
    }
}
