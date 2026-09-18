//! `+map` and `+move` (plan §3.5): the scene's side of a window — a facilitator
//! opens and closes the map, anyone on it picks a floor to look at and walks.
//! `+map` bare prints the frame (S4c); `+move` is a scene-class `Turn` command,
//! so a turn-based scene gates it like `+roll` and a freeform one does not.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, SceneClass};

use crate::module::{AtlasState, NS, is_staff};
use crate::window::{Compass, StepTarget, close_for_scene, open_for_scene, step};

/// Register `+map` and `+move`.
pub(crate) fn register_map(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+map".into(),
            shape: ArgShape::Raw,
            switches: vec!["open".into(), "close".into(), "floor".into()],
            help: "+map                    the map you stand on: where you are, the floors, who is on it\n\
                   +map/open               open the map over your scene's room (its facilitator, or staff)\n\
                   +map/close              close it, leaving everyone where they stand\n\
                   +map/floor <n>          look at floor n of the map (0 is the ground floor; ground is the\n\
                   \x20                       lot outside)\n\
                   The map is a window over the atlas's tiles: one cell size, one surface at a time. A\n\
                   timestop opens one over its room by itself."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_map),
    )?;
    reg.command(
        CommandSpec {
            name: "+move".into(),
            shape: ArgShape::Raw,
            scene_class: SceneClass::Turn,
            help: "+move <direction> [<n>]   walk n cells (one by default) north, south, east, west or\n\
                   \x20                          a diagonal on the map you stand on\n\
                   +move <x>,<y>             walk toward a cell of the map\n\
                   One cell costs one; a wall, someone standing there, a counter or a room you may not\n\
                   enter stops you. Stepping off the map's edge onto a room's ground grows the map. In a\n\
                   turn-based scene you walk on your turn."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_move),
    )
}

/// **The one sentence a map that is not there answers with** — a domain with no
/// open window, and (16b cut S.2, doc 16 §3.6 "World-side exclusion") a domain a
/// simulation authority holds. One line for both, so `+map` on an arena's ground
/// says nothing a closed window would not.
const MAP_CLOSED: &str = "Your map is closed.";

fn say(ctx: &mut Ctx<'_>, line: String) {
    let actor = ctx.actor;
    ctx.out.to_actor(
        actor,
        mush_proto::OutputEvent::Sys(mush_proto::RichText::tag(NS, line)),
    );
}

fn scene_of(ctx: &Ctx<'_>, actor: ObjId) -> Result<mush_scenes::SceneRecord, CmdError> {
    ctx.w
        .module_state::<mush_scenes::ScenesState>()
        .and_then(|st| st.active_scene_of(actor).cloned())
        .ok_or_else(|| CmdError::msg("You are not in a scene; a map opens over a scene's room."))
}

