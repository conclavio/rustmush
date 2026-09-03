//! Load / soak harness (M9 hardening).
//!
//! Four tests, two layers:
//!
//! * **Session layer (deterministic, in-process)** — the backpressure /
//!   slow-client and flood policies live in `mush-session`, so they are proven
//!   there with an injected clock and hand-built connections, free of socket
//!   timing:
//!   - [`fanout_backpressure_isolates_slow_and_bounds_memory`]: a genuinely
//!     slow connection (its receiver is never polled) overflows and is flagged
//!     dropped, while every well-behaved connection keeps receiving **every**
//!     event below its cap; the per-session resume ring stays bounded no matter
//!     how much is stamped through it (the memory bound).
//!   - [`flood_throttles_abuser_not_paced_client`]: an abuser hammering one
//!     connection is throttled after its burst while a paced connection on the
//!     same manager is never throttled (independent buckets).
//!
//! * **Socket layer (the real server on ephemeral ports)** — boots the full
//!   production wiring (world task, session layer, store-backed accounts, SSH
//!   listener + web ingress) with an in-memory store and drives it with real
//!   clients:
//!   - [`ssh_load_liveness_and_throughput`]: N concurrent russh clients
//!     connect, run a say/pose/look/move loop, then go link-dead and reconnect;
//!     the world loop is asserted to keep stepping (a WHO completes afterward)
//!     and rough throughput is logged.
//!   - [`slow_ws_client_is_detached_without_affecting_others`]: a WebSocket
//!     client that stops reading is detached by the driver's slow-client policy
//!     (it receives a Close frame) while a well-behaved WS client in the same
//!     room keeps working — end-to-end backpressure.
//!
//! The default run is fast (a handful of clients, seconds). `RUN_LOAD=1` scales
//! the socket load test up (more clients, more iterations) for a heavier soak.

mod support;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mush_core::{Engine, GOD};
use mush_net_web::WebConfig;
use mush_proto::{ClientCaps, ClientCommand, ClientKind, ColorDepth, Envelope, OutputEvent};
use mush_session::{
    CharKey, ConnId, ConnectionHandle, DetachReason, FloodVerdict, ReplayError, SessionConfig,
    SessionManager,
};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mush_testkit::text_of;

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::{IpGuard, IpGuardConfig};
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::web_bridge::WebBridge;
use mushd::world_task::{self, PersistHandles, ServerDeps};

use support::{STEP_TIMEOUT, Term, play_client, scratch_dir};

const GM_PASSWORD: &str = "gm-load-secret";
const BANNER: &str = "Welcome to the load MUSH!";
const COMMIT_PENDING_BUSY: &str =
    "The world is committing another action; please retry in a moment.";
const GATEWAY_QUEUE_BUSY: &str = "The server is busy; try again.";
const MAX_BUSY_ATTEMPTS: usize = 4_096;
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(2);

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

// ===========================================================================
// Session layer: deterministic backpressure / flood proofs
// ===========================================================================

fn ansi_caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi16,
        width: 80,
        height: 24,
        utf8: true,
        structured: false,
    }
}

fn localhost() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

