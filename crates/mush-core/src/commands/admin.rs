//! Admin/utility commands living entirely in core: `@wall` and `@ignore`.
//!
//! ## `@ignore` semantics (documented decision)
//!
//! The mute list is per-player, stored in the core attribute `ignore` as a
//! JSON array of object ids (dbref numbers), written through the normal
//! object persistence path. Enforcement is **silent-drop** ("gag"), the
//! classic MUSH debate resolved in favor of silence:
//!
//! - `page`/`whisper` from an ignored player are dropped at delivery
//!   shaping in the speech handlers — the *sender still sees the normal
//!   "You paged ..." confirmation* and is never told they are ignored.
//! - `say`/`pose`/`semipose`/`@emit` from an ignored player in the same
//!   room are filtered out of that listener's `Audience::Room` resolution
//!   by the world task's fan-out (which consults [`is_ignoring`]).
//! - Staff (Wizard+) can never be ignored: their lines always deliver.
//!
//! The rationale for silence: telling a harasser they are gagged invites
//! evasion; classic `@gag`/`@ignore` implementations (PennMUSH `@ignore`,
//! MUX gag) behave the same way.

use std::sync::Arc;

use mush_proto::{OutputEvent, RichText};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::Role;
use crate::obj::ObjId;
use crate::output::display_name;
use crate::perm;
use crate::world::World;

/// The core attribute holding a player's mute list (JSON array of object
/// id numbers).
pub const IGNORE_ATTR: &str = "ignore";

/// The ids `who` is currently ignoring (empty for a missing/garbled attr —
/// a text overwrite of the attribute simply disables the list).
#[must_use]
pub fn ignore_list(w: &World, who: ObjId) -> Vec<ObjId> {
    w.attr(who, IGNORE_ATTR)
        .and_then(|a| a.as_json())
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(serde_json::Value::as_u64)
                .filter_map(|n| u32::try_from(n).ok())
                .map(ObjId)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `listener` has muted `speaker`, honoring the staff exemption:
/// Wizard+ speakers can never be ignored, and nobody ignores themselves.
/// Delivery shaping (page/whisper drop in the speech handlers, room fan-out
/// filtering in the world task) routes through this one predicate.
#[must_use]
pub fn is_ignoring(w: &World, listener: ObjId, speaker: ObjId) -> bool {
    if listener == speaker || perm::has_role(w, speaker, Role::Wizard) {
        return false;
    }
    ignore_list(w, listener).contains(&speaker)
}

/// The attributed speaker of a [`OutputEvent::Narrative`] line, parsed from
/// its wire `ObjRef` (`"#5"`). `None` for unattributed or non-narrative
/// events. The world task uses this to apply [`is_ignoring`] when it
/// resolves `Audience::Room` fan-out.
#[must_use]
pub fn narrative_actor(ev: &OutputEvent) -> Option<ObjId> {
    match ev {
        OutputEvent::Narrative { actor: Some(r), .. } => r.id.parse::<ObjId>().ok(),
        _ => None,
    }
}

/// The occupant a room-audience event would *reveal* — its perception
/// subject — used by the world task's `Audience::Room` fan-out to apply the
/// M11 per-observer projection gate ([`crate::module::can_perceive`]): a
/// recipient who cannot perceive the subject is skipped for that event only.
///
/// Returns the attributed actor of a [`OutputEvent::Narrative`]
/// (say/pose/semipose/`@emit`) — every room-audience line that reveals a
/// specific character through its own wire `actor`. `None` for events with no
/// attributed subject (weather, panels, unattributed system/ambient emits):
/// those name no one and are delivered to the whole room exactly as before.
///
/// Movement/arrival/departure, take/drop/give, and connect/disconnect echoes
/// are emitted as *unattributed* narratives ([`crate::commands::narrative_emit`],
/// `actor: None`) so their wire form is unchanged, but they still reveal a
/// mover: those carry their subject in the routing layer instead
/// ([`crate::output::Outbox::to_room_from`] → `Audience::Room::subject`),
/// which the fan-out consults *before* falling back to this extractor. Keeping
/// the subject off the wire is what preserves byte-for-byte identical output
/// when no `Perceive` hook is registered.
#[must_use]
pub fn event_subject(ev: &OutputEvent) -> Option<ObjId> {
    match ev {
        OutputEvent::Narrative { actor: Some(r), .. } => r.id.parse::<ObjId>().ok(),
        _ => None,
    }
}

/// Persist `who`'s mute list: a JSON attr for a non-empty list, attribute
/// removal for an empty one.
fn save_ignore_list(ctx: &mut Ctx<'_>, mut ids: Vec<ObjId>) -> Result<(), CmdError> {
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        ctx.w.remove_attr(ctx.actor, IGNORE_ATTR)?;
    } else {
        let arr: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| serde_json::Value::from(u64::from(id.0)))
            .collect();
        ctx.w.set_attr(
            ctx.actor,
            IGNORE_ATTR,
            crate::attr::Attr::json(serde_json::Value::Array(arr), ctx.actor),
        )?;
    }
    Ok(())
}

