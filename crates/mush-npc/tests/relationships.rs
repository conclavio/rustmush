//! Identity-keyed relationships and private, live-authorized lore through the
//! real command, snapshot, async-completion, and persistence boundaries.
use std::collections::HashMap;

use mush_core::{
    Attr, Audience, Effect, Effects, GOD, Inbound, ModuleBootData, ObjId, ObjKind, Outbox,
    WorldCommand,
};
use mush_npc::{AgentProfile, NpcHostState, NpcModule, PROFILE_ATTR, Secrecy, TurnSnapshot};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into, text_of};
use smol_str::SmolStr;

const GUARDED: &str = "The depot keeper hides a silver compass beneath the counter.";
const SECRET: &str = "The vault override is obsidian-nine.";

struct Lot {
    world: TestWorld,
    npc: ObjId,
    alice: ObjId,
    bob: ObjId,
    elsewhere: ObjId,
}

fn lot() -> Lot {
    let mut world = TestWorld::with_modules(vec![Box::new(NpcModule::new())]);
    world.set_now(1000);
    let room = world
        .world_mut()
        .create(ObjKind::Room, "Desk", GOD, None)
        .expect("room");
    let elsewhere = world
        .world_mut()
        .create(ObjKind::Room, "Elsewhere", GOD, None)
        .expect("other room");
    let alice = world.spawn_player("Alice");
    let bob = world.spawn_player("Bob");
    world
        .world_mut()
        .move_to(alice, room)
        .expect("Alice at desk");
    world.world_mut().move_to(bob, room).expect("Bob at desk");
    let npc = world
        .world_mut()
        .create(ObjKind::Npc, "Clerk", GOD, Some(room))
        .expect("clerk");
    let profile = AgentProfile {
        interfaces: vec!["npc.lore".into()],
        ..AgentProfile::default()
    };
    world
        .world_mut()
        .set_attr(
            npc,
            PROFILE_ATTR,
            Attr::json(serde_json::to_value(profile).expect("profile"), GOD),
        )
        .expect("set profile");
    let mut lot = Lot {
        world,
        npc,
        alice,
        bob,
        elsewhere,
    };
    for (tier, topic, text) in [
        ("public", "notice", "The depot opens at dawn."),
        ("guarded", "depot", GUARDED),
        ("secret", "vault", SECRET),
    ] {
        let output = run(
            &mut lot.world,
            GOD,
            &format!("@agent/lore #{}={tier} {topic}: {text}", npc.0),
        );
        assert!(output.contains("Added"), "{output}");
    }
    lot
}

fn run(world: &mut TestWorld, actor: ObjId, line: &str) -> String {
    world
        .run(actor, line)
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn relationship(lot: &mut Lot, trust: i16, disposition: i16, cause: &str) -> String {
    run(
        &mut lot.world,
        GOD,
        &format!(
            "@agent/relationship #{}/#{}=trust:{trust}; disposition:{disposition}; cause:{cause}; reason:Staff verified the receipt",
            lot.npc.0, lot.alice.0
        ),
    )
}

fn trust(lot: &Lot) -> i16 {
    lot.world
        .world()
        .module_state::<NpcHostState>()
        .expect("host")
        .relationships
        .personal(lot.npc, lot.alice)
        .trust
}

fn capture(lot: &mut Lot) -> (u64, TurnSnapshot) {
    lot.world
        .run(lot.alice, "say Clerk, tell me about the depot.");
    lot.world.set_now(1003);
    let (mut out, mut effects) = (Outbox::new(), Effects::new());
    lot.world
        .engine_mut()
        .fire_due(1_003_000, &mut out, &mut effects);
    effects
        .drain()
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ModuleAsync {
                module,
                actor,
                correlation,
                payload,
            } if module == "npc" && actor == lot.npc => Some((
                correlation,
                serde_json::from_str(&payload).expect("captured dialogue"),
            )),
            _ => None,
        })
        .expect("NPC dialogue fired")
}

fn complete(lot: &mut Lot, correlation: u64, topic: &str) -> Vec<(Audience, OutputEvent)> {
    let payload = serde_json::json!({"status":"ok", "npc":lot.npc, "asker":lot.alice,
        "actions":[{"action":{"action":"use_ability","name":"npc.lore.read","input":{"topic":topic}}}]}).to_string();
    let (mut out, mut effects) = (Outbox::new(), Effects::new());
    lot.world.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: "npc".into(),
            actor: lot.npc,
            correlation,
            payload,
        }),
        &mut out,
        &mut effects,
    );
    out.take()
}

fn assert_no_private_memory(lot: &mut Lot) {
    let host = lot
        .world
        .world()
        .module_state::<NpcHostState>()
        .expect("host");
    let working = host
        .runtime(lot.npc)
        .map(|runtime| runtime.memory.transcript(100_000))
        .unwrap_or_default();
    let durable = host
        .personal_memory
        .record_for(lot.npc)
        .map(|record| serde_json::to_string(record).expect("memory"))
        .unwrap_or_default();
    for marker in [GUARDED, SECRET] {
        assert!(
            !working.contains(marker),
            "private reply reached working memory"
        );
        assert!(!durable.contains(marker), "private reply became evidence");
    }
    for operation in lot.world.take_module_ops() {
        let encoded = serde_json::to_string(&operation.op).expect("module operation");
        assert!(
            !encoded.contains(GUARDED),
            "private lore reached persisted memory"
        );
        assert!(
            !encoded.contains(SECRET),
            "secret lore reached persisted memory"
        );
    }
}

