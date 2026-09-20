//! **`SIM`, the lobby's one verb** (16b cut K.1; doc 16 §3.7 "Verbs", §3.8
//! "Lifecycle").
//!
//! ```text
//! SIM TEAM red | blue     claim one slot on a side
//! SIM LEAVE               give it up
//! SIM START               lock the pods and open boarding (a side's first pilot, or staff)
//! SIM ABORT               call the match off (any participant, or staff)
//! SIM STATUS              the centre, as it stands
//! SIM AI [n]              how many units a side fields when the machines fill in
//! SIM UNIT [<type>]       which unit type your own slot fields
//! ```
//!
//! # Why one verb and not five
//!
//! Doc 16 §3.7 spells the lobby's whole surface as one word with a selector, and
//! `legacy-handlers.toml` counts **primary verbs**: five would be five stanzas
//! and five names taken out of the global namespace for one subsystem that
//! exists in three rooms. So `SIM` is one command, and its selectors are
//! registered as its switches — which is what makes `SIM/STATUS` work as well as
//! doc 16's `SIM STATUS`, because a switch and a leading word resolve to the
//! **same** [`Selector`] on the way in. There is one code path, not two
//! spellings with two behaviours.
//!
//! # Why one global verb and not a per-room one
//!
//! There is no such thing as a verb that exists in one room: the command table
//! is global. So `SIM` is registered globally and **the handler** refuses it
//! anywhere but the lobby, the red room and the blue room with one line —
//! [`crate::match_::OUTSIDE_CENTRE`]. A player who types `SIM` in the bar is
//! told where the simulator is, which is better than "Huh?", and the lobby's
//! verbs are still contextual in the only sense that matters: outside the
//! centre, they do nothing at all. It was a `CommandGate` until **16b cut F.4**;
//! the comment block above [`register`] says why it is not one any more.
//!
//! A pilot **inside a pod** counts as being in that pod's team room, because
//! that is where cut K.2's `BOARD` leaves them and `SIM ABORT` is a
//! participant's right from wherever they are (doc 16 §3.8). A world that has no
//! centre at all — an OOC pack older than version 4 — answers "This server has
//! no simulator centre." rather than pointing down a corridor that is not there.
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
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, SceneId};
use smol_str::SmolStr;

use crate::arena::state::Team;
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::match_::lobby::{self, SimError};

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
    /// **`SIM AI [<n>]`** (16b cut Q.7): how many units a side fields with the
    /// AI fill on.
    Ai,
    /// **`SIM UNIT [<type>]`** (16b cut Q.8): which unit type the caller's own
    /// slot fields.
    Unit,
}

