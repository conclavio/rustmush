//! Vitals — the ruleset-neutral "numbers that kill you" feed (interface plan
//! I1, D-I-2).
//!
//! A ruleset declares which sheet fields are vitals on the schema
//! ([`FieldDef::vital`](crate::schema::FieldDef::vital)); [`from_sheet`] turns
//! the flagged fields into wire [`Vital`]s generically — a `Track` becomes
//! boxes plus the deepest filled level, a `Number` or `Dots` a bounded gauge —
//! and a ruleset adds what no field holds (a wound modifier, limits,
//! initiative) through its [`VitalsSource`]. [`register_vitals`] then wires,
//! under the calling module's identity:
//!
//! - the `"<ruleset>.vitals"` panel feed, so it is in the `hello` manifest and
//!   subscribable by name (clients key off the `.vitals` suffix the way they
//!   key off `.sheet` and `.roll`);
//! - a same-named **query handler**, so a subscribe yields a keyframe;
//! - an [`AttrSet`](WorldEventKind::AttrSet) subscription on the ruleset's
//!   sheet attribute, so every [`SheetStore::put_in`](crate::SheetStore::put_in)
//!   republishes the subject's vitals frame **and** their sheet keyframe to the
//!   subject only — never to the room.
//!
//! Frames are always whole and carry a per-subject `revision` that is
//! monotonic within a process ([`VitalsRevisions`], a module state slot), so a
//! client drops a late older frame; the watermark resets on every `hello`.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    Audience, Ctx, HookReadContext, ModError, ObjId, PanelFeedHandle, QueryHandler, Registrar,
    World, WorldEvent, WorldEventKind, WorldObserver, match_object, perm,
};
use mush_proto::{
    MAX_VITAL_DETAIL_BYTES, MAX_VITALS_PER_FEED, ObjRef, OutputEvent, PanelFrameKind, RichText,
    Vital, VitalState, VitalValue, VitalsFrame,
};
use smol_str::SmolStr;

use crate::doc::SheetDoc;
use crate::schema::{FieldType, SheetSchema};
use crate::store::SheetStore;

/// The vitals feed id for a ruleset: `"<ruleset>.vitals"`.
#[must_use]
pub fn vitals_feed(ruleset: &str) -> SmolStr {
    SmolStr::new(format!("{ruleset}.vitals"))
}

/// Per-subject vitals revisions (a module state slot, RAM only). Reset at
/// boot by construction, which is why clients reset their watermark on
/// `hello`. The `<ruleset>.sheet` feed reuses this same counter (interface
/// plan I13), so a sheet delta and the vitals frame carry the identical
/// per-subject revision after one write.
#[derive(Clone, Debug, Default)]
pub struct VitalsRevisions(pub HashMap<ObjId, u64>);

/// Per-subject sheet schema fingerprints (interface plan I13): a change forces
/// the next republish to a fresh keyframe instead of a delta. A module state
/// slot beside [`VitalsRevisions`], RAM only and reset at boot — 8 bytes per
/// subject, never a data snapshot.
#[derive(Clone, Debug, Default)]
pub struct SheetSchemaFps(pub HashMap<ObjId, u64>);

/// The materialized `<ruleset>.sheet` payload a viewer sees — the shared input
/// to both the on-subscribe keyframe and the republish delta (interface plan
/// I13). A ruleset's [`VitalsSource`] produces this; this crate owns the wire
/// shaping so the keyframe/delta discipline and the fingerprint live in one
/// place. Built with the same staff filter and overlay the query applies.
pub struct SheetView {
    /// The subject (`target` on the wire).
    pub target: ObjId,
    /// The whole schema description ([`SheetSchema::describe`]); sent only on a
    /// keyframe.
    pub schema: serde_json::Value,
    /// The approval block.
    pub approved: serde_json::Value,
    /// The staff-filtered field data.
    pub data: serde_json::Value,
    /// Whether staff-only fields are visible to the viewer. Folded into the
    /// fingerprint so a staff-visibility flip also forces a keyframe (the
    /// old-client key-drop fix): a full-data delta that silently drops a
    /// now-hidden field would otherwise leave it stale on a shallow-merge
    /// client.
    pub show_staff: bool,
}

