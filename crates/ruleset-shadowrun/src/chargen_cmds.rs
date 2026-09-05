//! The `+chargen` command set: the SR5 priority point-buy as a directly-testable
//! command surface (the `+chargen/wizard` guided front door layers over it).
//!
//! `+chargen` opens a session + a fresh Unapproved sheet; `+chargen/priority`
//! assigns the A–E letters; `+stat` allocates attributes/skills with live budget
//! feedback; `+spec` records specializations; and `+chargen/submit` finalizes,
//! flips the sheet to
//! [`ApprovalState::Pending`](mush_ruleset::ApprovalState::Pending), and opens a
//! `shadowrun-chargen` approval job whose payload handler stamps it Approved.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar};
use mush_jobs::{Job, OpenJob, open_job};
use mush_ruleset::schema::FieldType;
use mush_ruleset::{ApprovalState, SheetDoc};

use crate::Sr5Cmd;
use crate::chargen::{self, PRIORITY_CATEGORIES, Priorities, ledgers, persisted_priorities};
use crate::data::{Priority, Sr5Config};
use crate::schema::CHARGEN_PRIORITIES_FIELD;
use crate::state::Sr5State;

/// The `shadowrun-chargen` job queue name.
pub const CHARGEN_QUEUE: &str = "shadowrun-chargen";
/// The chargen payload-handler name.
pub const CHARGEN_HANDLER: &str = "shadowrun.chargen";

/// `+chargen` dispatch.
fn chargen_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("priority") {
        return set_priority(cmd, ctx);
    }
    if ctx.switches.has("submit") {
        return submit(cmd, ctx);
    }
    if ctx.switches.has("status") {
        return status(cmd, ctx);
    }
    start(cmd, ctx)
}

