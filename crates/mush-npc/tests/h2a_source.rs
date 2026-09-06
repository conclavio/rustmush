//! H2a: the `npc` decision source (`docs/design/11-h-implementation-plan.md`
//! §3.2). The families and definitions register under the module's owner;
//! `npc_idle` and `npc_decide` are projected for an NPC agent and for nobody
//! else; the idle fallback selects `idle` and errs without it; a queued request
//! becomes exactly one kernel capture on the next tick, for an agent only.

use mush_core::{
    CanonicalHash, CanonicalId, DecisionKeyRing, DecisionPrompt, DecisionSecret, DecisionToken,
    GOD, IdempotencyKeyText, ObjId, OfferKeyRing, OfferSecret, PerceivedRef, PrincipalId,
    ProjectedDecisionContext,
};
use mush_npc::{
    DECIDE_VERB, IDLE_VERB, MAX_DECISION_REQUESTS, NPC_CONTEXT_SCHEMA, NpcDecisionHandles,
    NpcDecisionRequest, NpcHostState, NpcModule, select_idle,
};
use mush_testkit::{TestWorld, text_of};

fn boot() -> TestWorld {
    let mut tw = TestWorld::with_modules(vec![Box::new(NpcModule::new())]);
    tw.engine_mut().set_decision_keys(
        DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).expect("secret"), [])
            .expect("key ring"),
    );
    tw.engine_mut().set_offer_keys(
        OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).expect("secret"), [])
            .expect("offer keys"),
    );
    tw
}