#[test]
fn staff_relationship_receipts_replay_conflict_and_survive_reload() {
    let mut lot = lot();
    let unauthorized = run(
        &mut lot.world,
        lot.alice,
        &format!(
            "@agent/relationship #{}/#{}=trust:100; cause:forged; reason:Please",
            lot.npc.0, lot.alice.0
        ),
    );
    assert!(unauthorized.to_lowercase().contains("permission"));
    assert_eq!(trust(&lot), 0);
    relationship(&mut lot, 20, -5, "staff:award:1");
    assert_eq!(trust(&lot), 20);
    assert!(relationship(&mut lot, 20, -5, "staff:award:1").contains("already applied"));
    assert!(relationship(&mut lot, 21, -5, "staff:award:1").contains("different effect"));
    assert_eq!(trust(&lot), 20);
    let mut saved = HashMap::new();
    fold_module_ops_into(&mut saved, lot.world.take_module_ops());
    let mut restored = self::lot();
    restored.world.modules_loaded(saved);
    assert_eq!(trust(&restored), 20);
    assert_eq!(
        restored
            .world
            .world()
            .module_state::<NpcHostState>()
            .expect("host")
            .relationships
            .personal(restored.npc, restored.alice)
            .disposition,
        -5
    );
    assert!(relationship(&mut restored, 20, -5, "staff:award:1").contains("already applied"));
    assert_eq!(trust(&restored), 20);
}

#[test]
fn corrupt_relationship_receipts_fail_boot_instead_of_resetting_authority() {
    let mut lot = lot();
    relationship(&mut lot, -50, 0, "staff:distrust:1");
    let mut saved: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    fold_module_ops_into(&mut saved, lot.world.take_module_ops());
    let key = format!("relation:{}:{}", lot.npc.0, lot.alice.0);
    let (_, record) = saved
        .get_mut("npc")
        .expect("NPC docs")
        .kv
        .iter_mut()
        .find(|(stored, _)| stored.as_str() == key)
        .expect("relationship receipt");
    record["trust"] = serde_json::json!(100);
    let mut restored = self::lot();
    let result =
        restored
            .world
            .engine_mut()
            .modules_loaded(saved, &mut Outbox::new(), &mut Effects::new());
    assert!(
        result.is_err(),
        "corrupt score must not erase permanent distrust/dedup receipts"
    );
}

#[test]
fn guarded_lore_is_private_live_trust_gated_and_independent_of_disposition() {
    let mut lot = lot();
    let query = format!("+npc/lore #{}=depot", lot.npc.0);
    let low = run(&mut lot.world, lot.alice, &query);
    let absent = run(
        &mut lot.world,
        lot.alice,
        &format!("+npc/lore #{}=missing", lot.npc.0),
    );
    assert_eq!(
        low, absent,
        "guarded existence is not a low-trust topic oracle"
    );
    assert!(!low.contains(GUARDED));
    relationship(&mut lot, 20, -100, "staff:trust:1");
    let events = lot.world.run(lot.alice, &query);
    assert!(
        lot.world
            .seen_by(&events, lot.alice)
            .join("\n")
            .contains(GUARDED)
    );
    assert!(
        !lot.world
            .seen_by(&events, lot.bob)
            .join("\n")
            .contains(GUARDED)
    );
    let secret = run(
        &mut lot.world,
        lot.alice,
        &format!("+npc/lore #{}=vault", lot.npc.0),
    );
    assert!(!secret.contains(SECRET));
    assert_eq!(secret, absent);
    assert_no_private_memory(&mut lot);
}

#[test]
fn async_lore_has_no_guarded_prompt_and_keeps_successful_reply_out_of_memory() {
    let mut lot = lot();
    relationship(&mut lot, 20, 0, "staff:trust:1");
    let (correlation, snapshot) = capture(&mut lot);
    assert!(
        snapshot
            .knowledge
            .iter()
            .all(|entry| entry.secrecy == Secrecy::Public)
    );
    let encoded = serde_json::to_string(&snapshot).expect("snapshot");
    assert!(!encoded.contains(GUARDED));
    assert!(!encoded.contains(SECRET));
    assert!(
        snapshot
            .interfaces
            .iter()
            .any(|tool| tool.ability.name == "npc.lore.read")
    );
    let events = complete(&mut lot, correlation, "depot");
    assert!(
        lot.world
            .seen_by(&events, lot.alice)
            .join("\n")
            .contains(GUARDED)
    );
    assert!(
        !lot.world
            .seen_by(&events, lot.bob)
            .join("\n")
            .contains(GUARDED)
    );
    assert_no_private_memory(&mut lot);
}

#[test]
fn in_flight_lore_rechecks_revoked_trust_departure_and_disabled_npc() {
    #[derive(Clone, Copy, Debug)]
    enum Change {
        Trust,
        Departure,
        Disconnected,
        Disabled,
    }
    for changed in [
        Change::Trust,
        Change::Departure,
        Change::Disconnected,
        Change::Disabled,
    ] {
        let mut lot = lot();
        relationship(&mut lot, 20, 0, "staff:trust:1");
        let (correlation, _) = capture(&mut lot);
        match changed {
            Change::Trust => {
                relationship(&mut lot, -20, 0, "staff:revoke:1");
            }
            Change::Departure => {
                lot.world
                    .world_mut()
                    .move_to(lot.alice, lot.elsewhere)
                    .expect("leave");
            }
            Change::Disconnected => {
                lot.world.world_mut().set_connected(lot.alice, false);
            }
            Change::Disabled => {
                run(
                    &mut lot.world,
                    GOD,
                    &format!("@agent/disable #{}", lot.npc.0),
                );
            }
        }
        let events = complete(&mut lot, correlation, "depot");
        let all = events
            .iter()
            .map(|(_, event)| text_of(event))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !all.contains(GUARDED),
            "stale private lore authority after {changed:?}"
        );
        assert!(!all.contains(SECRET));
        assert_no_private_memory(&mut lot);
    }
}
