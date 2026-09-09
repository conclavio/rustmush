//! The `+chargen` command set: the classic priority point-buy driven as a
//! command surface (chosen over an input-context wizard for a cleaner,
//! directly-testable UX; the wizard is the M9 terminal/web polish job).
//!
//! `+chargen` opens a session and a fresh Unapproved sheet; `+chargen/attr` and
//! `+chargen/abil` assign the 7/5/3 and 13/9/5 priorities; `+stat` allocates
//! dots with live budget feedback; `+spec` records specialties; and
//! `+chargen/submit` finalizes, flips the sheet to
//! [`ApprovalState::Pending`](mush_ruleset::ApprovalState::Pending), and opens a
//! `wod-chargen` approval job whose payload handler stamps the sheet Approved.
//!
//! The in-progress sheet persists as it is built (it is a normal Unapproved
//! `SheetDoc`); only the priority-pool choices live in the transient
//! [`ChargenSession`](crate::state::ChargenSession).

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar};
use mush_jobs::{Job, OpenJob, open_job};
use mush_ruleset::schema::FieldType;
use mush_ruleset::{ApprovalState, SheetDoc};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::chargen::{
    CHARGEN_POOLS_FIELD, Pools, ledgers, persisted_pools, persisted_priorities, session_pools,
};
use crate::config::WodConfig;
use crate::splats::{Dispatch, retemplate};
use crate::state::WodState;

/// The `wod-chargen` job queue name.
pub const CHARGEN_QUEUE: &str = "wod-chargen";
/// The chargen payload-handler name.
pub const CHARGEN_HANDLER: &str = "wod.chargen";

/// `+chargen` / `+chargen/status` / `+chargen/attr` / `+chargen/abil` /
/// `+chargen/submit`.
fn chargen_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("template") {
        return set_template(cmd, ctx);
    }
    if ctx.switches.has("attr") {
        return set_priorities(cmd, ctx, true);
    }
    if ctx.switches.has("abil") {
        return set_priorities(cmd, ctx, false);
    }
    if ctx.switches.has("residence") {
        return residence(cmd, ctx);
    }
    if ctx.switches.has("submit") {
        return submit(cmd, ctx);
    }
    if ctx.switches.has("status") {
        return status(cmd, ctx);
    }
    // Bare +chargen: start (or resume) a session.
    start(cmd, ctx)
}

/// The pools for a chargen assessment: session priority pools + the active
/// splat's contribution (selected by the sheet's `template`).
fn assessment_pools(cmd: &WodCmd, session_or_persisted: Pools, doc: &SheetDoc) -> Pools {
    let splat = cmd.splat_for_doc(doc);
    session_or_persisted.with_splat(&cmd.config, splat.as_ref(), doc)
}

/// `+chargen/template <id>` — choose a splat (Vampire/Mage/…). Re-shapes the
/// sheet to that template's schema, wiping any previous splat's fields.
fn set_template(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_chargen(ctx)?;
    let id = ctx.args.raw().trim().to_string();
    if id.is_empty() {
        let choices = cmd.schemas().ids().join(", ");
        return Err(CmdError::msg(format!(
            "Choose a template: +chargen/template <{choices}>."
        )));
    }
    set_template_to(cmd, ctx, &id)
}