/// `@agent/create` an NPC beside God and return its id.
fn create_agent(tw: &mut TestWorld, name: &str) -> ObjId {
    let events = tw.run(GOD, &format!("@agent/create {name}"));
    let text: String = events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n");
    let at = text.find("(#").expect("the create line names the id");
    let digits: String = text[at + 2..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    ObjId(digits.parse().expect("an object id"))
}

fn projected_verbs(tw: &TestWorld, actor: ObjId) -> Vec<String> {
    let engine = tw.engine();
    let query = engine
        .legal_action_query(
            PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .expect("a self query");
    engine
        .project_legal_actions(&query)
        .expect("projection")
        .projected()
        .into_iter()
        .map(|action| action.presentation().verb.clone())
        .collect()
}

fn request(npc: ObjId, tag: &str) -> NpcDecisionRequest {
    NpcDecisionRequest {
        npc,
        cause_kind: CanonicalId::new("npc.test").unwrap(),
        instance: IdempotencyKeyText::parse_canonical(format!("test-{tag}")).unwrap(),
        replaceable_key: None,
        context: serde_json::json!({ "tag": tag }),
        deadline_secs: 60,
        key: IdempotencyKeyText::parse_canonical(format!("key-{tag}")).unwrap(),
    }
}

#[test]
fn h2a_the_source_registers_its_families_and_definitions() {
    let tw = boot();
    let handles = tw
        .world()
        .module_state::<NpcDecisionHandles>()
        .expect("the source publishes its handles");
    assert_eq!(handles.decide.key().id.as_str(), "npc_decide");
    assert_eq!(handles.lease.key().id.as_str(), "npc_lease");
    assert_eq!(handles.idle.key().id.as_str(), "npc_idle");
    assert_eq!(handles.families.owner().as_str(), "npc");
    assert!(!handles.sink.is_armed());
    let sources = tw
        .registry()
        .decision_sources()
        .expect("a composed decision-source registry");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources.wake_adapters()[0].0.as_str(), "npc");
    assert_eq!(sources.wake_adapters()[0].1.as_str(), "npc_wake");
}

#[test]
fn h2a_idle_and_decide_are_projected_for_an_agent_and_for_nobody_else() {
    let mut tw = boot();
    let npc = create_agent(&mut tw, "Bob");
    let verbs = projected_verbs(&tw, npc);
    assert!(verbs.iter().any(|v| v == IDLE_VERB), "{verbs:?}");
    assert!(verbs.iter().any(|v| v == DECIDE_VERB), "{verbs:?}");
    // A player projects neither.
    let player = tw.spawn_player("Pat");
    let verbs = projected_verbs(&tw, player);
    assert!(
        !verbs.iter().any(|v| v == IDLE_VERB || v == DECIDE_VERB),
        "{verbs:?}"
    );
    // Nor does a plain thing.
    let thing = tw.run(GOD, "@create rock");
    assert!(!thing.is_empty());
}

#[test]
fn h2a_the_idle_fallback_selects_idle_and_errs_without_it() {
    let mut tw = boot();
    let npc = create_agent(&mut tw, "Bob");
    let engine = tw.engine();
    let query = engine
        .legal_action_query(
            PrincipalId::Object { id: npc },
            npc,
            PerceivedRef::SelfActor,
        )
        .expect("query");
    let offers = engine.project_decision_offers(&query).expect("offers");
    let (actions, _) = offers.into_parts();
    let idle_offer = actions
        .iter()
        .find(|a| a.presentation().verb == IDLE_VERB)
        .expect("idle offered")
        .offer();
    let token = DecisionToken {
        id: mush_core::DecisionId(CanonicalHash::from_bytes([1; 32])),
        revision: 1,
        prompt_digest: CanonicalHash::from_bytes([2; 32]),
        key_version: 1,
        tag: CanonicalHash::from_bytes([3; 32]),
    };
    let context = ProjectedDecisionContext::new(
        CanonicalId::new(NPC_CONTEXT_SCHEMA).unwrap(),
        serde_json::json!({}),
    )
    .unwrap();
    let prompt = DecisionPrompt {
        token,
        actions: actions.clone(),
        context: context.clone(),
    };
    let selection = select_idle(&prompt).expect("idle selected");
    assert_eq!(selection.offer, idle_offer);
    assert!(selection.targets.is_empty());
    let without: Vec<_> = actions
        .into_iter()
        .filter(|a| a.presentation().verb != IDLE_VERB)
        .collect();
    let prompt = DecisionPrompt {
        token,
        actions: without,
        context,
    };
    assert!(
        select_idle(&prompt).is_err(),
        "a prompt without idle is an error, not a guess"
    );
}

#[test]
fn h2a_a_queued_request_becomes_one_capture_on_the_next_tick() {
    let mut tw = boot();
    let npc = create_agent(&mut tw, "Bob");
    {
        let host = tw
            .world_mut()
            .module_state_mut::<NpcHostState>()
            .expect("host state");
        assert!(host.request_decision(request(npc, "a")));
        assert!(host.request_decision(request(npc, "b")));
    }
    assert!(!tw.engine().has_decision_capture());
    // The agent tick drains ONE request per step.
    let _ = tw.fire_due(5_000);
    assert!(
        tw.engine().has_decision_capture(),
        "the tick did not issue the capture"
    );
    let pending = tw
        .world()
        .module_state::<NpcHostState>()
        .expect("host")
        .decision_requests
        .len();
    assert_eq!(pending, 1, "the second request waits for the next step");
    let capture = tw
        .engine_mut()
        .take_decision_capture()
        .expect("queued capture");
    assert_eq!(capture.owner.as_str(), "npc");
    assert_eq!(capture.draft.actor, npc);
    assert_eq!(capture.draft.subject, PerceivedRef::SelfActor);
    assert_eq!(capture.draft.fallback.0.key().id.as_str(), "npc_idle");
    assert_eq!(capture.draft.deadline_secs, 60);
}

#[test]
fn h2a_a_request_for_a_player_or_a_full_queue_is_dropped() {
    let mut tw = boot();
    let player = tw.spawn_player("Pat");
    {
        let host = tw
            .world_mut()
            .module_state_mut::<NpcHostState>()
            .expect("host state");
        assert!(host.request_decision(request(player, "p")));
    }
    let _ = tw.fire_due(5_000);
    assert!(
        !tw.engine().has_decision_capture(),
        "a player is not an agent; nothing may be captured for one"
    );
    // The queue is bounded.
    let npc = create_agent(&mut tw, "Bob");
    let host = tw
        .world_mut()
        .module_state_mut::<NpcHostState>()
        .expect("host state");
    for n in 0..MAX_DECISION_REQUESTS {
        assert!(host.request_decision(request(npc, &format!("q{n}"))));
    }
    assert!(
        !host.request_decision(request(npc, "overflow")),
        "the {MAX_DECISION_REQUESTS}th request is refused, not queued"
    );
}

/// The kernel builds the whole plan from the selector — offers, witnesses, a
/// signed prompt, the deadline on the UTC clock — and refuses an actor whose
/// projection does not carry the fallback (a player projects no `npc_idle`).
#[test]
fn h2a_the_kernel_builds_a_capture_plan_for_an_agent_and_refuses_a_player() {
    let mut tw = boot();
    let npc = create_agent(&mut tw, "Bob");
    let player = tw.spawn_player("Pat");
    tw.set_now(1_000);
    let handles = tw
        .world()
        .module_state::<NpcDecisionHandles>()
        .expect("handles")
        .clone();
    let owner = mush_core::OwnerId::new("npc").unwrap();
    let draft = |actor: ObjId| mush_core::DecisionCaptureDraft {
        actor,
        subject: PerceivedRef::SelfActor,
        controller_key: CanonicalId::new("npc").unwrap(),
        cause: mush_core::DecisionCause {
            kind: CanonicalId::new("npc.test").unwrap(),
            instance: IdempotencyKeyText::parse_canonical("turn-9").unwrap(),
            replaceable_key: None,
        },
        context: ProjectedDecisionContext::new(
            CanonicalId::new(NPC_CONTEXT_SCHEMA).unwrap(),
            serde_json::json!({ "room": "Limbo" }),
        )
        .unwrap(),
        deadline_secs: 45,
        fallback: mush_core::FallbackPolicyRef(handles.idle.clone()),
        key: IdempotencyKeyText::parse_canonical("turn-9").unwrap(),
    };
    let plan = tw
        .engine()
        .build_decision_capture(&owner, &draft(npc), 1)
        .expect("an agent's plan builds");
    assert_eq!(plan.pending().actor, npc);
    assert_eq!(plan.pending().controller.owner, owner);
    assert_eq!(plan.pending().deadline.at, 1_000 + 45);
    assert!(
        plan.prompt().prompt.actions.len() >= 2,
        "idle and decide are offered"
    );
    assert!(
        plan.prompt()
            .prompt
            .actions
            .iter()
            .any(|a| a.presentation().verb == IDLE_VERB),
        "the prompt carries the fallback"
    );
    let refused = tw
        .engine()
        .build_decision_capture(&owner, &draft(player), 1)
        .expect_err("a player projects no idle offer");
    assert!(
        matches!(
            refused,
            mush_core::DecisionCaptureError::FallbackNotOffered(_)
        ),
        "{refused}"
    );
    let unknown_owner = mush_core::OwnerId::new("nobody").unwrap();
    let refused = tw
        .engine()
        .build_decision_capture(&unknown_owner, &draft(npc), 1)
        .expect_err("an owner without a source");
    assert!(
        matches!(refused, mush_core::DecisionCaptureError::NoSource(_)),
        "{refused}"
    );
}
