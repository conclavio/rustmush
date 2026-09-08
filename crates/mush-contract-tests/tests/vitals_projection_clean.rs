//! Interface cut I1 (look-back I1-C55): the `<ruleset>.vitals` frame is a
//! projection of the sheet, and like every other projection it must carry no
//! private field — not the staff-only chargen maps, not the approval block.
//! `mush-contract-tests`' `assert_projection_json_clean` is the house probe
//! for that; this is its first use over a vitals frame, for both shipped
//! rulesets with the private fields actually seeded.

use mush_contract_tests::assert_projection_json_clean;
use mush_core::{Effects, GOD, Inbound, ObjId, Outbox};
use mush_jobs::JobsModule;
use mush_proto::{OutputEvent, VitalsFrame};
use mush_testkit::TestWorld;
use ruleset_shadowrun::ShadowrunRuleset;
use ruleset_wod::WodRuleset;
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

const PRIVATE_FIELDS: &[&str] = &[
    "chargen_pools",
    "chargen_priorities",
    "staff_notes",
    "approved",
    "approved_by",
    "approval",
];

#[test]
fn the_wod_vitals_frame_carries_no_private_sheet_field() {
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(7)),
    ]);
    let nick = world.spawn_player("Nick");
    world.run(GOD, "@sheet/set Nick/strength=3");
    world.run(GOD, "@sheet/set Nick/chargen_pools:physical=7");
    let data = query_panel(&mut world, nick, "wod.vitals");
    let frame = VitalsFrame::from_panel_data(&data).expect("a vitals frame");
    assert!(!frame.vitals.is_empty());
    assert_projection_json_clean("wod.vitals frame", &data, PRIVATE_FIELDS, &["chargen"]);
}

#[test]
fn the_shadowrun_vitals_frame_carries_no_private_sheet_field() {
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(9)),
    ]);
    let nick = world.spawn_player("Nick");
    world.run(GOD, "@sheet/set Nick/body=4");
    world.run(GOD, "@sheet/set Nick/chargen_priorities:attributes=A");
    let data = query_panel(&mut world, nick, "shadowrun.vitals");
    let frame = VitalsFrame::from_panel_data(&data).expect("a vitals frame");
    assert!(!frame.vitals.is_empty());
    assert_projection_json_clean(
        "shadowrun.vitals frame",
        &data,
        PRIVATE_FIELDS,
        &["chargen", "priorit"],
    );
}