/// Open (or resume) a chargen session for the actor: refuse an
/// already-approved / submitted sheet, ensure an Unapproved sheet exists, open
/// the [`WodState`] session, and re-hydrate the priority pools from the sheet
/// (reboot-safe). Returns whether a session was **already** open. Emits no
/// output. Shared by `+chargen` and the chargen wizard.
pub(crate) fn open_chargen_session(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<bool, CmdError> {
    let actor = ctx.actor;
    let existing = cmd.read(ctx.w, actor);
    if let Some(doc) = &existing {
        match doc.approved {
            ApprovalState::Approved { .. } => {
                return Err(CmdError::msg("Your sheet is already approved."));
            }
            // A submitted (Pending) sheet is locked: re-opening it would let
            // `+stat` mutate it past the budget and then be rubber-stamped by
            // the still-open approval job. Ask staff to bounce the job first.
            ApprovalState::Pending(job) => {
                return Err(CmdError::msg(format!(
                    "Your sheet is awaiting approval (job {job}); it is locked until a \
                     storyteller resolves it."
                )));
            }
            ApprovalState::Unapproved => {}
        }
    }
    // Ensure an Unapproved sheet exists (a fresh sheet starts as the default
    // template; the player picks a splat with +chargen/template).
    if existing.is_none() {
        let doc = SheetDoc::new_default(&cmd.default_schema());
        cmd.default_store()
            .put_in(actor, &doc, actor, ctx)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    }
    // Re-hydrate the priority assignment from the sheet (reboot-safe: a session
    // lost to a reboot / dropped link comes back with the same budget basis).
    let (attr_pools, abil_pools) = existing
        .as_ref()
        .map(persisted_priorities)
        .unwrap_or_default();
    let already = ctx
        .state_mut::<WodState>()
        .map(|st| {
            let was = st.in_chargen(actor);
            if !was {
                st.open_chargen(actor);
            }
            if let Some(session) = st.chargen_mut(actor) {
                if session.attr_pools.is_empty() && !attr_pools.is_empty() {
                    session.attr_pools = attr_pools;
                }
                if session.abil_pools.is_empty() && !abil_pools.is_empty() {
                    session.abil_pools = abil_pools;
                }
            }
            was
        })
        .unwrap_or(false);
    Ok(already)
}

/// Start or resume chargen (`+chargen`).
fn start(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let already = open_chargen_session(cmd, ctx)?;
    if already {
        ctx.out.info(actor, "Resuming chargen.");
    } else {
        ctx.out.info(
            actor,
            "Chargen started. Pick a splat with +chargen/template (mortal is the \
             default), set priorities with +chargen/attr and +chargen/abil, allocate \
             with +stat, then +chargen/submit.",
        );
    }
    status(cmd, ctx)
}

/// `+chargen/attr physical=7 social=5 mental=3` (or the ability groups).
fn set_priorities(cmd: &WodCmd, ctx: &mut Ctx<'_>, attributes: bool) -> Result<(), CmdError> {
    let actor = ctx.actor;
    require_chargen(ctx)?;
    let (names, tiers): (Vec<&str>, [i32; 3]) = if attributes {
        (
            vec!["physical", "social", "mental"],
            cmd.config.attr_priorities,
        )
    } else {
        (
            vec!["talents", "skills", "knowledges"],
            cmd.config.abil_priorities,
        )
    };
    let assignment = parse_priority_assignment(ctx.args.raw(), &names, tiers)?;
    apply_priorities(cmd, ctx, attributes, assignment)?;
    ctx.out.info(actor, "Priorities set.");
    status(cmd, ctx)
}

/// Record a parsed priority assignment on the session **and** persist it onto
/// the sheet (staff-only `chargen_pools`, so it survives a reboot and can be
/// re-validated at approval time). Shared by `+chargen/attr|abil` and the
/// chargen wizard.
pub(crate) fn apply_priorities(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    attributes: bool,
    assignment: Vec<(SmolStr, i32)>,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let session = st
            .chargen_mut(actor)
            .ok_or_else(|| CmdError::msg("You are not in chargen."))?;
        if attributes {
            session.attr_pools = assignment.clone();
        } else {
            session.abil_pools = assignment.clone();
        }
    }
    let mut doc = cmd.read_or_default(ctx.w, actor);
    let store = cmd.store_for_doc(&doc);
    for (cat, n) in &assignment {
        store
            .set_map_entry(&mut doc, CHARGEN_POOLS_FIELD, cat, serde_json::json!(n))
            .map_err(|e| CmdError::msg(e.to_string()))?;
    }
    store
        .put_in(actor, &doc, actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    Ok(())
}

/// `+stat <field>[:<key>]=<value>` during chargen.
fn stat_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_chargen(ctx)?;
    let lhs = ctx.args.lhs().trim().to_string();
    let value_raw = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: +stat <field>[:<key>]=<value>"))?
        .to_string();
    if lhs.is_empty() {
        return Err(CmdError::msg("Usage: +stat <field>[:<key>]=<value>"));
    }
    let (field, key) = match lhs.split_once(':') {
        Some((f, k)) => (f.trim().to_string(), Some(k.trim().to_string())),
        None => (lhs.clone(), None),
    };
    // Changing the template re-shapes the sheet — route it through the same
    // path as +chargen/template.
    if field.eq_ignore_ascii_case("template") {
        return set_template_to(cmd, ctx, value_raw.trim());
    }
    apply_stat(cmd, ctx, &field, key.as_deref(), &value_raw)?;
    status(cmd, ctx)
}

