//! M3 admin-tooling integration tests, driven bridge-level (no sockets:
//! fake connections are plain mpsc channels, exactly like `server.rs`):
//! `@boot` closing every target connection with the message delivered
//! first, `@wall` fan-out, the `@ignore` room filter at Audience::Room
//! resolution, and the pure `@ps` / `@stats` renderers.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    Effects, Engine, GOD, Inbound, LIMBO, ModError, ObjId, ObjKind, Outbox, Registrar, TickCadence,
    World,
};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use mush_store::InMemoryStore;
use mush_testkit::text_of;
use tokio::sync::mpsc;

use mushd::ipguard::IpGuard;
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{
    self, PersistHandles, ServerDeps, StoreHealth, WorldHandle, char_key, render_ps, render_stats,
};

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

/// A fresh engine with core + engine + server staff commands, sharing one
/// staff queue with the world task (the real boot wiring, minus the store).
fn staff_engine() -> (Engine, Arc<StaffQueue>) {
    let mut world = World::new_minimal();
    world.set_now(mushd::unix_now_s());
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    (Engine::new(world, registry), staff)
}

fn spawn_staffed(
    engine: Engine,
    staff: Arc<StaffQueue>,
    sessions: &Arc<SessionManager>,
) -> PersistHandles {
    world_task::spawn_persist(
        engine,
        Arc::clone(sessions),
        ServerDeps {
            store: Arc::new(InMemoryStore::new()),
            guard: Arc::new(IpGuard::new()),
            staff,
        },
    )
}

async fn attach_direct(
    sessions: &SessionManager,
    world: &WorldHandle,
    conn: u64,
    actor: ObjId,
    account: &str,
) -> EventRx {
    let (tx, rx) = mpsc::channel(256);
    let handle = ConnectionHandle::new(ConnId(conn), IpAddr::from([127, 0, 0, 9]), caps(), tx);
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
    rx
}

