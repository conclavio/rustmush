//! Karma — the SR5 advancement currency (the XP analog): `+karma` (balance +
//! history), `+karma/spend` (file a costed raise for approval), and
//! `+karma/award` (gamemaster grant).
//!
//! A spend validates the SR5 cost from [`KarmaCosts`](crate::data::KarmaCosts)
//! (new-rating × multiplier; a new spell/complex form is a flat cost) and opens
//! a `shadowrun-karma` job carrying the diff. The payload handler applies the
//! sheet mutation **atomically on approval** — re-checking the current rating
//! and the recomputed cost — so an approved raise never double-spends.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, Role, match_object, perm};
use mush_jobs::{Job, JobPayload, OpenJob, open_job};
use mush_ruleset::SheetDoc;

use crate::Sr5Cmd;
use crate::data::{self, Sr5Config};
use crate::schema::{
    ACTIVE_SKILLS_FIELD, KNOWLEDGE_SKILLS_FIELD, LANGUAGE_SKILLS_FIELD, MAGIC, RESONANCE,
    SKILL_GROUPS_FIELD,
};

/// The `shadowrun-karma` job queue name.
pub const KARMA_QUEUE: &str = "shadowrun-karma";
/// The karma payload-handler name.
pub const KARMA_HANDLER: &str = "shadowrun.karma";

/// How a spend target is costed.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// A rated trait: `new × mult`.
    Rated(u32),
    /// A new spell (flat cost).
    Spell,
    /// A new complex form (flat cost).
    ComplexForm,
}

/// A parsed spend target.
struct SpendTarget {
    /// The sheet field.
    field: String,
    /// A map key (for skills) or a list value (for spells/forms).
    key: Option<String>,
    kind: Kind,
    /// The max rating (for rated traits).
    max: i32,
}

impl SpendTarget {
    /// The current rating (`0` for a not-yet-known spell/form).
    fn current(&self, doc: &SheetDoc) -> i32 {
        match self.kind {
            Kind::Rated(_) => match &self.key {
                Some(k) => doc
                    .map(&self.field)
                    .and_then(|m| m.get(k))
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0) as i32,
                None => doc.number(&self.field).unwrap_or(0) as i32,
            },
            Kind::Spell | Kind::ComplexForm => {
                let name = self.key.clone().unwrap_or_default();
                let known = doc
                    .list(&self.field)
                    .map(|l| {
                        l.iter()
                            .filter_map(serde_json::Value::as_str)
                            .any(|s| s.eq_ignore_ascii_case(&name))
                    })
                    .unwrap_or(false);
                i32::from(known)
            }
        }
    }

    /// The Karma cost to raise `from`→`to`.
    fn cost(&self, config: &Sr5Config, from: i32, to: i32) -> i32 {
        match self.kind {
            Kind::Rated(mult) => {
                data::KarmaCosts::raise_cost(mult, from.max(0) as u32, to.max(0) as u32) as i32
            }
            Kind::Spell => config.karma.spell as i32,
            Kind::ComplexForm => config.karma.complex_form as i32,
        }
    }

    /// Display label.
    fn label(&self) -> String {
        match &self.key {
            Some(k) => format!("{}:{k}", self.field),
            None => self.field.clone(),
        }
    }
}

/// `+karma` / `+karma/spend` / `+karma/award`.
fn karma_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("spend") {
        return spend(cmd, ctx);
    }
    if ctx.switches.has("award") {
        return award(cmd, ctx);
    }
    show(cmd, ctx)
}

/// `+karma [<target>]` — balance + history.
fn show(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim();
    let target = if raw.is_empty() {
        ctx.actor
    } else {
        let t = match_object(ctx.w, ctx.actor, raw)?;
        if t != ctx.actor && !perm::can_examine(ctx.w, ctx.actor, t) {
            return Err(CmdError::msg("You can't examine that character's Karma."));
        }
        t
    };
    let doc = cmd
        .read_ctx(ctx, target)?
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let cur = doc.number("karma").unwrap_or(0);
    let total = doc.number("karma_total").unwrap_or(0);
    let name = name_of(ctx, target);
    ctx.out.sys_to_actor(
        ctx.actor,
        "sr",
        format!("{name}: {cur} Karma available ({total} earned all-time)."),
    );
    if let Some(log) = doc.list("karma_log")
        && !log.is_empty()
    {
        ctx.out.sys_to_actor(ctx.actor, "sr", "History:");
        for entry in log.iter().rev().take(10) {
            if let Some(s) = entry.as_str() {
                ctx.out.sys_to_actor(ctx.actor, "sr", format!("  {s}"));
            }
        }
    }
    Ok(())
}