impl SheetView {
    /// A 64-bit fingerprint over the exact wire schema bytes plus the viewer's
    /// staff-visibility bit: any change to either flips it and forces a
    /// keyframe (interface plan I13). `describe()` is already computed for the
    /// keyframe, so this is cheap.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.schema.to_string().hash(&mut h);
        self.show_staff.hash(&mut h);
        h.finish()
    }

    fn base(&self, revision: u64) -> serde_json::Value {
        serde_json::json!({
            "target": self.target.to_string(),
            "approved": self.approved.clone(),
            "data": self.data.clone(),
            "revision": revision,
        })
    }

    /// The full keyframe (carries `schema` + `revision`).
    #[must_use]
    pub fn keyframe(&self, feed: &SmolStr, revision: u64) -> OutputEvent {
        let mut data = self.base(revision);
        data["schema"] = self.schema.clone();
        OutputEvent::Panel {
            feed: feed.clone(),
            kind: PanelFrameKind::Keyframe,
            data,
        }
    }

    /// The incremental delta (omits `schema`, carries `base_revision`).
    #[must_use]
    pub fn delta(&self, feed: &SmolStr, revision: u64) -> OutputEvent {
        let mut data = self.base(revision);
        data["base_revision"] = serde_json::json!(revision.saturating_sub(1));
        OutputEvent::Panel {
            feed: feed.clone(),
            kind: PanelFrameKind::Delta,
            data,
        }
    }
}

