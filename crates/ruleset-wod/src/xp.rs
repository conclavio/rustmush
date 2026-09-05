//! Experience: `+xp` (balance + history), `+xp/spend` (file a costed raise for
//! approval), and `+xp/award` (staff grant).
//!
//! A spend validates the oWoD cost from the data-driven [`XpCosts`] table
//! (current rating × multiplier; a brand-new Ability dot is a flat 3) and opens
//! a `wod-xp` job carrying the diff. The payload handler applies the sheet
//! mutation **atomically on approval** — re-checking affordability and that the
//! trait has not moved — so an approved raise never double-spends.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, Role, match_object, perm};
use mush_jobs::{Job, JobPayload, OpenJob, open_job};
use mush_ruleset::SheetDoc;
use mush_ruleset::schema::SheetSchema;

use crate::WodCmd;
use crate::config::{TraitKind, WodConfig};
use crate::schema::{ATTRIBUTES, KNOWLEDGES, SKILLS, TALENTS, VIRTUES};
use crate::splats::{Dispatch, Splat};

/// The `wod-xp` job queue name.
pub const XP_QUEUE: &str = "wod-xp";
/// The XP payload-handler name.
pub const XP_HANDLER: &str = "wod.xp";

/// How a spend target is costed: a base [`TraitKind`] table, or a splat-owned
/// trait the active splat prices (Vampire: Disciplines).
enum CostKind {
    /// A base trait costed from [`crate::config::XpCosts`].
    Base(TraitKind),
    /// A splat-owned trait costed by the active [`Splat`].
    Splat,
}

/// A parsed spend target: a scalar field, a background/discipline map entry.
struct SpendTarget {
    field: String,
    key: Option<String>,
    cost: CostKind,
    max: i32,
}

impl SpendTarget {
    /// The XP cost to raise this target `from`→`to` on `doc` under `splat`.
    fn raise_cost(
        &self,
        config: &WodConfig,
        doc: &SheetDoc,
        splat: &dyn Splat,
        from: i32,
        to: i32,
    ) -> i32 {
        match self.cost {
            CostKind::Base(kind) => config.xp_costs.rule(kind).raise_cost(from, to),
            CostKind::Splat => {
                splat.xp_cost(config, doc, &self.field, self.key.as_deref(), from, to)
            }
        }
    }
}

/// `+xp` / `+xp/spend` / `+xp/award`.
fn xp_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("spend") {
        return spend(cmd, ctx);
    }
    if ctx.switches.has("award") {
        return award(cmd, ctx);
    }
    show(cmd, ctx)
}

/// `+xp [<target>]` — balance and spend history.
fn show(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim();
    let target = if raw.is_empty() {
        ctx.actor
    } else {
        let t = match_object(ctx.w, ctx.actor, raw)?;
        if t != ctx.actor && !perm::can_examine(ctx.w, ctx.actor, t) {
            return Err(CmdError::msg("You can't examine that character's XP."));
        }
        t
    };
    let doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let cur = doc.number("experience").unwrap_or(0);
    let total = doc.number("total_xp").unwrap_or(0);
    let name = name_of(ctx, target);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("{name}: {cur} XP available ({total} earned all-time)."),
    );
    if let Some(log) = doc.list("xp_log")
        && !log.is_empty()
    {
        ctx.out.sys_to_actor(ctx.actor, "wod", "History:");
        for entry in log.iter().rev().take(10) {
            if let Some(s) = entry.as_str() {
                ctx.out.sys_to_actor(ctx.actor, "wod", format!("  {s}"));
            }
        }
    }
    Ok(())
}

/// `+xp/spend <field>[:<key>]=<new-rating>`.
fn spend(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let lhs = ctx.args.lhs().trim().to_string();
    let to: i32 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .ok_or_else(|| CmdError::msg("Usage: +xp/spend <field>[:<key>]=<new-rating>"))?;

    let doc = cmd
        .read(ctx.w, actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))?;
    let splat = cmd.splat_for_doc(&doc);
    let schema = cmd.schema_for_doc(&doc);
    let target = resolve_spend_target(&schema, splat.as_ref(), &doc, &lhs)?;

    let from = current_rating(&doc, &target);
    if to <= from {
        return Err(CmdError::msg(format!(
            "{} is already at {from}; pick a higher rating.",
            display_field(&target)
        )));
    }
    if to > target.max {
        return Err(CmdError::msg(format!(
            "{} caps at {}.",
            display_field(&target),
            target.max
        )));
    }
    let cost = target.raise_cost(&cmd.config, &doc, splat.as_ref(), from, to);
    let have = doc.number("experience").unwrap_or(0) as i32;
    if cost > have {
        return Err(CmdError::msg(format!(
            "Raising {} {from}→{to} costs {cost} XP; you have {have}.",
            display_field(&target)
        )));
    }

    let job = open_job(
        ctx,
        OpenJob {
            queue: XP_QUEUE.into(),
            title: format!(
                "XP: {} {}→{} ({cost}xp) for {}",
                display_field(&target),
                from,
                to,
                name_of(ctx, actor)
            ),
            opener: actor,
            text: Some(format!(
                "Raise {} from {from} to {to} for {cost} XP.",
                display_field(&target)
            )),
            payload: Some(JobPayload {
                handler: XP_HANDLER.into(),
                data: serde_json::json!({
                    "target": actor.0,
                    "field": target.field,
                    "key": target.key,
                    "from": from,
                    "to": to,
                    "cost": cost,
                }),
            }),
            tags: vec!["xp".into()],
        },
    )
    .map_err(CmdError::msg)?;
    ctx.out.info(
        actor,
        format!(
            "Filed job {job}: {} {from}→{to} for {cost} XP (pending approval).",
            display_field(&target)
        ),
    );
    Ok(())
}

