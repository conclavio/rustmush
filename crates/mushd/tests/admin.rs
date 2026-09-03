//! M3 admin-tooling integration tests, driven bridge-level (no sockets:
//! fake connections are plain mpsc channels, exactly like `server.rs`):
//! `@boot` closing every target connection with the message delivered
//! first, `@wall` fan-out, the `@ignore` room filter at Audience::Room
//! resolution, and the pure `@ps` / `@stats` renderers.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionValidationContext, Effects,
    Engine, GOD, Inbound, IncidentId, LIMBO, MAX_PENDING_ENTRIES_PER_INSPECTION, ModError,
    ModuleBootReport, ObjId, ObjKind, Outbox, OwnerId, RecoveryReporter, Registrar, ScheduledKind,
    TickCadence, World,
};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use mush_store::InMemoryStore;
use mush_testkit::text_of;
use tokio::sync::mpsc;

use mushd::ipguard::IpGuard;
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{
    self, LlmStatsLine, PersistHandles, ServerDeps, StatsExtra, StoreHealth, WorldHandle,
    WorldMode, char_key, render_ps, render_stats,
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
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::new(InMemoryStore::new()),
            guard: Arc::new(IpGuard::new()),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
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
    assert!(
        world.barrier().await,
        "direct test attachment must cross its exact ConnUp barrier"
    );
    rx
}

/// Receive until an event's text contains `needle`; returns every text seen
/// up to and including it (so callers can assert absences).
async fn collect_until(rx: &mut EventRx, needle: &str) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    loop {
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            panic!("timed out waiting for {needle:?}; saw {seen:?}");
        };
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(ev)) => {
                let t = text_of(&ev);
                let hit = t.contains(needle);
                seen.push(t);
                if hit {
                    return seen;
                }
            }
            Ok(None) => panic!("channel closed waiting for {needle:?}; saw {seen:?}"),
            Err(_) => panic!("timed out waiting for {needle:?}; saw {seen:?}"),
        }
    }
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
        // These are distinct player actions. Cross the exact persistence
        // barrier before enqueueing the next one so this ordering assertion
        // does not rely on the pre-K0 deferred-command behavior.
        assert!(world.barrier().await);
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

struct StatsRecovery;

impl RecoveryReporter for StatsRecovery {
    fn report(&self, _world: &World) -> ModuleBootReport {
        let mut report = ModuleBootReport::new(OwnerId::new("spoofed").unwrap());
        report.active = 2;
        report.frozen = 1;
        report.incident_count = 1;
        report
    }
}

struct StatsMechanicsMod;