/// Apply one trait allocation to the in-progress sheet — the core of `+stat`,
/// shared with the chargen wizard. Validates against the active splat's
/// chargen rules (settable fields, locked fields, chargen ceilings), coerces
/// the value for the field's type, persists, and lets the splat recompute
/// derived fields. Emits a `Set …` confirmation but no budget display (the
/// caller renders that). `template` is routed to [`apply_template`].
///
/// The caller is responsible for ensuring a chargen session is open
/// (`+stat` checks `require_chargen`; the wizard opens one on entry).
pub(crate) fn apply_stat(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    field: &str,
    key: Option<&str>,
    value_raw: &str,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if field.eq_ignore_ascii_case("template") {
        apply_template(cmd, ctx, value_raw.trim())?;
        return Ok(());
    }
    let mut doc = cmd.read_or_default(ctx.w, actor);
    let splat = cmd.splat_for_doc(&doc);
    // Resolve the field to its schema id BEFORE any gate reads it. Everything
    // below — the settable list, the splat's locked list, the chargen ceiling —
    // is written in canonical lowercase ids, and `chargen_locked_fields` is
    // compared with `contains`, which is exact. Tested against the raw text,
    // `+stat GENERATION=4` sailed past a lock that only ever saw "generation".
    // `SheetSchema::field` matches ids case-insensitively and matches nothing
    // else (no aliases), so canonicalising here is exactly a case fold — and
    // `store.set_field` already wrote `f.id` regardless of the spelling typed.
    let schema = cmd.schema_for_doc(&doc);
    let fdef = schema
        .field(field)
        .ok_or_else(|| CmdError::msg(format!("No such sheet field: {field}")))?
        .clone();
    let field = fdef.id.as_str();
    if !chargen_settable(field) || splat.chargen_locked_fields().contains(&field) {
        return Err(CmdError::msg(format!(
            "{field} is not a chargen-allocatable trait. XP, health, derived, and \
             bookkeeping fields are not set with +stat."
        )));
    }
    // A splat-imposed chargen ceiling (Mage: Arete caps at 3 at chargen).
    if let Some(cap) = splat.chargen_max(field)
        && value_raw.trim().parse::<i32>().is_ok_and(|n| n > cap)
    {
        return Err(CmdError::msg(format!(
            "{field} caps at {cap} during chargen; raise it further with XP after approval."
        )));
    }

    let store = cmd.store_for_doc(&doc);
    let audit = match (&fdef.ty, key) {
        (FieldType::Map { value }, Some(key)) => {
            let coerced = coerce(value, value_raw)
                .map_err(|e| CmdError::msg(format!("{field}:{key}: {e}")))?;
            store
                .set_map_entry(&mut doc, field, key, coerced)
                .map_err(|e| CmdError::msg(e.to_string()))?;
            format!("{field}:{key} = {}", value_raw.trim())
        }
        (FieldType::Map { .. }, None) => {
            return Err(CmdError::msg(format!(
                "{field} is a map; use +stat {field}:<name>=<value>."
            )));
        }
        (
            FieldType::Dots { .. }
            | FieldType::Number { .. }
            | FieldType::Text { .. }
            | FieldType::Enum { .. },
            None,
        ) => {
            let coerced =
                coerce(&fdef.ty, value_raw).map_err(|e| CmdError::msg(format!("{field}: {e}")))?;
            store
                .set_field(&mut doc, field, coerced)
                .map_err(|e| CmdError::msg(e.to_string()))?;
            format!("{field} = {}", value_raw.trim())
        }
        (ty, _) => {
            return Err(CmdError::msg(format!(
                "+stat cannot set a {} field here.",
                ty.name()
            )));
        }
    };
    // Let the splat recompute any derived fields (Vampire: clan weakness).
    splat.on_field_changed(&mut doc, field);
    store
        .put_in(actor, &doc, actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.info(actor, format!("Set {audit}."));
    Ok(())
}

/// Re-shape the in-progress sheet to `id`'s template (the pure apply shared by
/// `+chargen/template`, `+stat template=`, and the chargen wizard). Persists;
/// emits no output. Returns the canonical (lowercased) template id.
pub(crate) fn apply_template(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    id: &str,
) -> Result<String, CmdError> {
    let actor = ctx.actor;
    let id = id.to_ascii_lowercase();
    if !cmd.schemas().ids().iter().any(|t| t == &id) {
        let choices = cmd.schemas().ids().join(", ");
        return Err(CmdError::msg(format!(
            "Unknown template {id:?}. Choose one of: {choices}."
        )));
    }
    let target_schema = cmd.schemas().schema_for_id(&id);
    let mut doc = cmd.read_or_default(ctx.w, actor);
    retemplate(&mut doc, &target_schema, &id);
    cmd.store_for_doc(&doc)
        .put_in(actor, &doc, actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    Ok(id)
}

/// Set the template during chargen (shared by `+chargen/template` and
/// `+stat template=`): [`apply_template`] plus the confirmation and budget.
fn set_template_to(cmd: &WodCmd, ctx: &mut Ctx<'_>, id: &str) -> Result<(), CmdError> {
    let id = apply_template(cmd, ctx, id)?;
    ctx.out.info(ctx.actor, format!("Template set to {id}."));
    status(cmd, ctx)
}

/// `+chargen/residence [#<property>|none]` — the E5c **Residence** stage (08d §5.4)
/// on the classic command surface; the wizard's step drives the same helpers.
///
/// With no argument it **lists** the vacancies this character's Resources can carry:
/// names and tiers only. A `Let` or `Withdrawn` property is not listed at all, so
/// nothing about who lives where is disclosed — by presence or by absence. The step
/// is optional (08d §11 answer 2, binding): a character who never runs it is approved
/// with no home.
fn residence(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_chargen(ctx)?;
    if !cmd.econ.personal_mode.is_on() || !cmd.econ.housing {
        return Err(CmdError::msg("This game does not assign homes at chargen."));
    }
    let actor = ctx.actor;
    let level = crate::economy::sheet_resources(ctx.w, actor).unwrap_or(0);
    let raw = ctx.args.raw().trim().to_string();
    if raw.is_empty() {
        let offered = crate::economy::chargen::vacancies(ctx, level);
        ctx.out.sys_to_actor(
            actor,
            "wod",
            format!("Residence — your Resources ({level}) can carry tier {level} or lower:"),
        );
        if offered.is_empty() {
            ctx.out.info(
                actor,
                "Nothing is free right now. That is fine — staff can house you later.",
            );
            return Ok(());
        }
        for o in &offered {
            ctx.out.sys_to_actor(
                actor,
                "wod",
                format!("  #{}  {}  (tier {})", o.property, o.name, o.tier),
            );
        }
        ctx.out
            .info(actor, "Pick one with +chargen/residence #<n>, or 'none'.");
        return Ok(());
    }
    if raw.eq_ignore_ascii_case("none") || raw.eq_ignore_ascii_case("nowhere") {
        crate::economy::chargen::choose(ctx, actor, level, None).map_err(CmdError::msg)?;
        ctx.out.info(actor, "No residence chosen.");
        return Ok(());
    }
    let id = raw
        .trim_start_matches('#')
        .parse::<u64>()
        .map_err(|_| CmdError::msg("Choose one of the numbers listed, or 'none'."))?;
    let picked =
        crate::economy::chargen::choose(ctx, actor, level, Some(mush_housing::PropertyId(id)))
            .map_err(CmdError::msg)?;
    match picked {
        Some(v) => ctx.out.info(
            actor,
            format!("Residence set to {} (tier {}).", v.name, v.tier),
        ),
        None => ctx.out.info(actor, "No residence chosen."),
    }
    Ok(())
}

/// `+spec <ability>=<specialty>` — record a specialty on an ability.
fn spec_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let ability = ctx.args.lhs().trim().to_string();
    let specialty = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: +spec <ability>=<specialty>"))?
        .to_string();
    apply_spec(cmd, ctx, &ability, &specialty)
}

/// Record a free-text specialty on an ability — the core of `+spec`, shared
/// with the chargen wizard.
pub(crate) fn apply_spec(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    ability: &str,
    specialty: &str,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let ability = ability.trim();
    let specialty = specialty.trim();
    if specialty.is_empty() {
        return Err(CmdError::msg("Usage: +spec <ability>=<specialty>"));
    }
    let mut doc = cmd.read_or_default(ctx.w, actor);
    let schema = cmd.schema_for_doc(&doc);
    let fdef = schema
        .field(ability)
        .ok_or_else(|| CmdError::msg(format!("No such ability: {ability}")))?;
    if !matches!(fdef.ty, FieldType::Dots { .. }) {
        return Err(CmdError::msg("Specialties attach to abilities/attributes."));
    }
    let id = fdef.id.to_string();
    let mut list: Vec<serde_json::Value> = doc
        .map("specialties")
        .and_then(|m| m.get(&id))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if list
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|s| s.eq_ignore_ascii_case(specialty))
    {
        return Err(CmdError::msg("That specialty is already recorded."));
    }
    list.push(serde_json::json!(specialty));
    let store = cmd.store_for_doc(&doc);
    store
        .set_map_entry(&mut doc, "specialties", &id, serde_json::Value::Array(list))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_in(actor, &doc, actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out
        .info(actor, format!("Specialty {specialty:?} added to {id}."));
    Ok(())
}

/// The live budget for a chargen session: the ledgers (session priority pools
/// plus the active splat's contribution, assessed against the sheet) and the
/// outstanding [`ChargenIssue`]s. Shared by `+chargen/status` and the chargen
/// wizard so both show the exact same numbers.
pub(crate) fn budget_report(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
) -> Result<
    (
        Vec<mush_ruleset::chargen::BudgetLedger>,
        Vec<mush_ruleset::chargen::ChargenIssue>,
    ),
    CmdError,
> {
    let actor = ctx.actor;
    let base = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let session = st
            .chargen(actor)
            .ok_or_else(|| CmdError::msg("You are not in chargen. Start with +chargen."))?;
        session_pools(&cmd.config, session)
    };
    let doc = cmd.read_or_default(ctx.w, actor);
    let pools = assessment_pools(cmd, base, &doc);
    let ledgers = ledgers(&cmd.config, &doc, &pools);
    let issues = crate::chargen::issues(&cmd.config, &doc, &pools);
    Ok((ledgers, issues))
}

