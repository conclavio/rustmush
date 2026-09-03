//! M13 task 2 end-to-end: the generic `+action` adjudicator and the SILENT
//! per-perceiver witnesses, driven through the real [`Engine`](mush_core) via
//! [`mush_testkit::TestWorld`] against a network-free
//! [`MockProvider`](mush_llm::MockProvider).
//!
//! The full two-phase spine for a mundane action: `+action <freeform> using
//! <pool>` defers a ruling; the off-loop pass ([`run_st_async`]) returns a
//! scripted ruling; the completion re-validates + stores a pending action;
//! `+roll/silent` throws the ENGINE's dice (seeded) and defers an
//! interpretation; the interpretation pass shows the actor their apparent result
//! and then runs the SILENT WITNESS resolution — a tailored line reaches only
//! the observers who make their Perception + Awareness roll, and a distracted /
//! low-Perception observer receives NO event at all (A5 true silence).
//!
//! Also proves the Mage `+cast` accept→Paradox path and a reject path end-to-end
//! through the same machinery.

use std::sync::Arc;

use mush_core::{Audience, Effect, Effects, GOD, Inbound, ObjId, Outbox, WorldCommand};
use mush_jobs::JobsModule;
use mush_llm::{
    CompletionResponse, ContentBlock, LlmProvider, LlmService, MockProvider, RouteConfig,
    StopReason, Usage,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use ruleset_wod::WodRuleset;
use ruleset_wod::state::WodState;
use serde_json::json;

const S: u64 = 777;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

fn route() -> RouteConfig {
    RouteConfig {
        model: "test/mock".into(),
        max_tokens: 1024,
        timeout_s: 30,
        reasoning: None,
        reasoning_max_tokens: None,
    }
}

fn service(mock: &Arc<MockProvider>) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route("st_adjudicate", route())
            .route("st_interpret", route())
            .provider("openrouter", p)
            .global_enabled(true)
            .build(),
    )
}

fn base_world() -> (TestWorld, Arc<MockProvider>, Arc<LlmService>) {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = service(&mock);
    let tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S)),
    ]);
    (tw, mock, llm)
}

fn ruling_resp(input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "r1".into(),
            name: "submit_ruling".into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test/mock".into(),
    }
}

fn interp_resp(input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "i1".into(),
            name: "submit_interpretation".into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test/mock".into(),
    }
}

fn step_cmd(
    tw: &mut TestWorld,
    actor: ObjId,
    line: &str,
) -> (Vec<(Audience, OutputEvent)>, Vec<Effect>) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Cmd {
            conn: 0,
            actor,
            line: line.to_string(),
        },
        &mut out,
        &mut fx,
    );
    (out.take(), fx.drain())
}

fn apply_done(
    tw: &mut TestWorld,
    module: &str,
    actor: ObjId,
    correlation: u64,
    payload: String,
) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: module.into(),
            actor,
            correlation,
            payload,
        }),
        &mut out,
        &mut fx,
    );
    out.take()
}

fn module_async_opt(effects: &[Effect]) -> Option<(String, u64, String)> {
    effects.iter().find_map(|e| {
        if let Effect::ModuleAsync {
            module,
            correlation,
            payload,
            ..
        } = e
        {
            Some((module.to_string(), *correlation, payload.clone()))
        } else {
            None
        }
    })
}

fn module_async(effects: &[Effect]) -> (String, u64, String) {
    module_async_opt(effects).unwrap_or_else(|| panic!("expected one Effect::ModuleAsync"))
}

/// The system lines addressed specifically to `who`.
fn sys_to(events: &[(Audience, OutputEvent)], who: ObjId) -> Vec<String> {
    events
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Sys(t)) if *a == who => Some(t.to_plain_text()),
            _ => None,
        })
        .collect()
}

/// EVERY event addressed to `who` (any kind) — the A5 true-silence probe: a
/// witness who fails must have an empty list here (no wire artifact at all).
fn events_for(events: &[(Audience, OutputEvent)], who: ObjId) -> Vec<&OutputEvent> {
    events
        .iter()
        .filter_map(|(aud, e)| match aud {
            Audience::Actor(a) if *a == who => Some(e),
            _ => None,
        })
        .collect()
}

