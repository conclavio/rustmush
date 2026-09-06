//! H2b: NPC scene turns as durable decisions (`docs/design/11-h-implementation-plan.md`
//! §3.2 H2b). `+timestop/next` handing the turn to an NPC emits
//! `WorldEvent::SceneTurn`; the NPC module queues exactly one decision request
//! for it, keyed on `(scene, round, actor)` with a replaceable key on
//! `(scene, actor)`; the agent tick captures it; a player's turn, an NPC with
//! no profile, and an NPC in another room get nothing; the scene log names the
//! NPC turn.

use std::sync::Arc;

use mush_core::module::emit_world_event;
use mush_core::{
    DecisionKeyRing, DecisionSecret, GOD, ObjId, ObjKind, OfferKeyRing, OfferSecret, WorldEvent,
};
use mush_npc::{NpcHostState, NpcModule};
use mush_scenes::ScenesModule;
use mush_testkit::{TestWorld, text_of};

fn boot() -> TestWorld {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(ScenesModule::new()),
        Box::new(NpcModule::new().with_scene_turn_deadline_secs(45)),
    ]);
    tw.engine_mut().set_decision_keys(
        DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).unwrap(), []).unwrap(),
    );
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap());
    tw
}

fn texts(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    tw.run(actor, line)
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

/// God, one connected player and one NPC agent in a room; God facilitates.
struct Rig {
    room: ObjId,
    player: ObjId,
    npc: ObjId,
}

fn rig(tw: &mut TestWorld) -> Rig {
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "The Bar", GOD, None)
        .expect("a room");
    tw.world_mut().move_to(GOD, room).expect("move");
    let player = tw.spawn_player("Pat");
    tw.world_mut().move_to(player, room).expect("move");
    tw.world_mut().set_connected(player, true);
    tw.world_mut().set_live_conns(player, 1);
    let created = texts(tw, GOD, "@agent/create Bob");
    let at = created.find("(#").expect("the create line names the id");
    let digits: String = created[at + 2..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let npc = ObjId(digits.parse().unwrap());
    Rig { room, player, npc }
}

fn requests(tw: &TestWorld) -> Vec<mush_npc::NpcDecisionRequest> {
    tw.world()
        .module_state::<NpcHostState>()
        .expect("host")
        .decision_requests
        .iter()
        .cloned()
        .collect()
}

#[test]
fn h2b_advancing_to_an_npc_queues_one_request_and_the_tick_captures_it() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let started = texts(&mut tw, GOD, "+timestop");
    assert!(started.contains("timestop"), "{started}");
    // Turn order: connected players (God is not connected here; Pat is), then
    // NPCs. The first advance opens round 1 on Pat; the second hands the turn
    // to Bob.
    let _ = texts(&mut tw, GOD, "+timestop/next");
    assert!(
        requests(&tw).is_empty(),
        "the player's turn requested nothing"
    );
    let _ = texts(&mut tw, GOD, "+timestop/next");
    let queued = requests(&tw);
    assert_eq!(queued.len(), 1, "exactly one request for the NPC's turn");
    let req = &queued[0];
    assert_eq!(req.npc, r.npc);
    assert_eq!(req.cause_kind.as_str(), "npc.scene_turn");
    assert!(
        req.instance.as_str().starts_with("scene-"),
        "{}",
        req.instance.as_str()
    );
    assert!(
        req.instance
            .as_str()
            .ends_with(&format!("-npc-{}", r.npc.0))
    );
    assert!(req.instance.as_str().contains("-round-"));
    assert_eq!(
        req.replaceable_key.as_ref().map(|k| k.as_str().to_owned()),
        Some(format!(
            "scene-{}-npc-{}",
            req.instance.as_str().split('-').nth(1).unwrap(),
            r.npc.0
        ))
    );
    assert_eq!(
        req.deadline_secs, 45,
        "the [npc] deadline reached the request"
    );
    assert_eq!(req.context["room"], "The Bar");
    assert!(
        req.context["present"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "Pat"),
        "{}",
        req.context
    );
    let _ = r.player;
    // The tick issues the capture.
    let _ = tw.fire_due(5_000);
    assert!(
        tw.engine().has_decision_capture(),
        "the tick did not capture"
    );
    let capture = tw.engine_mut().take_decision_capture().unwrap();
    assert_eq!(capture.draft.actor, r.npc);
    assert_eq!(capture.draft.deadline_secs, 45);
}

