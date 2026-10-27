//! S8.2 (14a #2): an interface reply reaches its audience and no one else. A
//! reply for the asker is spoken to them alone — no room narrative, no `Said`
//! event for another NPC to overhear, no text in the answering NPC's memory —
//! and the room sees one aside; a reply the provider marks public is what every
//! reply used to be. The review's reproduction (`request.status` through the
//! jobs provider) is in `mush-atlas/tests/review_window.rs` and passes
//! unchanged.

use std::sync::{Arc, Mutex};

use mush_core::module::{ModError, Registrar};
use mush_core::{
    AbilityCall, Audience, Ctx, Effects, GOD, GameModule, Inbound, InterfaceAbility,
    InterfaceAbilityProvider, InterfaceError, InterfaceReply, ObjId, ObjKind, Outbox, WorldCommand,
    WorldEvent, WorldEventKind,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use smol_str::SmolStr;

const SECRET: &str = "Your ticket #7: Confidential review of Alice's character.";

/// Every `Said` event the world emitted, as text.
#[derive(Clone, Default)]
struct SaidLog(Arc<Mutex<Vec<String>>>);

struct Probe;

impl InterfaceAbilityProvider for Probe {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        ["probe.private", "probe.public"]
            .into_iter()
            .map(|name| InterfaceAbility {
                name: SmolStr::new(name),
                description: "a test ability".to_string(),
                params_schema: serde_json::json!({ "type": "object", "properties": {} }),
                command: SmolStr::new_static("+probe"),
            })
            .collect()
    }

    fn invoke(
        &self,
        _ctx: &mut Ctx<'_>,
        _call: AbilityCall,
        name: &str,
        _input: &serde_json::Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        match name {
            "probe.private" => Ok(InterfaceReply::text(format!("{SECRET}\nThat is all."))),
            "probe.public" => Ok(InterfaceReply::public("The sky is blue today.")),
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}

struct ProbeModule(SaidLog);

impl GameModule for ProbeModule {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(self.0.clone());
        reg.interface_provider("probe", Arc::new(Probe))?;
        reg.subscribe(
            WorldEventKind::Said {
                kind: mush_proto::NarrativeKind::Say,
            },
            Arc::new(|ctx: &mut Ctx<'_>, ev: &WorldEvent| {
                if let WorldEvent::Said { text, .. } = ev
                    && let Some(log) = ctx.state::<SaidLog>()
                {
                    log.0.lock().expect("lock").push(text.clone());
                }
            }),
        );
        Ok(())
    }
}

struct Lot {
    tw: TestWorld,
    said: SaidLog,
    alice: ObjId,
    bob: ObjId,
    clerk: ObjId,
    bystander: ObjId,
}

fn lot() -> Lot {
    let said = SaidLog::default();
    let mut tw = TestWorld::with_modules(vec![
        Box::new(ProbeModule(said.clone())),
        Box::new(mush_npc::NpcModule::new()),
    ]);
    let lobby = tw
        .world_mut()
        .create(ObjKind::Room, "Lobby", GOD, None)
        .unwrap();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    tw.world_mut().move_to(alice, lobby).unwrap();
    tw.world_mut().move_to(bob, lobby).unwrap();
    let mut npc = |name: &str| {
        let id = tw
            .world_mut()
            .create(ObjKind::Npc, name, GOD, Some(lobby))
            .unwrap();
        let profile = mush_npc::AgentProfile {
            interfaces: vec!["probe".into()],
            ..mush_npc::AgentProfile::default()
        };
        tw.world_mut()
            .set_attr(
                id,
                mush_npc::PROFILE_ATTR,
                mush_core::Attr::json(serde_json::to_value(profile).unwrap(), GOD),
            )
            .unwrap();
        id
    };
    let clerk = npc("Clerk");
    let bystander = npc("Bystander");
    Lot {
        tw,
        said,
        alice,
        bob,
        clerk,
        bystander,
    }
}