/// `+xp/award <target>=<n>` (staff).
fn award(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !perm::has_role(ctx.w, ctx.actor, Role::Royalty) {
        return Err(CmdError::msg("Permission denied."));
    }
    let target_spec = ctx.args.lhs().trim();
    let n: i32 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| CmdError::msg("Usage: +xp/award <target>=<n> (n > 0)"))?;
    let target = match_object(ctx.w, ctx.actor, target_spec)?;
    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let cur = doc.number("experience").unwrap_or(0) as i32;
    let total = doc.number("total_xp").unwrap_or(0) as i32;
    doc.set_number("experience", i64::from(cur + n));
    doc.set_number("total_xp", i64::from(total + n));
    push_log(
        &mut doc,
        format!("+{n} XP awarded by {}", name_of(ctx, ctx.actor)),
    );
    cmd.store_for_doc(&doc)
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.info(
        ctx.actor,
        format!("Awarded {n} XP to {}.", name_of(ctx, target)),
    );
    Ok(())
}

/// The `wod.xp` payload handler: re-validate and apply an approved raise.
pub(crate) fn xp_approval(
    config: &WodConfig,
    dispatch: &Dispatch,
    ctx: &mut Ctx<'_>,
    job: &Job,
) -> Result<(), String> {
    let data = job
        .payload
        .as_ref()
        .map(|p| &p.data)
        .ok_or_else(|| "job has no payload".to_string())?;
    let target = data
        .get("target")
        .and_then(serde_json::Value::as_u64)
        .map(|n| ObjId(n as u32))
        .ok_or_else(|| "payload has no target".to_string())?;
    let field = data
        .get("field")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "payload has no field".to_string())?
        .to_string();
    let key = data
        .get("key")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let from = data
        .get("from")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as i32;
    let to = data
        .get("to")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as i32;
    let cost = data
        .get("cost")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as i32;

    let mut doc = dispatch
        .read(ctx.w, target)
        .ok_or_else(|| "the character has no sheet".to_string())?;
    let splat = dispatch.splats.for_doc(&doc);
    let schema = dispatch.schemas.schema_for_doc(&doc);
    let spend_target =
        resolve_spend_target_from(&schema, splat.as_ref(), &doc, &field, key.as_deref())
            .ok_or_else(|| format!("unknown trait {field:?}"))?;

    // Re-check: the trait has not moved and the recomputed cost is still funded.
    let now_from = current_rating(&doc, &spend_target);
    if now_from != from {
        return Err(format!(
            "{} is now at {now_from}, not {from}; re-file the spend.",
            display_field(&spend_target)
        ));
    }
    // Re-check the (possibly sheet-dependent) cap: a Sphere spend filed while
    // Arete was higher must not apply above the current Arete.
    if to > spend_target.max {
        return Err(format!(
            "{} now caps at {}; re-file the spend.",
            display_field(&spend_target),
            spend_target.max
        ));
    }
    let recost = spend_target.raise_cost(config, &doc, splat.as_ref(), from, to);
    if recost != cost {
        return Err("the XP cost changed; re-file the spend.".to_string());
    }
    let have = doc.number("experience").unwrap_or(0) as i32;
    if cost > have {
        return Err(format!("insufficient XP ({have} < {cost})."));
    }

    apply_rating(&mut doc, &spend_target, to);
    doc.set_number("experience", i64::from(have - cost));
    push_log(
        &mut doc,
        format!(
            "-{cost} XP: {} {from}→{to} (job {})",
            display_field(&spend_target),
            job.id
        ),
    );
    dispatch
        .store_for_doc(&doc)
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| fmt_errs(&es))?;
    Ok(())
}