fn cmd_map(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if ctx.switches.has("open") {
        let scene = scene_of(ctx, actor)?;
        if scene.facilitator != actor && !is_staff(ctx.w, actor) {
            return Err(CmdError::msg(
                "Only the scene's facilitator (or staff) opens its map.",
            ));
        }
        let members: Vec<ObjId> = scene.members().collect();
        let domain = open_for_scene(ctx, "scenes", scene.id.0, scene.anchor, members)
            .map_err(CmdError::msg)?;
        say(
            ctx,
            format!("Window #{domain} opens over the scene; its map is being drawn."),
        );
        return Ok(());
    }
    if ctx.switches.has("close") {
        let scene = scene_of(ctx, actor)?;
        if scene.facilitator != actor && !is_staff(ctx.w, actor) {
            return Err(CmdError::msg(
                "Only the scene's facilitator (or staff) closes its map.",
            ));
        }
        match close_for_scene(ctx, "scenes", scene.id.0).map_err(CmdError::msg)? {
            Some(released) => say(ctx, format!("The map closes; {} released.", released.len())),
            None => return Err(CmdError::msg("The scene has no map open.")),
        }
        return Ok(());
    }
    if ctx.switches.has("floor") {
        let n: i16 =
            ctx.args.raw().trim().parse().map_err(|_| {
                CmdError::msg("Which floor? +map/floor <n>; 0 is the ground floor.")
            })?;
        let domain = mush_sim::pose_of(ctx.w, actor)
            .map(|(d, _)| d)
            .ok_or_else(|| CmdError::msg("You are not on a map."))?;
        // 16b cut S.2: the same exclusion the bare arm applies — a floor of a map
        // an arena holds is still that map.
        if mush_sim::deployed::excluded(ctx.w, mush_sim::deployed::Shown::Domain(domain)) {
            return Err(CmdError::msg(MAP_CLOSED));
        }
        let st = ctx
            .state_mut::<AtlasState>()
            .ok_or_else(|| CmdError::msg("The atlas module is not composed on this server."))?;
        let entry = st
            .windows
            .open
            .get(&domain.0)
            .ok_or_else(|| CmdError::msg(MAP_CLOSED))?;
        let index = entry
            .frame
            .surfaces
            .iter()
            .position(
                |s| matches!(s, crate::record::SurfaceRef::Floor { index, .. } if *index == n),
            )
            .ok_or_else(|| CmdError::msg(format!("The map has no floor {n}.")))?;
        st.windows.viewer_floor.insert(actor, index);
        say(ctx, format!("You look at floor {n} of the map."));
        crate::frame::send_keyframe(ctx, actor, domain);
        return Ok(());
    }
    // Bare: where you stand.
    let Some((domain, pose)) = mush_sim::pose_of(ctx.w, actor) else {
        return Err(CmdError::msg(
            "You are not on a map. A scene's facilitator opens one with +map/open.",
        ));
    };
    // 16b cut S.2 (doc 16 §3.6 "World-side exclusion"): a domain an arena holds is
    // not a map, for anybody. The answer is the closed window's own sentence —
    // `frame_for` has already stopped answering for it, and a second sentence here
    // would be a second fact about the match.
    if mush_sim::deployed::excluded(ctx.w, mush_sim::deployed::Shown::Domain(domain)) {
        return Err(CmdError::msg(MAP_CLOSED));
    }
    // S8.3 (14a #3): the text is the frame's view — built once for this
    // viewer, printed, then sent — so it names what the frame shows and
    // nothing the frame hides.
    let (lines, frame): (Vec<String>, Option<mush_proto::AtlasMapFrame>) = {
        let st = ctx
            .state::<AtlasState>()
            .ok_or_else(|| CmdError::msg("The atlas module is not composed on this server."))?;
        let entry = st
            .windows
            .open
            .get(&domain.0)
            .ok_or_else(|| CmdError::msg(MAP_CLOSED))?;
        let frame = crate::frame::frame_for(ctx.w, st, actor, domain);
        let mut lines = vec![crate::window::summary_for(ctx.w, st, entry, actor)];
        let here = ctx.w.get(actor).and_then(|o| o.location);
        // The pose is integer centimetres and millidegrees; the sentence a
        // player reads is metres and whole degrees, converted here and nowhere
        // else.
        lines.push(format!(
            "You stand at ({:.1}, {:.1}, z {:.1}) facing {}°, in {}.",
            crate::geom::metres(pose.x_cm),
            crate::geom::metres(pose.y_cm),
            crate::geom::metres(i64::from(pose.z_cm)),
            pose.heading_mdeg / 1000,
            here.map_or_else(
                || "nowhere".to_string(),
                |r| mush_core::output::display_name(ctx.w, r)
            )
        ));
        let floors: Vec<String> = entry
            .frame
            .surfaces
            .iter()
            .map(|s| match s {
                crate::record::SurfaceRef::Ground => "ground".to_string(),
                crate::record::SurfaceRef::Floor { index, .. } => format!("floor {index}"),
            })
            .collect();
        lines.push(format!("Surfaces: {}.", floors.join(", ")));
        let others: Vec<String> = frame
            .iter()
            .flat_map(|f| f.units.iter())
            .filter(|u| !u.you)
            .map(|u| format!("{} (cell {}, {})", u.name, u.cell.0, u.cell.1))
            .collect();
        if !others.is_empty() {
            lines.push(format!("On the map: {}.", others.join(", ")));
        }
        (lines, frame)
    };
    say(ctx, lines.join("\n"));
    // The frame itself: line mode prints it as the text grid, a client with
    // a map pane replaces its picture.
    if let Some(frame) = frame {
        crate::frame::send_frame(ctx, actor, &frame);
    }
    Ok(())
}

fn cmd_move(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let raw = ctx.args.raw().trim().to_string();
    if raw.is_empty() {
        return Err(CmdError::msg(
            "Usage: +move <direction> [<n>]  |  +move <x>,<y>",
        ));
    }
    let target = if let Some((x, y)) = raw.split_once(',') {
        let x: u16 = x
            .trim()
            .parse()
            .map_err(|_| CmdError::msg("A cell is <x>,<y> on the map."))?;
        let y: u16 = y
            .trim()
            .parse()
            .map_err(|_| CmdError::msg("A cell is <x>,<y> on the map."))?;
        StepTarget::Toward(x, y)
    } else {
        let mut words = raw.split_whitespace();
        let dir = words.next().unwrap_or("");
        let heading = Compass::parse(dir).ok_or_else(|| {
            CmdError::msg("Which way? north, south, east, west, or a diagonal (ne, nw, se, sw).")
        })?;
        let n: u16 = match words.next() {
            Some(n) => n
                .parse()
                .map_err(|_| CmdError::msg("How many cells? A number."))?,
            None => 1,
        };
        StepTarget::Along(heading, n)
    };
    let report = step(ctx, actor, target).map_err(CmdError::msg)?;
    let heading_word = match target {
        StepTarget::Along(h, _) => h.word().to_string(),
        StepTarget::Toward(x, y) => format!("toward ({x}, {y})"),
    };
    let room = report.room.map_or_else(
        || "open ground".to_string(),
        |r| mush_core::output::display_name(ctx.w, r),
    );
    let mut line = if report.walked == 0 {
        "You do not move".to_string()
    } else {
        format!(
            "You move {} cell{} {heading_word} to ({}, {}) in {room} (cost {}.{:02})",
            report.walked,
            if report.walked == 1 { "" } else { "s" },
            report.at.0,
            report.at.1,
            report.cost / 100,
            report.cost % 100
        )
    };
    if let Some(why) = &report.stopped {
        line.push_str(&format!(": {why}"));
    }
    line.push('.');
    say(ctx, line);
    if report.walked > 0
        && let Some((domain, _)) = mush_sim::pose_of(ctx.w, actor)
    {
        let note = format!(
            "{} moves {heading_word} to ({}, {}).",
            mush_core::output::display_name(ctx.w, actor),
            report.at.0,
            report.at.1
        );
        crate::frame::publish_delta(ctx, domain, &note);
    }
    Ok(())
}
