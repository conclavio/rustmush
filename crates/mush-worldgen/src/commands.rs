//! The `@gen` and `@llm` command surface (the theme verbs moved to `@atlas/theme`
//! in S2b), plus the
//! [`AsyncCompletion`] that re-enters a finished draft.
//!
//! The `@gen` handler is the world-loop half of the two-phase pattern: it
//! snapshots the prompt context synchronously and defers the LLM call as an
//! [`Effect::ModuleAsync`]. The call runs off-loop (mush-llm), and the
//! completion re-enters as `WorldCommand::ModuleAsyncDone`, routed here to
//! [`WorldgenCompletion::on_complete`], which stores the draft and notifies
//! the builder. Nothing the LLM produces is ever auto-committed.

use std::sync::Arc;

use mush_core::module::{AsyncCompletion, ContextOutcome, InputContext, WorldEvent};
use mush_core::{
    ArgShape, Attr, CmdError, CommandSpec, Ctx, Deferred, Effect, ModError, ModuleOp, ModuleWrite,
    ObjId, ObjKind, Powers, QueryHandler, Registrar, Role, World, match_object, perm,
};
use mush_llm::{BudgetStatus, DashboardSnapshot, LlmService, RecentCall, SpendRow};
use mush_proto::{InputMode, OutputEvent, PanelFrameKind, PromptSpec, RichText};
use smol_str::SmolStr;

use crate::data::{
    BatchRoom, BatchStatus, BriefRoomFact, BriefSnapshot, BuildingBrief, BuildingBriefPayload,
    DraftPayload, DraftStatus, ExitDraft, ExitFact, GenKind, GenOutcome, GenSnapshot, NS,
    NeighborDesc, ReviewCursor, ReviewTarget, RoomFacts, RoomState, StoredDraft, WorldgenReq,
    WorldgenState, building_key, draft_key,
};
use crate::generate::map_brief_concepts;
use mush_atlas::theme::resolve_theme;

/// The BFS room cap for a `@gen/building` walk (design 03 §5.3: N ≤ 40).
pub const MAX_BUILDING_ROOMS: usize = 40;

/// The smallest useful `@gen/building` walk: an entrance plus one other room.
pub const MIN_BUILDING_ROOMS: usize = 2;

/// When the entrance is unzoned, "interior" is bounded by hop count from the
/// entrance instead of zone membership (design 03 §5.3: "same zone, or bounded
/// hop count").
pub const MAX_UNZONED_HOPS: usize = 6;

/// Up to K neighbors' accepted descs seed the prompt (design 03 §5.2).
const MAX_NEIGHBORS: usize = 4;

/// The staff-only LLM cost dashboard's panel feed / query handler name
/// (M10c). Owned by worldgen alongside the rest of the `@llm/*` staff
/// surface, even though it isn't worldgen-specific — see the `@llm` command
/// registration just below for the same convention.
const DASHBOARD_FEED: &str = "llm.dashboard";

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
                "building".into(),
                "review".into(),
                "all".into(),
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
                   \n\
                   Whole building (rooms/exits must already be dug):\n\
                   @gen/building <entrance>=<prose> -- generate a coherent whole building: \
                   a reviewed brief first, then a room draft per room.\n\
                   @gen/review [<building>] -- list a building's drafts + status.\n\
                   @gen/review next -- step to the next item (brief, then each room) and \
                   review it with the same accept/edit/retry/discard loop.\n\
                   @gen/accept/all -- accept every drafted room in the building at once.\n\
                   Requires the BUILDER_LLM power."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_gen),
    )?;
    reg.command(
        CommandSpec {
            name: "@llm".into(),
            shape: ArgShape::Raw,
            switches: vec![
                "log".into(),
                "budget".into(),
                "dashboard".into(),
                "disable".into(),
                "enable".into(),
                "kill".into(),
                "revive".into(),
            ],
            min_role: Role::Wizard,
            help: "@llm/budget -- today's LLM token/cost totals.\n\
                   @llm/dashboard -- fuller cost dashboard: budgets vs caps, top \
                   builders/NPCs by spend, and recent calls (the terminal view of the \
                   web LLM panel).\n\
                   @llm/log [<feature>] -- recent LLM calls (optionally one feature route).\n\
                   @llm/disable -- engage the global kill switch (stops all LLM calls).\n\
                   @llm/enable -- lift the global kill switch.\n\
                   @llm/kill <npc> -- park one NPC agent (no wakes, no tokens).\n\
                   @llm/revive <npc> -- wake a killed NPC agent."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(LlmStaffCmd {
            llm: Arc::clone(&llm),
        }),
    )?;
    // The staff-only cost dashboard (M10c): a panel feed + query handler for
    // the web LLM tab, gated Role::Wizard+ in the handler itself (the web
    // client cannot know the viewer's role, so this refusal IS the gate —
    // same contract as `@gen`'s BUILDER_LLM check; see
    // web/src/lib/stores/session.svelte.ts's `worldgenAvailable` doc). The
    // Wizard floor matches the terminal `@llm/*` family (`min_role: Wizard`
    // above) so both surfaces of the dashboard agree — see `docs/help/llm.md`.
    reg.panel_feed(DASHBOARD_FEED)?;
    reg.query_handler(DASHBOARD_FEED, Arc::new(LlmDashboardQuery { llm }))?;
    reg.async_completion(Arc::new(WorldgenCompletion))?;
    // S3: the building agent's player surface.
    crate::build_cmd::register_build(reg)?;
    Ok(())
}

// ----------------------------------------------------------------------
// @gen
// ----------------------------------------------------------------------

