//! Interface cut I1: the `wod.vitals` feed — declared on the schema, computed
//! generically, republished with the sheet keyframe to the subject alone on
//! every command-side sheet write.

use mush_core::{Audience, GOD, ObjId};
use mush_jobs::JobsModule;
use mush_proto::{OutputEvent, VitalRole, VitalState, VitalValue, VitalsFrame};
use mush_testkit::TestWorld;
use ruleset_wod::WodRuleset;

fn world() -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(7)),
    ]);
    let nick = tw.spawn_player("Nick");
    (tw, nick)
}

/// Every `Panel` frame on `feed` addressed to `who`.
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
fn harm_republishes_vitals_and_the_sheet_to_the_subject_only() {
    let (mut tw, nick) = world();
    let bob = tw.spawn_player("Bob");
    // A staff edit seeds Nick's sheet (and republishes, to Nick only).
    let seed = tw.run(GOD, "@sheet/set Nick/strength=2");
    assert_eq!(panels(&seed, nick, "wod.vitals").len(), 1, "{seed:?}");
    assert!(!any_panel_to(&seed, bob) && !any_panel_to(&seed, GOD));

    let harm = tw.run(GOD, "+harm Nick=2b");
    let frames = panels(&harm, nick, "wod.vitals");
    assert_eq!(frames.len(), 1, "one vitals keyframe per write: {harm:?}");
    let frame = VitalsFrame::from_panel_data(&frames[0]).expect("a vitals frame");
    assert_eq!(frame.subject.id, nick.to_string());
    let health = frame
        .vitals
        .iter()
        .find(|v| v.id == "wod.health")
        .expect("health is a declared vital");
    assert_eq!(health.role, VitalRole::Track);
    let VitalValue::Track { boxes, level } = &health.value else {
        panic!("health is a track: {health:?}");
    };
    assert_eq!(boxes.iter().filter(|b| b.is_some()).count(), 2);
    assert_eq!(level.as_deref(), Some("Hurt"));
    assert_eq!(health.state, VitalState::Warn);
    assert_eq!(health.detail.as_deref(), Some("-1 dice"));
    assert!(
        frame.vitals.iter().any(|v| v.id == "wod.willpower_temp"),
        "temporary Willpower is declared: {:?}",
        frame.vitals
    );
    assert_eq!(
        panels(&harm, nick, "wod.sheet").len(),
        1,
        "the sheet keyframe republishes with the vitals"
    );
    assert!(
        !any_panel_to(&harm, bob),
        "a bystander never receives another character's frames"
    );
    assert!(!any_panel_to(&harm, GOD), "nor does the staffer who wrote");

    let again = tw.run(GOD, "+harm Nick=1b");
    let next = VitalsFrame::from_panel_data(&panels(&again, nick, "wod.vitals")[0]).unwrap();
    assert!(next.revision > frame.revision, "revisions are monotonic");
}

#[test]
fn vitals_query_answers_the_subjects_frame_from_the_same_source() {
    let (mut tw, nick) = world();
    tw.run(GOD, "@sheet/set Nick/strength=2");
    tw.run(GOD, "+harm Nick=3b");
    let (_, handler) = tw
        .registry()
        .query_handler("wod.vitals")
        .expect("the vitals query is registered");
    let reply = handler
        .query(tw.world(), nick, &serde_json::json!({}))
        .expect("Nick has a sheet");
    let OutputEvent::Panel { feed, data, .. } = reply else {
        panic!("expected a Panel, got {reply:?}");
    };
    assert_eq!(feed.as_str(), "wod.vitals");
    let frame = VitalsFrame::from_panel_data(&data).unwrap();
    let health = frame.vitals.iter().find(|v| v.id == "wod.health").unwrap();
    assert!(matches!(&health.value, VitalValue::Track { boxes, .. }
        if boxes.iter().filter(|b| b.is_some()).count() == 3));
    assert!(
        frame.vitals.len() <= mush_proto::MAX_VITALS_PER_FEED,
        "bounded"
    );
}
