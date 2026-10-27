//! WS01-S8: the boundary reader notices a dead world instead of lying about it.
//!
//! The forked server's world task is single-writer and never restarts in
//! place. Once its receiver is gone every later `inbound` is refused forever —
//! so a reader that keeps looping is a server that accepts a gateway, accepts
//! frames, and drops every one of them into a world that will never see them.
//! Worse, the one correlated answer it did send said `Rejected { retryable:
//! true }`, which invites the gateway to replay a request that can never land.
//!
//! Every proof here drives the **real** [`reader_loop`] against a **real**
//! world task that has really stopped: the rig boots the world through
//! [`spawn_persist_with_gateway`], ends it, and joins it before a single frame
//! is fed in. Nothing simulates a closed channel.
//!
//! The last proof closes the other half of the same failure — `serve` awaiting
//! the reader to completion *before* it ever looked at the world's join handle,
//! so a world that died under a still-connected gateway left the process up
//! with its IPC listener bound and nothing for the supervisor to heal.
//!
//! [`reader_loop`]: mush_server::reader_loop
//! [`spawn_persist_with_gateway`]: mush_server::world_task::spawn_persist_with_gateway

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_config::config::Config;
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{
    ArgShape, CmdError, CommandSpec, ControlMsg, Ctx, Engine, GOD, GameModule, Inbound, LIMBO,
    ModError, ObjKind, OfferRouteTable, Registrar,
};
use mush_ipc::{
    DetachCause, FrameReader, GatewayInput, GatewayRequest, GatewayRequestDisposition, RequestUid,
    SendTarget, ToGateway, ToServer, WireInbound, write_frame,
};
use mush_proto::OutputEvent;
use mush_server::gateway_sink::{GatewaySink, SocketSink};
use mush_server::persist::WorldExitStatus;
use mush_server::quarantine::QuarantineSpool;
use mush_server::staff::StaffQueue;
use mush_server::world_task::{
    self, Gateway, PersistHandles, ServerDeps, WorldHandle, WorldOutcome,
    spawn_persist_with_gateway,
};
use mush_store::{InMemoryStore, ObjectStore};
use tokio::sync::{Notify, mpsc, oneshot};

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

/// The world loop needs a sink; nothing here inspects delivery.
struct BlackholeGateway;

impl GatewaySink for BlackholeGateway {
    fn transmit(&self, _sends: Vec<(SendTarget, Arc<OutputEvent>)>) {}

    fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<OutputEvent>,
        _cause: DetachCause,
    ) {
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

/// One command that mutates and then panics, so a proof can end the world
/// through the dispatch firewall rather than through its own front door.
struct DoomModule;

impl GameModule for DoomModule {
    fn name(&self) -> &'static str {
        "dead_world_probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "doom".into(),
                shape: ArgShape::None,
                help: "test-only mutate-then-panic".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.w
                    .create(ObjKind::Thing, "doomed-orb", ctx.actor, Some(LIMBO))
                    .map_err(|error| CmdError::msg(error.to_string()))?;
                panic!("doom: the world task unwinds with the mutation still in RAM");
            }),
        )?;
        Ok(())
    }
}

fn boot() -> PersistHandles {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let engine = Engine::with_modules(
        mush_core::World::new_minimal(),
        world_task::base_registry(),
        vec![Box::new(DoomModule)],
    )
    .expect("doom module composes");
    spawn_persist_with_gateway(
        engine,
        ServerDeps {
            store,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        |_| -> Gateway { Arc::new(BlackholeGateway) },
    )
}

/// How the world under test was killed before the reader ever saw a frame.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Death {
    /// An operator shutdown: the loop published a clean exit on its way out.
    Shutdown,
    /// A caught panic: the loop broke through the firewall, so nothing on the
    /// reader's side can call this an ordinary exit.
    Fault,
}