fn cmd_gen(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("building") {
        gen_building(ctx)
    } else if ctx.switches.has("review") {
        gen_review(ctx)
    } else if ctx.switches.has("accept") {
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

/// The batch-review cursor's current target for this builder, if any.
fn cursor_target(ctx: &Ctx<'_>) -> Option<(ObjId, ReviewTarget)> {
    ctx.state::<WorldgenState>()
        .and_then(|st| st.review.get(&ctx.actor).copied())
        .map(|c| (c.entrance, c.target))
}

/// The room the review sub-commands act on: a batch cursor's room wins over
/// the builder's current room. `None` means the cursor points at a brief.
fn review_room(ctx: &Ctx<'_>) -> Option<ObjId> {
    match cursor_target(ctx) {
        Some((_, ReviewTarget::Room(r))) => Some(r),
        Some((_, ReviewTarget::Brief)) => None,
        None => here(ctx).ok(),
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
    // The single-room draft is keyed by the builder's CURRENT room (the review
    // subcommands resolve it via `here` when no batch cursor is active), so a
    // draft for a room the builder is not standing in would be unreachable.
    // Enforce the `@gen here=` contract: the target must be the current room.
    // (Whole-building generation is `@gen/building`, which reaches remote rooms
    // through the batch-review cursor instead.)
    if here(ctx).ok() != Some(room) {
        return Err(CmdError::msg(
            "You can only @gen the room you're in. Go to that room first, then @gen here=<prose>.",
        ));
    }
    ensure_generatable_room(ctx.w, ctx.actor, room)?;
    // Starting fresh single-room work ends any batch-review stepping.
    clear_review_cursor(ctx);

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
    let guidance = {
        let g = ctx.args.raw().trim();
        (!g.is_empty()).then(|| g.to_string())
    };
    // A brief cursor: regenerate the whole-building brief.
    if let Some((entrance, ReviewTarget::Brief)) = cursor_target(ctx) {
        return retry_brief(ctx, entrance, guidance);
    }
    let room = review_room(ctx).ok_or_else(|| CmdError::msg("You are nowhere."))?;
    ensure_generatable_room(ctx.w, ctx.actor, room)?;
    // A batch room: regenerate it in-place, seeded with the brief + neighbors.
    if let Some(entrance) = batch_of_room(ctx, room) {
        return retry_batch_room(ctx, entrance, room, guidance);
    }
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
    // A brief cursor: discard the entire batch.
    if let Some((entrance, ReviewTarget::Brief)) = cursor_target(ctx) {
        return discard_batch(ctx, entrance);
    }
    let room = review_room(ctx).ok_or_else(|| CmdError::msg("You are nowhere."))?;
    if draft_of(ctx, room).is_none() {
        return Err(CmdError::msg("No draft here to discard."));
    }
    let building = batch_of_room(ctx, room);
    clear_draft(ctx, room, ctx.actor);
    if let Some(entrance) = building {
        set_batch_room_status(ctx, entrance, room, RoomState::NeedsAttention);
        ctx.out.info(
            ctx.actor,
            format!(
                "Draft for {} discarded (marked needs-attention). @gen/retry regenerates it.",
                room_name(ctx.w, room)
            ),
        );
        advance_and_show(ctx, entrance);
    } else {
        ctx.out.info(ctx.actor, "Draft discarded.");
    }
    Ok(())
}

fn gen_accept(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // `@gen/accept/all`: accept every drafted room in the builder's batch.
    if ctx.switches.has("all") {
        return accept_all(ctx);
    }
    // A brief cursor: accepting the brief launches the room pass.
    if let Some((entrance, ReviewTarget::Brief)) = cursor_target(ctx) {
        return accept_brief(ctx, entrance);
    }
    let room = review_room(ctx).ok_or_else(|| CmdError::msg("You are nowhere."))?;
    if !perm::can_modify(ctx.w, ctx.actor, room) {
        return Err(CmdError::msg("Permission denied."));
    }
    let draft = draft_of(ctx, room).ok_or_else(|| {
        CmdError::msg("No draft here to accept. Start one with @gen here=<prose>.")
    })?;
    let building = draft.building;
    let written = write_accepted_fields(
        ctx,
        room,
        &draft.payload,
        &AcceptFields::parse(ctx.args.raw())?,
    );

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
    if let Some(entrance) = building {
        set_batch_room_status(ctx, entrance, room, RoomState::Accepted);
        advance_and_show(ctx, entrance);
    }
    Ok(())
}

/// Write the requested `@gen/accept` fields through the ordinary attribute
/// path; returns which field groups were written.
fn write_accepted_fields(
    ctx: &mut Ctx<'_>,
    room: ObjId,
    payload: &DraftPayload,
    want: &AcceptFields,
) -> Vec<&'static str> {
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
    written
}

fn gen_edit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if let Some((_, ReviewTarget::Brief)) = cursor_target(ctx) {
        return Err(CmdError::msg(
            "The building brief isn't line-editable; steer it with @gen/retry <guidance>, or \
             @gen/accept to generate the rooms.",
        ));
    }
    let room = review_room(ctx).ok_or_else(|| CmdError::msg("You are nowhere."))?;
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
            GenOutcome::Plan { outcome } => crate::proposal::on_outcome(ctx, outcome),
            GenOutcome::Err {
                room,
                builder,
                message,
                building,
                budget,
                ..
            } => match building {
                Some(entrance) => on_batch_room_err(ctx, entrance, room, builder, message, budget),
                None => {
                    if ctx.w.get(builder).is_some() {
                        ctx.out.error(builder, message);
                    }
                }
            },
            GenOutcome::Ok {
                room,
                builder,
                draft,
                prose,
                prompt_hash,
                model,
                building,
            } => {
                // Re-validate against current state (treaty ruling 3): the
                // room may be gone, or the builder may have lost control.
                if ctx.w.get(room).is_none_or(|o| o.kind != ObjKind::Room) {
                    match building {
                        Some(entrance) => {
                            set_batch_room_status(ctx, entrance, room, RoomState::NeedsAttention);
                            if ctx.w.get(builder).is_some() {
                                ctx.out.error_on_commit(
                                    builder,
                                    format!(
                                        "A building room (#{}) no longer exists; skipped.",
                                        room.0
                                    ),
                                );
                            }
                            defer_next_room(ctx, entrance);
                        }
                        None => {
                            if ctx.w.get(builder).is_some() {
                                ctx.out.error(
                                    builder,
                                    "The room no longer exists; discarding the generated draft.",
                                );
                            }
                        }
                    }
                    return;
                }
                if !perm::can_modify(ctx.w, builder, room) {
                    match building {
                        Some(entrance) => {
                            set_batch_room_status(ctx, entrance, room, RoomState::NeedsAttention);
                            if ctx.w.get(builder).is_some() {
                                ctx.out.error_on_commit(
                                    builder,
                                    format!(
                                        "You no longer control {}; skipped.",
                                        room_name(ctx.w, room)
                                    ),
                                );
                            }
                            defer_next_room(ctx, entrance);
                        }
                        None => {
                            if ctx.w.get(builder).is_some() {
                                ctx.out.error(
                                    builder,
                                    "You no longer control that room; discarding the generated draft.",
                                );
                            }
                        }
                    }
                    return;
                }
                // A cancelled batch: a room completion outran a `@gen/discard`.
                if let Some(entrance) = building
                    && batch_get(ctx, entrance).is_none()
                {
                    tracing::debug!(
                        correlation,
                        "batch room completion for a cancelled batch dropped"
                    );
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
                    building,
                };
                store_draft(ctx, stored.clone());
                emit_draft_panel(ctx, &stored);

                match building {
                    Some(entrance) => on_batch_room_ok(ctx, entrance, &stored),
                    None => {
                        for line in render_draft_lines(ctx.w, &stored) {
                            ctx.out.info(builder, line);
                        }
                        ctx.out.info(
                            builder,
                            "Draft ready. @gen/accept to write it, @gen/edit to tweak, \
                             @gen/retry to regenerate, or @gen/discard.",
                        );
                    }
                }
            }
            GenOutcome::Brief {
                entrance,
                builder,
                brief,
                prose,
                prompt_hash,
                model,
            } => on_brief_ready(ctx, entrance, builder, brief, prose, prompt_hash, model),
            GenOutcome::BriefErr {
                entrance,
                builder,
                message,
                ..
            } => {
                set_batch_status(ctx, entrance, BatchStatus::Failed);
                if ctx.w.get(builder).is_some() {
                    ctx.out.error_on_commit(
                        builder,
                        format!("{message} (@gen/retry regenerates the brief.)"),
                    );
                }
            }
        }
    }
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
            for route in [
                "worldgen_room",
                "worldgen_brief",
                "worldgen_plan",
                "npc_dialogue_major",
            ] {
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
        if ctx.switches.has("dashboard") {
            return llm_dashboard_terminal(&self.llm, ctx);
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

// ----------------------------------------------------------------------
// @llm/dashboard + the "llm.dashboard" query handler (M10c)
// ----------------------------------------------------------------------

/// The `"llm.dashboard"` query handler: the staff-only cost dashboard
/// ([`LlmService::dashboard`]) rendered as a `Panel` keyframe for the web
/// LLM tab. **Staff-only by role check, not by client gating** — the web
/// client can only know the panel feed is advertised, never the viewer's
/// role (see `web/src/lib/stores/session.svelte.ts`'s `worldgenAvailable`
/// doc for the identical contract on `@gen`), so a non-staff query is
/// refused right here. Gated at `Role::Wizard` to match the terminal
/// `@llm/dashboard` (the whole `@llm` family is `min_role: Wizard`) so both
/// surfaces of the same aggregates share one floor (M10c).
struct LlmDashboardQuery {
    llm: Arc<LlmService>,
}

impl QueryHandler for LlmDashboardQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        if !perm::has_role(w, actor, Role::Wizard) {
            return Err(RichText::tag(
                "error",
                "Staff only (the LLM dashboard needs Wizard or better).",
            ));
        }
        let snapshot = self.llm.dashboard();
        Ok(OutputEvent::Panel {
            feed: SmolStr::new(DASHBOARD_FEED),
            kind: PanelFrameKind::Keyframe,
            data: dashboard_json(w, &snapshot),
        })
    }
}

