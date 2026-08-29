//! The M7 worldgen demo end-to-end (in-memory store, ungated, a network-free
//! [`MockProvider`] injected as the server's `LlmService` — never the real
//! adapters, never a real key):
//!
//! - `worldgen_full_journey_over_ssh` — a builder (God, who holds
//!   `BUILDER_LLM`) sets a **zone theme**, `@dig`s two adjacent rooms, `@gen`s
//!   and **accepts** a desc on the first, then `@gen here=<prose>` in the
//!   second. The mock returns canned structured JSON, so a draft arrives
//!   **asynchronously** through the real defer → `run_worldgen_async` →
//!   `WorldCommand::ModuleAsyncDone` path; the draft renders, `@gen/retry`
//!   regenerates, `@gen/accept` writes the desc, and `look` shows it. The
//!   prompt the mock received for the second room is asserted to carry the
//!   theme **and** the first room's accepted desc (the cross-room consistency
//!   mechanism). A non-builder's `@gen` is refused.
//! - `keyless_boot_disables_gen` — with `[llm].enabled = false` (here: a
//!   globally-disabled service) the server still boots and worldgen commands
//!   still register, but `@gen` answers "LLM features are disabled." and the
//!   provider is never called.
//! - `draft_survives_store_reboot_before_accept` — a `@gen` draft persisted
//!   through the store's module-op write-through is reloaded by
//!   `modules::boot_modules` on a fresh boot, and `@gen/accept` then commits
//!   it — proving the draft outlives a restart.
//!
//! The full-journey and keyless tests drive the ACTUAL server front door over
//! real SSH (`support::boot_worldgen_server`); the reboot test drives the
//! engine + store directly (the store round-trip is the point).

mod support;

use std::sync::Arc;

use mush_core::{Effect, Effects, Engine, GOD, Inbound, ObjKind, Outbox, World, WorldCommand};
use mush_llm::{
    CompletionRequest, CompletionResponse, ContentBlock, LlmProvider, LlmService, MockProvider,
    RouteConfig, StopReason, Usage,
};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_worldgen::run_worldgen_async;
use serde_json::json;
use support::{
    boot_worldgen_server, login_create, parse_id_after, play_client, scratch_dir, strip_ansi,
};

const GM_PASSWORD: &str = "gm-secret-1";
const THEME_STYLE: &str = "rain-slick neon streets of a drowned city";
const GALLERY_DESC: &str =
    "A vaulted gallery where rain drums on the glass roof and puddles mirror the neon.";
const STUDY_DESC: &str =
    "A cramped study, its shelves swollen with damp ledgers under a bare bulb.";
const STUDY_DESC2: &str =
    "A cramped study drowned in shadow, its damp ledgers reeking of mildew and rust.";

// ----------------------------------------------------------------------
// Shared LLM harness (MockProvider — no network, no key)
// ----------------------------------------------------------------------

/// The `worldgen_room` route the mock serves. Any non-`anthropic/` model id
/// derives to the `openrouter` provider (where the mock is registered).
fn route() -> RouteConfig {
    RouteConfig {
        model: "z-ai/glm-5.3".into(),
        max_tokens: 1500,
        timeout_s: 60,
        reasoning: Some("low".into()),
        reasoning_max_tokens: None,
    }
}

/// A service backed by `mock`, either globally enabled or (kill-switch on)
/// disabled.
fn service(mock: &Arc<MockProvider>, enabled: bool) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route("worldgen_room", route())
            .provider("openrouter", p)
            .global_enabled(enabled)
            .build(),
    )
}

