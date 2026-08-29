//! The `@theme`, `@gen`, and `@llm` command surface, plus the
//! [`AsyncCompletion`] that re-enters a finished draft.
//!
//! The `@gen` handler is the world-loop half of the two-phase pattern: it
//! snapshots the prompt context synchronously and defers the LLM call as an
//! [`Effect::ModuleAsync`]. The call runs off-loop (mush-llm), and the
//! completion re-enters as [`WorldCommand::ModuleAsyncDone`], routed here to
//! [`WorldgenCompletion::on_complete`], which stores the draft and notifies
//! the builder. Nothing the LLM produces is ever auto-committed.

use std::sync::Arc;

use mush_core::module::{AsyncCompletion, ContextOutcome, InputContext, WorldEvent};
use mush_core::{
    ArgShape, Attr, CmdError, CommandSpec, Ctx, Deferred, Effect, ModError, ModuleOp, ModuleWrite,
    ObjId, ObjKind, Powers, Registrar, Role, World, match_object, perm,
};
use mush_llm::LlmService;
use mush_proto::{InputMode, PromptSpec, RichText};
use smol_str::SmolStr;

use crate::data::{
    DraftPayload, DraftStatus, ExitDraft, ExitFact, GenKind, GenOutcome, GenSnapshot, NS,
    NeighborDesc, RoomFacts, StoredDraft, WorldgenState, draft_key,
};
use crate::theme::{THEME_ATTR, Theme, resolve_theme, theme_on};

/// Up to K neighbors' accepted descs seed the prompt (design 03 §5.2).
const MAX_NEIGHBORS: usize = 4;

/// Register every worldgen command. `llm` backs the staff `@llm/*` commands
/// (audit/budget/kill-switch views); the `@gen`/`@theme` handlers are
/// service-free (they only snapshot + defer / edit attributes).
pub(crate) fn register(reg: &mut Registrar<'_>, llm: Arc<LlmService>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@gen".into(),
            shape: ArgShape::Raw,
            switches: vec![
                "accept".into(),
                "edit".into(),
                "retry".into(),
                "discard".into(),
            ],
            power: Some(Powers::BUILDER_LLM),
            help: "@gen here=<prose> -- draft an LLM room description from your brief. \
                   The draft is never committed automatically; review it, then:\n\
                   @gen/accept [fields] -- write it (all fields, or any of: desc short \
                   details exits) through the normal attribute path.\n\
                   @gen/edit -- hand-tweak the draft description in a line editor.\n\
                   @gen/retry [guidance] -- regenerate; your guidance is appended and the \
                   rejected draft is avoided.\n\
                   @gen/discard -- throw the draft away.\n\
                   Requires the BUILDER_LLM power."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_gen),
    )?;
    reg.command(
        CommandSpec {
            name: "@theme".into(),
            shape: ArgShape::EqPair,
            switches: vec!["show".into(), "exemplar".into()],
            min_role: Role::Builder,
            help: "@theme <zone>=<field>:<value> -- set a theme field on a zone, room, or \
                   #0 (game default). Fields: name, style_guide, vocabulary, banned, \
                   exemplars, desc_length, pov_rules (list fields take comma-separated \
                   values).\n\
                   @theme/show [<zone>] -- show the effective (resolved) theme for a \
                   zone/room (default: here).\n\
                   @theme/exemplar <zone>=<room> -- copy a room's accepted desc in as an \
                   exemplar."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_theme),
    )?;
    reg.command(
        CommandSpec {
            name: "@llm".into(),
            shape: ArgShape::Raw,
            switches: vec![
                "log".into(),
                "budget".into(),
                "disable".into(),
                "enable".into(),
                "kill".into(),
                "revive".into(),
            ],
            min_role: Role::Wizard,
            help: "@llm/budget -- today's LLM token/cost totals.\n\
                   @llm/log [<feature>] -- recent LLM calls (optionally one feature route).\n\
                   @llm/disable -- engage the global kill switch (stops all LLM calls).\n\
                   @llm/enable -- lift the global kill switch.\n\
                   @llm/kill <npc> -- park one NPC agent (no wakes, no tokens).\n\
                   @llm/revive <npc> -- wake a killed NPC agent."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(LlmStaffCmd { llm }),
    )?;
    reg.async_completion(Arc::new(WorldgenCompletion))?;
    Ok(())
}