/// Resolve an `@ignore` target: `*name`/name via the global player index,
/// or a `#dbref` naming an actor.
fn resolve_actor(w: &World, raw: &str) -> Result<ObjId, CmdError> {
    let q = raw.trim();
    if let Ok(id) = q.parse::<ObjId>() {
        return if w.get(id).is_some_and(|o| o.kind.is_actor()) {
            Ok(id)
        } else {
            Err(CmdError::msg("I don't recognize that player."))
        };
    }
    let name = q.strip_prefix('*').unwrap_or(q);
    w.resolve_player(name)
        .ok_or_else(|| CmdError::msg("I don't recognize that player."))
}

/// Render one mute-list entry: the live name, or the bare dbref for a
/// since-destroyed player.
fn entry_name(w: &World, id: ObjId) -> String {
    match w.get(id) {
        Some(o) => o.name.clone(),
        None => id.to_string(),
    }
}

fn list_ignored(ctx: &mut Ctx<'_>) {
    let ids = ignore_list(ctx.w, ctx.actor);
    if ids.is_empty() {
        ctx.out.info(ctx.actor, "You are not ignoring anyone.");
        return;
    }
    let names: Vec<String> = ids.iter().map(|&id| entry_name(ctx.w, id)).collect();
    ctx.out.info(
        ctx.actor,
        format!("You are ignoring: {}.", names.join(", ")),
    );
}

fn ignore_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().to_string();
    if ctx.switches.has("clear") {
        let mut ids = ignore_list(ctx.w, ctx.actor);
        if raw.is_empty() {
            // Bare @ignore/clear empties the whole list.
            let had_any = !ids.is_empty();
            save_ignore_list(ctx, Vec::new())?;
            ctx.out.info(
                ctx.actor,
                if had_any {
                    "You are no longer ignoring anyone."
                } else {
                    "You were not ignoring anyone."
                },
            );
            return Ok(());
        }
        let target = resolve_actor(ctx.w, &raw)?;
        let name = entry_name(ctx.w, target);
        if !ids.contains(&target) {
            return Err(CmdError::msg(format!("You are not ignoring {name}.")));
        }
        ids.retain(|&id| id != target);
        save_ignore_list(ctx, ids)?;
        ctx.out
            .info(ctx.actor, format!("You are no longer ignoring {name}."));
        return Ok(());
    }
    if ctx.switches.has("list") || raw.is_empty() {
        list_ignored(ctx);
        return Ok(());
    }
    let target = resolve_actor(ctx.w, &raw)?;
    if target == ctx.actor {
        return Err(CmdError::msg("You can't ignore yourself."));
    }
    if perm::has_role(ctx.w, target, Role::Wizard) {
        return Err(CmdError::msg("You can't ignore staff."));
    }
    let mut ids = ignore_list(ctx.w, ctx.actor);
    let name = display_name(ctx.w, target);
    if ids.contains(&target) {
        return Err(CmdError::msg(format!("You are already ignoring {name}.")));
    }
    ids.push(target);
    save_ignore_list(ctx, ids)?;
    ctx.out.info(
        ctx.actor,
        format!("You are now ignoring {name}. Their pages, whispers, and room speech will not reach you."),
    );
    Ok(())
}

fn wall_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let msg = ctx.args.raw().to_string();
    if msg.is_empty() {
        return Err(CmdError::msg("Usage: @wall <message>."));
    }
    let name = display_name(ctx.w, ctx.actor);
    ctx.out.to_all(OutputEvent::Sys(RichText::tag(
        "system",
        format!("Announcement from {name}: {msg}"),
    )));
    Ok(())
}