#[test]
fn h2b_the_same_round_is_one_instance_and_a_new_round_shares_the_replaceable_key() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let fire = |tw: &mut TestWorld, round: u32| {
        let mut out = mush_core::Outbox::new();
        let engine = tw.engine_mut();
        emit_world_event(
            &engine.registry,
            &mut engine.world,
            &mut out,
            &WorldEvent::SceneTurn {
                scene: 7,
                actor: r.npc,
                round,
            },
        );
    };
    fire(&mut tw, 2);
    fire(&mut tw, 2);
    fire(&mut tw, 3);
    let queued = requests(&tw);
    assert_eq!(queued.len(), 3);
    assert_eq!(
        queued[0].instance, queued[1].instance,
        "a re-advance is the same instance"
    );
    assert_ne!(
        queued[0].instance, queued[2].instance,
        "a new round is a new instance"
    );
    assert_eq!(
        queued[0].replaceable_key, queued[2].replaceable_key,
        "a new round displaces the older pending record"
    );
    assert_eq!(
        queued[0].key, queued[0].instance,
        "the chosen action keys on the instance"
    );
    let _ = r.room;
}

#[test]
fn h2b_a_player_a_profileless_npc_and_a_disabled_agent_get_no_request() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let ghost = tw
        .world_mut()
        .create(ObjKind::Npc, "Ghost", GOD, Some(r.room))
        .expect("an npc object with no profile");
    let _ = texts(&mut tw, GOD, "@agent/disable Bob");
    let mut out = mush_core::Outbox::new();
    for actor in [r.player, ghost, r.npc] {
        let engine = tw.engine_mut();
        emit_world_event(
            &engine.registry,
            &mut engine.world,
            &mut out,
            &WorldEvent::SceneTurn {
                scene: 9,
                actor,
                round: 1,
            },
        );
    }
    assert!(
        requests(&tw).is_empty(),
        "a player, a profileless npc or a disabled agent requested a decision: {:?}",
        requests(&tw)
    );
}

#[test]
fn h2b_the_scene_log_names_the_npc_turn() {
    let mut tw = boot();
    let _r = rig(&mut tw);
    let _ = texts(&mut tw, GOD, "+timestop");
    let _ = texts(&mut tw, GOD, "+timestop/next");
    let _ = texts(&mut tw, GOD, "+timestop/next");
    let log = texts(&mut tw, GOD, "+timestop/log");
    assert!(
        log.contains("Bob's turn (an NPC: its decision was requested)"),
        "{log}"
    );
    assert!(log.contains("Pat's turn."), "{log}");
}

#[test]
fn h2b_the_turn_context_stays_under_the_kernel_cap_at_its_bounds() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let long_name = "N".repeat(400);
    for n in 0..(mush_npc::MAX_TURN_PRESENT + 4) {
        let p = tw.spawn_player(&format!("{long_name}{n}"));
        tw.world_mut().move_to(p, r.room).expect("move");
        tw.world_mut().set_connected(p, true);
        tw.world_mut().set_live_conns(p, 1);
    }
    for n in 0..(mush_npc::MAX_TURN_EXITS + 4) {
        let _ = texts(&mut tw, GOD, &format!("@open {long_name}{n}"));
    }
    let _ = texts(&mut tw, GOD, &format!("@desc here={}", "d".repeat(40_000)));
    let _ = texts(
        &mut tw,
        GOD,
        &format!("@agent/set Bob/persona={}", "p".repeat(20_000)),
    );
    let profile = mush_npc::AgentProfile {
        persona: "p".repeat(20_000),
        ..mush_npc::AgentProfile::default()
    };
    let context = mush_npc::scene_turn_context(tw.world(), &profile, 1, 1, r.npc, r.room);
    let bytes = serde_json::to_vec(&context).unwrap().len();
    assert!(
        bytes < mush_core::MAX_DECISION_CONTEXT_BYTES,
        "{bytes} bytes at the bounds"
    );
    assert!(context["present"].as_array().unwrap().len() <= mush_npc::MAX_TURN_PRESENT);
    assert!(context["exits"].as_array().unwrap().len() <= mush_npc::MAX_TURN_EXITS);
    assert!(context["persona"].as_str().unwrap().len() <= mush_npc::MAX_TURN_PERSONA_BYTES);
    let accepted = mush_core::ProjectedDecisionContext::new(
        mush_core::CanonicalId::new(mush_npc::NPC_CONTEXT_SCHEMA).unwrap(),
        context,
    );
    assert!(accepted.is_ok(), "the kernel refused the bounded context");
    let _ = Arc::new(());
}