impl Selector {
    /// The seven words, in help order.
    pub const ALL: [(Selector, &'static str); 7] = [
        (Selector::Team, "team"),
        (Selector::Leave, "leave"),
        (Selector::Start, "start"),
        (Selector::Abort, "abort"),
        (Selector::Status, "status"),
        (Selector::Ai, "ai"),
        (Selector::Unit, "unit"),
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

/// The switches `SIM` registers — the same words, so `SIM/STATUS` and
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
    const USAGE: &str = "SIM TEAM red|blue | SIM LEAVE | SIM START | SIM ABORT | SIM STATUS | \
                         SIM AI [n] | SIM UNIT [type]";
    // **Where before what** (16b cut F.4 residual). The deleted gate answered
    // every spelling of the verb outside the centre with the one
    // [`crate::match_::OUTSIDE_CENTRE`] line, and taking it away put the parser
    // in front: `SIM wibble` printed the usage block and a bare `SIM TEAM` asked
    // "Which side?" to somebody standing in the bar — the simulator explaining
    // itself to a player who is nowhere near one, and a second spelling of a
    // refusal that is supposed to have exactly one. The library's own selectors
    // ask again inside their handlers, because `lobby` is a public surface and
    // its guards are its own; this is what makes the ANSWER the same for every
    // line the verb is given.
    lobby::outside_centre(ctx.w, ctx.actor).map_err(refusal)?;
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
        // **16b cut Q.7**: bare reads, with a number sets. A word that is not a
        // number is the usage line and never a silent read — `SIM AI four` must
        // not quietly answer "AI fill: 2 a side."
        Selector::Ai => {
            let rest = rest.trim();
            let size = if rest.is_empty() {
                None
            } else {
                match rest.parse::<u8>() {
                    Ok(size) => Some(size),
                    Err(_) => {
                        return Err(CmdError::msg("How many a side? `SIM AI 2`."));
                    }
                }
            };
            lobby::ai(ctx, size).map_err(refusal)
        }
        // **16b cut Q.8**: bare lists and reads, with a word chooses. Every
        // refusal is the library's, because `lobby` is a public surface and its
        // guards are its own.
        Selector::Unit => {
            let rest = rest.trim();
            let choice = (!rest.is_empty()).then_some(rest);
            lobby::unit_type(ctx, choice).map_err(refusal)
        }
        Selector::Status => {
            // A read, and the only selector that does not need the centre to be
            // in any particular state — but it is still of the centre, and the
            // check at the top of this function is what refuses it outside one.
            // `lobby::status_lines` has no location check of its own.
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

// ---------------------------------------------------------------------------
// Why there is no longer a `SIM` gate (16b cut F.4)
// ---------------------------------------------------------------------------
//
// There was one, and it refused `SIM` anywhere but the centre. A command gate
// runs at step 2 of `mush_core::cmd::dispatch`, **before** the alias stage and
// before home, exits and the command table (step 5 is the exit match) — so the
// gate ate every line whose first word was `sim`, including the `sim` alias the
// OOC pack put on the Welcome Hall's own exit. A player read `Simulator Lobby
// <SIM>` on the exit list, typed `sim`, and was told "The simulator is through
// the lobby." while standing in front of it; the exit never matched.
//
// The word belongs to the verb, so version 5 of the pack takes `sim` off that
// exit and off the two team rooms' return exits (where the opposite happened:
// the gate stood aside inside the centre, the exit matched first, and a bare
// `SIM` walked a pilot out of the room `BOARD` needs them in). But a gate that
// can swallow any line beginning with its verb is a trap for the next piece of
// content too, so the gate goes as well as the alias: [`sim_command`] refuses
// with the same [`OUTSIDE_CENTRE`] sentence from inside the command table, where
// an exit alias and a player's own alias both get their turn first.
//
// `BOARD` and the cockpit selectors keep a gate — theirs is contextual inside
// the centre, which is work a handler cannot do — and cut F.4 gives it the same
// stand-aside rule outside it.

/// The boarding tick's body: one comparison, and nothing at all on a centre that
/// is not boarding — **plus the boot pod sweep, once** (16b cut F.1).
///
/// The tick is registered whether or not the layer is on, and the two halves
/// want opposite things of that (16b cut F.4). The **sweep** must still run with
/// the layer off: a server restarted with `[tacsim].enabled` turned off is
/// exactly where bodies are left shut in pods by the run before it, and nothing
/// else lets them out. The **deadline** must not: it is the simulator driving
/// itself on a server whose simulator is off, so `lobby::board_deadline` asks
/// the layer first.
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

/// Register the lobby's surface on the **tacsim** module: the verb and the
/// boarding tick. There is no command gate; 16b cut F.4 deleted it, and
/// [`sim_command`] does the location check itself (see the block comment above).
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
                   SIM AI [n]          how many units a side fields when the server fills the \
                   unclaimed pods with machines. A side's first pilot or staff, before the \
                   match starts; bare, it reads the number.\n\
                   SIM UNIT [type]     which unit type your own slot fields. Bare, it lists \
                   what this server can field and what you are flying; with a type, your pod's \
                   tank is refitted to it when the match starts. Your own slot only, before the \
                   match starts, and the pod goes back to its own tank afterwards.\n\
                   The switch spelling (SIM/STATUS) is the same command. All of it works in \
                   the simulator lobby, the two team rooms and the pods; anywhere else every \
                   spelling of SIM answers only where the simulator is."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(sim_command),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
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
        // 16b cut Q.7 made it six: `ai` joined the five. 16b cut Q.8 made it
        // seven with `unit`.
        assert_eq!(switches.len(), 7);
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
