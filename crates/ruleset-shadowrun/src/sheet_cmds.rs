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
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, QueryHandler, Registrar, Role, World,
    match_object, perm,
};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{FieldType, SheetSchema};
use mush_ruleset::store::SHEET_ATTR_NS;
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
    let Some(doc) = cmd.read(ctx.w, target) else {
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

    let mut doc = cmd.read_or_default(ctx.w, target);
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
        .put_by(ctx.w, target, &doc, ctx.actor)
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
}

impl QueryHandler for SheetQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
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
        let doc = store
            .get(w, target)
            .ok_or_else(|| RichText::tag("error", "That character has no sheet."))?;
        let show_staff = perm::has_role(w, actor, Role::Royalty);
        let data = if show_staff {
            doc.data.clone()
        } else {
            strip_staff_only(&self.schema, &doc.data)
        };
        Ok(OutputEvent::Panel {
            feed: self.feed.clone(),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "target": target.to_string(),
                "schema": self.schema.describe(),
                "approved": approval_json(&doc),
                "data": data,
            }),
        })
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