/// What a ruleset supplies to the vitals publisher: its own reads of a
/// subject's sheet (with whatever authoritative overlay it keeps), in the two
/// contexts the publisher runs in, plus the sheet it republishes.
pub trait VitalsSource: Send + Sync {
    /// The subject's vitals for a read-only query. `None` when the subject
    /// has no sheet.
    fn vitals(&self, ctx: &HookReadContext<'_>, subject: ObjId) -> Option<Vec<Vital>>;

    /// The subject's vitals inside a command or observer. `None` when the
    /// subject has no sheet.
    fn vitals_in(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<Vec<Vital>>;

    /// The subject's own sheet as they see it (staff-filtered, overlay
    /// applied), for the republish-on-write. `None` skips the sheet republish.
    /// This crate turns it into a keyframe or a delta (interface plan I13,
    /// replacing the former prebuilt-keyframe return).
    fn sheet_view(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<SheetView>;
}

/// The vitals a sheet declares, in schema order: every field with a
/// [`vital`](crate::schema::FieldDef::vital) role that is not staff-only.
/// Ids are `"<ruleset>.<field>"`; a track's level and wound penalty come from
/// the schema's damage levels; states follow one generic rule (a pool at or
/// below a quarter is `Crit`, at or below half `Warn`, empty `Down`; a track
/// with a penalty is `Warn`, at −3 or worse `Crit`, full `Down`) — a ruleset
/// with a sharper judgement supplies it in its own extra vitals instead.
#[must_use]
pub fn from_sheet(schema: &SheetSchema, doc: &SheetDoc, store: &SheetStore) -> Vec<Vital> {
    let mut out = Vec::new();
    for (_, f) in schema.fields() {
        let Some(role) = f.vital else { continue };
        if f.staff_only {
            continue;
        }
        let id = SmolStr::new(format!("{}.{}", schema.ruleset, f.id));
        let vital = match &f.ty {
            FieldType::Track { levels, .. } => {
                let Some(boxes) = doc.track(&f.id) else {
                    continue;
                };
                let filled = boxes.iter().filter(|m| m.is_some()).count();
                let level = (filled > 0 && !levels.is_empty())
                    .then(|| levels[(filled - 1).min(levels.len() - 1)].label.clone());
                let penalty = store.wound_penalty(doc, &f.id);
                let state = if !boxes.is_empty() && filled >= boxes.len() {
                    VitalState::Down
                } else if penalty <= -3 {
                    VitalState::Crit
                } else if penalty < 0 {
                    VitalState::Warn
                } else {
                    VitalState::Ok
                };
                Vital {
                    id,
                    label: f.label.clone(),
                    role,
                    value: VitalValue::Track { boxes, level },
                    state,
                    detail: (penalty != 0).then(|| format!("{penalty} dice")),
                }
            }
            FieldType::Number { min, max } => {
                let Some(current) = doc.number(&f.id) else {
                    continue;
                };
                bounded(id, f.label.clone(), role, current, *min, *max)
            }
            FieldType::Dots { min, max } => {
                let Some(current) = doc.dots(&f.id) else {
                    continue;
                };
                bounded(
                    id,
                    f.label.clone(),
                    role,
                    i64::from(current),
                    i64::from(*min),
                    i64::from(*max),
                )
            }
            // Text, enums, lists and maps are never gauges.
            _ => continue,
        };
        out.push(vital);
    }
    out
}

/// The generic pool ladder, in one place (interface plan I8 lifted it out of
/// [`bounded`] so the tactical unit view judges a cell exactly as a sheet
/// pool is judged): an empty span is `Ok`, an empty pool `Down`, a quarter or
/// less `Crit`, half or less `Warn`. Bounds are righted and `current` clamped
/// first. It is a **depletion** ladder — right for armor, structure, ammo and
/// blood; an accumulator (heat) reads inverted until a rules pack declares its
/// own `warn`/`crit` thresholds (plan §9).
#[must_use]
pub fn pool_state(current: i64, min: i64, max: i64) -> VitalState {
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    let current = current.clamp(min, max);
    let span = max.saturating_sub(min);
    let rel = current.saturating_sub(min);
    if span <= 0 {
        VitalState::Ok
    } else if rel <= 0 {
        VitalState::Down
    } else if rel.saturating_mul(4) <= span {
        VitalState::Crit
    } else if rel.saturating_mul(2) <= span {
        VitalState::Warn
    } else {
        VitalState::Ok
    }
}

/// A `Pool` gauge over [`pool_state`] — the shape a tactical unit's
/// unit-level cells take on the `tactics.vitals` feed (interface plan I8).
#[must_use]
pub fn pool(id: SmolStr, label: String, current: i64, min: i64, max: i64) -> Vital {
    bounded(id, label, mush_proto::VitalRole::Pool, current, min, max)
}

/// A bounded gauge with the generic pool thresholds, clamped into its bounds.
fn bounded(
    id: SmolStr,
    label: String,
    role: mush_proto::VitalRole,
    current: i64,
    min: i64,
    max: i64,
) -> Vital {
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    let current = current.clamp(min, max);
    let state = pool_state(current, min, max);
    Vital {
        id,
        label,
        role,
        value: VitalValue::Bounded { current, min, max },
        state,
        detail: None,
    }
}

/// Build a frame, bringing an over-long list or detail back inside the caps
/// (with a warning) rather than dropping the whole frame; a frame that still
/// refuses (an out-of-bounds gauge a ruleset built by hand) is logged and
/// `None`.
#[must_use]
pub fn frame(subject: ObjRef, revision: u64, mut vitals: Vec<Vital>) -> Option<VitalsFrame> {
    if vitals.len() > MAX_VITALS_PER_FEED {
        tracing::warn!(
            subject = %subject.id,
            count = vitals.len(),
            "vitals: more than the cap declared; truncating"
        );
        vitals.truncate(MAX_VITALS_PER_FEED);
    }
    for v in &mut vitals {
        if let Some(d) = &mut v.detail
            && d.len() > MAX_VITAL_DETAIL_BYTES
        {
            let mut end = MAX_VITAL_DETAIL_BYTES;
            while end > 0 && !d.is_char_boundary(end) {
                end -= 1;
            }
            d.truncate(end);
        }
    }
    match VitalsFrame::new(subject, revision, vitals) {
        Ok(frame) => Some(frame),
        Err(error) => {
            tracing::warn!(%error, "vitals: frame refused");
            None
        }
    }
}

/// Wire the vitals feed, its query and the republish-on-write subscription
/// for the calling ruleset module. `sheet_attr` is the attribute its sheets
/// live under (`sheet_attr_name(ruleset)`), which is what the `AttrSet`
/// subscription filters on.
///
/// # Errors
///
/// Returns a [`ModError`] if the feed or query handler collides.
pub fn register_vitals(
    reg: &mut Registrar<'_>,
    ruleset: &str,
    sheet_attr: &str,
    source: Arc<dyn VitalsSource>,
) -> Result<(), ModError> {
    let feed_name = vitals_feed(ruleset);
    let feed = reg.panel_feed(&feed_name)?;
    reg.state(VitalsRevisions::default());
    reg.state(SheetSchemaFps::default());
    reg.query_handler(
        &feed_name,
        Arc::new(VitalsQuery {
            module: SmolStr::new(ruleset),
            feed: feed_name.clone(),
            source: Arc::clone(&source),
        }),
    )?;
    reg.subscribe(
        WorldEventKind::AttrSet,
        Arc::new(VitalsRepublish {
            feed,
            sheet_feed: SmolStr::new(format!("{ruleset}.sheet")),
            sheet_attr: sheet_attr.to_string(),
            source,
        }),
    );
    Ok(())
}

/// The `<ruleset>.vitals` query handler: the subject's frame as a `Panel`
/// keyframe. Params: `{ "target": <dbref | name | null> }` (null = self);
/// another subject needs examine rights, like the sheet query.
struct VitalsQuery {
    module: SmolStr,
    feed: SmolStr,
    source: Arc<dyn VitalsSource>,
}

impl QueryHandler for VitalsQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let context = HookReadContext::without_typed_state(w, &self.module);
        self.query_with_context(&context, actor, params)
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let w = context.world();
        let target = match params.get("target") {
            None | Some(serde_json::Value::Null) => actor,
            Some(v) => {
                let spec = v
                    .as_str()
                    .ok_or_else(|| RichText::tag("error", "bad target"))?;
                let t = match_object(w, actor, spec)
                    .map_err(|e| RichText::tag("error", e.to_string()))?;
                if t != actor && !perm::can_examine(w, actor, t) {
                    return Err(RichText::tag("error", "You can't examine that."));
                }
                t
            }
        };
        let vitals = self
            .source
            .vitals(context, target)
            .ok_or_else(|| RichText::tag("error", "That character has no sheet."))?;
        let revision = w
            .module_state::<VitalsRevisions>()
            .and_then(|r| r.0.get(&target))
            .copied()
            .unwrap_or(0);
        let subject = mush_core::output::obj_ref(w, target);
        let frame = frame(subject, revision, vitals)
            .ok_or_else(|| RichText::tag("error", "Vitals are unavailable."))?;
        Ok(OutputEvent::Panel {
            feed: self.feed.clone(),
            kind: PanelFrameKind::Keyframe,
            data: frame.to_panel_data(),
        })
    }
}

/// The `AttrSet` observer: a write to the ruleset's sheet attribute bumps the
/// subject's revision and republishes their vitals frame and sheet frame to
/// them alone. The sheet frame is a delta unless the schema fingerprint
/// changed (a template swap) or this is the first write after boot, in which
/// case it is a fresh keyframe (interface plan I13).
struct VitalsRepublish {
    feed: PanelFeedHandle,
    sheet_feed: SmolStr,
    sheet_attr: String,
    source: Arc<dyn VitalsSource>,
}

impl WorldObserver for VitalsRepublish {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        let WorldEvent::AttrSet { obj, name, .. } = ev else {
            return;
        };
        if !name.eq_ignore_ascii_case(&self.sheet_attr) {
            return;
        }
        let subject = *obj;
        let revision = match ctx.state_mut::<VitalsRevisions>() {
            Some(r) => {
                let slot = r.0.entry(subject).or_insert(0);
                *slot += 1;
                *slot
            }
            None => 0,
        };
        if let Some(vitals) = self.source.vitals_in(ctx, subject)
            && let Some(frame) = frame(mush_core::output::obj_ref(ctx.w, subject), revision, vitals)
        {
            self.feed
                .keyframe_to(ctx, Audience::Actor(subject), frame.to_panel_data());
        }
        if let Some(view) = self.source.sheet_view(ctx, subject) {
            let fp = view.fingerprint();
            let prior = ctx
                .state::<SheetSchemaFps>()
                .and_then(|m| m.0.get(&subject).copied());
            // Force a keyframe on a schema/staff-visibility change, on the first
            // write after boot (no prior fp), or at revision 0 (no revision
            // counter registered) — never emit a delta the client cannot base.
            let event = if revision == 0 || prior != Some(fp) {
                view.keyframe(&self.sheet_feed, revision)
            } else {
                view.delta(&self.sheet_feed, revision)
            };
            if let Some(m) = ctx.state_mut::<SheetSchemaFps>() {
                m.0.insert(subject, fp);
            }
            ctx.out.to_actor(subject, event);
        } else if let Some(m) = ctx.state_mut::<SheetSchemaFps>() {
            // No sheet to publish (wiped, or unreadable): forget its
            // fingerprint so a recreated sheet force-keyframes rather than
            // arriving as a delta the client cannot base.
            m.0.remove(&subject);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::VitalRole;

    use crate::schema::{
        DamageLevelDef, FieldDef, SectionLayout, SheetSchemaBuilder, SheetSection, TrackSize,
    };

    fn schema() -> Arc<SheetSchema> {
        Arc::new(
            SheetSchemaBuilder::new("toy", 1)
                .section(
                    SheetSection::new("core", "Core", SectionLayout::List)
                        .field(
                            FieldDef::track(
                                "health",
                                "Health",
                                TrackSize::Fixed(4),
                                vec![
                                    DamageLevelDef::new("bruised", "Bruised", 0),
                                    DamageLevelDef::new("hurt", "Hurt", -1),
                                    DamageLevelDef::new("mauled", "Mauled", -3),
                                    DamageLevelDef::new("out", "Incapacitated", -9),
                                ],
                            )
                            .vital(VitalRole::Track),
                        )
                        .field(FieldDef::number("blood", "Blood", 0, 10).vital(VitalRole::Pool))
                        .field(FieldDef::dots("will", "Willpower", 1, 5).vital(VitalRole::Pool))
                        .field(FieldDef::number("xp", "Experience", 0, 999))
                        .field(
                            FieldDef::number("secret", "Secret", 0, 9)
                                .vital(VitalRole::Pool)
                                .staff_only(),
                        )
                        .field(FieldDef::text("concept", "Concept", 40).vital(VitalRole::Readout)),
                )
                .unwrap()
                .build(),
        )
    }

    #[test]
    fn from_sheet_lists_only_flagged_gaugeable_non_staff_fields() {
        let schema = schema();
        let store = SheetStore::new(Arc::clone(&schema));
        let doc = SheetDoc::new_default(&schema);
        let vitals = from_sheet(&schema, &doc, &store);
        let ids: Vec<&str> = vitals.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, vec!["toy.health", "toy.blood", "toy.will"]);
        assert!(
            vitals.iter().all(|v| !v.id.contains("secret")),
            "a staff-only field is never a vital"
        );
    }

