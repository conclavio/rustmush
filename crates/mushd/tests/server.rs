//! Integration tests for the mushd wiring: the single-writer world task,
//! session fan-out, the typed async re-entry seam (`@slowtest`), and the
//! SSH bridge flows (accounts, connect, QUIT, linkdead, flood, IP guard) —
//! all without sockets: fake connections are plain mpsc channels.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_core::{GOD, Inbound, LIMBO, ObjId, ObjKind};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent};
use mush_session::{ConnId, ConnectionHandle, SessionConfig, SessionManager};
use mush_testkit::text_of;
use tokio::sync::mpsc;

use mush_store::{InMemoryStore, ObjectStore};
use mushd::accounts::{Accounts, AuthBackend, AuthOutcome, CharSummary, CreateErr};
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{AuthResult, ConnectErr, LineOutcome, NewConn, SshBridge};
use mushd::world_task::{self, CreatePlayerErr, WorldHandle, char_key};

type EventRx = mpsc::Receiver<Arc<OutputEvent>>;

fn caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi16,
        width: 80,
        height: 24,
        utf8: true,
        structured: false,
    }
}

fn ip(last: u8) -> IpAddr {
    IpAddr::from([127, 0, 0, last])
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mushd-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Receive events until one renders to text containing `needle`; panic on
/// channel close or timeout. Returns the matching event.
async fn expect_text(rx: &mut EventRx, needle: &str) -> Arc<OutputEvent> {
    let deadline = Duration::from_secs(60);
    let fut = async {
        loop {
            match rx.recv().await {
                Some(ev) => {
                    if text_of(&ev).contains(needle) {
                        return ev;
                    }
                }
                None => panic!("event channel closed while waiting for {needle:?}"),
            }
        }
    };
    tokio::time::timeout(deadline, fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"))
}

/// Attach a fake connection (bare channel, no bridge) for `actor` and
/// announce it to the world. Returns the receiving end.
async fn attach_direct(
    sessions: &SessionManager,
    world: &WorldHandle,
    conn: u64,
    actor: ObjId,
    account: &str,
) -> EventRx {
    attach_direct_buffered(sessions, world, conn, actor, account, 256).await
}

/// [`attach_direct`] with an explicit output-queue capacity (slow-client
/// tests use a tiny one).
async fn attach_direct_buffered(
    sessions: &SessionManager,
    world: &WorldHandle,
    conn: u64,
    actor: ObjId,
    account: &str,
    buffer: usize,
) -> EventRx {
    let (tx, rx) = mpsc::channel(buffer);
    let handle = ConnectionHandle::new(ConnId(conn), ip(9), caps(), tx);
    sessions.attach(handle, char_key(actor), account.to_string());
    assert!(
        world
            .inbound(Inbound::ConnUp {
                conn,
                actor,
                caps_structured: false,
                from_linkdead: false,
            })
            .await
    );
    assert!(
        world.barrier().await,
        "direct test connection must cross the ConnUp commit/order boundary"
    );
    rx
}

#[tokio::test(flavor = "multi_thread")]
async fn world_task_fans_out_say_and_who() {
    let sessions = Arc::new(SessionManager::new());
    let mut engine = world_task::boot_engine();
    let alice = engine
        .world
        .create(ObjKind::Player, "Alice", GOD, Some(LIMBO))
        .unwrap();
    let bob = engine
        .world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    let (world, join) = world_task::spawn(engine, Arc::clone(&sessions));

    let mut alice_rx = attach_direct(&sessions, &world, 1, alice, "alice").await;
    let mut bob_rx = attach_direct(&sessions, &world, 2, bob, "bob").await;

    // Alice (already up) sees Bob connect.
    expect_text(&mut alice_rx, "Bob has connected.").await;

    // say fans out: the world resolves room -> actors, the session manager
    // resolves actor -> connections.
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: alice,
                line: "say hello there".into(),
            })
            .await
    );
    let ev = expect_text(&mut bob_rx, "hello there").await;
    assert!(
        matches!(&*ev, OutputEvent::Narrative { .. }),
        "say should arrive as a Narrative, got {ev:?}"
    );

    // WHO reaches only the asker, with real (well, zero-aged) times.
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 2,
                actor: bob,
                line: "WHO".into(),
            })
            .await
    );
    let ev = expect_text(&mut bob_rx, "players connected").await;
    let OutputEvent::Who(entries) = &*ev else {
        panic!("expected Who, got {ev:?}");
    };
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"Alice") && names.contains(&"Bob"),
        "{names:?}"
    );

    // Shutdown notice reaches everyone; the task exits.
    assert!(
        world
            .inbound(Inbound::Control(mush_core::ControlMsg::Shutdown))
            .await
    );
    expect_text(&mut alice_rx, "Shutdown").await;
    expect_text(&mut bob_rx, "Shutdown").await;
    join.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn create_player_is_a_typed_world_message() {
    let sessions = Arc::new(SessionManager::new());
    let (world, join) = world_task::spawn(world_task::boot_engine(), sessions);
    let id = world.create_player("Zoe", None).await.unwrap();
    assert!(id.0 > GOD.0);
    assert_eq!(
        world.create_player("Zoe", None).await,
        Err(CreatePlayerErr::NameTaken)
    );
    drop(world);
    join.await.unwrap();
}