/// Resolve a spend target from a `+xp/spend` LHS against the active schema +
/// splat, applying any sheet-dependent cap (Mage: a Sphere is capped at Arete).
fn resolve_spend_target(
    schema: &SheetSchema,
    splat: &dyn Splat,
    doc: &SheetDoc,
    lhs: &str,
) -> Result<SpendTarget, CmdError> {
    let (field, key) = match lhs.split_once(':') {
        Some((f, k)) => (f.trim(), Some(k.trim())),
        None => (lhs.trim(), None),
    };
    resolve_spend_target_from(schema, splat, doc, field, key)
        .ok_or_else(|| CmdError::msg(format!("Cannot buy {field:?} with XP.")))
}

/// Classify a field into a costed spend target (returns `None` for a field XP
/// cannot buy).
fn resolve_spend_target_from(
    schema: &SheetSchema,
    splat: &dyn Splat,
    doc: &SheetDoc,
    field: &str,
    key: Option<&str>,
) -> Option<SpendTarget> {
    let f = field.to_ascii_lowercase();
    // Splat-owned traits first (Vampire: Disciplines).
    if let Some(max) = splat.xp_max(&f, key) {
        // A sheet-dependent cap (Mage: Sphere ≤ Arete) narrows the flat max.
        let max = splat
            .xp_target_cap(doc, &f, key)
            .map_or(max, |cap| cap.min(max));
        return Some(SpendTarget {
            field: f,
            key: key.map(str::to_string),
            cost: CostKind::Splat,
            max,
        });
    }
    // Backgrounds: a map entry.
    if f == "backgrounds" {
        let key = key?;
        return Some(SpendTarget {
            field: "backgrounds".to_string(),
            key: Some(key.to_string()),
            cost: CostKind::Base(TraitKind::Background),
            max: 5,
        });
    }
    // Otherwise a scalar field on the active schema.
    let fdef = schema.field(&f)?;
    let id = fdef.id.to_string();
    let kind = trait_kind(&id)?;
    let max = match kind {
        TraitKind::Willpower => 10,
        _ => 5,
    };
    Some(SpendTarget {
        field: id,
        key: None,
        cost: CostKind::Base(kind),
        max,
    })
}

/// The [`TraitKind`] of a scalar field id (or `None` if XP cannot buy it).
fn trait_kind(id: &str) -> Option<TraitKind> {
    if ATTRIBUTES.iter().any(|(a, _, _)| *a == id) {
        Some(TraitKind::Attribute)
    } else if TALENTS
        .iter()
        .chain(SKILLS)
        .chain(KNOWLEDGES)
        .any(|(a, _)| *a == id)
    {
        Some(TraitKind::Ability)
    } else if id == "willpower" {
        Some(TraitKind::Willpower)
    } else if VIRTUES.iter().any(|(a, _)| *a == id) {
        Some(TraitKind::Virtue)
    } else {
        None
    }
}

/// The current rating of a spend target on `doc`.
fn current_rating(doc: &SheetDoc, target: &SpendTarget) -> i32 {
    match &target.key {
        Some(key) => doc
            .map(&target.field)
            .and_then(|m| m.get(key))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32,
        None => doc.number(&target.field).unwrap_or(0) as i32,
    }
}

/// Apply a new rating to a spend target on `doc`.
fn apply_rating(doc: &mut SheetDoc, target: &SpendTarget, to: i32) {
    match &target.key {
        Some(key) => doc.map_set(&target.field, key, serde_json::json!(to)),
        None => doc.set_number(&target.field, i64::from(to)),
    }
}

fn display_field(t: &SpendTarget) -> String {
    match &t.key {
        Some(k) => format!("{}:{k}", t.field),
        None => t.field.clone(),
    }
}

/// Append a line to the sheet's XP log (bounded at 100 entries).
fn push_log(doc: &mut SheetDoc, line: String) {
    let mut log: Vec<serde_json::Value> = doc.list("xp_log").cloned().unwrap_or_default();
    log.push(serde_json::json!(line));
    let overflow = log.len().saturating_sub(100);
    if overflow > 0 {
        log.drain(0..overflow);
    }
    doc.set("xp_log", serde_json::Value::Array(log));
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

/// Register the XP command.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    cmd: Arc<WodCmd>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+xp".into(),
            switches: vec!["spend".into(), "award".into()],
            shape: ArgShape::EqPair,
            help: "+xp [<target>]  --  Show XP balance and spend history.\n\
                   +xp/spend <field>[:<key>]=<new-rating>  --  File a costed raise for \
                   approval (e.g. +xp/spend brawl=3, +xp/spend backgrounds:Resources=2). \
                   Applied atomically when a storyteller approves the job.\n\
                   +xp/award <target>=<n>  --  Grant XP (staff)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| xp_cmd(&cmd, ctx))
        },
    )
}