    #[test]
    fn track_reports_level_penalty_and_state() {
        let schema = schema();
        let store = SheetStore::new(Arc::clone(&schema));
        let mut doc = SheetDoc::new_default(&schema);
        let mark = |n: usize| -> Vec<Option<SmolStr>> {
            (0..4)
                .map(|i| (i < n).then(|| SmolStr::new("lethal")))
                .collect()
        };
        doc.set_track("health", &mark(0));
        let v = &from_sheet(&schema, &doc, &store)[0];
        assert_eq!(v.state, VitalState::Ok);
        assert!(matches!(&v.value, VitalValue::Track { level: None, .. }));

        doc.set_track("health", &mark(2));
        let v = &from_sheet(&schema, &doc, &store)[0];
        assert_eq!(v.state, VitalState::Warn);
        assert!(matches!(&v.value, VitalValue::Track { level: Some(l), .. } if l == "Hurt"));
        assert_eq!(v.detail.as_deref(), Some("-1 dice"));

        doc.set_track("health", &mark(3));
        assert_eq!(from_sheet(&schema, &doc, &store)[0].state, VitalState::Crit);

        doc.set_track("health", &mark(4));
        assert_eq!(from_sheet(&schema, &doc, &store)[0].state, VitalState::Down);
    }

    #[test]
    fn pools_follow_the_generic_thresholds_and_clamp() {
        let schema = schema();
        let store = SheetStore::new(Arc::clone(&schema));
        let mut doc = SheetDoc::new_default(&schema);
        let blood = |doc: &SheetDoc| {
            from_sheet(&schema, doc, &store)
                .into_iter()
                .find(|v| v.id == "toy.blood")
                .unwrap()
        };
        doc.set_number("blood", 10);
        assert_eq!(blood(&doc).state, VitalState::Ok);
        doc.set_number("blood", 5);
        assert_eq!(blood(&doc).state, VitalState::Warn);
        doc.set_number("blood", 2);
        assert_eq!(blood(&doc).state, VitalState::Crit);
        doc.set_number("blood", 0);
        assert_eq!(blood(&doc).state, VitalState::Down);
        // An out-of-range stored value clamps rather than refusing the frame.
        doc.set_number("blood", 40);
        assert_eq!(
            blood(&doc).value,
            VitalValue::Bounded {
                current: 10,
                min: 0,
                max: 10
            }
        );
    }