/// The M1 exit-criterion seam: a fake 3-second async call re-enters the
/// world as a typed command without ever blocking the loop. Tokio's paused
/// clock proves both halves: a command sent *after* `@slowtest` completes
/// *before* it, and the whole test runs in well under 3 real seconds.
#[tokio::test(start_paused = true)]
async fn slowtest_reenters_typed_without_blocking_the_loop() {
    let started = Instant::now();
    let sessions = Arc::new(SessionManager::new());
    let (world, _join) = world_task::spawn(world_task::boot_engine(), Arc::clone(&sessions));
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@slowtest".into(),
            })
            .await
    );
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "think loop-not-blocked".into(),
            })
            .await
    );

    expect_text(&mut god_rx, "Thinking...").await;
    // The think lands BEFORE the 3s completion: the loop never blocked.
    expect_text(&mut god_rx, "loop-not-blocked").await;
    expect_text(&mut god_rx, "SLOWTEST: fake async call").await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "paused-clock test must not really sleep (took {:?})",
        started.elapsed()
    );
}

/// `@wait` rides the scheduler sleep in the world task select loop. Real
/// time (not paused): the scheduler's deadlines are wall-clock unix ms, so
/// only a real clock moves them.
#[tokio::test(flavor = "multi_thread")]
async fn wait_timer_fires_through_the_loop() {
    let sessions = Arc::new(SessionManager::new());
    let (world, _join) = world_task::spawn(world_task::boot_engine(), Arc::clone(&sessions));
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@wait 1=think delayed-fire".into(),
            })
            .await
    );
    expect_text(&mut god_rx, "delayed-fire").await;
}

// ---------------------------------------------------------------------------
// SSH bridge flows
// ---------------------------------------------------------------------------

/// A permissive auth backend for bridge tests that must not pay for argon2:
/// any credentials pass, the character list is fixed, creation unsupported.
struct FakeAuth {
    chars: Vec<CharSummary>,
}

impl AuthBackend for FakeAuth {
    async fn verify(&self, account: &str, _password: &str) -> AuthOutcome {
        AuthOutcome::Ok {
            account: account.to_string(),
            characters: self.chars.clone(),
        }
    }
    async fn characters(&self, _account: &str) -> Vec<CharSummary> {
        self.chars.clone()
    }
    async fn create(
        &self,
        _account: &str,
        _password: &str,
        _char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        Err(CreateErr::Io("FakeAuth cannot create".into()))
    }
    async fn create_character(
        &self,
        _account: &str,
        _char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        Err(CreateErr::Io("FakeAuth cannot create".into()))
    }
}

struct BridgeRig {
    sessions: Arc<SessionManager>,
    bridge: Arc<SshBridge<FakeAuth>>,
}