/// Show the live budget ledgers.
fn status(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let (ls, outstanding) = budget_report(cmd, ctx)?;
    ctx.out.sys_to_actor(actor, "wod", "Chargen budget:");
    for l in &ls {
        let over = if l.spent > l.total && l.name != "freebies" {
            format!(" (+{} to freebies)", l.spent - l.total)
        } else {
            String::new()
        };
        ctx.out.sys_to_actor(
            actor,
            "wod",
            format!(
                "  {:<12} {}/{}{over}",
                l.label,
                l.spent.min(l.total),
                l.total
            ),
        );
    }
    if outstanding.is_empty() {
        ctx.out
            .info(actor, "Budget balances. Ready for +chargen/submit.");
    } else {
        ctx.out.info(
            actor,
            format!("{} item(s) still to resolve.", outstanding.len()),
        );
    }
    Ok(())
}

/// `+chargen/submit` — finalize, flip to Pending, open a wod-chargen job.
pub(crate) fn submit(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    // Refuse to re-submit an already-submitted (Pending) or approved sheet:
    // that would orphan the first job and open a second one for one character.
    if let Some(doc) = cmd.read(ctx.w, actor) {
        match doc.approved {
            ApprovalState::Pending(job) => {
                return Err(CmdError::msg(format!(
                    "Your sheet is already submitted (job {job}); wait for a storyteller."
                )));
            }
            ApprovalState::Approved { .. } => {
                return Err(CmdError::msg("Your sheet is already approved."));
            }
            ApprovalState::Unapproved => {}
        }
    }
    let base = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let session = st
            .chargen(actor)
            .ok_or_else(|| CmdError::msg("You are not in chargen."))?;
        if !session.priorities_set() {
            return Err(CmdError::msg(
                "Set attribute and ability priorities first (+chargen/attr, +chargen/abil).",
            ));
        }
        session_pools(&cmd.config, session)
    };
    let mut doc = cmd
        .read(ctx.w, actor)
        .ok_or_else(|| CmdError::msg("You have no sheet to submit."))?;
    let splat = cmd.splat_for_doc(&doc);
    let pools = base.with_splat(&cmd.config, splat.as_ref(), &doc);
    crate::chargen::finalize_sheet(&cmd.config, &mut doc, &pools, splat.as_ref())
        .map_err(|i| CmdError::msg(format!("Cannot submit: {i}")))?;

    // Validate the exact finalized document before allocating an approval job.
    // `open_job` mutates the jobs cache and queues its write-through ops; if
    // validation waited for `put_by` below, a malformed pre-existing sheet
    // could report submit failure while still committing an orphan job.
    mush_ruleset::validate(cmd.store_for_doc(&doc).schema(), &doc.data)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;

    // Open the approval job carrying the target, then mark the sheet Pending.
    let job = open_job(
        ctx,
        OpenJob {
            queue: CHARGEN_QUEUE.into(),
            title: format!("Chargen approval: {}", name_of(ctx, actor)),
            opener: actor,
            text: Some("Character submitted for approval.".to_string()),
            payload: Some(mush_jobs::JobPayload {
                handler: CHARGEN_HANDLER.into(),
                data: serde_json::json!({ "target": actor.0 }),
            }),
            tags: vec!["chargen".into()],
        },
    )
    .map_err(CmdError::msg)?;

    doc.approved = ApprovalState::Pending(job);
    cmd.store_for_doc(&doc)
        .put_in(actor, &doc, actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    if let Some(st) = ctx.state_mut::<WodState>() {
        st.close_chargen(actor);
    }
    ctx.out.info(
        actor,
        format!("Submitted for approval as job {job}. A storyteller will review it."),
    );
    Ok(())
}