// ----------------------------------------------------------------------
// @gen
// ----------------------------------------------------------------------

fn cmd_gen(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("accept") {
        gen_accept(ctx)
    } else if ctx.switches.has("edit") {
        gen_edit(ctx)
    } else if ctx.switches.has("retry") {
        gen_retry(ctx)
    } else if ctx.switches.has("discard") {
        gen_discard(ctx)
    } else {
        gen_here(ctx)
    }
}

fn gen_here(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().to_string();
    let (target, prose) = raw
        .split_once('=')
        .ok_or_else(|| CmdError::msg("Usage: @gen here=<prose describing the room>."))?;
    let prose = prose.trim();
    if prose.is_empty() {
        return Err(CmdError::msg(
            "Describe the room in a sentence or two after the =.",
        ));
    }
    let room = match_object(ctx.w, ctx.actor, target.trim())?;
    // The draft is keyed by the builder's CURRENT room (every review
    // subcommand — accept/retry/edit/discard — resolves it via `here`), so a
    // draft for a room the builder is not standing in would be unreachable.
    // Enforce the `@gen here=` contract: the target must be the current room.
    // (Remote / whole-building generation is the M10 seam.)
    if here(ctx).ok() != Some(room) {
        return Err(CmdError::msg(
            "You can only @gen the room you're in. Go to that room first, then @gen here=<prose>.",
        ));
    }
    ensure_generatable_room(ctx.w, ctx.actor, room)?;

    let snapshot = build_snapshot(ctx.w, room, ctx.actor, prose.to_string(), None, None);
    defer_generation(ctx, room, &snapshot)?;
    let name = room_name(ctx.w, room);
    ctx.out.info(
        ctx.actor,
        format!("Generating a description for {name}... (the game never blocks; the draft arrives shortly.)"),
    );
    Ok(())
}

fn gen_retry(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let room = here(ctx)?;
    ensure_generatable_room(ctx.w, ctx.actor, room)?;
    let guidance = {
        let g = ctx.args.raw().trim();
        (!g.is_empty()).then(|| g.to_string())
    };
    let prior = draft_of(ctx, room).ok_or_else(|| {
        CmdError::msg("No draft here to retry. Start one with @gen here=<prose>.")
    })?;
    let snapshot = build_snapshot(
        ctx.w,
        room,
        ctx.actor,
        prior.prose.clone(),
        guidance,
        Some(prior.payload.clone()),
    );
    defer_generation(ctx, room, &snapshot)?;
    ctx.out.info(ctx.actor, "Regenerating the description...");
    Ok(())
}

fn gen_discard(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let room = here(ctx)?;
    if draft_of(ctx, room).is_none() {
        return Err(CmdError::msg("No draft here to discard."));
    }
    clear_draft(ctx, room, ctx.actor);
    ctx.out.info(ctx.actor, "Draft discarded.");
    Ok(())
}