/// World with players Nick + Ada, a fake-auth bridge, configurable grace.
fn bridge_rig(grace: Duration) -> (BridgeRig, ObjId, ObjId) {
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: grace,
        ..SessionConfig::default()
    }));
    let mut engine = world_task::boot_engine();
    let nick = engine
        .world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let ada = engine
        .world
        .create(ObjKind::Player, "Ada", GOD, Some(LIMBO))
        .unwrap();
    let (world, _join) = world_task::spawn(engine, Arc::clone(&sessions));
    let auth = Arc::new(FakeAuth {
        chars: vec![
            CharSummary {
                id: nick,
                name: "Nick".into(),
            },
            CharSummary {
                id: ada,
                name: "Ada".into(),
            },
        ],
    });
    let bridge = Arc::new(SshBridge::new(
        auth,
        Arc::clone(&sessions),
        world,
        Arc::new(IpGuard::new()),
    ));
    (BridgeRig { sessions, bridge }, nick, ada)
}

async fn bridge_connect(rig: &BridgeRig, conn: u64, actor: ObjId, addr: IpAddr) -> EventRx {
    let (tx, rx) = mpsc::channel(256);
    rig.bridge
        .connect_character(
            NewConn {
                id: ConnId(conn),
                addr,
                caps: caps(),
                tx,
                stamped_tx: None,
            },
            "fake",
            u64::from(actor.0),
        )
        .await
        .unwrap();
    rx
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_connect_look_say_quit_flow() {
    let (rig, nick, ada) = bridge_rig(Duration::from_secs(300));
    let mut nick_rx = bridge_connect(&rig, 10, nick, ip(1)).await;
    let mut ada_rx = bridge_connect(&rig, 11, ada, ip(2)).await;

    // Connecting shows who/where you are, then the room.
    expect_text(&mut nick_rx, "Connected as Nick (fake)").await;
    expect_text(&mut nick_rx, "Limbo(#0)").await;
    // Nick (already in the room) sees Ada arrive.
    expect_text(&mut nick_rx, "Ada has connected.").await;
    expect_text(&mut ada_rx, "Limbo(#0)").await;

    // Input flows through the bridge into the world and fans out.
    assert_eq!(
        rig.bridge.line(ConnId(10), "say ahoy ada").await,
        LineOutcome::Continue
    );
    expect_text(&mut ada_rx, "ahoy ada").await;

    // Sanitation: ESC never reaches another player's screen.
    rig.bridge
        .line(ConnId(10), "say \x1b[31mred injection")
        .await;
    let ev = expect_text(&mut ada_rx, "red injection").await;
    assert!(!text_of(&ev).contains('\x1b'), "ESC must be stripped");

    // A character not on the account is refused.
    let (tx, _rx2) = mpsc::channel(8);
    let err = rig
        .bridge
        .connect_character(
            NewConn {
                id: ConnId(12),
                addr: ip(1),
                caps: caps(),
                tx,
                stamped_tx: None,
            },
            "fake",
            u64::from(GOD.0),
        )
        .await
        .unwrap_err();
    assert_eq!(err, ConnectErr::NotYourCharacter);

    // QUIT: farewell + clean close + room announcement, no linkdead.
    assert_eq!(
        rig.bridge.line(ConnId(10), "QUIT").await,
        LineOutcome::Close
    );
    expect_text(&mut nick_rx, "Goodbye.").await;
    expect_text(&mut ada_rx, "Nick has disconnected.").await;
    assert_eq!(rig.sessions.session_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_linkdead_announces_then_reaps_after_grace() {
    let (rig, nick, ada) = bridge_rig(Duration::from_millis(500));
    let _nick_rx = bridge_connect(&rig, 20, nick, ip(1)).await;
    let mut ada_rx = bridge_connect(&rig, 21, ada, ip(2)).await;
    expect_text(&mut ada_rx, "Limbo(#0)").await;

    // Transport loss: linkdead announce, character still around...
    rig.bridge.disconnected(ConnId(20)).await;
    expect_text(&mut ada_rx, "Nick has gone link-dead.").await;
    assert!(
        rig.sessions.session_count() == 2,
        "linkdead session retained"
    );

    // ...until the grace expires, then the full disconnect.
    expect_text(&mut ada_rx, "Nick has disconnected.").await;
    assert_eq!(rig.sessions.session_count(), 1);

    // WHO no longer lists Nick (only ada: "1 player connected.").
    rig.bridge.line(ConnId(21), "WHO").await;
    let ev = expect_text(&mut ada_rx, "player connected").await;
    let OutputEvent::Who(entries) = &*ev else {
        panic!("expected Who");
    };
    assert!(entries.iter().all(|e| e.name != "Nick"), "{entries:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_reconnect_cancels_linkdead() {
    let (rig, nick, ada) = bridge_rig(Duration::from_millis(300));
    let _nick_rx = bridge_connect(&rig, 30, nick, ip(1)).await;
    let mut ada_rx = bridge_connect(&rig, 31, ada, ip(2)).await;
    expect_text(&mut ada_rx, "Limbo(#0)").await;

    rig.bridge.disconnected(ConnId(30)).await;
    expect_text(&mut ada_rx, "Nick has gone link-dead.").await;

    // Reconnect within the grace: the session revives and the bridge
    // forwards `from_linkdead`, so the room hears the classic
    // "has reconnected." — and the reaper never fires.
    let mut nick_rx2 = bridge_connect(&rig, 32, nick, ip(1)).await;
    expect_text(&mut ada_rx, "Nick has reconnected.").await;
    expect_text(&mut nick_rx2, "Limbo(#0)").await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        rig.sessions.session_count(),
        2,
        "reap must have been skipped"
    );
    // Nick still hears the room.
    rig.bridge.line(ConnId(31), "say still with us").await;
    expect_text(&mut nick_rx2, "still with us").await;
}

/// Multi-connect (the untested half of treaty ruling 2): a second
/// connection joins the same session, both receive room output, and
/// dropping one of two is silent while the survivor keeps working.
#[tokio::test(flavor = "multi_thread")]
async fn bridge_multi_connect_fans_out_and_single_drop_is_silent() {
    let (rig, nick, ada) = bridge_rig(Duration::from_secs(300));
    let mut nick_rx1 = bridge_connect(&rig, 60, nick, ip(1)).await;
    let mut ada_rx = bridge_connect(&rig, 61, ada, ip(2)).await;
    expect_text(&mut nick_rx1, "Limbo(#0)").await;
    expect_text(&mut ada_rx, "Limbo(#0)").await;

    // Second connection for the same character: joins, announces.
    let mut nick_rx2 = bridge_connect(&rig, 62, nick, ip(1)).await;
    expect_text(&mut ada_rx, "Nick has reconnected.").await;
    expect_text(&mut nick_rx2, "Limbo(#0)").await;
    assert_eq!(rig.sessions.session_count(), 2, "one session, two conns");
    assert_eq!(rig.sessions.connection_count(), 3);

    // Room output reaches BOTH of Nick's connections.
    rig.bridge.line(ConnId(61), "say hello both").await;
    expect_text(&mut nick_rx1, "hello both").await;
    expect_text(&mut nick_rx2, "hello both").await;

    // Dropping one of two: no announcement, survivor still routed.
    rig.bridge.disconnected(ConnId(60)).await;
    rig.bridge.line(ConnId(61), "say still here").await;
    let mut leaked = Vec::new();
    let deadline = Duration::from_secs(60);
    tokio::time::timeout(deadline, async {
        loop {
            let ev = ada_rx.recv().await.expect("ada channel open");
            let text = text_of(&ev);
            if text.contains("link-dead") || text.contains("disconnected") {
                leaked.push(text.clone());
            }
            if text.contains("still here") {
                break;
            }
        }
    })
    .await
    .expect("ada must hear the follow-up say");
    assert!(
        leaked.is_empty(),
        "dropping one of two connections must be silent, got {leaked:?}"
    );
    expect_text(&mut nick_rx2, "still here").await;
    assert_eq!(rig.sessions.connection_count(), 2);
}

/// Negative delivery: a Room audience must reach only that room. Guards
/// against an `Audience::Room` → `All` regression, which every
/// presence-only assertion would miss.
#[tokio::test(flavor = "multi_thread")]
async fn say_does_not_leak_to_other_rooms() {
    let sessions = Arc::new(SessionManager::new());
    let mut engine = world_task::boot_engine();
    let alice = engine
        .world
        .create(ObjKind::Player, "Alice", GOD, Some(LIMBO))
        .unwrap();
    let vault = engine
        .world
        .create(ObjKind::Room, "Vault", GOD, None)
        .unwrap();
    let bob = engine
        .world
        .create(ObjKind::Player, "Bob", GOD, Some(vault))
        .unwrap();
    let (world, _join) = world_task::spawn(engine, Arc::clone(&sessions));
    let _alice_rx = attach_direct(&sessions, &world, 1, alice, "alice").await;
    let mut bob_rx = attach_direct(&sessions, &world, 2, bob, "bob").await;

    // Alice speaks in Limbo; Bob (in the Vault) then produces a sentinel.
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: alice,
                line: "say secret-for-limbo".into(),
            })
            .await
    );
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 2,
                actor: bob,
                line: "think bob-sentinel".into(),
            })
            .await
    );
    // Everything Bob receives up to his sentinel must exclude the say.
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let ev = bob_rx.recv().await.expect("bob channel open");
            let text = text_of(&ev);
            assert!(
                !text.contains("secret-for-limbo"),
                "room output leaked into another room: {text:?}"
            );
            if text.contains("bob-sentinel") {
                break;
            }
        }
    })
    .await
    .expect("bob must see his sentinel");
}