/// The `wod.chargen` payload handler: re-validate the budget against the
/// pools the sheet was built against (persisted on the sheet), then stamp it
/// Approved. Re-validation makes approval more than a rubber stamp — a sheet
/// that no longer balances (e.g. edited after submit) is refused, leaving the
/// job open.
pub(crate) fn chargen_approval(
    config: &WodConfig,
    dispatch: &Dispatch,
    onboarding: &crate::onboarding::OnboardingConfig,
    econ: &crate::economy::WodEconomyConfig,
    ctx: &mut Ctx<'_>,
    job: &Job,
) -> Result<(), String> {
    let target = payload_target(job)?;
    // Read schema-agnostically, then re-validate against the active template's
    // pools + contribution.
    let mut doc = dispatch
        .read(ctx.w, target)
        .ok_or_else(|| "the character has no sheet".to_string())?;
    let splat = dispatch.splats.for_doc(&doc);
    let pools = persisted_pools(config, &doc).with_splat(config, splat.as_ref(), &doc);
    if let Some(issue) = crate::chargen::issues(config, &doc, &pools)
        .into_iter()
        .next()
    {
        return Err(format!(
            "the sheet no longer balances ({issue}); it cannot be approved as-is"
        ));
    }
    doc.approved = ApprovalState::Approved {
        by: ctx.actor,
        at: ctx.w.now(),
    };
    dispatch
        .store_for_doc(&doc)
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| fmt_errs(&es))?;
    // E5c (08d §5.4): the economic artifacts, per `personal_mode` — nothing / a
    // `SpendingProfile` / an account with starting funds — plus the tenancy if the
    // player picked a residence. Deliberately **after** the sheet is stamped and
    // **before** the onboarding move, and deliberately not fallible: a sheet a
    // storyteller has signed off is not un-signed because an account could not be
    // opened. Anything that went wrong is reported to the approver instead.
    let by = ctx.actor;
    for line in crate::economy::chargen::on_approved(ctx, econ, target) {
        ctx.out.sys_to_actor(by, "wod", line);
    }
    // Onboarding: lift the gate — move the newly-approved character to the live
    // start room and re-render it (a no-op when chargen_required is off).
    crate::onboarding::on_approved(onboarding, ctx, target);
    Ok(())
}

