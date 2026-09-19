//! **`SIM`, the lobby's one verb, and the gate that gives it a place** (16b cut
//! K.1; doc 16 §3.7 "Verbs", §3.8 "Lifecycle").
//!
//! ```text
//! SIM TEAM red | blue     claim one slot on a side
//! SIM LEAVE               give it up
//! SIM START               lock the pods and open boarding (a side's first pilot, or staff)
//! SIM ABORT               call the match off (any participant, or staff)
//! SIM STATUS              the centre, as it stands
//! ```
//!
//! # Why one verb and not five
//!
//! Doc 16 §3.7 spells the lobby's whole surface as one word with a selector, and
//! `legacy-handlers.toml` counts **primary verbs**: five would be five stanzas
//! and five names taken out of the global namespace for one subsystem that
//! exists in three rooms. So `SIM` is one command, and the five selectors are
//! registered as its switches — which is what makes `SIM/STATUS` work as well as
//! doc 16's `SIM STATUS`, because a switch and a leading word resolve to the
//! **same** [`Selector`] on the way in. There is one code path, not two
//! spellings with two behaviours.
//!
//! # Why a gate and not a per-room verb
//!
//! There is no such thing as a verb that exists in one room: the command table
//! is global. So `SIM` is registered globally and a [`CommandGate`] refuses it
//! anywhere but the lobby, the red room and the blue room with one line —
//! [`crate::match_::OUTSIDE_CENTRE`]. A player who types `SIM` in the bar is
//! told where the simulator is, which is better than "Huh?", and the lobby's
//! verbs are still contextual in the only sense that matters: outside the
//! centre, they do nothing at all.
//!
//! A pilot **inside a pod** counts as being in that pod's team room, because
//! that is where cut K.2's `BOARD` leaves them and `SIM ABORT` is a
//! participant's right from wherever they are (doc 16 §3.8). A world that has no
//! centre at all — an OOC pack older than version 4 — gets no gate: the verb
//! answers "This server has no simulator centre." rather than pointing down a
//! corridor that is not there.
//!
//! The three rooms are resolved at dispatch through the **content latch**, by
//! the pack entry ids C.1 authored (`room.simulator_lobby`, `room.sim_red`,
//! `room.sim_blue`) — never by name, which a staffer may change and two worlds
//! need not share.
//!
//! # The boarding deadline is a world tick
//!
//! `Boarding` has no arena and therefore no clock of its own, so
//! `match.board_timeout_ms` is a **due world tick**: one coarse tick, once a
//! second, that asks the centre whether its deadline has passed and does nothing
//! at all when it has not. A second is coarse next to a two-minute timeout and
//! costs one comparison against one `Option<i64>` on a world that is idle
//! anyway; a `Fast` cadence would buy nothing a boarding pilot could perceive.

use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registry, SceneId, World};
use smol_str::SmolStr;

use crate::arena::state::Team;
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::match_::centre;
use crate::match_::lobby::{self, OUTSIDE_CENTRE, SimError};

/// The tick that closes boarding.
pub const BOARD_TICK: &str = "tacsim.board";

/// How often it fires.
pub const BOARD_TICK_PERIOD: Duration = Duration::from_secs(1);

/// The verb itself.
pub const SIM_VERB: &str = "sim";

/// What a `SIM` line asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    /// `SIM TEAM red|blue`.
    Team,
    /// `SIM LEAVE`.
    Leave,
    /// `SIM START`.
    Start,
    /// `SIM ABORT`.
    Abort,
    /// `SIM STATUS`, and the bare `SIM`.
    Status,
}

impl Selector {
    /// The five words, in help order.
    pub const ALL: [(Selector, &'static str); 5] = [
        (Selector::Team, "team"),
        (Selector::Leave, "leave"),
        (Selector::Start, "start"),
        (Selector::Abort, "abort"),
        (Selector::Status, "status"),
    ];

    /// One word, however it was spelled.
    #[must_use]
    pub fn parse(word: &str) -> Option<Selector> {
        let word = word.trim().to_ascii_lowercase();
        Selector::ALL
            .iter()
            .find(|(_, name)| *name == word)
            .map(|(selector, _)| *selector)
    }
}

/// The switches `SIM` registers — the same five words, so `SIM/STATUS` and
/// `SIM STATUS` are one command and not two.
#[must_use]
pub fn register_switches() -> Vec<SmolStr> {
    Selector::ALL
        .iter()
        .map(|(_, name)| SmolStr::new(*name))
        .collect()
}

/// Read the selector and the rest of the line out of a `SIM` invocation.
///
/// A switch wins when one is given (`SIM/TEAM red`); otherwise the first word of
/// the argument is the selector (`SIM TEAM red`), and an empty line is
/// `SIM STATUS`.
fn selector_of(ctx: &Ctx<'_>) -> Option<(Selector, String)> {
    for (selector, name) in Selector::ALL {
        if ctx.switches.has(name) {
            return Some((selector, ctx.args.raw().trim().to_owned()));
        }
    }
    // `ArgShape::Raw`: the whole argument text, which is where the selector word
    // is when it was not given as a switch.
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Some((Selector::Status, String::new()));
    }
    let (head, rest) = match raw.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (raw, ""),
    };
    Selector::parse(head).map(|selector| (selector, rest.to_owned()))
}