/// A slow client (its receiver is never drained) must overflow and be flagged
/// dropped; well-behaved clients must receive every event below their cap; and
/// the per-session resume ring must stay bounded under a heavy stamp load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fanout_backpressure_isolates_slow_and_bounds_memory() {
    let sm = Arc::new(SessionManager::new());
    let n_fast = 16usize;
    let per_char = 200u64;

    // --- Fast connections: a buffer larger than the burst + a reader draining
    // it. "No dropped-below-cap delivery" means an event that fits the buffer is
    // never dropped; a buffer of `per_char + margin` makes that deterministic
    // regardless of when the async reader is scheduled (a smaller buffer could
    // legitimately fill mid-burst — that IS backpressure, tested on the slow
    // client below). ------------------------------------------------------
    let cap = per_char as usize + 64;
    let mut fast_chars = Vec::new();
    let mut fast_conns = Vec::new();
    let mut counters = Vec::new();
    let mut readers = Vec::new();
    for i in 0..n_fast {
        let ch = CharKey(1000 + i as u64);
        let conn_id = ConnId(1000 + i as u64);
        let (tx, mut rx) = mpsc::channel::<Arc<OutputEvent>>(cap);
        let conn = ConnectionHandle::new_at(conn_id, localhost(), ansi_caps(), tx, Instant::now());
        sm.attach(conn, ch, format!("fast{i}"));
        let cnt = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::clone(&cnt);
        readers.push(tokio::spawn(async move {
            while rx.recv().await.is_some() {
                c2.fetch_add(1, Ordering::Relaxed);
            }
        }));
        counters.push(cnt);
        fast_chars.push(ch);
        fast_conns.push(conn_id);
    }

    // --- Slow connection: capacity 1, never drained. ------------------------
    let slow_ch = CharKey(9999);
    let slow_conn_id = ConnId(9999);
    let (slow_tx, slow_rx) = mpsc::channel::<Arc<OutputEvent>>(1);
    let slow_conn = ConnectionHandle::new_at(
        slow_conn_id,
        localhost(),
        ansi_caps(),
        slow_tx,
        Instant::now(),
    );
    sm.attach(slow_conn, slow_ch, "slow".to_string());
    // Hold the receiver so the channel stays open but is never polled: this is
    // a client that has genuinely stopped reading.
    let _slow_rx = slow_rx;

    let mut slow_dropped = false;
    for i in 0..per_char {
        let report = sm.send_event_to_char(slow_ch, OutputEvent::Ping { nonce: i });
        if !report.dropped.is_empty() {
            assert_eq!(report.dropped, vec![slow_conn_id]);
            slow_dropped = true;
        }
    }
    assert!(
        slow_dropped,
        "a client that stopped reading must overflow and be reported dropped"
    );
    let slow_handle = sm.conn_handle(slow_conn_id).expect("slow conn present");
    assert!(
        slow_handle.output_dropped(),
        "the slow client's dropped flag must be set (driver would detach it)"
    );

    // --- Blast every fast char concurrently: not one drop below cap. --------
    let mut tasks = Vec::new();
    for &ch in &fast_chars {
        let sm2 = Arc::clone(&sm);
        tasks.push(tokio::spawn(async move {
            for i in 0..per_char {
                let report = sm2.send_event_to_char(ch, OutputEvent::Ping { nonce: i });
                assert_eq!(report.delivered, 1, "fast conn must deliver every event");
                assert!(
                    report.dropped.is_empty(),
                    "no dropped-below-cap delivery to a well-behaved client"
                );
            }
        }));
    }
    for t in tasks {
        t.await.expect("fast blast task");
    }

    // Isolation: the saturated slow client did not degrade a fast one.
    let report = sm.send_event_to_char(fast_chars[0], OutputEvent::Ping { nonce: u64::MAX });
    assert_eq!(report.delivered, 1);
    assert!(report.dropped.is_empty());

    // Detach the fast conns (drops their senders) so the readers finish, then
    // confirm each well-behaved reader received EVERY event (per_char, plus the
    // one isolation event for char 0). This is the "no drops below cap" proof
    // observed from the receiving end.
    for &conn_id in &fast_conns {
        sm.detach(conn_id, DetachReason::Quit);
    }
    for (i, reader) in readers.into_iter().enumerate() {
        reader.await.expect("reader task");
        let got = counters[i].load(Ordering::Relaxed) as u64;
        let want = if i == 0 { per_char + 1 } else { per_char };
        assert_eq!(got, want, "well-behaved conn {i} lost events under load");
    }

    // --- Memory bound: the resume ring never grows without limit. -----------
    let ring_cfg = SessionConfig::default();
    let ring_ch = CharKey(7777);
    let (ring_tx, _ring_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
    let ring_conn = ConnectionHandle::new_at(
        ConnId(7777),
        localhost(),
        ansi_caps(),
        ring_tx,
        Instant::now(),
    );
    let (ring_sess, _) = sm.attach(ring_conn, ring_ch, "ring".to_string());
    let total = ring_cfg.resume_ring as u64 + 100; // stamp well past the ring
    for i in 0..total {
        sm.send_event_to_char(ring_ch, OutputEvent::Ping { nonce: i });
    }
    assert_eq!(
        sm.last_seq(ring_sess),
        Some(total),
        "every event was stamped"
    );
    // Old events were evicted (the ring is bounded), so replaying from 0 fails.
    assert_eq!(
        sm.replay_since(ring_sess, 0),
        Err(ReplayError::TooOld),
        "an unbounded ring would have replayed from 0; it must evict instead"
    );
    // The newest events are still there: a recent resume replays exactly.
    let recent = sm
        .replay_since(ring_sess, total - 10)
        .expect("recent replay");
    assert_eq!(recent.len(), 10, "the ring retains its most recent window");
}

