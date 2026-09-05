//! Interface cut I1: the `scenes` panel feed (a keyframe to every member on
//! each lifecycle transition), its query, and the room-ambient glance.

use std::collections::HashMap;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::output::room_view;
use mush_core::{
    Audience, GOD, LIMBO, ObjId, ObjKind, Outbox, Registry, World, register_core_commands,
    register_engine_commands,
};
use mush_proto::{OutputEvent, PanelFrameKind};
use mush_scenes::ScenesModule;

struct Rig {
    engine: Engine,
    bar: ObjId,
    nick: ObjId,
    ann: ObjId,
    rae: ObjId,
}

fn rig() -> Rig {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let bar = world
        .create(ObjKind::Room, "The Tap Room", GOD, None)
        .unwrap();
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(bar))
        .unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(bar))
        .unwrap();
    // Rae is connected elsewhere: never a member, never a recipient.
    let rae = world
        .create(ObjKind::Player, "Rae", GOD, Some(LIMBO))
        .unwrap();
    for who in [nick, ann, rae] {
        world.set_connected(who, true);
        world.set_live_conns(who, 1);
    }
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(mush_canon::CanonModule::new()),
            Box::new(ScenesModule::new()),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    let _ = engine.take_module_ops();
    Rig {
        engine,
        bar,
        nick,
        ann,
        rae,
    }
}

fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut fx,
    );
    out
}

/// The `scenes` keyframes addressed to `who`.
fn scene_panels(out: &Outbox, who: ObjId) -> Vec<serde_json::Value> {
    out.events()
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (
                Audience::Actor(a),
                OutputEvent::Panel {
                    feed,
                    kind: PanelFrameKind::Keyframe,
                    data,
                },
            ) if *a == who && feed.as_str() == "scenes" => Some(data.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn lifecycle_publishes_scene_keyframes_to_members_only() {
    let mut rig = rig();
    let start = run(&mut rig.engine, rig.nick, "+timestop");
    for who in [rig.nick, rig.ann] {
        let frames = scene_panels(&start, who);
        assert_eq!(frames.len(), 1, "{who} gets one keyframe on start");
        assert_eq!(frames[0]["status"], "active");
        assert_eq!(frames[0]["round"], 1);
        assert!(
            frames[0]["participants"]
                .as_array()
                .is_some_and(|p| p.len() >= 2)
        );
    }
    assert!(
        scene_panels(&start, rig.rae).is_empty(),
        "an outsider hears nothing"
    );

    let next = run(&mut rig.engine, rig.nick, "+timestop/next");
    let frame = &scene_panels(&next, rig.ann)[0];
    assert_eq!(frame["turn"], 1, "the turn cursor advanced: {frame}");

    let pause = run(&mut rig.engine, rig.nick, "+timestop/pause");
    assert_eq!(scene_panels(&pause, rig.ann)[0]["status"], "paused");
    let resume = run(&mut rig.engine, rig.nick, "+timestop/resume");
    assert_eq!(scene_panels(&resume, rig.ann)[0]["status"], "active");

    let release = run(&mut rig.engine, rig.nick, "+timestop/release Ann");
    let ann_frames = scene_panels(&release, rig.ann);
    assert!(
        ann_frames.iter().any(serde_json::Value::is_null),
        "the released member is told they have no scene: {ann_frames:?}"
    );
    let nick_frames = scene_panels(&release, rig.nick);
    assert!(
        nick_frames.last().is_some_and(|f| f["participants"]
            .as_array()
            .is_some_and(|p| p.iter().all(|m| m["name"] != "Ann"))),
        "the remaining members see her gone: {nick_frames:?}"
    );

    let end = run(&mut rig.engine, rig.nick, "+timestop/end");
    assert_eq!(scene_panels(&end, rig.nick)[0]["status"], "closed");
}

#[test]
fn query_and_room_ambient_reflect_the_active_scene() {
    let mut rig = rig();
    let (_, handler) = rig
        .engine
        .registry
        .query_handler("scenes")
        .expect("the scenes query is registered");
    let none = handler
        .query(&rig.engine.world, rig.nick, &serde_json::json!({}))
        .unwrap();
    assert!(
        matches!(&none, OutputEvent::Panel { data, .. } if data.is_null()),
        "no scene, null: {none:?}"
    );
    let rv = room_view(&rig.engine.registry, &rig.engine.world, rig.nick, rig.bar).unwrap();
    assert!(!rv.ambient.contains_key("scene"));

    run(&mut rig.engine, rig.nick, "+timestop");
    let some = handler
        .query(&rig.engine.world, rig.nick, &serde_json::json!({}))
        .unwrap();
    let OutputEvent::Panel { data, .. } = some else {
        panic!("expected a Panel");
    };
    assert_eq!(data["status"], "active");
    assert!(data["facilitator"]["name"] == "Nick", "{data}");
    let outsider = handler
        .query(&rig.engine.world, rig.rae, &serde_json::json!({}))
        .unwrap();
    assert!(matches!(&outsider, OutputEvent::Panel { data, .. } if data.is_null()));

    let rv = room_view(&rig.engine.registry, &rig.engine.world, rig.nick, rig.bar).unwrap();
    let glance = rv
        .ambient
        .get("scene")
        .expect("the anchored room carries the glance");
    assert_eq!(glance["status"], "active");
    assert_eq!(glance["id"], data["id"]);
    assert!(rv.ambient.contains_key("ic_time"));

    run(&mut rig.engine, rig.nick, "+timestop/end");
    let rv = room_view(&rig.engine.registry, &rig.engine.world, rig.nick, rig.bar).unwrap();
    assert!(
        !rv.ambient.contains_key("scene"),
        "closed scenes are not glanced"
    );
}
