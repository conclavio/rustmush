//! **`SIM`, the lobby's one verb** (16b cut K.1; doc 16 §3.7 "Verbs", §3.8
//! "Lifecycle").
//!
//! ```text
//! SIM TEAM red | blue     claim one slot on a side
//! SIM LEAVE               give it up
//! SIM START               lock the pods and open boarding (a side's lead, or staff)
//! SIM ABORT               call the match off (any participant, or staff)
//! SIM STATUS              the centre, as it stands
//! SIM AI [n]              how many units a side fields when the machines fill in
//! SIM UNIT [<type>]       which unit type your own slot fields
//! SIM CONCEDE             give the match up for your side (its lead, while it runs)
//! SIM HISTORY             the centre's last twenty matches, newest first
//! SIM REPORT [<match>]    your own debrief of a match you flew
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
    /// **`SIM CONCEDE`** (16b cut D.3, `B-33`): a side's lead gives the
    /// running match up, and the other side wins it.
    Concede,
    /// **`SIM HISTORY`** (16b cut V.2; 16c N2): the centre's last
    /// [`crate::match_::HISTORY_DEPTH`] finished matches, newest first, in
    /// public words.
    History,
    /// **`SIM REPORT [<match>]`** (16b cut V.3; 16c N2, Q37): the actor's own
    /// debrief of one match they flew — [`crate::match_::report::request`].
    Report,
}

impl Selector {
    /// The ten words, in help order (16b cut V.2 added `history`, ninth; 16b
    /// cut V.3 `report`, tenth).
    pub const ALL: [(Selector, &'static str); 10] = [
        (Selector::Team, "team"),
        (Selector::Leave, "leave"),
        (Selector::Start, "start"),
        (Selector::Abort, "abort"),
        (Selector::Concede, "concede"),
        (Selector::Status, "status"),
        (Selector::Ai, "ai"),
        (Selector::Unit, "unit"),
        (Selector::History, "history"),
        (Selector::Report, "report"),
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

/// `SIM <team|leave|start|abort|concede|status|ai|unit|history|report>` — the
/// whole lobby (doc 16 §3.7).
///
/// # Errors
///
/// Whatever the centre refused, as the one line the player sees.
pub fn sim_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "SIM TEAM red|blue | SIM LEAVE | SIM START | SIM ABORT | SIM CONCEDE | \
                         SIM STATUS | SIM AI [n] | SIM UNIT [type] | SIM HISTORY | \
                         SIM REPORT [match]";
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
        // **16b cut D.3 (`B-33`)**: the eighth selector, and no eighth verb.
        Selector::Concede => lobby::concede(ctx).map_err(refusal),
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
            // **16b cut D.1**: the board takes a reader, and this one is a
            // player. While a match runs they read their own side in full and
            // every other side as index, callsign and pilot name alone.
            let actor = ctx.actor;
            for row in lobby::status_lines(ctx.w, lobby::BoardReader::Player(actor)) {
                ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, row);
            }
            Ok(())
        }
        // **16b cut V.2 (16c N2, the result half)**: the ninth selector, and no
        // ninth verb. A pure read of the book's RAM ring — filled from the result
        // documents at boot and at every result write — so it writes nothing and
        // reads no store. Of the centre like every selector, and refused outside
        // it by the check at the top of this function. Its lines are the public
        // outcome: no object number, no pilot's name, no position, no contact and
        // never the seed (`record::history_lines`).
        Selector::History => {
            let actor = ctx.actor;
            let lines = ctx
                .w
                .module_state::<crate::match_::MatchBook>()
                .map(|book| crate::match_::history_lines(book.history(), actor))
                .unwrap_or_else(|| crate::match_::history_lines(std::iter::empty(), actor));
            for line in lines {
                ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
            }
            Ok(())
        }
        // **16b cut V.3 (16c N2, Q37)**: the tenth selector, and no tenth verb. A
        // deferred read of ONE `tacsim:report:<match>:<unit>` document through
        // `Ctx::read_store`, answered by the module's store-read completion
        // (`report::ReportRead`); one in flight per actor. It writes no document
        // and mutates nothing but the book's pending-read row.
        Selector::Report => crate::match_::report::request(ctx, &rest),
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
/// is not boarding — **plus, after a boot, the hatch lines the boot's pod sweep
/// owes** (16b cut U.4, 16c `B-37`).
///
/// The sweep itself is no longer here. Cut F.1 armed it at boot and ran it on
/// this tick's first fire, because the pods resolve through the content latch
/// and nothing ran after every module's load; the kernel now has that hook, so
/// the module's `on_all_loaded` moves the bodies and gives the billets back
/// inside the boot (`crate::match_::cockpit::eject_all_at_boot`). What it cannot
/// do there is speak — boot output reaches no session — so it leaves
/// [`crate::match_::MatchBook::hatch_notices`], and this tick says each one to
/// a body whose session is attached. A body that is not yet attached is held,
/// for at most [`crate::match_::HATCH_HOLD_S`]: on the split server the pilot's
/// surviving SSH session is re-attached by the gateway's re-announce, which
/// reaches the world after the boot and in no order against this tick, and a
/// line said before it would reach nobody. Only the boot adds to the list, so a
/// pilot who boards later in the same boot is never told a hatch opened.
///
/// The tick is registered whether or not the layer is on, and the two halves
/// want opposite things of that (16b cut F.4). The **hatch lines** are said with
/// the layer off too, because the sweep that owes them ran with it off: a server
/// restarted with `[tacsim].enabled` turned off is exactly where bodies were left
/// in pods by the run before it. The **deadline** must not run: it is the
/// simulator driving itself on a server whose simulator is off, so
/// `lobby::board_deadline` asks the layer first.
fn board_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    say_owed_hatch_lines(ctx);
    let now_ms = ctx.w.now().saturating_mul(1_000);
    lobby::board_deadline(ctx, now_ms);
    // **16b cut D.3 (`B-33`)**: the other deadline the lobby owns. `Boarding` has
    // no arena to hold a clock and neither does *a match nobody is in* — the
    // arena is stepping, but it has no idea that every seat it is publishing
    // frames to is empty, and only the centre knows that. One comparison against
    // one `Option<i64>` on a world that is idle anyway, beside the one above it.
    lobby::unattended_deadline(ctx, now_ms);
}

