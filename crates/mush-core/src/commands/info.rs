//! Info commands: `WHO` (connected players with real idle/connect times
//! from the world's session stats) and `help`/`+help` (rendered from every
//! registered [`CommandSpec`]'s real help text). The same WHO view also
//! backs the `"who"` query handler (web WHO-rail hydration, M4).

use std::sync::Arc;

use mush_proto::{OutputEvent, PanelFrameKind, RichText, SmolStr, WhoEntry};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::{Flags, Powers, Role};
use crate::module::{HookReadContext, QueryHandler, Registrar};
use crate::obj::ObjId;
use crate::perm;
use crate::world::World;

use super::attr_text;

/// The WHO listing as seen by `viewer`: connected actors with real
/// idle/connect times from the engine-maintained session stats. Staff
/// (Wizard+ or `SEE_HIDDEN`) see every location and DARK players; a player
/// sees the location of whoever shares their room and nothing else
/// (interface plan I3, D-I-12), and every entry says whether it is `here`.
/// Shared by the `WHO` command and the `"who"` query handler — one view,
/// two transports.
#[must_use]
pub fn who_entries(w: &World, viewer: ObjId) -> Vec<WhoEntry> {
    let staff =
        perm::has_role(w, viewer, Role::Wizard) || perm::has_power(w, viewer, Powers::SEE_HIDDEN);
    let viewer_room = w.get(viewer).and_then(|o| o.location);
    let mut entries = Vec::new();
    for id in w.connected() {
        let Some(o) = w.get(id) else { continue };
        if o.flags.contains(Flags::DARK) && !staff && id != viewer {
            continue;
        }
        let here = viewer_room.is_some() && o.location == viewer_room;
        let location = if staff || here {
            o.location.and_then(|l| w.get(l)).map(|r| r.name.clone())
        } else {
            None
        };
        // Idle/connect times come from the engine-maintained session stats
        // (clamped to 0 against clock skew); actors marked connected by
        // other means (tests, NPCs) report zeros.
        let now = w.now();
        let (idle_s, conn_s) = w.session_stat(id).map_or((0, 0), |s| {
            (
                u64::try_from(now - s.last_input).unwrap_or(0),
                u64::try_from(now - s.connect_time).unwrap_or(0),
            )
        });
        entries.push(WhoEntry {
            name: o.name.clone(),
            alias: o.aliases.first().map(ToString::to_string),
            idle_s,
            conn_s,
            location,
            here,
            doing: attr_text(w, id, "doing"),
        });
    }
    entries
}

fn who_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let entries = who_entries(ctx.w, ctx.actor);
    ctx.out.to_actor(ctx.actor, OutputEvent::Who(entries));
    Ok(())
}

/// The longest `OUTPUTPREFIX`/`OUTPUTSUFFIX` text, in characters
/// (16b cut AP.4).
pub const MAX_OUTPUT_BRACKET_CHARS: usize = 256;

/// Which of a connection's two reply brackets a command sets.
#[derive(Clone, Copy)]
enum Bracket {
    Prefix,
    Suffix,
}

impl Bracket {
    const fn verb(self) -> &'static str {
        match self {
            Bracket::Prefix => "OUTPUTPREFIX",
            Bracket::Suffix => "OUTPUTSUFFIX",
        }
    }
}

/// `OUTPUTPREFIX [<text>]` / `OUTPUTSUFFIX [<text>]` (16b cut AP.4, 16c
/// `N20`): PennMUSH's per-connection reply framing. Sets the dispatching
/// connection's bracket to the trimmed text, or clears it when bare; SILENT
/// on success, so a script that sets a suffix reads the suffix as its first
/// answer (`Engine::step` reads the suffix after the body). Refuses, in this
/// order: a line from no connection (a script, an NPC, a queued action —
/// `World::dispatching_conn` is `None`), then a text past
/// [`MAX_OUTPUT_BRACKET_CHARS`] characters or carrying a control character.
/// It touches one transient RAM row on the World and nothing else: no
/// document, no object, no attribute.
fn output_bracket(ctx: &mut Ctx<'_>, which: Bracket) -> Result<(), CmdError> {
    let verb = which.verb();
    let Some(conn) = ctx.w.dispatching_conn() else {
        return Err(CmdError::msg(format!(
            "{verb} refused. That is a setting of a connection, and this line came from none."
        )));
    };
    let text = ctx.args.raw();
    if text.chars().count() > MAX_OUTPUT_BRACKET_CHARS || text.chars().any(char::is_control) {
        return Err(CmdError::msg(format!(
            "{verb} refused. The text is at most {MAX_OUTPUT_BRACKET_CHARS} printable characters."
        )));
    }
    let text = (!text.is_empty()).then(|| text.to_string());
    match which {
        Bracket::Prefix => ctx.w.set_output_prefix(conn, text),
        Bracket::Suffix => ctx.w.set_output_suffix(conn, text),
    }
    Ok(())
}

