//! **The split's world task asks its provider** (16b cut AC.1, 16c `B-82`).
//!
//! The M16b two-process rig — `mush_server::serve_with_store` on one thread and
//! runtime, `mush_gateway::serve` on another, a real localhost IPC socket between
//! them and one shared store — booted with the production `[llm]` path.
//!
//! # What it proves
//!
//! On the production shape the world task used to be handed no LLM service: the
//! split half built its dependencies from throwaway in-memory ones and never set
//! `llm`, so every LLM-backed `Effect::ModuleAsync` met `no LlmService on this
//! world task` — worldgen, npc and weather dropped, a squad's deliberation refused
//! and the squad left on its own — while the all-in-one, which dev1 does not run,
//! ran them ("refused back to doctrine" here until 17b cut M5.7). dev1
//! logged that WARN on every deliberation. Here a machine squad's first
//! deliberation (its `NoOrder` signal on the first pulse after the squads form,
//! doc 16 section 6) is raised on the server half and must REACH the provider
//! across the boundary, and the WARN must never print.
//!
//! # Why it is config-only
//!
//! A keyless `openai`-kind provider at a loopback URL is a legitimate `[llm]`
//! deployment (`LlmService::from_config` registers a provider with no
//! `api_key_env` as a local Ollama/vLLM would be), so the provider here is an
//! axum stub named in the config and nothing else: the path under test is the
//! one dev1 boots through — `[llm]` -> `config_ext::llm_config` ->
//! `LlmService::from_config` -> `boot::build_engine` -> `ServerDeps::production`
//! -> the effect dispatcher. No seam, no `MockProvider`, and no `mush-llm` in the
//! gateway's graph. The provider id is `openrouter` because route derivation
//! sends every model without an `anthropic/` prefix there.
//!
//! The stub answers the well-formed EMPTY chat completion `mush-llm`'s own wire
//! tests use: the adapter parses it, the structured call finds no tool call,
//! appends one repair and posts again, and the reply is a `malformed` refusal —
//! the squad stays on its machines' own minds (doc 16 section 6's fallback).
//! The HITS are the proof; the reply's content is not what is under test.
//!
//! # Why it is its own binary
//!
//! The WARN is emitted by the world task, which runs on the server runtime's
//! worker threads, so only a GLOBAL tracing subscriber sees it — a thread-scoped
//! one would miss the very line this test is about. A global subscriber cannot be
//! shared with sibling tests, so this file holds exactly one test.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore, WriteBatch};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::GatewayOptions;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;
/// The gateway's answer while the server half is not admitting connections yet
/// (`arena_story_split.rs` explains the window); the first `connect` waits it out.
const WORLD_NOT_UP: &str = "Connection failed: the game is shutting down";
/// How long the first login waits for the server half, in 200 ms steps.
const ADMISSION_TRIES: usize = 25;

/// The one pilot, seeded into the shared store's world before either half boots.
const ADA: ObjId = ObjId(2);

/// The well-formed empty chat completion `crates/mush-llm/tests/wire.rs` answers
/// with: parseable, and carrying no tool call.
const BODY: &str = r#"{"model":"m","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":""}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

/// The WARN every LLM-backed `Effect::ModuleAsync` arm logs when its world task
/// holds no service — the line dev1 printed on every deliberation.
const NO_SERVICE: &str = "no LlmService on this world task";

// ---------------------------------------------------------------------------
// The log capture (`mush-server/tests/split_boundary/boot_parity.rs`'s, copied: the gateway
// tests carry no support module)
// ---------------------------------------------------------------------------

/// A `MakeWriter` that collects formatted log lines into a shared buffer.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the capture buffer")).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("the capture buffer")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// The split rig — `arena_story_split.rs`' config and store, `two_process_e2e.rs`'
// boot
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-2proc-llm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// A fast arena, reached through `[tacsim]` exactly as `arena_story_split.rs`
/// reaches it (the validation's floors included).
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.tacsim.enabled = true;
    cfg.tacsim.arena.tempo.quiet_ms = 100;
    cfg.tacsim.arena.tempo.alert_ms = 100;
    cfg.tacsim.arena.tempo.engaged_ms = 100;
    cfg.tacsim.arena.tempo.substep_max_ms = 100;
    cfg.tacsim.arena.ledger.summary_ms = 1_000;
    cfg.tacsim.match_.startup_ms = 300;
    cfg.tacsim.match_.board_timeout_ms = 120_000;
    cfg
}