/// **The boot's hatch lines, to the bodies that can read them** (16b cut U.4,
/// 16c `B-37`) — see [`board_tick`].
///
/// A connected body is told and shown the room it stands in now (the one the
/// sweep put it in, unless it has walked since — a room line about a room it
/// has left would be a lie). A body back inside a pod is told nothing: it
/// boarded again, and its hatch is shut. A body with no session attached is
/// held until the hold the first holding tick set, and dropped after it.
fn say_owed_hatch_lines(ctx: &mut Ctx<'_>) {
    let Some((owed, until_s)) = ctx
        .w
        .module_state_mut::<crate::match_::MatchBook>()
        .map(|book| (book.take_hatch_notices(), book.hatch_hold_until_s()))
    else {
        return;
    };
    if owed.is_empty() {
        return;
    }
    let now_s = ctx.w.now();
    let until_s = until_s.unwrap_or_else(|| now_s.saturating_add(crate::match_::HATCH_HOLD_S));
    let mut held = Vec::new();
    for (body, pod) in owed {
        if !ctx.w.is_connected(body) {
            if now_s < until_s {
                held.push((body, pod));
            }
            continue;
        }
        if crate::match_::cockpit::in_a_pod(ctx.w, body) {
            continue;
        }
        let Some(room) = ctx.w.get(body).and_then(|object| object.location) else {
            continue;
        };
        crate::match_::cockpit::tell_hatch_open(ctx, body, pod, room);
    }
    if let Some(book) = ctx.w.module_state_mut::<crate::match_::MatchBook>() {
        book.hold_hatch_notices(held, until_s);
    }
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
                   SIM START           lock the pods and open boarding. A side's lead, or \
                   staff, with at least one pilot a side.\n\
                   SIM ABORT           call the match off (any pilot in it, or staff). A \
                   running match stops at the arena's next pulse; before one is running there \
                   is nothing to abort, and SIM LEAVE is how a claimant gets out.\n\
                   SIM CONCEDE         give the running match up for your side. Its lead \
                   only: the other side wins it, and the result record says so.\n\
                   SIM STATUS          the centre: its state, its sides, its slots and its \
                   pods. Also the bare SIM.\n\
                   SIM AI [n]          how many units a side fields when the server fills the \
                   unclaimed pods with machines. A side's lead or staff, before the \
                   match starts; bare, it reads the number.\n\
                   SIM UNIT [type]     which unit type your own slot fields. Bare, it lists \
                   what this server can field and what you are flying; with a type, your pod's \
                   tank is refitted to it when the match starts. Your own slot only, before the \
                   match starts, and the pod goes back to its own tank afterwards.\n\
                   SIM HISTORY         the centre's last twenty matches, newest first: who \
                   won and why, how long, how many a side, when; your own callsign on the ones \
                   you flew.\n\
                   SIM REPORT [match]  your own debrief of a match you flew (the newest, bare): \
                   your shots and what became of each, the hits on your own tank, what you \
                   destroyed, and the public outcome — told as your own sensors told it, so a \
                   target they had not told apart stays a contact.\n\
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
        // seven with `unit`, 16b cut D.3 eight with `concede`, 16b cut V.2
        // nine with `history`, and 16b cut V.3 ten with `report`.
        assert_eq!(switches.len(), 10);
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