fn payload_target(job: &Job) -> Result<ObjId, String> {
    job.payload
        .as_ref()
        .and_then(|p| p.data.get("target"))
        .and_then(serde_json::Value::as_u64)
        .map(|n| ObjId(n as u32))
        .ok_or_else(|| "job payload has no target".to_string())
}

/// Parse `cat=n cat=n cat=n`, requiring the exact category set and the exact
/// tier multiset.
pub(crate) fn parse_priority_assignment(
    raw: &str,
    names: &[&str],
    tiers: [i32; 3],
) -> Result<Vec<(SmolStr, i32)>, CmdError> {
    let mut out: Vec<(SmolStr, i32)> = Vec::new();
    for tok in raw.split_whitespace() {
        let (cat, n) = tok
            .split_once('=')
            .ok_or_else(|| CmdError::msg("Use <category>=<n> for each of the three."))?;
        let cat = cat.trim().to_lowercase();
        if !names.iter().any(|nm| *nm == cat) {
            return Err(CmdError::msg(format!(
                "Unknown category {cat:?}; expected one of {names:?}."
            )));
        }
        if out.iter().any(|(c, _)| c.as_str() == cat) {
            return Err(CmdError::msg(format!("Category {cat:?} given twice.")));
        }
        let n: i32 = n
            .trim()
            .parse()
            .map_err(|_| CmdError::msg(format!("Bad number in {tok:?}.")))?;
        out.push((SmolStr::new(cat), n));
    }
    if out.len() != names.len() {
        return Err(CmdError::msg(format!(
            "Assign all {} categories, e.g. {}.",
            names.len(),
            example(names, tiers)
        )));
    }
    let mut got: Vec<i32> = out.iter().map(|(_, n)| *n).collect();
    let mut want: Vec<i32> = tiers.to_vec();
    got.sort_unstable();
    want.sort_unstable();
    if got != want {
        return Err(CmdError::msg(format!(
            "The three values must be exactly {tiers:?}."
        )));
    }
    Ok(out)
}