/// The slow-client policy (design 02 §8.4): a connection whose output
/// buffer overflows is detached as a slow client, the engine hears the
/// ConnDown (link-dead announce), and nobody else is affected.
#[tokio::test(flavor = "multi_thread")]
async fn slow_client_is_detached_and_goes_linkdead() {
    let sessions = Arc::new(SessionManager::new());
    let mut engine = world_task::boot_engine();
    let alice = engine
        .world
        .create(ObjKind::Player, "Alice", GOD, Some(LIMBO))
        .unwrap();
    let bob = engine
        .world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    let (world, _join) = world_task::spawn(engine, Arc::clone(&sessions));
    let mut alice_rx = attach_direct(&sessions, &world, 1, alice, "alice").await;
    // Bob's queue holds one event and nobody drains it (kept alive so the
    // channel reads as full, not closed).
    let _bob_rx = attach_direct_buffered(&sessions, &world, 2, bob, "bob", 1).await;
    expect_text(&mut alice_rx, "Bob has connected.").await;

    // Two says: the first fills Bob's queue, the second overflows it and
    // triggers the slow-client detach inside the same delivery pass.
    for line in ["say filling the pipe", "say overflow now"] {
        assert!(
            world
                .inbound(Inbound::Cmd {
                    conn: 1,
                    actor: alice,
                    line: line.into(),
                })
                .await
        );
    }
    expect_text(&mut alice_rx, "Bob has gone link-dead.").await;
    assert!(
        rig_session_gone(&sessions, 2),
        "slow connection must be detached"
    );
    // The world loop and other players are unaffected.
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: alice,
                line: "think still-alive".into(),
            })
            .await
    );
    expect_text(&mut alice_rx, "still-alive").await;
}

