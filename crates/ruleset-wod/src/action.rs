//! `+action <freeform> [using <pool>]` — the generic, splat-agnostic Storyteller
//! adjudication path (M13 task 1).
//!
//! Any character with a sheet may propose a freeform action and (optionally)
//! name the dice pool it should roll. The command snapshots the proposal + the
//! actor's REAL pool (resolved against the sheet) and defers an ST ruling, then
//! returns without rolling — exactly the two-phase spine the Mage `+cast`
//! adjudicator uses, with [`ActionKind::Generic`] selected so the off-loop
//! prompts, the sheet fence, and the mechanical consequences all take the
//! mundane path (a pool from the sheet, no Spheres, no Paradox, no sheet
//! mutation — narrative + silent witnesses only).
//!
//! The player then throws the dice with `+roll/silent`: the roll is
//! ST-authoritative (the ruling's pool at the ruling's difficulty, re-resolved
//! against the sheet so it can never exceed it), the room stays silent, and the
//! ST *interprets* the engine's logged outcome — delivering a tailored line only
//! to observers who make their Perception + Awareness witness roll.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar};

use crate::WodCmd;
use crate::st::{ActionKind, RulingRequest, StRequest};
use crate::state::WodState;

/// The default pool when the player names none: a bare Wits check. Wits is a
/// core attribute present (≥ 1) on every WoD sheet, so it always resolves.
const DEFAULT_POOL: &str = "wits";

/// `+action <what you do> [using <pool>]` — propose a freeform action for the
/// Storyteller to adjudicate. Rolls nothing; a new proposal supersedes any prior
/// pending action for the actor.
fn action_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (freeform, pool_expr) = split_using(ctx.args.raw());
    if freeform.trim().is_empty() {
        return Err(CmdError::msg(
            "Describe the action: +action <what you do> [using <pool>].",
        ));
    }
    let doc = cmd
        .read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet to act with."))?;
    let schema = cmd.schema_for_doc(&doc);

    // Resolve the proposed pool against the sheet (the default if none named).
    // resolve_sheet_pool can never sum to more than the sheet holds — the fence
    // is enforced again at propose time so a bad pool fails fast, not silently.
    let pool = if pool_expr.trim().is_empty() {
        DEFAULT_POOL.to_string()
    } else {
        pool_expr.trim().to_string()
    };
    let resolved_pool = crate::dice::resolve_sheet_pool(&schema, &doc, &pool)
        .map_err(|e| CmdError::msg(format!("That pool does not resolve against your sheet: {e}.")))?
        .total;

    let actor = ctx.actor;
    let correlation = ctx.w.next_correlation_id();
    // A new proposal replaces any prior pending action and becomes the actor's
    // latest proposal, so an out-of-order older ruling cannot clobber it (A4).
    if let Some(st) = ctx.state_mut::<WodState>() {
        st.clear_pending(actor);
        st.note_proposal(actor, correlation);
    }
    // A superseded pending was just cleared; write the pending doc through so it
    // does not resurrect on a hot restart (M17). `latest_proposal` is not
    // persisted (it is re-derivable transient bookkeeping).
    if let Some(op) = ctx.state::<WodState>().map(WodState::pending_persist_op) {
        ctx.persist(op);
    }
    let req = StRequest::Ruling(RulingRequest {
        actor,
        freeform: freeform.trim().to_string(),
        proposed_spheres: Vec::new(),
        proposed_pool: pool,
        arete: 0,
        owned_spheres: Vec::new(),
        resolved_pool,
        vulgar: false,
        correlation,
        kind: ActionKind::Generic,
    });
    crate::st::defer_ruling(ctx, actor, correlation, &req)?;
    ctx.out.sys_to_actor(
        actor,
        "wod",
        "You set your intent and present the action to the Storyteller for judgment...",
    );
    Ok(())
}

/// Split a raw `+action` arg on the ` using ` keyword (case-insensitive),
/// returning `(freeform, pool)`. With no keyword the whole string is the
/// freeform and the pool is empty (the default pool applies).
fn split_using(raw: &str) -> (String, String) {
    let lower = raw.to_ascii_lowercase();
    let kw = " using ";
    match lower.rfind(kw) {
        Some(idx) => (
            raw[..idx].trim().to_string(),
            raw[idx + kw.len()..].trim().to_string(),
        ),
        None => (raw.trim().to_string(), String::new()),
    }
}

/// Register the `+action` command.
pub(crate) fn register(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+action".into(),
            shape: ArgShape::Raw,
            help: "+action <what you do> [using <pool>]  --  Propose a freeform action for the \
                   Storyteller to adjudicate. Your description is a PROPOSAL to be judged, not a \
                   command: the ST sets the difficulty (and how hard you are to notice), and the \
                   engine re-resolves your pool against your sheet so it can never exceed it.\n\
                   If the ST allows it you are told what to roll; then type +roll/silent to throw \
                   the dice (\"dice in your hands\"). The room stays silent — only onlookers who \
                   make a Perception + Awareness roll notice what you did.\n\
                   With no `using <pool>` a bare Wits check is assumed."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| action_cmd(&cmd, ctx))
        },
    )
}