/// `{ "id": "#123", "name": "Alice" }` — the standard [`mush_proto::ObjRef`]
/// wire shape, built by hand here (a dashboard row only carries the bare
/// [`ObjId`]; the query handler is what has `&World` to resolve a name).
fn obj_ref_json(w: &World, id: ObjId) -> serde_json::Value {
    let name = w
        .get(id)
        .map_or_else(|| "(gone)".to_string(), |o| o.name.clone());
    serde_json::json!({ "id": id.to_string(), "name": name })
}

fn spend_row_json(w: &World, row: &SpendRow) -> serde_json::Value {
    serde_json::json!({
        "obj": obj_ref_json(w, row.obj),
        "input_tokens": row.input_tokens,
        "output_tokens": row.output_tokens,
        "cost_usd": row.cost_usd,
    })
}

fn budget_status_json(b: &BudgetStatus) -> serde_json::Value {
    serde_json::json!({
        "scope": b.scope,
        "input_tokens": b.input_tokens,
        "output_tokens": b.output_tokens,
        "cost_usd": b.cost_usd,
        "tokens_cap": b.tokens_cap,
        "cost_cap": b.cost_cap,
        "tokens_remaining": b.tokens_remaining,
        "cost_remaining": b.cost_remaining,
    })
}

fn recent_call_json(w: &World, r: &RecentCall) -> serde_json::Value {
    serde_json::json!({
        "ts": r.ts,
        "feature": r.feature,
        "provider": r.provider,
        "model": r.model,
        "actor": r.actor.map(|id| obj_ref_json(w, id)),
        "npc": r.npc.map(|id| obj_ref_json(w, id)),
        "input_tokens": r.input_tokens,
        "output_tokens": r.output_tokens,
        "cost_usd": r.cost_usd,
        "latency_ms": r.latency_ms,
        "outcome": r.outcome,
    })
}

/// The full `llm.dashboard` panel payload. **Aggregates only** — every field
/// traces back to [`DashboardSnapshot`], which never carries prompt or
/// response text (see its module doc); this function only adds display
/// names for the [`ObjId`]s it already carries.
fn dashboard_json(w: &World, snap: &DashboardSnapshot) -> serde_json::Value {
    serde_json::json!({
        "kill_switch_engaged": snap.kill_switch_engaged,
        "budgets": snap.budgets.iter().map(budget_status_json).collect::<Vec<_>>(),
        "top_actors": snap.top_actors.iter().map(|r| spend_row_json(w, r)).collect::<Vec<_>>(),
        "top_npcs": snap.top_npcs.iter().map(|r| spend_row_json(w, r)).collect::<Vec<_>>(),
        "daily_series": snap.daily_series.iter().map(|d| serde_json::json!({
            "day": d.day,
            "feature": d.feature,
            "provider": d.provider,
            "calls": d.calls,
            "refusals": d.refusals,
            "budget_trips": d.budget_trips,
            "disabled": d.disabled,
            "errors": d.errors,
            "input_tokens": d.input_tokens,
            "output_tokens": d.output_tokens,
            "cost_usd": d.cost_usd,
        })).collect::<Vec<_>>(),
        "recent_calls": snap.recent_calls.iter().map(|r| recent_call_json(w, r)).collect::<Vec<_>>(),
    })
}

/// `@llm/dashboard` — the terminal view of the same aggregate the web LLM
/// panel renders: budgets vs caps, top builders/NPCs by spend, and a
/// recent-calls tail.
fn llm_dashboard_terminal(llm: &LlmService, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let snap = llm.dashboard();
    ctx.out.info(
        ctx.actor,
        format!(
            "LLM dashboard -- kill switch: {}.",
            if snap.kill_switch_engaged {
                "ENGAGED"
            } else {
                "off"
            }
        ),
    );
    ctx.out.info(ctx.actor, "Budgets today (used / cap):");
    for b in &snap.budgets {
        ctx.out
            .info(ctx.actor, format!("  {}", render_budget_row(b)));
    }
    if snap.top_actors.is_empty() {
        ctx.out
            .info(ctx.actor, "No builder LLM spend recorded today.");
    } else {
        ctx.out.info(ctx.actor, "Top builders by spend today:");
        for row in &snap.top_actors {
            ctx.out
                .info(ctx.actor, format!("  {}", render_spend_row(ctx.w, row)));
        }
    }
    if snap.top_npcs.is_empty() {
        ctx.out.info(ctx.actor, "No NPC LLM spend recorded today.");
    } else {
        ctx.out.info(ctx.actor, "Top NPCs by spend today:");
        for row in &snap.top_npcs {
            ctx.out
                .info(ctx.actor, format!("  {}", render_spend_row(ctx.w, row)));
        }
    }
    if snap.recent_calls.is_empty() {
        ctx.out.info(ctx.actor, "No recent calls logged.");
    } else {
        ctx.out.info(ctx.actor, "Recent calls:");
        for r in snap.recent_calls.iter().take(10) {
            ctx.out
                .info(ctx.actor, format!("  {}", render_recent_call(ctx.w, r)));
        }
    }
    Ok(())
}

fn render_budget_row(b: &BudgetStatus) -> String {
    let tokens = b.tokens_cap.map_or_else(
        || format!("{} tok/no cap", b.input_tokens + b.output_tokens),
        |cap| format!("{}/{} tok", b.input_tokens + b.output_tokens, cap),
    );
    let cost = b.cost_cap.map_or_else(
        || format!("${:.4}/no cap", b.cost_usd),
        |cap| format!("${:.4}/${cap:.2}", b.cost_usd),
    );
    format!("{}: {tokens}, {cost}", b.scope)
}

fn render_spend_row(w: &World, row: &SpendRow) -> String {
    format!(
        "{}: {} in + {} out tokens, ${:.4}",
        who(w, row.obj),
        row.input_tokens,
        row.output_tokens,
        row.cost_usd
    )
}

fn render_recent_call(w: &World, r: &RecentCall) -> String {
    let actor = r.actor.map_or_else(|| "-".to_string(), |id| who(w, id));
    let npc = r.npc.map_or_else(|| "-".to_string(), |id| who(w, id));
    format!(
        "{} {} [{}] {}/{}t ${:.4} {}ms actor={actor} npc={npc}",
        r.feature,
        r.outcome,
        r.model,
        r.input_tokens,
        r.output_tokens,
        r.cost_usd.unwrap_or(0.0),
        r.latency_ms
    )
}

