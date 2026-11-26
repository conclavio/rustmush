//! **The front door, through the split** (16b cut F.4; doc 16 §1, §3.1's *the
//! output path differs*; doc 10 R3).
//!
//! `crates/mushd/tests/sim_lenses/front_door.rs` is the in-process half. This is the shape
//! that is deployed: the SSH front door, the session layer and the account store
//! live on the **gateway**, the world and its account oracle live on the
//! **server**, and a character is created by a `mush-ipc`
//! `ServiceReq::CreatePlayer` crossing between them.
//!
//! That request is what cut F.4 changed. It used to carry a name and nothing
//! else, so the server made a Player its own account oracle never heard of, and
//! the gateway wrote the account document afterwards — a document only the *next*
//! boot would read. A player who registered on playtest night could not claim a
//! simulator slot at all. It now carries the account id, the server inserts
//! `character -> account` in the world-task turn that creates the Player, and the
//! claim works before the reply has crossed back.
//!
//! So this lens boots both halves against **one store with no player accounts**,
//! and two people register at the welcome screen, walk in, and claim a slot each.
//! Nothing is seeded and nothing is restarted.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_store::{InMemoryStore, ObjectStore};
use tokio::sync::oneshot;

use mush_config::config::Config;
use mush_gateway::GatewayOptions;

use super::harness::{AcceptAll, ServerUnit, Term, WORLD_NOT_UP, when_admitted};

const PASSWORD: &str = "front-door-1";

// ---------------------------------------------------------------------------
// The split rig — `arena_story_split.rs`', with an EMPTY shared store
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-front-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// Both halves' configuration. The TacSim layer is **on**, which is what puts the
/// simulator centre and its roster in the world — and, since cut F.4, what the
/// shipped dev configs do too.
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.tacsim.enabled = true;
    cfg
}

/// The server unit over `store` (`harness.rs`): a real server on its own
/// thread + runtime, which the gateway dials. Nothing here restarts it; the
/// unit handle may be dropped (the server runs on until the process ends).
fn unit(store: Arc<dyn ObjectStore>) -> ServerUnit {
    ServerUnit::new(store, || test_config("server"))
}

async fn start_split() -> SocketAddr {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = unit(Arc::clone(&store));
    unit.start().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let gw_store = Arc::clone(&store);
    let mut cfg = test_config("gateway");
    unit.aim(&mut cfg);
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let _ = rt.block_on(mush_gateway::serve(
            cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
                generation: None,
                ..Default::default()
            },
        ));
    });
    tokio::time::timeout(Duration::from_secs(60), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr")
}

// ---------------------------------------------------------------------------
// The terminal — `arena_story_split.rs`'s, trimmed to what this lens types
// ---------------------------------------------------------------------------

/// Make an account and its first character at the welcome screen — the create
/// that crosses the boundary.
/// Walk Limbo to the Welcome Hall to the Simulator Lobby by the exits' own
/// aliases (`simulator`, never `sim`: cut F.4 took that word off the exit).
async fn walk_to_the_lobby(term: &mut Term) {
    term.say("ooc", "Welcome Hall").await;
    // 16b cut D.5 (16c `B-11`): `l` looks, in the room every pilot crosses
    // on the way in. The hall's own name coming back is the proof.
    term.say("l", "Welcome Hall").await;
    term.say("simulator", "Simulator Lobby").await;
}

async fn register(addr: SocketAddr, name: &str) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut term) = Term::play(addr).await;
    let connected = format!("Connected as {name}");
    // The rig has no barrier against the server half still booting, so the
    // registration waits it out (`harness::when_admitted`).
    let mark = term.mark();
    when_admitted(
        &mut term,
        &format!("create {name} {PASSWORD}"),
        &["Account created."],
    )
    .await;
    // `create` logs the new character in itself. Only if the world came up
    // between the two halves of that one command does the rig type the login.
    let at = term
        .expect_one_of_after(mark, &[&connected, WORLD_NOT_UP[0]])
        .await;
    if at != 0 {
        when_admitted(
            &mut term,
            &format!("connect {name} {PASSWORD}"),
            &[&connected],
        )
        .await;
    }
    (handle, term)
}

// ---------------------------------------------------------------------------
// The lens
// ---------------------------------------------------------------------------

/// **Two accounts made after boot each claim a slot, across the boundary.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_accounts_made_after_boot_claim_a_slot_each_through_the_gateway() {
    let addr = start_split().await;

    // The gateway reserves each account document and asks the server to make the
    // Player; the account id rides on that request.
    let (_r, mut red) = register(addr, "Rowan").await;
    let (_b, mut blue) = register(addr, "Bly").await;

    walk_to_the_lobby(&mut red).await;
    walk_to_the_lobby(&mut blue).await;

    let watching = blue.mark();
    red.say("SIM TEAM red", "takes red slot 1").await;
    blue.expect_after(watching, "Rowan takes red slot 1").await;
    blue.say("SIM TEAM blue", "takes blue slot 1").await;

    let status = red.ask("SIM STATUS", "blue:").await;
    assert!(status.contains("selection"), "{status}");
    assert!(status.contains("Rowan"), "{status}");
    assert!(status.contains("Bly"), "{status}");

    for (who, seen) in [("Rowan", red.since(0)), ("Bly", blue.since(0))] {
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
