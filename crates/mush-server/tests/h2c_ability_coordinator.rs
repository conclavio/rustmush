//! H2c: `UseAbility` onto the coordinator (`docs/design/11-h-implementation-plan.md`
//! §3.2 H2c). A Willpower-backed `wod_roll` chosen by the model becomes the
//! NPC's own `willpower_roll` submission through the one authoritative
//! ingress; a plain `wod_roll` still runs the legacy ability (the pinned
//! residual); a jailbroken tool name maps to nothing and is refused; the
//! submission's key is deterministic over the NPC, the turn and the ability.

use std::sync::Arc;

use mush_core::{
    Ctx, Effect, GOD, ModuleActionState, ObjId, ObjKind, OfferKeyRing, OfferSecret, Outbox,
    ParsedArgs, PerceivedRef, ResolutionKeyRing, ResolutionSecret, SwitchSet, WorldCommand,
};
use mush_npc::{AgentGrounding, NpcModule, ability_submit_key};
use mush_server::modules::npc_agent_adapter;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

fn boot() -> TestWorld {
    let grounding: Arc<dyn AgentGrounding> =
        npc_agent_adapter(&["wod".to_string()]).expect("the wod grounding adapter");
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ruleset_wod::WodRuleset::with_seed(7)),
        Box::new(NpcModule::with_grounding(grounding)),
    ]);
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap());
    tw.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(ResolutionSecret::new(1, [0x72; 32]).unwrap(), []).unwrap(),
    );
    tw
}

fn texts(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    tw.run(actor, line)
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

struct Rig {
    npc: ObjId,
    player: ObjId,
}

/// A room with a connected player and an NPC agent with a mortal WoD sheet
/// and Willpower to spend.
fn rig(tw: &mut TestWorld) -> Rig {
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "The Bar", GOD, None)
        .unwrap();
    tw.world_mut().move_to(GOD, room).unwrap();
    let player = tw.spawn_player("Pat");
    tw.world_mut().move_to(player, room).unwrap();
    tw.world_mut().set_connected(player, true);
    tw.world_mut().set_live_conns(player, 1);
    let created = texts(tw, GOD, "@agent/create Bob");
    let at = created.find("(#").expect("the create line names the id");
    let digits: String = created[at + 2..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let npc = ObjId(digits.parse().unwrap());
    let sheet = texts(tw, GOD, "@agent/sheet Bob=mortal");
    assert!(sheet.contains("sheet"), "{sheet}");
    for line in [
        "@sheet/set Bob/strength=3",
        "@sheet/set Bob/willpower=5",
        "@sheet/set Bob/willpower_temp=3",
    ] {
        let out = texts(tw, GOD, line);
        assert!(
            !out.contains("No such") && !out.contains("Usage"),
            "{line}: {out}"
        );
    }
    Rig { npc, player }
}

/// Fire the npc agent tick directly (as the module's own tests do) and return
/// the off-loop turn it started for `npc`: its correlation.
fn start_turn(tw: &mut TestWorld, npc: ObjId) -> u64 {
    let engine = tw.engine_mut();
    let handler = engine
        .registry
        .module_ticks()
        .iter()
        .find(|t| t.module == "npc")
        .expect("npc tick")
        .handler
        .clone();
    let mut out = Outbox::new();
    {
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new("npc")),
            typed_state: ModuleActionState::unavailable(),
        };
        handler.tick(&mut ctx, "agent", None);
    }
    out.take_deferred()
        .into_iter()
        .find_map(|d| match d {
            mush_core::Deferred::Effect(Effect::ModuleAsync {
                module,
                actor,
                correlation,
                ..
            }) if module == "npc" && actor == npc => Some(correlation),
            _ => None,
        })
        .expect("the tick started a turn for the npc")
}

/// One `use_ability` turn outcome, as the off-loop turn would re-enter it.
fn ability_payload(npc: ObjId, name: &str, input: serde_json::Value) -> String {
    serde_json::json!({
        "status": "ok",
        "npc": npc.0,
        "actions": [{ "action": { "action": "use_ability", "name": name, "input": input } }],
    })
    .to_string()
}