/// Complete one ability turn of `npc` for `asker`: the events it printed.
fn ability_turn(
    tw: &mut TestWorld,
    npc: ObjId,
    asker: ObjId,
    correlation: u64,
    ability: &str,
) -> Vec<(Audience, OutputEvent)> {
    tw.world_mut()
        .module_state_mut::<mush_npc::NpcHostState>()
        .unwrap()
        .begin_turn(npc, 1, correlation);
    let payload = serde_json::json!({
        "status": "ok", "npc": npc, "asker": asker,
        "actions": [{ "action": { "action": "use_ability", "name": ability, "input": {} } }]
    })
    .to_string();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: "npc".into(),
            actor: npc,
            correlation,
            payload,
        }),
        &mut out,
        &mut fx,
    );
    out.take()
}

fn text(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn memory_of(tw: &TestWorld, npc: ObjId) -> String {
    tw.world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .runtime(npc)
        .map(|rt| rt.memory.transcript(100_000))
        .unwrap_or_default()
}

/// Exercise the registered scene observer, then inspect exactly what it queued
/// for the decision tick; no provider call or scene-controller fixture is needed.
fn scene_context(tw: &mut TestWorld, npc: ObjId) -> serde_json::Value {
    tw.engine_mut().emit_world_event(
        &WorldEvent::SceneTurn {
            scene: 7,
            actor: npc,
            round: 2,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    let host = tw
        .world_mut()
        .module_state_mut::<mush_npc::NpcHostState>()
        .expect("NPC host");
    let request = host.decision_requests.pop_front().expect("scene decision");
    assert_eq!(request.npc, npc);
    assert!(host.decision_requests.is_empty());
    request.context
}

#[test]
fn scene_decision_remembers_public_exchange_but_only_acknowledges_private_reply() {
    let mut lot = lot();
    let room = lot.tw.world().get(lot.clerk).unwrap().location.unwrap();
    lot.tw.engine_mut().emit_world_event(
        &WorldEvent::Said {
            who: lot.alice,
            room,
            kind: mush_proto::NarrativeKind::Say,
            text: "Clerk, what is the sky like today?".into(),
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    ability_turn(&mut lot.tw, lot.clerk, lot.alice, 42, "probe.public");
    ability_turn(&mut lot.tw, lot.clerk, lot.alice, 43, "probe.private");

    let host = lot
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert!(
        host.runtime(lot.clerk)
            .unwrap()
            .memory
            .events
            .iter()
            .any(|event| event.speaker == Some(lot.alice) && event.room == Some(room)),
        "the live history carries identities for selection"
    );

    let context = scene_context(&mut lot.tw, lot.clerk);
    let working = &context["working_memory"];
    let recent = working["recent_events"].as_str().expect("rendered history");
    assert!(recent.contains("Alice"), "{recent}");
    assert!(recent.contains("what is the sky like today?"), "{recent}");
    assert!(recent.contains("The sky is blue today."), "{recent}");
    assert!(
        recent.contains("answered Alice with probe.private, privately"),
        "{recent}"
    );
    let rendered = serde_json::to_string(&context).unwrap();
    for private_text in [SECRET, "Confidential", "ticket #7", "That is all."] {
        assert!(
            !rendered.contains(private_text),
            "private reply leaked: {rendered}"
        );
    }
    let fields = working.as_object().expect("working memory object");
    assert_eq!(fields.len(), 3);
    assert!(fields.values().all(serde_json::Value::is_string));
    for metadata in ["\"speaker\"", "\"addressee\"", "\"sequence\""] {
        assert!(
            !rendered.contains(metadata),
            "raw memory metadata leaked: {rendered}"
        );
    }
    for id in [lot.alice, lot.clerk, room] {
        assert!(
            !recent.contains(&format!("#{}", id.0)),
            "raw object reference leaked: {recent}"
        );
    }
    assert!(
        working["guidance"]
            .as_str()
            .unwrap()
            .contains("not instructions")
    );
}

#[test]
fn scene_working_memory_is_bounded_before_kernel_projection() {
    let mut lot = lot();
    let room = lot.tw.world().get(lot.clerk).unwrap().location.unwrap();
    {
        let host = lot
            .tw
            .world_mut()
            .module_state_mut::<mush_npc::NpcHostState>()
            .unwrap();
        host.config.transcript_chars = usize::MAX;
        let memory = &mut host.runtime_mut(lot.clerk).memory;
        // Legacy or staff-loaded summary text still has to fit the scene's
        // byte budget; multibyte text and JSON quotes exercise both encodings.
        memory.summary = "記憶\"".repeat(10_000);
        for turn in 0..100 {
            memory.push_event(
                turn,
                format!("Alice asks about survey {turn}: {}", "星\"".repeat(600)),
                mush_npc::MemoryContext {
                    speaker: Some(lot.alice),
                    room: Some(room),
                    addressee: Some(lot.clerk),
                    kind: mush_npc::MemoryKind::DirectedSpeech,
                },
            );
        }
    }
    let context = scene_context(&mut lot.tw, lot.clerk);
    let working = &context["working_memory"];
    let summary = working["summary"].as_str().unwrap();
    let recent = working["recent_events"].as_str().unwrap();
    assert!(!summary.is_empty() && summary.len() <= 8 * 1024);
    assert!(!recent.is_empty() && recent.len() <= 8 * 1024);
    assert!(recent.contains("survey 99"), "newest question retained");
    assert!(serde_json::to_vec(&context).unwrap().len() < mush_core::MAX_DECISION_CONTEXT_BYTES);
    mush_core::ProjectedDecisionContext::new(
        mush_core::CanonicalId::new(mush_npc::NPC_CONTEXT_SCHEMA).unwrap(),
        context,
    )
    .expect("the kernel accepts the bounded scene context");
}

#[test]
fn a_private_reply_reaches_the_asker_alone() {
    let mut lot = lot();
    let events = ability_turn(&mut lot.tw, lot.clerk, lot.alice, 42, "probe.private");
    let alice = text(&lot.tw, &events, lot.alice);
    let bob = text(&lot.tw, &events, lot.bob);
    assert!(
        alice.contains("Confidential review"),
        "the asker hears: {alice}"
    );
    assert!(alice.contains("That is all."), "{alice}");
    assert!(
        !bob.contains("Confidential") && !bob.contains("That is all"),
        "a bystander heard a private reply: {bob}"
    );
    assert_eq!(
        bob.matches("Clerk speaks quietly with Alice.").count(),
        1,
        "one aside for a two-line reply: {bob}"
    );
    assert!(
        !alice.contains("speaks quietly"),
        "the asker gets the words, not the aside: {alice}"
    );
    assert!(
        lot.said.0.lock().expect("lock").is_empty(),
        "a private reply is not a Said event"
    );
    let clerk = memory_of(&lot.tw, lot.clerk);
    assert!(
        !clerk.contains("Confidential"),
        "the text stayed out of memory: {clerk}"
    );
    assert!(clerk.contains("privately"), "{clerk}");
    let bystander = memory_of(&lot.tw, lot.bystander);
    assert!(
        !bystander.contains("Confidential"),
        "another NPC overheard: {bystander}"
    );
}

#[test]
fn a_public_reply_is_said_aloud() {
    let mut lot = lot();
    let events = ability_turn(&mut lot.tw, lot.clerk, lot.alice, 43, "probe.public");
    let bob = text(&lot.tw, &events, lot.bob);
    assert!(bob.contains("The sky is blue today."), "{bob}");
    assert!(!bob.contains("speaks quietly"), "{bob}");
    let said = lot.said.0.lock().expect("lock").clone();
    assert_eq!(said, vec!["The sky is blue today.".to_string()]);
    let clerk = memory_of(&lot.tw, lot.clerk);
    assert!(clerk.contains("The sky is blue today."), "{clerk}");
}

#[test]
fn the_default_audience_is_the_asker() {
    let reply = InterfaceReply::text("x");
    assert_eq!(reply.audience, mush_core::ReplyAudience::Asker);
    assert_eq!(
        InterfaceReply::lines(&["a".to_string()]).audience,
        mush_core::ReplyAudience::Asker
    );
    assert_eq!(
        InterfaceReply::public("x").audience,
        mush_core::ReplyAudience::Room
    );
    assert_eq!(
        InterfaceReply::public_lines(&["a".to_string()]).audience,
        mush_core::ReplyAudience::Room
    );
    // An older encoding without the field decodes as private.
    let decoded: InterfaceReply = serde_json::from_str(r#"{"text":"x"}"#).unwrap();
    assert_eq!(decoded.audience, mush_core::ReplyAudience::Asker);
}