/// `"Alice (#5)"`, or just `"#5"` if the object is gone.
fn who(w: &World, id: ObjId) -> String {
    w.get(id)
        .map_or_else(|| id.to_string(), |o| format!("{} ({id})", o.name))
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
        "kind": "room",
        "room": draft.room.to_string(),
        "room_name": room_name(ctx.w, draft.room),
        "builder": draft.builder.to_string(),
        "model": draft.model,
        // For a batch room: which building it belongs to (the entrance id).
        "building": draft.building.map(|b| b.to_string()),
        "current": { "desc": current_desc },
        "proposed": draft.payload,
    });
    panel.keyframe(ctx, data);
}

/// A `worldgen.draft` keyframe describing a building brief (kind `"brief"`),
/// so the web panel can render the reviewable brief the same way it renders a
/// room draft.
fn emit_brief_panel(ctx: &mut Ctx<'_>, batch: &BuildingBrief) {
    let Some(panel) = ctx.state::<WorldgenState>().map(|s| s.panel.clone()) else {
        return;
    };
    let rooms: Vec<serde_json::Value> = batch
        .rooms
        .iter()
        .map(|r| {
            serde_json::json!({
                "room": r.id.to_string(),
                "room_name": r.name,
                "concept": r.concept,
                "status": r.status.label(),
            })
        })
        .collect();
    let data = serde_json::json!({
        "kind": "brief",
        "building": batch.entrance.to_string(),
        "building_name": room_name(ctx.w, batch.entrance),
        "builder": batch.builder.to_string(),
        "model": batch.model,
        "status": batch.status.label(),
        "concept": batch.concept,
        "palette": batch.palette,
        "rooms": rooms,
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

// ----------------------------------------------------------------------
// @gen/building — the whole-building batch pipeline (design 03 §5.3)
// ----------------------------------------------------------------------

/// `@gen/building <entrance>=<prose>` — walk the building's interior room
/// graph and defer the reviewed brief pass. Rooms/exits must already be dug:
/// worldgen DESCRIBES, it never digs topology.
fn gen_building(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().to_string();
    let (target, prose) = raw.split_once('=').ok_or_else(|| {
        CmdError::msg("Usage: @gen/building <entrance-room>=<prose describing the building>.")
    })?;
    let prose = prose.trim();
    if prose.is_empty() {
        return Err(CmdError::msg(
            "Describe the building in a sentence or two after the =.",
        ));
    }
    let entrance = match_object(ctx.w, ctx.actor, target.trim())?;
    ensure_generatable_room(ctx.w, ctx.actor, entrance)?;

    // Walk the interior graph (BFS, bounded by the `[worldgen].batch_max_rooms`
    // config knob, held in state). A room with no interior exits is not a
    // "building" — describe it with @gen here= instead.
    let max_rooms = ctx
        .state::<WorldgenState>()
        .map_or(MAX_BUILDING_ROOMS, |st| st.batch_max_rooms);
    let rooms = walk_building(ctx.w, entrance, max_rooms);
    if rooms.len() < 2 {
        return Err(CmdError::msg(
            "That room has no interior exits to other rooms; describe a single room with \
             @gen here=<prose> instead.",
        ));
    }
    let capped = rooms.len() >= max_rooms;

    let batch_rooms: Vec<BatchRoom> = rooms
        .iter()
        .map(|&id| BatchRoom {
            id,
            name: room_name(ctx.w, id),
            concept: String::new(),
            status: RoomState::Pending,
        })
        .collect();
    let brief_facts = brief_room_facts(ctx.w, &rooms);
    let n = rooms.len();
    let batch = BuildingBrief {
        entrance,
        builder: ctx.actor,
        prose: prose.to_string(),
        created: ctx.w.now(),
        status: BatchStatus::BriefPending,
        concept: String::new(),
        palette: String::new(),
        rooms: batch_rooms,
        prompt_hash: String::new(),
        model: String::new(),
    };
    persist_batch(ctx, batch);
    // The review cursor targets this building's brief so `@gen/accept` works
    // as soon as "brief ready" arrives.
    set_review_cursor(ctx, entrance, ReviewTarget::Brief);

    let snap = BriefSnapshot {
        entrance,
        builder: ctx.actor,
        theme: resolve_theme(ctx.w, entrance),
        prose: prose.to_string(),
        rooms: brief_facts,
        guidance: None,
        prior_brief: None,
    };
    defer_brief(ctx, &snap)?;
    let cap_note = if capped {
        format!(" (capped at {max_rooms})")
    } else {
        String::new()
    };
    ctx.out.info(
        ctx.actor,
        format!(
            "Walking {}: found {n} interior rooms{cap_note}. Generating a building brief \
             (reviewed before any room is written)...",
            room_name(ctx.w, entrance)
        ),
    );
    // An unzoned entrance has no zone boundary to stop the interior walk, so it
    // falls back to a hop-count radius and can sweep in streets or neighbouring
    // buildings within MAX_UNZONED_HOPS. Nothing auto-commits (each accept is
    // permission-gated and reviewed), but warn so a builder does not
    // `@gen/accept/all` descriptions across a wider area than they meant. Zone
    // the building (`@zone <room>=<zone-master>`) to bound the walk exactly.
    if ctx.w.get(entrance).and_then(|o| o.zone).is_none() {
        ctx.out.info(
            ctx.actor,
            "Note: this entrance is unzoned, so the walk used a hop-count radius and may include \
             rooms outside the building. Review each room before accepting (avoid @gen/accept/all \
             here), or @zone the building first to bound the walk.",
        );
    }
    Ok(())
}

/// `@gen/review [<building>|next]` — list a batch's status, or step to the
/// next reviewable item.
fn gen_review(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let arg = ctx.args.raw().trim().to_string();
    if arg.eq_ignore_ascii_case("next") {
        return review_next(ctx);
    }
    let entrance = if arg.is_empty() {
        resolve_review_building(ctx)?
    } else {
        match_object(ctx.w, ctx.actor, &arg)?
    };
    if batch_get(ctx, entrance).is_none() {
        return Err(CmdError::msg(
            "No building batch here. Start one with @gen/building <entrance>=<prose>.",
        ));
    }
    // Resume a batch whose room pass stalled with no call in flight (a reboot
    // lost the in-flight room; `on_world_loaded` downgraded it to `Pending` but
    // cannot re-arm the pipeline at boot). Self-healing on the first review.
    rearm_stalled_batch(ctx, entrance);
    let Some(batch) = batch_get(ctx, entrance) else {
        return Ok(());
    };
    review_list(ctx, &batch);
    // Point the cursor at the first reviewable item (or the brief placeholder)
    // so a bare `@gen/accept` acts on it.
    let first = reviewable_after(&batch, None).unwrap_or(ReviewTarget::Brief);
    set_review_cursor(ctx, entrance, first);
    Ok(())
}

/// `@gen/review next` — advance the cursor to the next reviewable item.
fn review_next(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (entrance, current) = match cursor_target(ctx) {
        Some((e, t)) => (e, Some(t)),
        None => (resolve_review_building(ctx)?, None),
    };
    let Some(batch) = batch_get(ctx, entrance) else {
        clear_review_cursor(ctx);
        return Err(CmdError::msg("That building batch is gone."));
    };
    match reviewable_after(&batch, current) {
        Some(next) => {
            set_review_cursor(ctx, entrance, next);
            show_review_item(ctx, &batch, next);
        }
        None => {
            clear_review_cursor(ctx);
            let pending = batch
                .rooms
                .iter()
                .filter(|r| r.status == RoomState::Pending || r.status == RoomState::Generating)
                .count();
            ctx.out.info(
                ctx.actor,
                format!(
                    "Nothing left to review for '{}' ({} drafted, {} still generating).",
                    room_name(ctx.w, entrance),
                    batch.drafted_count(),
                    pending
                ),
            );
        }
    }
    Ok(())
}

/// The batch to review when none is named: the one containing the current
/// room, else the builder's single batch.
fn resolve_review_building(ctx: &Ctx<'_>) -> Result<ObjId, CmdError> {
    if let Ok(h) = here(ctx)
        && let Some(e) = batch_of_room(ctx, h)
    {
        return Ok(e);
    }
    let st = ctx
        .state::<WorldgenState>()
        .ok_or_else(|| CmdError::msg("Worldgen state unavailable."))?;
    let mut it = st.batches_of(ctx.actor);
    match (it.next(), it.next()) {
        (Some(b), None) => Ok(b.entrance),
        (None, _) => Err(CmdError::msg(
            "You have no building batches. Start one with @gen/building <entrance>=<prose>.",
        )),
        (Some(_), Some(_)) => Err(CmdError::msg(
            "You have several building batches; name one: @gen/review <entrance>.",
        )),
    }
}

/// Render a batch's status + per-room lines.
fn review_list(ctx: &mut Ctx<'_>, batch: &BuildingBrief) {
    ctx.out.info(
        ctx.actor,
        format!(
            "--- Building '{}' [{}] -- {}/{} rooms drafted ---",
            room_name(ctx.w, batch.entrance),
            batch.status.label(),
            batch.drafted_count(),
            batch.rooms.len()
        ),
    );
    if !batch.concept.trim().is_empty() {
        ctx.out
            .info(ctx.actor, format!("Concept: {}", batch.concept));
    }
    if !batch.palette.trim().is_empty() {
        ctx.out
            .info(ctx.actor, format!("Palette: {}", batch.palette));
    }
    for (i, r) in batch.rooms.iter().enumerate() {
        let concept = if r.concept.trim().is_empty() {
            String::new()
        } else {
            format!(" -- {}", r.concept)
        };
        ctx.out.info(
            ctx.actor,
            format!("  {}. [{}] {}{concept}", i + 1, r.status.label(), r.name),
        );
    }
    ctx.out.info(
        ctx.actor,
        "Use @gen/review next to step through drafts (accept/edit/retry/discard), or \
         @gen/accept/all to accept every drafted room.",
    );
}

/// Show one review item (the brief or a room draft) and its prompt.
fn show_review_item(ctx: &mut Ctx<'_>, batch: &BuildingBrief, target: ReviewTarget) {
    match target {
        ReviewTarget::Brief => {
            ctx.out.info(
                ctx.actor,
                format!(
                    "--- Building brief for '{}' (model: {}) ---",
                    room_name(ctx.w, batch.entrance),
                    batch.model
                ),
            );
            if !batch.concept.trim().is_empty() {
                ctx.out
                    .info(ctx.actor, format!("Concept: {}", batch.concept));
            }
            ctx.out
                .info(ctx.actor, format!("Palette: {}", batch.palette));
            for (i, r) in batch.rooms.iter().enumerate() {
                ctx.out.info(
                    ctx.actor,
                    format!("  {}. {} -- {}", i + 1, r.name, r.concept),
                );
            }
            ctx.out.info(
                ctx.actor,
                "@gen/accept generates the rooms, @gen/retry <guidance> re-briefs, \
                 @gen/discard cancels the building.",
            );
        }
        ReviewTarget::Room(room) => {
            let status = batch
                .room(room)
                .map(|r| r.status.label())
                .unwrap_or("unknown");
            if let Some(draft) = draft_of(ctx, room) {
                for line in render_draft_lines(ctx.w, &draft) {
                    ctx.out.info(ctx.actor, line);
                }
                ctx.out.info(
                    ctx.actor,
                    "@gen/accept writes it, @gen/edit tweaks, @gen/retry regenerates, \
                     @gen/discard drops it. @gen/review next for the next.",
                );
            } else {
                ctx.out.info(
                    ctx.actor,
                    format!(
                        "{} [{status}] has no draft. @gen/retry regenerates it.",
                        room_name(ctx.w, room)
                    ),
                );
            }
        }
    }
}

/// `@gen/accept` on the brief cursor: launch the room pass.
fn accept_brief(ctx: &mut Ctx<'_>, entrance: ObjId) -> Result<(), CmdError> {
    let Some(mut batch) = batch_get(ctx, entrance) else {
        return Err(CmdError::msg("That building batch is gone."));
    };
    match batch.status {
        BatchStatus::BriefReady => {}
        BatchStatus::BriefPending => {
            return Err(CmdError::msg(
                "The brief is still generating; wait for 'brief ready'.",
            ));
        }
        BatchStatus::Failed => {
            return Err(CmdError::msg(
                "The brief failed; @gen/retry regenerates it before you can generate rooms.",
            ));
        }
        BatchStatus::Generating | BatchStatus::Complete => {
            return Err(CmdError::msg(
                "This building's rooms are already generating. Use @gen/review next.",
            ));
        }
    }
    batch.status = BatchStatus::Generating;
    let n = batch.rooms.len();
    persist_batch(ctx, batch);
    // Stepping the brief is done; rooms enter review as they draft.
    clear_review_cursor(ctx);
    ctx.out.info(
        ctx.actor,
        format!(
            "Brief accepted. Generating {n} room drafts at background priority; drafts arrive \
             as they complete. Review with @gen/review."
        ),
    );
    // Kick off the completion-pipelined room pass at the first pending room.
    defer_next_room(ctx, entrance);
    Ok(())
}

/// `@gen/retry` on the brief cursor: re-walk and regenerate the brief.
fn retry_brief(
    ctx: &mut Ctx<'_>,
    entrance: ObjId,
    guidance: Option<String>,
) -> Result<(), CmdError> {
    let Some(mut batch) = batch_get(ctx, entrance) else {
        return Err(CmdError::msg("That building batch is gone."));
    };
    ensure_generatable_room(ctx.w, ctx.actor, entrance)?;
    let max_rooms = ctx
        .state::<WorldgenState>()
        .map_or(MAX_BUILDING_ROOMS, |st| st.batch_max_rooms);
    let rooms = walk_building(ctx.w, entrance, max_rooms);
    if rooms.len() < 2 {
        return Err(CmdError::msg(
            "That room no longer has interior exits to other rooms.",
        ));
    }
    // A previously-produced brief to avoid, reconstructed from the batch.
    let prior = (!batch.palette.trim().is_empty() || !batch.concept.trim().is_empty()).then(|| {
        BuildingBriefPayload {
            concept: batch.concept.clone(),
            palette: batch.palette.clone(),
            rooms: Vec::new(),
        }
    });
    // Refresh the room list (topology may have changed) while keeping any
    // already-accepted rooms' status.
    let brief_facts = brief_room_facts(ctx.w, &rooms);
    let mut new_rooms: Vec<BatchRoom> = Vec::with_capacity(rooms.len());
    for &id in &rooms {
        let (concept, status) = batch
            .room(id)
            .map(|r| (r.concept.clone(), r.status))
            .unwrap_or_default();
        new_rooms.push(BatchRoom {
            id,
            name: room_name(ctx.w, id),
            concept,
            status,
        });
    }
    batch.rooms = new_rooms;
    batch.status = BatchStatus::BriefPending;
    let prose = batch.prose.clone();
    persist_batch(ctx, batch);
    set_review_cursor(ctx, entrance, ReviewTarget::Brief);
    let snap = BriefSnapshot {
        entrance,
        builder: ctx.actor,
        theme: resolve_theme(ctx.w, entrance),
        prose,
        rooms: brief_facts,
        guidance,
        prior_brief: prior,
    };
    defer_brief(ctx, &snap)?;
    ctx.out
        .info(ctx.actor, "Regenerating the building brief...");
    Ok(())
}

/// `@gen/discard` on the brief cursor: cancel the whole batch (removes the
/// batch record + every room DRAFT; already-accepted room descs are kept).
fn discard_batch(ctx: &mut Ctx<'_>, entrance: ObjId) -> Result<(), CmdError> {
    let Some(batch) = batch_get(ctx, entrance) else {
        return Err(CmdError::msg("That building batch is gone."));
    };
    for r in &batch.rooms {
        clear_draft(ctx, r.id, ctx.actor);
    }
    let actor = ctx.actor;
    if let Some(st) = ctx.state_mut::<WorldgenState>() {
        st.briefs.remove(&(entrance, actor));
    }
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::Del {
            key: building_key(entrance, actor),
        },
    });
    clear_review_cursor(ctx);
    ctx.out.info(
        ctx.actor,
        format!(
            "Building batch for {} discarded (accepted room descriptions kept).",
            room_name(ctx.w, entrance)
        ),
    );
    Ok(())
}

/// `@gen/accept/all` — accept every drafted room in the builder's batch.
fn accept_all(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let entrance = match cursor_target(ctx) {
        Some((e, _)) => e,
        None => resolve_review_building(ctx)?,
    };
    let Some(batch) = batch_get(ctx, entrance) else {
        return Err(CmdError::msg("That building batch is gone."));
    };
    // The nag (design 03 §5.3: "@gen/accept/all exists but nags").
    ctx.out.info(
        ctx.actor,
        "Accepting every drafted room without individual review -- consider @gen/review next \
         to check each first.",
    );
    let rooms: Vec<ObjId> = batch
        .rooms
        .iter()
        .filter(|r| r.status == RoomState::Drafted)
        .map(|r| r.id)
        .collect();
    if rooms.is_empty() {
        return Err(CmdError::msg(
            "No drafted rooms to accept yet (generate the rooms first, or wait for drafts).",
        ));
    }
    let want = AcceptFields::all();
    let mut accepted = 0;
    for room in rooms {
        if !perm::can_modify(ctx.w, ctx.actor, room) {
            continue;
        }
        let Some(draft) = draft_of(ctx, room) else {
            continue;
        };
        write_accepted_fields(ctx, room, &draft.payload, &want);
        clear_draft(ctx, room, ctx.actor);
        set_batch_room_status(ctx, entrance, room, RoomState::Accepted);
        accepted += 1;
    }
    clear_review_cursor(ctx);
    ctx.out.info(
        ctx.actor,
        format!("Accepted {accepted} room drafts (written through the normal attribute path)."),
    );
    Ok(())
}

/// `@gen/retry` on a batch room: regenerate it (seeded with the brief + its
/// already-drafted neighbors), keeping the prior draft as "avoid this".
fn retry_batch_room(
    ctx: &mut Ctx<'_>,
    entrance: ObjId,
    room: ObjId,
    guidance: Option<String>,
) -> Result<(), CmdError> {
    if batch_get(ctx, entrance).is_none() {
        return Err(CmdError::msg("That building batch is gone."));
    }
    let prior = draft_of(ctx, room).map(|d| d.payload);
    if !defer_batch_room(ctx, entrance, room, guidance, prior) {
        return Err(CmdError::msg("Could not regenerate that room."));
    }
    ctx.out.info(
        ctx.actor,
        format!("Regenerating {}...", room_name(ctx.w, room)),
    );
    Ok(())
}

/// A batch room draft arrived: mark it drafted, stream progress, and pipeline
/// the next room.
fn on_batch_room_ok(ctx: &mut Ctx<'_>, entrance: ObjId, stored: &StoredDraft) {
    set_batch_room_status(ctx, entrance, stored.room, RoomState::Drafted);
    if let Some(batch) = batch_get(ctx, entrance) {
        ctx.out.info(
            stored.builder,
            format!(
                "Drafted {} ({}/{} rooms).",
                room_name(ctx.w, stored.room),
                batch.drafted_count(),
                batch.rooms.len()
            ),
        );
    }
    defer_next_room(ctx, entrance);
}

/// A batch room failed: mark it needs-attention. A budget trip degrades the
/// remaining rooms and stops; other failures continue the pipeline.
fn on_batch_room_err(
    ctx: &mut Ctx<'_>,
    entrance: ObjId,
    room: ObjId,
    builder: ObjId,
    message: String,
    budget: bool,
) {
    if batch_get(ctx, entrance).is_none() {
        // The batch was cancelled while this room was in flight.
        return;
    }
    set_batch_room_status(ctx, entrance, room, RoomState::NeedsAttention);
    if budget {
        degrade_remaining(ctx, entrance);
        if ctx.w.get(builder).is_some() {
            ctx.out.error_on_commit(
                builder,
                format!(
                    "Budget reached mid-building: {} and the remaining rooms are marked \
                     needs-attention (nothing was written). Raise the budget, then \
                     @gen/review to retry them.",
                    room_name(ctx.w, room)
                ),
            );
        }
    } else {
        if ctx.w.get(builder).is_some() {
            ctx.out.error_on_commit(
                builder,
                format!("{} needs attention: {message}", room_name(ctx.w, room)),
            );
        }
        defer_next_room(ctx, entrance);
    }
}

/// Defer the next `Pending` room in BFS order, or finish the batch. A no-op
/// unless the batch is actively `Generating`: a manual `@gen/retry` during
/// review (batch already `Complete`) replaces one draft without restarting
/// the pass.
fn defer_next_room(ctx: &mut Ctx<'_>, entrance: ObjId) {
    loop {
        let Some(batch) = batch_get(ctx, entrance) else {
            return;
        };
        if batch.status != BatchStatus::Generating {
            return;
        }
        let Some(room) = batch.next_pending() else {
            finish_batch(ctx, entrance);
            return;
        };
        if defer_batch_room(ctx, entrance, room, None, None) {
            return; // deferred; its completion re-drives the pipeline
        }
        // `defer_batch_room` could not defer (an unrealistic serialize failure
        // on owned plain data, or a vanished state slot). Don't wedge the batch
        // on a room that never leaves `Pending`: mark it needs-attention so
        // `next_pending` advances, then try the next. If we can't even mark it
        // (state/batch gone), stop rather than spin.
        set_batch_room_status(ctx, entrance, room, RoomState::NeedsAttention);
        let advanced = batch_get(ctx, entrance).and_then(|b| b.room(room).map(|r| r.status))
            == Some(RoomState::NeedsAttention);
        if !advanced {
            return;
        }
    }
}

/// Resume a batch whose room pass stalled with no off-loop call in flight —
/// e.g. a reboot during generation, which
/// [`on_world_loaded`](mush_core::GameModule::on_world_loaded) downgraded from
/// `Generating` back to `Pending` but could not re-arm (the driver drops
/// boot-time deferred effects). A no-op unless the batch is actively
/// `Generating`, has a pending room, and nothing is currently `Generating`
/// (the in-flight marker). Called from `@gen/review`, the natural "what is
/// happening with my batch" command, so recovery is self-healing.
fn rearm_stalled_batch(ctx: &mut Ctx<'_>, entrance: ObjId) {
    let Some(batch) = batch_get(ctx, entrance) else {
        return;
    };
    if batch.status != BatchStatus::Generating {
        return;
    }
    let stalled = batch.next_pending().is_some()
        && !batch
            .rooms
            .iter()
            .any(|r| r.status == RoomState::Generating);
    if stalled {
        defer_next_room(ctx, entrance);
    }
}

/// Build one batch room's snapshot on-loop (theme + concept + already-drafted
/// neighbors), mark it `Generating`, and defer its off-loop room call. Returns
/// whether the effect was deferred.
fn defer_batch_room(
    ctx: &mut Ctx<'_>,
    entrance: ObjId,
    room: ObjId,
    guidance: Option<String>,
    prior_draft: Option<DraftPayload>,
) -> bool {
    let Some(batch) = batch_get(ctx, entrance) else {
        return false;
    };
    let concept = batch
        .room(room)
        .map(|r| r.concept.clone())
        .unwrap_or_default();
    let brief_ctx = batch.context();
    let builder = ctx.actor;
    let snap = {
        let Some(st) = ctx.state::<WorldgenState>() else {
            return false;
        };
        let theme = resolve_theme(ctx.w, room);
        let facts = room_facts(ctx.w, room);
        let neighbors = batch_neighbor_descs(ctx.w, st, room, builder);
        GenSnapshot {
            kind: GenKind::Room,
            room,
            builder,
            theme,
            facts,
            neighbors,
            prose: concept,
            guidance,
            prior_draft,
        }
    };
    let req = WorldgenReq::BuildingRoom {
        snap,
        building: entrance,
        brief: brief_ctx,
    };
    let payload = match serde_json::to_string(&req) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "worldgen: could not serialize batch room request");
            return false;
        }
    };
    set_batch_room_status(ctx, entrance, room, RoomState::Generating);
    let correlation = ctx.w.next_correlation_id();
    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
        module: SmolStr::new(NS),
        actor: builder,
        correlation,
        payload,
    }));
    true
}