/// Stimulate the NPC, open its turn, and re-enter `payload` as the turn's
/// completion; returns the events the completion produced.
fn complete_turn(
    tw: &mut TestWorld,
    r: &Rig,
    at: i64,
    payload: impl Fn(ObjId) -> String,
) -> (u64, String) {
    tw.set_now(at);
    let _ = tw.run(r.player, "say do it");
    tw.set_now(at + 3);
    let correlation = start_turn(tw, r.npc);
    let events = tw.apply(WorldCommand::ModuleAsyncDone {
        module: SmolStr::new("npc"),
        actor: r.npc,
        correlation,
        payload: payload(r.npc),
    });
    let text = events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n");
    (correlation, text)
}

#[test]
fn h2c_a_willpower_backed_roll_becomes_the_npcs_own_authoritative_submission() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let (correlation, text) = complete_turn(&mut tw, &r, 1_000, |npc| {
        ability_payload(
            npc,
            "wod_roll",
            serde_json::json!({ "pool_expr": "strength", "willpower": true, "difficulty": 7 }),
        )
    });
    assert!(
        tw.engine().has_authoritative_submit(),
        "no authoritative submission was queued: {text}"
    );
    let submit = tw.engine_mut().take_authoritative_submit().unwrap();
    assert_eq!(submit.actor, r.npc);
    assert_eq!(submit.action.key().id.as_str(), "willpower_roll");
    assert_eq!(submit.subject, PerceivedRef::SelfActor);
    assert!(submit.targets.is_empty());
    assert_eq!(submit.parameters["pool"], "strength");
    assert_eq!(submit.parameters["difficulty"], 7);
    assert_eq!(submit.parameters["ignore_wounds"], false);
    assert_eq!(
        submit.key,
        ability_submit_key(r.npc, correlation, "wod_roll")
    );
    // Nothing rolled on the legacy path: no roll narration reached the room,
    // and the temporary Willpower is untouched until the coordinator spends it.
    assert!(!text.contains("rolls"), "the legacy roll ran: {text}");
    let sheet = texts(&mut tw, GOD, "@sheet Bob");
    assert!(sheet.contains("3"), "{sheet}");
}

#[test]
fn h2c_a_plain_roll_still_runs_the_legacy_ability_and_queues_nothing() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let (_, text) = complete_turn(&mut tw, &r, 1_000, |npc| {
        ability_payload(
            npc,
            "wod_roll",
            serde_json::json!({ "pool_expr": "strength" }),
        )
    });
    assert!(
        !tw.engine().has_authoritative_submit(),
        "a plain roll must stay on the residual path"
    );
    assert!(
        text.contains("rolls") || text.contains("Strength") || text.contains("success"),
        "the legacy roll did not narrate: {text}"
    );
}

#[test]
fn h2c_a_jailbroken_ability_name_maps_to_nothing_and_is_refused() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let (_, text) = complete_turn(&mut tw, &r, 1_000, |npc| {
        ability_payload(
            npc,
            "@shutdown",
            serde_json::json!({ "pool_expr": "strength", "willpower": true }),
        )
    });
    assert!(!tw.engine().has_authoritative_submit());
    assert!(!text.contains("shutdown"), "{text}");
}

#[test]
fn h2c_the_submission_key_is_deterministic_over_npc_turn_and_ability() {
    let a = ability_submit_key(ObjId(4), 77, "wod_roll");
    let b = ability_submit_key(ObjId(4), 77, "wod_roll");
    assert_eq!(a, b, "the same completion twice is one submission");
    assert_ne!(a, ability_submit_key(ObjId(4), 78, "wod_roll"));
    assert_ne!(a, ability_submit_key(ObjId(5), 77, "wod_roll"));
    assert_eq!(a.as_str(), "npc-4-77-wod-roll");
    assert_eq!(
        ability_submit_key(ObjId(4), 1, "@Shut Down!").as_str(),
        "npc-4-1--shut-down-"
    );
    assert_eq!(
        ability_submit_key(ObjId(4), 1, "").as_str(),
        "npc-4-1-ability"
    );
    let _ = Arc::new(());
}
