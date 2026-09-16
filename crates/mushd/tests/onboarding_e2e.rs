//! The onboarding deliverable as one scripted end-to-end journey over real SSH:
//! **new player signs up → character generation → approved → plays**.
//!
//! Boots the whole server (world task, session layer, store-backed accounts,
//! IP guard, the built-in modules + worldgen + npc + the one game sheet with
//! `chargen_required` on) on an ephemeral port — production wiring except for
//! port, paths, and an in-memory store — then drives real russh clients:
//!
//! - A brand-new player `create`s an account over the welcome screen, is
//!   placed in the restricted chargen area, and is **greeted and pointed at
//!   `+chargen`** on connect (the transport-agnostic `Connected` hook — no SSH
//!   code path special-cases it).
//! - While unapproved they are **blocked** by the onboarding gate from both a
//!   live-grid verb (`+roll`) and an *exit*, which the gate refuses by
//!   resolution rather than by verb name.
//! - They spend the six-point rating pool, name the pilot, and
//!   `+chargen/submit`, which files a `pilot-chargen` approval job and flips
//!   the sheet to Pending.
//! - A **reconnect while Pending** is greeted with the open job rather than
//!   invited back into chargen.
//! - Staff (God) `+job/approve` the job; the gate lifts, the character is
//!   **moved to the live start room** and re-rendered, and the same `+roll`
//!   now works.
//!
//! A second journey proves the ungated default (`chargen_required = false`):
//! a new player is **not** greeted into chargen, lands in Limbo, and `+chargen`
//! opens normally.
//!
//! 16b cut R-b: this used to drive `ruleset-wod`'s guided wizard (a template,
//! attribute/ability priority tiers, virtues and freebies — 79 scripted lines).
//! The game's chargen is three commands, so the journey below is the same
//! journey over the sheet that exists.
//!
//! No real LLM is ever called: the worldgen/npc modules are wired to a
//! `MockProvider`-backed [`LlmService`] with the global kill switch engaged,
//! and nothing in the onboarding flow reaches the LLM anyway.

mod support;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{Engine, GOD, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::{SessionConfig, SessionManager};

use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps};

use support::{Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";

/// A running server plus the handles that keep it alive for the test's life.
struct Server {
    addr: SocketAddr,
    dir: PathBuf,
    _listener: SshListener,
    _handles: PersistHandles,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Boot the full production module stack over SSH with `chargen_required` set
/// as requested. The world is seeded fresh (Limbo `#0`, God `#1`), then two
/// rooms are created — a chargen area (`#2`) and the live start room (`#3`) —
/// and `[onboarding]` points at them. The `boss` account is linked to God so
/// staff can approve over the same front door.
async fn boot(tag: &str, chargen_required: bool) -> Server {
    let dir = scratch_dir(tag);

    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .unwrap();

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    // The chargen area (#2) and the live start room (#3). Created directly on
    // the loaded world before the task spawns — RAM is all the onboarding
    // seams read (this is an in-memory test; the gated persistence twin covers
    // durability).
    let chargen_area = world
        .create(ObjKind::Room, "Chargen Nexus", GOD, None)
        .expect("chargen area");
    let commons = world
        .create(ObjKind::Room, "The Commons", GOD, None)
        .expect("live start room");
    assert_eq!(
        chargen_area,
        mush_core::ObjId(2),
        "chargen-area id assumption"
    );
    assert_eq!(commons, mush_core::ObjId(3), "start-room id assumption");

    let onboarding = mush_core::OnboardingConfig {
        chargen_required,
        start_room: commons,
        start_room_unapproved: chargen_area,
    };

    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();

    // A MockProvider-backed LLM with the global kill switch engaged: the
    // worldgen/npc modules register cleanly and never make a real call.
    let mock: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("openrouter"));
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", mock)
            .global_enabled(false)
            .build(),
    );

    // The production assembly (`mush_server::boot::build_engine`): built-ins,
    // worldgen, the sheet-grounded NPC module, then the one game ruleset.
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(&llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::ruleset_modules(&onboarding, &Default::default()));

    let mut engine = Engine::with_modules(world, registry, mods).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let guard = Arc::new(IpGuard::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: Some(Arc::clone(&llm)),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            metrics: Arc::new(mush_net_web::Metrics::new()),
            outbox_adapters: Vec::new(),
        },
    );

    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 3));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let listener = start_listener(
        "127.0.0.1:0".parse().unwrap(),
        &dir,
        "Welcome to the onboarding MUSH!".to_string(),
        bridge,
    )
    .await
    .expect("ssh listener");
    let addr = listener.addr();

    Server {
        addr,
        dir,
        _listener: listener,
        _handles: handles,
    }
}