/// Receive until an event's text contains `needle`; returns every text seen
/// up to and including it (so callers can assert absences).
async fn collect_until(rx: &mut EventRx, needle: &str) -> Vec<String> {
    let deadline = Duration::from_secs(60);
    let fut = async {
        let mut seen = Vec::new();
        loop {
            match rx.recv().await {
                Some(ev) => {
                    let t = text_of(&ev);
                    let hit = t.contains(needle);
                    seen.push(t);
                    if hit {
                        return seen;
                    }
                }
                None => panic!("channel closed waiting for {needle:?}; saw {seen:?}"),
            }
        }
    };
    tokio::time::timeout(deadline, fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"))
}

/// Await the channel closing (all senders dropped), draining what remains.
async fn expect_closed(rx: &mut EventRx) -> Vec<String> {
    let deadline = Duration::from_secs(60);
    let fut = async {
        let mut seen = Vec::new();
        while let Some(ev) = rx.recv().await {
            seen.push(text_of(&ev));
        }
        seen
    };
    tokio::time::timeout(deadline, fut)
        .await
        .expect("timed out waiting for channel close")
}

#[tokio::test(flavor = "multi_thread")]
async fn boot_disconnects_every_connection_with_the_message() {
    let sessions = Arc::new(SessionManager::new());
    let (mut engine, staff) = staff_engine();
    let nick = engine
        .world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let handles = spawn_staffed(engine, staff, &sessions);
    let world = handles.handle;

    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    // Nick with TWO live connections (multi-connect).
    let mut nick_a = attach_direct(&sessions, &world, 2, nick, "nick").await;
    let mut nick_b = attach_direct(&sessions, &world, 3, nick, "nick").await;
    collect_until(&mut god_rx, "Nick has reconnected.").await;

    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@boot Nick=cool it".into(),
            })
            .await
    );

    // Both of Nick's connections see the message, then their channels
    // close (the session was force-closed and the handles dropped).
    for rx in [&mut nick_a, &mut nick_b] {
        collect_until(rx, "You have been booted by God: cool it").await;
        expect_closed(rx).await;
    }
    // The room disconnect announce lands, then the booter's confirmation.
    let seen = collect_until(&mut god_rx, "@boot: Nick booted (2 connections).").await;
    assert!(
        seen.iter().any(|t| t.contains("Nick has disconnected.")),
        "room must see the clean disconnect: {seen:?}"
    );
    assert!(
        seen.iter().all(|t| !t.contains("link-dead")),
        "@boot is a clean close, not link-dead: {seen:?}"
    );

    // World-side state: fully disconnected.
    assert!(world.barrier().await);
    assert!(sessions.session_of_char(char_key(nick)).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn wall_fans_out_to_every_connection() {
    let sessions = Arc::new(SessionManager::new());
    let (mut engine, staff) = staff_engine();
    let ann = engine
        .world
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    let handles = spawn_staffed(engine, staff, &sessions);
    let world = handles.handle;

    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    let mut ann_rx = attach_direct(&sessions, &world, 2, ann, "ann").await;

    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@wall Reboot at dawn.".into(),
            })
            .await
    );
    collect_until(&mut god_rx, "Announcement from God: Reboot at dawn.").await;
    collect_until(&mut ann_rx, "Announcement from God: Reboot at dawn.").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ignored_room_speech_is_filtered_per_listener() {
    let sessions = Arc::new(SessionManager::new());
    let (mut engine, staff) = staff_engine();
    let alice = engine
        .world
        .create(ObjKind::Player, "Alice", GOD, Some(LIMBO))
        .unwrap();
    let bob = engine
        .world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    let carol = engine
        .world
        .create(ObjKind::Player, "Carol", GOD, Some(LIMBO))
        .unwrap();
    let handles = spawn_staffed(engine, staff, &sessions);
    let world = handles.handle;

    let mut alice_rx = attach_direct(&sessions, &world, 1, alice, "alice").await;
    let mut carol_rx = attach_direct(&sessions, &world, 3, carol, "carol").await;
    let _bob_rx = attach_direct(&sessions, &world, 2, bob, "bob").await;

    // Alice mutes Bob.
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: alice,
                line: "@ignore Bob".into(),
            })
            .await
    );
    collect_until(&mut alice_rx, "You are now ignoring Bob.").await;

    // Bob speaks; Carol then speaks a marker line.
    for (conn, actor, line) in [(2, bob, "say sekrit plans"), (3, carol, "say marker line")] {
        assert!(
            world
                .inbound(Inbound::Cmd {
                    conn,
                    actor,
                    line: line.into(),
                })
                .await
        );
    }

    // Carol (no mutes) hears Bob, then herself.
    let carol_saw = collect_until(&mut carol_rx, "marker line").await;
    assert!(
        carol_saw.iter().any(|t| t.contains("sekrit plans")),
        "unmuted listener must hear Bob: {carol_saw:?}"
    );
    // Alice sees the marker but Bob's line was filtered from her fan-out.
    let alice_saw = collect_until(&mut alice_rx, "marker line").await;
    assert!(
        !alice_saw.iter().any(|t| t.contains("sekrit plans")),
        "muted speaker leaked into Alice's stream: {alice_saw:?}"
    );
}

// ---------------------------------------------------------------------
// Pure renderers: @ps and @stats
// ---------------------------------------------------------------------

/// A minimal module registering one coarse tick, so @ps has a module tick
/// to show.
struct PulseMod;

impl mush_core::GameModule for PulseMod {
    fn name(&self) -> &'static str {
        "pulse"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.tick(
            "heartbeat",
            TickCadence::Coarse(Duration::from_secs(60)),
            Arc::new(|_: &mut mush_core::Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {}),
        )
    }
}