/// The brief pass completed: fold it into the batch, preview it, and open it
/// for review.
fn on_brief_ready(
    ctx: &mut Ctx<'_>,
    entrance: ObjId,
    builder: ObjId,
    brief: BuildingBriefPayload,
    _prose: String,
    prompt_hash: String,
    model: String,
) {
    let Some(mut batch) = batch_get(ctx, entrance) else {
        // The builder discarded the batch while the brief ran.
        return;
    };
    // Map the brief's per-room concepts back onto the batch's ordered rooms.
    let facts: Vec<BriefRoomFact> = batch
        .rooms
        .iter()
        .map(|r| BriefRoomFact {
            id: r.id,
            name: r.name.clone(),
            exits: Vec::new(),
        })
        .collect();
    let concepts = map_brief_concepts(&facts, &brief);
    for (r, concept) in batch.rooms.iter_mut().zip(concepts) {
        r.concept = concept;
    }
    batch.concept = brief.concept.trim().to_string();
    batch.palette = brief.palette.trim().to_string();
    batch.prompt_hash = prompt_hash;
    batch.model = model;
    batch.status = BatchStatus::BriefReady;
    let n = batch.rooms.len();
    let batch_snapshot = batch.clone();
    persist_batch(ctx, batch);
    // Re-point the cursor at the brief (accept generates the rooms).
    set_review_cursor(ctx, entrance, ReviewTarget::Brief);
    emit_brief_panel(ctx, &batch_snapshot);
    if ctx.w.get(builder).is_some() {
        ctx.out.info(
            builder,
            format!(
                "Brief ready for '{}' ({n} rooms). Review it: @gen/review #{}. \
                 @gen/accept generates the rooms; @gen/retry <guidance> re-briefs.",
                room_name(ctx.w, entrance),
                entrance.0
            ),
        );
    }
}

