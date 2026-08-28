//! Generic sheet commands and the web query handler a ruleset can reuse or
//! override.
//!
//! [`register_sheet_commands`] wires, under the calling module's identity:
//!
//! - `+sheet [<target>]` — the classic terminal sheet (own sheet, or a
//!   target's if the actor can examine it), rendered from schema + doc.
//! - `@sheet [<target>]` / `@sheet/set <target>/<field>[:<key>]=<value>` — a
//!   staff-only viewer/editor that validates every write through
//!   [`SheetStore`] and audits it to the editing staffer.
//! - the `"<ruleset>.sheet"` **query handler** + a reserved `"<ruleset>.sheet"`
//!   panel feed, emitting the schema description + (staff-filtered) sheet data
//!   as a `Panel` keyframe for the web sheet tab.
//!
//! It also claims the [`SHEET_ATTR_NS`](crate::store::SHEET_ATTR_NS)
//! attribute namespace so the core `@set` path cannot let players hand-edit
//! `sheet:*` attributes behind the store's back (a conflict — another ruleset
//! already protecting it — is fine and ignored).

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, QueryHandler, Registrar, Role, World,
    match_object, perm,
};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use smol_str::SmolStr;

use crate::doc::SheetDoc;
use crate::schema::{FieldType, SheetSchema};
use crate::store::{MigrateFn, SHEET_ATTR_NS, SheetStore};

/// Wire the generic sheet command surface for the calling ruleset module.
/// `schema` is the ruleset's (composed) sheet schema; `migrate` lazily
/// upgrades stored docs behind the schema version (pass `None` for v1-only
/// rulesets).
///
/// # Errors
///
/// Returns a [`ModError`] if a command name, the panel feed, or the query
/// handler collides. The `sheet` attr-namespace claim is best-effort — an
/// existing owner means the namespace is already protected.
pub fn register_sheet_commands(
    reg: &mut Registrar<'_>,
    schema: Arc<SheetSchema>,
    migrate: Option<MigrateFn>,
) -> Result<(), ModError> {
    // Protect sheet:* from the core @set backdoor. If another ruleset already
    // owns the namespace, the protection is in place — swallow the conflict.
    if let Err(e) = reg.attr_namespace(SHEET_ATTR_NS)
        && !matches!(e, ModError::Conflict { .. })
    {
        return Err(e);
    }

    let ruleset = schema.ruleset.to_string();
    let store = match migrate {
        Some(m) => SheetStore::with_migration(Arc::clone(&schema), m),
        None => SheetStore::new(Arc::clone(&schema)),
    };

    // Query handler + reserved panel feed for the web sheet tab.
    let feed = SmolStr::new(format!("{ruleset}.sheet"));
    reg.panel_feed(&feed)?;
    reg.query_handler(
        &feed,
        Arc::new(SheetQuery {
            store: store.clone(),
            feed: feed.clone(),
        }),
    )?;

    // +sheet [<target>]
    let view_store = store.clone();
    reg.command(
        CommandSpec {
            name: "+sheet".into(),
            shape: ArgShape::Raw,
            help: "+sheet [<target>]  --  Show your character sheet, or a \
                   target's if you can examine them. Staff-only fields are \
                   hidden from players."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| sheet_view(&view_store, ctx, false)),
    )?;

    // @sheet [<target>] and @sheet/set <target>/<field>[:<key>]=<value>
    let staff_store = store;
    reg.command(
        CommandSpec {
            name: "@sheet".into(),
            switches: vec!["set".into()],
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "@sheet <target>  --  Staff view of a sheet (shows staff-only \
                   fields).\n@sheet/set <target>/<field>=<value>  --  Set a \
                   scalar field; @sheet/set <target>/<field>:<key>=<value> sets \
                   a map entry (empty value removes it). Validated and audited."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| {
            if ctx.switches.has("set") {
                sheet_set(&staff_store, ctx)
            } else {
                sheet_view(&staff_store, ctx, true)
            }
        }),
    )?;

    Ok(())
}

/// Resolve a `+sheet`/`@sheet` target: empty arg = self; otherwise match and
/// require examine rights.
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

/// `+sheet` / `@sheet` view. `staff` forces staff-field visibility (the
/// `@sheet` path, already role-gated); the player path shows staff fields
/// only to senior staff.
fn sheet_view(store: &SheetStore, ctx: &mut Ctx<'_>, staff: bool) -> Result<(), CmdError> {
    let target = resolve_view_target(ctx, ctx.args.raw())?;
    let Some(doc) = store.get(ctx.w, target) else {
        return Err(CmdError::msg("That character has no sheet."));
    };
    let show_staff = staff || perm::has_role(ctx.w, ctx.actor, Role::Royalty);
    for line in crate::render::render_sheet(store.schema(), &doc, show_staff) {
        ctx.out.sys_to_actor(ctx.actor, "sheet", line);
    }
    Ok(())
}

/// `@sheet/set <target>/<field>[:<key>]=<value>`.
fn sheet_set(store: &SheetStore, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
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
    let fdef = store
        .schema()
        .field(field)
        .ok_or_else(|| CmdError::msg(format!("No such sheet field: {field}")))?
        .clone();

    let mut doc = store.get_or_default(ctx.w, target);

    let audit = match (&fdef.ty, key) {
        // Map entry.
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
        // Scalars.
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
        // Tracks / lists are not scalar-editable here.
        (ty, _) => {
            return Err(CmdError::msg(format!(
                "@sheet/set cannot edit a {} field directly.",
                ty.name()
            )));
        }
    };

    store
        .put_by(ctx.w, target, &doc, ctx.actor)
        .map_err(|es| CmdError::msg(format_errors(&es)))?;

    let name = ctx
        .w
        .get(target)
        .map_or_else(|| target.to_string(), |o| o.name.clone());
    ctx.out
        .sys_to_actor(ctx.actor, "sheet", format!("Set {name}/{audit}."));
    Ok(())
}

/// Coerce a raw `@sheet/set` value string to a JSON value for a scalar
/// field type.
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

fn format_errors(errors: &[crate::validate::SheetError]) -> String {
    errors
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The `"<ruleset>.sheet"` query handler: emits the schema description plus
/// the (staff-filtered) sheet data as a `Panel` keyframe for the web sheet
/// tab. Params: `{ "target": <dbref | name | null> }` (null = self).
struct SheetQuery {
    store: SheetStore,
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
        let doc = self
            .store
            .get(w, target)
            .ok_or_else(|| RichText::tag("error", "That character has no sheet."))?;
        let show_staff = perm::has_role(w, actor, Role::Royalty);
        let data = if show_staff {
            doc.data.clone()
        } else {
            strip_staff_only(self.store.schema(), &doc.data)
        };
        Ok(OutputEvent::Panel {
            feed: self.feed.clone(),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "target": target.to_string(),
                "schema": self.store.schema().describe(),
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

/// Return a copy of `data` with every staff-only field removed.
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