    #[test]
    fn frame_truncates_past_the_caps_instead_of_dropping() {
        let many: Vec<Vital> = (0..(MAX_VITALS_PER_FEED + 5))
            .map(|i| Vital {
                id: SmolStr::new(format!("toy.v{i}")),
                label: "v".into(),
                role: VitalRole::Pool,
                value: VitalValue::Text { text: "x".into() },
                state: VitalState::Ok,
                detail: Some("d".repeat(MAX_VITAL_DETAIL_BYTES + 10)),
            })
            .collect();
        let f = frame(ObjRef::new("#1", "Nick"), 3, many).expect("frame builds");
        assert_eq!(f.vitals.len(), MAX_VITALS_PER_FEED);
        assert!(
            f.vitals
                .iter()
                .all(|v| v.detail.as_ref().unwrap().len() <= MAX_VITAL_DETAIL_BYTES)
        );
        assert_eq!(f.revision, 3);
    }

    fn view(schema: serde_json::Value, show_staff: bool) -> SheetView {
        SheetView {
            target: ObjId(1),
            schema,
            approved: serde_json::json!({ "state": "unapproved" }),
            data: serde_json::json!({ "strength": 3 }),
            show_staff,
        }
    }

    #[test]
    fn sheet_view_fingerprint_flips_on_schema_or_staff_visibility() {
        let base = view(serde_json::json!({ "v": 1, "fields": ["a"] }), false).fingerprint();
        // Same inputs → same fingerprint (a stable, no-op republish stays a delta).
        assert_eq!(
            base,
            view(serde_json::json!({ "v": 1, "fields": ["a"] }), false).fingerprint()
        );
        // Any wire-schema change flips it (a template swap, a field/label edit).
        assert_ne!(
            base,
            view(serde_json::json!({ "v": 2, "fields": ["a"] }), false).fingerprint()
        );
        assert_ne!(
            base,
            view(serde_json::json!({ "v": 1, "fields": ["a", "b"] }), false).fingerprint()
        );
        // A staff-visibility flip flips it (the M2 old-client key-drop fix).
        assert_ne!(
            base,
            view(serde_json::json!({ "v": 1, "fields": ["a"] }), true).fingerprint()
        );
    }

    #[test]
    fn keyframe_carries_schema_and_revision_delta_omits_schema_and_bases_on_prev() {
        let feed = SmolStr::new_static("wod.sheet");
        let v = view(serde_json::json!({ "s": 1 }), false);

        let OutputEvent::Panel { kind, data, .. } = v.keyframe(&feed, 5) else {
            panic!("keyframe is a Panel");
        };
        assert_eq!(kind, PanelFrameKind::Keyframe);
        assert!(data.get("schema").is_some(), "keyframe carries the schema");
        assert_eq!(data.get("revision").and_then(|x| x.as_u64()), Some(5));
        assert!(data.get("base_revision").is_none());

        let OutputEvent::Panel { kind, data, .. } = v.delta(&feed, 6) else {
            panic!("delta is a Panel");
        };
        assert_eq!(kind, PanelFrameKind::Delta);
        assert!(
            data.get("schema").is_none(),
            "a delta omits the static schema"
        );
        assert_eq!(data.get("revision").and_then(|x| x.as_u64()), Some(6));
        assert_eq!(data.get("base_revision").and_then(|x| x.as_u64()), Some(5));
        assert!(data.get("data").is_some(), "a delta carries the full data");
    }
}