fn output_prefix_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    output_bracket(ctx, Bracket::Prefix)
}

fn output_suffix_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    output_bracket(ctx, Bracket::Suffix)
}

/// The `"who"` query handler (web `Req { q: Who }` hydration): the same
/// listing the `WHO` command produces, wrapped by the engine in a `Reply`.
struct WhoQuery;

impl QueryHandler for WhoQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Who(who_entries(w, actor)))
    }
}

/// The query name and panel feed the command table answers on.
pub const COMMANDS_FEED: &str = "commands.list";
/// Maximum serialized bytes of one `commands.list` keyframe; the listing
/// stops (and says so) rather than splitting (interface plan I1, D-I-5).
pub const MAX_COMMANDS_LIST_BYTES: usize = 64 * 1024;
/// Maximum bytes of one command's `summary` in the listing.
pub const MAX_COMMAND_SUMMARY_BYTES: usize = 160;

/// One command as the palette sees it (interface plan I1, D-I-5).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CommandEntry {
    /// Primary verb.
    pub name: String,
    /// Alternate verbs.
    pub aliases: Vec<String>,
    /// Valid switches, without the slash.
    pub switches: Vec<String>,
    /// The usage line: the part of the help before the `  --  ` separator,
    /// or its first line.
    pub usage: String,
    /// The rest of the help, whitespace-collapsed and capped at
    /// [`MAX_COMMAND_SUMMARY_BYTES`] (an ellipsis marks a cut).
    pub summary: String,
    /// The registering module, when the registry records one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Whether `help <name>` also serves a prose topic of that name.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub topic: bool,
}

/// Split a command's help into `(usage, summary)`: at the house-style
/// `  --  ` separator when present, else at the first line break.
#[must_use]
pub fn split_help(help: &str) -> (String, String) {
    let (usage, rest) = match help.split_once("  --  ") {
        Some((u, r)) => (u, r),
        None => match help.split_once('\n') {
            Some((u, r)) => (u, r),
            None => (help, ""),
        },
    };
    let usage = usage.split_whitespace().collect::<Vec<_>>().join(" ");
    let summary = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    (usage, truncate_summary(&summary))
}

/// Cap a summary at [`MAX_COMMAND_SUMMARY_BYTES`] on a char boundary, the
/// ellipsis (three UTF-8 bytes) included in the cap.
fn truncate_summary(s: &str) -> String {
    if s.len() <= MAX_COMMAND_SUMMARY_BYTES {
        return s.to_string();
    }
    let mut end = MAX_COMMAND_SUMMARY_BYTES.saturating_sub('…'.len_utf8());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", s[..end].trim_end())
}

/// Whether `viewer` may see `spec` in a listing: the role, power and
/// privilege gates dispatch itself applies, and nothing else.
///
/// Locks are per-target and are not evaluated. Neither are
/// [`CommandGate`](crate::CommandGate)s: a gate takes the world and refuses
/// with a reason at dispatch time, so it cannot be run speculatively for a
/// listing — a gated-but-in-role verb still appears, in the palette and in
/// `help` alike. This is the one predicate both listings share, so the
/// line-mode `help` table and the web/TUI palette can never drift into
/// naming different surfaces.
fn spec_visible_to(w: &World, viewer: ObjId, role: Role, spec: &CommandSpec) -> bool {
    role >= spec.min_role
        && spec.power.is_none_or(|p| perm::has_power(w, viewer, p))
        && spec
            .privilege
            .is_none_or(|p| perm::has_privilege(w, viewer, p))
}