fn gen_accept(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let room = here(ctx)?;
    if !perm::can_modify(ctx.w, ctx.actor, room) {
        return Err(CmdError::msg("Permission denied."));
    }
    let draft = draft_of(ctx, room).ok_or_else(|| {
        CmdError::msg("No draft here to accept. Start one with @gen here=<prose>.")
    })?;
    let want = AcceptFields::parse(ctx.args.raw())?;
    let payload = draft.payload;
    let mut written: Vec<&str> = Vec::new();

    if want.desc && !payload.desc.trim().is_empty() {
        set_attr_audited(ctx, room, "desc", &payload.desc);
        written.push("desc");
    }
    if want.short && !payload.short_desc.trim().is_empty() {
        set_attr_audited(ctx, room, "short_desc", &payload.short_desc);
        written.push("short_desc");
    }
    if want.details {
        let mut n = 0;
        for d in &payload.details {
            let kw = d.keyword.trim();
            if kw.is_empty() || d.desc.trim().is_empty() {
                continue;
            }
            set_attr_audited(ctx, room, &format!("detail:{kw}"), &d.desc);
            n += 1;
        }
        if n > 0 {
            written.push("details");
        }
    }
    if want.exits {
        let mut n = 0;
        for ed in &payload.exits {
            if apply_exit(ctx, room, ed) {
                n += 1;
            }
        }
        if n > 0 {
            written.push("exits");
        }
    }

    clear_draft(ctx, room, ctx.actor);
    if written.is_empty() {
        ctx.out.info(
            ctx.actor,
            "Nothing to accept (the requested fields were empty).",
        );
    } else {
        ctx.out.info(
            ctx.actor,
            format!("Accepted. Wrote: {}. Draft cleared.", written.join(", ")),
        );
    }
    Ok(())
}

fn gen_edit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let room = here(ctx)?;
    let draft = draft_of(ctx, room)
        .ok_or_else(|| CmdError::msg("No draft here to edit. Start one with @gen here=<prose>."))?;
    ctx.out.info(
        ctx.actor,
        "Editing the draft description. Type the new text; end with a single \".\" on its \
         own line, or ~q to cancel.",
    );
    ctx.out
        .info(ctx.actor, format!("Current draft:\n{}", draft.payload.desc));
    ctx.push_input_context(Box::new(DraftEditContext {
        room,
        lines: Vec::new(),
    }));
    Ok(())
}

/// Snapshot the prompt context (owned data) and defer the LLM call.
fn defer_generation(
    ctx: &mut Ctx<'_>,
    room: ObjId,
    snapshot: &GenSnapshot,
) -> Result<(), CmdError> {
    let payload = serde_json::to_string(snapshot)
        .map_err(|e| CmdError::msg(format!("Could not prepare the generation request: {e}")))?;
    let correlation = ctx.w.next_correlation_id();
    let _ = room; // room travels inside the payload/outcome
    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
        module: SmolStr::new(NS),
        actor: ctx.actor,
        correlation,
        payload,
    }));
    Ok(())
}

// ----------------------------------------------------------------------
// The async completion (re-entry)
// ----------------------------------------------------------------------

struct WorldgenCompletion;

impl AsyncCompletion for WorldgenCompletion {
    fn on_complete(&self, ctx: &mut Ctx<'_>, correlation: u64, payload: &str) {
        let outcome: GenOutcome = match serde_json::from_str(payload) {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(correlation, error = %e, "worldgen: bad completion payload");
                return;
            }
        };
        match outcome {
            GenOutcome::Err {
                builder, message, ..
            } => {
                if ctx.w.get(builder).is_some() {
                    ctx.out.error(builder, message);
                }
            }
            GenOutcome::Ok {
                room,
                builder,
                draft,
                prose,
                prompt_hash,
                model,
            } => {
                // Re-validate against current state (treaty ruling 3): the
                // room may be gone, or the builder may have lost control.
                if ctx.w.get(room).is_none_or(|o| o.kind != ObjKind::Room) {
                    if ctx.w.get(builder).is_some() {
                        ctx.out.error(
                            builder,
                            "The room no longer exists; discarding the generated draft.",
                        );
                    }
                    return;
                }
                if !perm::can_modify(ctx.w, builder, room) {
                    if ctx.w.get(builder).is_some() {
                        ctx.out.error(
                            builder,
                            "You no longer control that room; discarding the generated draft.",
                        );
                    }
                    return;
                }

                let stored = StoredDraft {
                    room,
                    builder,
                    payload: draft,
                    prose,
                    prompt_hash,
                    model,
                    created: ctx.w.now(),
                    status: DraftStatus::Pending,
                };
                store_draft(ctx, stored.clone());

                for line in render_draft_lines(ctx.w, &stored) {
                    ctx.out.info(builder, line);
                }
                emit_draft_panel(ctx, &stored);
                ctx.out.info(
                    builder,
                    "Draft ready. @gen/accept to write it, @gen/edit to tweak, \
                     @gen/retry to regenerate, or @gen/discard.",
                );
            }
        }
    }
}

