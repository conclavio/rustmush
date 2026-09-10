//! LIVE NPC-dialogue smoke against the real OpenRouter key — the ONE NPC test
//! that makes a real LLM API call. It is **skipped by default** and makes no
//! network call unless `RUN_LLM_SMOKE=1`, so it never runs in local gates or CI
//! (every other NPC test uses a MockProvider). The orchestrator runs it once,
//! after the mock suite is green.
//!
//! It drives the exact production path an NPC dialogue turn uses — build an
//! `LlmService` from an `[llm]` block (the real `npc_dialogue_major` route,
//! resolving `OPENROUTER_API_KEY` from the environment / `.env`), assemble the
//! `TurnSnapshot` the world loop would snapshot for a bartender who was just
//! spoken to, and call `run_npc_turn_async` — then decode the typed
//! `TurnOutcome` and print the NPC's REAL generated reply. No SSH, no store, no
//! prod data touched, and (crucially) the reply still comes back only as one of
//! the allowlisted typed actions.
//!
//! Run it (from the repo root, with OPENROUTER_API_KEY in the environment or in
//! `.env`):
//!
//! ```text
//! RUN_LLM_SMOKE=1 cargo test -p mushd --test npc_smoke -- --nocapture
//! ```
//!
//! PowerShell:
//!
//! ```text
//! $env:RUN_LLM_SMOKE = "1"; cargo test -p mushd --test npc_smoke -- --nocapture
//! ```

use mush_core::ObjId;
use mush_llm::{LlmConfig, LlmService};
use mush_npc::{
    AgentProfile, Familiarity, NpcAction, NpcTier, PresentPlayer, SceneView, StimulusLine,
    TurnKind, TurnOutcome, TurnSnapshot, run_npc_turn_async,
};

#[tokio::test(flavor = "multi_thread")]
async fn live_npc_dialogue_openrouter_smoke() {
    if std::env::var("RUN_LLM_SMOKE").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping live_npc_dialogue_openrouter_smoke: set RUN_LLM_SMOKE=1 (and provide \
             OPENROUTER_API_KEY, via the environment or .env) to make the one real call"
        );
        return;
    }
    // Dev convenience: pull OPENROUTER_API_KEY from a .env if it is not already
    // exported (real env vars win). Mirrors what mushd does at boot.
    let _ = dotenvy::dotenv();
    assert!(
        std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.trim().is_empty()),
        "RUN_LLM_SMOKE=1 but OPENROUTER_API_KEY is not set (export it or put it in .env)"
    );

    // A real service: the production `npc_dialogue_major` route on a real
    // OpenRouter reasoning model, keyed from the env var by NAME (never
    // inlined). Parsed as the `[llm]` block would be, so this exercises the
    // real config path too.
    let cfg: LlmConfig = toml::from_str(
        r#"
        enabled = true

        [providers.openrouter]
        kind = "openai"
        base_url = "https://openrouter.ai/api/v1"
        api_key_env = "OPENROUTER_API_KEY"
        referer = "https://github.com/rustmush"
        title = "rustmush"

        [routes.npc_dialogue_major]
        model = "z-ai/glm-5.3"
        max_tokens = 1024
        timeout_s = 60
        reasoning = "low"
        "#,
    )
    .expect("smoke [llm] config parses");
    let llm = LlmService::from_config(&cfg).expect("build LlmService");

    // The snapshot the world loop assembles for a Major-tier bartender who was
    // just addressed by a player — the same owned data the off-loop turn runs
    // on (treaty ruling 3), with player speech that will be delimited/escaped
    // by the prompt assembler.
    let snap = TurnSnapshot {
        kind: TurnKind::Dialogue,
        npc: ObjId(2),
        npc_name: "Bartender".into(),
        profile: AgentProfile {
            tier: NpcTier::Major,
            persona: "a gruff but warm-hearted barkeep who has run this dockside tavern for \
                      thirty years"
                .into(),
            speech_style: "terse, salty, working-class".into(),
            goals: vec![
                "keep the peace in the bar".into(),
                "sell another round".into(),
            ],
            fallback_lines: vec!["The bartender grunts and keeps polishing a glass.".into()],
            ..AgentProfile::default()
        },
        agent_profile: None,
        scene: SceneView {
            room_name: "The Rusty Anchor".into(),
            room_desc: "A low, smoky dockside tavern; rain drums on the shutters.".into(),
            present: vec![PresentPlayer {
                name: "Alice".into(),
                short_desc: "a road-worn traveler in a wet cloak".into(),
                familiarity: Familiarity::default(),
            }],
            time: "late evening".into(),
            weather: Some("rain".into()),
            exits: vec!["out".into()],
        },
        knowledge: Vec::new(),
        summary: String::new(),
        recalled: Vec::new(),
        transcript: String::new(),
        stimuli: vec![StimulusLine {
            who: "Alice".into(),
            text: "Evening, barkeep. Cold night out there — what's good to drink?".into(),
            directed: true,
            tag: "say".into(),
        }],
        actor: Some(ObjId(3)),
        fold_events: Vec::new(),
        seed: 42,
        interfaces: Vec::new(),
    };
    let payload = serde_json::to_string(&snap).expect("serialize snapshot");

    let result = run_npc_turn_async(&llm, &payload).await;
    let outcome: TurnOutcome = serde_json::from_str(&result).expect("decode TurnOutcome");
    match outcome {
        TurnOutcome::Ok { actions, .. } => {
            eprintln!("live NPC turn OK — {} typed action(s):", actions.len());
            for a in &actions {
                eprintln!("  {:?}", a.action);
            }
            // The reply the player would actually hear: the first say/pose.
            let reply = actions.iter().find_map(|a| match &a.action {
                NpcAction::Say { text } | NpcAction::Pose { text } => Some(text.clone()),
                _ => None,
            });
            let reply = reply.expect("the model produced a say/pose reply, not just tool calls");
            eprintln!("\nBartender's real generated reply:\n  {reply}");
            assert!(
                !reply.trim().is_empty(),
                "the model returned an empty reply"
            );
        }
        TurnOutcome::Degraded { reason, line, .. } => {
            panic!("live NPC turn degraded (the real call failed): {reason} (fallback: {line:?})")
        }
        other => panic!("unexpected NPC outcome (no visible reply): {other:?}"),
    }
}
