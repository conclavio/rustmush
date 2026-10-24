//! **The front door** (16b cut F.4; doc 16 §1 "A1", §3.7, §3.8; doc 10 R3).
//!
//! The readiness review of 2026-09-18 booted the real server and walked A1 by
//! hand, and the path ended at `SIM TEAM`: the character → account map was built
//! once at boot, so a character created *after* boot was unmapped, `SIM TEAM`
//! failed closed on it (doc 10 R3), and the refusal advised the one thing that
//! could not help. Every other lens in this directory seeds its accounts in the
//! store **before** the engine is built, which is faster and which is exactly why
//! none of them could see it.
//!
//! So this lens does the opposite and nothing else: the world boots with **no
//! player accounts at all**, and only then do two people connect to the welcome
//! screen, type `create`, walk `ooc` → `simulator`, and claim a slot each. The
//! composition, the wiring and the front door are production's; the two accounts
//! are made the way a playtester makes one.
//!
//! Its twin through the split gateway is
//! `crates/mush-gateway/tests/front_door_split.rs`, where the create crosses a
//! real process boundary as a `mush-ipc` `ServiceReq::CreatePlayer`.
//!
//! # What is asserted, and what would have failed before
//!
//! | step | before cut F.4 |
//! |---|---|
//! | `create` at the welcome screen | fine — the Player and the account document are made |
//! | `SIM TEAM red` | **"Your account is not mapped yet … reconnect"**, for ever |
//! | `SIM TEAM blue` by the second account | the same |
//! | one slot per **account** | untestable: no claim ever succeeded |
//!
//! The last row is the one the map's shape decides rather than its freshness, so
//! it is asserted here too: the second character of the **same** account — made
//! after boot with `@newchar`, which is the other path that creates a character —
//! is refused a second slot, because both characters resolve to one account id.

mod support;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::Engine;
use mush_core::module::GameModule;
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{InMemoryStore, ObjectStore};
use mushd::accounts::Accounts;
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps};

use support::{Term, play_client, scratch_dir, strip_ansi};

const PASSWORD: &str = "front-door-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

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

/// The production module list, in production order, with the starter area on
/// (the centre is in it) and the TacSim layer on (the arena and its roster are
/// its ride-along). Nothing here is tuned: this lens never starts a match.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
) -> Vec<Box<dyn GameModule>> {
    let tacsim = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module_with(tacsim, true));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