/// An abuser is throttled after its burst; a paced client on the same manager
/// is never throttled (per-connection buckets are independent).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flood_throttles_abuser_not_paced_client() {
    let sm = SessionManager::with_config(SessionConfig {
        flood_rate: 10.0,
        flood_burst: 30.0,
        ..SessionConfig::default()
    });

    let abuser = ConnId(1);
    let (tx1, _rx1) = mpsc::channel::<Arc<OutputEvent>>(16);
    sm.attach(
        ConnectionHandle::new_at(abuser, localhost(), ansi_caps(), tx1, Instant::now()),
        CharKey(1),
        "abuser".to_string(),
    );
    let paced = ConnId(2);
    let (tx2, _rx2) = mpsc::channel::<Arc<OutputEvent>>(16);
    sm.attach(
        ConnectionHandle::new_at(paced, localhost(), ansi_caps(), tx2, Instant::now()),
        CharKey(2),
        "paced".to_string(),
    );

    let t0 = Instant::now();

    // Abuser dumps 100 lines in one instant: 30 pass (the burst), the rest are
    // throttled.
    let mut ok = 0;
    let mut throttled = 0;
    for _ in 0..100 {
        match sm.check_input_at(abuser, t0) {
            FloodVerdict::Ok => ok += 1,
            FloodVerdict::Throttled => throttled += 1,
        }
    }
    assert_eq!(ok, 30, "burst allowance");
    assert_eq!(throttled, 70, "everything past the burst is throttled");

    // Paced client: drain its own burst at t0, then one line every 100 ms — the
    // sustained rate. Never throttled, and wholly unaffected by the abuser.
    for _ in 0..30 {
        assert_eq!(sm.check_input_at(paced, t0), FloodVerdict::Ok);
    }
    for i in 1..=50u64 {
        let t = t0 + Duration::from_millis(100 * i);
        assert_eq!(
            sm.check_input_at(paced, t),
            FloodVerdict::Ok,
            "well-behaved paced line {i} must pass"
        );
    }
    // And the abuser is still throttled at t0 while the paced client sails on.
    assert_eq!(sm.check_input_at(abuser, t0), FloodVerdict::Throttled);
}

// ===========================================================================
// Socket layer: the full server on ephemeral ports
// ===========================================================================

struct Rig {
    ssh: SocketAddr,
    web: SocketAddr,
    dir: PathBuf,
    _ssh_listener: SshListener,
    _web_task: tokio::task::JoinHandle<()>,
    _handles: PersistHandles,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Rig {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.web)
    }
}

