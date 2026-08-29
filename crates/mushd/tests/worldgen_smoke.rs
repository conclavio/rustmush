//! LIVE `@gen` smoke against the real OpenRouter key — the ONE test that makes
//! a real LLM API call. It is **skipped by default** and makes no network call
//! unless `RUN_LLM_SMOKE=1`, so it never runs in local gates or CI (every other
//! test uses a MockProvider). The orchestrator runs it once, after the mock
//! suite is green.
//!
//! It drives the exact production path a single-room `@gen` uses — build an
//! `LlmService` from `[llm]` config (resolving `OPENROUTER_API_KEY` from the
//! environment / `.env`), assemble a one-room `GenSnapshot`, and call
//! `run_worldgen_async` — then asserts a non-empty room description came back
//! from a real model. No SSH, no store, no prod data touched.
//!
//! Run it (from the repo root, with OPENROUTER_API_KEY in the environment or in
//! `.env`):
//!
//! ```text
//! RUN_LLM_SMOKE=1 cargo test -p mushd --test worldgen_smoke -- --nocapture
//! ```
//!
//! PowerShell:
//!
//! ```text
//! $env:RUN_LLM_SMOKE = "1"; cargo test -p mushd --test worldgen_smoke -- --nocapture
//! ```

use mush_core::ObjId;
use mush_llm::LlmConfig;
use mush_worldgen::{
    DescLength, GenKind, GenOutcome, GenSnapshot, RoomFacts, Theme, run_worldgen_async,
};

#[tokio::test(flavor = "multi_thread")]
async fn live_worldgen_openrouter_smoke() {
    if std::env::var("RUN_LLM_SMOKE").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping live_worldgen_openrouter_smoke: set RUN_LLM_SMOKE=1 (and provide \
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

    // A real service: the worldgen_room route on a real OpenRouter reasoning
    // model, keyed from the env var by NAME (never inlined). Parsed as the
    // `[llm]` block would be, so this exercises the real config path too.
    let cfg: LlmConfig = toml::from_str(
        r#"
        enabled = true

        [providers.openrouter]
        kind = "openai"
        base_url = "https://openrouter.ai/api/v1"
        api_key_env = "OPENROUTER_API_KEY"
        referer = "https://github.com/rustmush"
        title = "rustmush"

        [routes.worldgen_room]
        model = "z-ai/glm-5.3"
        max_tokens = 1500
        timeout_s = 90
        reasoning = "low"
        "#,
    )
    .expect("smoke [llm] config parses");
    let llm = mush_llm::LlmService::from_config(&cfg).expect("build LlmService");

    // A single room to describe (no exits, no neighbours) — the minimal
    // GenSnapshot the @gen handler would snapshot synchronously.
    let snapshot = GenSnapshot {
        kind: GenKind::Room,
        room: ObjId(2),
        builder: ObjId(1),
        theme: Theme {
            name: "Smoke Test".into(),
            style_guide: "damp Victorian dockside, gaslight, wet stone, low fog".into(),
            desc_length: DescLength::Short,
            ..Theme::default()
        },
        facts: RoomFacts {
            name: "The Old Wharf".into(),
            exits: Vec::new(),
        },
        neighbors: Vec::new(),
        prose: "A long wooden wharf, tar and rope, a customs shed at the far end, \
                smelling of brine and coal smoke."
            .into(),
        guidance: None,
        prior_draft: None,
    };
    let payload = serde_json::to_string(&snapshot).expect("serialize snapshot");

    let result = run_worldgen_async(&llm, &payload).await;
    let outcome: GenOutcome = serde_json::from_str(&result).expect("decode GenOutcome");
    match outcome {
        GenOutcome::Ok { draft, model, .. } => {
            eprintln!("live @gen OK via {model}:\n{}", draft.desc);
            assert!(
                !draft.desc.trim().is_empty(),
                "the model returned an empty description"
            );
        }
        GenOutcome::Err {
            message, retryable, ..
        } => panic!("live @gen failed (retryable={retryable}): {message}"),
    }
}
