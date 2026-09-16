//! Character generation: three stages, one six-point pool, one approval job.
//!
//! The whole flow is `+chargen` (open or show), `+stat <field>=<value>` (spend),
//! `+chargen/submit` (file for approval) — and then a storyteller's
//! `+job/approve`, whose payload handler [`chargen_approval`] re-validates the
//! budget before it stamps the sheet.
//!
//! What the WoD crate had here and this does not: the guided wizard, the
//! attribute/ability priority tiers, the eleven templates, specialties, freebie
//! points, and the transient chargen *session*. There is no session because
//! there is nothing to keep in one: the in-progress sheet **is** the state (a
//! normal [`ApprovalState::Unapproved`] [`SheetDoc`]), so a dropped link or a hot
//! restart loses nothing, and `+stat` needs no "you are not in chargen" refusal
//! — an unapproved sheet is the permission.
//!
//! # The budget
//!
//! One pool, [`RATING_POINTS`] points, spent one-for-one on dots above the free
//! [`RATING_MIN`](super::schema::RATING_MIN) each rating starts at, capped at
//! [`RATING_MAX`](super::schema::RATING_MAX). `5/2/1` and `3/3/3` both spend it
//! exactly; `5/5/1` is refused at the keystroke, not at submit.
//!
//! [`ApprovalState::Unapproved`]: mush_ruleset::ApprovalState::Unapproved

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar};
use mush_jobs::{Job, JobPayload, OpenJob, open_job};
use mush_ruleset::chargen::{
    BudgetLedger, BudgetPlan, ChargenFlow, ChargenIssue, ChargenStage, CostRule, FieldSelector,
    PoolDef,
};
use mush_ruleset::schema::{FieldType, SheetSchema};
use mush_ruleset::{ApprovalState, SheetDoc, SheetError, SheetStore};
use smol_str::SmolStr;

use crate::sheet::schema::{FIELD_CALLSIGN, FIELD_NAME, RATING_MAX, RATING_MIN, RATINGS};
use crate::sheet::{NS, onboarding};

/// The approval queue's name (`+jobs pilot-chargen`).
pub const CHARGEN_QUEUE: &str = "pilot-chargen";
/// The payload-handler name `+job/approve` dispatches.
pub const CHARGEN_HANDLER: &str = "pilot.chargen";

/// The one budget pool's id.
pub const POOL_RATINGS: &str = "ratings";
/// How many dots a new pilot distributes above the free one in each rating.
pub const RATING_POINTS: i32 = 6;

/// The `ratings` stage id.
pub const STAGE_RATINGS: &str = "ratings";
/// The `identity` stage id.
pub const STAGE_IDENTITY: &str = "identity";
/// The `review` stage id.
pub const STAGE_REVIEW: &str = "review";

/// The one spend plan: [`RATING_POINTS`] points over the three ratings.
#[must_use]
pub fn ratings_budget() -> BudgetPlan {
    BudgetPlan::new()
        .pool(PoolDef::new(POOL_RATINGS, "Pilot ratings", RATING_POINTS))
        .rule(
            CostRule::new(
                POOL_RATINGS,
                FieldSelector::Fields(RATINGS.iter().map(|r| SmolStr::new(*r)).collect()),
                i32::from(RATING_MIN),
                1,
            )
            .with_cap(i32::from(RATING_MAX)),
        )
}

/// The game's chargen flow.
pub struct TacsimChargen {
    schema: Arc<SheetSchema>,
}

impl TacsimChargen {
    /// A flow over `schema`.
    #[must_use]
    pub fn new(schema: Arc<SheetSchema>) -> TacsimChargen {
        TacsimChargen { schema }
    }

    /// The live pool ledger for `doc` — what `+chargen/status` prints.
    #[must_use]
    pub fn ledgers(&self, doc: &SheetDoc) -> Vec<BudgetLedger> {
        ratings_budget().assess(&self.schema, doc)
    }