/// `+karma/spend <field>[:<key>]=<new-rating|1>`.
fn spend(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let lhs = ctx.args.lhs().trim().to_string();
    let to: i32 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .ok_or_else(|| CmdError::msg("Usage: +karma/spend <field>[:<key>]=<new-rating>"))?;

    let doc = cmd
        .read_ctx(ctx, actor)?
        .ok_or_else(|| CmdError::msg("You have no sheet."))?;
    let target = resolve_target(&doc, &lhs)?;
    let from = target.current(&doc);
    if to <= from {
        return Err(CmdError::msg(format!(
            "{} is already at {from}; pick a higher rating.",
            target.label()
        )));
    }
    if matches!(target.kind, Kind::Rated(_)) && to > target.max {
        return Err(CmdError::msg(format!(
            "{} caps at {}.",
            target.label(),
            target.max
        )));
    }
    let cost = target.cost(&cmd.config, from, to);
    let have = doc.number("karma").unwrap_or(0) as i32;
    if cost > have {
        return Err(CmdError::msg(format!(
            "Raising {} {from}→{to} costs {cost} Karma; you have {have}.",
            target.label()
        )));
    }
    let job = open_job(
        ctx,
        OpenJob {
            queue: KARMA_QUEUE.into(),
            title: format!(
                "Karma: {} {from}→{to} ({cost}) for {}",
                target.label(),
                name_of(ctx, actor)
            ),
            opener: actor,
            text: Some(format!(
                "Raise {} from {from} to {to} for {cost} Karma.",
                target.label()
            )),
            payload: Some(JobPayload {
                handler: KARMA_HANDLER.into(),
                data: serde_json::json!({
                    "target": actor.0,
                    "field": target.field,
                    "key": target.key,
                    "from": from,
                    "to": to,
                    "cost": cost,
                }),
            }),
            tags: vec!["karma".into()],
        },
    )
    .map_err(CmdError::msg)?;
    ctx.out.info(
        actor,
        format!(
            "Filed job {job}: {} {from}→{to} for {cost} Karma (pending approval).",
            target.label()
        ),
    );
    Ok(())
}

/// `+karma/award <target>=<n>` (gamemaster).
fn award(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !perm::has_role(ctx.w, ctx.actor, Role::Royalty) {
        return Err(CmdError::msg("Permission denied."));
    }
    let target_spec = ctx.args.lhs().trim();
    let n: i32 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| CmdError::msg("Usage: +karma/award <target>=<n> (n > 0)"))?;
    let target = match_object(ctx.w, ctx.actor, target_spec)?;
    let mut doc = cmd
        .read_ctx(ctx, target)?
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let cur = doc.number("karma").unwrap_or(0) as i32;
    let total = doc.number("karma_total").unwrap_or(0) as i32;
    doc.set_number("karma", i64::from(cur + n));
    doc.set_number("karma_total", i64::from(total + n));
    push_log(
        &mut doc,
        format!("+{n} Karma awarded by {}", name_of(ctx, ctx.actor)),
    );
    cmd.store()
        .put_by(ctx.w, target, &doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.info(
        ctx.actor,
        format!("Awarded {n} Karma to {}.", name_of(ctx, target)),
    );
    Ok(())
}

/// The `shadowrun.karma` payload handler: re-validate and apply an approved
/// raise atomically.
pub(crate) fn karma_approval(
    config: &Sr5Config,
    cmd: &Sr5Cmd,
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

    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| "the character has no sheet".to_string())?;
    let lhs = match &key {
        Some(k) => format!("{field}:{k}"),
        None => field.clone(),
    };
    let target_t = resolve_target(&doc, &lhs).map_err(|e| e.to_string())?;
    let now_from = target_t.current(&doc);
    if now_from != from {
        return Err(format!(
            "{} is now at {now_from}, not {from}; re-file the spend.",
            target_t.label()
        ));
    }
    let recost = target_t.cost(config, from, to);
    if recost != cost {
        return Err("the Karma cost changed; re-file the spend.".to_string());
    }
    let have = doc.number("karma").unwrap_or(0) as i32;
    if cost > have {
        return Err(format!("insufficient Karma ({have} < {cost})."));
    }

    apply(&mut doc, &target_t, to);
    doc.set_number("karma", i64::from(have - cost));
    push_log(
        &mut doc,
        format!(
            "-{cost} Karma: {} {from}→{to} (job {})",
            target_t.label(),
            job.id
        ),
    );
    // A raised attribute changes derived stats.
    crate::derive::apply_derived(&mut doc);
    cmd.store()
        .put_by(ctx.w, target, &doc, ctx.actor)
        .map_err(|es| fmt_errs(&es))?;
    Ok(())
}