// ----------------------------------------------------------------------
// @theme
// ----------------------------------------------------------------------

fn cmd_theme(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("show") {
        return theme_show(ctx);
    }
    if ctx.switches.has("exemplar") {
        return theme_exemplar(ctx);
    }
    theme_set(ctx)
}

fn theme_set(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx
        .args
        .rhs()
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg("Usage: @theme <zone>=<field>:<value>."))?;
    let (field, value) = rhs
        .split_once(':')
        .ok_or_else(|| CmdError::msg("Usage: @theme <zone>=<field>:<value>."))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(CmdError::msg("Permission denied."));
    }
    let mut theme = theme_on(ctx.w, target).unwrap_or_default();
    let canonical = theme.set_field(field, value).map_err(CmdError::msg)?;
    store_theme(ctx, target, &theme)?;
    ctx.out.info(
        ctx.actor,
        format!("Theme {canonical} set on {}.", room_name(ctx.w, target)),
    );
    Ok(())
}

fn theme_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs();
    let target = if lhs.is_empty() {
        here(ctx)?
    } else {
        match_object(ctx.w, ctx.actor, lhs)?
    };
    let theme = resolve_theme(ctx.w, target);
    ctx.out.info(
        ctx.actor,
        format!("Effective theme for {}:", room_name(ctx.w, target)),
    );
    for line in render_theme_lines(&theme) {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

fn theme_exemplar(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx
        .args
        .rhs()
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg("Usage: @theme/exemplar <zone>=<room>."))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(CmdError::msg("Permission denied."));
    }
    let room = match_object(ctx.w, ctx.actor, &rhs)?;
    let desc = ctx
        .w
        .attr(room, "desc")
        .and_then(|a| a.as_text())
        .map(str::to_string)
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| CmdError::msg("That room has no accepted description to copy."))?;
    let mut theme = theme_on(ctx.w, target).unwrap_or_default();
    theme.exemplars.push(desc);
    store_theme(ctx, target, &theme)?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Exemplar copied from {} into {}'s theme ({} total).",
            room_name(ctx.w, room),
            room_name(ctx.w, target),
            theme.exemplars.len()
        ),
    );
    Ok(())
}

fn store_theme(ctx: &mut Ctx<'_>, target: ObjId, theme: &Theme) -> Result<(), CmdError> {
    let value = serde_json::to_value(theme)
        .map_err(|e| CmdError::msg(format!("Could not serialize the theme: {e}")))?;
    ctx.w
        .set_attr(target, THEME_ATTR, Attr::json(value, ctx.actor))?;
    Ok(())
}

// ----------------------------------------------------------------------
// @llm (staff)
// ----------------------------------------------------------------------

struct LlmStaffCmd {
    llm: Arc<LlmService>,
}

/// The NPC agent-profile attribute (owned by `mush-npc`). Kept as a local
/// constant so worldgen needn't depend on `mush-npc`: `@llm/kill|revive` toggle
/// only the `enabled` flag of this opaque JSON document — the one documented
/// cross-module contract. Any other profile field is left untouched.
const AGENT_PROFILE_ATTR: &str = "agent:profile";