/// Boot the full production wiring (SSH + web) on ephemeral ports with an
/// in-memory store and a boss/God account seeded. `send_queue` sizes each web
/// connection's output buffer and `send_timeout_ms` bounds one blocked WS send
/// (small values = a stalled client overflows cheaply); flood limits are set
/// generous here so the throughput test is not fighting the rate limiter (which
/// `flood_throttles_abuser_not_paced_client` covers).
async fn boot_load(tag: &str, send_queue: usize, send_timeout_ms: u64) -> Rig {
    let dir = scratch_dir(tag);
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
    let world = load_or_seed_world(&*store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, modules::server_modules()).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");

    let mut panels: Vec<String> = engine
        .registry
        .panel_feeds()
        .iter()
        .map(|(f, _)| f.to_string())
        .collect();
    panels.push("who".to_string());
    let features: Vec<String> = engine
        .modules()
        .iter()
        .map(|m| match m.name() {
            "comsys" => "channels".to_string(),
            other => other.to_string(),
        })
        .collect();

    // Generous flood limits: the throughput test fires bursts on purpose.
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        flood_rate: 10_000.0,
        flood_burst: 100_000.0,
        ..SessionConfig::default()
    }));
    // Every load client shares 127.0.0.1, so lift the per-IP concurrent-
    // connection cap (default 8) well above the client count; real load arrives
    // from many IPs. The cap itself is exercised by the ipguard unit tests.
    let guard = Arc::new(IpGuard::with_config(IpGuardConfig {
        max_conns: 4096,
        ..IpGuardConfig::default()
    }));
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            metrics: Arc::new(mush_net_web::Metrics::new()),
        },
    );
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 3));

    let ssh_bridge = Arc::new(SshBridge::new(
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let ssh_listener = start_listener(
        "127.0.0.1:0".parse().unwrap(),
        &dir,
        BANNER.to_string(),
        ssh_bridge,
    )
    .await
    .expect("ssh listener");
    let ssh = ssh_listener.addr();

    let web_bridge: Arc<dyn mush_net_web::WebHooks> = Arc::new(WebBridge::new(
        accounts,
        sessions,
        handles.handle.clone(),
        guard,
        store,
        panels.clone(),
    ));
    let web_config = WebConfig {
        server_name: "rustmush load".to_string(),
        features,
        panels,
        auth_burst: 0,
        send_queue,
        send_timeout_ms,
        ..WebConfig::default()
    };
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let web = listener.local_addr().unwrap();
    let app = mush_net_web::router(web_bridge, web_config);
    let web_task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    Rig {
        ssh,
        web,
        dir,
        _ssh_listener: ssh_listener,
        _web_task: web_task,
        _handles: handles,
    }
}

/// One SSH player's whole journey: create, a say/pose/look/move loop, then a
/// link-dead drop + reconnect + final say. Returns the number of commands it
/// issued (for throughput accounting).
async fn send_with_busy_retry(term: &mut Term, line: &str, acknowledgement: &str) {
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    for attempt in 1..=MAX_BUSY_ATTEMPTS {
        let mark = term.mark();
        term.send_line(line).await;
        let outcome = tokio::time::timeout_at(
            deadline,
            term.expect_one_of_after(
                mark,
                &[acknowledgement, COMMIT_PENDING_BUSY, GATEWAY_QUEUE_BUSY],
            ),
        )
        .await
        .unwrap_or_else(|_| {
            panic!(
                "command {line:?} did not settle within {STEP_TIMEOUT:?} after {attempt} attempts"
            )
        });
        if outcome == 0 {
            return;
        }
        assert!(
            attempt < MAX_BUSY_ATTEMPTS,
            "command {line:?} remained busy after {MAX_BUSY_ATTEMPTS} attempts"
        );
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "command {line:?} remained busy until the {STEP_TIMEOUT:?} deadline"
        );
        tokio::time::sleep(BUSY_RETRY_DELAY.min(remaining)).await;
    }
    unreachable!("bounded retry loop either succeeds or panics");
}

async fn wait_for_phase(barrier: &tokio::sync::Barrier, phase: &str) {
    tokio::time::timeout(STEP_TIMEOUT, barrier.wait())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for all clients at the {phase} phase"));
}

async fn run_ssh_client(
    addr: SocketAddr,
    name: String,
    iters: usize,
    connection_gate: Arc<tokio::sync::Semaphore>,
    phase_barrier: Arc<tokio::sync::Barrier>,
) -> usize {
    let pw = "player-pw-123";
    let (handle, mut term) = play_client(addr).await;
    {
        let _permit = connection_gate
            .acquire()
            .await
            .expect("connection gate remains open");
        let mark = term.mark();
        term.send_line(&format!("create {name} {pw}")).await;
        term.expect_after(mark, &format!("Connected as {name}"))
            .await;
        term.expect_after(mark, "Limbo(#0)").await;
    }
    let mut cmds = 1usize;

    // Finish all serialized account/character setup before beginning the
    // concurrent command phase. The gate must not hide world contention from
    // the actual load loop below.
    wait_for_phase(&phase_barrier, "load start").await;

    // Each command is drained before the next: like a real terminal, this
    // client reads continuously, so heavy room-wide cross-talk never piles up
    // in its own output buffer (a client that fired a burst and then stopped
    // reading is exactly the slow client the backpressure test covers). Every
    // command echoes something to the speaker, giving a per-command sync point
    // that is also a proof the world loop processed THIS client's input. A
    // CommitPending or gateway-queue refusal is an explicit retry signal:
    // back off, then resend this same command. Clients remain concurrent with
    // one another while each individual client has at most one command in
    // flight.
    for i in 0..iters {
        send_with_busy_retry(&mut term, "look", "Limbo(#0)").await;
        let stretch = format!("stretches-{name}-{i}");
        let pose = format!("pose {stretch}");
        send_with_busy_retry(&mut term, &pose, &stretch).await;
        let tick = format!("tick-{name}-{i}");
        send_with_busy_retry(&mut term, &format!("say {tick}"), &tick).await;
        send_with_busy_retry(&mut term, "east", "Loop(#").await;
        send_with_busy_retry(&mut term, "west", "Limbo(#0)").await;
        cmds += 5;
    }

    // Link-dead drop, then reconnect within the grace and speak again.
    wait_for_phase(&phase_barrier, "reconnect start").await;
    let (handle2, mut term2) = {
        let _permit = connection_gate
            .acquire()
            .await
            .expect("connection gate remains open");
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
        drop(term);
        drop(handle);

        let (handle2, mut term2) = play_client(addr).await;
        let mark = term2.mark();
        term2.send_line(&format!("connect {name} {pw}")).await;
        term2
            .expect_after(mark, &format!("Connected as {name}"))
            .await;
        term2.expect_after(mark, "Limbo(#0)").await;
        (handle2, term2)
    };
    wait_for_phase(&phase_barrier, "final command").await;
    send_with_busy_retry(
        &mut term2,
        &format!("say back-{name}"),
        &format!("back-{name}"),
    )
    .await;
    cmds += 2;
    let _ = handle2
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    cmds
}

