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
    DEFAULT_ROUND_SECONDS, KIND_TIMESTOP, MAX_IC_FLOOR_LEAD_SECS, MAX_OUTCOME_NOTE_BYTES,
    MAX_OUTCOME_SUBJECTS, PauseReason, SceneEvent, SceneEventKind, SceneMode, SceneOutcome,
    SceneRecord, SceneStatus, SceneUid, ScenesState, persist_meta, persist_scene,
};

/// Record one lifecycle event into `s`'s log (stamped with the current wall
/// time, `s`'s scene-local IC time, and its present membership as witnesses).
fn log_event(
    ctx: &mut Ctx<'_>,
    s: &SceneRecord,
    kind: SceneEventKind,
    actor: Option<ObjId>,
    detail: serde_json::Value,
) {
    let now = ctx.w.now();
    crate::record_scene_event(ctx, s.id, SceneEvent::new(s, now, kind, actor, detail));
}

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
        Some(a) => format!("Round {} — it is {}'s turn.", s.round, display_name(w, a)),
        None => format!("Round {} — no participants.", s.round),
    }
}

// ----------------------------------------------------------------------
// +timestop  (start + lifecycle switches)
// ----------------------------------------------------------------------

fn cmd_timestop(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("who") {
        show_scene(ctx)
    } else if ctx.switches.has("log") {
        timestop_log(ctx)
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
    let ic_now = ctx.w.world_ic_now();
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
            clock_generation: 1,
            turn: 0,
            round_seconds: DEFAULT_ROUND_SECONDS,
            round: 1,
            turn_index: 0,
            ended_at: None,
            sealer: None,
            seal_run: None,
            outcome: None,
            ic_floor: 0,
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
    log_event(
        ctx,
        &rec,
        SceneEventKind::Started,
        Some(actor),
        serde_json::json!({
            "participants": rec.participants.iter().map(|o| o.0).collect::<Vec<_>>()
        }),
    );

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
            if order.is_empty() {
                "(none)".into()
            } else {
                order.join(", ")
            },
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

/// The scene the actor may READ here (`+scene`, `/who`, `/log`, `+recap`): the
/// most recent scene they are a member of — **including one that just closed**,
/// so the transcript stays reviewable right after it ends — or, for staff, the
/// most recent scene anchored to the room they stand in. No facilitator gate;
/// reads are open to any member.
fn readable_scene(ctx: &Ctx<'_>) -> Option<SceneRecord> {
    let actor = ctx.actor;
    state_ref(ctx).latest_scene_of(actor).cloned().or_else(|| {
        if !is_staff(ctx.w, actor) {
            return None;
        }
        let room = ctx.w.get(actor).and_then(|o| o.location)?;
        state_ref(ctx).latest_scene_at(room).cloned()
    })
}

fn timestop_next(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = driver_scene(ctx)?;
    if s.status != SceneStatus::Active {
        return Err(CmdError::msg("The scene is not active (resume it first)."));
    }
    let (anchor, updated) = {
        let st = state(ctx);
        let rec = st.scenes.get_mut(&s.id).expect("resolved above");
        rec.advance_turn()
            .map_err(|error| CmdError::msg(error.to_string()))?;
        (rec.anchor, rec.clone())
    };
    persist_scene(ctx.out, &updated);
    announce(ctx, anchor, turn_line(ctx.w, &updated));
    log_event(
        ctx,
        &updated,
        SceneEventKind::TurnAdvanced,
        updated.current_actor(),
        serde_json::json!({ "round": updated.round }),
    );
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
    announce(
        ctx,
        anchor,
        format!("The timestop (scene #{}) is paused.", updated.id),
    );
    log_event(
        ctx,
        &updated,
        SceneEventKind::Paused,
        Some(ctx.actor),
        serde_json::json!({}),
    );
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
        format!(
            "The timestop (scene #{}) resumes. {}",
            updated.id,
            turn_line(ctx.w, &updated)
        ),
    );
    log_event(
        ctx,
        &updated,
        SceneEventKind::Resumed,
        Some(ctx.actor),
        serde_json::json!({}),
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
    // The facilitator always remains a participant (they cannot release
    // themselves — that is `/end`), so a release can never empty the scene:
    // there is no close path here.
    let (anchor, updated, found) = {
        let st = state(ctx);
        let rec = st.scenes.get_mut(&s.id).expect("resolved above");
        let found = rec.remove_member(who);
        (rec.anchor, rec.clone(), found)
    };
    if !found {
        return Err(CmdError::msg("They are not in this scene."));
    }
    ctx.w.clear_exclusive_scene_member(who);
    persist_scene(ctx.out, &updated);
    let name = display_name(ctx.w, who);
    announce(
        ctx,
        anchor,
        format!(
            "{name} is released from the timestop (scene #{}).",
            updated.id
        ),
    );
    log_event(
        ctx,
        &updated,
        SceneEventKind::Released,
        Some(who),
        serde_json::json!({}),
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
    log_event(
        ctx,
        &updated,
        SceneEventKind::Ended,
        Some(ctx.actor),
        serde_json::json!({}),
    );
    crate::emit_scene_concluded(ctx, &updated);
    Ok(())
}

// ----------------------------------------------------------------------
// +scene  (read-only)  +  +timestop/log  +  +recap
// ----------------------------------------------------------------------

/// Render one recorded [`SceneEvent`] to a one-line summary.
fn render_event(w: &World, ev: &SceneEvent) -> String {
    let who = ev.actor.map_or_else(String::new, |a| display_name(w, a));
    let s = |k: &str| ev.detail.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match ev.kind {
        SceneEventKind::Started => "The scene began.".to_string(),
        SceneEventKind::TurnAdvanced => {
            let round = ev
                .detail
                .get("round")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            format!("Round {round} — {who}'s turn.")
        }
        SceneEventKind::Paused => "The scene was paused.".to_string(),
        SceneEventKind::Resumed => "The scene resumed.".to_string(),
        SceneEventKind::Released => format!("{who} was released from the scene."),
        SceneEventKind::Ended => "The scene ended.".to_string(),
        SceneEventKind::Said => format!("{who} [{}]: {}", s("narr"), s("text")),
        SceneEventKind::Rolled => format!("{who} rolled: {}", s("summary")),
        SceneEventKind::Joined => format!("{who} arrives."),
        SceneEventKind::Left => format!("{who} leaves."),
        SceneEventKind::Sealed => format!("{who} sealed the outcome: {}.", s("outcome")),
    }
}

/// `+timestop/log` — the recent (RAM-resident tail) events of the scene you are
/// in, as a deterministic mechanical timeline. Older, sealed history needs the
/// store-read seam (a later prerequisite); for a normal-length scene the tail
/// is the whole log.
fn timestop_log(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(s) = readable_scene(ctx) else {
        to_actor(ctx, "You are not in a scene.");
        return Ok(());
    };
    let log_id = s.id.to_string();
    // Distinguish "nothing recorded" from "the open tail rolled over" — the
    // total is the ledger's sequence count, not just the RAM tail's length.
    let total = state_ref(ctx).events.next_seq(&log_id);
    if total == 0 {
        let msg = if s.status == SceneStatus::Closed {
            format!(
                "Scene #{}'s transcript is archived; reading a finished scene from a prior \
                 session needs the archive (not yet built).",
                s.id
            )
        } else {
            format!("Scene #{} has no recorded events yet.", s.id)
        };
        to_actor(ctx, msg);
        return Ok(());
    }
    let entries = state_ref(ctx).events.open_entries(&log_id).to_vec();
    let lines: Vec<String> = entries
        .iter()
        .filter_map(|e| {
            serde_json::from_value::<SceneEvent>(e.value.clone())
                .ok()
                .map(|ev| format!("  {:>3}. {}", e.seq, render_event(ctx.w, &ev)))
        })
        .collect();
    let omitted = total.saturating_sub(lines.len() as u64);
    let header = if omitted > 0 {
        format!(
            "Scene #{} — showing {} of {} events ({} earlier not shown here):",
            s.id,
            lines.len(),
            total,
            omitted
        )
    } else {
        format!("Scene #{} — {} event(s):", s.id, lines.len())
    };
    to_actor(ctx, header);
    for line in lines {
        to_actor(ctx, line);
    }
    Ok(())
}

/// Render one witnessed [`SceneEvent`] as a line of narrative prose, or `None`
/// to omit purely structural events from the recap.
fn recap_line(ctx: &Ctx<'_>, ev: &SceneEvent) -> Option<String> {
    let who = ev
        .actor
        .map_or_else(String::new, |a| display_name(ctx.w, a));
    let s = |k: &str| {
        ev.detail
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    match ev.kind {
        SceneEventKind::Started => Some("The scene begins.".to_string()),
        SceneEventKind::TurnAdvanced => {
            let r = ev
                .detail
                .get("round")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            Some(format!("-- round {r} --"))
        }
        SceneEventKind::Said => Some(match s("narr").as_str() {
            "say" => format!("{who} says, \"{}\"", s("text")),
            "pose" => format!("{who} {}", s("text")),
            "semipose" => format!("{who}{}", s("text")),
            "emit" | "spoof" => s("text"),
            _ => format!("{who}: {}", s("text")),
        }),
        SceneEventKind::Rolled => Some(format!("{who} rolls — {}", s("summary"))),
        SceneEventKind::Joined => Some(format!("{who} arrives.")),
        SceneEventKind::Left => Some(format!("{who} departs.")),
        SceneEventKind::Ended => Some("The scene ends.".to_string()),
        SceneEventKind::Sealed => Some(format!("The outcome is sealed: {}.", s("outcome"))),
        // Pause/resume/release are operational, not narrative.
        SceneEventKind::Paused | SceneEventKind::Resumed | SceneEventKind::Released => None,
    }
}

/// `+recap` — a narrative recap of the scene you are in, filtered to the events
/// **you witnessed** (a personal recap; staff see everything). Distinct from
/// `+timestop/log`, which is the raw mechanical timeline. Reads the RAM-resident
/// tail; a finished scene from a prior session needs the archive (store-read
/// seam), not yet built.
fn scene_recap(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(s) = readable_scene(ctx) else {
        to_actor(ctx, "You are not in a scene.");
        return Ok(());
    };
    let staff = is_staff(ctx.w, actor);
    let log_id = s.id.to_string();
    let total = state_ref(ctx).events.next_seq(&log_id);
    if total == 0 {
        let msg = if s.status == SceneStatus::Closed {
            "That scene's transcript is archived; reading a finished scene from a prior session \
             needs the archive (not yet built)."
        } else {
            "Nothing has happened yet."
        };
        to_actor(
            ctx,
            format!("Recap of {} (scene #{}): {msg}", s.title, s.id),
        );
        return Ok(());
    }
    let entries = state_ref(ctx).events.open_entries(&log_id).to_vec();
    let omitted = total.saturating_sub(entries.len() as u64);
    let lines: Vec<String> = entries
        .iter()
        .filter_map(|e| serde_json::from_value::<SceneEvent>(e.value.clone()).ok())
        // Personal recap: only what this viewer perceived (all events, for
        // staff). The witness set was frozen when each event was recorded.
        .filter(|ev| staff || ev.actor == Some(actor) || ev.witnesses.contains(&actor))
        .filter_map(|ev| recap_line(ctx, &ev))
        .collect();
    to_actor(
        ctx,
        format!(
            "Recap of {} (scene #{}) — as you witnessed it:",
            s.title, s.id
        ),
    );
    if omitted > 0 {
        to_actor(
            ctx,
            format!("  (… {omitted} earlier event(s) not shown here …)"),
        );
    }
    if lines.is_empty() {
        to_actor(ctx, "  (nothing you witnessed)");
    }
    for l in lines {
        to_actor(ctx, format!("  {l}"));
    }
    Ok(())
}

fn show_scene(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(s) = readable_scene(ctx) else {
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
// ---- E6b: B6 — the conferred sealer, the outcome, and the IC floor ----
// ----------------------------------------------------------------------

/// Rule 2 (08i §4): **nothing may let an NPC's completion seal a settlement.**
///
/// The sealer of a scene outcome must be a live world *player* object. An
/// `ObjKind::Npc` is driven by `NpcModule::on_complete` (and, in a later cut, by
/// an LLM controller), so conferring the seal on one would hand a settlement
/// authority to a process, not a principal. Refused at conferral **and** at
/// seal, because the object could in principle change kind in between.
fn is_sealable_principal(w: &World, who: ObjId) -> bool {
    w.get(who).is_some_and(|o| o.kind == ObjKind::Player)
}

/// The scene a **staff** verb names: `@scene/... <id|here>=...`.
fn staff_scene(ctx: &Ctx<'_>, arg: &str) -> Result<SceneRecord, CmdError> {
    let arg = arg.trim();
    let st = state_ref(ctx);
    if arg.is_empty() || arg.eq_ignore_ascii_case("here") {
        let room = actor_room(ctx)?;
        return st
            .latest_scene_at(room)
            .cloned()
            .ok_or_else(|| CmdError::msg("No such scene."));
    }
    let id = arg
        .trim_start_matches('#')
        .parse::<u64>()
        .map_err(|_| CmdError::msg("No such scene."))?;
    st.scenes
        .get(&SceneUid(id))
        .cloned()
        .ok_or_else(|| CmdError::msg("No such scene."))
}

/// Write `rec` back into the state and mirror it. One path, so a scene mutation
/// can never persist without the state agreeing with it.
fn store_scene(ctx: &mut Ctx<'_>, rec: &SceneRecord) {
    if let Some(st) = ctx.w.module_state_mut::<ScenesState>() {
        st.scenes.insert(rec.id, rec.clone());
    }
    persist_scene(ctx.out, rec);
}

/// `+scene` — the read, plus E6b's `/outcome` seal.
fn cmd_scene(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("outcome") {
        scene_outcome(ctx)
    } else {
        show_scene(ctx)
    }
}

fn cmd_scene_staff(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("facilitate") {
        scene_facilitate(ctx)
    } else if ctx.switches.has("icfloor") {
        scene_icfloor(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @scene/facilitate <scene>=<who>  |  @scene/icfloor <scene>=<unix>",
        ))
    }
}

/// `@scene/facilitate <scene>=<who>` (staff) — confer the **seal** authority.
/// This is not the scene's `facilitator`, which any player grants themselves by
/// starting a scene; it is the conferred authority 08f §4.1 requires.
fn scene_facilitate(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @scene/facilitate <scene>=<who|none> [run=<id>]";
    let lhs = ctx.args.lhs().to_string();
    let raw = ctx.args.rhs().unwrap_or_default().to_string();
    let mut rec = staff_scene(ctx, &lhs)?;
    // E6b verify fold (M2/R12): staff confer **which run** this scene may seal, and
    // `+scene/outcome` may name no other. The id is opaque — `mush-scenes` has no
    // economy edge and never asks what it means — but binding it at conferral is
    // what stops a sealer conferred on one scene from settling somebody else's run
    // by typing its number.
    let (rhs, run) = match find_key(&raw, "run=") {
        Some(at) => {
            let rest = raw[at + "run=".len()..].trim();
            let token = rest.split_whitespace().next().unwrap_or("");
            let run = token
                .trim_start_matches('#')
                .parse::<u64>()
                .map_err(|_| CmdError::msg(USAGE))?;
            (raw[..at].trim().to_string(), Some(run))
        }
        None => (raw.trim().to_string(), None),
    };
    if rhs.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    if rhs.eq_ignore_ascii_case("none") {
        if rec.outcome.is_some() {
            return Err(CmdError::msg(
                "That scene is already sealed; its sealer is part of the record.",
            ));
        }
        rec.sealer = None;
        rec.seal_run = None;
        store_scene(ctx, &rec);
        to_actor(ctx, format!("Scene #{} can no longer be sealed.", rec.id));
        return Ok(());
    }
    let who = mush_core::match_object(ctx.w, ctx.actor, &rhs)?;
    if !is_sealable_principal(ctx.w, who) {
        return Err(CmdError::msg(
            "Only a player may be conferred a scene's seal — an NPC is driven, not accountable.",
        ));
    }
    if rec.outcome.is_some() {
        return Err(CmdError::msg(
            "That scene is already sealed; its outcome is terminal.",
        ));
    }
    rec.sealer = Some(who);
    rec.seal_run = run;
    store_scene(ctx, &rec);
    let name = display_name(ctx.w, who);
    to_actor(
        ctx,
        match run {
            Some(id) => format!(
                "{name} may now seal the outcome of scene #{} for run {id}, and for no other.",
                rec.id
            ),
            None => format!(
                "{name} may now seal the outcome of scene #{} — for the chronicle only, since no                  run was named (`run=<id>` binds it to one).",
                rec.id
            ),
        },
    );
    Ok(())
}

/// `@scene/icfloor <scene>=<unix>` (staff) — raise the scene's IC time floor.
/// Monotonic, and it never writes `clock_generation` (08i §9 answer 9).
fn scene_icfloor(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_string();
    let mut rec = staff_scene(ctx, &lhs)?;
    let now = ctx.w.game_now_unix();
    let at = if rhs.eq_ignore_ascii_case("now") {
        now
    } else {
        rhs.parse::<i64>()
            .map_err(|_| CmdError::msg("Usage: @scene/icfloor <scene>=<unix seconds|now>"))?
    };
    // E6b verify fold (R4): **monotonic and unrepairable means bounded.** The raise
    // has no inverse anywhere in the crate, so one pasted epoch-millis value would
    // stamp every future canon record from this scene at the year 55 000 for ever.
    // A floor is for catching a scene clock up to the world, so it may not run ahead
    // of the world it is catching up to.
    if at > now.saturating_add(MAX_IC_FLOOR_LEAD_SECS) {
        return Err(CmdError::msg(format!(
            "That floor is further ahead of the world's IC clock than a scene may be raised              (at most {MAX_IC_FLOOR_LEAD_SECS} s). The world reads {now}."
        )));
    }
    // And never after the seal: the outcome's `ic_time` is already canon.
    if rec.outcome.is_some() {
        return Err(CmdError::msg(
            "That scene is already sealed; its IC time is part of the chronicle.",
        ));
    }
    let generation = rec.clock_generation;
    rec.raise_ic_floor(at).map_err(CmdError::msg)?;
    debug_assert_eq!(generation, rec.clock_generation);
    if rec.validate_clock().is_err() {
        return Err(CmdError::msg("The scene IC floor only moves forward."));
    }
    store_scene(ctx, &rec);
    to_actor(
        ctx,
        format!(
            "Scene #{}'s IC floor is now {at} (its clock reads {}).",
            rec.id,
            rec.scene_ic_time()
        ),
    );
    Ok(())
}

/// What `+scene/outcome`'s right-hand side parsed to.
struct OutcomeArgs {
    subjects: Vec<String>,
    run: Option<u64>,
    note: String,
}

/// Parse `<subject>[, <subject>…] run=<id> [note=<text>]`.
///
/// `note=` is greedy to end of line (chronicle prose contains spaces); `run=`
/// is a single token. Everything before either is the comma-separated subject
/// list. Bounded by [`MAX_OUTCOME_SUBJECTS`] and [`MAX_OUTCOME_NOTE_BYTES`].
fn parse_outcome_args(rhs: &str) -> Result<OutcomeArgs, CmdError> {
    let usage = || {
        CmdError::msg("Usage: +scene/outcome <row>=<subject>[,<subject>] run=<id> [note=<text>]")
    };
    let (head, note) = match find_key(rhs, "note=") {
        Some(at) => (&rhs[..at], rhs[at + "note=".len()..].trim().to_string()),
        None => (rhs, String::new()),
    };
    let (head, run) = match find_key(head, "run=") {
        Some(at) => {
            let rest = &head[at + "run=".len()..];
            let token = rest.split_whitespace().next().unwrap_or("");
            let run = token
                .trim_start_matches('#')
                .parse::<u64>()
                .map_err(|_| usage())?;
            (&head[..at], Some(run))
        }
        None => (head, None),
    };
    if note.len() > MAX_OUTCOME_NOTE_BYTES {
        return Err(CmdError::msg(format!(
            "A sealed outcome's note is at most {MAX_OUTCOME_NOTE_BYTES} bytes."
        )));
    }
    let subjects: Vec<String> = head
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if subjects.is_empty() {
        return Err(usage());
    }
    if subjects.len() > MAX_OUTCOME_SUBJECTS {
        return Err(CmdError::msg(format!(
            "A sealed outcome names at most {MAX_OUTCOME_SUBJECTS} subjects."
        )));
    }
    Ok(OutcomeArgs {
        subjects,
        run,
        note,
    })
}

/// The byte offset of `key` in `text` at a token boundary, case-insensitively.
fn find_key(text: &str, key: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find(key) {
        let at = from + rel;
        let boundary = at == 0
            || lower[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if boundary {
            return Some(at);
        }
        from = at + key.len();
    }
    None
}

/// `+scene/outcome <row>=<subject…> run=<id> [note=<text>]` — **the seal**.
///
/// Only from the conferred `sealer` or staff, only once, only on an
/// `Active`/`Paused`/`Closed` scene, and only on a scene that has a sealer at
/// all (fail-closed: `None` means this scene cannot be sealed). The commit is
/// terminal (`mush_canon::commit` pins the deferred prefix), so it is the last
/// mutating act on the path.
fn scene_outcome(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let staff = is_staff(ctx.w, actor);
    let name = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().unwrap_or_default().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(
            "Usage: +scene/outcome <row>=<subject>[,<subject>] run=<id> [note=<text>]",
        ));
    }
    let Some(mut rec) = readable_scene(ctx) else {
        return Err(CmdError::msg("You are not in a scene."));
    };
    // Fail closed, before anything else is resolved: no conferred sealer, no
    // seal — for staff too. A scene nobody was made accountable for produces no
    // outcome at all (08f §4.1).
    // E6b verify fold (D9): **one refusal.** "Nobody was conferred the seal here" and
    // "somebody was, and it is not you" were two messages, so any participant could
    // read a staff conferral off the difference — and conferral is told only to the
    // staffer who made it. Staff, who may see everything anyway, keep the detail.
    const NOT_YOURS: &str = "You cannot seal this scene's outcome.";
    let Some(sealer) = rec.sealer else {
        return Err(CmdError::msg(if staff {
            "No one has been conferred the seal on this scene (@scene/facilitate)."
        } else {
            NOT_YOURS
        }));
    };
    if sealer != actor && !staff {
        return Err(CmdError::msg(NOT_YOURS));
    }
    // Rule 2, again at seal: neither the conferred sealer nor the actor may be
    // an NPC, whose completion would otherwise seal a settlement.
    if !is_sealable_principal(ctx.w, sealer) || !is_sealable_principal(ctx.w, actor) {
        return Err(CmdError::msg(
            "Only a player may seal a scene outcome — an NPC is driven, not accountable.",
        ));
    }
    if rec.outcome.is_some() {
        return Err(CmdError::msg(
            "That scene's outcome is already sealed; a seal is written once.",
        ));
    }
    // Rule 2, again at seal: neither the conferred sealer nor the actor may be an
    // NPC. (The check above this one refuses before the scene's state is disclosed.)
    if matches!(rec.status, SceneStatus::Preparing) {
        return Err(CmdError::msg(
            "A scene that has not begun has no outcome to seal.",
        ));
    }
    let args = parse_outcome_args(&rhs)?;
    // E6b verify fold (M2/R12): the seal names **the run staff bound to this scene**,
    // and no other. Giving `run=` is allowed as a confirmation and must agree.
    if let Some(named) = args.run
        && rec.seal_run != Some(named)
    {
        return Err(CmdError::msg(
            "This scene's seal is not for that run. Staff bind a scene to the run it settles              (@scene/facilitate <scene>=<who> run=<id>).",
        ));
    }
    let mut subjects: Vec<ObjId> = Vec::with_capacity(args.subjects.len());
    for s in &args.subjects {
        let who = mush_core::match_object(ctx.w, actor, s)?;
        if !subjects.contains(&who) {
            subjects.push(who);
        }
    }
    let now = ctx.w.now();
    let outcome = SceneOutcome::new(
        smol_str::SmolStr::new(&name),
        subjects,
        args.note,
        actor,
        now,
    )
    .ok_or_else(|| CmdError::msg("That outcome does not fit a seal's bounds."))?;
    // OPEN 3: the seal **raises the floor to the world IC clock first**, then
    // stamps `scene_ic_time()`. One rule, identical value: the chronicle can
    // never sit before events the settlement followed, and the general floor is
    // the same floor `@scene/icfloor` and cut 6's raise use.
    let ic_now = ctx.w.game_now_unix();
    let _ = rec.raise_ic_floor(ic_now);
    rec.outcome = Some(outcome.clone());
    if let Err(error) = rec.validate_clock() {
        return Err(CmdError::msg(format!(
            "That seal is not recordable: {error}"
        )));
    }
    store_scene(ctx, &rec);
    log_event(
        ctx,
        &rec,
        SceneEventKind::Sealed,
        Some(actor),
        serde_json::json!({ "outcome": name, "run": rec.seal_run }),
    );
    let anchor = rec.anchor;
    announce(
        ctx,
        anchor,
        format!("The outcome of scene #{} is sealed: {name}.", rec.id),
    );
    // Terminal, and therefore last (`mush_canon::commit` pins the deferred
    // prefix; nothing mutating may follow it on a path that can still fail).
    crate::emit_scene_outcome(ctx, &rec, &outcome);
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
                "log".into(),
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
                   +timestop/log   --  Show the scene's recorded event timeline (speech, turns, \
                   arrivals) — a deterministic transcript of what happened.\n\
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
            shape: ArgShape::EqPair,
            switches: vec!["outcome".into()],
            help: "+scene  --  Show the scene you are in: its status, the round and whose turn \
                   it is, and the participants in turn order (your position marked).\n\
                   +scene/outcome <row>=<subject>[,<subject>] run=<id> [note=<text>]  --  Seal \
                   this scene's outcome. Only the conferred sealer (or staff) may, only once, \
                   and only on a scene a staffer conferred the seal on (@scene/facilitate); the \
                   subjects are named here, never inherited from the participant list, and the \
                   run is the contract the seal answers. The seal becomes canon."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_scene),
    )?;
    reg.command(
        CommandSpec {
            name: "@scene".into(),
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            switches: vec!["facilitate".into(), "icfloor".into()],
            help: "@scene/facilitate <scene>=<who|none>  --  Confer (or withdraw) the authority \
                   to seal that scene's outcome. This is NOT the scene's facilitator, which any \
                   player grants themselves by starting a scene: a scene with no conferred \
                   sealer cannot be sealed at all. Only a player may be conferred it.\n\
                   @scene/icfloor <scene>=<unix|now>  --  Raise the scene's IC time floor. \
                   Monotonic (a lower value is refused) and it never re-clocks the scene.\n\
                   <scene> is a durable scene id, or `here` for the room you stand in."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_scene_staff),
    )?;
    reg.command(
        CommandSpec {
            name: "+recap".into(),
            shape: ArgShape::None,
            help: "+recap  --  A narrative recap of the scene you are in, told as the events you \
                   witnessed: speech, arrivals, the rounds. (The raw mechanical timeline is \
                   +timestop/log.)"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(scene_recap),
    )?;
    Ok(())
}
