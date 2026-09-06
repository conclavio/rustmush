//! The template-dispatching sheet surface: `+sheet`, `@sheet`, `@sheet/set`,
//! and the `wod.sheet` web query — the WoD replacement for the framework's
//! single-schema [`register_sheet_commands`](mush_ruleset::register_sheet_commands).
//!
//! Every entry point selects the character's **active schema** by their
//! `template` field (via [`WodCmd`]'s dispatch helpers), so a vampire renders,
//! validates and serializes with Disciplines / Blood Pool / Generation while a
//! mortal never sees them. `@sheet/set template=<id>` re-shapes the sheet
//! through the same [`retemplate`](crate::splats::retemplate) path chargen uses.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, HookReadContext, ModError, ObjId, QueryHandler,
    Registrar, Role, World, match_object, perm,
};
use mush_proto::{OutputEvent, RichText, Vital};
use mush_ruleset::schema::{FieldType, SheetSchema};
use mush_ruleset::store::SHEET_ATTR_NS;
use mush_ruleset::{SheetDoc, SheetView, VitalsRevisions, VitalsSource};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::splats::{Dispatch, retemplate};

/// Wire the WoD sheet surface (dispatching per character `template`).
///
/// # Errors
///
/// Returns a [`ModError`] on a command / panel / query-handler collision.
pub(crate) fn register(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    // Protect sheet:* from the core @set backdoor (best-effort; a prior owner
    // means it is already protected).
    if let Err(e) = reg.attr_namespace(SHEET_ATTR_NS)
        && !matches!(e, ModError::Conflict { .. })
    {
        return Err(e);
    }

    // Web sheet query + its reserved panel feed.
    let feed = SmolStr::new("wod.sheet");
    reg.panel_feed(&feed)?;
    reg.query_handler(
        &feed,
        Arc::new(SheetQuery {
            dispatch: Arc::clone(&cmd.dispatch),
            feed: feed.clone(),
            roll_state: cmd.roll_state.clone(),
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
                   (empty value removes it). Setting <field>=template re-shapes the \
                   sheet to that splat. Validated and audited."
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

/// Resolve a `+sheet`/`@sheet` target: empty arg = self; otherwise match +
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

/// `+sheet` / `@sheet` — render against the character's active schema.
fn sheet_view(cmd: &WodCmd, ctx: &mut Ctx<'_>, staff: bool) -> Result<(), CmdError> {
    let target = resolve_view_target(ctx, ctx.args.raw())?;
    let Some(doc) = cmd.read_ctx(ctx, target)? else {
        return Err(CmdError::msg("That character has no sheet."));
    };
    let schema = cmd.schema_for_doc(&doc);
    let show_staff = staff || perm::has_role(ctx.w, ctx.actor, Role::Royalty);
    for line in mush_ruleset::render_sheet(&schema, &doc, show_staff) {
        ctx.out.sys_to_actor(ctx.actor, "sheet", line);
    }
    Ok(())
}

/// `@sheet/set <target>/<field>[:<key>]=<value>` — validated staff edit
/// against the target's active schema.
fn sheet_set(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
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

    // Changing the template re-shapes the sheet through the dispatch path.
    if field.eq_ignore_ascii_case("template") {
        let id = value_raw.trim().to_string();
        return staff_set_template(cmd, ctx, target, &id);
    }
    if field.eq_ignore_ascii_case("willpower_temp")
        && cmd.roll_state_is_authoritative(ctx, target)?
    {
        return Err(CmdError::msg(format!(
            "Temporary Willpower is authoritative. Use +act willpower-set on {target} with \
             {{\"current\":<0-10>}}."
        )));
    }

    let mut doc = cmd.read_or_default_ctx(ctx, target)?;
    let schema = cmd.schema_for_doc(&doc);
    let fdef = schema
        .field(field)
        .ok_or_else(|| CmdError::msg(format!("No such sheet field: {field}")))?
        .clone();
    let store = cmd.store_for_doc(&doc);

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

    // Let the splat recompute derived fields (Vampire: clan weakness).
    cmd.splat_for_doc(&doc).on_field_changed(&mut doc, field);
    store
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;

    let name = name_of(ctx, target);
    ctx.out
        .sys_to_actor(ctx.actor, "sheet", format!("Set {name}/{audit}."));
    Ok(())
}

/// Staff `@sheet/set <target>/template=<id>` — re-shape the sheet.
fn staff_set_template(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    target: ObjId,
    id: &str,
) -> Result<(), CmdError> {
    let id = id.to_ascii_lowercase();
    if !cmd.schemas().ids().iter().any(|t| t == &id) {
        let choices = cmd.schemas().ids().join(", ");
        return Err(CmdError::msg(format!(
            "Unknown template {id:?}. Choose one of: {choices}."
        )));
    }
    let schema = cmd.schemas().schema_for_id(&id);
    let mut doc = cmd.read_or_default_ctx(ctx, target)?;
    retemplate(&mut doc, &schema, &id);
    cmd.store_for_doc(&doc)
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    let name = name_of(ctx, target);
    ctx.out
        .sys_to_actor(ctx.actor, "sheet", format!("Set {name}/template = {id}."));
    Ok(())
}

/// Coerce a raw value string to JSON for a scalar field type.
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

/// The `wod.sheet` query handler: emit the (template-selected) schema
/// description + staff-filtered data for the web sheet tab.
struct SheetQuery {
    dispatch: Arc<Dispatch>,
    feed: SmolStr,
    roll_state: mush_core::StateFamilyHandle<ObjId, crate::authoritative::WodRollState>,
}

impl SheetQuery {
    fn query_inner(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
        roll_state: Option<&crate::authoritative::WodRollState>,
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
        let mut doc = self
            .dispatch
            .read(w, target)
            .ok_or_else(|| RichText::tag("error", "That character has no sheet."))?;
        if let Some(roll_state) = roll_state {
            overlay_roll_state(&mut doc, roll_state);
        }
        // The on-subscribe frame is always a keyframe, stamped with the same
        // per-subject revision the republish reads (interface plan I13).
        let revision = w
            .module_state::<VitalsRevisions>()
            .and_then(|r| r.0.get(&target).copied())
            .unwrap_or(0);
        Ok(build_sheet_view(&self.dispatch, w, actor, target, &doc).keyframe(&self.feed, revision))
    }
}

/// The K8.4 authoritative temporary-Willpower overlay on a read sheet.
fn overlay_roll_state(doc: &mut SheetDoc, roll_state: &crate::authoritative::WodRollState) {
    let permanent = doc.number("willpower").unwrap_or(0).clamp(0, 10);
    doc.set_number(
        "willpower_temp",
        roll_state.willpower_current.min(permanent),
    );
}

/// The `wod.sheet` payload for `target` as `viewer` sees it — the one builder
/// behind the query keyframe and the republish frame (interface plan I1/I13;
/// 10b §4 rule 4). This crate turns the [`SheetView`] into a keyframe or a
/// delta.
fn build_sheet_view(
    dispatch: &Dispatch,
    w: &World,
    viewer: ObjId,
    target: ObjId,
    doc: &SheetDoc,
) -> SheetView {
    let schema = dispatch.schemas.schema_for_doc(doc);
    let show_staff = perm::has_role(w, viewer, Role::Royalty);
    let data = if show_staff {
        doc.data.clone()
    } else {
        strip_staff_only(&schema, &doc.data)
    };
    SheetView {
        target,
        schema: schema.describe(),
        approved: approval_json(doc),
        data,
        show_staff,
    }
}

/// The WoD [`VitalsSource`]: the template-selected schema's declared vitals
/// (health, temporary Willpower, the splat pools) read through the same
/// authoritative overlay the sheet query applies, plus the sheet keyframe
/// the republish-on-write sends (interface plan I1, D-I-2).
pub(crate) struct WodVitals {
    cmd: Arc<WodCmd>,
}

impl WodVitals {
    pub(crate) fn new(cmd: Arc<WodCmd>) -> WodVitals {
        WodVitals { cmd }
    }

    fn vitals_of(&self, doc: &SheetDoc) -> Vec<Vital> {
        let schema = self.cmd.schema_for_doc(doc);
        let store = self.cmd.store_for_doc(doc);
        mush_ruleset::from_sheet(&schema, doc, &store)
    }
}

impl VitalsSource for WodVitals {
    fn vitals(&self, ctx: &HookReadContext<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let w = ctx.world();
        let mut doc = self.cmd.dispatch.read(w, subject)?;
        if let Ok(Some(roll_state)) = ctx.action_state(&self.cmd.roll_state, &subject) {
            overlay_roll_state(&mut doc, roll_state);
        }
        Some(self.vitals_of(&doc))
    }

    fn vitals_in(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let doc = self.cmd.read_ctx(ctx, subject).ok().flatten()?;
        Some(self.vitals_of(&doc))
    }

    fn sheet_view(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<SheetView> {
        let doc = self.cmd.read_ctx(ctx, subject).ok().flatten()?;
        Some(build_sheet_view(
            &self.cmd.dispatch,
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
        let roll_state = context
            .action_state(&self.roll_state, &target)
            .map_err(|_| RichText::tag("error", "WoD roll state is unavailable."))?;
        self.query_inner(context.world(), actor, params, roll_state)
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

/// A copy of `data` with every staff-only field removed.
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