    /// Every issue blocking submission, in stage order: the budget first, then
    /// identity. Empty means the sheet is ready to file.
    #[must_use]
    pub fn issues(&self, doc: &SheetDoc) -> Vec<ChargenIssue> {
        let mut issues = ratings_budget().issues(&self.schema, doc);
        issues.extend(identity_issues(doc));
        issues
    }
}

/// `name` and `callsign` must both carry something: a match ledger with a blank
/// callsign in it is a bug report, and the cockpit prints one every pulse.
fn identity_issues(doc: &SheetDoc) -> Vec<ChargenIssue> {
    let mut issues = Vec::new();
    for (field, label) in [(FIELD_NAME, "name"), (FIELD_CALLSIGN, "callsign")] {
        if doc.text(field).unwrap_or("").trim().is_empty() {
            issues.push(ChargenIssue::msg(format!(
                "your {label} is empty (+stat {field}=<text>)"
            )));
        }
    }
    issues
}

impl ChargenFlow for TacsimChargen {
    fn stages(&self) -> Vec<ChargenStage> {
        vec![
            ChargenStage::new(
                STAGE_RATINGS,
                "Pilot ratings",
                format!(
                    "Spend {RATING_POINTS} points across gunnery, piloting and sensors. \
                     Each starts at {RATING_MIN} and none may pass {RATING_MAX}. \
                     +stat gunnery=3"
                ),
            )
            .with_budget(ratings_budget()),
            ChargenStage::new(
                STAGE_IDENTITY,
                "Identity",
                "Your name, your callsign, and a line of concept. \
                 +stat callsign=Ladder",
            ),
            ChargenStage::new(
                STAGE_REVIEW,
                "Review",
                "+chargen/status shows what is left; +chargen/submit files it for approval.",
            ),
        ]
    }

    fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue> {
        match stage.as_str() {
            STAGE_RATINGS => ratings_budget().issues(&self.schema, sheet),
            STAGE_IDENTITY => identity_issues(sheet),
            STAGE_REVIEW => self.issues(sheet),
            _ => Vec::new(),
        }
    }

