//! Interface cut I1 (look-back I1-C55): the `<ruleset>.vitals` frame is a
//! projection of the sheet, and like every other projection it must carry no
//! private field — not the staff-only sheet fields, not the approval block.
//! `mush-contract-tests`' `assert_projection_json_clean` is the house probe
//! for that; this is its first use over a vitals frame, with the private
//! fields actually seeded.
//!
//! 16b cut R-b: the two game rulesets this used to run over are gone, so the
//! probe runs over the `toy` test ruleset — the platform seam (a sheet with a
//! staff-only field and an approval block, projected through
//! `mush_ruleset::register_vitals`) is identical.

use std::sync::Arc;

use mush_contract_tests::assert_projection_json_clean;
use mush_core::{Effects, GOD, Inbound, ObjId, Outbox};
use mush_jobs::JobsModule;
use mush_proto::{OutputEvent, VitalsFrame};
use mush_ruleset::SheetStore;
use mush_testkit::TestWorld;
use mush_testkit::toy_ruleset::{ToyRuleset, toy_schema};
use serde_json::{Value, json};

/// The keyframe the `<feed>` query answers `actor` with.
fn query_panel(world: &mut TestWorld, actor: ObjId, feed: &str) -> Value {
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    world.engine_mut().step(
        Inbound::Query {
            conn: 1,
            actor,
            id: 91,
            name: feed.into(),
            args: json!({}),
        },
        &mut out,
        &mut effects,
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            OutputEvent::Reply { id: 91, event } => match event.as_ref() {
                OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("the vitals query answered with a panel keyframe")
}

/// The generic private-field names a sheet projection must never carry, plus
/// the toy's own staff-only trait (`secret`, the `.staff_only()` field on its
/// bio section — the `chargen_pools` / `chargen_priorities` maps of the deleted
/// rulesets played exactly this role).
const PRIVATE_FIELDS: &[&str] = &[
    "chargen_pools",
    "chargen_priorities",
    "staff_notes",
    "secret",
    "approved",
    "approved_by",
    "approval",
];

#[test]
fn the_toy_vitals_frame_carries_no_private_sheet_field() {
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ToyRuleset::with_seed(7)),
    ]);
    let nick = world.spawn_player("Nick");
    world.run(GOD, "@sheet/set Nick/strength=3");
    world.run(GOD, "@sheet/set Nick/willpower=5");
    // The staff-only trait is really on the stored document...
    world.run(GOD, "@sheet/set Nick/secret=4");
    assert_eq!(
        SheetStore::new(Arc::new(toy_schema("toy")))
            .get(world.world(), nick)
            .and_then(|doc| doc.number("secret")),
        Some(4),
        "the private field is on the stored sheet the projection reads"
    );

    // ...and the vitals projection carries neither it nor the approval block.
    let data = query_panel(&mut world, nick, "toy.vitals");
    let frame = VitalsFrame::from_panel_data(&data).expect("a vitals frame");
    assert!(!frame.vitals.is_empty());
    assert!(
        frame.vitals.iter().all(|v| !v.id.contains("secret")),
        "a staff-only field is never a vital: {:?}",
        frame.vitals
    );
    assert_projection_json_clean(
        "toy.vitals frame",
        &data,
        PRIVATE_FIELDS,
        &["chargen", "secret", "approv"],
    );
}