/// `SIM <team|leave|start|abort|status>` — the whole lobby (doc 16 §3.7).
///
/// # Errors
///
/// Whatever the centre refused, as the one line the player sees.
pub fn sim_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "SIM TEAM red|blue | SIM LEAVE | SIM START | SIM ABORT | SIM STATUS";
    let Some((selector, rest)) = selector_of(ctx) else {
        return Err(CmdError::msg(format!("Usage: {USAGE}")));
    };
    match selector {
        Selector::Team => {
            let team = match rest.trim().to_ascii_lowercase().as_str() {
                "red" => Team::Red,
                "blue" => Team::Blue,
                _ => {
                    return Err(CmdError::msg(
                        "Which side? `SIM TEAM red` or `SIM TEAM blue`.",
                    ));
                }
            };
            lobby::claim(ctx, team).map_err(refusal)
        }
        Selector::Leave => lobby::leave(ctx).map_err(refusal),
        Selector::Start => lobby::start(ctx).map_err(refusal),
        Selector::Abort => lobby::abort(ctx).map_err(refusal),
        Selector::Status => {
            // A read, and the only selector that does not need the centre to be
            // in any particular state — but it is still of the centre, so it is
            // still refused outside it (the gate has already done that).
            let actor = ctx.actor;
            for row in lobby::status_lines(ctx.w) {
                ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, row);
            }
            Ok(())
        }
    }
}

/// Render one refusal as the line the player sees.
fn refusal(error: SimError) -> CmdError {
    CmdError::msg(error.to_string())
}

/// **The contextual gate** — `SIM` exists everywhere and means something in
/// three rooms.
pub struct SimGate;

impl mush_core::CommandGate for SimGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        actor: ObjId,
        verb: &str,
        _line: &str,
    ) -> Result<(), String> {
        // The leading verb arrives with its switches stripped, so `SIM/STATUS`
        // reaches this as `sim` like every other spelling.
        if !verb.eq_ignore_ascii_case(SIM_VERB) {
            return Ok(());
        }
        if centre::where_in_centre(w, actor).is_some() {
            return Ok(());
        }
        // A world with no centre at all — an OOC pack older than version 4, or a
        // composition without the content module — has no lobby to point at, so
        // the gate stands aside and the verb answers the true thing ("This
        // server has no simulator centre.") rather than sending a player down a
        // corridor that is not there.
        if centre::centre_rooms(w).is_none() {
            return Ok(());
        }
        Err(OUTSIDE_CENTRE.to_owned())
    }
}

/// The boarding tick's body: one comparison, and nothing at all on a centre that
/// is not boarding — **plus the boot pod sweep, once** (16b cut F.1).
fn board_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    // The boot pass arms this and the first tick performs it: a pod is resolved
    // through the content latch, which is the content module's state and is
    // still empty while this module's `on_world_loaded` runs (see
    // `crate::match_::MatchBook::arm_boot_sweep`). The stock-tank reset needs no
    // latch and is done inside the boot pass itself, where nothing can race it.
    // Once per boot, and on a clean world it finds nothing at all.
    let owed = ctx
        .w
        .module_state_mut::<crate::match_::MatchBook>()
        .is_some_and(crate::match_::MatchBook::take_boot_sweep);
    if owed {
        let ejected = crate::match_::cockpit::eject_all(ctx);
        if !ejected.is_empty() {
            tracing::warn!(
                bodies = ejected.bodies.len(),
                seats = ejected.seats.len(),
                "tacsim: a restart left pilots in the simulator's pods; they are out and the \
                 cockpit billets are given back"
            );
        }
    }
    let now_ms = ctx.w.now().saturating_mul(1_000);
    lobby::board_deadline(ctx, now_ms);
}

/// Register the lobby's surface on the **tacsim** module: the verb, the gate and
/// the boarding tick.
///
/// The `MatchBook` slot the centre lives on is
/// [`crate::arena_bridge::register`]'s, registered beside the arena registry, so
/// nothing here declares state.
///
/// # Errors
///
/// [`ModError`] if the verb name or the tick tag is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // `SIM TEAM` and `SIM START` mint a match id (one document), write the
    // centre's slot and, at the trigger, run cut T.3's whole check-out. That is
    // the same lane `@arena` moved onto in cut T.3, and for the same reason.
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: SIM_VERB.into(),
            shape: ArgShape::Raw,
            switches: register_switches(),
            help: "SIM TEAM red|blue   claim one pilot slot on a side. One slot per account, \
                   four a side.\n\
                   SIM LEAVE           give your slot up; the last one to leave closes the \
                   match.\n\
                   SIM START           lock the pods and open boarding. A side's first pilot, \
                   or staff, with at least one pilot a side.\n\
                   SIM ABORT           call the match off (any pilot in it, or staff). A \
                   running match stops at the arena's next pulse.\n\
                   SIM STATUS          the centre: its state, its sides, its slots and its \
                   pods. Also the bare SIM.\n\
                   The switch spelling (SIM/STATUS) is the same command. All of it works in \
                   the simulator lobby, the two team rooms and the pods, and nowhere else."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(sim_command),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command_gate(Arc::new(SimGate));
    reg.tick(
        BOARD_TICK,
        TickCadence::Coarse(BOARD_TICK_PERIOD),
        Arc::new(board_tick),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_selector_has_a_word_and_a_switch() {
        let switches = register_switches();
        assert_eq!(switches.len(), 5);
        for (selector, name) in Selector::ALL {
            assert!(switches.iter().any(|switch| switch == name), "{name}");
            assert_eq!(Selector::parse(name), Some(selector));
            assert_eq!(
                Selector::parse(&name.to_ascii_uppercase()),
                Some(selector),
                "doc 16 spells them in capitals"
            );
        }
        assert_eq!(Selector::parse("board"), None, "cut K.2's, not this cut's");
        assert_eq!(Selector::parse(""), None);
    }
}