fn contains(lines: &[String], needle: &str) -> bool {
    lines.iter().any(|l| l.contains(needle))
}

fn pending_exists(tw: &TestWorld, who: ObjId) -> bool {
    tw.world()
        .module_state::<WodState>()
        .expect("WodState")
        .pending(who)
        .is_some()
}

async fn run(llm: &LlmService, payload: &str) -> String {
    ruleset_wod::run_st_async(llm, payload).await
}

// ----------------------------------------------------------------------
// The headline e2e: +action -> ruling -> +roll/silent -> interpret + witnesses.
// A sharp-eyed onlooker notices; a distracted one gets true silence (A5).
// ----------------------------------------------------------------------

#[tokio::test]
async fn generic_action_silent_witnesses_only_the_sharp_eyed_notice() {
    let (mut tw, mock, llm) = base_world();
    tw.set_now(1000);

    // Alice acts; Hawk is sharp-eyed (Per 5 + Awa 5); Dozer is distracted
    // (Per 0 + Awa 0 => a single die, at the ST's furtive difficulty).
    let alice = tw.spawn_player("Alice");
    tw.run(GOD, "@sheet/set Alice/dexterity=4");
    tw.run(GOD, "@sheet/set Alice/subterfuge=3");
    let hawk = tw.spawn_player("Hawk");
    tw.run(GOD, "@sheet/set Hawk/perception=5");
    tw.run(GOD, "@sheet/set Hawk/awareness=5");
    let dozer = tw.spawn_player("Dozer");
    tw.run(GOD, "@sheet/set Dozer/perception=1");

    // PROPOSE. Nothing is rolled; a ruling is deferred.
    let (open, effects) = step_cmd(
        &mut tw,
        alice,
        "+action slip powdered nightshade into Mary's wineglass using dexterity+subterfuge",
    );
    assert!(contains(&sys_to(&open, alice), "Storyteller"));
    let (module, corr, payload) = module_async(&effects);
    assert_eq!(module, "wod");

    // RULING: accept, a hard-to-notice furtive act (witness difficulty 8).
    mock.push_response(ruling_resp(json!({
        "accept": true,
        "spheres_valid": true,
        "pool": "dexterity+subterfuge",
        "difficulty": 8,
        "paradox": 0,
        "witness_difficulty": 8
    })));
    let done = apply_done(&mut tw, &module, alice, corr, run(&llm, &payload).await);
    assert!(
        contains(
            &sys_to(&done, alice),
            "Roll dexterity+subterfuge at difficulty 8"
        ),
        "{:?}",
        sys_to(&done, alice)
    );
    assert!(pending_exists(&tw, alice));

    // ALICE THROWS. The room gets no dice line (silent); only Alice sees her own.
    let (roll_ev, roll_fx) = step_cmd(&mut tw, alice, "+roll/silent");
    assert!(
        roll_ev
            .iter()
            .all(|(aud, _)| matches!(aud, Audience::Actor(_))),
        "the action roll emits nothing to the room: {roll_ev:?}"
    );
    assert!(contains(&sys_to(&roll_ev, alice), "attempts the action"));
    assert!(!pending_exists(&tw, alice), "pending consumed exactly once");
    let (imod, icorr, ipayload) = module_async(&roll_fx);

    // INTERPRET: the ST narrates the actor result + the witness line.
    mock.push_response(interp_resp(json!({
        "actor_result": "You tip the vial; the powder vanishes into the red wine.",
        "witnessed": "You catch Alice doing something furtive to Mary's glass."
    })));
    let idone = apply_done(&mut tw, &imod, alice, icorr, run(&llm, &ipayload).await);

    // The actor sees her own apparent result.
    assert!(
        contains(&sys_to(&idone, alice), "powder vanishes"),
        "{:?}",
        sys_to(&idone, alice)
    );
    // The sharp-eyed onlooker gets the tailored witness line.
    assert!(
        contains(&sys_to(&idone, hawk), "something furtive to Mary's glass"),
        "the sharp-eyed witness notices: {:?}",
        sys_to(&idone, hawk)
    );
    // A5 TRUE SILENCE: the distracted onlooker receives NO event of any kind.
    assert!(
        events_for(&idone, dozer).is_empty(),
        "the distracted witness gets no wire artifact at all: {:?}",
        events_for(&idone, dozer)
    );
}

