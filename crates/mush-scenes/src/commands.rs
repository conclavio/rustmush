//! The timestop command surface: `+timestop` (+ `/who /next /pause /resume
//! /end`) and the read-only `+scene`.
//!
//! Any connected player may `+timestop` their room (becoming its facilitator);
//! driving the scene (`/next /pause /resume /end`) is the facilitator's or
//! staff's. Every mutation mirrors write-through and keeps the core
//! exclusive-scene index in step with membership.

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, ObjKind, Registrar, Role, World, perm,
};

use crate::data::{
    DEFAULT_ROUND_SECONDS, KIND_TIMESTOP, PauseReason, SceneMode, SceneRecord, SceneStatus,
    ScenesState, persist_meta, persist_scene,
};

fn state<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut ScenesState {
    ctx.w
        .module_state_mut::<ScenesState>()
        .expect("scenes state slot installed at registration")
}

fn state_ref<'a>(ctx: &'a Ctx<'_>) -> &'a ScenesState {
    ctx.w
        .module_state::<ScenesState>()
        .expect("scenes state slot installed at registration")
}

fn is_staff(w: &World, who: ObjId) -> bool {
    perm::has_role(w, who, Role::Royalty)
}

/// Announce a system line to everyone in `room` (shared impl in `lib`).
fn announce(ctx: &mut Ctx<'_>, room: ObjId, text: impl Into<String>) {
    crate::announce_scene(ctx, room, text);
}

fn to_actor(ctx: &mut Ctx<'_>, text: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "scene", text.into());
}

/// The actor's current room, or a user error.
fn actor_room(ctx: &Ctx<'_>) -> Result<ObjId, CmdError> {
    ctx.w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere."))
}

/// A one-line turn summary: round, whose turn, and the scene-local IC time.
fn turn_line(w: &World, s: &SceneRecord) -> String {
    match s.current_actor() {
        Some(a) => format!(
            "Round {} — it is {}'s turn.",
            s.round,
            display_name(w, a)
        ),
        None => format!("Round {} — no participants.", s.round),
    }
}

// ----------------------------------------------------------------------
// +timestop  (start + lifecycle switches)
// ----------------------------------------------------------------------

fn cmd_timestop(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("who") {
        show_scene(ctx)
    } else if ctx.switches.has("next") {
        timestop_next(ctx)
    } else if ctx.switches.has("pause") {
        timestop_pause(ctx)
    } else if ctx.switches.has("resume") {
        timestop_resume(ctx)
    } else if ctx.switches.has("release") {
        timestop_release(ctx)
    } else if ctx.switches.has("end") {
        timestop_end(ctx)
    } else {
        timestop_start(ctx)
    }
}

fn timestop_start(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if ctx.w.get(actor).is_none_or(|o| o.kind != ObjKind::Player) {
        return Err(CmdError::msg("Only players start a timestop."));
    }
    let room = actor_room(ctx)?;
    // One exclusive scene per room, and one per actor.
    if state_ref(ctx).active_scene_at(room).is_some() {
        return Err(CmdError::msg("A scene is already running here."));
    }
    if ctx.w.in_exclusive_scene(actor) {
        return Err(CmdError::msg("You are already in a scene."));
    }

    // Participants: connected players present, then NPCs present, in dbref
    // order (the turn order). Everyone here is frozen for the duration. An
    // object already frozen by another scene is skipped defensively (physics
    // makes this impossible today — you can't be in two rooms — but it keeps
    // the one-exclusive-scene-per-object invariant a property of the code).
    let mut participants: Vec<ObjId> = ctx
        .w
        .contents_of(room)
        .iter()
        .copied()
        .filter(|&o| {
            ctx.w.get(o).is_some_and(|x| x.kind == ObjKind::Player)
                && ctx.w.is_connected(o)
                && !ctx.w.in_exclusive_scene(o)
        })
        .collect();
    let mut npcs: Vec<ObjId> = ctx
        .w
        .contents_of(room)
        .iter()
        .copied()
        .filter(|&o| {
            ctx.w.get(o).is_some_and(|x| x.kind == ObjKind::Npc) && !ctx.w.in_exclusive_scene(o)
        })
        .collect();
    participants.sort_by_key(|o| o.0);
    npcs.sort_by_key(|o| o.0);
    participants.extend(npcs);

    let now = ctx.w.now();
    let ic_now = ctx.w.game_now_unix();
    let room_name = display_name(ctx.w, room);
    let rec = {
        let st = state(ctx);
        let id = st.alloc_uid();
        let rec = SceneRecord {
            id,
            title: format!("Timestop in {room_name}"),
            kind: smol_str::SmolStr::new(KIND_TIMESTOP),
            mode: SceneMode::TurnBased,
            status: SceneStatus::Active,
            pause_reason: None,
            exclusive: true,
            anchor: room,
            participants,
            observers: Vec::new(),
            facilitator: actor,
            created_by: actor,
            created_at: now,
            ic_started_at: ic_now,
            round_seconds: DEFAULT_ROUND_SECONDS,
            round: 1,
            turn_index: 0,
            ended_at: None,
        };
        st.scenes.insert(id, rec.clone());
        rec
    };
    // Freeze every member in the core index.
    for obj in rec.members().collect::<Vec<_>>() {
        ctx.w.set_exclusive_scene_member(obj);
    }
    let next_uid = state_ref(ctx).next_scene_uid;
    persist_scene(ctx.out, &rec);
    persist_meta(ctx.out, next_uid);

    let order: Vec<String> = rec
        .participants
        .iter()
        .map(|&p| display_name(ctx.w, p))
        .collect();
    announce(
        ctx,
        room,
        format!(
            "Time freezes — a timestop begins (scene #{}). Turn order: {}. {}",
            rec.id,
            if order.is_empty() { "(none)".into() } else { order.join(", ") },
            turn_line(ctx.w, &rec)
        ),
    );
    Ok(())
}