impl mush_core::CommandHandler for LlmStaffCmd {
    fn call(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        if ctx.switches.has("kill") || ctx.switches.has("revive") {
            return llm_toggle_npc(ctx, ctx.switches.has("revive"));
        }
        if ctx.switches.has("disable") {
            self.llm.kill_switch().set_global(true);
            ctx.out
                .info(ctx.actor, "LLM globally DISABLED (kill switch engaged).");
            return Ok(());
        }
        if ctx.switches.has("enable") {
            self.llm.kill_switch().set_global(false);
            ctx.out
                .info(ctx.actor, "LLM globally enabled (kill switch lifted).");
            return Ok(());
        }
        if ctx.switches.has("budget") {
            let g = self.llm.ledger().global();
            ctx.out.info(
                ctx.actor,
                format!(
                    "LLM budget today -- global: {} in + {} out tokens, ${:.4}. Kill switch: {}.",
                    g.input_tokens,
                    g.output_tokens,
                    g.cost_usd,
                    if self.llm.kill_switch().global_disabled() {
                        "ENGAGED"
                    } else {
                        "off"
                    }
                ),
            );
            for route in ["worldgen_room", "worldgen_brief", "npc_dialogue_major"] {
                let c = self.llm.ledger().feature(route);
                if c.input_tokens + c.output_tokens > 0 || c.cost_usd > 0.0 {
                    ctx.out.info(
                        ctx.actor,
                        format!(
                            "  {route}: {} in + {} out tokens, ${:.4}",
                            c.input_tokens, c.output_tokens, c.cost_usd
                        ),
                    );
                }
            }
            return Ok(());
        }
        // Default / /log: recent audit rows (optionally filtered by feature).
        let filter = ctx.args.raw().trim().to_string();
        let rows = self.llm.audit_log().recent(20);
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|r| filter.is_empty() || r.feature.eq_ignore_ascii_case(&filter))
            .collect();
        if rows.is_empty() {
            ctx.out.info(ctx.actor, "No matching LLM calls logged.");
            return Ok(());
        }
        ctx.out.info(ctx.actor, "Recent LLM calls:");
        for r in rows {
            let actor = r
                .actor
                .map(|a| format!("#{}", a.0))
                .unwrap_or_else(|| "-".to_string());
            ctx.out.info(
                ctx.actor,
                format!(
                    "  {} {} [{}] {}/{}t ${:.4} {}ms actor={}",
                    r.feature,
                    r.outcome.as_str(),
                    r.model,
                    r.input_tokens,
                    r.output_tokens,
                    r.cost_usd.unwrap_or(0.0),
                    r.latency_ms,
                    actor
                ),
            );
        }
        Ok(())
    }
}

/// `@llm/kill <npc>` / `@llm/revive <npc>` — the per-NPC LLM kill switch: flip
/// the NPC's `agent:profile.enabled` flag (see [`AGENT_PROFILE_ATTR`]). A
/// killed agent takes no turns and burns no tokens; the runtime re-reads the
/// flag every tick, so no state push is needed.
fn llm_toggle_npc(ctx: &mut Ctx<'_>, revive: bool) -> Result<(), CmdError> {
    let spec = ctx.args.raw().trim().to_string();
    if spec.is_empty() {
        return Err(CmdError::msg(
            "Usage: @llm/kill <npc>  (or @llm/revive <npc>).",
        ));
    }
    let npc = match_object(ctx.w, ctx.actor, &spec)?;
    if ctx.w.get(npc).is_none_or(|o| o.kind != ObjKind::Npc) {
        return Err(CmdError::msg("That is not an NPC agent."));
    }
    // Read the opaque profile JSON (or start a minimal one), set only `enabled`.
    let mut profile = ctx
        .w
        .attr(npc, AGENT_PROFILE_ATTR)
        .and_then(mush_core::Attr::as_json)
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    match profile.as_object_mut() {
        Some(obj) => {
            obj.insert("enabled".to_string(), serde_json::Value::Bool(revive));
        }
        None => profile = serde_json::json!({ "enabled": revive }),
    }
    ctx.w
        .set_attr(npc, AGENT_PROFILE_ATTR, Attr::json(profile, ctx.actor))?;
    let name = ctx
        .w
        .get(npc)
        .map_or_else(|| npc.to_string(), |o| o.name.clone());
    ctx.out.info(
        ctx.actor,
        if revive {
            format!("NPC {name} agent revived (LLM re-enabled).")
        } else {
            format!("NPC {name} agent killed (LLM disabled; no wakes, no tokens).")
        },
    );
    Ok(())
}