/// Mark the batch complete and report the tally.
fn finish_batch(ctx: &mut Ctx<'_>, entrance: ObjId) {
    set_batch_status(ctx, entrance, BatchStatus::Complete);
    if let Some(batch) = batch_get(ctx, entrance) {
        let drafted = batch.drafted_count();
        let needs = batch
            .rooms
            .iter()
            .filter(|r| r.status == RoomState::NeedsAttention)
            .count();
        if ctx.w.get(batch.builder).is_some() {
            ctx.out.info(
                batch.builder,
                format!(
                    "Building '{}' complete: {drafted} rooms drafted, {needs} need attention. \
                     Review with @gen/review #{}.",
                    room_name(ctx.w, entrance),
                    entrance.0
                ),
            );
        }
    }
}

/// Mark every not-yet-drafted room needs-attention and complete the batch
/// (a budget trip mid-batch — never a partial auto-commit).
fn degrade_remaining(ctx: &mut Ctx<'_>, entrance: ObjId) {
    let Some(mut batch) = batch_get(ctx, entrance) else {
        return;
    };
    for r in &mut batch.rooms {
        if matches!(r.status, RoomState::Pending | RoomState::Generating) {
            r.status = RoomState::NeedsAttention;
        }
    }
    batch.status = BatchStatus::Complete;
    persist_batch(ctx, batch);
}