/// Resolve the scene the actor may drive, checking the facilitator/staff gate.
/// A participant drives their own scene; **staff** may additionally drive the
/// scene anchored to the room they are standing in even if they were never a
/// participant — the recovery path when, say, the facilitator was destroyed and
/// an admin walks in to end it.
fn driver_scene(ctx: &Ctx<'_>) -> Result<SceneRecord, CmdError> {
    let actor = ctx.actor;
    let staff = is_staff(ctx.w, actor);
    let s = state_ref(ctx)
        .active_scene_of(actor)
        .cloned()
        .or_else(|| {
            if !staff {
                return None;
            }
            let room = ctx.w.get(actor).and_then(|o| o.location)?;
            state_ref(ctx).active_scene_at(room).cloned()
        })
        .ok_or_else(|| CmdError::msg("You are not in a scene."))?;
    if s.facilitator != actor && !staff {
        return Err(CmdError::msg(
            "Only the scene's facilitator (or staff) can do that.",
        ));
    }
    Ok(s)
}

fn timestop_next(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = driver_scene(ctx)?;
    if s.status != SceneStatus::Active {
        return Err(CmdError::msg(
            "The scene is not active (resume it first).",
        ));
    }
    let (anchor, updated) = {
        let st = state(ctx);
        let rec = st.scenes.get_mut(&s.id).expect("resolved above");
        rec.advance_turn();
        (rec.anchor, rec.clone())
    };
    persist_scene(ctx.out, &updated);
    announce(ctx, anchor, turn_line(ctx.w, &updated));
    Ok(())
}

fn timestop_pause(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = driver_scene(ctx)?;
    if s.status == SceneStatus::Paused {
        return Err(CmdError::msg("The scene is already paused."));
    }
    let (anchor, updated) = {
        let st = state(ctx);
        let rec = st.scenes.get_mut(&s.id).expect("resolved above");
        rec.status = SceneStatus::Paused;
        rec.pause_reason = Some(PauseReason::Facilitator);
        (rec.anchor, rec.clone())
    };
    persist_scene(ctx.out, &updated);
    announce(ctx, anchor, format!("The timestop (scene #{}) is paused.", updated.id));
    Ok(())
}

fn timestop_resume(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = driver_scene(ctx)?;
    if s.status == SceneStatus::Active {
        return Err(CmdError::msg("The scene is already active."));
    }
    let (anchor, updated) = {
        let st = state(ctx);
        let rec = st.scenes.get_mut(&s.id).expect("resolved above");
        rec.status = SceneStatus::Active;
        rec.pause_reason = None;
        (rec.anchor, rec.clone())
    };
    persist_scene(ctx.out, &updated);
    announce(
        ctx,
        anchor,
        format!("The timestop (scene #{}) resumes. {}", updated.id, turn_line(ctx.w, &updated)),
    );
    Ok(())
}