/// Assert `needle` has NOT appeared since `mark`. Every caller pairs this with
/// an `expect_after` on the same span, which is the round-trip barrier proving
/// the command was processed.
fn not_seen_since(term: &Term, mark: usize, needle: &str) -> bool {
    !strip_ansi(&term.seen[mark..]).contains(needle)
}

async fn send_and_expect(t: &mut Term, line: &str, needle: &str) {
    let mark = t.mark();
    t.send_line(line).await;
    t.expect_after(mark, needle).await;
}

/// Spend the six-point pool exactly, name the pilot, and file the sheet.
///
/// Gunnery 4 / piloting 3 / sensors 2 is `3 + 2 + 1 == 6` — the budget counts
/// dots **above** the free one every rating starts at. Returns the
/// `pilot-chargen` job id, parsed from the submit confirmation.
async fn build_a_pilot_and_submit(t: &mut Term) -> u64 {
    send_and_expect(t, "+chargen", "A blank pilot sheet is yours").await;
    for (line, acknowledgement) in [
        ("+stat name=Ada Renard", "Set name = Ada Renard."),
        ("+stat callsign=Ladder", "Set callsign = Ladder."),
        ("+stat gunnery=4", "Set gunnery = 4."),
        ("+stat piloting=3", "Set piloting = 3."),
        ("+stat sensors=2", "Set sensors = 2."),
    ] {
        send_and_expect(t, line, acknowledgement).await;
    }
    send_and_expect(t, "+chargen/submit", "Submitted for approval as job").await;

    // Parse the job id out of the confirmation line.
    let hay = strip_ansi(&t.seen);
    let idx = hay
        .find("Submitted for approval as job ")
        .expect("submit confirmation present");
    let tail = &hay[idx + "Submitted for approval as job ".len()..];
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect("job id parses")
}

