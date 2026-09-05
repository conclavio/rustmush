//! Interface cut I1: the `shadowrun.vitals` feed — the two Condition
//! Monitors and available Edge from the schema, the wound modifier, Limits
//! and Initiative from the ruleset, republished with the sheet keyframe to
//! the subject alone on every command-side sheet write.

use mush_core::{Audience, GOD, ObjId};
use mush_jobs::JobsModule;
use mush_proto::{OutputEvent, VitalRole, VitalState, VitalValue, VitalsFrame};
use mush_testkit::TestWorld;
use ruleset_shadowrun::ShadowrunRuleset;

fn world() -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(7)),
    ]);
    let nick = tw.spawn_player("Nick");
    (tw, nick)
}

fn panels(events: &[(Audience, OutputEvent)], who: ObjId, feed: &str) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Panel { feed: f, data, .. })
                if *a == who && f.as_str() == feed =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

fn any_panel_to(events: &[(Audience, OutputEvent)], who: ObjId) -> bool {
    events.iter().any(|(aud, ev)| {
        matches!((aud, ev), (Audience::Actor(a), OutputEvent::Panel { .. }) if *a == who)
    })
}

#[test]
fn a_sheet_write_republishes_monitors_edge_and_readouts_to_the_subject_only() {
    let (mut tw, nick) = world();
    let bob = tw.spawn_player("Bob");
    tw.run(GOD, "@sheet/set Nick/body=5");
    tw.run(GOD, "@sheet/set Nick/willpower=4");
    let ev = tw.run(GOD, "@sheet/set Nick/edge_current=3");

    let frames = panels(&ev, nick, "shadowrun.vitals");
    assert_eq!(frames.len(), 1, "{ev:?}");
    let frame = VitalsFrame::from_panel_data(&frames[0]).expect("a vitals frame");
    let by_id = |id: &str| {
        frame
            .vitals
            .iter()
            .find(|v| v.id == id)
            .unwrap_or_else(|| panic!("{id} declared: {:?}", frame.vitals))
    };

    let edge = by_id("shadowrun.edge_current");
    assert_eq!(edge.role, VitalRole::Pool);
    assert_eq!(
        edge.value,
        VitalValue::Bounded {
            current: 3,
            min: 0,
            max: 8
        }
    );

    let physical = by_id("shadowrun.condition_physical");
    let VitalValue::Track { boxes, level } = &physical.value else {
        panic!("a track: {physical:?}");
    };
    assert_eq!(boxes.len(), 11, "8 + ceil(5/2) boxes");
    assert!(boxes.iter().all(Option::is_none) && level.is_none());
    assert_eq!(physical.state, VitalState::Ok);
    let stun = by_id("shadowrun.condition_stun");
    assert!(matches!(&stun.value, VitalValue::Track { boxes, .. } if boxes.len() == 10));

    let wound = by_id("shadowrun.wound");
    assert_eq!(wound.role, VitalRole::Readout);
    assert_eq!(
        wound.value,
        VitalValue::Text {
            text: "0".to_string()
        }
    );
    let limits = by_id("shadowrun.limits");
    assert!(
        matches!(&limits.value, VitalValue::Text { text } if text.starts_with("P")),
        "{limits:?}"
    );
    let init = by_id("shadowrun.initiative");
    assert!(
        matches!(&init.value, VitalValue::Text { text } if text.contains("D6")),
        "{init:?}"
    );

    assert_eq!(panels(&ev, nick, "shadowrun.sheet").len(), 1);
    assert!(!any_panel_to(&ev, bob) && !any_panel_to(&ev, GOD));
}

#[test]
fn vitals_query_answers_for_the_viewer_and_refuses_a_sheetless_target() {
    let (mut tw, nick) = world();
    tw.run(GOD, "@sheet/set Nick/edge_current=2");
    let (_, handler) = tw
        .registry()
        .query_handler("shadowrun.vitals")
        .expect("registered");
    let reply = handler
        .query(tw.world(), nick, &serde_json::json!({}))
        .expect("Nick has a sheet");
    let OutputEvent::Panel { data, .. } = reply else {
        panic!("expected a Panel");
    };
    let frame = VitalsFrame::from_panel_data(&data).unwrap();
    assert!(
        frame
            .vitals
            .iter()
            .any(|v| v.id == "shadowrun.edge_current")
    );
    let bob = tw.spawn_player("Bob");
    assert!(
        handler
            .query(tw.world(), bob, &serde_json::json!({}))
            .is_err(),
        "no sheet, no frame"
    );
}