fn rig_session_gone(sessions: &SessionManager, conn: u64) -> bool {
    sessions.session_of_conn(ConnId(conn)).is_none()
}

/// QUIT is serialized through the world task: output for lines typed just
/// before QUIT reaches the quitter before the farewell, never dropped.
#[tokio::test(flavor = "multi_thread")]
async fn quit_flushes_pending_output_first() {
    let (rig, nick, _ada) = bridge_rig(Duration::from_secs(300));
    let mut nick_rx = bridge_connect(&rig, 70, nick, ip(1)).await;
    expect_text(&mut nick_rx, "Limbo(#0)").await;

    // Command and QUIT back-to-back (a client trigger / fast paste).
    rig.bridge.line(ConnId(70), "think parting-thought").await;
    assert_eq!(
        rig.bridge.line(ConnId(70), "QUIT").await,
        LineOutcome::Close
    );

    // Drain to channel close (detach dropped the sender): the thought must
    // be present and precede the goodbye.
    let mut texts = Vec::new();
    while let Some(ev) = nick_rx.recv().await {
        texts.push(text_of(&ev));
    }
    let thought = texts.iter().position(|t| t.contains("parting-thought"));
    let goodbye = texts.iter().position(|t| t.contains("Goodbye."));
    assert!(
        thought.is_some(),
        "pre-QUIT output must not be dropped: {texts:?}"
    );
    assert!(goodbye.is_some(), "QUIT must say goodbye: {texts:?}");
    assert!(
        thought < goodbye,
        "pre-QUIT output must precede the farewell: {texts:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_flood_control_throttles() {
    let (rig, nick, _ada) = bridge_rig(Duration::from_secs(300));
    let mut nick_rx = bridge_connect(&rig, 40, nick, ip(1)).await;
    expect_text(&mut nick_rx, "Limbo(#0)").await;
    // Burst cap is 30; 35 instant lines must trip the throttle.
    for _ in 0..35 {
        rig.bridge.line(ConnId(40), "think spam").await;
    }
    expect_text(&mut nick_rx, "You're doing that too fast.").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_ip_guard_blocks_after_failures() {
    let (rig, _nick, _ada) = bridge_rig(Duration::from_secs(300));
    assert!(!rig.bridge.is_ip_blocked(ip(7)));
    for _ in 0..5 {
        rig.bridge.auth_failed(ip(7));
    }
    assert!(rig.bridge.is_ip_blocked(ip(7)));
    // Even good credentials are refused while blocked (FakeAuth passes all).
    assert_eq!(
        rig.bridge.auth_account("fake", "whatever", ip(7)).await,
        AuthResult::Denied
    );
    // Other IPs unaffected.
    assert!(matches!(
        rig.bridge.auth_account("fake", "whatever", ip(8)).await,
        AuthResult::Ok { .. }
    ));
}

/// The real store-backed accounts end to end: create over the bridge
/// (argon2 + ack-gated world round trip), auth, connect, and the newchar
/// cap. (Restart survival is covered by tests/persistence.rs.)
#[tokio::test(flavor = "multi_thread")]
async fn accounts_create_auth_connect_end_to_end() {
    let sessions = Arc::new(SessionManager::new());
    let (world, _join) = world_task::spawn(world_task::boot_engine(), Arc::clone(&sessions));
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), world.clone(), 2));
    let bridge = Arc::new(SshBridge::new(
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        world.clone(),
        Arc::new(IpGuard::new()),
    ));

    // Create: account + Player object via the typed world message.
    let created = bridge
        .create_account("nick", "hunter42", "Nick", ip(1))
        .await
        .unwrap();
    assert_eq!(created.name, "Nick");

    // The character name is now taken world-side; the account rolls back.
    let err = bridge
        .create_account("other", "hunter42", "Nick", ip(1))
        .await
        .unwrap_err();
    assert_eq!(err, CreateErr::CharacterNameTaken);
    assert!(matches!(
        bridge.auth_account("other", "hunter42", ip(1)).await,
        AuthResult::Denied
    ));

    // Auth against the real argon2 hash.
    let AuthResult::Ok {
        account,
        characters,
    } = bridge.auth_account("NICK", "hunter42", ip(1)).await
    else {
        panic!("auth should succeed");
    };
    assert_eq!(account, "nick");
    assert_eq!(characters.len(), 1);
    assert!(matches!(
        bridge.auth_account("nick", "wrong", ip(1)).await,
        AuthResult::Denied
    ));

    // Connect the freshly created character and play a line.
    let (tx, mut rx) = mpsc::channel(256);
    bridge
        .connect_character(
            NewConn {
                id: ConnId(50),
                addr: ip(1),
                caps: caps(),
                tx,
                stamped_tx: None,
            },
            "nick",
            characters[0].id,
        )
        .await
        .unwrap();
    expect_text(&mut rx, "Connected as Nick (nick)").await;
    expect_text(&mut rx, "Limbo(#0)").await;
    bridge.line(ConnId(50), "think it works").await;
    expect_text(&mut rx, "it works").await;

    // newchar honors the per-account cap (2 here): one more fits, the
    // next is refused before touching the world.
    let second = bridge.create_character("nick", "Nick2").await.unwrap();
    assert_eq!(second.name, "Nick2");
    assert_eq!(
        bridge.create_character("nick", "Nick3").await.unwrap_err(),
        CreateErr::CharacterLimit
    );
    assert_eq!(
        bridge.create_character("nick", "n").await.unwrap_err(),
        CreateErr::BadCharacterName
    );
    // Both characters are on the account doc in the store.
    let rec = store.get_account("nick").await.unwrap().unwrap();
    let names: Vec<&str> = rec.characters.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["Nick", "Nick2"]);
}