#[tokio::test(flavor = "multi_thread")]
async fn new_player_signs_up_builds_a_pilot_is_approved_and_plays() {
    let server = boot("gated", true).await;
    let addr = server.addr;

    // --- Staff (God) connects through the same welcome screen. -----------
    let (god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the onboarding MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    // A global exit on Limbo, so the gate has real movement to refuse: an exit
    // held by Limbo resolves from any room, including the chargen area.
    let dig_mark = god.mark();
    god.send_line("@dig Waystation=out,back").await;
    god.expect_after(dig_mark, "Opened exit out").await;

    // --- A brand-new player creates an account, lands in the restricted --
    // --- chargen area, and is greeted by the connect-time hook. ----------
    let (nick_handle, mut nick) = play_client(addr).await;
    nick.expect("connect <account> <password>").await;
    nick.send_line("create Nick secret123").await;
    nick.expect("Account created.").await;
    nick.expect("Connected as Nick").await;
    // The ObjCreated observer placed them in `start_room_unapproved`...
    nick.expect("Chargen Nexus").await;
    // ...and the Connected hook points them at the three commands they have.
    nick.expect("Type +chargen").await;

    // --- The gate holds: a live-grid verb is refused... -------------------
    let blocked_mark = nick.mark();
    nick.send_line("+roll 2d6").await;
    nick.expect_after(blocked_mark, "must finish creating")
        .await;
    assert!(
        not_seen_since(&nick, blocked_mark, "roll  Nick"),
        "a gated +roll must not produce a roll result"
    );

    // --- ...and so is an exit, which no verb allowlist would catch. -------
    let exit_mark = nick.mark();
    nick.send_line("out").await;
    nick.expect_after(exit_mark, "must finish creating").await;
    assert!(
        not_seen_since(&nick, exit_mark, "Waystation"),
        "the gated exit must not move Nick onto the live grid"
    );

    // --- Build the pilot and file it for approval. ------------------------
    let job = build_a_pilot_and_submit(&mut nick).await;

    // --- Still gated while Pending. ---------------------------------------
    let pending_mark = nick.mark();
    nick.send_line("+roll 2d6").await;
    nick.expect_after(pending_mark, "must finish creating")
        .await;

    // --- A reconnect while Pending is told about the open job rather than -
    // --- invited back into a chargen they cannot edit. --------------------
    nick.send_line("quit").await;
    nick.expect_close().await;
    let _ = nick_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    let (nick_handle, mut nick) = play_client(addr).await;
    nick.expect("connect <account> <password>").await;
    nick.send_line("connect Nick secret123").await;
    nick.expect("Connected as Nick").await;
    nick.expect("awaiting approval").await;
    assert!(
        strip_ansi(&nick.seen).contains(&format!("job {job}")),
        "the greeting names the open job: {:?}",
        nick.seen
    );

    // --- Staff approves the chargen job. ---------------------------------
    let god_mark = god.mark();
    god.send_line(&format!("+job/approve {job}=Welcome aboard."))
        .await;
    god.expect_after(god_mark, "approved").await;

    // --- Gate lifts: Nick is moved to the live start room, re-rendered, --
    // --- and the same roll now works. ------------------------------------
    let approved_mark = nick.mark();
    nick.expect_after(approved_mark, "approved").await; // "...has been approved!"
    nick.expect_after(approved_mark, "The Commons").await; // re-rendered there
    let play_mark = nick.mark();
    nick.send_line("+roll 2d6").await;
    nick.expect_after(play_mark, "roll  Nick").await;
    assert!(
        not_seen_since(&nick, play_mark, "must finish creating"),
        "an approved character's +roll must not be gated"
    );

    let _ = god_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    let _ = nick_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chargen_not_required_leaves_the_grid_open() {
    // The default: no gate, no greeting, `+chargen` is an ordinary command.
    let server = boot("ungated", false).await;
    let addr = server.addr;

    let (mort_handle, mut mort) = play_client(addr).await;
    mort.expect("connect <account> <password>").await;
    mort.send_line("create Morty pass1234").await;
    mort.expect("Account created.").await;
    mort.expect("Connected as Morty").await;
    // Lands in Limbo (the seeded start room), NOT the chargen area — the
    // onboarding spawn observer is inert when chargen_required is off.
    mort.expect("Limbo #0").await;

    // Nothing was pushed at them: an ordinary command runs immediately.
    let mark = mort.mark();
    mort.send_line("say hello world").await;
    mort.expect_after(mark, "Morty says, \"hello world\"").await;
    assert!(
        not_seen_since(&mort, mark, "Type +chargen"),
        "no chargen greeting is pushed when chargen_required is off"
    );

    // And +chargen opens normally rather than being gated.
    let chargen_mark = mort.mark();
    mort.send_line("+chargen").await;
    mort.expect_after(chargen_mark, "A blank pilot sheet is yours")
        .await;
    assert!(
        not_seen_since(&mort, chargen_mark, "must finish creating"),
        "+chargen is never gated when chargen_required is off"
    );
    // The budget surface works: spend a point.
    let stat_mark = mort.mark();
    mort.send_line("+stat gunnery=3").await;
    mort.expect_after(stat_mark, "Set gunnery = 3.").await;

    let _ = mort_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}
