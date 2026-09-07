//! Interface cut I13, the `ActionStateChanged` republish: a committed change
//! to a ruleset's own action-state family republishes the affected subject's
//! vitals and sheet through the overlay, to them alone — the path an
//! overlay-only mutation (a typed Willpower spend, Condition-Monitor damage)
//! takes, which the `AttrSet` republish never saw.
//!
//! A toy ruleset stands in for wod/shadowrun: one `focus` number declared as a
//! pool vital, one `focus` state family keyed by decimal `ObjId`, and a
//! `VitalsSource` whose `state_subject` decodes that codec. A second module
//! owning a family of the same name (and publishing no vitals — one world
//! runs one ruleset, and the revision slots are one-owner) proves the owner
//! filter.

use std::sync::{Arc, Mutex};

use mush_core::{
    Audience, CanonicalId, Ctx, GOD, GameModule, HookReadContext, ModError, ObjId, Registrar,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use mush_proto::{OutputEvent, PanelFrameKind, Vital, VitalRole, VitalValue, VitalsFrame};
use mush_ruleset::{
    FieldDef, MAX_REPUBLISH_SUBJECTS, SectionLayout, SheetDoc, SheetSchema, SheetSchemaBuilder,
    SheetSection, SheetStore, SheetView, VitalsSource, from_sheet, register_vitals,
    sheet_attr_name,
};
use mush_testkit::TestWorld;
use serde::{Deserialize, Serialize};

// ---- the toy ruleset --------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Focus {
    current: i64,
}

type FocusHandle = StateFamilyHandle<ObjId, Focus>;

fn schema(ruleset: &str) -> Arc<SheetSchema> {
    Arc::new(
        SheetSchemaBuilder::new(ruleset, 1)
            .section(
                SheetSection::new("core", "Core", SectionLayout::List)
                    .field(FieldDef::number("focus", "Focus", 0, 10).vital(VitalRole::Pool)),
            )
            .unwrap()
            .build(),
    )
}

struct Source {
    schema: Arc<SheetSchema>,
    handle: Arc<Mutex<Option<FocusHandle>>>,
}

impl Source {
    fn store(&self) -> SheetStore {
        SheetStore::new(Arc::clone(&self.schema))
    }

    fn handle(&self) -> FocusHandle {
        self.handle
            .lock()
            .unwrap()
            .clone()
            .expect("the family is registered before any event")
    }

    /// The sheet read through the authoritative overlay — what a real ruleset's
    /// `read_ctx` does with its temp-Willpower / Condition-Monitor state.
    fn overlaid(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<SheetDoc> {
        let mut doc = self.store().get(ctx.w, subject)?;
        if let Ok(Some(focus)) = ctx.action_state(&self.handle(), &subject) {
            doc.set_number("focus", focus.current);
        }
        Some(doc)
    }
}

impl VitalsSource for Source {
    fn vitals(&self, ctx: &HookReadContext<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let doc = self.store().get(ctx.world(), subject)?;
        Some(from_sheet(&self.schema, &doc, &self.store()))
    }

    fn vitals_in(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let doc = self.overlaid(ctx, subject)?;
        Some(from_sheet(&self.schema, &doc, &self.store()))
    }

    fn sheet_view(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<SheetView> {
        let doc = self.overlaid(ctx, subject)?;
        Some(SheetView {
            target: subject,
            schema: self.schema.describe(),
            approved: serde_json::json!({}),
            data: doc.data.clone(),
            show_staff: false,
        })
    }

    fn state_subject(&self, family: &StateFamilyId, key: &str) -> Option<ObjId> {
        if family.as_str() != "focus" {
            return None;
        }
        key.parse::<u32>().ok().map(ObjId)
    }
}

struct ToyRuleset {
    name: &'static str,
    schema: Arc<SheetSchema>,
    handle: Arc<Mutex<Option<FocusHandle>>>,
    /// Whether this module wires the vitals publisher (only one ruleset per
    /// world does: the revision slots are one-owner).
    publishes: bool,
}

impl GameModule for ToyRuleset {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let id = |value: &str| CanonicalId::new(value).map_err(|e| ModError::msg(e.to_string()));
        let handle = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("focus").map_err(|e| ModError::msg(e.to_string()))?,
                id("obj_id.decimal.v1")?,
                id(&format!("{}.focus.v1", self.name))?,
                1,
                id(&format!("{}.focus.decode_json.v1", self.name))?,
                id(&format!("{}.focus.validate.v1", self.name))?,
                id(&format!("{}.focus.json.v1", self.name))?,
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &serde_json::Value| {
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())
            },
            |_value: &Focus| Ok(()),
            |value: &Focus| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        *self.handle.lock().unwrap() = Some(handle);
        if !self.publishes {
            return Ok(());
        }
        register_vitals(
            reg,
            self.name,
            &sheet_attr_name(self.name),
            Arc::new(Source {
                schema: Arc::clone(&self.schema),
                handle: Arc::clone(&self.handle),
            }),
        )
    }
}