/// One help entry as the viewer's own `help <name>` command reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpEntry {
    /// The topic title or command name, aliases and switches.
    pub header: String,
    /// The registered help text, without paraphrasing.
    pub body: String,
    /// The canonical command name; prose topics have no command.
    pub command: Option<SmolStr>,
    /// Public prose, or a command without role, power, privilege or lock gates.
    /// This describes its help, not permission to execute a particular action.
    pub publicly_visible: bool,
}

/// Resolve an exact command name or alias before a public prose topic. Commands
/// hidden from the viewer behave exactly like unknown commands; a public topic
/// of the same name remains readable. Target locks and runtime command gates are
/// not evaluated, matching the command listing's visibility rules.
#[must_use]
pub fn help_entry(reg: &Registry, w: &World, viewer: ObjId, name: &str) -> Option<HelpEntry> {
    let name = name.trim();
    let role = perm::role_of(w, viewer);
    let Some(spec) = reg
        .spec_of(name)
        .filter(|spec| spec_visible_to(w, viewer, role, spec))
    else {
        return reg.help_topic(name).map(|topic| HelpEntry {
            header: topic.title.clone(),
            body: topic.body.clone(),
            command: None,
            publicly_visible: true,
        });
    };
    let mut header = spec.name.to_string();
    if !spec.aliases.is_empty() {
        let aliases: Vec<&str> = spec.aliases.iter().map(SmolStr::as_str).collect();
        header.push_str(&format!("  (aliases: {})", aliases.join(", ")));
    }
    if !spec.switches.is_empty() {
        let switches: Vec<&str> = spec.switches.iter().map(SmolStr::as_str).collect();
        header.push_str(&format!("  [switches: /{}]", switches.join(" /")));
    }
    Some(HelpEntry {
        header,
        body: spec.help.clone(),
        command: Some(spec.name.clone()),
        publicly_visible: spec.min_role == Role::Player
            && spec.power.is_none()
            && spec.privilege.is_none()
            && spec.lock.is_none(),
    })
}

/// The command table as `viewer` may use it: every registered command whose
/// role and power gates the viewer passes (locks are per-target and are not
/// evaluated), sorted by name, each with its usage and summary. Staff-gated
/// verbs are absent for non-staff, not greyed. Shared by the
/// `commands.list` query and any future terminal listing — one view, every
/// transport.
#[must_use]
pub fn command_entries(reg: &Registry, w: &World, viewer: ObjId) -> Vec<CommandEntry> {
    let role = perm::role_of(w, viewer);
    let mut entries: Vec<CommandEntry> = reg
        .command_registrations()
        .filter(|(spec, _, _)| spec_visible_to(w, viewer, role, spec))
        .map(|(spec, module, _)| {
            let (usage, summary) = split_help(&spec.help);
            CommandEntry {
                name: spec.name.to_string(),
                aliases: spec.aliases.iter().map(ToString::to_string).collect(),
                switches: spec.switches.iter().map(ToString::to_string).collect(),
                usage,
                summary,
                module: module.map(str::to_string),
                topic: reg.help_topic(&spec.name).is_some(),
            }
        })
        .collect();
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    entries
}

/// The `commands.list` keyframe body: `{ commands: [...], truncated: bool }`,
/// stopped at [`MAX_COMMANDS_LIST_BYTES`] with `truncated: true` rather than
/// split, so the DTO is bounded whatever the composition registers.
#[must_use]
pub fn commands_keyframe(reg: &Registry, w: &World, viewer: ObjId) -> serde_json::Value {
    let mut listed = Vec::new();
    let mut bytes = 32usize; // the envelope around the array
    let mut truncated = false;
    for entry in command_entries(reg, w, viewer) {
        let value = serde_json::to_value(&entry).expect("a command entry serializes");
        let cost = serde_json::to_vec(&value).map_or(0, |v| v.len()) + 1;
        if bytes + cost > MAX_COMMANDS_LIST_BYTES {
            truncated = true;
            break;
        }
        bytes += cost;
        listed.push(value);
    }
    serde_json::json!({ "commands": listed, "truncated": truncated })
}