/// N concurrent russh clients hammer the server through a full command loop;
/// afterward the world loop must still be stepping (a fresh WHO completes), and
/// rough throughput is logged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ssh_load_liveness_and_throughput() {
    let heavy = std::env::var("RUN_LOAD").is_ok();
    let n_clients = if heavy { 32 } else { 6 };
    let iters = if heavy { 24 } else { 8 };

    let rig = boot_load("ssh-load", 256, 10_000).await;

    // Boss digs a Limbo <-> Loop pair so every client's move loop has exits,
    // then disconnects: a connection sitting in the busy room without reading
    // would itself overflow and be slow-client-detached under the cross-talk
    // (that path is proven deliberately in the WS test) — here it would just be
    // noise. The load clients each drain continuously, so they never overflow.
    {
        let (boss_handle, mut boss) = play_client(rig.ssh).await;
        boss.expect(BANNER).await;
        boss.send_line(&format!("connect boss {GM_PASSWORD}")).await;
        boss.expect("Connected as God").await;
        boss.expect("Limbo(#0)").await;
        boss.send_line("@dig Loop = east;e, west;w").await;
        boss.expect("Opened exit east(").await;
        boss.expect("Opened exit west(").await;
        let _ = boss_handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
    }
    assert!(
        rig._handles.handle.barrier().await,
        "boss disconnect must settle before load starts"
    );

    let started = Instant::now();
    let mut tasks = Vec::new();
    let connection_gate = Arc::new(tokio::sync::Semaphore::new(1));
    let phase_barrier = Arc::new(tokio::sync::Barrier::new(n_clients));
    for i in 0..n_clients {
        let addr = rig.ssh;
        let connection_gate = Arc::clone(&connection_gate);
        let phase_barrier = Arc::clone(&phase_barrier);
        tasks.push(tokio::spawn(run_ssh_client(
            addr,
            format!("Load{i}"),
            iters,
            connection_gate,
            phase_barrier,
        )));
    }
    let mut total_cmds = 0usize;
    for t in tasks {
        total_cmds += t.await.expect("client task did not panic");
    }
    let elapsed = started.elapsed();
    let rate = total_cmds as f64 / elapsed.as_secs_f64();
    eprintln!(
        "[load] ssh: {n_clients} clients x {iters} iters = {total_cmds} commands in {:.2}s \
         (~{rate:.0} cmd/s)",
        elapsed.as_secs_f64()
    );

    // Liveness: the world loop kept stepping — a fresh connection + WHO
    // round-trips promptly. Completing within STEP_TIMEOUT is the "no deadlock /
    // no stall" assertion (a wedged world loop would never answer).
    let (checker_handle, mut checker) = play_client(rig.ssh).await;
    checker.expect(BANNER).await;
    checker
        .send_line(&format!("connect boss {GM_PASSWORD}"))
        .await;
    checker.expect("Connected as God").await;
    checker.expect("Limbo(#0)").await;
    let live_started = Instant::now();
    checker.send_line("WHO").await;
    checker.expect("players connected.").await;
    let latency = live_started.elapsed();
    eprintln!(
        "[load] post-load WHO latency: {:.1}ms",
        latency.as_secs_f64() * 1000.0
    );
    assert!(
        latency < STEP_TIMEOUT,
        "world loop stalled: WHO did not complete promptly"
    );

    let _ = checker_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

// ---- WebSocket helpers (the SPA's connect path) ----------------------------