/// The one store both halves are pointed at: the staff account, Ada's account,
/// and the world her character lives in, written through `mush_server::persist`'s
/// own boot-batch path so `SIM TEAM`'s account oracle knows her.
async fn shared_store() -> Arc<dyn ObjectStore> {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).expect("a hash"),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the staff account");
    let mut world = mush_server::persist::load_or_seed_world(&*store)
        .await
        .expect("boot world");
    let made = world
        .create(ObjKind::Player, "Ada", GOD, Some(LIMBO))
        .expect("a character");
    assert_eq!(made, ADA, "character id assumption");
    store
        .create_account(AccountRec {
            id: None,
            name: "ada".to_string(),
            argon2_hash: hash_password(PILOT_PASSWORD).expect("a hash"),
            created: i64::from(ADA.0),
            characters: vec![CharacterRef {
                id: ADA,
                name: "Ada".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the pilot's account");
    let dirty = world.take_dirty();
    let batch: WriteBatch = mush_server::persist::batch_from_dirty(&world, dirty, 0, true);
    store.write(batch).await.expect("the character seed batch");
    store
}

/// Boot the server half on its own thread and runtime through the UNCHANGED
/// `serve_with_store`, then the gateway on another, aimed at its endpoint,
/// both over `store` — `two_process_e2e.rs`'s `boot_split_sharing_store` shape.
async fn boot_split(
    server_cfg: Config,
    store: Arc<dyn ObjectStore>,
) -> (
    SocketAddr,
    std::thread::JoinHandle<()>,
    std::thread::JoinHandle<()>,
) {
    let (ipc_tx, ipc_rx) = oneshot::channel();
    let server_store = Arc::clone(&store);
    let server_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("the server runtime");
        let _ = rt.block_on(mush_server::serve_with_store(
            server_cfg,
            Some(ipc_tx),
            Some(server_store),
        ));
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(60), ipc_rx)
        .await
        .expect("server bound its IPC socket")
        .expect("ipc endpoint");

    let (ssh_tx, ssh_rx) = oneshot::channel();
    let gateway_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("the gateway runtime");
        let mut gw_cfg = test_config("gateway");
        gw_cfg.ipc.listen = endpoint.describe();
        let _ = rt.block_on(mush_gateway::serve(
            gw_cfg,
            GatewayOptions {
                store: Some(store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
            },
        ));
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(60), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    (ssh_addr, server_thread, gateway_thread)
}

// ---------------------------------------------------------------------------
// The terminal — `arena_story_split.rs`' `Term`, trimmed to what one pilot says
// ---------------------------------------------------------------------------

struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    fn mark(&self) -> usize {
        self.seen.len()
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

    async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let seen = strip_ansi(&self.seen[mark..]);
            if let Some(index) = needles.iter().position(|needle| seen.contains(needle)) {
                return index;
            }
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timeout waiting for any of {needles:?}; seen: {:?}",
                        self.seen
                    )
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for any of {needles:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    async fn expect(&mut self, needle: &str) {
        self.expect_one_of_after(0, &[needle]).await;
    }

    /// Run one line and wait for `needle`, retrying the two transients. The needle
    /// must not appear in the line's own echo.
    async fn say(&mut self, line: &str, needle: &str) {
        assert!(
            !line.contains(needle),
            "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
        );
        for _ in 0..RETRY_BUDGET {
            let mark = self.mark();
            self.send_line(line).await;
            match self
                .expect_one_of_after(mark, &[needle, COMMIT_PENDING, CONTROL_UNKNOWN])
                .await
            {
                0 => return,
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
        panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
    }
}

async fn play_client(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    let auth = handle.authenticate_none("play").await.expect("auth-none");
    assert!(auth.success(), "play must be accepted with no auth");
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");
    (
        handle,
        Term {
            channel,
            seen: String::new(),
        },
    )
}

/// `connect ada <pw>`, re-sent while the server half is not admitting yet.
async fn login_ada(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    for _ in 0..ADMISSION_TRIES {
        let mark = term.mark();
        term.send_line(&format!("connect ada {PILOT_PASSWORD}"))
            .await;
        if term
            .expect_one_of_after(mark, &["Connected as Ada", WORLD_NOT_UP])
            .await
            == 0
        {
            return (handle, term);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the server half never admitted `ada`");
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_split_world_task_asks_its_provider() {
    // ---- the one subscriber of this binary, before either half starts ----
    let capture = Capture::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(capture.clone())
            .finish(),
    )
    .expect("the one subscriber of this binary");

    // ---- the provider: a loopback stub that counts what reaches it ----
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("the stub binds");
    let port = listener.local_addr().expect("the stub's address").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&hits);
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move || {
            let hits = Arc::clone(&counted);
            async move {
                hits.fetch_add(1, SeqCst);
                (
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    BODY,
                )
            }
        }),
    );
    let stub = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // ---- the server half's config: the arena, a machine to fill blue, and
    // the production `[llm]` path at the stub ----
    let mut cfg = test_config("server");
    cfg.tacsim.match_.ai_fill = true;
    // No `api_key_env`: `from_config` registers the provider keyless. The id is
    // `openrouter` because that is where route derivation sends every model
    // without an `anthropic/` prefix; `tool_calling` and `forced_tool_choice`
    // default to true. `deliberate_min_ms` stays at its default: a machine
    // squad's FIRST signal is `NoOrder` and the floor gates only the second.
    cfg.llm = toml::from_str(&format!(
        "enabled = true\ndecision_workers = 8\n\
         [providers.openrouter]\nkind = \"openai\"\nbase_url = \"http://127.0.0.1:{port}/v1\"\n\
         [routes.tacsim_deliberate]\nmodel = \"z-ai/glm-5.3-flash\"\nmax_tokens = 512\ntimeout_s = 30\n"
    ))
    .expect("[llm] is a table");

    let store = shared_store().await;
    let (ssh, _server_thread, _gateway_thread) = boot_split(cfg, store).await;

    // ---- Ada alone walks into the simulator and boards; the machine takes blue ----
    let (_ada_handle, mut ada) = login_ada(ssh).await;
    ada.say("ooc", "Welcome Hall").await;
    // `SIM` is a verb before it is an exit, so the walk takes the exit's other name.
    ada.say("simulator", "Simulator Lobby").await;
    ada.say("SIM TEAM red", "takes red slot 1").await;
    ada.say("SIM START", "is boarding").await;
    ada.say("red", "Red Room").await;
    // A settle before `BOARD`. On this rig a `BOARD` typed the instant the walk's
    // room view arrives is, on roughly one run in three, never answered at all
    // (no `climb into`, no refusal, no transient; a second `BOARD` then seats
    // her, so the first never applied), with or without `[llm]`. It never
    // happened across a dozen runs with this pause. That is a split-shape
    // defect of its own, filed apart from 16c `B-82`; the step exists here only
    // to reach a running match, so it is spaced as `arena_story_split.rs`'s walk
    // is by its second pilot's.
    tokio::time::sleep(Duration::from_secs(1)).await;
    ada.say("BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // ---- the machine squad's commander asks the provider, across the boundary ----
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while hits.load(SeqCst) < 1 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no deliberation reached the provider within 60 s of the match starting \
             (16c B-82: the split's world task must hold the booted LLM service); \
             captured log:\n{}",
            capture.text()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // ---- the log: two positive controls, then the negative ----
    let captured = capture.text();
    assert!(
        captured.contains("world task running"),
        "the capture must see the world task's own INFO line — it is emitted on the \
         threads the WARN would come from, so without it a broken capture would pass \
         the negative below in silence; captured log:\n{captured}"
    );
    assert!(
        // The whole line, fields included: `build_engine` prints `llm service
        // built` on every boot (it always builds a service, even under
        // `enabled = false` with no provider), so only the fields say the rig's
        // `[llm]` stanza — enabled, the one `openrouter` provider, the one
        // `tacsim_deliberate` route — is what the production path built.
        captured.contains("llm service built enabled=true providers=1 routes=1"),
        "the server half must build its service from the rig's `[llm]` on the \
         production path (enabled, one provider, one route); captured log:\n{captured}"
    );
    assert!(
        !captured.contains(NO_SERVICE),
        "the split's world task met an LLM-backed effect with no service (16c B-82); \
         captured log:\n{captured}"
    );

    // The terminal, the halves' threads and the stub end with the process, as the
    // siblings' do.
    drop(ada);
    stub.abort();
}