// -- batch graph walk + neighbor seeding --------------------------------

/// BFS from `entrance` over interior exits, bounded at `max_rooms` (the
/// `[worldgen].batch_max_rooms` config knob, default
/// [`MAX_BUILDING_ROOMS`]). "Interior" = a linked exit whose destination room
/// is in the entrance's zone; for an unzoned entrance, within
/// [`MAX_UNZONED_HOPS`] hops. The entrance is always first (BFS order).
fn walk_building(w: &World, entrance: ObjId, max_rooms: usize) -> Vec<ObjId> {
    use std::collections::{HashSet, VecDeque};
    let zone = w.get(entrance).and_then(|o| o.zone);
    let mut visited: HashSet<ObjId> = HashSet::new();
    let mut order: Vec<ObjId> = Vec::new();
    let mut queue: VecDeque<(ObjId, usize)> = VecDeque::new();
    visited.insert(entrance);
    order.push(entrance);
    queue.push_back((entrance, 0));
    while let Some((cur, hop)) = queue.pop_front() {
        if order.len() >= max_rooms {
            break;
        }
        for &e in w.exits_of(cur) {
            let Some(exit) = w.get(e) else { continue };
            let Some(dest) = exit.destination else {
                continue;
            };
            if visited.contains(&dest) {
                continue;
            }
            let Some(dobj) = w.get(dest) else { continue };
            if dobj.kind != ObjKind::Room {
                continue;
            }
            let interior = match zone {
                Some(z) => dobj.zone == Some(z),
                None => hop < MAX_UNZONED_HOPS,
            };
            if !interior {
                continue;
            }
            visited.insert(dest);
            order.push(dest);
            if order.len() >= max_rooms {
                break;
            }
            queue.push_back((dest, hop + 1));
        }
    }
    order
}