// ----------------------------------------------------------------------
// The draft line editor (input context)
// ----------------------------------------------------------------------

struct DraftEditContext {
    room: ObjId,
    lines: Vec<String>,
}

impl InputContext for DraftEditContext {
    fn prompt(&self, _w: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain(
                "draft-edit (\".\" to finish, ~q to cancel)> ",
            )),
        }
    }

    fn feed(&mut self, ctx: &mut Ctx<'_>, line: &str) -> ContextOutcome {
        if line.trim() == "." {
            let new_desc = self.lines.join("\n");
            if new_desc.trim().is_empty() {
                ctx.out
                    .info(ctx.actor, "Empty description; draft left unchanged.");
                return ContextOutcome::Pop;
            }
            // Update the cached + persisted draft desc in place.
            let actor = ctx.actor;
            let updated = {
                let Some(st) = ctx.state_mut::<WorldgenState>() else {
                    ctx.out.error(actor, "Worldgen state unavailable.");
                    return ContextOutcome::Pop;
                };
                match st.drafts.get_mut(&(self.room, actor)) {
                    Some(d) => {
                        d.payload.desc = new_desc;
                        Some(d.clone())
                    }
                    None => None,
                }
            };
            match updated {
                Some(d) => {
                    persist_draft(ctx, &d);
                    ctx.out.info(
                        ctx.actor,
                        "Draft description updated (still a draft; @gen/accept to commit).",
                    );
                }
                None => ctx.out.error(ctx.actor, "That draft is gone."),
            }
            return ContextOutcome::Pop;
        }
        self.lines.push(line.to_string());
        ContextOutcome::Consumed
    }
}

// ----------------------------------------------------------------------
// Snapshot assembly + draft store/persist + rendering
// ----------------------------------------------------------------------

fn build_snapshot(
    w: &World,
    room: ObjId,
    builder: ObjId,
    prose: String,
    guidance: Option<String>,
    prior_draft: Option<DraftPayload>,
) -> GenSnapshot {
    let theme = resolve_theme(w, room);
    let facts = room_facts(w, room);
    let neighbors = neighbor_descs(w, room);
    GenSnapshot {
        kind: GenKind::Room,
        room,
        builder,
        theme,
        facts,
        neighbors,
        prose,
        guidance,
        prior_draft,
    }
}

fn room_facts(w: &World, room: ObjId) -> RoomFacts {
    let name = room_name(w, room);
    let mut exits = Vec::new();
    for &e in w.exits_of(room) {
        let Some(o) = w.get(e) else { continue };
        let mut segs = o.name.split(';').map(str::trim).filter(|s| !s.is_empty());
        let primary = segs.next().unwrap_or("").to_string();
        let mut aliases: Vec<String> = segs.map(str::to_string).collect();
        aliases.extend(o.aliases.iter().map(|a| a.to_string()));
        let destination = o.destination.map(|d| room_name(w, d));
        exits.push(ExitFact {
            name: primary,
            aliases,
            destination,
        });
    }
    RoomFacts { name, exits }
}

fn neighbor_descs(w: &World, room: ObjId) -> Vec<NeighborDesc> {
    let mut out = Vec::new();
    for &e in w.exits_of(room) {
        if out.len() >= MAX_NEIGHBORS {
            break;
        }
        let Some(exit) = w.get(e) else { continue };
        let Some(dest) = exit.destination else {
            continue;
        };
        let Some(desc) = w.attr(dest, "desc").and_then(|a| a.as_text()) else {
            continue;
        };
        if desc.trim().is_empty() {
            continue;
        }
        let direction = exit.name.split(';').next().unwrap_or("").trim().to_string();
        out.push(NeighborDesc {
            direction,
            name: room_name(w, dest),
            desc: desc.to_string(),
        });
    }
    out
}