// ---------------------------------------------------------------------------
// Full stack over a real socket
// ---------------------------------------------------------------------------

/// The M1 exit criterion end to end: two real SSH clients against the whole
/// server — listener, welcome screen, account creation (argon2 + world
/// round trip), shared room, say fan-out, WHO, QUIT — with only the port
/// (ephemeral) and data dir (scratch) differing from production wiring.
mod ssh_full_stack {
    use super::*;
    use mushd::accounts::Accounts;
    use mushd::ssh_bridge::start_listener;
    use russh::ChannelMsg;

    const STEP_TIMEOUT: Duration = Duration::from_secs(60);

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

    /// Drop ANSI CSI sequences so assertions match what a player reads.
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
        async fn expect(&mut self, needle: &str) {
            let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
            while !strip_ansi(&self.seen).contains(needle) {
                let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                    .await
                    .unwrap_or_else(|_| {
                        panic!("timeout waiting for {needle:?}; seen: {:?}", self.seen)
                    });
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        self.seen.push_str(&String::from_utf8_lossy(&data));
                    }
                    Some(_) => {}
                    None => panic!(
                        "channel closed waiting for {needle:?}; seen: {:?}",
                        self.seen
                    ),
                }
            }
        }

        async fn send_line(&mut self, line: &str) {
            let bytes = format!("{line}\r").into_bytes();
            self.channel.data(&bytes[..]).await.expect("send line");
        }

        async fn expect_close(&mut self) {
            let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
            loop {
                let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                    .await
                    .expect("timeout waiting for close");
                match msg {
                    None | Some(ChannelMsg::Close) => return,
                    Some(ChannelMsg::Data { data }) => {
                        self.seen.push_str(&String::from_utf8_lossy(&data));
                    }
                    Some(_) => {}
                }
            }
        }
    }

    /// `ssh play@addr` with a PTY + shell, landing on the welcome screen.
    async fn play_client(addr: std::net::SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn two_players_create_talk_who_quit_over_real_ssh() {
        let dir = scratch_dir("ssh-full-stack");
        std::fs::create_dir_all(&dir).unwrap();
        let sessions = Arc::new(SessionManager::new());
        let (world, _join) = world_task::spawn(world_task::boot_engine(), Arc::clone(&sessions));
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        let accounts = Arc::new(Accounts::new(store, world.clone(), 3));
        let bridge = Arc::new(SshBridge::new(
            accounts,
            Arc::clone(&sessions),
            world,
            Arc::new(IpGuard::new()),
        ));
        let addr = start_listener(
            "127.0.0.1:0".parse().unwrap(),
            &dir,
            "Welcome to the full-stack test MUSH!".to_string(),
            bridge,
        )
        .await
        .expect("listener")
        .addr();

        // Player one: welcome screen -> create -> auto-connect into Limbo.
        let (h1, mut nick) = play_client(addr).await;
        nick.expect("Welcome to the full-stack test MUSH!").await;
        nick.expect("connect <account> <password>").await;
        nick.send_line("create nick s3cretpw").await;
        nick.expect("Account created.").await;
        nick.expect("Connected as nick (account nick, 1 connection).")
            .await;
        nick.expect("Limbo").await;

        // Player two joins; both see the shared room.
        let (h2, mut ada) = play_client(addr).await;
        ada.send_line("create ada hunter42").await;
        ada.expect("Limbo").await;
        nick.expect("ada has connected.").await;

        // say fans out room-wide through world task + session manager
        // (speaker included; the ANSI renderer is third-person for all).
        nick.send_line("say hello ada").await;
        ada.expect("nick says, \"hello ada\"").await;
        nick.expect("nick says, \"hello ada\"").await;

        // WHO with real session facts lists both.
        ada.send_line("WHO").await;
        ada.expect("2 connected, 2 in your room.").await;
        assert!(strip_ansi(&ada.seen).contains("nick"), "{:?}", ada.seen);

        // QUIT: farewell + transport close; the other player hears it.
        nick.send_line("QUIT").await;
        nick.expect("Goodbye.").await;
        nick.expect_close().await;
        ada.expect("nick has disconnected.").await;

        let _ = h1
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
        let _ = h2
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