/// The `commands.list` query handler: the viewer-filtered command table as a
/// `Panel` keyframe on the [`COMMANDS_FEED`] feed (web `Req { q: Commands }`
/// and the TUI's palette both hydrate from it). The table lives in the
/// registry, which only the engine's query dispatch attaches to the read
/// context; the plain `query` path (no registry) refuses politely.
struct CommandsQuery;

impl QueryHandler for CommandsQuery {
    fn query(
        &self,
        _w: &World,
        _actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        Err(RichText::tag(
            "error",
            "The command table is only available through the live engine.",
        ))
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let Some(reg) = context.registry() else {
            return self.query(context.world(), actor, params);
        };
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(COMMANDS_FEED),
            kind: PanelFrameKind::Keyframe,
            data: commands_keyframe(reg, context.world(), actor),
        })
    }
}

/// The `"room"` query handler (16b cut B.4, `B-14`): **the attach look**.
///
/// A connection that attaches to a character already in the world used to be
/// shown the room by a real `look` dispatched AS the actor — and a command's
/// output goes to the actor, so every other connection attached to that
/// character printed the whole room again. A query's answer is a `Reply` the
/// engine sends to the ONE connection that asked, so the gateway's attach now
/// asks this instead and only the new connection sees the room.
///
/// It answers the actor's OWN location and no other room (it takes no
/// argument), through [`room_view`](crate::output::room_view) with the live
/// typed-state lens — the same perception hooks a typed `look` renders
/// through, so an attach names nothing a `look` would not. The `LookRendered`
/// hook lines a typed `look` appends are deliberately NOT on this view (a
/// recorded decision of cut B.4: their one implementor is a travel journal
/// line, and a typed `look` still carries them).
///
/// The room view lives behind the registry's perception hooks, which only the
/// engine's query dispatch attaches to the read context; the plain `query`
/// path (no registry) refuses politely, as [`CommandsQuery`] does.
struct RoomQuery;

impl QueryHandler for RoomQuery {
    fn query(
        &self,
        _w: &World,
        _actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        Err(RichText::tag(
            "error",
            "The room view is only available through the live engine.",
        ))
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let Some(reg) = context.registry() else {
            return self.query(context.world(), actor, params);
        };
        let w = context.world();
        w.get(actor)
            .and_then(|o| o.location)
            .and_then(|location| crate::output::room_view(reg, w, actor, location, context.lens()))
            .map(OutputEvent::Room)
            .ok_or_else(|| RichText::tag("error", "You are nowhere."))
    }
}

/// `time` / `@time`: the IC ("in-character") date and time, the season, and
/// the time-of-day — all derived from the core game clock (its IC:OOC ratio
/// and calendar phase come from `mushd.toml [game]`). Public info; the OOC
/// wall clock is deliberately not shown.
fn time_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let datetime = ctx.w.ic_datetime();
    let season = ctx.w.season();
    let tod = ctx.w.time_of_day();
    // The article agrees with the word it precedes (the season): "an autumn
    // evening", "a summer night".
    let season = season.to_string();
    ctx.out.info(
        ctx.actor,
        format!(
            "Game time: {datetime} -- {} {season} {tod}.",
            article(&season)
        ),
    );
    Ok(())
}

/// "a" or "an" for the following word, by its first letter (vowel → "an").
fn article(word: &str) -> &'static str {
    match word.chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

/// `help` — the line-mode command table and the prose topics.
///
/// The listing is filtered by [`spec_visible_to`], the same predicate the
/// web/TUI palette uses: a listing never names a verb the actor cannot
/// dispatch, so `help` is not an enumeration of the staff surface. `help
/// <name>` for an out-of-role verb answers the same "No help available" an
/// unknown name gets — the refusal must not confirm the verb exists — after
/// falling through to the prose topics, which are public.
fn help_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let topic = ctx.args.raw().trim().to_string();
    let role = perm::role_of(ctx.w, ctx.actor);
    if topic.is_empty() {
        ctx.out
            .info(ctx.actor, "Commands (type \"help <command>\" for details):");
        let mut names: Vec<&str> = ctx
            .reg
            .specs()
            .filter(|s| spec_visible_to(ctx.w, ctx.actor, role, s))
            .map(|s| s.name.as_str())
            .collect();
        names.sort_unstable();
        ctx.out.info(ctx.actor, names.join("  "));
        let topics: Vec<&str> = ctx.reg.help_topic_names().collect();
        if !topics.is_empty() {
            ctx.out
                .info(ctx.actor, "Topics (type \"help <topic>\" to read):");
            ctx.out.info(ctx.actor, topics.join("  "));
        }
        return Ok(());
    }
    let Some(entry) = help_entry(ctx.reg, ctx.w, ctx.actor, &topic) else {
        return Err(CmdError::msg(format!("No help available for \"{topic}\".")));
    };
    ctx.out.info(ctx.actor, entry.header);
    ctx.out.info(ctx.actor, entry.body);
    Ok(())
}