fn store_draft(ctx: &mut Ctx<'_>, draft: StoredDraft) {
    persist_draft(ctx, &draft);
    if let Some(st) = ctx.state_mut::<WorldgenState>() {
        st.drafts.insert((draft.room, draft.builder), draft);
    }
}

fn persist_draft(ctx: &mut Ctx<'_>, draft: &StoredDraft) {
    let value = match serde_json::to_value(draft) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "worldgen: could not serialize draft");
            return;
        }
    };
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet {
            key: draft_key(draft.room, draft.builder),
            value,
        },
    });
}

fn clear_draft(ctx: &mut Ctx<'_>, room: ObjId, builder: ObjId) {
    if let Some(st) = ctx.state_mut::<WorldgenState>() {
        st.drafts.remove(&(room, builder));
    }
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::Del {
            key: draft_key(room, builder),
        },
    });
}

/// The current draft for `(room, actor)`, cloned out of the state cache.
fn draft_of(ctx: &Ctx<'_>, room: ObjId) -> Option<StoredDraft> {
    ctx.state::<WorldgenState>()?
        .draft(room, ctx.actor)
        .cloned()
}

fn emit_draft_panel(ctx: &mut Ctx<'_>, draft: &StoredDraft) {
    let Some(panel) = ctx.state::<WorldgenState>().map(|s| s.panel.clone()) else {
        return;
    };
    let current_desc = ctx
        .w
        .attr(draft.room, "desc")
        .and_then(|a| a.as_text())
        .unwrap_or("")
        .to_string();
    let data = serde_json::json!({
        "room": draft.room.to_string(),
        "room_name": room_name(ctx.w, draft.room),
        "builder": draft.builder.to_string(),
        "model": draft.model,
        "current": { "desc": current_desc },
        "proposed": draft.payload,
    });
    panel.keyframe(ctx, data);
}

/// Write one attribute through the ordinary attribute-set path, emitting the
/// same `AttrSet` observer signal `@set`/`@desc` do — so an accepted draft
/// looks like any builder edit in the audit trail.
fn set_attr_audited(ctx: &mut Ctx<'_>, obj: ObjId, name: &str, value: &str) {
    if ctx
        .w
        .set_attr(obj, name, Attr::text(value, ctx.actor))
        .is_ok()
    {
        ctx.emit_world_event(&WorldEvent::AttrSet {
            obj,
            name: name.to_string(),
            setter: ctx.actor,
        });
    }
}

/// Match an [`ExitDraft`] to one of the room's existing exits (by direction
/// or suggested name against the exit's names/aliases) and write its `desc`
/// and third-person transit line. Never creates an exit. Returns whether one
/// was written.
fn apply_exit(ctx: &mut Ctx<'_>, room: ObjId, ed: &ExitDraft) -> bool {
    let needle_dir = ed.direction.trim();
    let needle_name = ed.name.trim();
    let matched = ctx.w.exits_of(room).iter().copied().find(|&e| {
        let Some(o) = ctx.w.get(e) else { return false };
        let names = o
            .name
            .split(';')
            .map(str::trim)
            .chain(o.aliases.iter().map(|a| a.as_str()));
        names.into_iter().any(|n| {
            (!needle_dir.is_empty() && n.eq_ignore_ascii_case(needle_dir))
                || (!needle_name.is_empty() && n.eq_ignore_ascii_case(needle_name))
        })
    });
    let Some(exit) = matched else { return false };
    let mut wrote = false;
    if !ed.desc.trim().is_empty() {
        set_attr_audited(ctx, exit, "desc", &ed.desc);
        wrote = true;
    }
    if !ed.transit_msg.trim().is_empty() {
        set_attr_audited(ctx, exit, "osucc", &ed.transit_msg);
        wrote = true;
    }
    wrote
}

