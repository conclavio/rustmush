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
        ["probe.private", "probe.public", "probe.refuse"]
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
            "probe.refuse" => Err(InterfaceError::Refused(SECRET.to_owned())),
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
        reg.hook(mush_core::HookPoint::Perceive, Arc::new(CloakHook));
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

struct CloakHook;

impl mush_core::EventHook for CloakHook {
    fn perceives(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        _observer: ObjId,
        subject: ObjId,
    ) -> mush_core::Perceive {
        if ctx.world().attr(subject, "test_cloaked").is_some() {
            mush_core::Perceive::Conceal
        } else {
            mush_core::Perceive::Abstain
        }
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
        Box::new(mush_faction::FactionModule),
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

#[test]
fn refusal_details_are_private_and_absent_from_memory_and_observers() {
    let mut lot = lot();
    let events = ability_turn(&mut lot.tw, lot.clerk, lot.alice, 49, "probe.refuse");
    assert!(text(&lot.tw, &events, lot.alice).contains(SECRET));
    assert!(!text(&lot.tw, &events, lot.bob).contains(SECRET));
    assert!(!memory_of(&lot.tw, lot.clerk).contains(SECRET));
    assert!(!memory_of(&lot.tw, lot.bystander).contains(SECRET));
    assert!(
        lot.said
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|line| !line.contains(SECRET))
    );
}

#[test]
fn personal_memory_persists_public_results_but_never_private_answers_or_refusals() {
    let mut lot = lot();
    lot.tw.set_now(1000);
    lot.tw
        .run(lot.alice, "say Clerk, what is the sky like today?");
    ability_turn(&mut lot.tw, lot.clerk, lot.alice, 70, "probe.public");
    let before = lot
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .personal_memory
        .record_for(lot.clerk)
        .unwrap()
        .clone();
    assert!(before.experiences.iter().any(|experience| {
        experience.kind == mush_npc::ExperienceKind::Outcome
            && experience.text.contains("The sky is blue today.")
    }));
    ability_turn(&mut lot.tw, lot.clerk, lot.alice, 71, "probe.private");
    ability_turn(&mut lot.tw, lot.clerk, lot.alice, 72, "probe.refuse");
    let host = lot
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    let after = host.personal_memory.record_for(lot.clerk).unwrap();
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(&before).unwrap(),
        "private delivery markers cannot become new public evidence"
    );
    let bystander = serde_json::to_string(&host.personal_memory.record_for(lot.bystander)).unwrap();
    let boot = mush_testkit::fold_module_ops(lot.tw.take_module_ops());
    let persisted = serde_json::to_string(&boot["npc"].kv).unwrap();
    assert!(persisted.contains("The sky is blue today."));
    let context = scene_context(&mut lot.tw, lot.clerk);
    let recollections = context["working_memory"]["recollections"].as_str().unwrap();
    assert!(recollections.len() <= 8 * 1024);
    let rendered_context = serde_json::to_string(&context).unwrap();
    for text in [bystander, persisted, rendered_context] {
        for private in [SECRET, "Confidential", "ticket #7", "That is all."] {
            assert!(
                !text.contains(private),
                "private durable-memory leak: {text}"
            );
        }
    }
}