impl mush_core::GameModule for StatsMechanicsMod {
    fn name(&self) -> &'static str {
        "stats_mechanics"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("schema").unwrap(),
            |context: &DefinitionValidationContext<'_>| {
                let entry = context.definition();
                if entry.reference().key().id.as_str() == "sample"
                    && entry.reference().version() == 1
                    && entry.payload() == &serde_json::json!({"safe": true})
                    && entry.references().is_empty()
                    && entry.is_current()
                {
                    Ok(())
                } else {
                    Err("unexpected stats test definition".into())
                }
            },
        )?;
        reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new("sample").unwrap(),
                1,
                serde_json::json!({"safe": true}),
            )
            .unwrap()
            .as_current(),
        )?;
        reg.recovery_reporter(Arc::new(StatsRecovery))
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
fn render_ps_bounds_a_large_pending_set_and_reports_exact_omissions() {
    let mut engine = Engine::new(World::new_minimal(), world_task::base_registry());

    // Later entries are inserted first: the diagnostic must select by the
    // scheduler's `(due, seq)` firing order, not heap iteration or insertion
    // prefix, while retaining only its fixed display budget.
    for i in 0..300 {
        engine.sched.insert(
            20_000,
            ScheduledKind::WaitCmd {
                actor: GOD,
                line: format!("say late-{i:03}"),
            },
        );
    }
    for i in 0..300 {
        engine.sched.insert(
            10_000,
            ScheduledKind::WaitCmd {
                actor: GOD,
                line: format!("say early-{i:03}"),
            },
        );
    }

    let text = render_ps(&engine, 0);
    assert!(text.contains("Total scheduled entries: 600"), "{text}");
    assert!(
        text.contains(&format!(
            "Displayed earliest entries: {MAX_PENDING_ENTRIES_PER_INSPECTION} (344 omitted)"
        )),
        "{text}"
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("  God  in "))
            .count(),
        MAX_PENDING_ENTRIES_PER_INSPECTION,
        "only the bounded earliest set may be rendered"
    );
    assert!(text.contains("say early-000"), "{text}");
    assert!(text.contains("say early-255"), "{text}");
    assert!(!text.contains("say early-256"), "{text}");
    assert!(!text.contains("say late-"), "{text}");
    assert_eq!(engine.sched.len(), 600, "inspection must not dequeue work");
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
    let mut engine = Engine::with_modules(
        world,
        world_task::base_registry(),
        vec![Box::new(StatsMechanicsMod)],
    )
    .unwrap();
    engine.record_loop_lag(12);
    engine.record_loop_lag(7);

    let now = 10_000i64;
    let health = StoreHealth {
        mode: WorldMode::Writable,
        last_write_unix: Some(now - 5),
    };
    // The enriched aggregates (M9 observability): transport-split connections,
    // write-queue depth, NPC counts, p99 lag, and today's LLM spend.
    let extra = StatsExtra {
        ssh_conns: 3,
        web_conns: 1,
        write_queue_depth: 2,
        npc_active: 4,
        npc_thinking: 1,
        lag_p99: 25,
        ipc_frames_shed: 7,
        ipc_write_timeouts: 2,
        ipc_to_gw_overflow: 3,
        ipc_hw_to_server: 900,
        ipc_hw_to_gw: 4096,
        llm: Some(LlmStatsLine {
            globally_disabled: false,
            today_input: 1200,
            today_output: 340,
            today_cost: 0.0567,
            features: vec![("worldgen_room".to_string(), 1000, 200, 0.05)],
        }),
    };
    // Booted 1h 1m 1s ago.
    let text = render_stats(&engine, &health, now - 3_661, now, &extra);
    assert!(text.contains("Uptime: 1h 1m 1s"), "{text}");
    assert!(
        text.contains("Objects: 6 (2 rooms, 1 exits, 1 things, 2 players, 0 NPCs)"),
        "{text}"
    );
    // The classic connected count survives; the transport split is appended.
    assert!(text.contains("Connected: 1 (ssh 3, web 1)"), "{text}");
    assert!(
        text.contains("Loop lag: last 7ms, max 12ms, p99 25ms"),
        "{text}"
    );
    assert!(
        text.contains("Store: healthy; last write 5s ago; write-queue 2"),
        "{text}"
    );
    assert!(text.contains("NPC agents: 4 active, 1 thinking"), "{text}");
    assert!(
        text.contains(
            "Mechanics: 1 definitions from 1 owners, 0 bridges, 0 migrations; fingerprint "
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "Recovery stats_mechanics: 2 active, 1 frozen, 0 migrating, 0 quarantined; 1 incidents, 0 migration plans"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "IPC boundary: 7 frames shed, 2 write timeouts, 3 to_gw overflows; high-water to_server 900, to_gw 4096"
        ),
        "{text}"
    );
    assert!(
        text.contains("LLM today: 1200 in + 340 out tokens, $0.0567 (kill switch: off)"),
        "{text}"
    );
    assert!(
        text.contains("worldgen_room: 1000 in + 200 out tokens, $0.0500"),
        "{text}"
    );
    assert!(text.contains("Db size: 6 objects"), "{text}");

    // Read-only mode + no writes yet, no LLM wired (mortals never see this
    // form; the LLM block is simply absent).
    let health = StoreHealth::new();
    let ro = StoreHealth {
        mode: WorldMode::StoreReadOnly {
            incident: IncidentId(1),
        },
        ..health
    };
    let text = render_stats(&engine, &ro, now, now, &StatsExtra::default());
    assert!(text.contains("READ-ONLY"), "{text}");
    assert!(text.contains("no writes yet this boot"), "{text}");
    assert!(
        !text.contains("LLM today:"),
        "no LLM line without a service"
    );
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
    // @wait is intentionally silent, so its exact world/store barrier is the
    // acknowledgement that the scheduled mutation committed before @ps reads
    // the scheduler. A concurrent command is correctly Busy in CommitPending.
    assert!(world.barrier().await, "@wait must commit before @ps");
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