    /// Exactly spent, and named. The budget's own
    /// [`Underspent`](ChargenIssue::Underspent) issue is what makes "exactly"
    /// rather than "at most" the rule.
    fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue> {
        match self.issues(sheet).into_iter().next() {
            Some(issue) => Err(issue),
            None => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// The command surface
// ---------------------------------------------------------------------------

/// `+chargen`, `+chargen/status`, `+chargen/submit` and `+stat`.
///
/// # Errors
///
/// [`ModError`] if a command name collides with one already registered.
pub fn register(reg: &mut Registrar<'_>, schema: Arc<SheetSchema>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+chargen".into(),
            switches: vec!["status".into(), "submit".into()],
            shape: ArgShape::Raw,
            help: "+chargen  --  Start (or resume) character generation: you get an \
                   unapproved sheet and the live budget.\n\
                   +chargen/status  --  Show the budget and anything still blocking \
                   submission.\n\
                   +chargen/submit  --  File the sheet for staff approval."
                .into(),
            ..CommandSpec::default()
        },
        {
            let schema = Arc::clone(&schema);
            Arc::new(move |ctx: &mut Ctx<'_>| {
                if ctx.switches.has("submit") {
                    submit(&schema, ctx)
                } else if ctx.switches.has("status") {
                    status(&schema, ctx)
                } else {
                    start(&schema, ctx)
                }
            })
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+stat".into(),
            shape: ArgShape::EqPair,
            help: "+stat <field>=<value>  --  Set a field on your own unapproved sheet, \
                   with live budget feedback (+stat gunnery=3, +stat callsign=Ladder). \
                   A spend that would overrun the pool is refused."
                .into(),
            ..CommandSpec::default()
        },
        {
            let schema = Arc::clone(&schema);
            Arc::new(move |ctx: &mut Ctx<'_>| stat(&schema, ctx))
        },
    )?;
    Ok(())
}

/// Read the actor's sheet, refusing once it is out of their hands.
fn editable(store: &SheetStore, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    let doc = store.get_or_default(ctx.w, ctx.actor);
    match doc.approved {
        ApprovalState::Approved { .. } => Err(CmdError::msg("Your sheet is already approved.")),
        // A submitted sheet is locked: re-opening it would let `+stat` walk past
        // the budget and then be rubber-stamped by the job that is already open.
        ApprovalState::Pending(job) => Err(CmdError::msg(format!(
            "Your sheet is awaiting approval (job {job}); it is locked until a \
             storyteller resolves it."
        ))),
        ApprovalState::Unapproved => Ok(doc),
    }
}

/// Bare `+chargen` — open (or resume) chargen: ensure an Unapproved sheet
/// exists, then print the budget.
fn start(schema: &Arc<SheetSchema>, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let store = SheetStore::new(Arc::clone(schema));
    let actor = ctx.actor;
    let doc = editable(&store, ctx)?;
    if store.get(ctx.w, actor).is_none() {
        store
            .put_in(actor, &doc, actor, ctx)
            .map_err(|errors| CmdError::msg(format_errors(&errors)))?;
        ctx.out.sys_to_actor(
            actor,
            NS,
            "A blank pilot sheet is yours. Spend your ratings with +stat, then \
             +chargen/submit.",
        );
    }
    show_status(schema, ctx, &doc);
    Ok(())
}

/// `+chargen/status` — the budget, without conjuring a sheet for someone who
/// has not started one. An approved or submitted sheet says so rather than
/// printing a budget nobody can act on.
fn status(schema: &Arc<SheetSchema>, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let store = SheetStore::new(Arc::clone(schema));
    let doc = editable(&store, ctx)?;
    if store.get(ctx.w, ctx.actor).is_none() {
        return Err(CmdError::msg(
            "You have not started a character yet. Type +chargen.",
        ));
    }
    show_status(schema, ctx, &doc);
    Ok(())
}

/// Print the pool ledger and every outstanding issue.
fn show_status(schema: &Arc<SheetSchema>, ctx: &mut Ctx<'_>, doc: &SheetDoc) {
    let flow = TacsimChargen::new(Arc::clone(schema));
    let actor = ctx.actor;
    for ledger in flow.ledgers(doc) {
        ctx.out.sys_to_actor(
            actor,
            NS,
            format!(
                "{}: {} of {} spent, {} left.",
                ledger.label,
                ledger.spent,
                ledger.total,
                ledger.remaining()
            ),
        );
    }
    let dots: Vec<String> = RATINGS
        .iter()
        .map(|r| format!("{r} {}", doc.number(r).unwrap_or(i64::from(RATING_MIN))))
        .collect();
    ctx.out.sys_to_actor(actor, NS, dots.join("  "));
    let issues = flow.issues(doc);
    if issues.is_empty() {
        ctx.out
            .sys_to_actor(actor, NS, "Ready: +chargen/submit files it for approval.");
    } else {
        for issue in issues {
            ctx.out.sys_to_actor(actor, NS, format!("- {issue}"));
        }
    }
}

/// `+stat <field>=<value>` on the actor's own unapproved sheet.
fn stat(schema: &Arc<SheetSchema>, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let store = SheetStore::new(Arc::clone(schema));
    let actor = ctx.actor;
    let field_spec = ctx.args.lhs().trim().to_string();
    let value_raw = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: +stat <field>=<value>"))?
        .to_string();
    if field_spec.is_empty() {
        return Err(CmdError::msg("Usage: +stat <field>=<value>"));
    }
    let mut doc = editable(&store, ctx)?;
    // Canonicalise to the schema's own id before any gate reads it: the
    // staff-only test below is exact, and `+stat REPUTATION=5` must not walk
    // past a check written in lowercase.
    let field = schema
        .field(&field_spec)
        .ok_or_else(|| CmdError::msg(format!("No such sheet field: {field_spec}")))?
        .clone();
    if field.staff_only {
        return Err(CmdError::msg(format!(
            "{} is set by staff, not in chargen.",
            field.id
        )));
    }
    let coerced = coerce(&field.ty, &value_raw)
        .map_err(|detail| CmdError::msg(format!("{}: {detail}", field.id)))?;
    store
        .set_field(&mut doc, field.id.as_str(), coerced)
        .map_err(|error| CmdError::msg(error.to_string()))?;
    // Refuse the overspend at the keystroke rather than at submit: a player who
    // types `+stat gunnery=5` twice should learn it on the second one.
    if let Some(over) = ratings_budget()
        .issues(schema, &doc)
        .into_iter()
        .find(|issue| matches!(issue, ChargenIssue::Overspent { .. }))
    {
        return Err(CmdError::msg(format!("Not enough points: {over}.")));
    }
    store
        .put_in(actor, &doc, actor, ctx)
        .map_err(|errors| CmdError::msg(format_errors(&errors)))?;
    ctx.out.sys_to_actor(
        actor,
        NS,
        format!("Set {} = {}.", field.id, value_raw.trim()),
    );
    show_status(schema, ctx, &doc);
    Ok(())
}

/// `+chargen/submit` — finalize, open the approval job, flip to Pending.
fn submit(schema: &Arc<SheetSchema>, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let store = SheetStore::new(Arc::clone(schema));
    let actor = ctx.actor;
    let mut doc = editable(&store, ctx)?;
    if store.get(ctx.w, actor).is_none() {
        return Err(CmdError::msg(
            "You have no sheet to submit. Start +chargen.",
        ));
    }
    TacsimChargen::new(Arc::clone(schema))
        .finalize(&mut doc)
        .map_err(|issue| CmdError::msg(format!("Cannot submit: {issue}")))?;
    // Validate the exact document before allocating a job: `open_job` mutates the
    // jobs cache and queues its write-through ops, so a sheet that fails
    // validation afterwards would leave an orphan ticket behind.
    mush_ruleset::validate(store.schema(), &doc.data)
        .map_err(|errors| CmdError::msg(format_errors(&errors)))?;
    let job = open_job(
        ctx,
        OpenJob {
            queue: CHARGEN_QUEUE.into(),
            title: format!("Pilot approval: {}", name_of(ctx, actor)),
            opener: actor,
            text: Some("Pilot submitted for approval.".to_string()),
            payload: Some(JobPayload {
                handler: CHARGEN_HANDLER.into(),
                data: serde_json::json!({ "target": actor.0 }),
            }),
            tags: vec!["chargen".into()],
        },
    )
    .map_err(CmdError::msg)?;
    doc.approved = ApprovalState::Pending(job);
    store
        .put_in(actor, &doc, actor, ctx)
        .map_err(|errors| CmdError::msg(format_errors(&errors)))?;
    ctx.out.sys_to_actor(
        actor,
        NS,
        format!("Submitted for approval as job {job}. A storyteller will review it."),
    );
    Ok(())
}

/// The [`CHARGEN_HANDLER`] payload handler: re-validate, stamp
/// [`ApprovalState::Approved`], then lift the onboarding gate.
///
/// Re-validation is what makes approval more than a rubber stamp — a sheet that
/// no longer balances is refused and the job stays open.
///
/// # Errors
///
/// A message for the approver when the target has no sheet, the sheet no longer
/// balances, or the validated write fails. The job keeps its status.
pub fn chargen_approval(
    schema: &Arc<SheetSchema>,
    onboarding_cfg: &mush_core::OnboardingConfig,
    ctx: &mut Ctx<'_>,
    job: &Job,
) -> Result<(), String> {
    let target = payload_target(job)?;
    let store = SheetStore::new(Arc::clone(schema));
    let mut doc = store
        .get(ctx.w, target)
        .ok_or_else(|| "the character has no sheet".to_string())?;
    if let Some(issue) = TacsimChargen::new(Arc::clone(schema))
        .issues(&doc)
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
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|errors| format_errors(&errors))?;
    onboarding::on_approved(onboarding_cfg, ctx, target);
    Ok(())
}