/// Open (or resume) a chargen session for the actor: refuse an already-approved
/// / submitted sheet, ensure an Unapproved sheet exists, open the session, and
/// re-hydrate the priority assignment from the sheet (reboot-safe). Returns
/// whether a session was already open. Emits no output. Shared with the wizard.
pub(crate) fn open_chargen_session(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<bool, CmdError> {
    let actor = ctx.actor;
    let existing = cmd.read(ctx.w, actor);
    if let Some(doc) = &existing {
        match doc.approved {
            ApprovalState::Approved { .. } => {
                return Err(CmdError::msg("Your sheet is already approved."));
            }
            ApprovalState::Pending(job) => {
                return Err(CmdError::msg(format!(
                    "Your sheet is awaiting approval (job {job}); it is locked until a \
                     gamemaster resolves it."
                )));
            }
            ApprovalState::Unapproved => {}
        }
    }
    if existing.is_none() {
        let doc = SheetDoc::new_default(&cmd.schema);
        cmd.store()
            .put_by(ctx.w, actor, &doc, actor)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    }
    let persisted = existing
        .as_ref()
        .map(persisted_priorities)
        .unwrap_or_default();
    let already = ctx
        .state_mut::<Sr5State>()
        .map(|st| {
            let was = st.in_chargen(actor);
            if !was {
                st.open_chargen(actor);
            }
            if let Some(session) = st.chargen_mut(actor)
                && session.priorities.is_empty()
                && !persisted.map.is_empty()
            {
                session.priorities = persisted.map.clone();
            }
            was
        })
        .unwrap_or(false);
    Ok(already)
}

/// `+chargen` — start or resume.
fn start(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let already = open_chargen_session(cmd, ctx)?;
    if already {
        ctx.out.info(actor, "Resuming chargen.");
    } else {
        ctx.out.info(
            actor,
            "Chargen started. Assign priorities with +chargen/priority \
             (metatype=A attributes=B magic=C skills=D resources=E), then allocate \
             with +stat and +spec, and +chargen/submit when balanced. \
             +chargen/wizard walks you through it.",
        );
    }
    status(cmd, ctx)
}

/// `+chargen/priority metatype=A attributes=B magic=C skills=D resources=E`.
fn set_priority(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_chargen(ctx)?;
    let priorities = parse_priority_assignment(ctx.args.raw())?;
    apply_priorities(cmd, ctx, &priorities)?;
    ctx.out.info(ctx.actor, "Priorities set.");
    status(cmd, ctx)
}

/// Record a parsed priority assignment on the session and persist it to the
/// sheet's staff-only `chargen_priorities` map (reboot-safe + re-validated at
/// approval). Shared with the wizard.
pub(crate) fn apply_priorities(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    priorities: &Priorities,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    {
        let st = ctx
            .state_mut::<Sr5State>()
            .ok_or_else(|| CmdError::msg("The Shadowrun ruleset is not loaded."))?;
        let session = st
            .chargen_mut(actor)
            .ok_or_else(|| CmdError::msg("You are not in chargen."))?;
        session.priorities = priorities.map.clone();
    }
    let mut doc = cmd.read_or_default(ctx.w, actor);
    let store = cmd.store();
    for (cat, p) in &priorities.map {
        store
            .set_map_entry(
                &mut doc,
                CHARGEN_PRIORITIES_FIELD,
                cat,
                serde_json::json!(p.as_char().to_string()),
            )
            .map_err(|e| CmdError::msg(e.to_string()))?;
    }
    store
        .put_by(ctx.w, actor, &doc, actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    Ok(())
}

/// `+stat <field>[:<key>]=<value>` during chargen.
fn stat_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
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
    apply_stat(cmd, ctx, &field, key.as_deref(), &value_raw)?;
    status(cmd, ctx)
}

/// Apply one trait allocation to the in-progress sheet — the core of `+stat`,
/// shared with the wizard. Validates the field is chargen-allocatable, coerces
/// the value, persists, and recomputes derived stats after attribute changes.
pub(crate) fn apply_stat(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    field: &str,
    key: Option<&str>,
    value_raw: &str,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if !chargen_settable(field) {
        return Err(CmdError::msg(format!(
            "{field} is not a chargen-allocatable trait. Derived stats, Condition \
             Monitors, Karma, and bookkeeping fields are not set with +stat."
        )));
    }
    let mut doc = cmd.read_or_default(ctx.w, actor);
    let fdef = cmd
        .schema
        .field(field)
        .ok_or_else(|| CmdError::msg(format!("No such sheet field: {field}")))?
        .clone();
    let store = cmd.store();
    let audit = match (&fdef.ty, key) {
        (FieldType::Map { value }, Some(key)) => {
            if value_raw.trim().is_empty() {
                if doc.map_remove(field, key) {
                    format!("{field}:{key} cleared")
                } else {
                    return Err(CmdError::msg(format!("{field} has no entry {key:?}.")));
                }
            } else {
                let coerced = coerce(value, value_raw)
                    .map_err(|e| CmdError::msg(format!("{field}:{key}: {e}")))?;
                store
                    .set_map_entry(&mut doc, field, key, coerced)
                    .map_err(|e| CmdError::msg(e.to_string()))?;
                format!("{field}:{key} = {}", value_raw.trim())
            }
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
    // Choosing a metatype seeds every core attribute (and Edge) up to that
    // metatype's minimum, so a Troll starts at Body/Strength 5 rather than the
    // schema default of 1 (which the attribute ledger would otherwise treat as
    // free, unflagged spend).
    if field.eq_ignore_ascii_case("metatype") {
        chargen::seed_metatype_minimums(&mut doc);
    }
    // Recompute derived stats after any attribute/essence change so the CM
    // tracks + Limits stay consistent (the sheet must always validate).
    crate::derive::apply_derived(&mut doc);
    store
        .put_by(ctx.w, actor, &doc, actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.info(actor, format!("Set {audit}."));
    Ok(())
}

/// `+spec <skill>=<specialization>` — record a specialization on an active skill.
fn spec_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let skill = ctx.args.lhs().trim().to_string();
    let spec = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: +spec <skill>=<specialization>"))?
        .to_string();
    apply_spec(cmd, ctx, &skill, &spec)
}

/// Record a specialization on an active skill — shared with the wizard.
pub(crate) fn apply_spec(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    skill: &str,
    spec: &str,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let skill = skill.trim().to_ascii_lowercase();
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(CmdError::msg("Usage: +spec <skill>=<specialization>"));
    }
    let sk = crate::data::active_skill(&skill)
        .ok_or_else(|| CmdError::msg(format!("No such active skill: {skill}")))?;
    let mut doc = cmd.read_or_default(ctx.w, actor);
    let id = sk.id.to_string();
    let mut list: Vec<serde_json::Value> = doc
        .map(crate::schema::SKILL_SPECIALIZATIONS_FIELD)
        .and_then(|m| m.get(&id))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if list
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|s| s.eq_ignore_ascii_case(spec))
    {
        return Err(CmdError::msg("That specialization is already recorded."));
    }
    list.push(serde_json::json!(spec));
    let store = cmd.store();
    store
        .set_map_entry(
            &mut doc,
            crate::schema::SKILL_SPECIALIZATIONS_FIELD,
            &id,
            serde_json::Value::Array(list),
        )
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, actor, &doc, actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out
        .info(actor, format!("Specialization {spec:?} added to {id}."));
    Ok(())
}

/// The live budget for a chargen session (ledgers + outstanding issues). Shared
/// with the wizard.
pub(crate) fn budget_report(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
) -> Result<
    (
        Vec<chargen::Ledger>,
        Vec<mush_ruleset::chargen::ChargenIssue>,
    ),
    CmdError,
> {
    let actor = ctx.actor;
    let priorities = session_priorities(ctx, actor)?;
    let doc = cmd.read_or_default(ctx.w, actor);
    let ls = ledgers(&cmd.config, &doc, &priorities);
    let iss = chargen::issues(&cmd.config, &doc, &priorities);
    Ok((ls, iss))
}

/// The priorities for the actor's open session (falls back to the persisted set).
fn session_priorities(ctx: &mut Ctx<'_>, actor: ObjId) -> Result<Priorities, CmdError> {
    let st = ctx
        .state::<Sr5State>()
        .ok_or_else(|| CmdError::msg("The Shadowrun ruleset is not loaded."))?;
    let session = st
        .chargen(actor)
        .ok_or_else(|| CmdError::msg("You are not in chargen. Start with +chargen."))?;
    Ok(Priorities {
        map: session.priorities.clone(),
    })
}

/// `+chargen/status` — show the live budget.
fn status(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let (ls, outstanding) = budget_report(cmd, ctx)?;
    ctx.out.sys_to_actor(actor, "sr", "Chargen budget:");
    for l in &ls {
        let over = if l.spent > l.total {
            " OVER".to_string()
        } else {
            String::new()
        };
        ctx.out.sys_to_actor(
            actor,
            "sr",
            format!("  {:<24} {}/{}{over}", l.label, l.spent, l.total),
        );
    }
    if outstanding.is_empty() {
        ctx.out
            .info(actor, "Budget balances. Ready for +chargen/submit.");
    } else {
        ctx.out.sys_to_actor(actor, "sr", "Outstanding:");
        for i in outstanding.iter().take(8) {
            ctx.out.sys_to_actor(actor, "sr", format!("  - {i}"));
        }
    }
    Ok(())
}

/// `+chargen/submit` — finalize, flip to Pending, open a shadowrun-chargen job.
pub(crate) fn submit(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if let Some(doc) = cmd.read(ctx.w, actor) {
        match doc.approved {
            ApprovalState::Pending(job) => {
                return Err(CmdError::msg(format!(
                    "Your sheet is already submitted (job {job}); wait for a gamemaster."
                )));
            }
            ApprovalState::Approved { .. } => {
                return Err(CmdError::msg("Your sheet is already approved."));
            }
            ApprovalState::Unapproved => {}
        }
    }
    let priorities = session_priorities(ctx, actor)?;
    if !priorities.complete() {
        return Err(CmdError::msg(
            "Assign all five priorities first (+chargen/priority).",
        ));
    }
    let mut doc = cmd
        .read(ctx.w, actor)
        .ok_or_else(|| CmdError::msg("You have no sheet to submit."))?;
    chargen::finalize_sheet(&cmd.config, &mut doc, &priorities)
        .map_err(|i| CmdError::msg(format!("Cannot submit: {i}")))?;

    // Validate the exact finalized document before allocating an approval job.
    // Otherwise a malformed pre-existing sheet can make `put_by` fail only
    // after `open_job` has queued an orphan approval job for persistence.
    mush_ruleset::validate(cmd.store().schema(), &doc.data)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;

    let job = open_job(
        ctx,
        OpenJob {
            queue: CHARGEN_QUEUE.into(),
            title: format!("Chargen approval: {}", name_of(ctx, actor)),
            opener: actor,
            text: Some("Runner submitted for approval.".to_string()),
            payload: Some(mush_jobs::JobPayload {
                handler: CHARGEN_HANDLER.into(),
                data: serde_json::json!({ "target": actor.0 }),
            }),
            tags: vec!["chargen".into()],
        },
    )
    .map_err(CmdError::msg)?;

    doc.approved = ApprovalState::Pending(job);
    cmd.store()
        .put_by(ctx.w, actor, &doc, actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    if let Some(st) = ctx.state_mut::<Sr5State>() {
        st.close_chargen(actor);
    }
    ctx.out.info(
        actor,
        format!("Submitted for approval as job {job}. A gamemaster will review it."),
    );
    Ok(())
}

/// The `shadowrun.chargen` payload handler: re-validate against the persisted
/// priorities, then stamp Approved. Re-validation means a sheet edited out of
/// balance after submit is refused, leaving the job open.
pub(crate) fn chargen_approval(
    config: &Sr5Config,
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    job: &Job,
) -> Result<(), String> {
    let target = payload_target(job)?;
    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| "the character has no sheet".to_string())?;
    let priorities = persisted_priorities(&doc);
    if let Some(issue) = chargen::issues(config, &doc, &priorities)
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
    cmd.store()
        .put_by(ctx.w, target, &doc, ctx.actor)
        .map_err(|es| fmt_errs(&es))?;
    Ok(())
}

/// The E6a half of approval (08f §3.5): open the `sr.nuyen` account and mint the
/// priority allotment, **after** [`chargen_approval`] has stamped the sheet.
///
/// It is deliberately a second call rather than a step inside `chargen_approval`,
/// and deliberately returns nothing: a sheet that has been signed off is not
/// un-signed by an economic hiccup (08d deviation 11), so the mint's own refusal is
/// reported to the approving staffer and the approval stands. `@sr/migrate commit`
/// adopts a character approved with an empty account (M13).
pub(crate) fn chargen_economy(
    econ: &crate::economy::SrEconomyConfig,
    _config: &Sr5Config,
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    job: &Job,
) {
    let Ok(target) = payload_target(job) else {
        return;
    };
    let Some(doc) = cmd.read(ctx.w, target) else {
        return;
    };
    let lines = crate::economy::chargen::on_approved(ctx, econ, &cmd.config, &doc, target);
    let actor = ctx.actor;
    for line in lines {
        ctx.out.sys_to_actor(actor, "sr.economy", line);
    }
}

fn payload_target(job: &Job) -> Result<ObjId, String> {
    job.payload
        .as_ref()
        .and_then(|p| p.data.get("target"))
        .and_then(serde_json::Value::as_u64)
        .map(|n| ObjId(n as u32))
        .ok_or_else(|| "job payload has no target".to_string())
}

/// Parse `metatype=A attributes=B magic=C skills=D resources=E` (exact category
/// set, valid A–E permutation).
pub(crate) fn parse_priority_assignment(raw: &str) -> Result<Priorities, CmdError> {
    let mut map: Vec<(String, Priority)> = Vec::new();
    for tok in raw.split_whitespace() {
        let (cat, letter) = tok
            .split_once('=')
            .ok_or_else(|| CmdError::msg("Use <category>=<A-E> for each of the five."))?;
        let cat = cat.trim().to_lowercase();
        if !PRIORITY_CATEGORIES.iter().any(|c| *c == cat) {
            return Err(CmdError::msg(format!(
                "Unknown category {cat:?}; expected one of {PRIORITY_CATEGORIES:?}."
            )));
        }
        if map.iter().any(|(c, _)| *c == cat) {
            return Err(CmdError::msg(format!("Category {cat:?} given twice.")));
        }
        let p = letter
            .trim()
            .chars()
            .next()
            .and_then(Priority::from_char)
            .ok_or_else(|| CmdError::msg(format!("{letter:?} is not a priority A-E.")))?;
        map.push((cat, p));
    }
    let priorities = Priorities { map };
    if !priorities.complete() {
        return Err(CmdError::msg(
            "Assign all five: metatype=A attributes=B magic=C skills=D resources=E.",
        ));
    }
    if !priorities.is_permutation() {
        return Err(CmdError::msg(
            "The five priorities must use each of A, B, C, D, E exactly once.",
        ));
    }
    Ok(priorities)
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

/// Fields `+stat` must never touch during chargen: derived stats, the Condition
/// Monitors, session Edge, Karma bookkeeping, and the staff-only chargen record.
fn chargen_settable(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    !matches!(
        f.as_str(),
        "physical_limit"
            | "mental_limit"
            | "social_limit"
            | "initiative_base"
            | "phys_cm_max"
            | "stun_cm_max"
            | "physical_overflow"
            | "condition_physical"
            | "condition_stun"
            | "edge_current"
            | "karma"
            | "karma_total"
            | CHARGEN_PRIORITIES_FIELD
    )
}

fn require_chargen(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let ok = ctx
        .state::<Sr5State>()
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
    cmd: Arc<Sr5Cmd>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+chargen".into(),
            switches: vec![
                "wizard".into(),
                "status".into(),
                "priority".into(),
                "submit".into(),
            ],
            shape: ArgShape::Raw,
            help: "+chargen  --  Begin SR5 character generation (priority point-buy).\n\
                   +chargen/wizard  --  Guided step-by-step creation.\n\
                   +chargen/priority metatype=A attributes=B magic=C skills=D \
                   resources=E  --  Assign the A–E priorities (each used once).\n\
                   +chargen/status  --  Show the live budget.\n\
                   +chargen/submit  --  Finalize and file for gamemaster approval."
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
                   with live budget feedback (e.g. +stat body=5, +stat \
                   active_skills:pistols=4, +stat metatype=ork)."
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
            help: "+spec <skill>=<specialization>  --  Record a specialization on an \
                   active skill (+2 dice when it applies; used by +roll spec <name>)."
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