/// `+timestop/release <participant>` — the facilitator (or staff) surgically
/// releases one participant from the freeze, without ending the scene. This is
/// the sanctioned extraction path: a frozen non-staff player cannot be
/// `@teleport`ed out (the move gate holds the moved object), so release is how
/// you free someone stuck in a scene. Releasing the last participant ends it;
/// the facilitator releases *themselves* via `+timestop/end`.
fn timestop_release(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = driver_scene(ctx)?;
    let arg = ctx.args.raw().trim().to_string();
    if arg.is_empty() {
        return Err(CmdError::msg("Usage: +timestop/release <participant>"));
    }
    let who = mush_core::match_object(ctx.w, ctx.actor, &arg)?;
    if who == s.facilitator {
        return Err(CmdError::msg(
            "The facilitator ends the scene with +timestop/end, not /release.",
        ));
    }
    let now = ctx.w.now();
    let (anchor, updated, found, closed) = {
        let st = state(ctx);
        let rec = st.scenes.get_mut(&s.id).expect("resolved above");
        let found = rec.remove_member(who);
        let closed = found && rec.participants.is_empty();
        if closed {
            rec.status = SceneStatus::Closed;
            rec.pause_reason = None;
            rec.ended_at = Some(now);
        }
        (rec.anchor, rec.clone(), found, closed)
    };
    if !found {
        return Err(CmdError::msg("They are not in this scene."));
    }
    ctx.w.clear_exclusive_scene_member(who);
    if closed {
        for m in updated.members().collect::<Vec<_>>() {
            ctx.w.clear_exclusive_scene_member(m);
        }
    }
    persist_scene(ctx.out, &updated);
    let name = display_name(ctx.w, who);
    announce(
        ctx,
        anchor,
        if closed {
            format!("{name} leaves — the timestop (scene #{}) ends.", updated.id)
        } else {
            format!("{name} is released from the timestop (scene #{}).", updated.id)
        },
    );
    Ok(())
}

fn timestop_end(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = driver_scene(ctx)?;
    let now = ctx.w.now();
    let (anchor, members, updated) = {
        let st = state(ctx);
        let rec = st.scenes.get_mut(&s.id).expect("resolved above");
        rec.status = SceneStatus::Closed;
        rec.pause_reason = None;
        rec.ended_at = Some(now);
        let members: Vec<ObjId> = rec.members().collect();
        (rec.anchor, members, rec.clone())
    };
    // Release the freeze atomically.
    for obj in members {
        ctx.w.clear_exclusive_scene_member(obj);
    }
    persist_scene(ctx.out, &updated);
    announce(
        ctx,
        anchor,
        format!("Time resumes — the timestop (scene #{}) ends.", updated.id),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// +scene  (read-only)
// ----------------------------------------------------------------------

fn show_scene(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(s) = state_ref(ctx).active_scene_of(actor).cloned() else {
        to_actor(ctx, "You are not in a scene.");
        return Ok(());
    };
    let mut lines = vec![
        format!("{} (scene #{}) — {}", s.title, s.id, s.status.label()),
        turn_line(ctx.w, &s),
    ];
    let order: Vec<String> = s
        .participants
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let marker = if i == s.turn_index { " <=" } else { "" };
            format!("  {}{}", display_name(ctx.w, p), marker)
        })
        .collect();
    lines.push("Participants (turn order):".to_string());
    lines.extend(order);
    if let Some(PauseReason::ServerRestart) = s.pause_reason {
        lines.push(
            "  (paused by a server restart — the facilitator must +timestop/resume)".to_string(),
        );
    }
    for line in lines {
        to_actor(ctx, line);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// Registration
// ----------------------------------------------------------------------

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+timestop".into(),
            shape: ArgShape::Raw,
            switches: vec![
                "who".into(),
                "next".into(),
                "pause".into(),
                "resume".into(),
                "release".into(),
                "end".into(),
            ],
            help: "+timestop  --  Freeze the current room into an exclusive, turn-based scene: \
                   everyone present is held in place (no wandering off), NPCs stop acting on \
                   their own, and initiative order + rounds are tracked. You become its \
                   facilitator.\n\
                   +timestop/who   --  Show the scene here (round, whose turn, order).\n\
                   +timestop/next  --  (facilitator/staff) Advance to the next turn; wrapping \
                   past the last starts a new round.\n\
                   +timestop/pause , +timestop/resume  --  Freeze / unfreeze turn advancement.\n\
                   +timestop/release <who>  --  (facilitator/staff) Release one participant from \
                   the freeze without ending the scene (the way to free a stuck player).\n\
                   +timestop/end   --  End the scene and release everyone.\n\
                   Staff (Royalty+) can drive or end a scene by standing in its room. A scene \
                   has a durable id and survives a server restart, rehydrating paused until a \
                   facilitator resumes it. See also +scene."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_timestop),
    )?;
    reg.command(
        CommandSpec {
            name: "+scene".into(),
            shape: ArgShape::None,
            help: "+scene  --  Show the scene you are in: its status, the round and whose turn \
                   it is, and the participants in turn order (your position marked)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(show_scene),
    )?;
    Ok(())
}