fn payload_target(job: &Job) -> Result<ObjId, String> {
    job.payload
        .as_ref()
        .and_then(|p| p.data.get("target"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .map(ObjId)
        .ok_or_else(|| "job payload has no target".to_string())
}

/// Coerce a raw `+stat` value for a scalar field type.
fn coerce(ty: &FieldType, raw: &str) -> Result<serde_json::Value, String> {
    let raw = raw.trim();
    match ty {
        FieldType::Dots { .. } | FieldType::Number { .. } => raw
            .parse::<i64>()
            .map(|n| serde_json::json!(n))
            .map_err(|_| format!("{raw:?} is not a number")),
        FieldType::Text { .. } | FieldType::Enum { .. } => Ok(serde_json::json!(raw)),
        other => Err(format!("+stat cannot set a {} field.", other.name())),
    }
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

fn format_errors(errors: &[SheetError]) -> String {
    errors
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::schema::{FIELD_GUNNERY, FIELD_PILOTING, FIELD_SENSORS, sheet_schema};

    fn flow() -> TacsimChargen {
        TacsimChargen::new(Arc::new(sheet_schema()))
    }

    fn named(doc: &mut SheetDoc) {
        doc.set(FIELD_NAME, serde_json::json!("Ada Renard"));
        doc.set(FIELD_CALLSIGN, serde_json::json!("Ladder"));
    }

    #[test]
    fn the_three_stages_are_in_order_and_only_the_first_has_a_budget() {
        let stages = flow().stages();
        let ids: Vec<&str> = stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, [STAGE_RATINGS, STAGE_IDENTITY, STAGE_REVIEW]);
        assert!(stages.first().is_some_and(|s| s.budget.is_some()));
        assert!(stages.iter().skip(1).all(|s| s.budget.is_none()));
    }

    #[test]
    fn a_fresh_sheet_is_underspent_and_unnamed() {
        let doc = SheetDoc::new_default(&sheet_schema());
        let issues = flow().issues(&doc);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ChargenIssue::Underspent { by, .. } if *by == RATING_POINTS))
        );
        assert_eq!(issues.len(), 3, "underspend plus name plus callsign");
    }

    #[test]
    fn an_exactly_spent_named_sheet_finalizes() {
        let mut doc = SheetDoc::new_default(&sheet_schema());
        named(&mut doc);
        doc.set(FIELD_GUNNERY, serde_json::json!(4));
        doc.set(FIELD_PILOTING, serde_json::json!(3));
        doc.set(FIELD_SENSORS, serde_json::json!(2));
        assert!(flow().finalize(&mut doc).is_ok(), "3 + 2 + 1 == 6");
    }

    #[test]
    fn an_overspent_sheet_does_not_finalize() {
        let mut doc = SheetDoc::new_default(&sheet_schema());
        named(&mut doc);
        doc.set(FIELD_GUNNERY, serde_json::json!(5));
        doc.set(FIELD_PILOTING, serde_json::json!(5));
        assert!(matches!(
            flow().finalize(&mut doc),
            Err(ChargenIssue::Overspent { .. })
        ));
    }

    #[test]
    fn spending_it_all_but_staying_nameless_does_not_finalize() {
        let mut doc = SheetDoc::new_default(&sheet_schema());
        doc.set(FIELD_GUNNERY, serde_json::json!(4));
        doc.set(FIELD_PILOTING, serde_json::json!(4));
        assert!(matches!(
            flow().finalize(&mut doc),
            Err(ChargenIssue::Message(_))
        ));
    }
}