// ---- rig --------------------------------------------------------------------

struct Rig {
    tw: TestWorld,
    nick: ObjId,
    toy: FocusHandle,
    other: FocusHandle,
}

fn rig() -> Rig {
    let toy_handle = Arc::new(Mutex::new(None));
    let other_handle = Arc::new(Mutex::new(None));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(ToyRuleset {
            name: "toy",
            schema: schema("toy"),
            handle: Arc::clone(&toy_handle),
            publishes: true,
        }),
        Box::new(ToyRuleset {
            name: "other",
            schema: schema("other"),
            handle: Arc::clone(&other_handle),
            publishes: false,
        }),
    ]);
    let nick = tw.spawn_player("Nick");
    seed_sheet(&mut tw, nick, 7);
    Rig {
        tw,
        nick,
        toy: toy_handle.lock().unwrap().clone().unwrap(),
        other: other_handle.lock().unwrap().clone().unwrap(),
    }
}

/// Seed a toy sheet silently — fixture setup, not a write anyone hears.
fn seed_sheet(tw: &mut TestWorld, who: ObjId, focus: i64) {
    let store = SheetStore::new(schema("toy"));
    let mut doc = SheetDoc::new_default(store.schema());
    doc.set_number("focus", focus);
    store.put(tw.world_mut(), who, &doc).unwrap();
}

/// One committed transaction on `handle`, installed the way the world task
/// installs after an exact receipt; returns what the announcement produced.
fn commit(
    tw: &mut TestWorld,
    handle: &FocusHandle,
    writes: &[(ObjId, i64)],
    replace: bool,
) -> Vec<(Audience, OutputEvent)> {
    let prepared = {
        let mut txn = tw.engine_mut().action_transaction(handle).unwrap();
        for (subject, current) in writes {
            let focus = Focus { current: *current };
            if replace {
                txn.replace(handle, subject, focus).unwrap();
            } else {
                txn.insert(handle, subject, focus).unwrap();
            }
        }
        txn.prepare().unwrap()
    };
    let (_, installation) = prepared.into_parts();
    tw.install_prepared_action_announcing(installation)
}

/// Every `Panel` frame on `feed` addressed to `who`, as `(kind, data)`.
fn panels_to(
    events: &[(Audience, OutputEvent)],
    who: ObjId,
    feed: &str,
) -> Vec<(PanelFrameKind, serde_json::Value)> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (
                Audience::Actor(a),
                OutputEvent::Panel {
                    feed: f,
                    kind,
                    data,
                },
            ) if *a == who && f.as_str() == feed => Some((*kind, data.clone())),
            _ => None,
        })
        .collect()
}

fn panel_recipients(events: &[(Audience, OutputEvent)], feed: &str) -> Vec<ObjId> {
    let mut who: Vec<ObjId> = events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Panel { feed: f, .. }) if f.as_str() == feed => {
                Some(*a)
            }
            _ => None,
        })
        .collect();
    who.sort();
    who.dedup();
    who
}

fn focus_of(frame_data: &serde_json::Value) -> i64 {
    let frame = VitalsFrame::from_panel_data(frame_data).expect("a vitals frame");
    let vital = frame
        .vitals
        .iter()
        .find(|v| v.id == "toy.focus")
        .expect("focus is a declared vital");
    match vital.value {
        VitalValue::Bounded { current, .. } => current,
        ref other => panic!("focus is a pool: {other:?}"),
    }
}

// ---- tests ------------------------------------------------------------------

