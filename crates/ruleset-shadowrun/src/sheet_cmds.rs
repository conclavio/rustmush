//! The SR5 sheet surface: `+sheet`, `@sheet`, `@sheet/set`, and the
//! `shadowrun.sheet` web query.
//!
//! This mirrors the framework's
//! [`register_sheet_commands`](mush_ruleset::register_sheet_commands) — reusing
//! the classic [`render_sheet`](mush_ruleset::render_sheet) renderer and the
//! same schema-generic web payload (`schema.describe()` + staff-filtered data),
//! so the web sheet panel renders SR5 with **no** web change — but adds the one
//! SR5-specific behaviour the generic surface lacks: after any `@sheet/set`
//! edit it re-runs [`crate::derive::apply_derived`], so a staff change to an
//! attribute or Essence keeps the Limits / Condition Monitors / Initiative
//! consistent (SR5's `ceil`-division derived stats cannot be a `DerivedExpr`).

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, HookReadContext, ModError, ObjId, QueryHandler,
    Registrar, Role, World, match_object, perm,
};
use mush_proto::{OutputEvent, PanelFrameKind, RichText, Vital, VitalRole, VitalState, VitalValue};
use mush_ruleset::schema::{FieldType, SheetSchema};
use mush_ruleset::store::SHEET_ATTR_NS;
use mush_ruleset::{SheetDoc, VitalsSource};
use smol_str::SmolStr;

use crate::Sr5Cmd;

/// Wire the SR5 sheet surface.
pub(crate) fn register(reg: &mut Registrar<'_>, cmd: Arc<Sr5Cmd>) -> Result<(), ModError> {
    // Protect sheet:* from the core @set backdoor (best-effort).
    if let Err(e) = reg.attr_namespace(SHEET_ATTR_NS)
        && !matches!(e, ModError::Conflict { .. })
    {
        return Err(e);
    }

    // Web sheet query + reserved panel feed.
    let feed = SmolStr::new("shadowrun.sheet");
    reg.panel_feed(&feed)?;
    reg.query_handler(
        &feed,
        Arc::new(SheetQuery {
            schema: Arc::clone(&cmd.schema),
            feed: feed.clone(),
            condition_state: cmd.condition_state.clone(),
        }),
    )?;

    reg.command(
        CommandSpec {
            name: "+sheet".into(),
            shape: ArgShape::Raw,
            help: "+sheet [<target>]  --  Show your character sheet (or a target's if \
                   you can examine them). Staff-only fields are hidden from players."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| sheet_view(&cmd, ctx, false))
        },
    )?;

    reg.command(
        CommandSpec {
            name: "@sheet".into(),
            switches: vec!["set".into()],
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "@sheet <target>  --  Staff view of a sheet (shows staff-only \
                   fields).\n@sheet/set <target>/<field>=<value>  --  Set a scalar \
                   field; @sheet/set <target>/<field>:<key>=<value> a map entry \
                   (empty value removes it). Derived stats recompute automatically. \
                   Validated and audited."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| {
                if ctx.switches.has("set") {
                    sheet_set(&cmd, ctx)
                } else {
                    sheet_view(&cmd, ctx, true)
                }
            })
        },
    )?;
    Ok(())
}

fn resolve_view_target(ctx: &Ctx<'_>, raw: &str) -> Result<ObjId, CmdError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(ctx.actor);
    }
    let target = match_object(ctx.w, ctx.actor, raw)?;
    if target != ctx.actor && !perm::can_examine(ctx.w, ctx.actor, target) {
        return Err(CmdError::msg("You can't examine that character's sheet."));
    }
    Ok(target)
}

/// `+sheet` / `@sheet` — render with the framework renderer.
fn sheet_view(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>, staff: bool) -> Result<(), CmdError> {
    let target = resolve_view_target(ctx, ctx.args.raw())?;
    let Some(doc) = cmd.read_ctx(ctx, target)? else {
        return Err(CmdError::msg("That character has no sheet."));
    };
    let show_staff = staff || perm::has_role(ctx.w, ctx.actor, Role::Royalty);
    for line in mush_ruleset::render_sheet(&cmd.schema, &doc, show_staff) {
        ctx.out.sys_to_actor(ctx.actor, "sheet", line);
    }
    Ok(())
}