/// Apply the raise / learn to `doc`.
fn apply(doc: &mut SheetDoc, target: &SpendTarget, to: i32) {
    match target.kind {
        Kind::Rated(_) => match &target.key {
            Some(k) => doc.map_set(&target.field, k, serde_json::json!(to)),
            None => doc.set_number(&target.field, i64::from(to)),
        },
        Kind::Spell | Kind::ComplexForm => {
            let name = target.key.clone().unwrap_or_default();
            let mut list: Vec<serde_json::Value> =
                doc.list(&target.field).cloned().unwrap_or_default();
            if !list
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|s| s.eq_ignore_ascii_case(&name))
            {
                list.push(serde_json::json!(name));
            }
            doc.set(&target.field, serde_json::Value::Array(list));
        }
    }
}

/// Resolve a `+karma/spend` LHS into a costed target.
fn resolve_target(doc: &SheetDoc, lhs: &str) -> Result<SpendTarget, CmdError> {
    let (field, key) = match lhs.split_once(':') {
        Some((f, k)) => (f.trim().to_ascii_lowercase(), Some(k.trim().to_string())),
        None => (lhs.trim().to_ascii_lowercase(), None),
    };
    let costs = data::KarmaCosts::default();

    // Learning a new spell / complex form.
    if field == "spell" || field == "spells" {
        let name = key.ok_or_else(|| CmdError::msg("Usage: +karma/spend spell:<name>=1"))?;
        return Ok(SpendTarget {
            field: "spells".to_string(),
            key: Some(name),
            kind: Kind::Spell,
            max: 1,
        });
    }
    if field == "complexform" || field == "complex_forms" {
        let name = key.ok_or_else(|| CmdError::msg("Usage: +karma/spend complexform:<name>=1"))?;
        return Ok(SpendTarget {
            field: "complex_forms".to_string(),
            key: Some(name),
            kind: Kind::ComplexForm,
            max: 1,
        });
    }

    // Map-based skills.
    if let Some(k) = &key {
        let (kind, max) = match field.as_str() {
            ACTIVE_SKILLS_FIELD => (Kind::Rated(costs.active_skill_mult), 12),
            SKILL_GROUPS_FIELD => (Kind::Rated(costs.skill_group_mult), 12),
            KNOWLEDGE_SKILLS_FIELD | LANGUAGE_SKILLS_FIELD => {
                (Kind::Rated(costs.knowledge_skill_mult), 12)
            }
            other => {
                return Err(CmdError::msg(format!("Cannot buy {other}:{k} with Karma.")));
            }
        };
        return Ok(SpendTarget {
            field,
            key,
            kind,
            max,
        });
    }

    // Scalar attributes + special attributes.
    if data::CORE_ATTRS.iter().any(|(id, _, _)| *id == field) {
        // Max = metatype natural max.
        let mt =
            data::metatype(doc.text("metatype").unwrap_or("human")).unwrap_or(&data::METATYPES[0]);
        let max = mt
            .attr_range(&field)
            .map(|(_, hi)| i32::from(hi))
            .unwrap_or(6);
        return Ok(SpendTarget {
            field,
            key: None,
            kind: Kind::Rated(costs.attribute_mult),
            max,
        });
    }
    if field == "edge" {
        let mt =
            data::metatype(doc.text("metatype").unwrap_or("human")).unwrap_or(&data::METATYPES[0]);
        return Ok(SpendTarget {
            field,
            key: None,
            kind: Kind::Rated(costs.special_attribute_mult),
            max: i32::from(mt.edge_range.1),
        });
    }
    if field == MAGIC || field == RESONANCE {
        return Ok(SpendTarget {
            field,
            key: None,
            kind: Kind::Rated(costs.special_attribute_mult),
            max: 12,
        });
    }
    Err(CmdError::msg(format!("Cannot buy {field:?} with Karma.")))
}

/// Append a line to the Karma log (bounded at 100 entries).
fn push_log(doc: &mut SheetDoc, line: String) {
    let mut log: Vec<serde_json::Value> = doc.list("karma_log").cloned().unwrap_or_default();
    log.push(serde_json::json!(line));
    let overflow = log.len().saturating_sub(100);
    if overflow > 0 {
        log.drain(0..overflow);
    }
    doc.set("karma_log", serde_json::Value::Array(log));
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

/// Register `+karma`.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    cmd: Arc<Sr5Cmd>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+karma".into(),
            switches: vec!["spend".into(), "award".into()],
            shape: ArgShape::EqPair,
            help: "+karma [<target>]  --  Show Karma balance and history.\n\
                   +karma/spend <field>[:<key>]=<new-rating>  --  File a costed raise \
                   for approval (e.g. +karma/spend body=5, +karma/spend \
                   active_skills:pistols=5, +karma/spend spell:Fireball=1). Applied \
                   atomically when a gamemaster approves the job.\n\
                   +karma/award <target>=<n>  --  Grant Karma (gamemaster)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| karma_cmd(&cmd, ctx))
        },
    )
}