/// Each room's interior exits as `"dir -> DestName"` descriptors (only exits
/// whose destination is another room of this building — never invented).
fn brief_room_facts(w: &World, rooms: &[ObjId]) -> Vec<BriefRoomFact> {
    use std::collections::HashSet;
    let set: HashSet<ObjId> = rooms.iter().copied().collect();
    rooms
        .iter()
        .map(|&id| {
            let mut exits = Vec::new();
            for &e in w.exits_of(id) {
                let Some(o) = w.get(e) else { continue };
                let Some(dest) = o.destination else { continue };
                if !set.contains(&dest) {
                    continue;
                }
                let dir = o.name.split(';').next().unwrap_or("").trim();
                let dir = if dir.is_empty() { "exit" } else { dir };
                exits.push(format!("{dir} -> {}", room_name(w, dest)));
            }
            BriefRoomFact {
                id,
                name: room_name(w, id),
                exits,
            }
        })
        .collect()
}

/// Neighbor descriptions for a batch room: prefer a sibling's *drafted* desc
/// (not yet committed) so later rooms cohere with earlier drafts; fall back to
/// a committed desc.
fn batch_neighbor_descs(
    w: &World,
    st: &WorldgenState,
    room: ObjId,
    builder: ObjId,
) -> Vec<NeighborDesc> {
    let mut out = Vec::new();
    for &e in w.exits_of(room) {
        if out.len() >= MAX_NEIGHBORS {
            break;
        }
        let Some(exit) = w.get(e) else { continue };
        let Some(dest) = exit.destination else {
            continue;
        };
        let desc = st
            .draft(dest, builder)
            .map(|d| d.payload.desc.clone())
            .filter(|d| !d.trim().is_empty())
            .or_else(|| {
                w.attr(dest, "desc")
                    .and_then(|a| a.as_text())
                    .map(str::to_string)
            })
            .filter(|d| !d.trim().is_empty());
        let Some(desc) = desc else { continue };
        let direction = exit.name.split(';').next().unwrap_or("").trim().to_string();
        out.push(NeighborDesc {
            direction,
            name: room_name(w, dest),
            desc,
        });
    }
    out
}

// -- batch store/cache + cursor helpers ---------------------------------

/// Serialize a snapshot and defer the brief pass off-loop.
fn defer_brief(ctx: &mut Ctx<'_>, snap: &BriefSnapshot) -> Result<(), CmdError> {
    let req = WorldgenReq::Brief(snap.clone());
    let payload = serde_json::to_string(&req)
        .map_err(|e| CmdError::msg(format!("Could not prepare the brief request: {e}")))?;
    let correlation = ctx.w.next_correlation_id();
    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
        module: SmolStr::new(NS),
        actor: ctx.actor,
        correlation,
        payload,
    }));
    Ok(())
}

/// The builder's batch record for `entrance`, cloned out of the cache.
fn batch_get(ctx: &Ctx<'_>, entrance: ObjId) -> Option<BuildingBrief> {
    ctx.state::<WorldgenState>()?
        .brief(entrance, ctx.actor)
        .cloned()
}

/// The batch (entrance id) that owns `room` for this builder, if any.
fn batch_of_room(ctx: &Ctx<'_>, room: ObjId) -> Option<ObjId> {
    ctx.state::<WorldgenState>()?
        .batch_containing(ctx.actor, room)
        .map(|b| b.entrance)
}

/// Write a batch record through the store and update the cache.
fn persist_batch(ctx: &mut Ctx<'_>, batch: BuildingBrief) {
    match serde_json::to_value(&batch) {
        Ok(value) => ctx.persist(ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::JsonSet {
                key: building_key(batch.entrance, batch.builder),
                value,
            },
        }),
        Err(e) => {
            tracing::error!(error = %e, "worldgen: could not serialize batch");
            return;
        }
    }
    if let Some(st) = ctx.state_mut::<WorldgenState>() {
        st.briefs.insert((batch.entrance, batch.builder), batch);
    }
}

/// Set one batch room's status and persist.
fn set_batch_room_status(ctx: &mut Ctx<'_>, entrance: ObjId, room: ObjId, status: RoomState) {
    let Some(mut batch) = batch_get(ctx, entrance) else {
        return;
    };
    if let Some(r) = batch.room_mut(room) {
        r.status = status;
    }
    persist_batch(ctx, batch);
}

/// Set a batch's lifecycle status and persist.
fn set_batch_status(ctx: &mut Ctx<'_>, entrance: ObjId, status: BatchStatus) {
    let Some(mut batch) = batch_get(ctx, entrance) else {
        return;
    };
    batch.status = status;
    persist_batch(ctx, batch);
}

fn set_review_cursor(ctx: &mut Ctx<'_>, entrance: ObjId, target: ReviewTarget) {
    let actor = ctx.actor;
    if let Some(st) = ctx.state_mut::<WorldgenState>() {
        st.review.insert(actor, ReviewCursor { entrance, target });
    }
}

fn clear_review_cursor(ctx: &mut Ctx<'_>) {
    let actor = ctx.actor;
    if let Some(st) = ctx.state_mut::<WorldgenState>() {
        st.review.remove(&actor);
    }
}

/// Advance the review cursor to the next reviewable item and show it, or clear
/// it and report the batch tally (used after accept/discard of a batch item).
fn advance_and_show(ctx: &mut Ctx<'_>, entrance: ObjId) {
    let current = cursor_target(ctx).map(|(_, t)| t);
    let Some(batch) = batch_get(ctx, entrance) else {
        clear_review_cursor(ctx);
        return;
    };
    match reviewable_after(&batch, current) {
        Some(next) => {
            set_review_cursor(ctx, entrance, next);
            show_review_item(ctx, &batch, next);
        }
        None => {
            clear_review_cursor(ctx);
            let pending = batch
                .rooms
                .iter()
                .filter(|r| r.status == RoomState::Pending || r.status == RoomState::Generating)
                .count();
            ctx.out.info(
                ctx.actor,
                format!(
                    "That was the last draft to review for '{}' ({} drafted, {pending} still \
                     generating).",
                    room_name(ctx.w, entrance),
                    batch.drafted_count()
                ),
            );
        }
    }
}

/// The stable ordering key of a review target within a batch (brief first,
/// then rooms in BFS order).
fn target_key(batch: &BuildingBrief, t: ReviewTarget) -> i64 {
    match t {
        ReviewTarget::Brief => -1,
        ReviewTarget::Room(r) => batch
            .rooms
            .iter()
            .position(|x| x.id == r)
            .map_or(i64::MAX, |i| i as i64),
    }
}

/// The next reviewable target strictly after `current` (brief while
/// `BriefReady`, then rooms whose draft awaits review).
fn reviewable_after(batch: &BuildingBrief, current: Option<ReviewTarget>) -> Option<ReviewTarget> {
    let cur_key = current.map_or(-2, |t| target_key(batch, t));
    let mut cands: Vec<(i64, ReviewTarget)> = Vec::new();
    if batch.status == BatchStatus::BriefReady {
        cands.push((-1, ReviewTarget::Brief));
    }
    for (i, r) in batch.rooms.iter().enumerate() {
        if r.status.reviewable() {
            cands.push((i as i64, ReviewTarget::Room(r.id)));
        }
    }
    cands
        .into_iter()
        .filter(|(k, _)| *k > cur_key)
        .min_by_key(|(k, _)| *k)
        .map(|(_, t)| t)
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