#[test]
fn delayed_private_answers_refresh_only_their_existing_exchange() {
    let mut lot = lot();
    let room = lot.tw.world().get(lot.clerk).unwrap().location.unwrap();
    lot.tw
        .world_mut()
        .module_state_mut::<mush_npc::NpcHostState>()
        .unwrap()
        .runtime_mut(lot.clerk)
        .note_conversation(lot.alice, room, 0);
    lot.tw.set_now(121);
    ability_turn(&mut lot.tw, lot.clerk, lot.alice, 50, "probe.private");
    let host = lot
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert_eq!(
        host.runtime(lot.clerk).unwrap().focus_for(room, 122_000),
        Some(lot.alice)
    );
    assert!(!memory_of(&lot.tw, lot.clerk).contains(SECRET));

    lot.tw
        .world_mut()
        .module_state_mut::<mush_npc::NpcHostState>()
        .unwrap()
        .runtime_mut(lot.clerk)
        .note_conversation(lot.bob, room, 122_000);
    lot.tw.set_now(123);
    ability_turn(&mut lot.tw, lot.clerk, lot.alice, 51, "probe.private");
    let host = lot
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert_eq!(
        host.runtime(lot.clerk).unwrap().focus_for(room, 124_000),
        Some(lot.bob)
    );
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

fn dialogue_snapshot(tw: &mut TestWorld, npc: ObjId, speaker: ObjId) -> mush_npc::TurnSnapshot {
    tw.run(speaker, "say Clerk, how should we proceed?");
    let next = tw.world().now() + 3;
    tw.set_now(next);
    let (mut out, mut effects) = (Outbox::new(), Effects::new());
    tw.engine_mut()
        .fire_due(next * 1000, &mut out, &mut effects);
    effects
        .drain()
        .into_iter()
        .find_map(|effect| match effect {
            mush_core::Effect::ModuleAsync {
                module,
                actor,
                payload,
                ..
            } if module == "npc" && actor == npc => Some(serde_json::from_str(&payload).unwrap()),
            _ => None,
        })
        .expect("captured dialogue")
}

#[test]
fn scene_and_dialogue_share_qualitative_stances_without_private_reasons_or_principals() {
    let mut lot = lot();
    lot.tw.set_now(1000);
    lot.tw.run(GOD, "@faction/create PublicDesk");
    lot.tw.run(GOD, "@faction/create UnspokenPrincipal");
    let factions = lot
        .tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .unwrap();
    let public = factions.faction_by_name("PublicDesk").unwrap().id;
    let private = factions.faction_by_name("UnspokenPrincipal").unwrap().id;
    let profile = mush_npc::AgentProfile {
        public_faction: Some(public),
        principal: Some(mush_faction::Principal::faction(private)),
        ..mush_npc::AgentProfile::default()
    };
    lot.tw
        .world_mut()
        .set_attr(
            lot.clerk,
            mush_npc::PROFILE_ATTR,
            mush_core::Attr::json(serde_json::to_value(profile).unwrap(), GOD),
        )
        .unwrap();
    for (person, trust, disposition, receipt) in [
        (lot.alice, 30, -30, "private_alice_receipt"),
        (lot.bob, -30, 30, "private_bob_receipt"),
    ] {
        let output = lot.tw.run(GOD, &format!(
            "@agent/relationship #{}/#{}=trust:{trust}; disposition:{disposition}; cause:{receipt}; reason:Sealed staff explanation",
            lot.clerk.0, person.0
        ));
        assert!(text(&lot.tw, &output, GOD).contains("Relationship"));
    }
    let snapshot = dialogue_snapshot(&mut lot.tw, lot.clerk, lot.alice);
    let prompt = mush_npc::user_prompt(&snapshot);
    let context = scene_context(&mut lot.tw, lot.clerk);
    let social = &context["social_context"];
    assert!(prompt.contains(social["guidance"].as_str().unwrap()));
    let people = social["people"].as_array().unwrap();
    assert_eq!(people.len(), 2);
    for person in people {
        let stance = person["stance"].as_str().unwrap();
        assert!(
            prompt.contains(stance),
            "scene/dialogue differ: {stance}\n{prompt}"
        );
        assert!(stance.contains("Public standing with PublicDesk: neutral"));
        if person["name"] == "Alice" {
            assert!(stance.contains("Personal trust: trusting"));
            assert!(stance.contains("disposition: cold"));
        } else {
            assert_eq!(person["name"], "Bob");
            assert!(stance.contains("Personal trust: distrustful"));
            assert!(stance.contains("disposition: friendly"));
        }
    }
    let rendered = serde_json::to_string(&context).unwrap();
    for withheld in [
        "UnspokenPrincipal",
        "private_alice_receipt",
        "private_bob_receipt",
        "Sealed staff explanation",
    ] {
        assert!(!prompt.contains(withheld), "{prompt}");
        assert!(!rendered.contains(withheld), "{rendered}");
    }
    for metadata in [
        "\"character\"",
        "\"npc\"",
        "\"principal\"",
        "\"cause\"",
        "\"trust\"",
        "\"score\"",
    ] {
        assert!(
            !rendered.contains(metadata),
            "social metadata leaked: {rendered}"
        );
    }
    lot.tw.run(GOD, "@faction/set PublicDesk=hidden:yes");
    let hidden = scene_context(&mut lot.tw, lot.clerk);
    assert!(!hidden.to_string().contains("PublicDesk"));
}

#[test]
fn live_scene_and_dialogue_omit_concealed_people_and_their_relationships() {
    let mut lot = lot();
    lot.tw.set_now(1000);
    lot.tw
        .world_mut()
        .set_attr(lot.bob, "test_cloaked", mush_core::Attr::text("yes", GOD))
        .unwrap();
    lot.tw.run(GOD, &format!(
        "@agent/relationship #{}/#{}=trust:50; disposition:50; cause:hidden_contact; reason:Private hidden history",
        lot.clerk.0, lot.bob.0
    ));
    let snapshot = dialogue_snapshot(&mut lot.tw, lot.clerk, lot.alice);
    assert!(
        snapshot
            .scene
            .present
            .iter()
            .all(|person| person.name != "Bob")
    );
    let context = scene_context(&mut lot.tw, lot.clerk);
    assert_eq!(context["present"], serde_json::json!(["Alice"]));
    let people = context["social_context"]["people"].as_array().unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0]["name"], "Alice");
    assert!(
        !serde_json::to_string(&context)
            .unwrap()
            .contains("Private hidden history")
    );
}