async fn register(rig: &Rig, account: &str, password: &str, char_name: &str) -> (String, u64) {
    let resp = reqwest::Client::new()
        .post(rig.url("/api/auth/register"))
        .json(&serde_json::json!({
            "account": account, "password": password, "char_name": char_name
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "register must succeed");
    let body: serde_json::Value = resp.json().await.unwrap();
    (
        body["token"].as_str().unwrap().to_string(),
        body["characters"][0]["key"].as_u64().unwrap(),
    )
}

async fn connect_ws(rig: &Rig, token: &str, char_key: u64) -> Result<Ws, WsError> {
    let mut req = format!("ws://{}/ws?char={char_key}", rig.web)
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rustmush.v1.{token}")).unwrap(),
    );
    connect_async(req).await.map(|(ws, _resp)| ws)
}

async fn recv_env(ws: &mut Ws) -> Envelope {
    loop {
        let msg = tokio::time::timeout(STEP_TIMEOUT, ws.next())
            .await
            .expect("timed out waiting for a frame")
            .expect("stream ended")
            .expect("ws error");
        if let Message::Text(text) = msg {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

async fn expect_env(ws: &mut Ws, needle: &str) {
    let mut seen = 0;
    loop {
        let env = recv_env(ws).await;
        if text_of(&env.event).contains(needle) {
            return;
        }
        seen += 1;
        assert!(
            seen < 500,
            "no envelope containing {needle:?} after {seen} frames"
        );
    }
}

async fn send_ws_line(ws: &mut Ws, line: &str) {
    let cmd = ClientCommand::Line(line.to_string());
    ws.send(Message::Text(serde_json::to_string(&cmd).unwrap().into()))
        .await
        .unwrap();
}

/// A WS client that stops reading is detached by the driver's slow-client
/// policy (Close frame) while a well-behaved WS client in the same room is
/// unaffected — the end-to-end backpressure proof.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_ws_client_is_detached_without_affecting_others() {
    // Small per-connection queue AND a short send-timeout: once a stalled
    // client's OS/TCP buffers fill, the very next server-side send blocks past
    // this timeout and the pump closes it as SLOW. Large say bodies fill those
    // buffers in a handful of frames, so the trip is fast and reliable.
    let rig = boot_load("slow-ws", 16, 200).await;

    let (good_tok, good_key) = register(&rig, "GoodWeb", "pw-good-123", "GoodWeb").await;
    let (slow_tok, slow_key) = register(&rig, "SlowWeb", "pw-slow-123", "SlowWeb").await;

    // Good client: connect and drain (it stays healthy throughout).
    let mut good = connect_ws(&rig, &good_tok, good_key)
        .await
        .expect("good ws");
    // Slow client: connect, then STOP reading (only the initial hello is read).
    let mut slow = connect_ws(&rig, &slow_tok, slow_key)
        .await
        .expect("slow ws");
    let _ = recv_env(&mut slow).await; // the hello, seq 0 — then never read again.

    // The good client floods the shared room (Limbo) with big says. Each fans
    // out to the slow client too; ~6 KB/frame overruns the socket buffers and
    // the 16-slot queue within a few dozen frames, tripping the slow-client
    // detach. The good client keeps draining its own echoes and stays healthy.
    let body = "z".repeat(6000);
    for i in 0..200 {
        send_ws_line(&mut good, &format!("say M{i}Z {body}")).await;
        // Drain the good client so IT never becomes a slow client.
        expect_env(&mut good, &format!("M{i}Z")).await;
    }

    // Good client is still fully functional (isolation): a fresh say round-trips
    // while the slow client is being (or has been) torn down.
    send_ws_line(&mut good, "say still-here").await;
    expect_env(&mut good, "still-here").await;

    // Give the server's send-timeout a moment to fire and close the slow client
    // before we start reading it (reading would otherwise un-stall it).
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    // The slow client must have been detached: draining its backlog we reach a
    // Close frame / end-of-stream / reset within the timeout (never a hang).
    let closed = tokio::time::timeout(STEP_TIMEOUT, async {
        loop {
            match slow.next().await {
                Some(Ok(Message::Close(_))) | None => return true,
                Some(Ok(_)) => {}            // buffered frames before the close
                Some(Err(_)) => return true, // reset counts as detached
            }
        }
    })
    .await
    .expect("slow client was never detached (backpressure policy failed to fire)");
    assert!(
        closed,
        "slow WS client should have been closed by the driver"
    );

    // And the good client is STILL usable after the slow one is gone.
    send_ws_line(&mut good, "say after-slow-gone").await;
    expect_env(&mut good, "after-slow-gone").await;
    let _ = good.close(None).await;
}