#[test]
fn an_action_state_commit_republishes_the_subjects_vitals_and_sheet_to_them_alone() {
    let mut r = rig();
    let events = commit(&mut r.tw, &r.toy, &[(r.nick, 3)], false);

    let vitals = panels_to(&events, r.nick, "toy.vitals");
    assert_eq!(vitals.len(), 1, "{events:?}");
    assert_eq!(vitals[0].0, PanelFrameKind::Keyframe);
    assert_eq!(
        focus_of(&vitals[0].1),
        3,
        "the frame reads the committed overlay, not the stored sheet (7)"
    );
    assert_eq!(vitals[0].1["revision"], 1);

    let sheet = panels_to(&events, r.nick, "toy.sheet");
    assert_eq!(sheet.len(), 1, "{events:?}");
    assert_eq!(
        sheet[0].0,
        PanelFrameKind::Keyframe,
        "the first republish after boot is a keyframe"
    );
    assert_eq!(sheet[0].1["data"]["focus"], 3);
    assert_eq!(sheet[0].1["revision"], 1);
    assert!(sheet[0].1.get("schema").is_some());

    assert!(
        events
            .iter()
            .all(|(aud, _)| matches!(aud, Audience::Actor(a) if *a == r.nick)),
        "subject-only: {events:?}"
    );
}

#[test]
fn a_second_commit_republishes_a_sheet_delta_on_the_prior_revision() {
    let mut r = rig();
    commit(&mut r.tw, &r.toy, &[(r.nick, 3)], false);
    let events = commit(&mut r.tw, &r.toy, &[(r.nick, 2)], true);

    let vitals = panels_to(&events, r.nick, "toy.vitals");
    assert_eq!(vitals.len(), 1);
    assert_eq!(focus_of(&vitals[0].1), 2);
    assert_eq!(vitals[0].1["revision"], 2);

    let sheet = panels_to(&events, r.nick, "toy.sheet");
    assert_eq!(sheet.len(), 1);
    assert_eq!(sheet[0].0, PanelFrameKind::Delta);
    assert_eq!(sheet[0].1["revision"], 2);
    assert_eq!(sheet[0].1["base_revision"], 1);
    assert_eq!(sheet[0].1["data"]["focus"], 2);
    assert!(
        sheet[0].1.get("schema").is_none(),
        "a delta omits the schema"
    );
}

#[test]
fn a_foreign_owners_family_of_the_same_name_republishes_nothing_here() {
    let mut r = rig();
    let events = commit(&mut r.tw, &r.other, &[(r.nick, 9)], false);
    assert!(
        panels_to(&events, r.nick, "toy.vitals").is_empty()
            && panels_to(&events, r.nick, "toy.sheet").is_empty(),
        "the toy ruleset does not decode another owner's `focus`: {events:?}"
    );
    // Nothing else publishes on this commit either.
    assert!(
        events
            .iter()
            .all(|(_, ev)| !matches!(ev, OutputEvent::Panel { .. })),
        "{events:?}"
    );
    // And the toy republish still works afterwards, at its own first revision.
    let events = commit(&mut r.tw, &r.toy, &[(r.nick, 1)], false);
    assert_eq!(panels_to(&events, r.nick, "toy.vitals")[0].1["revision"], 1);
}

#[test]
fn a_subject_without_a_sheet_gets_no_frame() {
    let mut r = rig();
    let events = commit(&mut r.tw, &r.toy, &[(GOD, 5)], false);
    assert!(
        events
            .iter()
            .all(|(_, ev)| !matches!(ev, OutputEvent::Panel { .. })),
        "no sheet, nothing to republish: {events:?}"
    );
}

#[test]
fn one_commit_republishes_at_most_the_capped_number_of_subjects() {
    let mut r = rig();
    let mut crowd = Vec::new();
    for index in 0..=MAX_REPUBLISH_SUBJECTS {
        let who = r.tw.spawn_player(&format!("Extra{index}"));
        seed_sheet(&mut r.tw, who, 4);
        crowd.push((who, 6));
    }
    let events = commit(&mut r.tw, &r.toy, &crowd, false);
    let recipients = panel_recipients(&events, "toy.vitals");
    assert_eq!(
        recipients.len(),
        MAX_REPUBLISH_SUBJECTS,
        "the republish is bounded per commit: {} subjects written",
        crowd.len()
    );
    assert_eq!(
        panel_recipients(&events, "toy.sheet").len(),
        MAX_REPUBLISH_SUBJECTS
    );
    // The one past the cap is not wrong, merely later: its next write or
    // query carries the committed value.
    let left_out = crowd
        .iter()
        .map(|(who, _)| *who)
        .find(|who| !recipients.contains(who))
        .expect("exactly one subject was left for later");
    let events = commit(&mut r.tw, &r.toy, &[(left_out, 6)], true);
    assert_eq!(
        focus_of(&panels_to(&events, left_out, "toy.vitals")[0].1),
        6
    );
}