/// A canned `submit_room` tool call carrying `desc`.
fn tool_resp(desc: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "submit_room".into(),
            input: json!({ "desc": desc }),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 120,
            output_tokens: 60,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

/// The concatenated text of every user/assistant turn in a request (what the
/// provider actually received — theme, facts, neighbors, prose live here).
fn prompt_text(req: &CompletionRequest) -> String {
    let mut s = String::new();
    for m in &req.messages {
        for b in &m.content {
            if let ContentBlock::Text { text } = b {
                s.push_str(text);
                s.push('\n');
            }
        }
    }
    s
}

// ----------------------------------------------------------------------
// The full journey, over real SSH
// ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn worldgen_full_journey_over_ssh() {
    let dir = scratch_dir("worldgen-e2e");
    let mock = Arc::new(MockProvider::new("openrouter"));
    // Three drafts, consumed in order: Gallery @gen, Study @gen, Study retry.
    mock.push_response(tool_resp(GALLERY_DESC));
    mock.push_response(tool_resp(STUDY_DESC));
    mock.push_response(tool_resp(STUDY_DESC2));
    let llm = service(&mock, true);
    let addr =
        boot_worldgen_server(&dir, "Welcome to the worldgen e2e MUSH!", GM_PASSWORD, llm).await;

    // --- God, the builder. ----------------------------------------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the worldgen e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    // --- A zone master carrying the theme. ------------------------------
    god.send_line("@create Districts").await;
    god.expect("Created: Districts(#").await;
    let zone = parse_id_after(&strip_ansi(&god.seen), "Created: Districts(#");
    god.send_line(&format!("@set #{zone}=ZONE_MASTER")).await;
    god.expect("Set.").await;
    god.send_line(&format!("@theme #{zone}=style_guide:{THEME_STYLE}"))
        .await;
    god.expect("Theme style_guide set").await;

    // --- Two adjacent rooms: Gallery (off Limbo), Study (off Gallery). ---
    god.send_line("@dig Gallery=gallery,out").await;
    god.expect("Gallery created with room number #").await;
    let gallery = parse_id_after(&strip_ansi(&god.seen), "Gallery created with room number #");
    god.send_line(&format!("@zone #{gallery}=#{zone}")).await;
    god.expect("Zoned.").await;

    let mark = god.mark();
    god.send_line("gallery").await;
    god.expect_after(mark, "Gallery(#").await;

    god.send_line("@dig Study=study,back").await;
    god.expect("Study created with room number #").await;
    let study = parse_id_after(&strip_ansi(&god.seen), "Study created with room number #");
    god.send_line(&format!("@zone #{study}=#{zone}")).await;
    god.expect("Zoned.").await;

    // --- @gen the Gallery and accept it (this is the neighbor's desc). ---
    let mark = god.mark();
    god.send_line("@gen here=A grand gallery under a glass roof.")
        .await;
    god.expect_after(mark, "Generating a description").await;
    god.expect_after(mark, "Draft ready").await;
    god.expect_after(mark, "vaulted gallery").await;
    let mark = god.mark();
    god.send_line("@gen/accept").await;
    god.expect_after(mark, "Accepted").await;

    // --- Move to the Study and @gen it. ---------------------------------
    let mark = god.mark();
    god.send_line("study").await;
    god.expect_after(mark, "Study(#").await;

    let mark = god.mark();
    god.send_line("@gen here=A small study just off the gallery.")
        .await;
    god.expect_after(mark, "Draft ready").await;
    god.expect_after(mark, "cramped study").await;

    // The consistency mechanism: the Study prompt (mock call #1 — call #0 was
    // the Gallery) carried the theme AND the Gallery's accepted desc.
    let calls = mock.calls();
    assert!(
        calls.len() >= 2,
        "expected >=2 provider calls, got {}",
        calls.len()
    );
    let study_prompt = prompt_text(&calls[1]);
    assert!(
        study_prompt.contains(THEME_STYLE),
        "Study prompt missing the theme:\n{study_prompt}"
    );
    assert!(
        study_prompt.contains(GALLERY_DESC),
        "Study prompt missing the neighbor's accepted desc:\n{study_prompt}"
    );

    // --- @gen/retry regenerates: guidance appended, prior draft avoided. -
    let mark = god.mark();
    god.send_line("@gen/retry make it darker").await;
    god.expect_after(mark, "Regenerating").await;
    god.expect_after(mark, "Draft ready").await;
    god.expect_after(mark, "drowned in shadow").await;
    let calls = mock.calls();
    assert!(
        calls.len() >= 3,
        "retry did not call the provider: {}",
        calls.len()
    );
    let retry_prompt = prompt_text(&calls[2]);
    assert!(
        retry_prompt.contains("make it darker"),
        "retry prompt missing the guidance:\n{retry_prompt}"
    );
    assert!(
        retry_prompt.contains("cramped study"),
        "retry prompt missing the avoided prior draft:\n{retry_prompt}"
    );

    // --- Accept the retried draft; look shows the committed desc. --------
    let mark = god.mark();
    god.send_line("@gen/accept").await;
    god.expect_after(mark, "Accepted").await;
    let mark = god.mark();
    god.send_line("look").await;
    god.expect_after(mark, "drowned in shadow").await;

    // --- A non-builder is refused. --------------------------------------
    let (_alice_handle, mut alice) = login_create(addr, "Alice", "alicepw12").await;
    let mark = alice.mark();
    alice.send_line("@gen here=let me build something").await;
    alice.expect_after(mark, "Permission denied").await;

    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------------
// Keyless boot: disabled LLM still boots; @gen answers cleanly
// ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn keyless_boot_disables_gen() {
    let dir = scratch_dir("worldgen-e2e-off");
    let mock = Arc::new(MockProvider::new("openrouter"));
    // Globally disabled — the shape a keyless clone boots in ([llm].enabled =
    // false engages the same kill switch).
    let llm = service(&mock, false);
    let addr = boot_worldgen_server(&dir, "Keyless MUSH", GM_PASSWORD, llm).await;

    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Keyless MUSH").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    // @gen still registers (the command dispatches and defers), but the
    // off-loop generator hits the kill switch and re-enters a clean message.
    let mark = god.mark();
    god.send_line("@gen here=A room the LLM will never touch.")
        .await;
    god.expect_after(mark, "LLM features are disabled").await;
    // The provider was never actually called (kill switch is pre-call).
    assert_eq!(
        mock.call_count(),
        0,
        "a disabled service must not call the provider"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------------
// A draft survives a store reboot (module-op write-through + boot reload)
// ----------------------------------------------------------------------

/// A minimal engine with only the worldgen module (core + engine commands
/// plus `@gen`/`@theme`/`@llm`), backed by `llm`.
fn engine_with_worldgen(llm: Arc<LlmService>) -> Engine {
    let registry = mushd::world_task::base_registry();
    let mut world = World::new_minimal();
    world.set_now(0);
    Engine::with_modules(world, registry, vec![mushd::modules::worldgen_module(llm)]).unwrap()
}

/// Write module ops through the store the way the world task's `flush_dirty`
/// does (durable, awaited).
async fn write_ops(store: &dyn ObjectStore, ops: Vec<mush_core::ModuleOp>) {
    let batch = WriteBatch {
        serial: 0,
        durable: true,
        module_ops: ops,
        ..WriteBatch::default()
    };
    store.write(batch).await.unwrap();
}

fn cmd(line: &str) -> Inbound {
    Inbound::Cmd {
        conn: 0,
        actor: GOD,
        line: line.to_string(),
    }
}

#[tokio::test]
async fn draft_survives_store_reboot_before_accept() {
    const REBOOT_DESC: &str = "A study that outlives a reboot, its dust undisturbed.";
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());

    // --- Boot 1: generate a draft and persist it through the store. -----
    let mock = Arc::new(MockProvider::new("openrouter"));
    mock.push_response(tool_resp(REBOOT_DESC));
    let llm = service(&mock, true);
    let mut e1 = engine_with_worldgen(Arc::clone(&llm));
    let study = e1.world.create(ObjKind::Room, "Study", GOD, None).unwrap();
    e1.world.move_to(GOD, study).unwrap();

    // Dispatch @gen, run the off-loop generator, re-enter the completion.
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    e1.step(cmd("@gen here=A quiet study."), &mut out, &mut fx);
    let (module, correlation, payload) = take_module_async(fx.drain());
    let result = run_worldgen_async(&llm, &payload).await;
    let mut out2 = Outbox::new();
    let mut fx2 = Effects::new();
    e1.step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: module.into(),
            actor: GOD,
            correlation,
            payload: result,
        }),
        &mut out2,
        &mut fx2,
    );
    // The completion queued the draft persist op; write it through.
    let ops = e1.take_module_ops();
    assert!(
        ops.iter().any(|op| matches!(&op.op,
            mush_core::ModuleWrite::JsonSet { key, .. } if key.starts_with("draft:"))),
        "the completion must persist a draft op"
    );
    write_ops(&*store, ops).await;
    drop(e1); // the "old" process is gone; only the store survives.

    // --- Boot 2: a fresh engine reloads the draft from the store. -------
    let mock2 = Arc::new(MockProvider::new("openrouter"));
    let llm2 = service(&mock2, true);
    let mut e2 = engine_with_worldgen(llm2);
    // The mushd boot path: run module migrations, load each namespace's boot
    // data, and hand it to on_world_loaded (worldgen rebuilds its draft cache).
    mushd::modules::boot_modules(&mut e2, &*store)
        .await
        .unwrap();
    // Recreate the room identically (same deterministic id the draft names).
    let study2 = e2.world.create(ObjKind::Room, "Study", GOD, None).unwrap();
    assert_eq!(study2, study, "room id must be recreated identically");
    e2.world.move_to(GOD, study2).unwrap();

    // Accepting the reloaded draft commits the desc — no regeneration needed.
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    e2.step(cmd("@gen/accept"), &mut out, &mut fx);
    assert_eq!(
        e2.world.attr(study2, "desc").and_then(|a| a.as_text()),
        Some(REBOOT_DESC),
        "the reloaded draft's desc must be written on accept"
    );
    assert_eq!(mock2.call_count(), 0, "accept must not call the provider");
}

/// Pull the single `Effect::ModuleAsync` a `@gen` deferred.
fn take_module_async(effects: Vec<Effect>) -> (String, u64, String) {
    for e in effects {
        if let Effect::ModuleAsync {
            module,
            correlation,
            payload,
            ..
        } = e
        {
            return (module.to_string(), correlation, payload);
        }
    }
    panic!("expected one Effect::ModuleAsync");
}
