//! Cross-module M3 end-to-end journey over real SSH (in-memory store,
//! ungated): the full server wiring — world task, session layer, accounts,
//! SSH listener — **plus the M3 modules** exactly as `main.rs` boots them
//! (`Engine::with_modules` → `boot_modules`), then two players and God
//! exercising comsys, mail (compose over the real line editor), jobs with a
//! payload handler, `@wall`/`@boot`, and the markdown help topics — all
//! asserted on the rendered ANSI the clients actually read.

mod support;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Engine, GOD, GameModule, ModError, ModuleBootData,
    ModuleCtx, Registrar, World,
};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};

use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, ServerDeps};
use mushd::{modules, unix_now_s};

use support::{Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";

// ---------------------------------------------------------------------
// A test-only producer module: files a payload-carrying job so the whole
// payload-handler flow can be driven from the SSH surface.
// ---------------------------------------------------------------------

struct E2eProducer;

impl GameModule for E2eProducer {
    fn name(&self) -> &'static str {
        "e2etest"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.command(
            CommandSpec {
                name: "+tgrant".into(),
                shape: ArgShape::Raw,
                help: "+tgrant <text> -- test producer: file a REQ job whose payload \
                       hands <text> back to you when staff approve it."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(tgrant_cmd),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        // The producer pattern the WoD XP flow will use in M6: register the
        // handler once the world (and the jobs module's state slot) is up.
        mush_jobs::register_payload_handler(
            ctx.w,
            "e2e.grant",
            Arc::new(|ctx: &mut Ctx<'_>, job: &mush_jobs::Job| {
                let text = job
                    .payload
                    .as_ref()
                    .and_then(|p| p.data.get("text"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                let opener = job.opener;
                ctx.out
                    .sys_to_actor(opener, "jobs", format!("PAYLOAD GRANTED: {text}"));
                Ok(())
            }),
        )
        .map_err(ModError::msg)
    }
}

fn tgrant_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().trim().to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Usage: +tgrant <text>"));
    }
    let opener = ctx.actor;
    let id = mush_jobs::open_job(
        ctx,
        mush_jobs::OpenJob {
            queue: mush_jobs::REQ_QUEUE.into(),
            title: format!("grant {text}"),
            opener,
            text: Some("payload flow test".to_string()),
            payload: Some(mush_jobs::JobPayload {
                handler: "e2e.grant".into(),
                data: serde_json::json!({ "text": text }),
            }),
            tags: Vec::new(),
        },
    )
    .map_err(CmdError::msg)?;
    ctx.out.info(opener, format!("Test job {id} filed."));
    Ok(())
}

// ---------------------------------------------------------------------
// Boot: production wiring + modules, on an ephemeral port.
// ---------------------------------------------------------------------

async fn boot_server(dir: &Path) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    // ONE store shared by module boot, world persistence, and accounts.
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
        })
        .await
        .unwrap();

    // The main.rs boot order: base registry + staff commands + help topics,
    // then Engine::with_modules + boot_modules, then the world task.
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let help_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/help"));
    let topics = mushd::help::load_help_topics(&mut registry, help_dir).unwrap();
    assert!(topics >= 10, "repo help topics must load (got {topics})");

    let mut world = World::new_minimal();
    world.set_now(unix_now_s());
    let mut mods = modules::server_modules();
    mods.push(Box::new(E2eProducer));
    let mut engine = Engine::with_modules(world, registry, mods).unwrap();
    modules::boot_modules(&mut engine, &*store).await.unwrap();

    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(&store),
            guard: Arc::new(IpGuard::new()),
            staff,
            llm: None,
            quarantine_dir: None,
        },
    );
    let world = handles.handle;
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), world.clone(), 3));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world,
        Arc::new(IpGuard::new()),
    ));
    start_listener(
        "127.0.0.1:0".parse().unwrap(),
        dir,
        "Welcome to the modules e2e MUSH!".to_string(),
        bridge,
    )
    .await
    .expect("listener")
    .addr()
}