/// `@sheet/set <target>/<field>[:<key>]=<value>` — validated staff edit; then
/// recompute derived stats.
fn sheet_set(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    let value_raw = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: @sheet/set <target>/<field>[:<key>]=<value>"))?;

    let (target_spec, field_spec) = lhs
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: @sheet/set <target>/<field>[:<key>]=<value>"))?;
    let target = match_object(ctx.w, ctx.actor, target_spec.trim())?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(CmdError::msg("You can't modify that character's sheet."));
    }

    let field_spec = field_spec.trim();
    let (field, key) = match field_spec.split_once(':') {
        Some((f, k)) => (f.trim(), Some(k.trim())),
        None => (field_spec, None),
    };
    if field.eq_ignore_ascii_case(crate::schema::PHYSICAL_OVERFLOW)
        && cmd.condition_is_authoritative(ctx, target)?
    {
        return Err(CmdError::msg(
            "That Condition Monitor is authoritative; use condition-heal or condition-damage rather than editing overflow directly.",
        ));
    }

    let mut doc = cmd.read_or_default_ctx(ctx, target)?;
    // E6a (D-E6-1, 08f §3.5): after approval the **account** is authoritative and the
    // sheet's `nuyen` is the chargen budget and the migration source and nothing
    // else. Editing it afterwards would either be money nobody minted, or a silent
    // disagreement with the balance every render already shows. `chargen_settable`
    // is untouched, so `+stat nuyen=` still works *in* chargen, which is where the
    // number belongs.
    if field.eq_ignore_ascii_case("nuyen")
        && matches!(doc.approved, mush_ruleset::ApprovalState::Approved { .. })
    {
        return Err(CmdError::msg(
            "Nuyen is money after approval: it lives in the character's sr.nuyen account, not on \
             the sheet. Move it with @econ/pay or @econ/mint.",
        ));
    }
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
                "{field} is a map; use @sheet/set <target>/{field}:<key>=<value>."
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
                "@sheet/set cannot edit a {} field directly.",
                ty.name()
            )));
        }
    };

    // SR5-specific: keep Limits / Condition Monitors / Initiative consistent.
    crate::derive::apply_derived(&mut doc);
    store
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;

    let name = name_of(ctx, target);
    ctx.out
        .sys_to_actor(ctx.actor, "sheet", format!("Set {name}/{audit}."));
    Ok(())
}

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

/// The `shadowrun.sheet` web query handler.
struct SheetQuery {
    schema: Arc<SheetSchema>,
    feed: SmolStr,
    condition_state: mush_core::StateFamilyHandle<ObjId, crate::authoritative::ConditionState>,
}

impl SheetQuery {
    fn query_inner(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
        condition: Option<&crate::authoritative::ConditionState>,
    ) -> Result<OutputEvent, RichText> {
        let target = match params.get("target") {
            None | Some(serde_json::Value::Null) => actor,
            Some(v) => {
                let spec = v
                    .as_str()
                    .ok_or_else(|| RichText::tag("error", "bad target"))?;
                let t = match_object(w, actor, spec)
                    .map_err(|e| RichText::tag("error", e.to_string()))?;
                if t != actor && !perm::can_examine(w, actor, t) {
                    return Err(RichText::tag("error", "You can't examine that sheet."));
                }
                t
            }
        };
        let store = mush_ruleset::SheetStore::new(Arc::clone(&self.schema));
        let mut doc = store
            .get(w, target)
            .ok_or_else(|| RichText::tag("error", "That character has no sheet."))?;
        if let Some(condition) = condition {
            condition.apply_to_sheet(&mut doc);
        }
        Ok(sheet_panel(
            &self.schema,
            &self.feed,
            w,
            actor,
            target,
            &doc,
        ))
    }
}

/// The `shadowrun.sheet` keyframe for `target` as `viewer` sees it — the one
/// payload builder behind the query and the republish-on-write (interface
/// plan I1; 10b §4 rule 4).
fn sheet_panel(
    schema: &SheetSchema,
    feed: &SmolStr,
    w: &World,
    viewer: ObjId,
    target: ObjId,
    doc: &SheetDoc,
) -> OutputEvent {
    let show_staff = perm::has_role(w, viewer, Role::Royalty);
    let data = if show_staff {
        doc.data.clone()
    } else {
        strip_staff_only(schema, &doc.data)
    };
    OutputEvent::Panel {
        feed: feed.clone(),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "target": target.to_string(),
            "schema": schema.describe(),
            "approved": approval_json(doc),
            "data": data,
        }),
    }
}