// ----------------------------------------------------------------------
// A generic action mutates no sheet: even on a failure/botch, no Paradox and
// no other sheet write (there is no paradox field on a mortal anyway).
// ----------------------------------------------------------------------

#[tokio::test]
async fn generic_action_reject_path_stores_no_pending() {
    let (mut tw, mock, llm) = base_world();
    let alice = tw.spawn_player("Alice");
    tw.run(GOD, "@sheet/set Alice/dexterity=4");

    let (_open, effects) = step_cmd(
        &mut tw,
        alice,
        "+action fly to the moon by flapping my arms using dexterity",
    );
    let (module, corr, payload) = module_async(&effects);
    mock.push_response(ruling_resp(json!({
        "accept": false,
        "spheres_valid": true,
        "reject_reason": "You cannot fly unaided",
        "pool": "dexterity",
        "difficulty": 6,
        "paradox": 0,
        "witness_difficulty": 6
    })));
    let done = apply_done(&mut tw, &module, alice, corr, run(&llm, &payload).await);
    assert!(
        contains(
            &sys_to(&done, alice),
            "rejects the action: You cannot fly unaided"
        ),
        "{:?}",
        sys_to(&done, alice)
    );
    assert!(!pending_exists(&tw, alice));

    // A later +roll is an ordinary roll (no pending to consume, nothing deferred).
    let (_ev, fx) = step_cmd(&mut tw, alice, "+roll dexterity");
    assert!(
        module_async_opt(&fx).is_none(),
        "an ordinary roll defers nothing"
    );
}

// ----------------------------------------------------------------------
// The Mage accept -> Paradox path, end-to-end through the same machinery, with
// witnesses wired in (a witness with no perception still gets true silence).
// ----------------------------------------------------------------------

#[tokio::test]
async fn mage_cast_accept_applies_paradox_and_runs_witnesses() {
    let (mut tw, mock, llm) = base_world();
    tw.set_now(1000);

    let nick = tw.spawn_player("Nick");
    tw.run(GOD, "@sheet/set Nick/template=mage");
    tw.run(GOD, "@sheet/set Nick/arete=4");
    tw.run(GOD, "@sheet/set Nick/spheres:Forces=2");
    let mort = tw.spawn_player("Mort"); // no perception -> true silence

    let (_open, effects) = step_cmd(
        &mut tw,
        nick,
        "+cast/vulgar hurl a bolt of fire proposing Forces",
    );
    let (module, corr, payload) = module_async(&effects);
    mock.push_response(ruling_resp(json!({
        "accept": true,
        "spheres_valid": true,
        "pool": "arete",
        "difficulty": 7,
        "paradox": 3,
        "witness_difficulty": 9
    })));
    apply_done(&mut tw, &module, nick, corr, run(&llm, &payload).await);
    assert!(pending_exists(&tw, nick));

    let (_roll_ev, roll_fx) = step_cmd(&mut tw, nick, "+roll/silent");
    let (imod, icorr, ipayload) = module_async(&roll_fx);
    mock.push_response(interp_resp(json!({
        "actor_result": "Flame erupts from your outstretched hand.",
        "witnessed": "You see Nick wreathe his hand in fire."
    })));
    let idone = apply_done(&mut tw, &imod, nick, icorr, run(&llm, &ipayload).await);

    assert!(contains(&sys_to(&idone, nick), "Flame erupts"));
    assert!(contains(&sys_to(&idone, nick), "Paradox"));
    // The vulgar working's 3 Paradox landed on the sheet.
    let sheet = tw
        .world()
        .attr(nick, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .and_then(|v| serde_json::from_value::<mush_ruleset::SheetDoc>(v.clone()).ok())
        .expect("a mage sheet");
    assert_eq!(sheet.number("paradox"), Some(3));
    // Mort has no perception -> true silence (A5).
    assert!(
        events_for(&idone, mort).is_empty(),
        "a witness with no perception gets nothing: {:?}",
        events_for(&idone, mort)
    );
}