/// Register `@wall` and `@ignore`.
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "@wall".into(),
            shape: ArgShape::Raw,
            min_role: Role::Wizard,
            help: "@wall <message>  --  Broadcast an announcement to every \
                   connected player, shown as \"Announcement from <you>: \
                   <message>\". Wizards only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(wall_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@ignore".into(),
            switches: vec!["clear".into(), "list".into()],
            shape: ArgShape::Raw,
            help: "@ignore <player>  --  Mute a player: their pages and whispers \
                   to you are silently dropped (they still see a normal \
                   delivery confirmation) and their say/pose/emit lines in \
                   your room are filtered from what you hear. Staff cannot be \
                   ignored. @ignore alone (or @ignore/list) lists your mutes; \
                   @ignore/clear <player> lifts one; bare @ignore/clear lifts \
                   them all."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(ignore_cmd),
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

    use super::*;
    use crate::cmd::dispatch;
    use crate::flags::Flags;
    use crate::obj::{GOD, LIMBO, ObjKind};
    use crate::output::{Audience, Outbox};

    fn setup() -> (World, Registry, ObjId, ObjId) {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
        let mut reg = Registry::new();
        crate::commands::register_core_commands(&mut reg).unwrap();
        (w, reg, nick, bob)
    }

    fn run(reg: &Registry, w: &mut World, actor: ObjId, line: &str) -> Outbox {
        let mut out = Outbox::new();
        dispatch(reg, w, actor, line, &mut out, DETACHED);
        out
    }

    fn texts_for(out: &Outbox, actor: ObjId) -> Vec<String> {
        out.events()
            .iter()
            .filter(|(aud, _)| matches!(aud, Audience::Actor(a) if *a == actor))
            .map(|(_, e)| match e {
                OutputEvent::Sys(t) => t.to_plain_text(),
                OutputEvent::Narrative { text, .. } => text.to_plain_text(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn wall_broadcasts_to_all_and_gates_on_wizard() {
        let (mut w, reg, nick, _) = setup();
        // Mortals are refused before the handler runs.
        let out = run(&reg, &mut w, nick, "@wall free candy");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t == "Permission denied.")
        );

        let out = run(&reg, &mut w, GOD, "@wall Reboot in 5 minutes.");
        let hit = out.events().iter().any(|(aud, e)| {
            matches!(aud, Audience::All)
                && matches!(e, OutputEvent::Sys(t)
                    if t.to_plain_text() == "Announcement from God: Reboot in 5 minutes.")
        });
        assert!(hit, "{:?}", out.events());

        // Empty message errors.
        let out = run(&reg, &mut w, GOD, "@wall");
        assert!(
            texts_for(&out, GOD)
                .iter()
                .any(|t| t.contains("Usage: @wall"))
        );
    }

    #[test]
    fn ignore_add_list_clear_round_trip() {
        let (mut w, reg, nick, bob) = setup();
        // Empty list.
        let out = run(&reg, &mut w, nick, "@ignore");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t == "You are not ignoring anyone.")
        );

        // Add by name.
        let out = run(&reg, &mut w, nick, "@ignore Bob");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t.contains("You are now ignoring Bob."))
        );
        assert_eq!(ignore_list(&w, nick), vec![bob]);
        assert!(is_ignoring(&w, nick, bob));
        assert!(!is_ignoring(&w, bob, nick), "mutes are one-directional");

        // Duplicate add errors; list shows the entry.
        let out = run(&reg, &mut w, nick, "@ignore *bob");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t.contains("already ignoring Bob"))
        );
        let out = run(&reg, &mut w, nick, "@ignore/list");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t == "You are ignoring: Bob.")
        );

        // Clear one.
        let out = run(&reg, &mut w, nick, "@ignore/clear Bob");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t == "You are no longer ignoring Bob.")
        );
        assert!(ignore_list(&w, nick).is_empty());
        assert!(
            w.attr(nick, IGNORE_ATTR).is_none(),
            "empty list removes the attr"
        );

        // Clearing a non-entry errors; bare clear empties everything.
        let out = run(&reg, &mut w, nick, "@ignore/clear Bob");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t.contains("not ignoring Bob"))
        );
        run(&reg, &mut w, nick, "@ignore Bob");
        let out = run(&reg, &mut w, nick, "@ignore/clear");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t == "You are no longer ignoring anyone.")
        );
        assert!(ignore_list(&w, nick).is_empty());
    }

    #[test]
    fn ignore_refuses_self_and_staff() {
        let (mut w, reg, nick, bob) = setup();
        let out = run(&reg, &mut w, nick, "@ignore Nick");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t.contains("can't ignore yourself"))
        );
        let out = run(&reg, &mut w, nick, "@ignore God");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t.contains("can't ignore staff"))
        );
        let out = run(&reg, &mut w, nick, "@ignore nobody-here");
        assert!(
            texts_for(&out, nick)
                .iter()
                .any(|t| t.contains("don't recognize"))
        );

        // A speaker promoted to Wizard after being muted bypasses the gag.
        run(&reg, &mut w, nick, "@ignore Bob");
        assert!(is_ignoring(&w, nick, bob));
        w.set_flag(bob, Flags::WIZARD, true).unwrap();
        assert!(!is_ignoring(&w, nick, bob), "staff can never be ignored");
    }

    #[test]
    fn narrative_actor_parses_attributed_lines_only() {
        let (mut w, reg, nick, _) = setup();
        let out = run(&reg, &mut w, nick, "say hello");
        let ev = out
            .events()
            .iter()
            .find_map(|(_, e)| matches!(e, OutputEvent::Narrative { .. }).then_some(e))
            .expect("say queues a narrative");
        assert_eq!(narrative_actor(ev), Some(nick));
        assert_eq!(
            narrative_actor(&OutputEvent::Sys(RichText::plain("x"))),
            None
        );
        assert_eq!(
            narrative_actor(&OutputEvent::Narrative {
                kind: mush_proto::NarrativeKind::Emit,
                actor: None,
                text: RichText::plain("x"),
            }),
            None
        );
    }

    #[test]
    fn garbled_ignore_attr_disables_the_list() {
        let (mut w, _reg, nick, bob) = setup();
        w.set_attr(nick, IGNORE_ATTR, crate::attr::Attr::text("junk", nick))
            .unwrap();
        assert!(ignore_list(&w, nick).is_empty());
        assert!(!is_ignoring(&w, nick, bob));
    }
}