/// A world handle whose task has already ended and been JOINED — the receiver
/// is genuinely dropped, so every `inbound` from here on is refused.
async fn dead_world(death: Death) -> (WorldHandle, WorldExitStatus) {
    let handles = boot();
    let world_exit = handles.world_exit.clone();
    let handle = handles.handle.clone();
    match death {
        Death::Shutdown => {
            assert!(
                handles
                    .handle
                    .inbound(Inbound::Control(ControlMsg::Shutdown))
                    .await,
                "the live world accepts the shutdown control"
            );
            let joined = handles.world_join.await.expect("world task shut down");
            assert_eq!(joined, WorldOutcome::Clean, "an operator shutdown is clean");
            assert!(world_exit.is_clean(), "and publishes its clean exit");
        }
        Death::Fault => {
            assert!(
                handles
                    .handle
                    .inbound(Inbound::Cmd {
                        conn: 1,
                        actor: GOD,
                        line: "doom".to_string(),
                    })
                    .await,
                "the live world accepts the doomed command"
            );
            let joined = handles.world_join.await.expect("the firewall caught it");
            assert!(
                matches!(joined, WorldOutcome::Faulted(_)),
                "the handler must really panic the world task: {joined:?}"
            );
            assert!(
                !world_exit.is_clean(),
                "a faulted world never publishes a clean exit"
            );
        }
    }
    // `handles` (and with it the last live sender besides `handle`) is dropped
    // here; the task itself is already joined, so its receiver is long gone.
    drop(handles.handle);
    assert!(
        !handle
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "look".to_string(),
            })
            .await,
        "the world really is unreachable before the reader is driven"
    );
    (handle, world_exit)
}

/// Serialize `frames` into one buffer the reader can consume as its socket.
async fn wire(frames: &[ToServer]) -> mush_ipc::BoxRead {
    let mut bytes: Vec<u8> = Vec::new();
    for frame in frames {
        write_frame(&mut bytes, frame).await.expect("frame encodes");
    }
    Box::new(std::io::Cursor::new(bytes))
}

/// Everything the reader answers on, plus the answers it gave.
struct Answers {
    rx: mpsc::Receiver<ToGateway>,
}

impl Answers {
    fn dispositions(&mut self) -> Vec<(RequestUid, GatewayRequestDisposition)> {
        let mut seen = Vec::new();
        while let Ok(frame) = self.rx.try_recv() {
            if let ToGateway::RequestDisposition {
                request,
                disposition,
            } = frame
            {
                seen.push((request, disposition));
            }
        }
        seen
    }
}

/// Drive the real reader over `frames` against an already-dead world.
async fn drive(death: Death, frames: &[ToServer]) -> (std::io::Result<()>, Answers) {
    let (world, world_exit) = dead_world(death).await;
    let (tx, rx) = mpsc::channel::<ToGateway>(64);
    let metrics = Arc::new(Metrics::new());
    let sink = SocketSink::new(tx, Arc::clone(&metrics), Arc::new(Notify::new()), 64, 90);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let result = mush_server::reader_loop(
        wire(frames).await,
        world,
        store,
        Arc::new(OfferRouteTable::default()),
        sink,
        &metrics,
        world_exit,
    )
    .await;
    (result, Answers { rx })
}

fn query(epoch: u64, sequence: u64) -> GatewayRequest {
    GatewayRequest::new(
        epoch,
        sequence,
        GatewayInput::Query {
            conn: 1,
            actor: GOD,
            id: sequence,
            name: "who".into(),
            args: serde_json::json!({}),
        },
    )
}

fn assert_world_is_gone(result: std::io::Result<()>) {
    let error = result.expect_err(
        "a reader that cannot reach the world must END the boundary; running on to EOF is the \
         bug — the gateway keeps feeding a server that answers nothing",
    );
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe,
        "the closed world is a broken pipe, not a protocol fault: {error}"
    );
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// The lie this step exists to stop. A query the world can never see was
/// answered `Rejected { retryable: true }` — an invitation to replay into a
/// world that is gone for good — and the reader then went back for the next
/// frame. It must answer `retryable: false` and end the boundary.
#[tokio::test(flavor = "multi_thread")]
async fn a_query_against_a_dead_world_is_permanently_rejected_and_ends_the_boundary() {
    let first = query(7, 1);
    let second = query(7, 2);
    let (result, mut answers) = drive(
        Death::Shutdown,
        &[
            ToServer::Input(first.clone()),
            ToServer::Input(second.clone()),
        ],
    )
    .await;

    assert_world_is_gone(result);
    assert_eq!(
        answers.dispositions(),
        vec![(
            first.uid,
            GatewayRequestDisposition::Rejected { retryable: false }
        )],
        "exactly one correlated answer, and it must not be retryable; a second answer would \
         mean the reader looped on into the next frame"
    );
    assert_ne!(first.uid, second.uid, "the two frames are distinguishable");
}

/// A faulted world is just as gone as a shut-down one. Provenance decides the
/// log severity, never whether the boundary ends.
#[tokio::test(flavor = "multi_thread")]
async fn a_faulted_world_ends_the_boundary_the_same_way() {
    let request = query(9, 1);
    let (result, mut answers) = drive(Death::Fault, &[ToServer::Input(request.clone())]).await;

    assert_world_is_gone(result);
    assert_eq!(
        answers.dispositions(),
        vec![(
            request.uid,
            GatewayRequestDisposition::Rejected { retryable: false }
        )],
    );
}