/// The vitals no sheet field holds, judged by the ruleset: the wound
/// modifier (with its state), the three Limits, and Initiative
/// (interface plan I1: clients render, rulesets judge).
fn extra_vitals(doc: &SheetDoc) -> Vec<Vital> {
    let wound = crate::derive::condition_monitors(doc).wound_modifier;
    let limits = crate::derive::limits(doc);
    let init_base = doc.number(crate::schema::INITIATIVE_BASE).unwrap_or(0);
    let init_dice = doc.number(crate::schema::INITIATIVE_DICE).unwrap_or(1);
    vec![
        Vital {
            id: SmolStr::new_static("shadowrun.wound"),
            label: "Wound".into(),
            role: VitalRole::Readout,
            value: VitalValue::Text {
                text: format!("{wound}"),
            },
            state: if wound <= -3 {
                VitalState::Crit
            } else if wound < 0 {
                VitalState::Warn
            } else {
                VitalState::Ok
            },
            detail: Some("to every pool".into()),
        },
        Vital {
            id: SmolStr::new_static("shadowrun.limits"),
            label: "Limits".into(),
            role: VitalRole::Readout,
            value: VitalValue::Text {
                text: format!("P{} M{} S{}", limits.physical, limits.mental, limits.social),
            },
            state: VitalState::Ok,
            detail: None,
        },
        Vital {
            id: SmolStr::new_static("shadowrun.initiative"),
            label: "Initiative".into(),
            role: VitalRole::Readout,
            value: VitalValue::Text {
                text: format!("{init_base} + {init_dice}D6"),
            },
            state: VitalState::Ok,
            detail: None,
        },
    ]
}

/// The Shadowrun [`VitalsSource`]: the schema's declared vitals (the two
/// Condition Monitors, available Edge) read through the K8.4 authoritative
/// condition overlay, plus the derived readouts, plus the sheet keyframe the
/// republish-on-write sends (interface plan I1, D-I-2).
pub(crate) struct SrVitals {
    cmd: Arc<Sr5Cmd>,
}

impl SrVitals {
    pub(crate) fn new(cmd: Arc<Sr5Cmd>) -> SrVitals {
        SrVitals { cmd }
    }

    fn vitals_of(&self, doc: &SheetDoc) -> Vec<Vital> {
        let store = self.cmd.store();
        let mut vitals = mush_ruleset::from_sheet(&self.cmd.schema, doc, &store);
        vitals.extend(extra_vitals(doc));
        vitals
    }
}

impl VitalsSource for SrVitals {
    fn vitals(&self, ctx: &HookReadContext<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let w = ctx.world();
        let mut doc = self.cmd.store().get(w, subject)?;
        if let Ok(Some(condition)) = ctx.action_state(&self.cmd.condition_state, &subject) {
            condition.apply_to_sheet(&mut doc);
        }
        Some(self.vitals_of(&doc))
    }

    fn vitals_in(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let doc = self.cmd.read_ctx(ctx, subject).ok().flatten()?;
        Some(self.vitals_of(&doc))
    }

    fn sheet_keyframe(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<OutputEvent> {
        let doc = self.cmd.read_ctx(ctx, subject).ok().flatten()?;
        Some(sheet_panel(
            &self.cmd.schema,
            &SmolStr::new_static("shadowrun.sheet"),
            ctx.w,
            subject,
            subject,
            &doc,
        ))
    }
}

impl QueryHandler for SheetQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        self.query_inner(w, actor, params, None)
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let target = query_target(context.world(), actor, params)?;
        let condition = context
            .action_state(&self.condition_state, &target)
            .map_err(|_| RichText::tag("error", "Condition state is unavailable."))?;
        self.query_inner(context.world(), actor, params, condition)
    }
}

fn query_target(w: &World, actor: ObjId, params: &serde_json::Value) -> Result<ObjId, RichText> {
    match params.get("target") {
        None | Some(serde_json::Value::Null) => Ok(actor),
        Some(v) => {
            let spec = v
                .as_str()
                .ok_or_else(|| RichText::tag("error", "bad target"))?;
            let target =
                match_object(w, actor, spec).map_err(|e| RichText::tag("error", e.to_string()))?;
            if target != actor && !perm::can_examine(w, actor, target) {
                return Err(RichText::tag("error", "You can't examine that sheet."));
            }
            Ok(target)
        }
    }
}

fn approval_json(doc: &SheetDoc) -> serde_json::Value {
    serde_json::json!({
        "state": doc.approved.label(),
        "approved": doc.approved.is_approved(),
        "detail": serde_json::to_value(&doc.approved).unwrap_or(serde_json::Value::Null),
    })
}

fn strip_staff_only(schema: &SheetSchema, data: &serde_json::Value) -> serde_json::Value {
    let Some(obj) = data.as_object() else {
        return data.clone();
    };
    let mut out = obj.clone();
    for (_, f) in schema.fields() {
        if f.staff_only {
            out.remove(f.id.as_str());
        }
    }
    serde_json::Value::Object(out)
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
    ctx.w
        .get(who)
        .map_or_else(|| who.to_string(), |o| o.name.clone())
}

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