/// Register `WHO`, `OUTPUTPREFIX`, `OUTPUTSUFFIX`, `time` and `help` (plus
/// the core `"who"` query handler).
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    // Core query handlers register under the reserved module identity
    // "core". This runs before any game module registers, so a collision is
    // a programming error, not a runtime condition.
    //
    // 16b cut B.4 (`B-14`): `room`, the attach look answered to one
    // connection, registers beside `who` through the same one call.
    let core_queries: [(&str, Arc<dyn QueryHandler>); 2] =
        [("who", Arc::new(WhoQuery)), ("room", Arc::new(RoomQuery))];
    let mut queries = Registrar::new(reg, "core");
    for (name, handler) in core_queries {
        queries
            .query_handler(name, handler)
            .expect("core query handler registration must not collide");
    }
    // The command table: a core panel feed (so it is in the `hello` manifest
    // and subscribable by name) answered by the same-named query handler.
    let mut core = Registrar::new(reg, "core");
    core.panel_feed(COMMANDS_FEED)
        .expect("core panel feed registration must not collide");
    core.query_handler(COMMANDS_FEED, Arc::new(CommandsQuery))
        .expect("core query handler registration must not collide");
    reg.register(
        CommandSpec {
            name: "WHO".into(),
            shape: ArgShape::None,
            help: "WHO  --  List connected players with their idle and connected \
                   times. Staff also see locations; DARK players are hidden from \
                   non-staff."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(who_cmd),
    )?;
    // 16b cut AP.4 (16c `N20`): PennMUSH's per-connection reply brackets,
    // registered exact-only — a socket-level setting is never abbreviated,
    // and a prefix-matched `OUTPUTPREFIX` would take `o` from `ORDER` and
    // `out` from an exit (review `AP4-5`).
    reg.register_exact(
        CommandSpec {
            name: "OUTPUTPREFIX".into(),
            shape: ArgShape::Raw,
            help: "OUTPUTPREFIX [<text>]  --  Print <text> on a line of its own \
                   before the answer to every line you type on this connection; \
                   bare, clear it. Silent; typed in full. See \"help macros\"."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(output_prefix_cmd),
    )?;
    reg.register_exact(
        CommandSpec {
            name: "OUTPUTSUFFIX".into(),
            shape: ArgShape::Raw,
            help: "OUTPUTSUFFIX [<text>]  --  Print <text> on a line of its own \
                   after the answer to every line you type on this connection; \
                   bare, clear it. Silent; typed in full. See \"help macros\"."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(output_suffix_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "time".into(),
            aliases: vec!["@time".into()],
            help: "time  --  Show the in-character date and time: the game's \
                   calendar date, the clock, the current season, and the time \
                   of day. IC time runs at the configured IC:OOC ratio (a ratio \
                   of 4 means four IC hours pass each real hour). @time is the \
                   same command."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(time_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "help".into(),
            aliases: vec!["+help".into()],
            help: "help [<command>|<topic>]  --  With no argument, list every \
                   command and every prose topic. With a command name (exact, \
                   including its @ or + prefix), show its usage, aliases, and \
                   switches; otherwise serve the matching prose topic (e.g. \
                   \"help getting-started\"). An exact command name wins over a \
                   topic of the same name. +help is the same command."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(help_cmd),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// A store-less lens for the dispatch calls below: these tests pin the
    /// answer a caller gets with no typed state at all, which is exactly what
    /// the old store-less twin handed out by default.
    const DETACHED: crate::action_state::TypedStateLens<'static> =
        crate::action_state::TypedStateLens::Detached(
            crate::action_state::DetachedTypedState::Test,
        );

    use crate::clock::{CivilDate, GameClock};
    use crate::cmd::{Registry, dispatch};
    use crate::obj::{GOD, LIMBO, ObjId, ObjKind};
    use crate::output::{Audience, Outbox};
    use crate::world::World;

    use mush_proto::OutputEvent;

    fn setup() -> (World, Registry, ObjId) {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        // Anchor a fixed IC datetime at OOC time 0, then read it at OOC 0.
        let epoch = CivilDate::parse("2075-08-28 02:57").unwrap();
        w.set_game_clock(GameClock {
            ic_ratio: 1,
            epoch_offset: epoch.to_ic_seconds(),
        });
        w.set_now(0);
        let mut reg = Registry::new();
        crate::commands::register_core_commands(&mut reg).unwrap();
        (w, reg, nick)
    }

    fn lines(out: &Outbox, actor: ObjId) -> Vec<String> {
        out.events()
            .iter()
            .filter(|(aud, _)| matches!(aud, Audience::Actor(a) if *a == actor))
            .filter_map(|(_, e)| match e {
                OutputEvent::Sys(t) => Some(t.to_plain_text()),
                _ => None,
            })
            .collect()
    }

    /// 16b cut AP.4: a line from no connection (no dispatching connection
    /// set — a script, an NPC) hears the one refusal and sets nothing.
    #[test]
    fn ap4_outputprefix_from_no_connection_is_refused_and_sets_nothing() {
        let (mut w, reg, nick) = setup();
        assert_eq!(w.dispatching_conn(), None);
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, nick, "OUTPUTPREFIX x", &mut out, DETACHED);
        assert_eq!(
            lines(&out, nick),
            vec![
                "OUTPUTPREFIX refused. That is a setting of a connection, and this line came from none."
                    .to_string()
            ]
        );
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, nick, "OUTPUTSUFFIX y", &mut out, DETACHED);
        assert_eq!(
            lines(&out, nick),
            vec![
                "OUTPUTSUFFIX refused. That is a setting of a connection, and this line came from none."
                    .to_string()
            ]
        );
        for conn in 0..4 {
            assert_eq!(w.output_brackets(conn), None);
        }
    }

    #[test]
    fn time_shows_ic_date_season_and_tod() {
        let (mut w, reg, nick) = setup();
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, nick, "time", &mut out, DETACHED);
        let got = lines(&out, nick);
        assert_eq!(
            got,
            vec!["Game time: August 28, 2075, 02:57 -- a summer night.".to_string()],
        );
    }

    #[test]
    fn time_uses_an_before_a_vowel_initial_season() {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        // Day-of-year 250 at OOC time 0 → autumn (a vowel-initial season),
        // hour 0 → night: the article agrees with the season → "an autumn".
        w.set_game_clock(GameClock {
            ic_ratio: 1,
            epoch_offset: 250 * 86_400,
        });
        w.set_now(0);
        let mut reg = Registry::new();
        crate::commands::register_core_commands(&mut reg).unwrap();
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, nick, "time", &mut out, DETACHED);
        let got = lines(&out, nick);
        assert!(
            got[0].contains("-- an autumn night."),
            "vowel-initial season takes 'an': {got:?}"
        );
    }

    #[test]
    fn at_time_alias_is_the_same_command() {
        let (mut w, reg, nick) = setup();
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, nick, "@time", &mut out, DETACHED);
        assert_eq!(
            lines(&out, nick),
            vec!["Game time: August 28, 2075, 02:57 -- a summer night.".to_string()],
        );
    }

    #[test]
    fn time_tracks_the_ratio_as_ooc_time_advances() {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        // Ratio 4, epoch at midnight: six OOC hours -> a full IC day.
        w.set_game_clock(GameClock {
            ic_ratio: 4,
            epoch_offset: CivilDate::parse("2100-01-01 00:00")
                .unwrap()
                .to_ic_seconds(),
        });
        let mut reg = Registry::new();
        crate::commands::register_core_commands(&mut reg).unwrap();

        w.set_now(6 * 3_600);
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, nick, "time", &mut out, DETACHED);
        assert!(
            lines(&out, nick)[0].contains("January 2, 2100"),
            "{:?}",
            lines(&out, nick)
        );
    }

    #[test]
    fn shared_help_resolves_aliases_before_topics_and_matches_the_command() {
        let (mut w, mut reg, nick) = setup();
        reg.register(
            crate::CommandSpec {
                name: "+guide".into(),
                aliases: vec!["+g".into()],
                switches: vec!["show".into(), "preview".into()],
                help: "+guide/show reads; +guide/preview commits nothing.".into(),
                ..crate::CommandSpec::default()
            },
            std::sync::Arc::new(|_ctx: &mut crate::Ctx<'_>| Ok(())),
        )
        .unwrap();
        reg.add_help_topic("+g", "Public topic", "The command wins.");
        let entry = super::help_entry(&reg, &w, nick, " +G ").unwrap();
        assert_eq!(entry.command.as_deref(), Some("+guide"));
        assert_eq!(
            entry.header,
            "+guide  (aliases: +g)  [switches: /show /preview]"
        );
        assert!(entry.publicly_visible);
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, nick, "help +G", &mut out, DETACHED);
        assert_eq!(lines(&out, nick), vec![entry.header, entry.body]);
    }

    #[test]
    fn shared_help_hides_role_power_and_privilege_commands_without_an_oracle() {
        let (mut w, mut reg, nick) = setup();
        let specs = [
            crate::CommandSpec {
                name: "+staff-guide".into(),
                min_role: crate::Role::Wizard,
                ..crate::CommandSpec::default()
            },
            crate::CommandSpec {
                name: "+power-guide".into(),
                power: Some(crate::Powers::SIM_ADMIN),
                ..crate::CommandSpec::default()
            },
            crate::CommandSpec {
                name: "+privilege-guide".into(),
                privilege: Some(crate::Privileges::DISCORD_IDENTITY),
                ..crate::CommandSpec::default()
            },
        ];
        for spec in specs {
            reg.register(
                spec,
                std::sync::Arc::new(|_ctx: &mut crate::Ctx<'_>| Ok(())),
            )
            .unwrap();
        }
        for name in [
            "+staff-guide",
            "+power-guide",
            "+privilege-guide",
            "+unknown",
        ] {
            assert_eq!(super::help_entry(&reg, &w, nick, name), None);
        }
        assert!(
            !super::help_entry(&reg, &w, GOD, "+staff-guide")
                .unwrap()
                .publicly_visible
        );
        w.get_mut(nick).unwrap().powers = crate::Powers::SIM_ADMIN;
        w.get_mut(nick).unwrap().privileges = crate::Privileges::DISCORD_IDENTITY;
        for name in ["+power-guide", "+privilege-guide"] {
            assert!(
                !super::help_entry(&reg, &w, nick, name)
                    .unwrap()
                    .publicly_visible
            );
        }
        reg.add_help_topic(
            "+staff-guide",
            "Public explanation",
            "Anyone can read this topic.",
        );
        let public = super::help_entry(&reg, &w, nick, "+staff-guide").unwrap();
        assert_eq!(public.command, None);
        assert_eq!(public.body, "Anyone can read this topic.");
        assert!(public.publicly_visible);
        assert!(
            super::help_entry(&reg, &w, GOD, "+staff-guide")
                .unwrap()
                .command
                .is_some()
        );
    }

    #[test]
    fn locked_command_help_remains_readable_but_is_not_public_guidance() {
        let (w, mut reg, nick) = setup();
        reg.register(
            crate::CommandSpec {
                name: "+locked-guide".into(),
                lock: Some(crate::lock::LockExpr::Is(GOD)),
                ..crate::CommandSpec::default()
            },
            std::sync::Arc::new(|_ctx: &mut crate::Ctx<'_>| Ok(())),
        )
        .unwrap();
        let entry = super::help_entry(&reg, &w, nick, "+locked-guide").unwrap();
        assert_eq!(entry.command.as_deref(), Some("+locked-guide"));
        assert!(!entry.publicly_visible);
    }
}