#[test]
fn scene_recall_tracks_the_observed_identity_after_rename_without_rewriting_evidence() {
    let mut lot = lot();
    lot.tw.set_now(1000);
    lot.tw
        .run(lot.alice, "say Clerk, the original manifest was delayed.");
    lot.tw.set_now(1001);
    lot.tw.run(lot.bob, "say Clerk, the garden is quiet today.");
    let before = lot
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .personal_memory
        .record_for(lot.clerk)
        .cloned()
        .unwrap();
    lot.tw.run(GOD, &format!("@name #{}=Alicia", lot.alice.0));
    assert_eq!(
        lot.tw.world().get(lot.alice).unwrap().name.as_str(),
        "Alicia"
    );
    let room = lot.tw.world().get(lot.clerk).unwrap().location.unwrap();
    let namesake = lot.tw.spawn_player("Alice");
    lot.tw.world_mut().move_to(namesake, room).unwrap();
    lot.tw
        .world_mut()
        .set_attr(lot.bob, "test_cloaked", mush_core::Attr::text("yes", GOD))
        .unwrap();
    lot.tw
        .world_mut()
        .module_state_mut::<mush_npc::NpcHostState>()
        .unwrap()
        .config
        .recall_top_n = 1;
    let context = scene_context(&mut lot.tw, lot.clerk);
    let recalled = context["working_memory"]["recollections"].as_str().unwrap();
    assert!(recalled.contains("original manifest"), "{recalled}");
    assert!(
        recalled.contains("same observed character now known here as \"Alicia\""),
        "{recalled}"
    );
    let elsewhere = lot
        .tw
        .world_mut()
        .create(ObjKind::Room, "Elsewhere", GOD, None)
        .unwrap();
    lot.tw.world_mut().move_to(lot.alice, elsewhere).unwrap();
    let namesake_only = scene_context(&mut lot.tw, lot.clerk);
    let recalled = namesake_only["working_memory"]["recollections"]
        .as_str()
        .unwrap();
    assert!(
        recalled.contains("garden is quiet"),
        "a namesake must not boost the old speaker: {recalled}"
    );
    assert!(!recalled.contains("original manifest"));
    assert_eq!(
        lot.tw
            .world()
            .module_state::<mush_npc::NpcHostState>()
            .unwrap()
            .personal_memory
            .record_for(lot.clerk),
        Some(&before)
    );
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
    assert_eq!(fields.len(), 4);
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
    let description = "\"".repeat(8 * 1024);
    let persona = "\"".repeat(4 * 1024);
    lot.tw
        .world_mut()
        .set_attr(room, "desc", mush_core::Attr::text(&description, GOD))
        .unwrap();
    let profile = mush_npc::AgentProfile {
        persona: persona.clone(),
        interfaces: vec!["probe".into()],
        ..mush_npc::AgentProfile::default()
    };
    lot.tw
        .world_mut()
        .set_attr(
            lot.clerk,
            mush_npc::PROFILE_ATTR,
            mush_core::Attr::json(serde_json::to_value(profile).unwrap(), GOD),
        )
        .unwrap();
    {
        let host = lot
            .tw
            .world_mut()
            .module_state_mut::<mush_npc::NpcHostState>()
            .unwrap();
        host.config.transcript_chars = usize::MAX;
        for at in 0..5 {
            host.personal_memory
                .observe(
                    lot.clerk,
                    "Alice",
                    &"\"".repeat(3000),
                    Some(lot.alice),
                    "Alice",
                    Some(room),
                    mush_npc::ExperienceKind::Speech,
                    at,
                )
                .unwrap();
        }
        let memory = &mut host.runtime_mut(lot.clerk).memory;
        // Legacy or staff-loaded summary text still has to fit the scene's
        // byte budget; multibyte text and JSON quotes exercise both encodings.
        memory.summary = format!("記憶{}", "\"".repeat(10_000));
        for turn in 0..100 {
            memory.push_event(
                turn,
                format!("Alice asks about survey {turn}: {}", "\"".repeat(2000)),
                mush_npc::MemoryContext {
                    speaker: Some(lot.alice),
                    room: Some(room),
                    addressee: Some(lot.clerk),
                    kind: mush_npc::MemoryKind::DirectedSpeech,
                },
            );
        }
    }
    let untrimmed_recollections = lot
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .personal_memory
        .recall(
            lot.clerk,
            &["Alice".into(), "Bob".into()],
            "",
            lot.tw.world().now(),
            5,
        )
        .iter()
        .flat_map(|memory| {
            memory
                .notes
                .iter()
                .map(|note| format!("{}: {note}", memory.subject))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let context = scene_context(&mut lot.tw, lot.clerk);
    let working = &context["working_memory"];
    let summary = working["summary"].as_str().unwrap();
    let recent = working["recent_events"].as_str().unwrap();
    let recollections = working["recollections"].as_str().unwrap();
    assert!(!recollections.is_empty() && recollections.len() <= 8 * 1024);
    assert!(
        recollections.len() < untrimmed_recollections.len(),
        "escaped document size must trim old recollections first"
    );
    assert!(!summary.is_empty() && summary.len() <= 8 * 1024);
    assert!(!recent.is_empty() && recent.len() <= 8 * 1024);
    assert!(recent.contains("survey 99"), "newest question retained");
    assert_eq!(context["description"].as_str().unwrap(), description);
    assert_eq!(context["persona"].as_str().unwrap(), persona);
    assert!(serde_json::to_vec(&context).unwrap().len() < mush_core::MAX_DECISION_CONTEXT_BYTES);
    let mut oversized = context.clone();
    oversized["working_memory"]["recollections"] = untrimmed_recollections.into();
    assert!(
        mush_core::ProjectedDecisionContext::new(
            mush_core::CanonicalId::new(mush_npc::NPC_CONTEXT_SCHEMA).unwrap(),
            oversized,
        )
        .is_err(),
        "the fixture must exercise the total encoded limit"
    );
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