/// The accept bool the `ToServer::Inbound` arm used to discard outright. A
/// lifecycle frame the world never received is not a frame that was delivered.
#[tokio::test(flavor = "multi_thread")]
async fn a_discarded_wire_inbound_ends_the_boundary() {
    let follow_on = query(7, 5);
    let (result, mut answers) = drive(
        Death::Shutdown,
        &[
            ToServer::Inbound(WireInbound::ConnUp {
                conn: 1,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            }),
            ToServer::Input(follow_on),
        ],
    )
    .await;

    assert_world_is_gone(result);
    assert!(
        answers.dispositions().is_empty(),
        "the reader stopped at the undeliverable ConnUp; it never reached the query behind it"
    );
}

/// The other discarded accept bool: operator control. A shutdown that finds the
/// world already gone is an ordinary exit, but it is still an exit.
#[tokio::test(flavor = "multi_thread")]
async fn a_discarded_control_frame_ends_the_boundary() {
    let (result, mut answers) = drive(
        Death::Shutdown,
        &[
            ToServer::Control(ControlMsg::Shutdown),
            ToServer::Input(query(7, 6)),
        ],
    )
    .await;

    assert_world_is_gone(result);
    assert!(
        answers.dispositions().is_empty(),
        "the reader stopped at the undeliverable control frame"
    );
}

/// A released legacy command cannot be executed by a world that is gone, and
/// the reader must not go on pretending it can execute the next one either.
#[tokio::test(flavor = "multi_thread")]
async fn a_released_legacy_command_against_a_dead_world_ends_the_boundary() {
    let cmd = GatewayRequest::new(
        7,
        1,
        GatewayInput::Cmd {
            conn: 1,
            actor: GOD,
            line: "look".to_string(),
        },
    );
    let (result, mut answers) = drive(
        Death::Shutdown,
        &[
            ToServer::Input(cmd.clone()),
            ToServer::GatewayReleased { request: cmd.uid },
            ToServer::Input(query(7, 7)),
        ],
    )
    .await;

    assert_world_is_gone(result);
    assert_eq!(
        answers.dispositions(),
        vec![(cmd.uid, GatewayRequestDisposition::LegacyClaimed)],
        "the claim is answered before the release; the query behind it is never reached"
    );
}

// ---------------------------------------------------------------------------
// The other half: `serve` must stop being a server
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-deadworld-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg
}

/// The composition bug. `serve` awaited the reader to completion and only THEN
/// looked at the world's join handle, so a world that ended under a live,
/// well-behaved gateway left this process running forever: listener bound,
/// frames accepted, every one of them dropped. The fix races the two, so the
/// process ends when either half does — which is the whole mechanism by which
/// the production supervisor heals the forked half (D2).
///
/// Here the gateway is deliberately POLITE: it connects, greets, and then just
/// holds the boundary open. Nothing but the world's own exit can end `serve`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_returns_when_the_world_ends_under_a_live_gateway() {
    let (ipc_tx, ipc_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();
    let cfg = test_config("serve");
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = rt.block_on(mush_server::serve_with_store(cfg, Some(ipc_tx), None));
        let _ = exit_tx.send(outcome);
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(60), ipc_rx)
        .await
        .expect("server bound its IPC socket")
        .expect("ipc endpoint");

    let (reader, mut writer) = mush_ipc::connect(&endpoint)
        .await
        .expect("connect boundary");
    let mut frames = FrameReader::new(reader);
    let hello = tokio::time::timeout(Duration::from_secs(30), frames.read::<ToGateway>())
        .await
        .expect("hello within timeout")
        .expect("hello read ok")
        .expect("a Hello frame");
    assert!(
        matches!(hello, ToGateway::Hello { .. }),
        "first frame is Hello"
    );

    // Kill the world, then keep the boundary open and read nothing more. The
    // reader half of `serve` is parked on this socket; only the world's join
    // handle can end the process now.
    write_frame(&mut writer, &ToServer::Control(ControlMsg::Shutdown))
        .await
        .expect("send shutdown control");

    let outcome = tokio::time::timeout(Duration::from_secs(60), exit_rx)
        .await
        .expect(
            "serve must return once the world is gone; before WS01-S8 it sat in the reader \
             forever with a dead world behind it",
        )
        .expect("the server thread published its outcome");
    assert_eq!(
        outcome,
        Ok(()),
        "an operator shutdown is an ordinary exit, not a fault"
    );

    // And the boundary really is gone, so a supervisor would see the EOF.
    drop(writer);
}