/// Boot the whole thing on an empty store. **No account is seeded** — that is
/// the point of the lens, and `hydrate` therefore returns an empty map.
async fn boot(tag: &str) -> Server {
    let dir = scratch_dir(tag);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let world = load_or_seed_world(&*store).await.expect("boot world");

    // The live grid is Limbo, which the OOC pack hangs its `ooc` exit off, and
    // chargen is off so a new character lands there able to walk. Approval is
    // not what this lens is about.
    let onboarding = mush_core::OnboardingConfig {
        chargen_required: false,
        start_room: mush_core::LIMBO,
        start_room_unapproved: mush_core::LIMBO,
    };
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).expect("server commands");
    let mock: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("openrouter"));
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", mock)
            .global_enabled(false)
            .build(),
    );

    let mut engine =
        Engine::with_modules(world, registry, module_list(&onboarding, &llm)).expect("modules");
    // The production installation, in production's own order
    // (`mush_server::boot::build_engine`: hydrate, install into both readers,
    // and only then `boot_modules`): ONE instance behind the engine's oracle and
    // the sim's holder. Since cut F.4 the world task writes through that one
    // instance, which is what this lens exists to prove.
    let identity: Arc<dyn mush_core::AccountIdentity> = Arc::new(
        mush_server::accounts_identity::hydrate(&*store)
            .await
            .expect("the account map hydrates"),
    );
    engine.set_account_identity(Arc::clone(&identity));
    if let Some(slot) = engine.world.module_state_mut::<mush_sim::AccountOracle>() {
        *slot = mush_sim::AccountOracle::new(identity);
    }
    assert_eq!(
        engine.account_of(mush_core::GOD),
        None,
        "the store has no accounts, so the boot map is empty"
    );
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(300),
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
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 8));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let listener = start_listener(
        "127.0.0.1:0".parse().expect("a loopback address"),
        &dir,
        "Welcome to the simulator.".to_string(),
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

// ---------------------------------------------------------------------------
// The terminal
// ---------------------------------------------------------------------------

/// Run one line and wait for `needle`, retrying the world's one transient.
async fn say(term: &mut Term, line: &str, needle: &str) {
    assert!(
        !line.contains(needle),
        "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
    );
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        match term
            .expect_one_of_after(mark, &[needle, COMMIT_PENDING])
            .await
        {
            0 => return,
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
    panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
}

/// Run one line, wait for `needle`, and hand back everything that arrived.
async fn ask(term: &mut Term, line: &str, needle: &str) -> String {
    let mark = term.mark();
    say(term, line, needle).await;
    strip_ansi(&term.seen[mark..])
}

/// **Make an account and a character at the welcome screen** — the front door,
/// with nothing seeded behind it.
async fn register(
    addr: SocketAddr,
    name: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the simulator.").await;
    term.send_line(&format!("create {name} {PASSWORD}")).await;
    term.expect("Account created.").await;
    term.expect(&format!("Connected as {name}")).await;
    (handle, term)
}

/// Walk Limbo → the Welcome Hall → the Simulator Lobby, by the exits' own
/// aliases. `simulator` rather than `sim`, because cut F.4 took `sim` off the
/// exit: it is the verb's word.
async fn walk_to_the_lobby(term: &mut Term) {
    say(term, "ooc", "Welcome Hall").await;
    // 16b cut D.5 (16c `B-11`): `l` looks. It used to be the Welcome Hall's
    // exit to the Lounge, so the room every pilot crosses to the simulator
    // walked them out of it. The hall's own name coming back is the proof.
    say(term, "l", "Welcome Hall").await;
    say(term, "simulator", "Simulator Lobby").await;
}

// ---------------------------------------------------------------------------
// The lens
// ---------------------------------------------------------------------------

/// **Two accounts made after boot each claim a slot.** The K6 point, end to end
/// over real SSH on the production composition.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_accounts_made_after_boot_walk_in_and_each_claims_a_slot() {
    let server = boot("front-door").await;

    // (1) Two people register at the welcome screen. Neither exists in any map
    // the boot pass read: the store had no accounts at all when it ran.
    let (_r, mut red) = register(server.addr, "Rowan").await;
    let (_b, mut blue) = register(server.addr, "Bly").await;

    // (2) They walk in through the OOC area, as a newcomer does.
    walk_to_the_lobby(&mut red).await;
    walk_to_the_lobby(&mut blue).await;

    // (3) Both claims succeed — and the announce reaches the other person in the
    // room, which is how each of them knows the other really holds a slot.
    let watching = blue.mark();
    say(&mut red, "SIM TEAM red", "takes red slot 1").await;
    blue.expect_after(watching, "Rowan takes red slot 1").await;
    say(&mut blue, "SIM TEAM blue", "takes blue slot 1").await;

    // (4) The board agrees, which is the centre's own record rather than the
    // claim's echo.
    let status = ask(&mut red, "SIM STATUS", "blue:").await;
    assert!(status.contains("selection"), "{status}");
    assert!(status.contains("Rowan"), "{status}");
    assert!(status.contains("Bly"), "{status}");

    // (5) And the refusal that used to stand here is nowhere in either
    // transcript, in either of its spellings.
    for (who, seen) in [
        ("Rowan", strip_ansi(&red.seen)),
        ("Bly", strip_ansi(&blue.seen)),
    ] {
        assert!(
            !seen.contains("not mapped yet"),
            "{who} was told its account was not mapped:\n{seen}"
        );
        assert!(
            !seen.contains("no account on record"),
            "{who} was told it has no account on record:\n{seen}"
        );
    }
}

/// **One slot per account still holds for a character made after boot.** An alt
/// created with `@newchar` maps to the same account as the character that made
/// it, so it cannot take a second slot — which is the property the map's *shape*
/// decides, and which only means anything now that a post-boot character maps at
/// all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_alt_made_after_boot_shares_its_account_and_cannot_take_a_second_slot() {
    let server = boot("front-door-alt").await;

    let (_r, mut red) = register(server.addr, "Quill").await;
    walk_to_the_lobby(&mut red).await;
    say(&mut red, "SIM TEAM red", "takes red slot 1").await;

    // The alt, made from inside the game on the caller's own account.
    say(&mut red, "@newchar Quillon", "created Quillon").await;

    // Play it: the account now owns two characters, so the welcome screen offers
    // the character select and the alt is picked by name.
    let (_a, mut alt) = {
        let (handle, mut term) = play_client(server.addr).await;
        term.expect("Welcome to the simulator.").await;
        term.send_line(&format!("connect Quill {PASSWORD}")).await;
        term.expect("Characters on Quill:").await;
        term.send_line("Quillon").await;
        term.expect("Connected as Quillon").await;
        (handle, term)
    };
    walk_to_the_lobby(&mut alt).await;
    // One slot per **account**, not per character: the alt is told which slot its
    // account already holds, which is only sayable because the alt maps at all.
    say(&mut alt, "SIM TEAM blue", "You already hold red slot 1.").await;
}

/// **A macro made at the front door survives a reconnect** (16b cut AD.1, 16c
/// N14). A character made after boot defines a macro, types it, lists it, quits
/// and comes back: the macro is on the character, read by the kernel's alias
/// stage on the production composition over real SSH.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_macro_made_at_the_front_door_survives_a_reconnect() {
    let server = boot("front-door-macro").await;

    let (_r, mut rowan) = register(server.addr, "Rowan").await;
    say(
        &mut rowan,
        "@macro hey=say hello, $1",
        "Macro hey set: say hello, $1",
    )
    .await;
    say(&mut rowan, "hey Bly", "hello, Bly").await;
    let listed = ask(&mut rowan, "@macro", "hey").await;
    assert!(listed.contains("Your macros (1 of 20):"), "{listed}");

    rowan.send_line("QUIT").await;
    rowan.expect_close().await;

    let (_again, mut back) = play_client(server.addr).await;
    back.expect("Welcome to the simulator.").await;
    back.send_line(&format!("connect Rowan {PASSWORD}")).await;
    back.expect("Connected as Rowan").await;
    say(&mut back, "hey again", "hello, again").await;
}