async fn login_create(
    addr: SocketAddr,
    name: &str,
    pw: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the modules e2e MUSH!").await;
    term.send_line(&format!("create {name} {pw}")).await;
    term.expect(&format!("Connected as {name}")).await;
    term.expect("Limbo(#0)").await;
    (handle, term)
}

// ---------------------------------------------------------------------
// The journey
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn m3_cross_module_journey() {
    let dir = scratch_dir("modules-e2e");
    let addr = boot_server(&dir).await;

    // --- God + two fresh players. ----------------------------------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the modules e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_alice_handle, mut alice) = login_create(addr, "Alice", "secret123").await;
    god.expect("Alice has connected.").await;
    let (_bob_handle, mut bob) = login_create(addr, "Bob", "hunter42").await;
    alice.expect("Bob has connected.").await;

    // ================= comsys =================
    // Character creation auto-joined both to Public under 'pub'.
    let mark = alice.mark();
    alice.send_line("comlist").await;
    alice.expect_after(mark, "Public").await;
    alice.expect_after(mark, "-- 1 channel --").await;

    // Leave and re-join through the classic surface.
    alice.send_line("delcom pub").await;
    alice.expect("Channel Public removed (alias 'pub').").await;
    let bob_join = bob.mark();
    alice.send_line("addcom pub=Public").await;
    alice.expect("Channel Public added with alias 'pub'.").await;
    bob.expect_after(bob_join, "[Public] Alice has joined this channel.")
        .await;

    // comtitle + alias-verb say + pose flavor, cross-delivered.
    alice.send_line("comtitle pub=the Bold").await;
    alice.expect("Title on Public set to \"the Bold\".").await;
    alice.send_line("pub Hello channel land").await;
    bob.expect("[Public] the Bold Alice: Hello channel land")
        .await;
    alice
        .expect("[Public] the Bold Alice: Hello channel land")
        .await;
    bob.send_line("pub :waves back").await;
    alice.expect("[Public] Bob waves back").await;

    // Recall replays the logged history.
    let mark = alice.mark();
    alice.send_line("@channel/recall pub").await;
    alice.expect_after(mark, "-- Public: last").await;
    alice
        .expect_after(mark, "the Bold Alice: Hello channel land")
        .await;
    alice.expect_after(mark, "Bob waves back").await;
    alice.expect_after(mark, "-- End of recall --").await;

    // ================= mail =================
    // Compose over the real SSH line editor: prompt, multi-line body,
    // preview, "--" sends.
    alice.send_line("@mail/send Bob=Test flight").await;
    alice
        .expect("MAIL: Composing to Bob -- subject: Test flight")
        .await;
    alice.expect("Mail>").await;
    alice.send_line("First line of body.").await;
    alice.send_line("Second line.").await;
    let mark = alice.mark();
    alice.send_line("~p").await;
    alice.expect_after(mark, "MAIL: --- preview ---").await;
    alice.expect_after(mark, "MAIL: To: Bob").await;
    alice.expect_after(mark, "First line of body.").await;
    alice.send_line("--").await;
    alice.expect("MAIL: Message 1 sent to Bob.").await;

    // Regression (prompt-clear fix): popping compose must CLEAR the stored
    // prompt — output after the pop must never redraw a stale "Mail>".
    let mark = alice.mark();
    alice.send_line("look").await;
    alice.expect_after(mark, "Limbo(#0)").await;
    assert!(
        !strip_ansi(&alice.seen[mark..]).contains("Mail>"),
        "stale Mail> prompt redrawn after compose popped:\n{}",
        strip_ansi(&alice.seen[mark..])
    );

    // Live delivery notice, then the recipient reads it.
    bob.expect("MAIL: New message (1) from Alice: Test flight")
        .await;
    let mark = bob.mark();
    bob.send_line("@mail").await;
    bob.expect_after(mark, "Test flight").await;
    bob.send_line("@mail 1").await;
    bob.expect_after(mark, "From:    Alice").await;
    bob.expect_after(mark, "Subject: Test flight").await;
    bob.expect_after(mark, "First line of body.").await;
    bob.expect_after(mark, "Second line.").await;

    // ================= jobs =================
    // A player request, worked by staff.
    alice
        .send_line("+request Need a ladder=There is no ladder in Limbo.")
        .await;
    alice.expect("Request filed as job 1 in queue REQ.").await;
    let mark = god.mark();
    god.send_line("+job 1").await;
    god.expect_after(mark, "Need a ladder").await;
    god.send_line("+job/approve 1=Granted.").await;
    god.expect("Job 1 approved.").await;
    alice
        .expect("Your job 1 (Need a ladder) has been approved by God. Granted.")
        .await;

    // The payload-handler flow: the producer's job carries a payload whose
    // registered handler runs (atomically) on approval.
    alice.send_line("+tgrant shiny").await;
    alice.expect("Test job 2 filed.").await;
    god.send_line("+job/approve 2").await;
    god.expect("Job 2 approved.").await;
    alice.expect("PAYLOAD GRANTED: shiny").await;

    // ================= reconnect: unread summary =================
    // Bob leaves; mail lands while he is away; the Connected hook reports
    // it when he returns.
    bob.send_line("QUIT").await;
    bob.expect("Goodbye.").await;
    bob.expect_close().await;
    alice.expect("Bob has disconnected.").await;
    alice
        .send_line("@mail/quick Bob=While you were out/Missed you.")
        .await;
    alice.expect("MAIL: Message 2 sent to Bob.").await;

    let (_bob2_handle, mut bob2) = play_client(addr).await;
    bob2.send_line("connect Bob hunter42").await;
    bob2.expect("Connected as Bob").await;
    bob2.expect("MAIL: You have 1 unread message (0 urgent).")
        .await;

    // ================= compose flood exemption =================
    // Pasting a long letter must not lose body lines to the transport
    // flood throttle (burst 30): while an actor sits in an input context
    // they are flood-exempt, so all 40 lines land.
    alice.send_line("@mail/send Bob=Bulk paste").await;
    alice
        .expect("MAIL: Composing to Bob -- subject: Bulk paste")
        .await;
    for i in 1..=40 {
        alice.send_line(&format!("bulk line {i}")).await;
    }
    alice.send_line("--").await;
    alice.expect("MAIL: Message 3 sent to Bob.").await;
    let mark = bob2.mark();
    bob2.send_line("@mail 3").await;
    bob2.expect_after(mark, "bulk line 1").await;
    bob2.expect_after(mark, "bulk line 40").await;

    // ================= help =================
    let mark = alice.mark();
    alice.send_line("help channels").await;
    alice.expect_after(mark, "Your alias becomes a verb").await;

    // ================= admin: @wall + @boot =================
    god.send_line("@wall Server restarting soon.").await;
    alice
        .expect("Announcement from God: Server restarting soon.")
        .await;
    bob2.expect("Announcement from God: Server restarting soon.")
        .await;

    god.send_line("@boot Bob=behave").await;
    bob2.expect("You have been booted by God: behave").await;
    bob2.expect_close().await;
    god.expect("@boot: Bob booted (1 connection).").await;
    alice.expect("Bob has disconnected.").await;

    // The survivors still talk — the world is healthy after all of it.
    alice.send_line("pub still here").await;
    god.expect("[Public] the Bold Alice: still here").await;

    // Sanity on the raw stream: the ANSI really was ANSI (the channel tag
    // colors the [Public] prefix), and stripping it yields the classic
    // line — "assert rendered ANSI output" both ways.
    assert!(
        god.seen.contains('\u{1b}'),
        "SSH output must carry ANSI escapes"
    );
    assert!(strip_ansi(&god.seen).contains("[Public] the Bold Alice: still here"));

    let _ = std::fs::remove_dir_all(&dir);
}