fn example(names: &[&str], tiers: [i32; 3]) -> String {
    names
        .iter()
        .zip(tiers.iter())
        .map(|(n, t)| format!("{n}={t}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Coerce a raw `+stat` value string to JSON for a scalar/map-element type.
fn coerce(ty: &FieldType, raw: &str) -> Result<serde_json::Value, String> {
    let raw = raw.trim();
    match ty {
        FieldType::Dots { .. } | FieldType::Number { .. } => raw
            .parse::<i64>()
            .map(|n| serde_json::json!(n))
            .map_err(|_| format!("{raw:?} is not a number")),
        FieldType::Text { .. } | FieldType::Enum { .. } => Ok(serde_json::json!(raw)),
        other => Err(format!("cannot set a {} value from text", other.name())),
    }
}

/// Fields `+stat` must never touch during chargen: experience/XP (would mint
/// free starting XP — none of these appear in any budget ledger), the derived
/// temporary Willpower pool (set from permanent at finalize), the Health track
/// (that is `+harm`/`+heal`), and the staff-only chargen bookkeeping map.
fn chargen_settable(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    !matches!(
        f.as_str(),
        "experience" | "total_xp" | "xp_log" | "willpower_temp"
    ) && f != crate::schema::HEALTH_FIELD
        && f != CHARGEN_POOLS_FIELD
}

fn require_chargen(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let ok = ctx
        .state::<WodState>()
        .is_some_and(|st| st.in_chargen(ctx.actor));
    if ok {
        Ok(())
    } else {
        Err(CmdError::msg(
            "You are not in chargen. Start with +chargen.",
        ))
    }
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Register the chargen command set.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    cmd: Arc<WodCmd>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+chargen".into(),
            switches: vec![
                "wizard".into(),
                "status".into(),
                "template".into(),
                "attr".into(),
                "abil".into(),
                "residence".into(),
                "submit".into(),
            ],
            shape: ArgShape::Raw,
            help: "+chargen  --  Begin character generation (priority point-buy).\n\
                   +chargen/wizard  --  Guided step-by-step character creation.\n\
                   +chargen/template <mortal|vampire|…>  --  Choose your splat (adds \
                   its traits to the sheet).\n\
                   +chargen/attr physical=7 social=5 mental=3  --  Assign attribute \
                   priorities.\n\
                   +chargen/abil talents=13 skills=9 knowledges=5  --  Assign ability \
                   priorities.\n\
                   +chargen/residence [#<n>|none]  --  Where you live: the vacant places \
                   your Resources can carry, by name and tier. Optional.\n\
                   +chargen/status  --  Show the live budget.\n\
                   +chargen/submit  --  Finalize and file for staff approval."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| {
                if ctx.switches.has("wizard") {
                    return crate::wizard::start_wizard(Arc::clone(&cmd), ctx);
                }
                chargen_cmd(&cmd, ctx)
            })
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+stat".into(),
            shape: ArgShape::EqPair,
            help: "+stat <field>[:<key>]=<value>  --  Allocate a trait during chargen \
                   with live budget feedback (e.g. +stat strength=3, +stat \
                   backgrounds:Resources=2, +stat concept=Detective)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| stat_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+spec".into(),
            shape: ArgShape::EqPair,
            help: "+spec <ability>=<specialty>  --  Record a free-text specialty on \
                   an ability (used by +roll/spec)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| spec_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}