fn render_draft_lines(w: &World, draft: &StoredDraft) -> Vec<String> {
    let mut lines = vec![
        format!(
            "--- Worldgen draft for {} (model: {}) ---",
            room_name(w, draft.room),
            draft.model
        ),
        format!("Desc: {}", draft.payload.desc),
    ];
    if !draft.payload.short_desc.trim().is_empty() {
        lines.push(format!("Short: {}", draft.payload.short_desc));
    }
    for d in &draft.payload.details {
        if !d.keyword.trim().is_empty() {
            lines.push(format!("Detail [{}]: {}", d.keyword, d.desc));
        }
    }
    for e in &draft.payload.exits {
        let handle = if e.direction.trim().is_empty() {
            e.name.clone()
        } else {
            e.direction.clone()
        };
        if !e.desc.trim().is_empty() {
            lines.push(format!("Exit [{handle}]: {}", e.desc));
        }
    }
    lines
}

fn render_theme_lines(theme: &Theme) -> Vec<String> {
    if theme.is_empty() {
        return vec!["  (no theme configured)".to_string()];
    }
    let mut lines = Vec::new();
    if !theme.name.is_empty() {
        lines.push(format!("  Name: {}", theme.name));
    }
    if !theme.style_guide.is_empty() {
        lines.push(format!("  Style: {}", theme.style_guide));
    }
    if !theme.pov_rules.is_empty() {
        lines.push(format!("  POV: {}", theme.pov_rules));
    }
    if !theme.vocabulary.is_empty() {
        lines.push(format!("  Vocabulary: {}", theme.vocabulary.join(", ")));
    }
    if !theme.banned.is_empty() {
        lines.push(format!("  Banned: {}", theme.banned.join(", ")));
    }
    lines.push(format!("  Length: {}", theme.desc_length.as_str()));
    if !theme.exemplars.is_empty() {
        lines.push(format!("  Exemplars: {}", theme.exemplars.len()));
    }
    lines
}

// ----------------------------------------------------------------------
// small helpers
// ----------------------------------------------------------------------

/// The actor's current room.
fn here(ctx: &Ctx<'_>) -> Result<ObjId, CmdError> {
    ctx.w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere."))
}

fn ensure_generatable_room(w: &World, actor: ObjId, room: ObjId) -> Result<(), CmdError> {
    match w.get(room) {
        Some(o) if o.kind == ObjKind::Room => {}
        Some(_) => return Err(CmdError::msg("You can only @gen a room.")),
        None => return Err(CmdError::msg("No such room.")),
    }
    if !perm::can_modify(w, actor, room) {
        return Err(CmdError::msg("Permission denied."));
    }
    Ok(())
}

fn room_name(w: &World, room: ObjId) -> String {
    w.get(room)
        .map(|o| o.name.clone())
        .unwrap_or_else(|| "nothing".to_string())
}

/// Which fields `@gen/accept [fields]` should write.
struct AcceptFields {
    desc: bool,
    short: bool,
    details: bool,
    exits: bool,
}

impl AcceptFields {
    fn all() -> AcceptFields {
        AcceptFields {
            desc: true,
            short: true,
            details: true,
            exits: true,
        }
    }

    fn parse(raw: &str) -> Result<AcceptFields, CmdError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(AcceptFields::all());
        }
        let mut f = AcceptFields {
            desc: false,
            short: false,
            details: false,
            exits: false,
        };
        for tok in raw.split([',', ' ']).filter(|s| !s.is_empty()) {
            match tok.to_ascii_lowercase().as_str() {
                "desc" | "description" => f.desc = true,
                "short" | "short_desc" => f.short = true,
                "details" | "detail" => f.details = true,
                "exits" | "exit" => f.exits = true,
                "all" => return Ok(AcceptFields::all()),
                other => {
                    return Err(CmdError::msg(format!(
                        "Unknown field {other:?}. Valid: desc, short, details, exits (or omit for all)."
                    )));
                }
            }
        }
        Ok(f)
    }
}
