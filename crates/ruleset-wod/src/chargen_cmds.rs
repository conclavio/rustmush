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
use mush_ruleset::{ApprovalState, SheetDoc, SheetStore};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::chargen::{
    CHARGEN_POOLS_FIELD, ledgers, persisted_pools, persisted_priorities, session_pools,
};
use crate::config::WodConfig;
use crate::state::WodState;

/// The `wod-chargen` job queue name.
pub const CHARGEN_QUEUE: &str = "wod-chargen";
/// The chargen payload-handler name.
pub const CHARGEN_HANDLER: &str = "wod.chargen";

/// `+chargen` / `+chargen/status` / `+chargen/attr` / `+chargen/abil` /
/// `+chargen/submit`.
fn chargen_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("attr") {
        return set_priorities(cmd, ctx, true);
    }
    if ctx.switches.has("abil") {
        return set_priorities(cmd, ctx, false);
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

/// Start or resume chargen.
fn start(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let existing = cmd.store.get(ctx.w, actor);
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
    // Ensure an Unapproved sheet exists.
    if existing.is_none() {
        let doc = SheetDoc::new_default(&cmd.schema);
        cmd.store
            .put_by(ctx.w, actor, &doc, actor)
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
    if already {
        ctx.out.info(actor, "Resuming chargen.");
    } else {
        ctx.out.info(
            actor,
            "Chargen started. Set priorities with +chargen/attr and +chargen/abil, \
             allocate with +stat, then +chargen/submit.",
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
    // Persist the assignment onto the sheet (staff-only `chargen_pools`) so it
    // survives a reboot and can be re-validated at approval time.
    let mut doc = cmd.store.get_or_default(ctx.w, actor);
    for (cat, n) in &assignment {
        cmd.store
            .set_map_entry(&mut doc, CHARGEN_POOLS_FIELD, cat, serde_json::json!(n))
            .map_err(|e| CmdError::msg(e.to_string()))?;
    }
    cmd.store
        .put_by(ctx.w, actor, &doc, actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.info(actor, "Priorities set.");
    status(cmd, ctx)
}

/// `+stat <field>[:<key>]=<value>` during chargen.
fn stat_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    require_chargen(ctx)?;
    let lhs = ctx.args.lhs().trim().to_string();
    let value_raw = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: +stat <field>[:<key>]=<value>"))?;
    if lhs.is_empty() {
        return Err(CmdError::msg("Usage: +stat <field>[:<key>]=<value>"));
    }
    let (field, key) = match lhs.split_once(':') {
        Some((f, k)) => (f.trim(), Some(k.trim())),
        None => (lhs.as_str(), None),
    };
    if !chargen_settable(field) {
        return Err(CmdError::msg(format!(
            "{field} is not a chargen-allocatable trait. XP, health, and bookkeeping \
             fields are not set with +stat."
        )));
    }
    let fdef = cmd
        .schema
        .field(field)
        .ok_or_else(|| CmdError::msg(format!("No such sheet field: {field}")))?
        .clone();

    let mut doc = cmd.store.get_or_default(ctx.w, actor);
    let audit = match (&fdef.ty, key) {
        (FieldType::Map { value }, Some(key)) => {
            let coerced = coerce(value, value_raw)
                .map_err(|e| CmdError::msg(format!("{field}:{key}: {e}")))?;
            cmd.store
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
            cmd.store
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
    cmd.store
        .put_by(ctx.w, actor, &doc, actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.info(actor, format!("Set {audit}."));
    status(cmd, ctx)
}

/// `+spec <ability>=<specialty>` — record a specialty on an ability.
fn spec_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let ability = ctx.args.lhs().trim().to_string();
    let specialty = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: +spec <ability>=<specialty>"))?
        .to_string();
    let fdef = cmd
        .schema
        .field(&ability)
        .ok_or_else(|| CmdError::msg(format!("No such ability: {ability}")))?;
    if !matches!(fdef.ty, FieldType::Dots { .. }) {
        return Err(CmdError::msg("Specialties attach to abilities/attributes."));
    }
    let id = fdef.id.to_string();
    let mut doc = cmd.store.get_or_default(ctx.w, actor);
    let mut list: Vec<serde_json::Value> = doc
        .map("specialties")
        .and_then(|m| m.get(&id))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if list
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|s| s.eq_ignore_ascii_case(&specialty))
    {
        return Err(CmdError::msg("That specialty is already recorded."));
    }
    list.push(serde_json::json!(specialty));
    cmd.store
        .set_map_entry(&mut doc, "specialties", &id, serde_json::Value::Array(list))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    cmd.store
        .put_by(ctx.w, actor, &doc, actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out
        .info(actor, format!("Specialty {specialty:?} added to {id}."));
    Ok(())
}

/// Show the live budget ledgers.
fn status(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let pools = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let session = st
            .chargen(actor)
            .ok_or_else(|| CmdError::msg("You are not in chargen. Start with +chargen."))?;
        session_pools(&cmd.config, session)
    };
    let doc = cmd.store.get_or_default(ctx.w, actor);
    let ls = ledgers(&cmd.config, &doc, &pools);
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
    let outstanding = crate::chargen::issues(&cmd.config, &doc, &pools);
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
fn submit(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    // Refuse to re-submit an already-submitted (Pending) or approved sheet:
    // that would orphan the first job and open a second one for one character.
    if let Some(doc) = cmd.store.get(ctx.w, actor) {
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
    let pools = {
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
        .store
        .get(ctx.w, actor)
        .ok_or_else(|| CmdError::msg("You have no sheet to submit."))?;
    crate::chargen::finalize_sheet(&cmd.config, &mut doc, &pools)
        .map_err(|i| CmdError::msg(format!("Cannot submit: {i}")))?;

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
    cmd.store
        .put_by(ctx.w, actor, &doc, actor)
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
    store: &SheetStore,
    ctx: &mut Ctx<'_>,
    job: &Job,
) -> Result<(), String> {
    let target = payload_target(job)?;
    let mut doc = store
        .get(ctx.w, target)
        .ok_or_else(|| "the character has no sheet".to_string())?;
    let pools = persisted_pools(config, &doc);
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
    store
        .put_by(ctx.w, target, &doc, ctx.actor)
        .map_err(|es| fmt_errs(&es))?;
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
fn parse_priority_assignment(
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
                "status".into(),
                "attr".into(),
                "abil".into(),
                "submit".into(),
            ],
            shape: ArgShape::Raw,
            help: "+chargen  --  Begin character generation (priority point-buy).\n\
                   +chargen/attr physical=7 social=5 mental=3  --  Assign attribute \
                   priorities.\n\
                   +chargen/abil talents=13 skills=9 knowledges=5  --  Assign ability \
                   priorities.\n\
                   +chargen/status  --  Show the live budget.\n\
                   +chargen/submit  --  Finalize and file for staff approval."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| chargen_cmd(&cmd, ctx))
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