#[test]
fn render_ps_shows_waits_ticks_and_counts() {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let mut registry = world_task::base_registry();
    let staff = Arc::new(StaffQueue::new());
    register_server_commands(&mut registry, &staff).unwrap();
    let mut engine = Engine::with_modules(world, registry, vec![Box::new(PulseMod)]).unwrap();

    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: "@wait 30=say tick tock goes the clock and the clock goes tick tock forever"
                .into(),
        },
        &mut out,
        &mut fx,
    );

    // now = 1_000s; the wait fires at 1_030_000ms, the tick at 1_060_000ms.
    let text = render_ps(&engine, 1_005_000);
    assert!(text.contains("Waits: 1"), "{text}");
    assert!(text.contains("God  in 25s: say tick tock"), "{text}");
    assert!(text.contains("..."), "long previews must truncate: {text}");
    assert!(
        !text.contains("tick tock forever"),
        "preview must be truncated: {text}"
    );
    assert!(text.contains("Module ticks: 1"), "{text}");
    assert!(
        text.contains("pulse:heartbeat  coarse 60s  next in 55s"),
        "{text}"
    );
    assert!(text.contains("Total scheduled entries: 2"), "{text}");

    // An empty scheduler renders zeros, not noise.
    let empty = Engine::new(World::new_minimal(), world_task::base_registry());
    let text = render_ps(&empty, 0);
    assert!(text.contains("Waits: 0"), "{text}");
    assert!(text.contains("Total scheduled entries: 0"), "{text}");
}

#[test]
fn render_stats_reports_counts_lag_and_store_health() {
    let mut world = World::new_minimal();
    world.set_now(2_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let _exit = world
        .create(ObjKind::Exit, "north", GOD, Some(hall))
        .unwrap();
    let _ball = world
        .create(ObjKind::Thing, "ball", GOD, Some(hall))
        .unwrap();
    world.set_connected(nick, true);
    let mut engine = Engine::new(world, world_task::base_registry());
    engine.record_loop_lag(12);
    engine.record_loop_lag(7);

    let now = 10_000i64;
    let health = StoreHealth {
        read_only: false,
        last_write_unix: Some(now - 5),
    };
    // Booted 1h 1m 1s ago.
    let text = render_stats(&engine, &health, now - 3_661, now);
    assert!(text.contains("Uptime: 1h 1m 1s"), "{text}");
    assert!(
        text.contains("Objects: 6 (2 rooms, 1 exits, 1 things, 2 players, 0 NPCs)"),
        "{text}"
    );
    assert!(text.contains("Connected: 1"), "{text}");
    assert!(text.contains("Loop lag: last 7ms, max 12ms"), "{text}");
    assert!(text.contains("Store: healthy; last write 5s ago"), "{text}");
    assert!(text.contains("Db size: 6 objects"), "{text}");

    // Read-only mode + no writes yet.
    let health = StoreHealth::new();
    let ro = StoreHealth {
        read_only: true,
        ..health
    };
    let text = render_stats(&engine, &ro, now, now);
    assert!(text.contains("READ-ONLY"), "{text}");
    assert!(text.contains("no writes yet this boot"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ps_and_stats_deliver_through_the_world_task() {
    let sessions = Arc::new(SessionManager::new());
    let (engine, staff) = staff_engine();
    let handles = spawn_staffed(engine, staff, &sessions);
    let world = handles.handle;
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@wait 60=say delayed".into(),
            })
            .await
    );
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@ps".into(),
            })
            .await
    );
    let seen = collect_until(&mut god_rx, "Waits: 1").await;
    assert!(
        seen.iter().any(|t| t.contains("say delayed")),
        "@ps must preview the queued wait: {seen:?}"
    );

    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@stats".into(),
            })
            .await
    );
    let seen = collect_until(&mut god_rx, "Server statistics:").await;
    // The ConnUp + commands above each produced a write batch.
    assert!(
        seen.iter()
            .any(|t| t.contains("Store: healthy; last write")),
        "@stats must report store health: {seen:?}"
    );
}
