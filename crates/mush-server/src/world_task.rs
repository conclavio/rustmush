//! The single-writer world task (design 01 §4.1) plus the M2 write-through
//! persistence seam.
//!
//! One tokio task owns the [`Engine`] (and therefore the `World`) outright —
//! zero locks on game state. Everything else sends [`WorldMsg`] over an mpsc
//! and receives output through the [`SessionManager`].
//!
//! Each loop iteration:
//!
//! 1. `select!` over the inbound channel and a sleep until the scheduler's
//!    next deadline ([`Scheduler::next_due`](mush_core::Scheduler::next_due));
//! 2. feed wall time in ([`Engine::set_now`]), then [`Engine::step`] (or
//!    [`Engine::fire_due`] on a timer wake);
//! 3. **classify and resolve**: each queued [`Audience`] is resolved against
//!    current world state and retained with its [`DeliveryClass`]; effects keep
//!    the same typed relationship to durable truth;
//! 4. **write through** (M2): whatever the step dirtied is snapshotted via
//!    `World::take_dirty()` into ONE [`WriteBatch`](mush_store::WriteBatch)
//!    and sent to the store-writer task immediately — no timer batching
//!    (`@destroy`ed husks are swept to tombstones first, so the same batch
//!    carries their store deletes). Every non-empty batch carries an exact
//!    receipt. Commit-bound output and **all external effects** remain held
//!    until that receipt matches; a deterministic failure releases no success
//!    and launches no effect. A clean read-only step responds immediately;
//! 5. **release**: after acknowledgement, eligible output crosses to the
//!    session/gateway layer and eligible effects become detached tasks whose
//!    completions re-enter as [`Inbound::Apply`]`(`[`WorldCommand`]`)`;
//! 6. **staff ops** (M2): `@siteban`/`@dump`/`@shutdown` handlers
//!    (registered by [`crate::staff`]) queued typed ops during the step;
//!    ban ops are executed by ONE sequential executor task (staff commands
//!    issued in order can never race in the store), `@dump` rides the
//!    store-writer queue, and all are ack-gated and audit-logged
//!    (`@shutdown` instead makes the loop step `ControlMsg::Shutdown` and
//!    exit, same as Ctrl-C/SIGTERM).
//!
//! ## Pause-mutations policy (PLAN risk 3)
//!
//! When the store-writer reports persistent write failure
//! ([`WriterSignal::StoreStatus`]), the world enters **read-only mode**: input
//! lines are refused unless they are on a conservative non-mutating
//! allowlist (look/WHO/speech/...) — a line matching an **exit** is refused
//! even when it spells an allowlisted verb, since dispatch matches exits
//! first and traversal mutates — and connected Wizards get a Sys alert.
//! [`WorldMsg::CreatePlayer`] is refused outright while read-only. The
//! writer keeps retrying the failed batch with backoff; on recovery the
//! mode lifts and Wizards are told. RAM never diverges beyond what is
//! already queued (in order) for the writer.
//!
//! Account-service requests ([`WorldMsg::CreatePlayer`]) arrive as typed
//! messages with a oneshot reply, so `accounts.rs` never touches `World`.
//! The reply is **deferred until the creation batch is acknowledged by the
//! store** — that is the ack gate account/character creation confirmation
//! output rides on.
//!
//! ## WHO wiring (decision)
//!
//! `WHO` inside `mush-core` reads the engine-maintained per-actor session
//! stats (`World::session_stat`), which this task keeps real by feeding wall
//! time before every step and by the engine stamping `last_input` on each
//! [`Inbound::Cmd`] and `connect_time` on the first `ConnUp`. That keeps
//! `mush-core` completely ignorant of `mush-session` — no snapshot pushing,
//! no trait threading — and WHO's idle/connect columns are exact. (The
//! session manager remains the authority on peer addresses; the M1 WHO
//! format does not show them.)

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mush_config::ipguard::IpGuard;
use mush_config::metrics::{LlmFeatureStat, LlmTodaySpend, Metrics, WorldSample};
use mush_core::commands::movement::find_exit;
use mush_core::{
    Audience, ControlMsg, DeliveryClass, DeliveryOutcome, DisconnectKind, Effect, Effects, Engine,
    Flags, Inbound, IncidentId, ObjId, ObjKind, Outbox, PreparedInstallation, Registry, Role,
    ScheduledKind, StagedAction, StagedActionPersistence, StepOutcome, TickCadence, World,
    WorldCommand, WorldEvent, can_perceive, event_subject, is_ignoring, narrative_actor,
    register_core_commands, register_engine_commands,
};
use mush_ipc::{MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS, SendTarget};
use mush_llm::LlmService;
use mush_proto::{OutputEvent, RichText};
use mush_store::{
    AccountRec, AuthoritativeReceipt, InMemoryStore, ObjectStore, SshKeyRec, WriteBatch,
};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::gateway_sink::GatewaySink;
use crate::persist::{self, AuthoritativeCommitFailure, PersistenceIntegrityLatch, StoreJob};
use crate::retained::RetainedSeals;
use crate::staff::{StaffOp, StaffOpKind, StaffQueue};

/// The server-side view of the delivery seam: an `Arc<dyn GatewaySink>` the
/// world loop hands its resolved output and session-control to. The all-in-one
/// binary passes an in-process sink (routes to the shared `SessionManager`); the
/// forked server passes the socket sink (serializes `ToGateway` frames).
pub type Gateway = std::sync::Arc<dyn GatewaySink>;
use mush_config::{unix_now_ms, unix_now_s};

/// Capacity of the world task's inbound channel.
const INBOUND_CAPACITY: usize = 1024;

/// Barrier waiters retained behind one exact receipt. Further callers fail
/// closed (`WorldHandle::barrier` returns false) instead of growing an
/// unbounded side queue during a prolonged store outage.
const MAX_PENDING_BARRIERS: usize = 256;

/// Latest full gateway reannounce snapshot retained while one exact receipt is
/// pending. A reannounce is replaceable state, not an ordered event stream, so
/// newer snapshots coalesce over older ones. This matches the independently
/// enforced lifecycle-obligation ceiling at the IPC boundary.
const MAX_PENDING_REATTACH_RECORDS: usize = MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS;

/// Conservative logical byte charges for fixed-shape operational custody. The
/// count ceilings are the physical bound; these charges make the byte budget
/// explicit and stable across allocator/platform layout changes.
const PENDING_BARRIER_CHARGE_BYTES: usize = 256;
const PENDING_REATTACH_CHARGE_BYTES: usize = 64;
const MAX_PENDING_REATTACH_BYTES: usize =
    MAX_PENDING_REATTACH_RECORDS * PENDING_REATTACH_CHARGE_BYTES;
const MAX_PENDING_OPERATIONAL_BYTES: usize =
    MAX_PENDING_BARRIERS * PENDING_BARRIER_CHARGE_BYTES + MAX_PENDING_REATTACH_BYTES;

/// Capacity of the sequential staff-op executor's queue (rare wizard
/// commands; the bound only matters as backpressure).
const STAFF_OPS_CAPACITY: usize = 64;

/// One loop iteration (step + delivery + flush) taking longer than this is
/// logged as loop lag — the observability hook PLAN's risk 2 asks for. The
/// world loop must never block on slow work; lag here means something
/// synchronous is too slow or the writer queue is back-pressuring.
const LOOP_LAG_WARN: Duration = Duration::from_millis(500);

/// While any sealed ledger chunk is awaiting a durable ack (E0), the world loop
/// wakes on this cadence even with no other traffic, so a dropped seal is
/// re-emitted promptly on an otherwise idle world rather than waiting for the
/// next incidental command. Short enough to keep the loss window tight; long
/// enough that the re-flush is negligible load. Acks normally arrive within one
/// world step, so in the steady state this fires at most once per seal.
const REEMIT_INTERVAL: Duration = Duration::from_millis(250);

/// Global alarm threshold on the retained-seal depth. In healthy
/// operation the table holds at most a handful of seals (one per log that just
/// rolled over, cleared within a step or two). A depth past this means acks are
/// not flowing — the store is wedged or a seal is poison — and is logged so a
/// scraper/operator notices before the bounded writer queue backs the loop up.
const RETAINED_DEPTH_ALARM: usize = 64;

/// Player creation failure, reported back over the oneshot.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CreatePlayerErr {
    /// An actor with that name (or alias) already exists.
    #[error("that name is already in use")]
    NameTaken,
    /// Any other world error (invariant violation — should not happen).
    #[error("world error: {0}")]
    World(String),
    /// The world task is gone (shutdown).
    #[error("the world is shutting down")]
    WorldGone,
    /// Another authoritative action is awaiting its exact receipt. The caller
    /// may retry; the request was refused before touching world state.
    #[error("the world is committing another action; please try again shortly")]
    Busy,
    /// The store is unavailable (read-only mode): creating a Player would
    /// mutate RAM with no prospect of a timely durable ack, so creation is
    /// refused up front instead of leaving the requester hanging.
    #[error("the database is temporarily unavailable; please try again shortly")]
    StoreUnavailable,
}

/// One held connection to silently re-attach after a server restart (M16c).
///
/// Built by the boundary reader from a [`mush_ipc::AttachRec`]; carries only the
/// fields [`Engine::reattach`](mush_core::Engine::reattach) needs. The character
/// name / account on the wire record are for the gateway's own logging — the
/// server resolves the actor's name from the reloaded world, so they are not
/// carried here.
#[derive(Clone, Debug)]
pub struct ReAttachRec {
    /// The physical connection id.
    pub conn: u64,
    /// The actor the connection is playing.
    pub actor: ObjId,
    /// Whether the client understands structured (JSON) events.
    pub caps_structured: bool,
}

/// Everything the world task can receive: engine inbound traffic plus typed
/// service requests from async subsystems that must not touch `World`.
pub enum WorldMsg {
    /// Ordinary engine input (commands, connection lifecycle, applies,
    /// control).
    Inbound(Inbound),
    /// Silent re-attach of the gateway's held online connections after a server
    /// restart (M16c). Each rec is applied via
    /// [`Engine::reattach`](mush_core::Engine::reattach) — a SILENT `ConnUp`
    /// (no room announce, no connect hook, no `WorldEvent::Connected`) — then a
    /// fresh per-session `look` renders the reloaded room. One world-loop
    /// iteration rebuilds the whole live-connection map after a boot.
    ReAttach {
        /// The held connections to re-attach.
        recs: Box<[ReAttachRec]>,
    },
    /// Create a Player object (account/character creation): the world task
    /// creates it in the start room, writes the creation batch through the
    /// store (`durable: true`), and replies with the new id **once the
    /// store has acknowledged the batch** — the ack gate for creation
    /// confirmations.
    CreatePlayer {
        /// The character name.
        name: String,
        /// Where the result goes.
        reply: oneshot::Sender<Result<ObjId, CreatePlayerErr>>,
    },
    /// Is this actor name (player index, case-insensitive) unclaimed?
    /// Used by account creation to pre-check the character name before the
    /// account record is reserved (the create itself still re-validates).
    NameFree {
        /// The name to probe.
        name: String,
        /// `true` = free.
        reply: oneshot::Sender<Result<bool, WorldReadError>>,
    },
    /// Is this actor currently inside an input context (chargen wizard, mail
    /// compose, ...)? The connect flow probes this after `ConnUp` so it can
    /// skip the initial auto-`look` when a `Connected` hook has dropped the
    /// player into a context (else the `look` is captured as context input —
    /// e.g. fed to the chargen wizard as a bogus template name, or appended to
    /// a mail body on reconnect).
    InInputContext {
        /// The actor to probe.
        actor: ObjId,
        /// `true` = inside a context.
        reply: oneshot::Sender<Result<bool, WorldReadError>>,
    },
    /// Verify that an admitted `ConnUp` produced the exact engine-side
    /// connection-to-actor association before the gateway publishes its local
    /// session as ready.
    ConnectionReady {
        /// The physical connection id.
        conn: u64,
        /// The expected actor.
        actor: ObjId,
        /// `true` only for the exact tracked association.
        reply: oneshot::Sender<Result<bool, WorldReadError>>,
    },
    /// Ordering barrier: replied to once every message queued before it has
    /// been processed *and its output delivered*. The QUIT flow uses this to
    /// flush a quitting player's in-flight output before detaching them.
    Barrier {
        /// Completed only when the preceding delivery boundary succeeds.
        reply: oneshot::Sender<Result<(), WorldReadError>>,
    },
}

/// A typed refusal for a world read that cannot safely observe authoritative
/// state right now.
///
/// In particular, [`WorldMode::CommitPending`] has already installed the
/// tentative in-RAM post-image but has not received its exact durable receipt.
/// Returning `false` from `NameFree` in that window would lie that a name is
/// taken; returning an input-context default could feed an automatic `look`
/// into a live editor. Callers must keep these outcomes distinct from a real
/// negative answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorldReadError {
    /// A serialization point is active; retry after the pending commit resolves.
    #[error("the world is busy committing another action")]
    Busy,
    /// The world task disappeared or is not able to answer this read.
    #[error("the world is unavailable")]
    Unavailable,
}

/// A control signal from the store-writer task to the world loop (health
/// transitions and authoritative-ledger breaches).
///
/// These are drained from [`WriterSignalReceiver`]'s bounded, coalescing
/// mailbox. The channel itself carries only a capacity-one wake token; mandatory
/// state lives in fixed slots plus an exact-resolution map bounded by the same
/// identity limit as [`RetainedSeals`]. The writer only takes a short sync lock
/// and `try_send`s the wake, so it can never await the world and deadlock against
/// store-job backpressure.
pub enum WriterSignal {
    /// Store-writer health transition: `false` enters read-only mode (mutations
    /// paused, Wizards alerted), `true` lifts it (unless a ledger-breach freeze
    /// is latched).
    StoreStatus {
        /// Whether writes are succeeding again.
        healthy: bool,
        /// The triggering error, for the staff alert (empty on recovery).
        detail: String,
    },
    /// An **authoritative-ledger breach** (E0): a batch carrying a sealed ledger
    /// chunk was dropped by a *deterministic* store rejection. Latches the
    /// non-auto-lifting [`WorldMode::IntegrityFrozen`] state.
    LedgerBreach {
        /// Human detail for the staff freeze alert.
        detail: String,
        /// Per-seal resolution once the writer has isolated one:
        /// `Some((ns, key, payload_sha256, quarantined))` — the digest makes the
        /// resolution exact even if corrupt input presents two payloads under
        /// one key. `quarantined = true` is a confirmed poison seal (drop it
        /// from re-emit and keep it out; it is preserved in the quarantine
        /// spool), `false` is a seal healed in isolation (durable now; just drop
        /// it from re-emit). `None` is the initial fail-closed freeze with no
        /// specific seal yet resolved.
        seal: Option<(smol_str::SmolStr, smol_str::SmolStr, String, bool)>,
    },
    /// A non-ledger write batch was rejected deterministically. RAM may now
    /// contain an uncommitted mutation, so continuing writable would compound
    /// divergence; latch the same world integrity barrier.
    DeterministicFailure {
        /// Bounded store/writer diagnostic for incident identity and paging.
        detail: String,
    },
}

const WRITER_SIGNAL_DETAIL_BYTES: usize = 4 * 1024;
const MAX_WRITER_RESOLUTION_BYTES: usize = 16 * 1024 * 1024;

type WriterSealIdentity = (SmolStr, SmolStr, String);

struct BufferedSealResolution {
    detail: String,
    quarantined: bool,
}

#[derive(Default)]
struct WriterSignalState {
    store_status: Option<(bool, String)>,
    deterministic_failure: Option<String>,
    ledger_breach: Option<String>,
    resolutions: BTreeMap<WriterSealIdentity, BufferedSealResolution>,
    resolution_bytes: usize,
}

/// Non-blocking writer half of the mandatory control mailbox.
#[derive(Clone)]
pub struct WriterSignalSender {
    state: Arc<Mutex<WriterSignalState>>,
    wake: mpsc::Sender<()>,
}

impl WriterSignalSender {
    pub(crate) fn store_status(&self, healthy: bool, detail: String) {
        self.with_state(|state| {
            // Only the latest transition matters before the world observes it:
            // fail→recover coalescing to healthy leaves the correct final mode;
            // recover→fail leaves it read-only.
            state.store_status = Some((healthy, bound_writer_detail(detail)));
        });
        self.wake();
    }

    pub(crate) fn deterministic_failure(&self, detail: String) {
        self.with_state(|state| {
            // The first root cause is the useful paging identity. The process
            // integrity latch is monotonic, so further details need not grow a
            // queue in order to preserve the freeze fact.
            state
                .deterministic_failure
                .get_or_insert_with(|| bound_writer_detail(detail));
        });
        self.wake();
    }

    /// Publish a ledger freeze or one exact seal resolution. `false` means the
    /// exact-resolution buffer was unexpectedly saturated; the caller must
    /// keep the identity in writer-local custody and retry it when the retained
    /// seal is re-emitted. No resolution may be discarded on this path.
    #[must_use]
    pub(crate) fn ledger_breach(
        &self,
        detail: String,
        seal: Option<(SmolStr, SmolStr, String, bool)>,
    ) -> bool {
        let detail = bound_writer_detail(detail);
        let accepted = self.with_state(|state| {
            let Some((ns, key, payload_hash, quarantined)) = seal else {
                state.ledger_breach.get_or_insert(detail);
                return true;
            };
            let identity = (ns, key, payload_hash);
            if let Some(existing) = state.resolutions.get_mut(&identity) {
                if existing.quarantined != quarantined {
                    // Contradictory resolution is itself an integrity breach.
                    // Quarantine is the conservative terminal posture.
                    existing.quarantined = true;
                    state.ledger_breach.get_or_insert_with(|| {
                        "writer reported contradictory outcomes for one exact seal identity"
                            .to_string()
                    });
                }
                return true;
            }
            let accounted_bytes = identity
                .0
                .len()
                .saturating_add(identity.1.len())
                .saturating_add(identity.2.len())
                .saturating_add(detail.len());
            let projected_bytes = state.resolution_bytes.saturating_add(accounted_bytes);
            if state.resolutions.len() >= crate::retained::MAX_RETAINED_SEALS
                || projected_bytes > MAX_WRITER_RESOLUTION_BYTES
            {
                state.ledger_breach.get_or_insert_with(|| {
                    "exact writer-resolution mailbox saturated; identity remains retained for retry"
                        .to_string()
                });
                return false;
            }
            state.resolution_bytes = projected_bytes;
            state.resolutions.insert(
                identity,
                BufferedSealResolution {
                    detail,
                    quarantined,
                },
            );
            true
        });
        self.wake();
        accepted
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut WriterSignalState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut state)
    }

    fn wake(&self) {
        match self.wake.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => {}
            Err(TrySendError::Closed(())) => {
                tracing::debug!(
                    "world task gone; mandatory writer signal retained until sender drop"
                );
            }
        }
    }
}

/// World half of the mandatory writer-control mailbox.
pub struct WriterSignalReceiver {
    state: Arc<Mutex<WriterSignalState>>,
    wake: mpsc::Receiver<()>,
    ready: VecDeque<WriterSignal>,
}

impl WriterSignalReceiver {
    /// Await one coalesced signal. A single wake drains every fact published up
    /// to the lock acquisition; a racing publisher either lands in that drain
    /// or leaves the capacity-one token armed for the next call.
    pub async fn recv(&mut self) -> Option<WriterSignal> {
        loop {
            if let Some(signal) = self.ready.pop_front() {
                return Some(signal);
            }
            match self.wake.recv().await {
                Some(()) => self.drain_state(),
                None => {
                    self.drain_state();
                    return self.ready.pop_front();
                }
            }
        }
    }

    /// Non-blocking receive used by deterministic writer tests.
    pub fn try_recv(&mut self) -> Result<WriterSignal, TryRecvError> {
        loop {
            if let Some(signal) = self.ready.pop_front() {
                return Ok(signal);
            }
            match self.wake.try_recv() {
                Ok(()) => self.drain_state(),
                Err(TryRecvError::Empty) => return Err(TryRecvError::Empty),
                Err(TryRecvError::Disconnected) => {
                    self.drain_state();
                    return self.ready.pop_front().ok_or(TryRecvError::Disconnected);
                }
            }
        }
    }

    fn drain_state(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((healthy, detail)) = state.store_status.take() {
            self.ready
                .push_back(WriterSignal::StoreStatus { healthy, detail });
        }
        if let Some(detail) = state.deterministic_failure.take() {
            self.ready
                .push_back(WriterSignal::DeterministicFailure { detail });
        }
        if let Some(detail) = state.ledger_breach.take() {
            self.ready
                .push_back(WriterSignal::LedgerBreach { detail, seal: None });
        }
        for ((ns, key, payload_hash), resolution) in std::mem::take(&mut state.resolutions) {
            self.ready.push_back(WriterSignal::LedgerBreach {
                detail: resolution.detail,
                seal: Some((ns, key, payload_hash, resolution.quarantined)),
            });
        }
        state.resolution_bytes = 0;
    }
}

/// Construct the bounded/coalescing mandatory writer-control mailbox.
#[must_use]
pub fn writer_signal_channel() -> (WriterSignalSender, WriterSignalReceiver) {
    let state = Arc::new(Mutex::new(WriterSignalState::default()));
    let (wake_tx, wake_rx) = mpsc::channel(1);
    (
        WriterSignalSender {
            state: Arc::clone(&state),
            wake: wake_tx,
        },
        WriterSignalReceiver {
            state,
            wake: wake_rx,
            ready: VecDeque::new(),
        },
    )
}

fn bound_writer_detail(mut detail: String) -> String {
    if detail.len() > WRITER_SIGNAL_DETAIL_BYTES {
        let mut boundary = WRITER_SIGNAL_DETAIL_BYTES;
        while !detail.is_char_boundary(boundary) {
            boundary -= 1;
        }
        detail.truncate(boundary);
    }
    detail
}

/// Driver-local identity of the commit whose acknowledgement is currently
/// pending. Migrated K3 actions carry the exact durable bundle token; legacy
/// batches retain their process-local writer serial during the transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitToken {
    /// Process-local identity for a legacy `WriteBatch` receipt.
    Legacy(u64),
    /// Durable K3 identity embedded in an immutable authoritative bundle.
    Authoritative(mush_core::CommitToken),
}

impl CommitToken {
    /// Construct the identity of a legacy batch from its monotonically
    /// increasing process-local writer serial.
    #[must_use]
    pub const fn from_serial(serial: u64) -> Self {
        Self::Legacy(serial)
    }

    #[must_use]
    pub const fn from_authoritative(token: mush_core::CommitToken) -> Self {
        Self::Authoritative(token)
    }
}

/// The single structural authority over what the world loop may execute.
///
/// Modes are deliberately mutually exclusive. In particular, an integrity
/// freeze cannot be accidentally lifted by toggling an unrelated store-health
/// boolean, and admission never infers this state from command text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldMode {
    /// Boot validation is still in progress; mutating ingress is not attached.
    Booting,
    /// Durable recovery/reconciliation is still in progress.
    Recovering {
        /// Incident being recovered, when recovery was incident-driven.
        incident: Option<IncidentId>,
    },
    /// Normal operation.
    Writable,
    /// One authoritative commit is awaiting its exact acknowledgement.
    CommitPending {
        /// The commit which owns the serialization point.
        token: CommitToken,
    },
    /// The persistence backend is transiently unavailable.
    StoreReadOnly {
        /// Stable identity for this observed outage.
        incident: IncidentId,
    },
    /// Integrity is uncertain; only observation, recovery and shutdown remain.
    IntegrityFrozen {
        /// Stable identity of the integrity incident.
        incident: IncidentId,
    },
}

impl WorldMode {
    /// Whether ordinary world mutation may execute.
    #[must_use]
    pub const fn is_writable(self) -> bool {
        matches!(self, Self::Writable)
    }

    /// Whether store-backed mutation is unavailable in this mode.
    #[must_use]
    pub const fn store_read_only(self) -> bool {
        matches!(
            self,
            Self::StoreReadOnly { .. } | Self::IntegrityFrozen { .. }
        )
    }

    /// Whether an authoritative integrity incident is latched.
    #[must_use]
    pub const fn integrity_frozen(self) -> bool {
        matches!(self, Self::IntegrityFrozen { .. })
    }
}

/// The world loop's in-task mirror of store-writer health — the small
/// `StoreStatus` mirror `@stats` reads (the writer task itself only sends
/// health *transitions*; the loop keeps the current picture).
#[derive(Clone, Copy, Debug)]
pub struct StoreHealth {
    /// The world's one authoritative execution/admission mode.
    pub mode: WorldMode,
    /// Unix seconds when the most recent batch was handed to the writer
    /// (`None` before the first write of this boot).
    pub last_write_unix: Option<i64>,
}

impl StoreHealth {
    /// Healthy, nothing written yet.
    #[must_use]
    pub fn new() -> StoreHealth {
        StoreHealth {
            mode: WorldMode::Writable,
            last_write_unix: None,
        }
    }
}

impl Default for StoreHealth {
    fn default() -> Self {
        StoreHealth::new()
    }
}

fn incident_id(domain: &str, detail: &str) -> IncidentId {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(detail.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    IncidentId(u64::from_be_bytes(bytes))
}

/// Structural ingress classes used by the mode gate. This list names execution
/// effects rather than command spellings; adding a new [`WorldMsg`],
/// [`Inbound`], [`WriterSignal`] or [`StaffOpKind`] cannot compile until its
/// class is chosen in the exhaustive classifiers below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IngressClass {
    /// A gameplay/world read whose answer depends on authoritative world or
    /// module state. It may not observe the tentative post-image of an
    /// unacknowledged commit.
    WorldRead,
    /// Receipt/order bookkeeping which does not inspect gameplay state.
    OrderingBarrier,
    Command,
    TransportBookkeeping,
    ConnectionHooks,
    WorldMutation,
    AsyncApply,
    Timer,
    WriterRecovery,
    StaffStatus,
    StaffMutation,
    ExternalEffect,
    Shutdown,
}

/// Why structurally classified work was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefusalReason {
    NotReady,
    CommitPending,
    StoreUnavailable,
    IntegrityFrozen,
}

impl RefusalReason {
    fn text(self) -> &'static str {
        match self {
            Self::NotReady => "The world is still starting; please try again shortly.",
            Self::CommitPending => {
                "The world is committing another action; please retry in a moment."
            }
            Self::StoreUnavailable => {
                "The database is currently unavailable; commands that change the world are paused."
            }
            Self::IntegrityFrozen => {
                "The world is integrity-frozen; mutating actions are paused while recovery runs."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Admission {
    Allow,
    Refuse(RefusalReason),
    /// Store-read-only mode retains the legacy conservative command parser.
    /// Hard integrity modes never take this branch: every command is refused
    /// structurally before its verb or input-context contents are inspected.
    CheckStoreReadOnlyCommand,
}

fn classify_inbound(msg: &Inbound) -> IngressClass {
    match msg {
        Inbound::Cmd { .. } => IngressClass::Command,
        Inbound::Query { .. } => IngressClass::WorldRead,
        Inbound::ConnUp { .. } | Inbound::ConnDown { .. } | Inbound::SessionGone { .. } => {
            IngressClass::ConnectionHooks
        }
        Inbound::Apply(_) => IngressClass::AsyncApply,
        Inbound::Control(ControlMsg::Shutdown) => IngressClass::Shutdown,
        Inbound::Control(ControlMsg::Dump) => IngressClass::StaffMutation,
    }
}

fn classify_world_msg(msg: &WorldMsg) -> IngressClass {
    match msg {
        WorldMsg::Inbound(msg) => classify_inbound(msg),
        WorldMsg::ReAttach { .. } => IngressClass::TransportBookkeeping,
        WorldMsg::CreatePlayer { .. } => IngressClass::WorldMutation,
        WorldMsg::NameFree { .. }
        | WorldMsg::InInputContext { .. }
        | WorldMsg::ConnectionReady { .. } => IngressClass::WorldRead,
        WorldMsg::Barrier { .. } => IngressClass::OrderingBarrier,
    }
}

/// Work whose producer has already transferred semantic/lifecycle custody and
/// therefore cannot be answered Busy or dropped merely because another exact
/// receipt is pending. One such item may be held locally to preserve FIFO.
/// Ordinary commands and typed service mutations are refused immediately so
/// they cannot head-of-line block later operational traffic.
fn must_defer_behind_commit(msg: &WorldMsg) -> bool {
    matches!(
        msg,
        WorldMsg::Inbound(
            Inbound::ConnUp { .. }
                | Inbound::ConnDown { .. }
                | Inbound::SessionGone { .. }
                | Inbound::Apply(_)
        )
    )
}

fn classify_writer_signal(signal: &WriterSignal) -> IngressClass {
    match signal {
        WriterSignal::StoreStatus { .. }
        | WriterSignal::LedgerBreach { .. }
        | WriterSignal::DeterministicFailure { .. } => IngressClass::WriterRecovery,
    }
}

fn classify_staff_op(kind: &StaffOpKind) -> IngressClass {
    match kind {
        StaffOpKind::ListBans
        | StaffOpKind::ListSshKeys { .. }
        | StaffOpKind::Ps
        | StaffOpKind::Stats => IngressClass::StaffStatus,
        StaffOpKind::Shutdown => IngressClass::Shutdown,
        StaffOpKind::SetBan { .. }
        | StaffOpKind::ClearBan { .. }
        | StaffOpKind::Dump
        | StaffOpKind::Boot { .. }
        | StaffOpKind::NewPassword { .. }
        | StaffOpKind::AddSshKey { .. }
        | StaffOpKind::DelSshKey { .. }
        | StaffOpKind::NewChar { .. } => IngressClass::StaffMutation,
    }
}

const fn admission_for(mode: WorldMode, class: IngressClass) -> Admission {
    use Admission::{Allow, CheckStoreReadOnlyCommand, Refuse};
    use IngressClass::{
        AsyncApply, Command, ConnectionHooks, ExternalEffect, OrderingBarrier, Shutdown,
        StaffMutation, StaffStatus, Timer, TransportBookkeeping, WorldMutation, WorldRead,
        WriterRecovery,
    };

    match mode {
        WorldMode::Writable => Allow,
        WorldMode::Booting | WorldMode::Recovering { .. } => match class {
            WorldRead | OrderingBarrier | TransportBookkeeping | WriterRecovery | StaffStatus
            | Shutdown => Allow,
            Command | ConnectionHooks | WorldMutation | AsyncApply | Timer | StaffMutation
            | ExternalEffect => Refuse(RefusalReason::NotReady),
        },
        WorldMode::CommitPending { .. } => match class {
            OrderingBarrier | TransportBookkeeping | WriterRecovery | StaffStatus | Shutdown => {
                Allow
            }
            WorldRead | Command | ConnectionHooks | WorldMutation | AsyncApply | Timer
            | StaffMutation | ExternalEffect => Refuse(RefusalReason::CommitPending),
        },
        WorldMode::StoreReadOnly { .. } => match class {
            WorldRead | OrderingBarrier | TransportBookkeeping | ConnectionHooks
            | WriterRecovery | StaffStatus | Shutdown => Allow,
            Command => CheckStoreReadOnlyCommand,
            WorldMutation | AsyncApply | Timer | StaffMutation | ExternalEffect => {
                Refuse(RefusalReason::StoreUnavailable)
            }
        },
        WorldMode::IntegrityFrozen { .. } => match class {
            WorldRead | OrderingBarrier | TransportBookkeeping | WriterRecovery | StaffStatus
            | Shutdown => Allow,
            Command | ConnectionHooks | WorldMutation | AsyncApply | Timer | StaffMutation
            | ExternalEffect => Refuse(RefusalReason::IntegrityFrozen),
        },
    }
}

fn admit_world_msg(mode: WorldMode, world: &World, msg: &WorldMsg) -> Admission {
    let admission = admission_for(mode, classify_world_msg(msg));
    if admission != Admission::CheckStoreReadOnlyCommand {
        return admission;
    }
    let WorldMsg::Inbound(inbound) = msg else {
        unreachable!("only Inbound::Cmd is classified as Command")
    };
    if refuse_store_read_only(world, inbound).is_some() {
        Admission::Refuse(RefusalReason::StoreUnavailable)
    } else {
        Admission::Allow
    }
}

/// Cloneable handle for sending into the world task.
#[derive(Clone)]
pub struct WorldHandle {
    tx: mpsc::Sender<WorldMsg>,
    shutdown: Option<watch::Sender<bool>>,
}

impl WorldHandle {
    /// Send one engine inbound message; `false` if the world task is gone.
    pub async fn inbound(&self, msg: Inbound) -> bool {
        if matches!(msg, Inbound::Control(ControlMsg::Shutdown))
            && let Some(shutdown) = &self.shutdown
        {
            return shutdown.send(true).is_ok();
        }
        self.tx.send(WorldMsg::Inbound(msg)).await.is_ok()
    }

    /// Silently re-attach a batch of held connections after a server restart
    /// (M16c): each rebuilds its world-side bookkeeping WITHOUT a connect
    /// announce/hook/event, then the reloaded room is pushed to each reconnected
    /// player (one render per session, fanned to its connections). Rebuilds
    /// `connected`/`live_conns`/`session_start` for the whole held roster in one
    /// world-loop iteration. Fire-and-forget; `false` if the world task is gone
    /// or the caller violates the bounded lifecycle-roster protocol. The latter
    /// is rejected atomically before any record reaches the world.
    pub async fn reattach(&self, recs: Vec<ReAttachRec>) -> bool {
        if recs.len() > MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS {
            tracing::error!(
                records = recs.len(),
                cap = MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
                "reattach roster violates bounded lifecycle custody; refusing whole snapshot"
            );
            return false;
        }
        self.tx
            .send(WorldMsg::ReAttach {
                recs: recs.into_boxed_slice(),
            })
            .await
            .is_ok()
    }

    /// Ask the world task to create a Player named `name`. Resolves only
    /// after the creation batch is acknowledged by the store.
    pub async fn create_player(&self, name: &str) -> Result<ObjId, CreatePlayerErr> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::CreatePlayer {
                name: name.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return Err(CreatePlayerErr::WorldGone);
        }
        rx.await.unwrap_or(Err(CreatePlayerErr::WorldGone))
    }

    /// Whether an actor name is currently unclaimed.
    ///
    /// A read arriving during [`WorldMode::CommitPending`] receives
    /// [`WorldReadError::Busy`] instead of seeing tentative RAM or inventing a
    /// false "taken" answer. A gone task is [`WorldReadError::Unavailable`].
    pub async fn name_free(&self, name: &str) -> Result<bool, WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::NameFree {
                name: name.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return Err(WorldReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
    }

    /// Whether `actor` is currently inside an input context (chargen wizard,
    /// mail compose, ...). Probed by the connect flow to decide whether to send
    /// the initial auto-`look`; busy/unavailable is typed so that flow can fail
    /// closed instead of guessing that no context exists.
    pub async fn in_input_context(&self, actor: ObjId) -> Result<bool, WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::InInputContext { actor, reply })
            .await
            .is_err()
        {
            return Err(WorldReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
    }

    /// Whether the engine tracks `conn` as belonging to `actor`. This is the
    /// correlated post-`ConnUp` readiness proof used by the gateway; unlike an
    /// ordering barrier alone, it detects lifecycle ingress refused by a hard
    /// world mode or ignored for a missing actor.
    pub async fn connection_ready(&self, conn: u64, actor: ObjId) -> Result<bool, WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::ConnectionReady { conn, actor, reply })
            .await
            .is_err()
        {
            return Err(WorldReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
    }

    /// Typed round-trip barrier: resolves once everything sent through this
    /// handle before the call has been processed and its output delivered.
    /// Bounded pending-custody overload stays retryable [`WorldReadError::Busy`]
    /// while an aborted receipt, explicit refusal, or vanished task is
    /// [`WorldReadError::Unavailable`].
    pub async fn barrier_result(&self) -> Result<(), WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(WorldMsg::Barrier { reply }).await.is_err() {
            return Err(WorldReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
    }

    /// Compatibility wrapper for internal callers that only need a completion
    /// bit. Service boundaries use [`Self::barrier_result`] so Busy is not
    /// collapsed into an unavailable failure.
    pub async fn barrier(&self) -> bool {
        self.barrier_result().await.is_ok()
    }
}

/// The server-side dependencies the world task drives besides the engine:
/// the store (staff ops, bgsave), the IP guard (siteban mirror), and the
/// staff-command queue shared with the registry handlers.
pub struct ServerDeps {
    /// The persistence backend.
    pub store: Arc<dyn ObjectStore>,
    /// The shared per-IP guard (its siteban mirror is updated here).
    pub guard: Arc<IpGuard>,
    /// The queue `register_server_commands` handlers push into.
    pub staff: Arc<StaffQueue>,
    /// The M7 LLM service handle. Present in the real server (built from
    /// `[llm]` config); [`spawn_effects`] uses it to run a module's deferred
    /// [`Effect::ModuleAsync`] (worldgen `@gen`) off the world loop and
    /// re-enter as [`WorldCommand::ModuleAsyncDone`]. `None` for the compat
    /// [`spawn`] and lightweight tests that boot no LLM-driven module — such
    /// an effect is then logged and dropped, never a panic. The M8 NPC module
    /// shares this same handle (its off-loop turn is driven from [`spawn_effects`]);
    /// its on-loop ability execution goes through the module's grounding adapter,
    /// not this struct.
    pub llm: Option<Arc<LlmService>>,
    /// The shared metrics registry (PLAN "Observability"): the world loop
    /// records loop lag every iteration and publishes the world-owned gauges
    /// (object counts, store health, scheduler depth, NPC + LLM aggregates)
    /// each throttled sample; the store writer stamps write-queue depth and
    /// last-ack age. Read back by `mush-net-web`'s `/healthz` + `/metrics`.
    pub metrics: Arc<Metrics>,
    /// Required E0 authoritative-ledger quarantine sink. Production supplies a
    /// durable, boot-validated spool; tests opt into the explicit in-memory
    /// implementation. There is no log-only preservation mode.
    pub quarantine_spool: crate::quarantine::QuarantineSpool,
}

impl ServerDeps {
    /// Throwaway deps for tests and the compat [`spawn`]: fresh in-memory
    /// store, fresh guard, fresh (never-populated) staff queue, no LLM, a
    /// throwaway metrics registry, and an explicit in-memory quarantine spool.
    #[must_use]
    pub fn in_memory() -> ServerDeps {
        ServerDeps {
            store: Arc::new(InMemoryStore::new()),
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
        }
    }
}

/// Handles from [`spawn_persist`]: the sender everything talks through and
/// the join handles the shutdown sequence awaits **in order** (world first,
/// then the writer — dropping the world task's job sender is what lets the
/// writer drain and exit).
pub struct PersistHandles {
    /// Sender handle for the world task.
    pub handle: WorldHandle,
    /// The world task itself.
    pub world_join: JoinHandle<()>,
    /// The store-writer task (drains after the world task ends).
    pub writer_join: JoinHandle<()>,
    /// Process-local fail-closed snapshot barrier shared by the world,
    /// store writer, and shutdown coordinator.
    pub persistence_integrity: PersistenceIntegrityLatch,
}

/// Boot the standard engine: minimal world (Limbo `#0`, God `#1`) with
/// core + engine commands registered and the clock set to now.
///
/// Deliberately *not* `mush_testkit::boot_world` — the testkit is a dev
/// tool; the server owns its own boot wiring (and additionally registers
/// the engine commands `@slowtest` / `@wait`). The real server boot loads
/// the world from the store instead ([`persist::load_or_seed_world`]) and
/// also registers the staff commands
/// ([`crate::staff::register_server_commands`]).
///
/// # Panics
///
/// Panics if command registration collides — a programming error.
#[must_use]
pub fn boot_engine() -> Engine {
    let mut world = World::new_minimal();
    world.set_now(unix_now_s());
    Engine::new(world, base_registry())
}

/// The always-registered command table: core + engine commands.
///
/// # Panics
///
/// Panics if command registration collides — a programming error.
#[must_use]
pub fn base_registry() -> Registry {
    let mut registry = Registry::new();
    register_core_commands(&mut registry).expect("core command registration must not collide");
    register_engine_commands(&mut registry).expect("engine command registration must not collide");
    registry
}

/// Compatibility spawn (tests, tools): the full world task over throwaway
/// in-memory persistence. Returns the handle everything sends through and
/// the world join handle; the writer task is detached (its writes go to a
/// store nobody reads).
/// Spawn the world task and its store-writer with a caller-supplied server →
/// gateway [`GatewaySink`] (M16b). The `make_gateway` factory is handed the
/// world loop's inbound [`WeakSender`](mpsc::WeakSender) so the all-in-one
/// in-process sink can wire its slow-client / link-dead policing feedback (a
/// WEAK sender: the world loop owns the sink, so a strong handle would keep its
/// own inbound channel alive and it could never shut down). The forked
/// `mush-server` ignores it and returns the [`SocketSink`](crate::gateway_sink::SocketSink),
/// whose output is serialized as [`ToGateway`](mush_ipc::ToGateway) frames onto
/// the boundary.
///
/// The world task exits when [`Inbound::Control`]`(Shutdown)` steps to
/// [`StepOutcome::Shutdown`] or when every [`WorldHandle`] (and internal effect
/// sender) is dropped; the writer drains its queue and exits after that.
pub fn spawn_persist_with_gateway(
    engine: Engine,
    deps: ServerDeps,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> PersistHandles {
    spawn_persist_with_gateway_mode(engine, deps, None, make_gateway)
}

/// Shared spawn implementation. Production always supplies `None`; unit tests
/// can start the real loop in a hard mode to exercise the execution gate rather
/// than merely asserting its classification table.
fn spawn_persist_with_gateway_mode(
    engine: Engine,
    deps: ServerDeps,
    initial_mode: Option<WorldMode>,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> PersistHandles {
    let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = WorldHandle {
        tx: tx.clone(),
        shutdown: Some(shutdown_tx),
    };
    // Build the delivery sink now that the inbound queue exists: the all-in-one
    // in-process sink wires its policing feedback onto this loop's own inbound
    // queue via the weak sender; the socket sink ignores it.
    let gateway = make_gateway(tx.downgrade());
    // The durable-commit high-water (E0): the writer publishes the serial of
    // every batch the store acks (monotone `fetch_max`); the world loop reads
    // it to clear retained sealed-chunk re-emits. A lock-free `AtomicU64` shared
    // both ways — never a channel, so it can never deadlock against the writer's
    // job queue.
    let high_water = Arc::new(AtomicU64::new(0));
    // The writer's mandatory control facts (health transitions, breach latch,
    // exact seal resolutions) ride a dedicated coalescing mailbox, never the
    // bounded inbound queue. The mailbox keeps facts in bounded state and uses
    // only a capacity-one wake token, so publishing cannot await the world or
    // deadlock against `writer.send(StoreJob)` back-pressure.
    let (signal_tx, signal_rx) = writer_signal_channel();
    // This latch is not an eventually delivered world-mode notification. Both
    // the writer and world publish breaches directly into it, and the shutdown
    // coordinator reads the same atomic before any ordinary snapshot.
    let persistence_integrity = PersistenceIntegrityLatch::new();
    // The writer publishes the commit high-water, stamps the metrics registry
    // (write-queue depth + last-ack age) as it drains each batch, and sends its
    // control signals over `signal_tx`.
    let (writer_tx, writer_join) = persist::spawn_store_writer(
        Arc::clone(&deps.store),
        signal_tx,
        Arc::clone(&deps.metrics),
        Arc::clone(&high_water),
        deps.quarantine_spool.clone(),
        persistence_integrity.clone(),
    );
    // Sequential executor for staff store ops (@siteban set/clear/list):
    // ONE task draining ONE queue, so two staff commands issued in order
    // can never race each other in the store or the IpGuard mirror. It
    // drains and exits when the world loop (the only sender) ends.
    let (staff_tx, staff_rx) = mpsc::channel(STAFF_OPS_CAPACITY);
    // The staff executor gets only a WEAK world sender (like the writer):
    // it needs the world for `@newpassword`'s web-connection boots, but a
    // strong sender would keep the world loop's channel alive while the
    // loop is what closes the staff queue — a shutdown deadlock.
    tokio::spawn(staff_ops_loop(
        Arc::clone(&deps.store),
        Arc::clone(&deps.guard),
        tx.downgrade(),
        staff_rx,
        gateway.clone(),
    ));
    // The loop keeps only a weak sender for effect completions — holding a
    // strong one would keep its own channel open forever, so dropping every
    // WorldHandle could never end the task.
    let world_join = tokio::spawn(run(
        engine,
        rx,
        tx.downgrade(),
        writer_tx,
        deps,
        staff_tx,
        gateway,
        high_water,
        signal_rx,
        shutdown_rx,
        persistence_integrity.clone(),
        initial_mode,
    ));
    PersistHandles {
        handle,
        world_join,
        writer_join,
        persistence_integrity,
    }
}

/// One `select!` wake: a message, a writer control signal, a due timer, or
/// channel closure.
enum Wake {
    Msg(WorldMsg),
    Signal(WriterSignal),
    CommitAck(Result<(), AuthoritativeCommitFailure>),
    Timer,
    Shutdown,
    ShutdownDeadline,
    Closed,
}

type ClassifiedSend = (DeliveryClass, SendTarget, Arc<OutputEvent>);

struct CreateCompletion {
    id: ObjId,
    reply: oneshot::Sender<Result<ObjId, CreatePlayerErr>>,
}

/// One authoritative batch whose presentation and external work remain held
/// until the writer returns its matching legacy serial or K3 token/hash receipt.
struct PendingCommit {
    token: CommitToken,
    receipt: PendingReceipt,
    sends: Vec<ClassifiedSend>,
    effects: Vec<(DeliveryClass, Effect)>,
    failure_actor: Option<ObjId>,
    create: Option<CreateCompletion>,
    /// Typed action-state images installed only after this exact receipt.
    prepared_installation: Option<PreparedInstallation>,
    barriers: Vec<oneshot::Sender<Result<(), WorldReadError>>>,
    /// Gateway reannounce is a full replaceable snapshot. Retain only the
    /// newest bounded snapshot while the exact receipt is unresolved; applying
    /// each one immediately would append another rendered room view to
    /// `sends` and make transport churn an unbounded custody side channel.
    coalesced_reattach: Option<Box<[ReAttachRec]>>,
    exit_after_ack: bool,
}

enum PendingReceipt {
    Legacy {
        expected_serial: u64,
        receiver: oneshot::Receiver<Result<u64, String>>,
    },
    Authoritative {
        expected_token: mush_core::CommitToken,
        expected_hash: mush_core::CanonicalHash,
        receiver: oneshot::Receiver<Result<AuthoritativeReceipt, AuthoritativeCommitFailure>>,
    },
}

impl PendingReceipt {
    const fn token(&self) -> CommitToken {
        match self {
            Self::Legacy {
                expected_serial, ..
            } => CommitToken::Legacy(*expected_serial),
            Self::Authoritative { expected_token, .. } => {
                CommitToken::Authoritative(*expected_token)
            }
        }
    }

    async fn receive(&mut self) -> Result<(), AuthoritativeCommitFailure> {
        match self {
            Self::Legacy {
                expected_serial,
                receiver,
            } => match receiver.await {
                Ok(Ok(serial)) if serial == *expected_serial => Ok(()),
                Ok(Ok(serial)) => Err(AuthoritativeCommitFailure::Integrity(format!(
                    "commit receipt identity mismatch: expected serial {expected_serial}, received {serial}"
                ))),
                Ok(Err(detail)) => Err(AuthoritativeCommitFailure::Integrity(detail)),
                Err(_) => Err(AuthoritativeCommitFailure::Integrity(
                    "store writer dropped the exact commit receipt".to_owned(),
                )),
            },
            Self::Authoritative {
                expected_token,
                expected_hash,
                receiver,
            } => match receiver.await {
                Ok(Ok(receipt))
                    if receipt.token() == *expected_token
                        && receipt.bundle_hash() == *expected_hash =>
                {
                    Ok(())
                }
                Ok(Ok(receipt)) => Err(AuthoritativeCommitFailure::Integrity(format!(
                    "authoritative receipt identity mismatch: expected {expected_token}/{expected_hash}, received {}/{}",
                    receipt.token(),
                    receipt.bundle_hash()
                ))),
                Ok(Err(failure)) => Err(failure),
                Err(_) => Err(AuthoritativeCommitFailure::Integrity(
                    "store writer dropped the authoritative commit receipt".to_owned(),
                )),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PendingCustody {
    barriers: usize,
    reattach_records: usize,
    operational_bytes: usize,
    sends: usize,
    effects: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReattachOverload {
    records: usize,
    logical_bytes: usize,
}

struct CompletedPending {
    outcome: DeliveryOutcome,
    barriers: Vec<oneshot::Sender<Result<(), WorldReadError>>>,
    coalesced_reattach: Option<Box<[ReAttachRec]>>,
}

impl PendingCommit {
    fn retain_barrier(&mut self, reply: oneshot::Sender<Result<(), WorldReadError>>) -> bool {
        if self.barriers.len() >= MAX_PENDING_BARRIERS {
            let _ = reply.send(Err(WorldReadError::Busy));
            return false;
        }
        self.barriers.push(reply);
        true
    }

    /// Replace the prior full snapshot atomically. The public handle converts
    /// to `Box<[T]>` before enqueue, so retained allocation tracks the bounded
    /// record count rather than a transport-controlled Vec capacity.
    fn coalesce_reattach(
        &mut self,
        recs: Box<[ReAttachRec]>,
    ) -> Result<bool, (Box<[ReAttachRec]>, ReattachOverload)> {
        let records = recs.len();
        let logical_bytes = records.saturating_mul(PENDING_REATTACH_CHARGE_BYTES);
        if records > MAX_PENDING_REATTACH_RECORDS || logical_bytes > MAX_PENDING_REATTACH_BYTES {
            return Err((
                recs,
                ReattachOverload {
                    records,
                    logical_bytes,
                },
            ));
        }
        let replaced = self.coalesced_reattach.replace(recs).is_some();
        debug_assert!(self.custody().operational_bytes <= MAX_PENDING_OPERATIONAL_BYTES);
        Ok(replaced)
    }

    fn custody(&self) -> PendingCustody {
        let barriers = self.barriers.len();
        let reattach_records = self
            .coalesced_reattach
            .as_deref()
            .map_or(0, <[ReAttachRec]>::len);
        PendingCustody {
            barriers,
            reattach_records,
            operational_bytes: barriers
                .saturating_mul(PENDING_BARRIER_CHARGE_BYTES)
                .saturating_add(reattach_records.saturating_mul(PENDING_REATTACH_CHARGE_BYTES)),
            sends: self.sends.len(),
            effects: self.effects.len(),
        }
    }
}

fn publish_pending_custody(metrics: &Metrics, pending: Option<&PendingCommit>) {
    let custody = pending.map_or_else(PendingCustody::default, PendingCommit::custody);
    let as_u64 = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
    metrics.set_commit_pending_custody(
        as_u64(custody.barriers),
        as_u64(custody.reattach_records),
        as_u64(custody.operational_bytes),
        as_u64(custody.sends),
        as_u64(custody.effects),
    );
}

#[allow(clippy::too_many_lines)] // the world loop is one cohesive state machine
#[allow(clippy::too_many_arguments)] // the world loop threads its whole context
async fn run(
    mut engine: Engine,
    mut rx: mpsc::Receiver<WorldMsg>,
    tx: mpsc::WeakSender<WorldMsg>,
    writer: mpsc::Sender<StoreJob>,
    deps: ServerDeps,
    staff_ops: mpsc::Sender<StaffOp>,
    gateway: Gateway,
    high_water: Arc<AtomicU64>,
    mut signals: WriterSignalReceiver,
    mut shutdown: watch::Receiver<bool>,
    persistence_integrity: PersistenceIntegrityLatch,
    initial_mode: Option<WorldMode>,
) {
    tracing::info!("world task running");
    let boot_unix = unix_now_s();
    // The LLM service handle (built from [llm] config) that drives a module's
    // deferred async work off-loop; cloned out of `deps` so the loop keeps
    // borrowing `deps` (staff queue) freely each iteration.
    let llm = deps.llm.clone();
    let metrics = Arc::clone(&deps.metrics);
    // The server -> gateway delivery seam (M16a/M16b): every resolved
    // `(SendTarget, OutputEvent)` pair is handed to this transport. In the
    // all-in-one binary it maps onto the shared `SessionManager`; in the forked
    // server it serializes each pair as a `ToGateway` frame onto the boundary
    // socket (passed in by `spawn_persist_with_gateway`). `resolve_sends` (the
    // World-reading half) stays server-side either way.
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    let mut serial: u64 = 0;
    let mut health = StoreHealth::new();
    if let Some(mode) = initial_mode {
        if mode.integrity_frozen() {
            persistence_integrity.latch();
        }
        health.mode = mode;
    }
    // E0 restart-safe poison: an unresolved quarantine record in the spool means
    // an authoritative-ledger breach was NOT repaired before this restart. Start
    // FROZEN — a poison freeze never auto-resumes merely because the process came
    // back up (staff repair the ledger + empty the spool, then restart).
    let unresolved_quarantine = deps.quarantine_spool.opened_state().unresolved_records;
    if unresolved_quarantine > 0 {
        persistence_integrity.latch();
        health.mode = WorldMode::IntegrityFrozen {
            incident: incident_id("boot-quarantine", &unresolved_quarantine.to_string()),
        };
        tracing::error!(
            unresolved = unresolved_quarantine,
            "BOOT FROZEN: unresolved ledger-quarantine record(s) in the spool; the world starts \
             READ-ONLY until staff repair the ledger and empty the quarantine spool"
        );
    }
    metrics.set_persistence_state(
        health.mode.store_read_only(),
        health.mode.integrity_frozen(),
    );
    // The E0 retained-write table: unacked sealed ledger chunks, re-emitted on
    // every flush until the commit high-water reaches them. See `crate::retained`.
    let mut retained = RetainedSeals::new();
    // At most one authoritative mutation batch may be in flight. Its
    // commit-bound presentation/effects are held here. Receipt/order/status
    // traffic and writer recovery remain live, but gameplay/world reads are
    // refused with a typed Busy response: RAM already contains the tentative
    // post-image, so it is not an authoritative query snapshot yet.
    let mut pending_commit: Option<PendingCommit> = None;
    // If a must-retain async completion or connection-lifecycle message is
    // already dequeued while another exact receipt is outstanding, retain that
    // ONE message locally and stop polling `rx`. This preserves FIFO without
    // inventing a second unbounded input queue; the bounded channel supplies
    // backpressure until the commit resolves. Ordinary Cmd/CreatePlayer/Dump
    // work is refused immediately and never occupies this slot. Operational
    // traffic behind a retained Apply/lifecycle message is still head-of-line
    // blocked: the separately bounded semantic/lifecycle lane remains K2 work.
    let mut deferred_world_msg: Option<WorldMsg> = None;
    // Store-backed staff operations drained beside a local operation remain
    // ordered behind any commit that local operation starts.
    let mut deferred_staff_ops: Vec<StaffOp> = Vec::new();
    // Whether the writer's control mailbox is still open (disabled once the
    // writer is gone and its final latched facts have been drained, so the
    // closed wake channel does not busy-loop the select).
    let mut signals_open = true;
    let mut inbound_open = true;
    // Rate-limit the retained-depth alarm so a genuinely wedged store logs once
    // per crossing, not every re-emit tick.
    let mut retained_alarmed = false;
    // Throttle the world-owned metrics sample (object scan, NPC/LLM
    // aggregation) so it never rides the hot path more than once a second.
    // `None` forces a sample on the first iteration.
    let mut last_sample: Option<Instant> = None;
    // Top-level shutdown has a priority watch lane, so it cannot be trapped
    // behind the bounded world queue while a commit holds one deferred message.
    let mut shutdown_requested = false;
    let mut shutdown_deadline: Option<Instant> = None;
    loop {
        let admission_mode = match (&pending_commit, health.mode) {
            // IntegrityFrozen is stricter than every in-flight state and may
            // never be hidden. A transient StoreReadOnly transition is not:
            // while an exact predecessor receipt remains unresolved,
            // CommitPending must continue to reject every potentially mutating
            // command/input-context line. Otherwise later RAM mutations and
            // success output could overtake the missing receipt.
            (_, mode @ WorldMode::IntegrityFrozen { .. }) => mode,
            (Some(pending), _) => WorldMode::CommitPending {
                token: pending.token,
            },
            (None, mode) => mode,
        };
        let next_due = engine.sched.next_due();
        // Arm the E0 re-emit nudge while any sealed chunk is unacked, so an idle
        // world still re-flushes a dropped seal. A bare `Timer` wake just runs
        // `fire_due` (a no-op when nothing is scheduled) and falls through to the
        // foot-of-loop flush, which does the actual re-emit + ack.
        let reemit_pending = pending_commit.is_none() && !retained.is_empty();
        // Under an authoritative-ledger freeze, do NOT arm the scheduler timer:
        // its due events would otherwise wake the loop immediately (a past due
        // fires instantly) and, since `fire_due` is skipped while frozen, spin.
        // Persistence/re-emit stays live (the re-emit nudge + inbound still run).
        let sched_armed = next_due.is_some()
            && admission_for(admission_mode, IngressClass::Timer) == Admission::Allow;
        let ready_deferred = if pending_commit.is_none() {
            deferred_world_msg.take()
        } else {
            None
        };
        let wake = if let Some(msg) = ready_deferred {
            Wake::Msg(msg)
        } else {
            tokio::select! {
            m = rx.recv(), if inbound_open && deferred_world_msg.is_none() => {
                m.map_or(Wake::Closed, Wake::Msg)
            },
            // Writer control facts ride their own bounded/coalescing mailbox
            // (see `WriterSignal`). `None` means the writer is gone and every
            // final latched fact was drained; disable this arm (avoiding a
            // busy-loop) rather than closing the world — the world's lifetime
            // is driven by its inbound channel, not this one.
            s = signals.recv(), if signals_open => match s {
                Some(sig) => Wake::Signal(sig),
                None => { signals_open = false; Wake::Timer }
            },
            ack = async {
                match pending_commit.as_mut() {
                    Some(pending) => pending.receipt.receive().await,
                    None => std::future::pending().await,
                }
            } => Wake::CommitAck(ack),
            () = sleep_until_unix_ms(next_due), if sched_armed => Wake::Timer,
            () = tokio::time::sleep(REEMIT_INTERVAL), if reemit_pending => Wake::Timer,
            changed = shutdown.changed(), if !shutdown_requested => {
                if changed.is_ok() && *shutdown.borrow_and_update() {
                    Wake::Shutdown
                } else {
                    Wake::Closed
                }
            },
            () = async {
                match shutdown_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                    None => std::future::pending().await,
                }
            } => Wake::ShutdownDeadline,
            }
        };
        let iteration_start = Instant::now();
        let now_unix = unix_now_s();
        // CommitPending freezes guard-producing truth; boot/recovery and an
        // integrity freeze likewise do not advance the world's logical clock.
        // Transient StoreReadOnly retains the historical wall-clock behavior.
        if matches!(
            admission_mode,
            WorldMode::Writable | WorldMode::StoreReadOnly { .. }
        ) {
            engine.set_now(now_unix);
        }
        // Liveness heartbeat (PLAN "Observability"): stamp the loop's most
        // recent iteration start every iteration, before the (possibly slow)
        // step. A loop wedged inside a sync handler stops advancing this while
        // `loop_lag_last_ms` freezes low — staleness of this gauge is what a
        // scraper alerts on to tell "wedged" from "fast".
        metrics.set_world_loop_iteration(now_unix);
        // Whose flood-exempt flag to re-sync after the step (the acting
        // player of a Cmd — the only path that pushes/pops input contexts).
        let mut flood_sync: Option<ObjId> = None;
        let mut create_completion: Option<CreateCompletion> = None;
        let mut failure_actor: Option<ObjId> = None;
        let outcome = match wake {
            Wake::Msg(msg) => {
                if matches!(&msg, WorldMsg::Inbound(Inbound::Cmd { .. })) {
                    metrics.command_processed();
                }
                match admit_world_msg(admission_mode, &engine.world, &msg) {
                    Admission::Refuse(reason) => {
                        if reason == RefusalReason::CommitPending && must_defer_behind_commit(&msg)
                        {
                            debug_assert!(deferred_world_msg.is_none());
                            deferred_world_msg = Some(msg);
                        } else {
                            refuse_world_msg(msg, reason, &gateway);
                        }
                        StepOutcome::Continue
                    }
                    Admission::CheckStoreReadOnlyCommand => {
                        unreachable!("admit_world_msg resolves store-read-only commands")
                    }
                    Admission::Allow => match msg {
                        WorldMsg::Inbound(msg) => {
                            if let Inbound::Cmd { actor, .. } = &msg {
                                flood_sync = Some(*actor);
                                failure_actor = Some(*actor);
                            }
                            if matches!(&msg, Inbound::Control(ControlMsg::Shutdown))
                                && pending_commit.is_some()
                            {
                                // Shutdown remains admissible while a commit is in
                                // flight, but its global notice must not overtake
                                // the earlier commit-bound presentation.
                                debug_assert!(deferred_world_msg.is_none());
                                deferred_world_msg = Some(WorldMsg::Inbound(msg));
                                StepOutcome::Continue
                            } else {
                                engine.step(msg, &mut out, &mut fx)
                            }
                        }
                        WorldMsg::ReAttach { recs } => {
                            if let Some(pending) = pending_commit.as_mut() {
                                // ReAnnounce is a full snapshot, so an older
                                // snapshot has no independent meaning. Applying
                                // each snapshot here would append another room
                                // render to `pending.sends` while the receipt is
                                // hung. Retain one bounded latest snapshot and
                                // apply it immediately after receipt resolution.
                                match pending.coalesce_reattach(recs) {
                                    Ok(replaced) => {
                                        if replaced {
                                            metrics.commit_pending_reattach_coalesced();
                                        }
                                    }
                                    Err((_recs, overload)) => {
                                        // `WorldHandle::reattach` and the split
                                        // boundary both reject this atomically
                                        // before enqueue. Reaching this arm is
                                        // therefore an internal invariant breach;
                                        // importantly, no prefix is applied.
                                        metrics.commit_pending_reattach_rejected();
                                        tracing::error!(
                                            records = overload.records,
                                            logical_bytes = overload.logical_bytes,
                                            max_records = MAX_PENDING_REATTACH_RECORDS,
                                            max_bytes = MAX_PENDING_REATTACH_BYTES,
                                            "internal oversized reattach snapshot refused atomically"
                                        );
                                    }
                                }
                            } else {
                                // M16c silent re-attach: rebuild every held
                                // connection's world-side state without a
                                // connect announce/hook/event, then push each
                                // session a fresh room render.
                                apply_reattach_snapshot(&mut engine, &recs, &mut out);
                                tracing::info!(
                                    count = recs.len(),
                                    "re-attached held connections after restart"
                                );
                            }
                            StepOutcome::Continue
                        }
                        WorldMsg::CreatePlayer { name, reply } => {
                            match create_player(&mut engine, &name) {
                                Ok(id) => {
                                    // Tell module observers a Player now exists
                                    // (comsys auto-joins the default channel here).
                                    // Emitted BEFORE the flush so whatever the
                                    // observers persist rides the same durable
                                    // creation batch.
                                    engine.emit_world_event(
                                        &WorldEvent::ObjCreated {
                                            obj: id,
                                            kind: ObjKind::Player,
                                            creator: mush_core::GOD,
                                        },
                                        &mut out,
                                        &mut fx,
                                    );
                                    // The common post-step commit path below owns
                                    // the exact receipt and completes this typed
                                    // reply only after releasing committed output.
                                    create_completion = Some(CreateCompletion { id, reply });
                                }
                                Err(e) => {
                                    // A dropped receiver: the requester gave up.
                                    let _ = reply.send(Err(e));
                                }
                            }
                            StepOutcome::Continue
                        }
                        WorldMsg::NameFree { name, reply } => {
                            let _ = reply.send(Ok(engine.world.resolve_player(&name).is_none()));
                            StepOutcome::Continue
                        }
                        WorldMsg::InInputContext { actor, reply } => {
                            let _ = reply.send(Ok(engine.world.input_depth(actor) > 0));
                            StepOutcome::Continue
                        }
                        WorldMsg::ConnectionReady { conn, actor, reply } => {
                            let _ = reply.send(Ok(engine.actor_of(conn) == Some(actor)));
                            StepOutcome::Continue
                        }
                        WorldMsg::Barrier { reply } => {
                            if let Some(pending) = pending_commit.as_mut() {
                                // Processing is not delivery: keep the barrier
                                // behind the prior exact receipt and held output.
                                if !pending.retain_barrier(reply) {
                                    metrics.commit_pending_barrier_refused();
                                    tracing::warn!(
                                        cap = MAX_PENDING_BARRIERS,
                                        "commit-pending barrier waiter cap reached; refusing waiter"
                                    );
                                }
                            } else {
                                let _ = reply.send(Ok(()));
                            }
                            StepOutcome::Continue
                        }
                    },
                }
            }
            Wake::Signal(signal) => {
                debug_assert_eq!(
                    classify_writer_signal(&signal),
                    IngressClass::WriterRecovery
                );
                match signal {
                    WriterSignal::StoreStatus { healthy, detail } => {
                        apply_store_status(&engine.world, &gateway, &mut health, healthy, &detail);
                    }
                    WriterSignal::LedgerBreach { detail, seal } => {
                        // Resolve a specific seal the writer isolated: a confirmed poison
                        // is quarantined (stop re-emitting it — it can never commit and
                        // is preserved in the spool); a collateral seal healed in
                        // isolation is simply dropped from re-emit (already durable).
                        if let Some((ns, key, payload_hash, quarantined)) = seal {
                            let matched = if quarantined {
                                retained.quarantine(ns.clone(), key.clone(), payload_hash.clone())
                            } else {
                                retained.healed(ns.clone(), key.clone(), &payload_hash)
                            };
                            if !matched {
                                let mismatch = format!(
                                    "writer resolution did not match the exact retained seal {ns}/{key} ({payload_hash})"
                                );
                                tracing::error!(
                                    %ns,
                                    %key,
                                    %payload_hash,
                                    "refusing to clear a retained seal for an unmatched writer resolution"
                                );
                                apply_integrity_freeze(
                                    &engine.world,
                                    &gateway,
                                    &mut health,
                                    &persistence_integrity,
                                    &mismatch,
                                );
                            }
                        }
                        apply_integrity_freeze(
                            &engine.world,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            &detail,
                        );
                    }
                    WriterSignal::DeterministicFailure { detail } => {
                        apply_integrity_freeze(
                            &engine.world,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            &detail,
                        );
                    }
                }
                StepOutcome::Continue
            }
            Wake::CommitAck(ack) => {
                let pending = pending_commit
                    .take()
                    .expect("commit receipt cannot resolve without a pending commit");
                let exit_after_ack = pending.exit_after_ack;
                let completed = complete_pending_commit(
                    pending,
                    ack,
                    &mut engine,
                    &gateway,
                    &mut health,
                    &persistence_integrity,
                    &tx,
                    llm.as_ref(),
                );
                finish_pending_operational(&mut engine, &gateway, completed);
                if exit_after_ack {
                    StepOutcome::Shutdown
                } else {
                    StepOutcome::Continue
                }
            }
            Wake::Timer => {
                // A poison freeze is a hard mutation barrier for scheduled work:
                // do not fire due timers (scheduled sim, NPC cadence, @wait) — the
                // ledger is holed and mutating further would compound it. A Timer
                // wake here is then only the re-emit nudge, which drives the
                // foot-of-loop persistence flush (re-emit stays live while frozen).
                if admission_for(admission_mode, IngressClass::Timer) == Admission::Allow {
                    engine.fire_due(unix_now_ms(), &mut out, &mut fx);
                }
                StepOutcome::Continue
            }
            Wake::Shutdown => {
                shutdown_requested = true;
                if pending_commit.is_some() {
                    // Supersede the one already-dequeued ordinary message and
                    // stop polling the bounded queue. The shutdown request did
                    // not need queue capacity, and the prior exact commit gets
                    // a bounded grace period to finish normally.
                    if let Some(blocked) = deferred_world_msg.take() {
                        refuse_world_msg(blocked, RefusalReason::CommitPending, &gateway);
                    }
                    deferred_world_msg =
                        Some(WorldMsg::Inbound(Inbound::Control(ControlMsg::Shutdown)));
                    shutdown_deadline = Some(Instant::now() + Duration::from_secs(15));
                    StepOutcome::Continue
                } else {
                    engine.step(Inbound::Control(ControlMsg::Shutdown), &mut out, &mut fx)
                }
            }
            Wake::ShutdownDeadline => {
                let detail = "shutdown deadline expired before the exact commit receipt";
                let retained_ops = retained.reemit_ops();
                persist::preserve_unqueued_seals(
                    &deps.quarantine_spool,
                    &retained_ops,
                    serial,
                    detail,
                )
                .await;
                if let Some(pending) = pending_commit.take() {
                    let completed = complete_pending_commit(
                        pending,
                        Err(AuthoritativeCommitFailure::Integrity(detail.to_owned())),
                        &mut engine,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                    finish_pending_operational(&mut engine, &gateway, completed);
                }
                metrics.set_persistence_state(
                    health.mode.store_read_only(),
                    health.mode.integrity_frozen(),
                );
                publish_pending_custody(&metrics, None);
                tracing::error!(
                    "world task stopped after bounded shutdown grace; exact retained seals were preserved before exit"
                );
                break;
            }
            Wake::Closed => {
                inbound_open = false;
                shutdown_requested = true;
                if let Some(pending) = pending_commit.as_mut() {
                    pending.exit_after_ack = true;
                    shutdown_deadline = Some(Instant::now() + Duration::from_secs(15));
                    tracing::warn!(
                        "world inbound channel closed with a commit pending; waiting a bounded grace period for exact custody"
                    );
                    StepOutcome::Continue
                } else {
                    tracing::info!("world inbound channel closed; world task exiting");
                    break;
                }
            }
        };
        // Flood-exempt sync: while the acting player sits in an input
        // context their lines bypass the transport throttle (a pasted mail
        // body must not lose lines 31+ to the burst cap).
        if let Some(actor) = flood_sync {
            // ToGateway::SetFloodExempt — driven across the seam rather than
            // mutating the gateway's throttle directly (M16b: serialized).
            gateway.set_flood_exempt(u64::from(actor.0), engine.world.input_depth(actor) > 0);
        }
        // Every world step uses the same typed delivery boundary. A clean
        // read-only step releases immediately; a dirty step always requests
        // an exact receipt and holds commit-bound output/effects until it
        // resolves. `DurableGate` supplies the preferred failure recipient;
        // its strict bit is subsumed by this boundary because no authoritative
        // success output is released after any failed write.
        if let Some((gate_actor, _strict)) = engine.take_durable_gate() {
            failure_actor = Some(gate_actor);
        }
        let mut classified_sends =
            resolve_classified_sends(&engine.registry, &engine.world, &mut out);
        let mut classified_effects = fx.drain_classified();
        if let Some(pending) = pending_commit.as_mut() {
            debug_assert!(create_completion.is_none());
            // CommitPending always has precedence over transient read-only
            // status. Only explicitly receipt-independent presentation may
            // pass; no command/hook output is inferred clean merely from a
            // store-health mode or command allowlist.
            release_independent_while_pending(
                &gateway,
                &mut classified_sends,
                &mut classified_effects,
                health.mode,
                &tx,
                llm.as_ref(),
            );
            pending.sends.extend(classified_sends);
            pending.effects.extend(classified_effects);
        } else {
            match flush_engine_step(
                &mut engine,
                &writer,
                &mut serial,
                &mut health,
                &metrics,
                &mut retained,
                &high_water,
                &deps.quarantine_spool,
                &persistence_integrity,
            )
            .await
            {
                FlushOutcome::Clean => {
                    if create_completion.is_some() {
                        // Creating a player necessarily dirties the world. A
                        // clean result is an invariant breach, never success.
                        abort_without_receipt(
                            classified_sends,
                            classified_effects,
                            failure_actor,
                            create_completion.take(),
                            "player creation produced no authoritative write",
                            &engine.world,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            &tx,
                            llm.as_ref(),
                        );
                    } else {
                        release_classified_sends(
                            &gateway,
                            classified_sends,
                            DeliveryOutcome::NoWrite,
                        );
                        release_classified_effects(
                            health.mode,
                            classified_effects,
                            DeliveryOutcome::NoWrite,
                            &tx,
                            llm.as_ref(),
                        );
                    }
                }
                FlushOutcome::Queued {
                    receipt: Some(receipt),
                    prepared_installation,
                } => {
                    release_best_effort_before_ack(
                        &gateway,
                        &mut classified_sends,
                        &mut classified_effects,
                        health.mode,
                        &tx,
                        llm.as_ref(),
                    );
                    pending_commit = Some(PendingCommit {
                        token: receipt.token(),
                        receipt,
                        sends: classified_sends,
                        effects: classified_effects,
                        failure_actor,
                        create: create_completion.take(),
                        prepared_installation,
                        barriers: Vec::new(),
                        coalesced_reattach: None,
                        exit_after_ack: outcome == StepOutcome::Shutdown,
                    });
                }
                FlushOutcome::Queued {
                    receipt: None,
                    prepared_installation,
                } => {
                    if let Some(installation) = prepared_installation {
                        engine.abort_prepared_action(installation);
                    }
                    abort_without_receipt(
                        classified_sends,
                        classified_effects,
                        failure_actor,
                        create_completion.take(),
                        "durable writer accepted a batch without returning its receipt",
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                }
                FlushOutcome::Refused(detail) => {
                    refuse_before_submission(
                        classified_sends,
                        classified_effects,
                        failure_actor,
                        create_completion.take(),
                        &detail,
                        &gateway,
                        &health,
                        &tx,
                        llm.as_ref(),
                    );
                }
                FlushOutcome::Rejected(detail) => {
                    abort_without_receipt(
                        classified_sends,
                        classified_effects,
                        failure_actor,
                        create_completion.take(),
                        &detail,
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                }
                FlushOutcome::WriterGone => {
                    abort_without_receipt(
                        classified_sends,
                        classified_effects,
                        failure_actor,
                        create_completion.take(),
                        "store writer disappeared before accepting the authoritative batch",
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                }
            }
        }
        // Staff ops the step's handlers queued. Local ops (@boot/@ps/@stats)
        // run inline here — the engine, scheduler, and session manager are
        // all at hand — AFTER the delivery above, so a @boot target's
        // Disconnect message reaches their connections before the session
        // is closed. Store ops go to the async executors, as in M2.
        let (local_ops, store_ops): (Vec<StaffOp>, Vec<StaffOp>) = if pending_commit.is_none() {
            admit_staff_ops(health.mode, deps.staff.drain(), &gateway)
                .into_iter()
                .partition(|op| op.kind.is_local())
        } else {
            (Vec::new(), Vec::new())
        };
        if !local_ops.is_empty() {
            let local_failure_actor = local_ops.first().map(|op| op.actor);
            run_local_staff_ops(
                &mut engine,
                &gateway,
                &health,
                boot_unix,
                local_ops,
                &mut out,
                &mut fx,
                llm.as_ref(),
                &metrics,
            );
            let mut sends = resolve_classified_sends(&engine.registry, &engine.world, &mut out);
            let mut effects = fx.drain_classified();
            match flush_dirty(
                &mut engine,
                &writer,
                &mut serial,
                true,
                &mut health,
                &metrics,
                &mut retained,
                &high_water,
                &deps.quarantine_spool,
                &persistence_integrity,
            )
            .await
            {
                FlushOutcome::Clean => {
                    release_classified_sends(&gateway, sends, DeliveryOutcome::NoWrite);
                    release_classified_effects(
                        health.mode,
                        effects,
                        DeliveryOutcome::NoWrite,
                        &tx,
                        llm.as_ref(),
                    );
                }
                FlushOutcome::Queued {
                    receipt: Some(receipt),
                    prepared_installation,
                } => {
                    release_best_effort_before_ack(
                        &gateway,
                        &mut sends,
                        &mut effects,
                        health.mode,
                        &tx,
                        llm.as_ref(),
                    );
                    pending_commit = Some(PendingCommit {
                        token: receipt.token(),
                        receipt,
                        sends,
                        effects,
                        failure_actor: local_failure_actor,
                        create: None,
                        prepared_installation,
                        barriers: Vec::new(),
                        coalesced_reattach: None,
                        exit_after_ack: false,
                    });
                }
                FlushOutcome::Queued {
                    receipt: None,
                    prepared_installation,
                } => {
                    if let Some(installation) = prepared_installation {
                        engine.abort_prepared_action(installation);
                    }
                    abort_without_receipt(
                        sends,
                        effects,
                        local_failure_actor,
                        None,
                        "durable writer accepted a staff batch without returning its receipt",
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                }
                FlushOutcome::Refused(detail) => refuse_before_submission(
                    sends,
                    effects,
                    local_failure_actor,
                    None,
                    &detail,
                    &gateway,
                    &health,
                    &tx,
                    llm.as_ref(),
                ),
                FlushOutcome::Rejected(detail) => abort_without_receipt(
                    sends,
                    effects,
                    local_failure_actor,
                    None,
                    &detail,
                    &engine.world,
                    &gateway,
                    &mut health,
                    &persistence_integrity,
                    &tx,
                    llm.as_ref(),
                ),
                FlushOutcome::WriterGone => abort_without_receipt(
                    sends,
                    effects,
                    local_failure_actor,
                    None,
                    "store writer disappeared before accepting a staff batch",
                    &engine.world,
                    &gateway,
                    &mut health,
                    &persistence_integrity,
                    &tx,
                    llm.as_ref(),
                ),
            }
        }
        deferred_staff_ops.extend(store_ops);
        let staff_shutdown = if pending_commit.is_none() {
            run_staff_ops(
                &deps,
                &gateway,
                &writer,
                &staff_ops,
                std::mem::take(&mut deferred_staff_ops),
            )
            .await
        } else {
            false
        };
        // Loop-lag watchdog (PLAN risk 2): the single-writer loop must stay
        // fast; a slow iteration means a slow sync handler or writer-queue
        // backpressure. The engine mirrors last/max for `@stats`; the metrics
        // registry additionally feeds the p99 histogram (cheap — a few
        // atomics every iteration).
        let lag = iteration_start.elapsed();
        let lag_ms = u64::try_from(lag.as_millis()).unwrap_or(u64::MAX);
        engine.record_loop_lag(lag_ms);
        metrics.observe_loop_lag(lag_ms);
        if lag > LOOP_LAG_WARN {
            tracing::warn!(lag_ms, "world loop iteration lagged");
        }
        // E0 retained-seal depth alarm: a table that stays deep means durable
        // acks are not flowing — a wedged store, or a poison seal the writer
        // keeps dropping. Log once per crossing (not every re-emit tick).
        let retained_depth = retained.depth();
        if retained_depth > RETAINED_DEPTH_ALARM && !retained_alarmed {
            retained_alarmed = true;
            tracing::error!(
                depth = retained_depth,
                "retained sealed-chunk backlog is deep — durable ledger acks are not \
                 flowing (store wedged or a poison seal); durability is degraded"
            );
        } else if retained_depth == 0 && retained_alarmed {
            retained_alarmed = false;
            tracing::info!("retained sealed-chunk backlog cleared");
        }
        metrics.set_persistence_state(
            health.mode.store_read_only(),
            health.mode.integrity_frozen(),
        );
        publish_pending_custody(&metrics, pending_commit.as_ref());
        // Publish the world-owned gauges (throttled). Done at the end of the
        // iteration so it reflects the just-applied state, and cheaply skipped
        // when the last sample is under a second old.
        maybe_publish_sample(
            &engine,
            &gateway,
            &health,
            llm.as_ref(),
            &metrics,
            &mut last_sample,
        );
        if outcome == StepOutcome::Shutdown && pending_commit.is_none() {
            tracing::info!("world task shut down cleanly");
            break;
        }
        if staff_shutdown {
            // In-game `@shutdown`: step the same ControlMsg the console path
            // sends, so the announce-to-all and the StepOutcome are
            // identical; main's shutdown sequence (writer drain → store
            // drain → BGSAVE) runs when this task's join resolves.
            let outcome = engine.step(Inbound::Control(ControlMsg::Shutdown), &mut out, &mut fx);
            debug_assert_eq!(outcome, StepOutcome::Shutdown);
            let mut sends = resolve_classified_sends(&engine.registry, &engine.world, &mut out);
            let mut effects = fx.drain_classified();
            match flush_dirty(
                &mut engine,
                &writer,
                &mut serial,
                true,
                &mut health,
                &metrics,
                &mut retained,
                &high_water,
                &deps.quarantine_spool,
                &persistence_integrity,
            )
            .await
            {
                FlushOutcome::Clean => {
                    release_classified_sends(&gateway, sends, DeliveryOutcome::NoWrite);
                    release_classified_effects(
                        health.mode,
                        effects,
                        DeliveryOutcome::NoWrite,
                        &tx,
                        llm.as_ref(),
                    );
                    tracing::info!("world task shut down cleanly");
                    break;
                }
                FlushOutcome::Queued {
                    receipt: Some(receipt),
                    prepared_installation,
                } => {
                    release_best_effort_before_ack(
                        &gateway,
                        &mut sends,
                        &mut effects,
                        health.mode,
                        &tx,
                        llm.as_ref(),
                    );
                    pending_commit = Some(PendingCommit {
                        token: receipt.token(),
                        receipt,
                        sends,
                        effects,
                        failure_actor: None,
                        create: None,
                        prepared_installation,
                        barriers: Vec::new(),
                        coalesced_reattach: None,
                        exit_after_ack: true,
                    });
                }
                FlushOutcome::Queued {
                    receipt: None,
                    prepared_installation,
                } => {
                    if let Some(installation) = prepared_installation {
                        engine.abort_prepared_action(installation);
                    }
                    abort_without_receipt(
                        sends,
                        effects,
                        None,
                        None,
                        "shutdown's authoritative write did not receive an acknowledgement",
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                    metrics.set_persistence_state(
                        health.mode.store_read_only(),
                        health.mode.integrity_frozen(),
                    );
                    tracing::error!("world task shutting down after an unacknowledged write");
                    break;
                }
                FlushOutcome::Refused(detail) => {
                    refuse_before_submission(
                        sends,
                        effects,
                        None,
                        None,
                        &detail,
                        &gateway,
                        &health,
                        &tx,
                        llm.as_ref(),
                    );
                    tracing::warn!(%detail, "shutdown write refused before submission");
                    break;
                }
                FlushOutcome::WriterGone => {
                    abort_without_receipt(
                        sends,
                        effects,
                        None,
                        None,
                        "shutdown's authoritative write did not receive an acknowledgement",
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                    metrics.set_persistence_state(
                        health.mode.store_read_only(),
                        health.mode.integrity_frozen(),
                    );
                    tracing::error!("world task shutting down after an unacknowledged write");
                    break;
                }
                FlushOutcome::Rejected(detail) => {
                    abort_without_receipt(
                        sends,
                        effects,
                        None,
                        None,
                        &detail,
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &tx,
                        llm.as_ref(),
                    );
                    metrics.set_persistence_state(
                        health.mode.store_read_only(),
                        health.mode.integrity_frozen(),
                    );
                    tracing::error!("world task shutting down after a rejected write");
                    break;
                }
            }
        }
    }
    // Dropping `writer` here closes the job queue; the writer task drains
    // whatever is still pending and exits (awaited by the shutdown
    // sequence via `PersistHandles::writer_join`).
}

/// Complete a refused message without ever handing it to the engine. Requests
/// with typed replies receive a conservative answer instead of hanging; work
/// without a reply is dropped with a bounded diagnostic.
fn refuse_world_msg(msg: WorldMsg, reason: RefusalReason, gateway: &Gateway) {
    match msg {
        WorldMsg::Inbound(Inbound::Cmd { actor, .. })
        | WorldMsg::Inbound(Inbound::ConnUp { actor, .. }) => {
            send_sys(gateway, actor, reason.text());
        }
        WorldMsg::Inbound(Inbound::Query { conn, id, .. }) => {
            // Preserve request correlation: a web/TUI query refusal is a typed
            // Reply to this connection, never an unrelated actor Sys line.
            gateway.transmit(vec![(
                SendTarget::Conn(conn),
                Arc::new(OutputEvent::Reply {
                    id,
                    event: Box::new(OutputEvent::Sys(RichText::tag("error", reason.text()))),
                }),
            )]);
        }
        WorldMsg::Inbound(Inbound::ConnDown { actor, .. })
        | WorldMsg::Inbound(Inbound::SessionGone { actor }) => {
            tracing::warn!(%actor, ?reason, "connection hook suppressed by world mode");
        }
        WorldMsg::Inbound(Inbound::Apply(_)) => {
            tracing::warn!(?reason, "async apply suppressed by world mode");
        }
        WorldMsg::Inbound(Inbound::Control(ControlMsg::Dump)) => {
            tracing::warn!(?reason, "dump control suppressed by world mode");
        }
        WorldMsg::Inbound(Inbound::Control(ControlMsg::Shutdown)) => {
            unreachable!("shutdown is admitted in every world mode")
        }
        WorldMsg::ReAttach { .. } => {
            unreachable!("transport-only reattach is admitted in every world mode")
        }
        WorldMsg::CreatePlayer { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                CreatePlayerErr::Busy
            } else {
                CreatePlayerErr::StoreUnavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::NameFree { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                WorldReadError::Busy
            } else {
                WorldReadError::Unavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::InInputContext { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                WorldReadError::Busy
            } else {
                WorldReadError::Unavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::ConnectionReady { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                WorldReadError::Busy
            } else {
                WorldReadError::Unavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::Barrier { reply } => {
            let error = if reason == RefusalReason::CommitPending {
                WorldReadError::Busy
            } else {
                WorldReadError::Unavailable
            };
            let _ = reply.send(Err(error));
        }
    }
}

/// In store-read-only mode, decide whether an inbound message must be refused.
/// Returns `Some((actor, message))` to refuse, `None` to let it through.
///
/// Only `Cmd` lines are ever refused, and only when the verb is not on the
/// conservative non-mutating allowlist — connection lifecycle, applies, and
/// control must keep flowing. Because dispatch matches **exits before the
/// command table** (treaty ruling 5), a line that resolves to an exit from
/// the actor's position is refused even when it spells an allowlisted verb
/// (classic builds alias exits `l`, `look`, `ex`, ... — traversing one
/// moves the player, dirtying three objects). Unlisted-but-actually-
/// read-only commands are refused too (conservative by design); a mutating
/// command that slips through an allowlisted verb would still be queued for
/// the writer and retried in order, so the gate bounds divergence rather
/// than guarding correctness.
fn refuse_store_read_only(world: &World, msg: &Inbound) -> Option<(ObjId, String)> {
    let Inbound::Cmd { actor, line, .. } = msg else {
        return None;
    };
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    // An actor inside an input context (mail compose, chargen) must never
    // be trapped there: their lines are arbitrary text, not verbs — the
    // allowlist would refuse the hard escape `~q` (which dispatch promises
    // ALWAYS pops) while letting allowlisted verbs through to be silently
    // captured as body text. Context feeds mutate only RAM and queue
    // ordered module ops — exactly the bounded divergence this gate's
    // contract accepts — so they pass wholesale.
    if world.input_depth(*actor) > 0 {
        return None;
    }
    // Speech prefixes never mutate the world (and dispatch rewrites them
    // before its exit match, so they cannot traverse an exit either).
    if matches!(t.chars().next(), Some('"' | ':' | ';' | '\\')) {
        return None;
    }
    let refusal = || {
        Some((
            *actor,
            "The database is currently unavailable; commands that change the world \
             are paused. Staff have been alerted -- please try again shortly."
                .to_string(),
        ))
    };
    // Exit match wins over the command table in dispatch, so it must win
    // over the allowlist here: movement mutates.
    if find_exit(world, *actor, t).is_some() {
        return refusal();
    }
    let verb = t
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    const ALLOWED: &[&str] = &[
        "look",
        "l",
        "who",
        "help",
        "think",
        "say",
        "pose",
        "semipose",
        "@emit",
        "examine",
        "ex",
        "inventory",
        "inv",
        "i",
        // Staff must be able to stop the server while the store is down.
        "@shutdown",
    ];
    if ALLOWED.contains(&verb.as_str()) {
        return None;
    }
    refusal()
}

/// Enter/leave read-only mode on a writer health transition, alerting
/// connected Wizards either way.
fn apply_store_status(
    world: &World,
    gateway: &Gateway,
    health: &mut StoreHealth,
    healthy: bool,
    detail: &str,
) {
    if healthy {
        // A ledger-breach freeze does NOT lift on the store merely recovering —
        // it needs an explicit staff-verified resume (repair + restart). Only
        // the transient read-only mode auto-lifts here.
        if matches!(health.mode, WorldMode::StoreReadOnly { .. }) {
            health.mode = WorldMode::Writable;
            tracing::info!("store recovered; leaving read-only mode");
            broadcast_wizards(
                world,
                gateway,
                "GAME: store writes recovered; the world is writable again.",
            );
        } else if health.mode.integrity_frozen() {
            tracing::warn!(
                "store writes recovered, but the world stays read-only: an \
                 authoritative-ledger breach requires a staff-verified resume"
            );
        }
    } else if matches!(health.mode, WorldMode::Writable) {
        health.mode = WorldMode::StoreReadOnly {
            incident: incident_id("store-read-only", detail),
        };
        tracing::error!(detail, "store write failure; entering read-only mode");
        broadcast_wizards(
            world,
            gateway,
            &format!(
                "GAME: store write failure ({detail}); mutating commands are \
                 paused until the store recovers."
            ),
        );
    }
}

/// Latch the non-auto-lifting freeze on any deterministic persistence breach:
/// enter [`WorldMode::IntegrityFrozen`] so a later transient recovery cannot
/// lift it, and alert Wizards. Idempotent — the writer may send an initial
/// failure plus per-seal resolution details; only the first transition alerts.
fn apply_integrity_freeze(
    world: &World,
    gateway: &Gateway,
    health: &mut StoreHealth,
    persistence_integrity: &PersistenceIntegrityLatch,
    detail: &str,
) {
    // Publish the process-wide barrier even if the in-loop mode already says
    // frozen. Shutdown reads this atomic directly and must not depend on a
    // queued writer signal or a later world-loop iteration.
    persistence_integrity.latch();
    if health.mode.integrity_frozen() {
        return;
    }
    health.mode = WorldMode::IntegrityFrozen {
        incident: incident_id("deterministic-persistence-breach", detail),
    };
    tracing::error!(
        detail,
        "PERSISTENCE INTEGRITY BREACH: a batch was dropped by a deterministic store \
         rejection; entering read-only until a staff-verified resume (repair + restart)"
    );
    broadcast_wizards(
        world,
        gateway,
        &format!(
            "GAME: persistence integrity breach ({detail}); the world is now READ-ONLY. \
             Mutations stay paused until staff repair the ledger and restart — the store \
             recovering on its own will NOT lift this."
        ),
    );
}

/// Send a Sys line to every connected Wizard-or-better, each as a
/// [`ToGateway::Deliver`](mush_ipc::ToGateway) through the seam.
fn broadcast_wizards(world: &World, gateway: &Gateway, text: &str) {
    let ev = Arc::new(OutputEvent::Sys(RichText::tag("system", text)));
    for actor in world.connected() {
        if world.get(actor).is_some_and(|o| o.role() >= Role::Wizard) {
            gateway.deliver(SendTarget::Char(u64::from(actor.0)), Arc::clone(&ev));
        }
    }
}

/// What became of one [`flush_dirty`] call. The distinction between
/// [`Clean`](FlushOutcome::Clean) and
/// [`WriterGone`](FlushOutcome::WriterGone) matters to ack-gated callers: a
/// clean flush has nothing to persist, while a gone writer means a real
/// batch was **dropped** and must never be confirmed as durable.
enum FlushOutcome {
    /// Nothing was dirty; no batch was produced.
    Clean,
    /// A batch was handed to the writer. Carries the receipt receiver when
    /// the batch was durable.
    Queued {
        receipt: Option<PendingReceipt>,
        prepared_installation: Option<PreparedInstallation>,
    },
    /// A normal final precondition (for example a revision guard) became
    /// stale before writer custody. No persistence-integrity breach occurred.
    Refused(String),
    /// The world-side retained-write invariant rejected the candidate batch;
    /// it was never handed to the writer.
    Rejected(String),
    /// The writer's queue is closed (teardown); the batch was dropped.
    WriterGone,
}

/// Select exactly one persistence class for a gameplay step. Prepared action
/// state never passes through the legacy dirty/module/retained-seal batcher.
#[allow(clippy::too_many_arguments)]
async fn flush_engine_step(
    engine: &mut Engine,
    writer: &mpsc::Sender<StoreJob>,
    serial: &mut u64,
    health: &mut StoreHealth,
    metrics: &Metrics,
    retained: &mut RetainedSeals,
    high_water: &AtomicU64,
    quarantine_spool: &crate::quarantine::QuarantineSpool,
    persistence_integrity: &PersistenceIntegrityLatch,
) -> FlushOutcome {
    if engine.has_staged_action() {
        flush_prepared_action(
            engine, writer, serial, health, metrics, retained, high_water,
        )
        .await
    } else {
        flush_dirty(
            engine,
            writer,
            serial,
            true,
            health,
            metrics,
            retained,
            high_water,
            quarantine_spool,
            persistence_integrity,
        )
        .await
    }
}

/// Submit one already-built action-state write set and retain its typed RAM
/// images for the matching acknowledgement. K2 callers use the legacy serial
/// lane; K3 callers transfer one complete exact-token bundle to the same FIFO
/// writer and install only after its matching durable receipt.
async fn flush_prepared_action(
    engine: &mut Engine,
    writer: &mpsc::Sender<StoreJob>,
    serial: &mut u64,
    health: &mut StoreHealth,
    metrics: &Metrics,
    retained: &mut RetainedSeals,
    high_water: &AtomicU64,
) -> FlushOutcome {
    let staged: StagedAction = match engine.take_staged_action() {
        Ok(Some(staged)) => staged,
        Ok(None) => unreachable!("prepared flush selected only with a staged action"),
        Err(error) => return FlushOutcome::Refused(error.to_string()),
    };
    let (persistence, installation) = staged.into_parts();

    // Staging checks this invariant once, but later code in the same engine
    // step must not be able to add legacy dirtiness and silently move it to a
    // subsequent commit. Recheck at the last boundary before writer custody.
    if engine.has_legacy_persistence_work() {
        engine.abort_prepared_action(installation);
        return FlushOutcome::Rejected(
            "prepared action reached submission alongside legacy persistence work".to_owned(),
        );
    }

    // An older unacknowledged ledger seal is a durability predecessor. It may
    // not be folded into, or overtaken by, this state-family transaction.
    retained.ack(high_water.load(Ordering::Relaxed));
    if !retained.is_empty() {
        engine.abort_prepared_action(installation);
        return FlushOutcome::Rejected(
            "prepared action reached submission with an unresolved legacy ledger predecessor"
                .to_owned(),
        );
    }
    let receipt = match persistence {
        StagedActionPersistence::Legacy(module_ops) => {
            if *serial == u64::MAX {
                engine.abort_prepared_action(installation);
                return FlushOutcome::Rejected(
                    "authoritative writer serial exhausted; restart/recovery required".to_owned(),
                );
            }
            *serial = serial
                .checked_add(1)
                .expect("u64::MAX was rejected before prepared batch construction");
            let batch = WriteBatch {
                serial: *serial,
                module_ops,
                durable: true,
                ..WriteBatch::default()
            };
            debug_assert!(!batch.is_empty());
            let (notify, receiver) = oneshot::channel();
            if writer
                .send(StoreJob::Write {
                    batch,
                    notify: Some(notify),
                })
                .await
                .is_err()
            {
                engine.abort_prepared_action(installation);
                tracing::error!(
                    serial = *serial,
                    "store writer gone before accepting prepared action"
                );
                return FlushOutcome::WriterGone;
            }
            PendingReceipt::Legacy {
                expected_serial: *serial,
                receiver,
            }
        }
        StagedActionPersistence::Authoritative(bundle) => {
            let expected_token = bundle.token();
            let expected_hash = bundle.bundle_hash();
            let (notify, receiver) = oneshot::channel();
            if writer
                .send(StoreJob::Authoritative { bundle, notify })
                .await
                .is_err()
            {
                engine.abort_prepared_action(installation);
                tracing::error!(
                    %expected_token,
                    "store writer gone before accepting authoritative action"
                );
                return FlushOutcome::WriterGone;
            }
            PendingReceipt::Authoritative {
                expected_token,
                expected_hash,
                receiver,
            }
        }
    };
    metrics.add_store_write_queue_depth(1);
    health.last_write_unix = Some(unix_now_s());
    FlushOutcome::Queued {
        receipt: Some(receipt),
        prepared_installation: Some(installation),
    }
}

fn legacy_reserved_action_state_violation(ops: &[mush_core::ModuleOp]) -> Option<String> {
    ops.iter().find_map(|op| {
        let key = match &op.op {
            mush_core::ModuleWrite::JsonSet { key, .. }
            | mush_core::ModuleWrite::Del { key }
            | mush_core::ModuleWrite::StreamAppend { key, .. } => key,
        };
        mush_core::is_action_state_persistence_key(key).then(|| {
            format!(
                "legacy module persistence attempted coordinator-reserved key {}:{key}",
                op.ns
            )
        })
    })
}

/// Snapshot the world's dirty set into one batch and hand it to the writer,
/// bumping `serial` when a batch is actually produced.
///
/// Before batching, any dirty object marked `GOING` (an `@destroy`ed husk)
/// is [`sweep`](World::sweep)t to a tombstone so the **same batch** carries
/// its store delete — without this, destroyed objects would linger in RAM
/// and in the store forever (nothing else in the server calls `sweep`).
#[allow(clippy::too_many_arguments)] // threads the loop's persistence context
async fn flush_dirty(
    engine: &mut Engine,
    writer: &mpsc::Sender<StoreJob>,
    serial: &mut u64,
    durable: bool,
    health: &mut StoreHealth,
    metrics: &Metrics,
    retained: &mut RetainedSeals,
    high_water: &AtomicU64,
    quarantine_spool: &crate::quarantine::QuarantineSpool,
    persistence_integrity: &PersistenceIntegrityLatch,
) -> FlushOutcome {
    if *serial == u64::MAX {
        tracing::error!("authoritative writer serial exhausted; refusing to reuse a commit token");
        return FlushOutcome::Rejected(
            "authoritative writer serial exhausted; restart/recovery required".to_owned(),
        );
    }
    if let Some(detail) = legacy_reserved_action_state_violation(engine.module_ops()) {
        persistence_integrity.latch();
        tracing::error!(%detail, "authoritative batch rejected before snapshot");
        return FlushOutcome::Rejected(detail);
    }
    // Apply the durable-commit high-water first (E0), so a seal already acked
    // by the writer is not needlessly re-emitted into this batch.
    retained.ack(high_water.load(Ordering::Relaxed));

    let mut dirty = engine.world.take_dirty();
    // Module persistence ops (comsys/mail/jobs writes queued via
    // Ctx::persist) ride the SAME batch as the step's object mutations —
    // that is the write-through atomicity contract of the module API.
    let mut module_ops = engine.take_module_ops();
    // Re-emit every still-unacked sealed ledger chunk (E0). A lone re-emit is
    // enough to DRIVE a real, serial-bumped batch (the emptiness check counts
    // it), so a dropped seal is rewritten even when nothing else dirtied the
    // world — the fix for "no guaranteed subsequent batch".
    let mut reemit = retained.reemit_ops();
    if dirty.is_empty() && module_ops.is_empty() && reemit.is_empty() {
        return FlushOutcome::Clean;
    }
    // `destroy` dirties everything it marks GOING, and every flush sweeps,
    // so scanning the dirty set finds every husk — never the whole world.
    if dirty.iter().any(|&id| {
        engine
            .world
            .get(id)
            .is_some_and(|o| o.flags.contains(Flags::GOING))
    }) {
        engine.world.sweep();
        // Swept ids were re-marked dirty (as deletions); fold them in.
        dirty.append(&mut engine.world.take_dirty());
    }
    *serial = serial
        .checked_add(1)
        .expect("u64::MAX was rejected before batch construction");
    let mut batch = persist::batch_from_dirty(&engine.world, dirty, *serial, durable);
    module_ops.append(&mut reemit);
    batch.module_ops = module_ops;
    if batch.is_empty() {
        return FlushOutcome::Clean;
    }
    // Record (and re-stamp) every sealed chunk this batch carries to this
    // serial, just before handing it off — `capture` reflects "sent to the
    // writer", and the only path where the send below fails is teardown.
    if let Err(retention_error) = retained.capture(&batch.module_ops, *serial) {
        // Capture is atomic, so the table still owns every previously accepted
        // seal and none of this candidate. Latch before durable spill: the
        // quarantine sink can retry beyond shutdown's grace period, and an
        // ordinary snapshot must already be prohibited while preservation is
        // in progress.
        persistence_integrity.latch();
        tracing::error!(
            error = %retention_error,
            serial = *serial,
            retained_seals = retained.custody_depth(),
            retained_bytes = retained.custody_bytes(),
            "authoritative batch rejected before enqueue: retained-seal custody invariant"
        );
        let detail = retention_error.to_string();
        persist::preserve_unqueued_seals(quarantine_spool, &batch.module_ops, *serial, &detail)
            .await;
        return FlushOutcome::Rejected(detail);
    }
    let (notify, receipt) = if durable {
        let (tx, rx) = oneshot::channel();
        (
            Some(tx),
            Some(PendingReceipt::Legacy {
                expected_serial: *serial,
                receiver: rx,
            }),
        )
    } else {
        (None, None)
    };
    if let Err(send_error) = writer.send(StoreJob::Write { batch, notify }).await {
        let StoreJob::Write { batch, .. } = send_error.0 else {
            unreachable!("flush_dirty sends only StoreJob::Write")
        };
        tracing::error!(
            serial = *serial,
            "store writer gone; batch dropped (teardown)"
        );
        persist::preserve_unqueued_seals(
            quarantine_spool,
            &batch.module_ops,
            *serial,
            "store writer queue closed before accepting authoritative batch",
        )
        .await;
        return FlushOutcome::WriterGone;
    }
    // The writer decrements this as it drains each batch; together they track
    // the store write-queue depth for `/metrics` and `@stats`.
    metrics.add_store_write_queue_depth(1);
    health.last_write_unix = Some(unix_now_s());
    FlushOutcome::Queued {
        receipt,
        prepared_installation: None,
    }
}

/// Handle the **local** staff ops (`@boot`, `@ps`, `@stats`) inline in the
/// world loop: they need the `Engine` (scheduler, world, loop-lag mirror)
/// and the session manager, and never touch the store. Output rides the
/// normal outbox; the caller delivers it right after.
fn admit_staff_ops(mode: WorldMode, ops: Vec<StaffOp>, gateway: &Gateway) -> Vec<StaffOp> {
    let mut admitted = Vec::with_capacity(ops.len());
    for op in ops {
        match admission_for(mode, classify_staff_op(&op.kind)) {
            Admission::Allow => admitted.push(op),
            Admission::Refuse(reason) => {
                tracing::warn!(actor = %op.actor, ?reason, "staff operation suppressed by world mode");
                send_sys(gateway, op.actor, reason.text());
            }
            Admission::CheckStoreReadOnlyCommand => {
                unreachable!("typed staff operations never use command-text classification")
            }
        }
    }
    admitted
}

#[allow(clippy::too_many_arguments)]
fn run_local_staff_ops(
    engine: &mut Engine,
    gateway: &Gateway,
    health: &StoreHealth,
    boot_unix: i64,
    ops: Vec<StaffOp>,
    out: &mut Outbox,
    fx: &mut Effects,
    llm: Option<&Arc<LlmService>>,
    metrics: &Metrics,
) {
    for op in ops {
        let actor = op.actor;
        match op.kind {
            StaffOpKind::Boot {
                target,
                ref message,
            } => {
                tracing::info!(%actor, %target, message, "@boot requested");
                let name = engine
                    .world
                    .get(target)
                    .map_or_else(|| target.to_string(), |o| o.name.clone());
                // Split mode (M16b): the forked server holds no `SessionManager`,
                // so it cannot walk `char`'s connections. Emit a fire-and-forget
                // `ToGateway::CloseSession` — the gateway resolves the char to
                // its live connections, drops them, and reports each resulting
                // `ConnDown(Boot)` back as inbound, which the world then steps
                // exactly as the in-process loop below does. The world loop
                // never blocks on a reply (D4). The target's Disconnect message
                // was already delivered as a normal `Deliver` frame ahead of
                // this, so socket FIFO ordering preserves "message then close".
                if gateway.is_socket() {
                    gateway.close_session(u64::from(target.0), mush_ipc::DetachCause::Booted);
                    out.info(actor, format!("@boot: {name} booted."));
                    continue;
                }
                // Close the whole session: every connection handle comes
                // back so each ConnDown(Boot) can be fed to the engine
                // (announce + world-side disconnect on the last one).
                // The handler's Disconnect event was already delivered, so
                // the target saw the message; dropping the handles closes
                // the transports once their queues drain.
                //
                // M16b (unrouted boundary crossing): `session_of_char` +
                // `close_session` read and mutate gateway-owned session state.
                // This becomes a `ToGateway::CloseSession{char,
                // DetachCause::Booted}`; the gateway closes the transports and
                // reports each resulting `ConnDown(Boot)` back as a
                // `ToServer::Inbound`, which the world steps exactly as the
                // loop below does. Kept direct in M16a (shared manager →
                // behaviour-identical).
                let mut booted = 0u32;
                for conn in gateway.close_char_connections(u64::from(target.0)) {
                    booted += 1;
                    engine.step(
                        Inbound::ConnDown {
                            conn,
                            actor: target,
                            reason: DisconnectKind::Boot,
                        },
                        out,
                        fx,
                    );
                }
                if booted == 0 {
                    // No live connections (linkdead, or a test world with
                    // no session layer attachment): force the world-side
                    // disconnect so WHO and the room agree.
                    engine.step(Inbound::SessionGone { actor: target }, out, fx);
                    out.info(
                        actor,
                        format!("@boot: {name} had no live connections; session cleared."),
                    );
                } else {
                    let plural = if booted == 1 {
                        "connection"
                    } else {
                        "connections"
                    };
                    out.info(actor, format!("@boot: {name} booted ({booted} {plural})."));
                }
            }
            StaffOpKind::Ps => {
                tracing::info!(%actor, "@ps requested");
                out.info(actor, render_ps(engine, unix_now_ms()));
            }
            StaffOpKind::Stats => {
                tracing::info!(%actor, "@stats requested");
                let extra = stats_extra(metrics, llm);
                out.info(
                    actor,
                    render_stats(engine, health, boot_unix, unix_now_s(), &extra),
                );
            }
            // Store ops never reach here (see the partition in `run`).
            _ => {}
        }
    }
}

/// Truncate a command line for `@ps` display.
fn preview(line: &str) -> String {
    const MAX: usize = 40;
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        let cut: String = line.chars().take(MAX).collect();
        format!("{cut}...")
    }
}

/// Human form of a tick cadence: `coarse 300s`, `fast 250ms`, `scene`.
fn cadence_text(c: TickCadence) -> String {
    match c {
        TickCadence::Coarse(d) => format!("coarse {}s", d.as_secs().max(1)),
        TickCadence::Fast(d) => format!("fast {}ms", d.as_millis().max(1)),
        TickCadence::Scene => "scene".to_string(),
    }
}

/// Render the `@ps` scheduler report: pending `@wait`s (actor, seconds
/// remaining, command preview), module ticks (tag, cadence, next fire),
/// scene ticks, and totals. Pure over the engine — unit-testable.
#[must_use]
pub fn render_ps(engine: &Engine, now_ms: i64) -> String {
    let snapshot = engine.sched.pending_entries();
    let entries = snapshot.entries();
    let mut next_fire: std::collections::HashMap<(String, String), i64> =
        std::collections::HashMap::new();
    let mut waits = Vec::new();
    let mut scenes = Vec::new();
    let mut control = 0usize;
    for &(due, kind) in entries {
        let remain_s = (due - now_ms).max(0) / 1000;
        match kind {
            ScheduledKind::WaitCmd { actor, line } => {
                let name = engine
                    .world
                    .get(*actor)
                    .map_or_else(|| actor.to_string(), |o| o.name.clone());
                waits.push(format!("  {name}  in {remain_s}s: {}", preview(line)));
            }
            ScheduledKind::ModuleTick { module, tag } => {
                next_fire.insert((module.to_string(), tag.to_string()), remain_s);
            }
            ScheduledKind::SceneTick { module, tag, scene } => {
                scenes.push(format!("  {module}:{tag}  {scene}  next in {remain_s}s"));
            }
            ScheduledKind::Control(_) => control += 1,
        }
    }
    let mut ticks = Vec::new();
    for t in engine.registry.module_ticks() {
        let when = next_fire
            .get(&(t.module.to_string(), t.tag.to_string()))
            .map_or_else(String::new, |s| format!("  next in {s}s"));
        ticks.push(format!(
            "  {}:{}  {}{when}",
            t.module,
            t.tag,
            cadence_text(t.cadence)
        ));
    }
    let mut text = String::from("Scheduler:");
    text.push_str(&format!("\nWaits: {}", waits.len()));
    for w in &waits {
        text.push('\n');
        text.push_str(w);
    }
    text.push_str(&format!("\nModule ticks: {}", ticks.len()));
    for t in &ticks {
        text.push('\n');
        text.push_str(t);
    }
    text.push_str(&format!("\nScene ticks: {}", scenes.len()));
    for s in &scenes {
        text.push('\n');
        text.push_str(s);
    }
    if control > 0 {
        text.push_str(&format!("\nControl timers: {control}"));
    }
    text.push_str(&format!("\nTotal scheduled entries: {}", snapshot.total()));
    text.push_str(&format!(
        "\nDisplayed earliest entries: {} ({} omitted)",
        entries.len(),
        snapshot.omitted()
    ));
    text
}

/// Object counts by kind, plus a count of husks pending destruction.
struct ObjectCounts {
    rooms: usize,
    exits: usize,
    things: usize,
    players: usize,
    npcs: usize,
    going: usize,
}

impl ObjectCounts {
    fn total(&self) -> usize {
        self.rooms + self.exits + self.things + self.players + self.npcs
    }
}

/// Scan the world once for object counts by kind (GOING husks excluded from
/// the kind tallies, counted separately). Shared by `@stats` and the metrics
/// sample.
fn object_counts(w: &World) -> ObjectCounts {
    let mut c = ObjectCounts {
        rooms: 0,
        exits: 0,
        things: 0,
        players: 0,
        npcs: 0,
        going: 0,
    };
    for o in w.iter() {
        if o.flags.contains(Flags::GOING) {
            c.going += 1;
            continue;
        }
        match o.kind {
            ObjKind::Room => c.rooms += 1,
            ObjKind::Exit => c.exits += 1,
            ObjKind::Thing => c.things += 1,
            ObjKind::Player => c.players += 1,
            ObjKind::Npc => c.npcs += 1,
        }
    }
    c
}

/// NPC agent runtime counts read from the module state slot: `active` = NPCs
/// with a live runtime (stimulated at least once this boot), `thinking` = NPCs
/// with an in-flight LLM turn. `(0, 0)` when the NPC module is not loaded.
fn npc_counts(engine: &Engine) -> (i64, i64) {
    match engine.world.module_state::<mush_npc::NpcHostState>() {
        Some(h) => {
            let active = h.runtimes.len() as i64;
            let thinking = h.runtimes.values().filter(|rt| rt.in_flight).count() as i64;
            (active, thinking)
        }
        None => (0, 0),
    }
}

/// The per-`(feature, provider)` cumulative LLM stats + today's global spend,
/// pulled from the service's audit aggregate and budget ledger. `None` when no
/// LLM service is wired.
struct LlmSample {
    globally_disabled: bool,
    features: Vec<LlmFeatureStat>,
    today: LlmTodaySpend,
}

fn llm_sample(llm: Option<&Arc<LlmService>>) -> Option<LlmSample> {
    let llm = llm?;
    let features = llm
        .audit_log()
        .totals()
        .into_iter()
        .map(|t| LlmFeatureStat {
            feature: t.feature,
            provider: t.provider,
            calls: t.calls,
            refusals: t.refusals,
            budget_trips: t.budget_trips,
            disabled: t.disabled,
            errors: t.errors,
            input_tokens: t.input_tokens,
            output_tokens: t.output_tokens,
            cost_usd: t.cost_usd,
        })
        .collect();
    let g = llm.ledger().global();
    Some(LlmSample {
        globally_disabled: llm.kill_switch().global_disabled(),
        features,
        today: LlmTodaySpend {
            input_tokens: g.input_tokens,
            output_tokens: g.output_tokens,
            cost_usd: g.cost_usd,
        },
    })
}

/// Publish the world-owned gauges into the metrics registry, throttled to at
/// most once a second so the object scan + NPC/LLM aggregation never rides the
/// hot path. Called at the end of each world-loop iteration.
fn maybe_publish_sample(
    engine: &Engine,
    gateway: &Gateway,
    health: &StoreHealth,
    llm: Option<&Arc<LlmService>>,
    metrics: &Metrics,
    last_sample: &mut Option<Instant>,
) {
    const MIN_INTERVAL: Duration = Duration::from_secs(1);
    let now = Instant::now();
    if let Some(prev) = last_sample
        && now.duration_since(*prev) < MIN_INTERVAL
    {
        return;
    }
    *last_sample = Some(now);

    let counts = object_counts(&engine.world);
    let (npc_active, npc_thinking) = npc_counts(engine);
    // The session/connection gauges are gateway-owned: the in-process sink reads
    // the shared manager, the socket sink reports `(0, 0)` (the gateway process
    // publishes them post-fork). Advisory gauges, not world state.
    let (sessions_current, connections_current) = gateway.session_counts();
    metrics.publish_world(WorldSample {
        sessions_current,
        connections_current,
        scheduler_queue_depth: engine.sched.len() as i64,
        rooms: counts.rooms as i64,
        exits: counts.exits as i64,
        things: counts.things as i64,
        players: counts.players as i64,
        npcs: counts.npcs as i64,
        store_read_only: health.mode.store_read_only(),
        integrity_frozen: health.mode.integrity_frozen(),
        npc_active,
        npc_thinking,
    });
    if let Some(s) = llm_sample(llm) {
        metrics.publish_llm(true, s.globally_disabled, s.features, s.today);
    } else {
        metrics.publish_llm(false, false, Vec::new(), LlmTodaySpend::default());
    }
}

/// Extra aggregates threaded into the wizard `@stats` render beyond what the
/// engine + store health carry: transport-split connection counts, the store
/// write-queue depth, NPC runtime counts, and today's LLM spend.
#[derive(Clone, Debug, Default)]
pub struct StatsExtra {
    /// Live SSH transport connections.
    pub ssh_conns: i64,
    /// Live web transport connections.
    pub web_conns: i64,
    /// Store write batches queued but not yet acked.
    pub write_queue_depth: i64,
    /// NPCs with a live runtime.
    pub npc_active: i64,
    /// NPCs with an in-flight LLM turn.
    pub npc_thinking: i64,
    /// Approximate p99 loop lag (ms) from the histogram.
    pub lag_p99: u64,
    /// Input frames shed gateway->server since boot (M18 IPC bounding).
    pub ipc_frames_shed: u64,
    /// IPC write timeouts on a wedged peer since boot (M18).
    pub ipc_write_timeouts: u64,
    /// Server->gateway (`to_gw`) overflow teardowns since boot (M18).
    pub ipc_to_gw_overflow: u64,
    /// High-water mark of the gateway->server (`to_server`) queue depth (M18).
    pub ipc_hw_to_server: u64,
    /// High-water mark of the server->gateway (`to_gw`) queue depth (M18).
    pub ipc_hw_to_gw: u64,
    /// Today's LLM spend, when a service is wired.
    pub llm: Option<LlmStatsLine>,
}

/// Today's LLM spend for the `@stats` LLM line.
#[derive(Clone, Debug)]
pub struct LlmStatsLine {
    /// Whether the global kill switch is engaged.
    pub globally_disabled: bool,
    /// Today's global input tokens.
    pub today_input: u64,
    /// Today's global output tokens.
    pub today_output: u64,
    /// Today's global cost, USD.
    pub today_cost: f64,
    /// Per-feature today spend `(feature, input, output, cost)`, only rows
    /// with non-zero usage.
    pub features: Vec<(String, u64, u64, f64)>,
}

/// Build the [`StatsExtra`] for a `@stats` render from the live registries.
fn stats_extra(metrics: &Metrics, llm: Option<&Arc<LlmService>>) -> StatsExtra {
    let snap = metrics.snapshot();
    let llm_line = llm.map(|svc| {
        let ledger = svc.ledger();
        let g = ledger.global();
        // Per-feature today spend: probe each route the audit aggregate has
        // seen (cheap — a handful) against today's ledger.
        let mut features = Vec::new();
        for t in svc.audit_log().totals() {
            let c = ledger.feature(&t.feature);
            if c.input_tokens + c.output_tokens > 0 || c.cost_usd > 0.0 {
                features.push((t.feature, c.input_tokens, c.output_tokens, c.cost_usd));
            }
        }
        features.sort_by(|a, b| a.0.cmp(&b.0));
        features.dedup_by(|a, b| a.0 == b.0);
        LlmStatsLine {
            globally_disabled: svc.kill_switch().global_disabled(),
            today_input: g.input_tokens,
            today_output: g.output_tokens,
            today_cost: g.cost_usd,
            features,
        }
    });
    StatsExtra {
        ssh_conns: snap.ssh_current,
        web_conns: snap.web_current,
        write_queue_depth: snap.store_write_queue_depth,
        npc_active: snap.npc_active,
        npc_thinking: snap.npc_thinking,
        lag_p99: snap.loop_lag_p99_ms(),
        ipc_frames_shed: snap.ipc_frames_shed_total,
        ipc_write_timeouts: snap.ipc_write_timeouts_total,
        ipc_to_gw_overflow: snap.ipc_to_gw_overflow_total,
        ipc_hw_to_server: snap.ipc_queue_high_water_to_server,
        ipc_hw_to_gw: snap.ipc_queue_high_water_to_gw,
        llm: llm_line,
    }
}

/// Render the wizard `@stats` report: uptime, object counts by kind,
/// connected count (with SSH/web split), loop lag (last/max/p99), store health
/// (read-only mode + last-write age + write-queue depth), NPC counts, today's
/// LLM spend, and db size. Pure over its inputs — unit-testable.
#[must_use]
pub fn render_stats(
    engine: &Engine,
    health: &StoreHealth,
    boot_unix: i64,
    now_unix: i64,
    extra: &StatsExtra,
) -> String {
    let w = &engine.world;
    let counts = object_counts(w);
    let total = counts.total();
    let connected = w.connected().count();
    let (lag_last, lag_max) = engine.loop_lag_ms();
    let uptime = (now_unix - boot_unix).max(0);
    let store = if health.mode.store_read_only() {
        "READ-ONLY (writes failing; mutations paused)".to_string()
    } else {
        "healthy".to_string()
    };
    let write_age = match health.last_write_unix {
        Some(at) => format!("{}s ago", (now_unix - at).max(0)),
        None => "no writes yet this boot".to_string(),
    };
    let mut text = String::from("Server statistics:");
    text.push_str(&format!(
        "\nUptime: {} (since unix {boot_unix})",
        human_duration(uptime)
    ));
    text.push_str(&format!(
        "\nObjects: {total} ({} rooms, {} exits, {} things, {} players, {} NPCs)",
        counts.rooms, counts.exits, counts.things, counts.players, counts.npcs
    ));
    if counts.going > 0 {
        text.push_str(&format!("\nPending destruction: {}", counts.going));
    }
    text.push_str(&format!(
        "\nConnected: {connected} (ssh {}, web {})",
        extra.ssh_conns, extra.web_conns
    ));
    text.push_str(&format!(
        "\nLoop lag: last {lag_last}ms, max {lag_max}ms, p99 {}ms",
        extra.lag_p99
    ));
    text.push_str(&format!(
        "\nStore: {store}; last write {write_age}; write-queue {}",
        extra.write_queue_depth
    ));
    text.push_str(&format!(
        "\nNPC agents: {} active, {} thinking",
        extra.npc_active, extra.npc_thinking
    ));
    text.push_str(&format!(
        "\nIPC boundary: {} frames shed, {} write timeouts, {} to_gw overflows; high-water to_server {}, to_gw {}",
        extra.ipc_frames_shed,
        extra.ipc_write_timeouts,
        extra.ipc_to_gw_overflow,
        extra.ipc_hw_to_server,
        extra.ipc_hw_to_gw
    ));
    if let Some(llm) = &extra.llm {
        text.push_str(&format!(
            "\nLLM today: {} in + {} out tokens, ${:.4} (kill switch: {})",
            llm.today_input,
            llm.today_output,
            llm.today_cost,
            if llm.globally_disabled {
                "ENGAGED"
            } else {
                "off"
            }
        ));
        for (feature, inp, out, cost) in &llm.features {
            text.push_str(&format!(
                "\n  {feature}: {inp} in + {out} out tokens, ${cost:.4}"
            ));
        }
    }
    if let (Some(definitions), Some(bridges), Some(migrations), Some(fingerprint)) = (
        engine.registry.definitions(),
        engine.registry.bridges(),
        engine.registry.definition_migrations(),
        engine.registry.mechanics_fingerprint(),
    ) {
        let summary = definitions.summary();
        text.push_str(&format!(
            "\nMechanics: {} definitions from {} owners, {} bridges, {} migrations; fingerprint {}",
            summary.definition_count,
            summary.owner_count,
            bridges.iter().len(),
            migrations.iter().len(),
            fingerprint,
        ));
    }
    for report in engine.registry.recovery_reports(w) {
        text.push_str(&format!(
            "\nRecovery {}: {} active, {} frozen, {} migrating, {} quarantined; {} incidents, {} migration plans{}",
            report.owner,
            report.active,
            report.frozen,
            report.migrating,
            report.quarantined,
            report.incident_count,
            report.migration_plan_count,
            if report.details_truncated { " (detail truncated)" } else { "" },
        ));
    }
    text.push_str(&format!(
        "\nDb size: {total} objects (next id #{})",
        w.next_id()
    ));
    text
}

/// `3661` → `"1h 1m 1s"`.
fn human_duration(mut s: i64) -> String {
    let h = s / 3600;
    s %= 3600;
    let m = s / 60;
    s %= 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Hand off the staff ops the step's command handlers queued. Ban ops
/// (set/clear/list) go to the **sequential** [`staff_ops_loop`] so two
/// staff commands issued in order can never race in the store; `@dump`
/// rides the store-writer queue (ordered behind pending batches);
/// `@shutdown` returns `true` — the caller (the world loop) then steps
/// `ControlMsg::Shutdown` and exits. Every op is audit-logged with the
/// requesting actor.
async fn run_staff_ops(
    _deps: &ServerDeps,
    gateway: &Gateway,
    writer: &mpsc::Sender<StoreJob>,
    staff_ops: &mpsc::Sender<StaffOp>,
    ops: Vec<StaffOp>,
) -> bool {
    let mut shutdown = false;
    for op in ops {
        let actor = op.actor;
        match op.kind {
            StaffOpKind::SetBan {
                ref ip,
                until_unix,
                ref reason,
            } => {
                tracing::info!(%actor, %ip, ?until_unix, reason, "@siteban requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@siteban failed: staff executor gone.");
                }
            }
            StaffOpKind::ClearBan { ref ip } => {
                tracing::info!(%actor, %ip, "@siteban/clear requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(
                        gateway,
                        actor,
                        "@siteban/clear failed: staff executor gone.",
                    );
                }
            }
            StaffOpKind::ListBans => {
                tracing::info!(%actor, "@siteban list requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@siteban failed: staff executor gone.");
                }
            }
            StaffOpKind::NewPassword { target, .. } => {
                // Audit: actor + target only — the op's password never
                // reaches a log (Secret redacts even derived Debug).
                tracing::info!(%actor, %target, "@newpassword requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@newpassword failed: staff executor gone.");
                }
            }
            StaffOpKind::AddSshKey {
                ref fingerprint, ..
            } => {
                // Audit: actor + fingerprint (public data). The raw pasted
                // line never reached this op — the command handler validated
                // and normalized it, queuing only the public key (S7).
                tracing::info!(%actor, fingerprint, "@sshkey/add requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@sshkey failed: staff executor gone.");
                }
            }
            StaffOpKind::ListSshKeys { .. } => {
                tracing::info!(%actor, "@sshkey/list requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@sshkey failed: staff executor gone.");
                }
            }
            StaffOpKind::DelSshKey { ref which, .. } => {
                tracing::info!(%actor, which, "@sshkey/del requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@sshkey failed: staff executor gone.");
                }
            }
            StaffOpKind::NewChar { ref name, .. } => {
                // Audit: actor + the requested name (both public). The account
                // is resolved store-side in the executor (invariant N1).
                tracing::info!(%actor, name, "@newchar requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@newchar failed: staff executor gone.");
                }
            }
            StaffOpKind::Dump => {
                tracing::info!(%actor, "@dump requested");
                let writer = writer.clone();
                let gateway = gateway.clone();
                tokio::spawn(async move {
                    let (done, done_rx) = oneshot::channel();
                    if writer.send(StoreJob::Bgsave { done }).await.is_err() {
                        send_sys(&gateway, actor, "@dump failed: store writer gone.");
                        return;
                    }
                    let text = match done_rx.await {
                        Ok(Ok(())) => "@dump: snapshot acknowledged by the store.".to_string(),
                        Ok(Err(e)) => format!("@dump failed: {e}"),
                        Err(_) => "@dump failed: store writer gone.".to_string(),
                    };
                    send_sys(&gateway, actor, &text);
                });
            }
            StaffOpKind::Shutdown => {
                tracing::info!(%actor, "@shutdown requested");
                shutdown = true;
            }
            // Local ops are partitioned out before this call.
            StaffOpKind::Boot { .. } | StaffOpKind::Ps | StaffOpKind::Stats => {
                debug_assert!(false, "local staff op reached the store executor path");
            }
        }
    }
    shutdown
}

/// Execute one `@newpassword`: argon2-hash on the blocking pool, find the
/// owning account by scanning the account docs (characters are few; the
/// store has no reverse index yet), update the stored hash, then revoke
/// every web token of that account (logout-everywhere) **and boot the
/// account's live web connections** — web transports are token-
/// authenticated, so they die with the tokens (SSH sessions are session-
/// authenticated and stay; boot them with `@boot` if needed). Returns the
/// confirmation text for the requesting wizard; delivers the target's
/// notification itself.
#[allow(clippy::too_many_arguments)]
async fn run_newpassword(
    store: &dyn ObjectStore,
    gateway: &Gateway,
    world: &mpsc::WeakSender<WorldMsg>,
    target: ObjId,
    name: &str,
    by: &str,
    password: &str,
) -> String {
    let hash = {
        let password = password.to_string();
        match tokio::task::spawn_blocking(move || mush_config::accounts::hash_password(&password))
            .await
        {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => return format!("@newpassword failed: {e}."),
            Err(e) => return format!("@newpassword failed: hashing task failed: {e}."),
        }
    };
    let account = match store.list_accounts().await {
        Ok(accounts) => accounts
            .into_iter()
            .find(|a| a.characters.iter().any(|c| c.id == target)),
        Err(e) => return format!("@newpassword failed: {e}"),
    };
    let Some(account) = account else {
        return format!("@newpassword: {name} is not linked to any account.");
    };
    match store.set_account_password(&account.name, &hash).await {
        Ok(true) => {}
        Ok(false) => {
            return format!(
                "@newpassword failed: account {} vanished mid-update.",
                account.name
            );
        }
        Err(e) => return format!("@newpassword failed: {e}"),
    }
    // Logout-everywhere: every web session token of the account dies with
    // the old password. (Live SSH connections are session-authenticated
    // and stay; boot them separately with @boot if needed.)
    let revoked = match store.tokens_del_account(&account.name).await {
        Ok(n) => n,
        Err(e) => {
            // The password DID change; report the partial failure honestly.
            return format!(
                "@newpassword: password for {name} changed, but revoking web \
                 sessions failed: {e}"
            );
        }
    };
    tracing::info!(%target, account = %account.name, revoked, "@newpassword applied");
    // Notify BEFORE booting, so a web-connected target actually sees why
    // their session is about to close.
    send_sys(
        gateway,
        target,
        &format!("GAME: Your password has been changed by {by}."),
    );
    // The tokens are dead; so are the live connections they opened. Every
    // structured (web) connection on any of the account's characters is booted.
    let notice = OutputEvent::Disconnect {
        reason: RichText::tag(
            "system",
            format!("Your password has been changed by {by}; please log in again."),
        ),
    };
    // Split mode (M16b): the forked server holds no `SessionManager`, so it
    // cannot walk the account's connections. Emit ONE account/char-addressed
    // fire-and-forget `ToGateway::CloseWebSessions` — the gateway resolves each
    // character to its live structured connections on its own manager, delivers
    // the notice, drops them, and reports each `ConnDown(Boot)` back as inbound,
    // which the world steps exactly as the in-process walk below does. No
    // reverse query, and this runs on the staff-op task — never the world loop
    // (D4).
    if gateway.is_socket() {
        let chars: Vec<u64> = account
            .characters
            .iter()
            .map(|c| u64::from(c.id.0))
            .collect();
        gateway.close_web_sessions(chars, Some(notice), mush_ipc::DetachCause::Booted);
        return format!(
            "@newpassword: Password for {name} (account {}) changed; \
             {revoked} web session token{} revoked; any live web connections \
             were logged out.",
            account.name,
            if revoked == 1 { "" } else { "s" },
        );
    }
    // All-in-one: walk the shared manager (through the in-process sink) directly
    // — announce + bookkeeping through the world when the connection is still
    // up, then close each structured (web) connection.
    let world = world.upgrade().map(|tx| WorldHandle { tx, shutdown: None });
    let chars: Vec<u64> = account
        .characters
        .iter()
        .map(|c| u64::from(c.id.0))
        .collect();
    let mut booted = 0u32;
    for (conn, char) in gateway.structured_conns_of(&chars) {
        let actor = ObjId(u32::try_from(char).unwrap_or(u32::MAX));
        if let Some(world) = &world {
            world
                .inbound(Inbound::ConnDown {
                    conn,
                    actor,
                    reason: DisconnectKind::Boot,
                })
                .await;
            world.barrier().await;
        }
        gateway.deliver(SendTarget::Conn(conn), Arc::new(notice.clone()));
        gateway.detach_conn_local(conn);
        booted += 1;
    }
    format!(
        "@newpassword: Password for {name} (account {}) changed; \
         {revoked} web session token{} revoked, {booted} live web \
         connection{} booted.",
        account.name,
        if revoked == 1 { "" } else { "s" },
        if booted == 1 { "" } else { "s" },
    )
}

/// Per-account cap on registered SSH public keys.
const SSH_KEYS_PER_ACCOUNT: usize = 10;

/// Resolve the account that lists `actor` among its characters — the same
/// scan `@newpassword` does, but keyed on the CALLER, so `@sshkey` can only
/// ever read/modify the caller's OWN account (invariant S5). `Ok(None)` means
/// the character is linked to no account (a built-in like `#1` in a bare
/// world); `Err` is a store failure.
async fn account_of_actor(
    store: &dyn ObjectStore,
    actor: ObjId,
) -> Result<Option<AccountRec>, String> {
    match store.list_accounts().await {
        Ok(accounts) => Ok(accounts
            .into_iter()
            .find(|a| a.characters.iter().any(|c| c.id == actor))),
        Err(e) => Err(e.to_string()),
    }
}

/// The OpenSSH comment (everything after `type base64`), or a placeholder.
fn ssh_comment(openssh: &str) -> &str {
    openssh
        .splitn(3, char::is_whitespace)
        .nth(2)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("(no comment)")
}

/// Execute `@sshkey/add`: resolve the caller's account, dedupe by fingerprint,
/// enforce the per-account cap, append the (already validated + normalized)
/// public key, and write it through. The gateway reads the account fresh per
/// auth, so the key authenticates on the very next `ssh -i` (invariant S4).
async fn run_add_sshkey(
    store: &dyn ObjectStore,
    actor: ObjId,
    openssh: &str,
    fingerprint: &str,
) -> String {
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@sshkey: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@sshkey/add failed: {e}"),
    };
    let mut keys = account.ssh_keys.clone();
    if keys.iter().any(|k| k.fingerprint == fingerprint) {
        return format!("@sshkey: that key is already registered ({fingerprint}).");
    }
    if keys.len() >= SSH_KEYS_PER_ACCOUNT {
        return format!(
            "@sshkey: your account already has the maximum of {SSH_KEYS_PER_ACCOUNT} keys; \
             remove one with @sshkey/del first."
        );
    }
    keys.push(SshKeyRec {
        openssh: openssh.to_string(),
        fingerprint: fingerprint.to_string(),
        added: unix_now_s(),
    });
    match store.set_account_ssh_keys(&account.name, &keys).await {
        Ok(true) => {
            // Audit: account + fingerprint (public); never the raw input (S7).
            tracing::info!(account = %account.name, fingerprint, "@sshkey/add applied");
            format!(
                "@sshkey: key {fingerprint} added. You can now log in with \
                 `ssh -i <your key> {}@game`.",
                account.name
            )
        }
        Ok(false) => format!(
            "@sshkey/add failed: account {} vanished mid-update.",
            account.name
        ),
        Err(e) => format!("@sshkey/add failed: {e}"),
    }
}

/// Execute `@sshkey/list`: render the caller's account's keys as
/// `index) SHA256:fp  <comment>  (added <date>)`.
async fn run_list_sshkeys(store: &dyn ObjectStore, actor: ObjId) -> String {
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@sshkey: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@sshkey/list failed: {e}"),
    };
    if account.ssh_keys.is_empty() {
        return "@sshkey: you have no registered SSH keys. Add one with \
                @sshkey/add <public key line>."
            .to_string();
    }
    let mut text = format!("SSH keys on account {}:", account.name);
    for (i, k) in account.ssh_keys.iter().enumerate() {
        text.push_str(&format!(
            "\n  {}) {}  {}  (added {})",
            i + 1,
            k.fingerprint,
            ssh_comment(&k.openssh),
            mush_mail::format_datetime(k.added),
        ));
    }
    text
}

/// Execute `@sshkey/del <index-or-fingerprint>`: remove one key from the
/// caller's account by 1-based list index or by exact `SHA256:...`
/// fingerprint, then write the remaining set back.
async fn run_del_sshkey(store: &dyn ObjectStore, actor: ObjId, which: &str) -> String {
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@sshkey: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@sshkey/del failed: {e}"),
    };
    let mut keys = account.ssh_keys.clone();
    // Prefer a 1-based index; fall back to a fingerprint match.
    let idx = match which.parse::<usize>() {
        Ok(n) if n >= 1 && n <= keys.len() => Some(n - 1),
        Ok(_) => None,
        Err(_) => keys.iter().position(|k| k.fingerprint == which),
    };
    let Some(idx) = idx else {
        return format!("@sshkey: no key matches {which:?}. Use @sshkey/list to see them.");
    };
    let removed = keys.remove(idx);
    match store.set_account_ssh_keys(&account.name, &keys).await {
        Ok(true) => {
            tracing::info!(account = %account.name, fingerprint = %removed.fingerprint, "@sshkey/del applied");
            format!("@sshkey: removed key {}.", removed.fingerprint)
        }
        Ok(false) => format!(
            "@sshkey/del failed: account {} vanished mid-update.",
            account.name
        ),
        Err(e) => format!("@sshkey/del failed: {e}"),
    }
}

/// Execute `@newchar <Name>`: create ONE more character on the CALLER's own
/// account (self-service; M20). Runs entirely off the world loop:
///
/// 1. Resolve the caller's OWN account with the same `list_accounts` scan
///    `@newpassword`/`@sshkey` use, keyed on `actor` — so it can only ever
///    touch the caller's own account (invariant N1).
/// 2. Enforce the `[game].max_characters` cap and an advisory world-name
///    pre-check BEFORE anything is created (N2). The cap check shares the same
///    accepted TOCTOU as the gateway `create_character` path: two concurrent
///    `@newchar`s at cap-1 on the same account can both pass (the append is
///    unconditional), overshooting the cap by one — it takes the account
///    holder racing themselves and the only damage is one extra character.
/// 3. Create the Player through the established durable, ack-gated server
///    create path ([`WorldHandle::create_player`], the same `WorldMsg::
///    CreatePlayer` route account creation uses) — which emits `ObjCreated`
///    (comsys auto-join) and waits for the store to acknowledge the creation
///    batch — then link it into the account doc (N3).
///
/// **Create-then-link ordering / orphan window:** identical to the gateway
/// `create_character` flow (`mush-gateway`'s `accounts::create_and_link`). The
/// Player is created (and durably persisted) BEFORE the account link; if the
/// link write then fails, a created-but-unlinked Player is left behind (the
/// store has no account-delete yet). That is the SAME accepted, loudly-logged
/// orphan window as `create_character` — no worse — and the pre-checks make it
/// a genuine store failure, not a normal path. The confirmation is returned
/// only after the link write returns (N3, ack-gated). The caller is NOT
/// switched to the new character (N4): they keep playing their current one.
async fn run_newchar(
    store: &dyn ObjectStore,
    world: &mpsc::WeakSender<WorldMsg>,
    actor: ObjId,
    name: &str,
    max_characters: usize,
) -> String {
    // (1) Resolve the caller's OWN account (N1). `account_of_actor` is the
    // same scan @sshkey/@newpassword use, keyed on the caller.
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@newchar: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@newchar failed: {e}"),
    };
    // (2) Cap check BEFORE any create (N2).
    if account.characters.len() >= max_characters {
        return format!("@newchar: you already have the maximum of {max_characters} characters.");
    }
    // The world handle for the durable, ack-gated create path. A dropped world
    // (teardown) means we cannot create; fail closed rather than orphan.
    let Some(world) = world.upgrade().map(|tx| WorldHandle { tx, shutdown: None }) else {
        return "@newchar failed: the world is shutting down.".to_string();
    };
    // Advisory name pre-check BEFORE create (N2); the create re-checks atomically.
    match world.name_free(name).await {
        Ok(true) => {}
        Ok(false) => return format!("@newchar: the name {name:?} is already taken."),
        Err(WorldReadError::Busy) => {
            return "@newchar: the world is committing another action; please retry shortly."
                .to_string();
        }
        Err(WorldReadError::Unavailable) => {
            return "@newchar failed: the world is unavailable.".to_string();
        }
    }
    // (3) Create through the established server create path — durable + ack-gated,
    // and it emits ObjCreated (comsys auto-join) exactly like account creation.
    let id = match world.create_player(name).await {
        Ok(id) => id,
        // The pre-check raced another create; no second Player is made (the world
        // create rejects the duplicate atomically) — so this never orphans.
        Err(CreatePlayerErr::NameTaken) => {
            return format!("@newchar: the name {name:?} is already taken.");
        }
        Err(e) => return format!("@newchar failed: {e}"),
    };
    // Link it into the account doc. On failure the Player exists but is
    // unlinked — the SAME accepted, logged orphan window as create_character.
    match store.link_character(&account.name, id, name).await {
        Ok(()) => {
            tracing::info!(account = %account.name, %id, name, "@newchar created + linked");
            format!(
                "@newchar: created {name} ({id}) on your account. To play it, reconnect and \
                 pick it at the character-select screen, or `ssh {}+{name}@<host>` to jump \
                 straight in. It starts unapproved; chargen runs the first time you play it.",
                account.name
            )
        }
        Err(e) => {
            tracing::error!(
                account = %account.name, %id, name, error = %e,
                "@newchar: Player created but linking to the account failed; Player left unlinked"
            );
            format!(
                "@newchar: created the character, but linking it to your account failed: {e}. \
                 Please contact staff."
            )
        }
    }
}

/// The sequential staff-op executor: one task, one queue, ops applied to
/// the store (and mirrored into the [`IpGuard`]) strictly in the order
/// staff issued them — a `@siteban/clear` can never lose to the `@siteban`
/// it follows. Confirmations are delivered to the requesting actor after
/// the store acknowledged. Exits when the world loop drops its sender.
async fn staff_ops_loop(
    store: Arc<dyn ObjectStore>,
    guard: Arc<IpGuard>,
    world: mpsc::WeakSender<WorldMsg>,
    mut rx: mpsc::Receiver<StaffOp>,
    gateway: Gateway,
) {
    // The staff executor's server->gateway output (confirmations, the target's
    // @newpassword notice) crosses the seam through this `gateway` sink — the
    // shared `SessionManager` in the all-in-one binary, the boundary socket in
    // the forked server. The session *control* ops @newpassword performs
    // (`structured_conns_of` / `detach_conn_local`) go through the same sink; in
    // split mode they resolve on the gateway process (this sink emits a
    // fire-and-forget `CloseWebSessions` instead).
    while let Some(op) = rx.recv().await {
        let actor = op.actor;
        let text = match op.kind {
            StaffOpKind::SetBan {
                ip,
                until_unix,
                reason,
            } => match store.set_ban(&ip.to_string(), until_unix, &reason).await {
                Ok(()) => {
                    guard.set_siteban(ip, until_unix);
                    match until_unix {
                        None => format!("@siteban: {ip} banned permanently."),
                        Some(until) => format!("@siteban: {ip} banned until unix {until}."),
                    }
                }
                Err(e) => format!("@siteban failed: {e}"),
            },
            StaffOpKind::ClearBan { ip } => match store.clear_ban(&ip.to_string()).await {
                Ok(removed) => {
                    guard.clear_siteban(ip);
                    if removed {
                        format!("@siteban: ban on {ip} lifted.")
                    } else {
                        format!("@siteban: no ban on {ip}.")
                    }
                }
                Err(e) => format!("@siteban/clear failed: {e}"),
            },
            StaffOpKind::NewPassword {
                target,
                name,
                by,
                password,
            } => run_newpassword(&*store, &gateway, &world, target, &name, &by, &password.0).await,
            StaffOpKind::ListBans => match store.get_bans().await {
                Ok(bans) if bans.is_empty() => "No active sitebans.".to_string(),
                Ok(mut bans) => {
                    bans.sort_by(|a, b| a.ip.cmp(&b.ip));
                    let mut text = String::from("Active sitebans:");
                    for b in bans {
                        let until = b
                            .until_unix
                            .map_or_else(|| "permanent".to_string(), |u| format!("until unix {u}"));
                        text.push_str(&format!("\n  {}  {}  {}", b.ip, until, b.reason));
                    }
                    text
                }
                Err(e) => format!("@siteban list failed: {e}"),
            },
            StaffOpKind::AddSshKey {
                actor: subject,
                openssh,
                fingerprint,
            } => run_add_sshkey(&*store, subject, &openssh, &fingerprint).await,
            StaffOpKind::ListSshKeys { actor: subject } => run_list_sshkeys(&*store, subject).await,
            StaffOpKind::DelSshKey {
                actor: subject,
                which,
            } => run_del_sshkey(&*store, subject, &which).await,
            StaffOpKind::NewChar {
                actor: subject,
                name,
                max_characters,
            } => run_newchar(&*store, &world, subject, &name, max_characters).await,
            // Dump/Shutdown and the local ops never enter this queue (see
            // run_staff_ops / run_local_staff_ops).
            StaffOpKind::Dump
            | StaffOpKind::Shutdown
            | StaffOpKind::Boot { .. }
            | StaffOpKind::Ps
            | StaffOpKind::Stats => continue,
        };
        send_sys(&gateway, actor, &text);
    }
    tracing::debug!("staff op executor drained and exiting");
}

/// Send one system line to an actor as a [`ToGateway::Deliver`](mush_ipc::ToGateway)
/// through the seam (staff-op confirmations, store-failure warnings).
fn send_sys(gateway: &Gateway, actor: ObjId, text: &str) {
    gateway.deliver(
        SendTarget::Char(u64::from(actor.0)),
        Arc::new(OutputEvent::Sys(RichText::tag("system", text))),
    );
}

/// Sleep until `due` (unix milliseconds). Only ever polled when `due` is
/// `Some` (the select guard); an already-past deadline yields immediately.
///
/// Deadlines are wall-clock unix ms armed against a monotonic sleep: a
/// backward NTP step delays timers by the step size (the loop re-arms
/// harmlessly each iteration), a forward step fires them early. Accepted
/// for M1; revisit if `@wait` precision ever matters.
async fn sleep_until_unix_ms(due: Option<i64>) {
    let Some(due) = due else { return };
    let delta = due.saturating_sub(unix_now_ms()).max(0);
    tokio::time::sleep(Duration::from_millis(u64::try_from(delta).unwrap_or(0))).await;
}

/// Create a Player in the start room (Limbo). Players own themselves.
fn create_player(engine: &mut Engine, name: &str) -> Result<ObjId, CreatePlayerErr> {
    match engine.world.create(
        ObjKind::Player,
        name,
        mush_core::GOD,
        Some(mush_core::LIMBO),
    ) {
        Ok(id) => {
            tracing::info!(%id, name, "created player");
            Ok(id)
        }
        Err(mush_core::WorldError::NameInUse(_)) => Err(CreatePlayerErr::NameTaken),
        Err(e) => Err(CreatePlayerErr::World(e.to_string())),
    }
}

/// Apply one complete gateway roster without firing connection hooks. Kept as
/// a single helper so immediate reattach and post-receipt coalesced reattach
/// cannot drift on partial-snapshot semantics.
fn apply_reattach_snapshot(engine: &mut Engine, recs: &[ReAttachRec], out: &mut Outbox) {
    for rec in recs {
        engine.reattach(rec.conn, rec.actor, rec.caps_structured, out);
    }
}

// `SendTarget` (the fully resolved send destination) now lives in `mush-ipc`
// as the server->gateway `Deliver` payload; its `Char`/`Conn` arms carry raw
// `u64` routing keys (`mush-ipc` must not depend on `mush-session`). `mushd`
// widens a `CharKey`/`SessConnId` in and out at the two boundary points below,
// exactly as before.
/// Resolve every queued audience against **current** world state into
/// per-character (or per-connection) sends. The world half of treaty
/// ruling 2 lives here: `Room` audiences become the room's *connected
/// actors*; the session manager later takes it from character to
/// connections. Split from [`transmit`] so a durable-gated step can
/// resolve now and deliver after the store ack.
#[cfg(test)]
fn resolve_sends(
    reg: &Registry,
    world: &World,
    out: &mut Outbox,
) -> Vec<(SendTarget, Arc<OutputEvent>)> {
    resolve_classified_sends(reg, world, out)
        .into_iter()
        .map(|(_class, target, event)| (target, event))
        .collect()
}

/// Resolve audiences while retaining each item's durability relationship.
/// Resolution happens at the end of the producing step, before later
/// transport-only work can change the connected-recipient set.
fn resolve_classified_sends(
    reg: &Registry,
    world: &World,
    out: &mut Outbox,
) -> Vec<ClassifiedSend> {
    let mut sends = Vec::new();
    for (class, audience, event) in out.take_classified() {
        let ev = Arc::new(event);
        match audience {
            Audience::Actor(actor) => {
                sends.push((class, SendTarget::Char(u64::from(actor.0)), ev));
            }
            Audience::Conn(conn) => {
                sends.push((class, SendTarget::Conn(conn), ev));
            }
            Audience::Room {
                room,
                exclude,
                subject: room_subject,
            } => {
                // `@ignore` room filtering happens HERE — the one place a
                // Room audience resolves to listeners (treaty ruling 2's
                // world half). An attributed narrative line (say/pose/
                // semipose/@emit) from a speaker a listener has muted is
                // skipped for that listener only; staff speakers are never
                // filtered (mush_core::is_ignoring owns the policy).
                let speaker = narrative_actor(&ev);
                // M11 per-observer projection: the occupant this event would
                // reveal (its subject). A recipient who cannot perceive that
                // subject is skipped for THIS event only. The subject is the
                // one the routing layer carried (movement/arrival/departure/
                // item/connect echoes) or, failing that, the attributed actor
                // of a narrative line (say/pose/@emit). Events with no subject
                // (weather/panels) name no one and reach the whole room as
                // before. Free pass with no Perceive hook registered, so
                // hookless fan-out is byte-identical.
                let subject = room_subject.or_else(|| event_subject(&ev));
                for &occupant in world.contents_of(room) {
                    if exclude.contains(&occupant) || !world.is_connected(occupant) {
                        continue;
                    }
                    if let Some(speaker) = speaker
                        && is_ignoring(world, occupant, speaker)
                    {
                        continue;
                    }
                    if let Some(subject) = subject
                        && !can_perceive(reg, world, occupant, subject)
                    {
                        continue;
                    }
                    sends.push((
                        class,
                        SendTarget::Char(u64::from(occupant.0)),
                        Arc::clone(&ev),
                    ));
                }
            }
            Audience::All => sends.push((class, SendTarget::All, ev)),
        }
    }
    sends
}

fn release_classified_sends(
    gateway: &Gateway,
    sends: Vec<ClassifiedSend>,
    outcome: DeliveryOutcome,
) -> usize {
    let released: Vec<_> = sends
        .into_iter()
        .filter_map(|(class, target, event)| class.releases_on(outcome).then_some((target, event)))
        .collect();
    let count = released.len();
    gateway.transmit(released);
    count
}

fn release_classified_effects(
    mode: WorldMode,
    effects: Vec<(DeliveryClass, Effect)>,
    outcome: DeliveryOutcome,
    tx: &mpsc::WeakSender<WorldMsg>,
    llm: Option<&Arc<LlmService>>,
) {
    if outcome == DeliveryOutcome::Aborted {
        if !effects.is_empty() {
            tracing::debug!(
                dropped = effects.len(),
                "all external effects discarded after authoritative abort"
            );
        }
        return;
    }
    let mut released = Vec::new();
    let mut outcome_dropped = 0_usize;
    for (class, effect) in effects {
        if class.releases_on(outcome) {
            released.push(effect);
        } else {
            outcome_dropped += 1;
        }
    }
    if outcome_dropped > 0 {
        tracing::debug!(
            dropped = outcome_dropped,
            ?outcome,
            "external effects discarded by delivery outcome"
        );
    }
    match admission_for(mode, IngressClass::ExternalEffect) {
        Admission::Allow => spawn_effect_list(released, tx, llm),
        Admission::Refuse(reason) => {
            if !released.is_empty() {
                tracing::warn!(
                    dropped = released.len(),
                    ?reason,
                    "external effects suppressed by world mode"
                );
            }
        }
        Admission::CheckStoreReadOnlyCommand => {
            unreachable!("effect launch never uses command-text classification")
        }
    }
}

/// Best-effort status may cross the boundary while a write is pending. Every
/// other class remains owned by the exact receipt.
fn release_best_effort_before_ack(
    gateway: &Gateway,
    sends: &mut Vec<ClassifiedSend>,
    _effects: &mut Vec<(DeliveryClass, Effect)>,
    _mode: WorldMode,
    _tx: &mpsc::WeakSender<WorldMsg>,
    _llm: Option<&Arc<LlmService>>,
) {
    let (immediate_sends, held_sends): (Vec<_>, Vec<_>) = std::mem::take(sends)
        .into_iter()
        .partition(|(class, _, _)| *class == DeliveryClass::BestEffort);
    *sends = held_sends;
    gateway.transmit(
        immediate_sends
            .into_iter()
            .map(|(_, target, event)| (target, event))
            .collect(),
    );

    // BestEffort relaxes presentation durability only. External work never
    // launches while any authoritative receipt is outstanding.
}

/// Operational work admitted while another commit is pending is independent
/// for BestEffort/OnAbort purposes. Gameplay/world reads are refused before
/// this point because RAM contains the tentative post-image. Reattach renders
/// are commit-bound and therefore join the outstanding receipt rather than
/// exposing that image early.
fn release_independent_while_pending(
    gateway: &Gateway,
    sends: &mut Vec<ClassifiedSend>,
    _effects: &mut Vec<(DeliveryClass, Effect)>,
    _mode: WorldMode,
    _tx: &mpsc::WeakSender<WorldMsg>,
    _llm: Option<&Arc<LlmService>>,
) {
    let is_independent =
        |class: DeliveryClass| matches!(class, DeliveryClass::BestEffort | DeliveryClass::OnAbort);
    let (immediate_sends, held_sends): (Vec<_>, Vec<_>) = std::mem::take(sends)
        .into_iter()
        .partition(|(class, _, _)| is_independent(*class));
    *sends = held_sends;
    gateway.transmit(
        immediate_sends
            .into_iter()
            .map(|(_, target, event)| (target, event))
            .collect(),
    );

    // Even independently clean work cannot launch an external effect while an
    // older authoritative write is unresolved. These remain in `effects` and
    // join that exact receipt below.
}

// Keep every authority involved in resolving an exact receipt explicit at this
// single boundary; hiding the mutable health/latch pair in ambient state would
// make it easier to freeze the world without publishing the shutdown barrier.
#[allow(clippy::too_many_arguments)]
fn complete_pending_commit(
    pending: PendingCommit,
    ack: Result<(), AuthoritativeCommitFailure>,
    engine: &mut Engine,
    gateway: &Gateway,
    health: &mut StoreHealth,
    persistence_integrity: &PersistenceIntegrityLatch,
    tx: &mpsc::WeakSender<WorldMsg>,
    llm: Option<&Arc<LlmService>>,
) -> CompletedPending {
    let PendingCommit {
        sends,
        effects,
        failure_actor,
        create,
        prepared_installation,
        barriers,
        coalesced_reattach,
        ..
    } = pending;
    let (outcome, failure, integrity_failure) = match ack {
        Ok(()) => (DeliveryOutcome::Committed, None, false),
        Err(AuthoritativeCommitFailure::Refused(detail)) => {
            (DeliveryOutcome::Aborted, Some(detail), false)
        }
        Err(AuthoritativeCommitFailure::Integrity(detail)) => {
            (DeliveryOutcome::Aborted, Some(detail), true)
        }
    };
    let has_abort_output = sends
        .iter()
        .any(|(class, _, _)| matches!(class, DeliveryClass::BestEffort | DeliveryClass::OnAbort));
    if let Some(installation) = prepared_installation {
        if outcome == DeliveryOutcome::Committed {
            // The exact applied receipt is the only transition that makes the
            // prepared typed post-images live. No provider, validator, codec,
            // serializer, or readiness callback runs here.
            engine.install_prepared_action(installation);
        } else {
            engine.abort_prepared_action(installation);
        }
    }
    release_classified_sends(gateway, sends, outcome);
    let effect_mode = if outcome == DeliveryOutcome::Committed && !health.mode.integrity_frozen() {
        // A successful receipt is itself the recovery fact for this batch;
        // StoreStatus(true) may still be queued behind it in the select.
        WorldMode::Writable
    } else {
        health.mode
    };
    release_classified_effects(effect_mode, effects, outcome, tx, llm);

    match (create, failure.as_deref()) {
        (Some(create), None) => {
            let _ = create.reply.send(Ok(create.id));
        }
        (Some(create), Some(detail)) => {
            let _ = create.reply.send(Err(CreatePlayerErr::World(format!(
                "store write failed: {}",
                bounded_failure_detail(detail)
            ))));
        }
        (None, _) => {}
    }

    if let Some(detail) = failure {
        if integrity_failure {
            apply_integrity_freeze(
                &engine.world,
                gateway,
                health,
                persistence_integrity,
                &detail,
            );
        }
        if let Some(actor) = failure_actor
            && !has_abort_output
        {
            send_commit_failure(gateway, actor, &detail);
        }
    }
    CompletedPending {
        outcome,
        barriers,
        coalesced_reattach,
    }
}

/// Finish transport-only work that accumulated behind an exact receipt. The
/// coalesced roster is applied before any ordering barrier is released. Its
/// room views use the same receipt disposition as the snapshots they replace:
/// visible on commit, dropped on abort, while connection bookkeeping is rebuilt
/// in either case just as it was before coalescing.
fn finish_pending_operational(engine: &mut Engine, gateway: &Gateway, completed: CompletedPending) {
    if let Some(recs) = completed.coalesced_reattach {
        let count = recs.len();
        let mut reattach_out = Outbox::new();
        apply_reattach_snapshot(engine, &recs, &mut reattach_out);
        let sends = resolve_classified_sends(&engine.registry, &engine.world, &mut reattach_out);
        release_classified_sends(gateway, sends, completed.outcome);
        tracing::info!(
            count,
            "applied coalesced reattach snapshot after exact receipt"
        );
    }
    // A barrier completes after both authoritative release and any preceding
    // coalesced transport snapshot, never merely after its message was dequeued.
    let barrier_result = if completed.outcome == DeliveryOutcome::Committed {
        Ok(())
    } else {
        Err(WorldReadError::Unavailable)
    };
    for barrier in completed.barriers {
        let _ = barrier.send(barrier_result);
    }
}

#[allow(clippy::too_many_arguments)]
fn abort_without_receipt(
    sends: Vec<ClassifiedSend>,
    effects: Vec<(DeliveryClass, Effect)>,
    failure_actor: Option<ObjId>,
    create: Option<CreateCompletion>,
    detail: &str,
    world: &World,
    gateway: &Gateway,
    health: &mut StoreHealth,
    persistence_integrity: &PersistenceIntegrityLatch,
    tx: &mpsc::WeakSender<WorldMsg>,
    llm: Option<&Arc<LlmService>>,
) {
    let has_abort_output = sends
        .iter()
        .any(|(class, _, _)| matches!(class, DeliveryClass::BestEffort | DeliveryClass::OnAbort));
    release_classified_sends(gateway, sends, DeliveryOutcome::Aborted);
    release_classified_effects(health.mode, effects, DeliveryOutcome::Aborted, tx, llm);
    apply_integrity_freeze(world, gateway, health, persistence_integrity, detail);
    if let Some(actor) = failure_actor
        && !has_abort_output
    {
        send_commit_failure(gateway, actor, detail);
    }
    if let Some(create) = create {
        let _ = create.reply.send(Err(CreatePlayerErr::World(format!(
            "store write failed: {}",
            bounded_failure_detail(detail)
        ))));
    }
}

#[allow(clippy::too_many_arguments)]
fn refuse_before_submission(
    sends: Vec<ClassifiedSend>,
    effects: Vec<(DeliveryClass, Effect)>,
    failure_actor: Option<ObjId>,
    create: Option<CreateCompletion>,
    detail: &str,
    gateway: &Gateway,
    health: &StoreHealth,
    tx: &mpsc::WeakSender<WorldMsg>,
    llm: Option<&Arc<LlmService>>,
) {
    let has_abort_output = sends
        .iter()
        .any(|(class, _, _)| matches!(class, DeliveryClass::BestEffort | DeliveryClass::OnAbort));
    release_classified_sends(gateway, sends, DeliveryOutcome::Aborted);
    release_classified_effects(health.mode, effects, DeliveryOutcome::Aborted, tx, llm);
    if let Some(actor) = failure_actor
        && !has_abort_output
    {
        send_sys(
            gateway,
            actor,
            &format!(
                "Your action was not committed ({}). Please retry against the current state.",
                bounded_failure_detail(detail)
            ),
        );
    }
    if let Some(create) = create {
        let _ = create.reply.send(Err(CreatePlayerErr::World(format!(
            "action precondition failed: {}",
            bounded_failure_detail(detail)
        ))));
    }
}

fn bounded_failure_detail(detail: &str) -> &str {
    const MAX_BYTES: usize = 512;
    if detail.len() <= MAX_BYTES {
        return detail;
    }
    let mut boundary = MAX_BYTES;
    while !detail.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &detail[..boundary]
}

fn send_commit_failure(gateway: &Gateway, actor: ObjId, detail: &str) {
    send_sys(
        gateway,
        actor,
        &format!(
            "The database rejected your last action ({}). Its success output and external effects were suppressed; the world is now integrity-frozen.",
            bounded_failure_detail(detail)
        ),
    );
}

/// Spawn one detached task per drained [`Effect`]; each completion re-enters
/// the world as a typed [`WorldCommand`] via [`Inbound::Apply`].
///
/// Each in-flight effect holds a strong sender (upgraded from the loop's
/// weak one), so the world task outlives every pending completion; once the
/// last handle *and* the last effect are gone, the channel closes and the
/// task ends.
fn spawn_effect_list(
    effects: Vec<Effect>,
    tx: &mpsc::WeakSender<WorldMsg>,
    llm: Option<&Arc<LlmService>>,
) {
    for effect in effects {
        match effect {
            Effect::SlowTest {
                actor,
                correlation,
                seconds,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(correlation, "world channel closing; slowtest dropped");
                    continue;
                };
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                    let done = WorldCommand::SlowTestDone {
                        actor,
                        correlation,
                        text: format!(
                            "SLOWTEST: fake async call #{correlation} finished after \
                             {seconds}s -- the world loop never blocked."
                        ),
                    };
                    if tx
                        .send(WorldMsg::Inbound(Inbound::Apply(done)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(correlation, "slowtest completion after shutdown dropped");
                    }
                });
            }
            // M7 LLM re-entry seam (mush-core `Effect::ModuleAsync`, treaty
            // ruling 3): a module snapshotted context synchronously and
            // deferred its async work; run it OFF the world loop against the
            // service handle mushd holds, then re-enter as the typed
            // `WorldCommand::ModuleAsyncDone`. Routing is by module name —
            // mush-core never interprets the payload.
            Effect::ModuleAsync {
                module,
                actor,
                correlation,
                payload,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(%module, correlation, "world channel closing; module-async dropped");
                    continue;
                };
                // worldgen `@gen`: run the LLM room generator off-loop and
                // re-enter with the serialized GenOutcome. When the service is
                // absent (repl / a test rig with no LLM) the effect is dropped
                // — but the real server always constructs one, even when
                // `[llm].enabled = false` (its kill switch is engaged, so the
                // generator re-enters a "disabled" outcome instead). The
                // `.as_str()` borrow ends with the condition, so `module` is
                // free to move into the completion below.
                if module.as_str() == mush_worldgen::NS {
                    let Some(llm) = llm.cloned() else {
                        tracing::warn!(
                            correlation,
                            "worldgen Effect::ModuleAsync dropped: no LlmService on this world task"
                        );
                        continue;
                    };
                    tokio::spawn(async move {
                        let result = mush_worldgen::run_worldgen_async(&llm, &payload).await;
                        let done = WorldCommand::ModuleAsyncDone {
                            module,
                            actor,
                            correlation,
                            payload: result,
                        };
                        if tx
                            .send(WorldMsg::Inbound(Inbound::Apply(done)))
                            .await
                            .is_err()
                        {
                            tracing::debug!(
                                correlation,
                                "worldgen completion after shutdown dropped"
                            );
                        }
                    });
                } else if module.as_str() == mush_npc::NS {
                    // M8 NPC agent turn: run the bounded tool loop off-loop
                    // against the shared LlmService, then re-enter with the
                    // serialized TurnOutcome (abilities execute on-loop in the
                    // module's completion, via its grounding adapter).
                    let Some(llm) = llm.cloned() else {
                        tracing::warn!(
                            correlation,
                            "npc Effect::ModuleAsync dropped: no LlmService on this world task"
                        );
                        continue;
                    };
                    tokio::spawn(async move {
                        let result = mush_npc::run_npc_turn_async(&llm, &payload).await;
                        let done = WorldCommand::ModuleAsyncDone {
                            module,
                            actor,
                            correlation,
                            payload: result,
                        };
                        if tx
                            .send(WorldMsg::Inbound(Inbound::Apply(done)))
                            .await
                            .is_err()
                        {
                            tracing::debug!(correlation, "npc completion after shutdown dropped");
                        }
                    });
                } else if module.as_str() == mush_weather::NS {
                    // M10 weather flavor top-up: `@weather/generate` runs the
                    // off-loop flavor generator against the shared LlmService
                    // (cheap Ambience route), then re-enters with the serialized
                    // FlavorGenOutcome (PENDING lines stored on-loop in the
                    // module's completion). Never blocks the tick.
                    let Some(llm) = llm.cloned() else {
                        tracing::warn!(
                            correlation,
                            "weather Effect::ModuleAsync dropped: no LlmService on this world task"
                        );
                        continue;
                    };
                    tokio::spawn(async move {
                        let result = mush_weather::run_weather_async(&llm, &payload).await;
                        let done = WorldCommand::ModuleAsyncDone {
                            module,
                            actor,
                            correlation,
                            payload: result,
                        };
                        if tx
                            .send(WorldMsg::Inbound(Inbound::Apply(done)))
                            .await
                            .is_err()
                        {
                            tracing::debug!(
                                correlation,
                                "weather completion after shutdown dropped"
                            );
                        }
                    });
                } else if module.as_str() == ruleset_wod::NS {
                    // M13 WoD Storyteller adjudication: run the ruling /
                    // interpretation pass off-loop against the shared
                    // LlmService, then re-enter with the serialized StResult
                    // (the ruling is re-validated + the outcome applied on-loop
                    // in the ruleset's completion). Never blocks the tick.
                    let Some(llm) = llm.cloned() else {
                        tracing::warn!(
                            correlation,
                            "wod Effect::ModuleAsync dropped: no LlmService on this world task"
                        );
                        continue;
                    };
                    tokio::spawn(async move {
                        let result = ruleset_wod::run_st_async(&llm, &payload).await;
                        let done = WorldCommand::ModuleAsyncDone {
                            module,
                            actor,
                            correlation,
                            payload: result,
                        };
                        if tx
                            .send(WorldMsg::Inbound(Inbound::Apply(done)))
                            .await
                            .is_err()
                        {
                            tracing::debug!(
                                correlation,
                                "wod st completion after shutdown dropped"
                            );
                        }
                    });
                } else {
                    tracing::warn!(
                        %module,
                        correlation,
                        "Effect::ModuleAsync for an unrouted module dropped"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use mush_core::{
        ActionError, ArgShape, AsyncCompletion, AuthoritativeBundle, AuthoritativeBundleDraft,
        CanonicalHash, CanonicalId, CmdError, CommandSpec, CommitToken as DurableCommitToken,
        ContextOutcome, Ctx, Deferred, DefinitionDescriptor, DefinitionId, DefinitionKind,
        DefinitionRegistryBuilder, EventHook, GOD, GameModule, GuardSet, HookPoint, IdempotencyKey,
        IdempotencyScope, InputContext, LIMBO, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite,
        OwnerId, PreparedCausalRef, PreparedTxn, PrincipalId, QueryHandler, Registrar, RequestUid,
        RevisionKey, RevisionKind, RevisionSet, RevisionStamp, RevisionValidator, RulesValue,
        ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey,
        TickHandler, WorldEventKind, WorldObserver,
    };
    use mush_proto::{InputMode, PromptSpec};
    use mush_store::AuthoritativeStore;

    struct BlackholeGateway;

    impl GatewaySink for BlackholeGateway {
        fn transmit(&self, _sends: Vec<(SendTarget, Arc<OutputEvent>)>) {}

        fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

        fn is_socket(&self) -> bool {
            false
        }

        fn close_session(&self, _char: u64, _cause: mush_ipc::DetachCause) {}

        fn close_web_sessions(
            &self,
            _chars: Vec<u64>,
            _notice: Option<OutputEvent>,
            _cause: mush_ipc::DetachCause,
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

    #[derive(Default)]
    struct RecordingGateway {
        sends: Mutex<Vec<(SendTarget, Arc<OutputEvent>)>>,
    }

    impl RecordingGateway {
        fn reply_text(&self, id: u64) -> Option<String> {
            self.sends
                .lock()
                .expect("recording gateway poisoned")
                .iter()
                .find_map(|(_, event)| match event.as_ref() {
                    OutputEvent::Reply {
                        id: reply_id,
                        event,
                    } if *reply_id == id => match event.as_ref() {
                        OutputEvent::Sys(text) => Some(text.to_plain_text()),
                        other => Some(format!("{other:?}")),
                    },
                    _ => None,
                })
        }
    }

    impl GatewaySink for RecordingGateway {
        fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
            self.sends
                .lock()
                .expect("recording gateway poisoned")
                .extend(sends);
        }

        fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

        fn is_socket(&self) -> bool {
            false
        }

        fn close_session(&self, _char: u64, _cause: mush_ipc::DetachCause) {}

        fn close_web_sessions(
            &self,
            _chars: Vec<u64>,
            _notice: Option<OutputEvent>,
            _cause: mush_ipc::DetachCause,
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

    #[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PreparedLaneState {
        amount: u8,
    }

    type PreparedLaneHandle = StateFamilyHandle<u64, PreparedLaneState>;

    #[derive(Clone)]
    struct PreparedLaneGeneration(u64);

    struct PreparedLaneRevision;

    impl RevisionValidator for PreparedLaneRevision {
        fn current_generation(
            &self,
            world: &World,
            _key: &RevisionKey,
        ) -> Result<Option<u64>, ActionError> {
            Ok(world
                .module_state::<PreparedLaneGeneration>()
                .map(|generation| generation.0))
        }
    }

    struct PreparedLaneModule {
        captured: Arc<Mutex<Option<PreparedLaneHandle>>>,
    }

    impl GameModule for PreparedLaneModule {
        fn name(&self) -> &'static str {
            "prepared_lane_test"
        }

        fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
            registrar.state(PreparedLaneGeneration(1));
            registrar.revision_validator(
                RevisionKind::new(CanonicalId::new("prepared_lane").expect("static revision kind")),
                PreparedLaneRevision,
            )?;
            let descriptor = StateFamilyDescriptor::new(
                StateFamilyId::new("receipt_state").expect("static family id"),
                CanonicalId::new("u64.decimal.v1").expect("static codec id"),
                CanonicalId::new("prepared_lane_state.v1").expect("static schema id"),
                1,
                CanonicalId::new("prepared_lane_state.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("prepared_lane_state.range.v1").expect("static validator id"),
                CanonicalId::new("prepared_lane_state.json.v1").expect("static serializer id"),
            )
            .expect("static state-family descriptor");
            let handle = registrar.state_family(
                descriptor,
                |key: &u64| Ok(key.to_string()),
                |value: &serde_json::Value| {
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
                },
                |_value: &PreparedLaneState| Ok(()),
                |value: &PreparedLaneState| Ok(serde_json::json!({ "amount": value.amount })),
            )?;
            *self.captured.lock().expect("capture mutex poisoned") = Some(handle);
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            ctx: &mut ModuleCtx<'_>,
            _data: ModuleBootData,
        ) -> Result<(), mush_core::ModError> {
            ctx.persist(ModuleOp {
                ns: "prepared_lane_test".into(),
                op: ModuleWrite::JsonSet {
                    key: "late_legacy_work".into(),
                    value: serde_json::json!({ "queued": true }),
                },
            });
            Ok(())
        }
    }

    fn prepared_lane_fixture() -> (Engine, PreparedLaneHandle) {
        let captured = Arc::new(Mutex::new(None));
        let module = PreparedLaneModule {
            captured: Arc::clone(&captured),
        };
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(module)],
        )
        .expect("prepared-lane test module composes");
        // The minimal world's seed objects are unrelated legacy dirtiness;
        // prepared action state must enter the dedicated persistence lane.
        engine.world.take_dirty();
        let handle = captured
            .lock()
            .expect("capture mutex poisoned")
            .clone()
            .expect("registration captured state-family handle");
        (engine, handle)
    }

    fn prepared_lane_guards(engine: &Engine, generation: u64) -> GuardSet {
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: RevisionKey {
                owner: OwnerId::new("prepared_lane_test").expect("static owner id"),
                kind: RevisionKind::new(
                    CanonicalId::new("prepared_lane").expect("static revision kind"),
                ),
                subject: SubjectKey::Object { id: GOD },
                facet: None,
            },
            generation,
        }])
        .expect("single revision stamp is canonical");
        engine
            .registry
            .server_guard_set_builder(revisions)
            .expect("composed registry can build guards")
            .finish()
            .expect("single revision guard is valid")
    }

    fn empty_guards(engine: &Engine) -> GuardSet {
        engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .expect("composed registry can build guards")
            .finish()
            .expect("empty guard set is valid")
    }

    fn prepare_lane_installation(
        engine: &mut Engine,
        handle: &PreparedLaneHandle,
        key: u64,
        amount: u8,
    ) -> PreparedInstallation {
        let mut transaction = engine
            .action_transaction(handle)
            .expect("test transaction begins");
        transaction
            .insert(handle, &key, PreparedLaneState { amount })
            .expect("test state is valid");
        let prepared = transaction.prepare().expect("test transaction prepares");
        let (_module_ops, installation) = prepared.into_parts();
        installation
    }

    fn prepared_lane_bundle(prepared: &PreparedTxn, identity: u8) -> AuthoritativeBundle {
        let owner = OwnerId::new("prepared_lane_test").expect("static owner");
        let mut definitions = DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("set_receipt_state").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome_definition = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("outcome").unwrap(),
                    DefinitionId::new("receipt_state_set").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let request = RequestUid::from_bytes([identity; 16]);
        let principal = PrincipalId::Object { id: GOD };
        AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: DurableCommitToken::from_bytes([identity.wrapping_add(1); 16]),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal.clone(),
                    owner: owner.clone(),
                    namespace: CanonicalId::new("set_receipt_state").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes([identity.wrapping_add(2); 32]),
            request_hash: CanonicalHash::from_bytes([identity.wrapping_add(3); 32]),
            plan_hash: CanonicalHash::from_bytes([identity.wrapping_add(4); 32]),
            action: action.clone(),
            provider: owner,
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action, outcome_definition.clone()],
            writes: prepared.authoritative_replay_writes(),
            outcome: RulesValue::new(outcome_definition, serde_json::json!({ "accepted": true }))
                .unwrap(),
            eligible_witnesses: vec![principal],
            durable_outbox: vec![],
        })
        .unwrap()
    }

    fn pending_with_prepared_installation(
        serial: u64,
        installation: PreparedInstallation,
    ) -> PendingCommit {
        let (_receipt_tx, receipt) = oneshot::channel();
        PendingCommit {
            token: CommitToken::from_serial(serial),
            receipt: PendingReceipt::Legacy {
                expected_serial: serial,
                receiver: receipt,
            },
            sends: Vec::new(),
            effects: Vec::new(),
            failure_actor: None,
            create: None,
            prepared_installation: Some(installation),
            barriers: Vec::new(),
            coalesced_reattach: None,
            exit_after_ack: false,
        }
    }

    #[test]
    fn legacy_module_writes_cannot_enter_the_action_state_keyspace() {
        let ordinary = ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::JsonSet {
                key: "ordinary".into(),
                value: serde_json::json!(true),
            },
        };
        assert!(legacy_reserved_action_state_violation(&[ordinary]).is_none());

        for op in [
            ModuleWrite::JsonSet {
                key: "action-state/v1/family/1".into(),
                value: serde_json::json!(true),
            },
            ModuleWrite::Del {
                key: "action-state/v2/family/1".into(),
            },
            ModuleWrite::StreamAppend {
                key: "action-state/future".into(),
                value: serde_json::json!(true),
                maxlen: 1,
            },
        ] {
            let violation = legacy_reserved_action_state_violation(&[ModuleOp {
                ns: "toy".into(),
                op,
            }]);
            assert!(
                violation
                    .as_deref()
                    .is_some_and(|detail| detail.contains("coordinator-reserved"))
            );
        }
    }

    #[derive(Clone, Debug, Default)]
    struct GateProbeState {
        mutations: usize,
    }

    #[derive(Default)]
    struct GateProbeCounts {
        command: AtomicUsize,
        input_context: AtomicUsize,
        connection_hook: AtomicUsize,
        observer: AtomicUsize,
        async_apply: AtomicUsize,
        timer: AtomicUsize,
    }

    impl GateProbeCounts {
        fn total(&self) -> usize {
            [
                &self.command,
                &self.input_context,
                &self.connection_hook,
                &self.observer,
                &self.async_apply,
                &self.timer,
            ]
            .into_iter()
            .map(|counter| counter.load(AtomicOrdering::SeqCst))
            .sum()
        }
    }

    fn record_probe_mutation(ctx: &mut Ctx<'_>, counter: &AtomicUsize, key: &str) {
        counter.fetch_add(1, AtomicOrdering::SeqCst);
        ctx.state_mut::<GateProbeState>()
            .expect("gate probe state installed")
            .mutations += 1;
        ctx.persist(ModuleOp {
            ns: "gate_probe".into(),
            op: ModuleWrite::JsonSet {
                key: key.into(),
                value: serde_json::json!({ "executed": true }),
            },
        });
    }

    struct ProbeConnected(Arc<GateProbeCounts>);

    impl EventHook for ProbeConnected {
        fn connected(&self, ctx: &mut Ctx<'_>, _actor: ObjId) {
            record_probe_mutation(ctx, &self.0.connection_hook, "connection_hook");
        }
    }

    struct ProbeObserver(Arc<GateProbeCounts>);

    impl WorldObserver for ProbeObserver {
        fn on_event(&self, ctx: &mut Ctx<'_>, _ev: &WorldEvent) {
            record_probe_mutation(ctx, &self.0.observer, "observer");
        }
    }

    struct ProbeAsync(Arc<GateProbeCounts>);

    impl AsyncCompletion for ProbeAsync {
        fn on_complete(&self, ctx: &mut Ctx<'_>, _correlation: u64, _payload: &str) {
            record_probe_mutation(ctx, &self.0.async_apply, "async_apply");
        }
    }

    struct ProbeTick(Arc<GateProbeCounts>);

    impl TickHandler for ProbeTick {
        fn tick(&self, ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
            record_probe_mutation(ctx, &self.0.timer, "timer");
            ctx.out.defer(Deferred::Effect(Effect::SlowTest {
                actor: GOD,
                correlation: 99,
                seconds: 0,
            }));
        }
    }

    struct ProbeQuery {
        input_actor: ObjId,
    }

    impl QueryHandler for ProbeQuery {
        fn query(
            &self,
            world: &World,
            _actor: ObjId,
            _params: &serde_json::Value,
        ) -> Result<OutputEvent, RichText> {
            let mutations = world
                .module_state::<GateProbeState>()
                .expect("gate probe state installed")
                .mutations;
            Ok(OutputEvent::Sys(RichText::plain(format!(
                "objects={} mutations={mutations} input_depth={}",
                world.iter().count(),
                world.input_depth(self.input_actor)
            ))))
        }
    }

    struct GateProbeModule {
        counts: Arc<GateProbeCounts>,
        input_actor: ObjId,
    }

    impl GameModule for GateProbeModule {
        fn name(&self) -> &'static str {
            "gate_probe"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
            reg.state(GateProbeState::default());

            let counts = Arc::clone(&self.counts);
            reg.command(
                CommandSpec {
                    name: "gate-mutate".into(),
                    shape: ArgShape::None,
                    help: "test-only gate mutation".into(),
                    ..CommandSpec::default()
                },
                Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                    counts.command.fetch_add(1, AtomicOrdering::SeqCst);
                    let created = ctx
                        .w
                        .create(ObjKind::Thing, "gate-leak", ctx.actor, Some(LIMBO))
                        .map_err(|error| CmdError::msg(error.to_string()))?;
                    ctx.emit_world_event(&WorldEvent::ObjCreated {
                        obj: created,
                        kind: ObjKind::Thing,
                        creator: ctx.actor,
                    });
                    ctx.state_mut::<GateProbeState>()
                        .expect("gate probe state installed")
                        .mutations += 1;
                    ctx.persist(ModuleOp {
                        ns: "gate_probe".into(),
                        op: ModuleWrite::JsonSet {
                            key: "command".into(),
                            value: serde_json::json!({ "executed": true }),
                        },
                    });
                    ctx.out.defer(Deferred::Effect(Effect::SlowTest {
                        actor: ctx.actor,
                        correlation: 7,
                        seconds: 0,
                    }));
                    Ok(())
                }),
            )?;
            reg.hook(
                HookPoint::Connected,
                Arc::new(ProbeConnected(Arc::clone(&self.counts))),
            );
            reg.subscribe(
                WorldEventKind::ObjCreated,
                Arc::new(ProbeObserver(Arc::clone(&self.counts))),
            );
            reg.tick(
                "due",
                TickCadence::Fast(Duration::from_millis(1)),
                Arc::new(ProbeTick(Arc::clone(&self.counts))),
            )?;
            reg.async_completion(Arc::new(ProbeAsync(Arc::clone(&self.counts))))?;
            reg.query_handler(
                "gate_probe.snapshot",
                Arc::new(ProbeQuery {
                    input_actor: self.input_actor,
                }),
            )?;
            Ok(())
        }
    }

    struct ProbeInputContext(Arc<GateProbeCounts>);

    impl InputContext for ProbeInputContext {
        fn prompt(&self, _world: &World) -> PromptSpec {
            PromptSpec {
                mode: InputMode::Normal,
                text: Some(RichText::plain("probe>")),
            }
        }

        fn feed(&mut self, ctx: &mut Ctx<'_>, _line: &str) -> ContextOutcome {
            record_probe_mutation(ctx, &self.0.input_context, "input_context");
            ContextOutcome::Consumed
        }
    }

    fn cmd(line: &str) -> Inbound {
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: line.to_string(),
        }
    }

    #[test]
    fn inbound_classifier_covers_every_variant() {
        let cases = [
            (cmd("look"), IngressClass::Command),
            (
                Inbound::Query {
                    conn: 1,
                    actor: GOD,
                    id: 7,
                    name: "who".into(),
                    args: serde_json::json!({}),
                },
                IngressClass::WorldRead,
            ),
            (
                Inbound::ConnUp {
                    conn: 1,
                    actor: GOD,
                    caps_structured: false,
                    from_linkdead: false,
                },
                IngressClass::ConnectionHooks,
            ),
            (
                Inbound::ConnDown {
                    conn: 1,
                    actor: GOD,
                    reason: DisconnectKind::Lost,
                },
                IngressClass::ConnectionHooks,
            ),
            (
                Inbound::SessionGone { actor: GOD },
                IngressClass::ConnectionHooks,
            ),
            (
                Inbound::Apply(WorldCommand::SlowTestDone {
                    actor: GOD,
                    correlation: 9,
                    text: "done".to_string(),
                }),
                IngressClass::AsyncApply,
            ),
            (
                Inbound::Control(ControlMsg::Shutdown),
                IngressClass::Shutdown,
            ),
            (
                Inbound::Control(ControlMsg::Dump),
                IngressClass::StaffMutation,
            ),
        ];
        for (msg, expected) in cases {
            assert_eq!(classify_inbound(&msg), expected);
        }
    }

    #[test]
    fn world_message_classifier_covers_every_variant() {
        assert_eq!(
            classify_world_msg(&WorldMsg::Inbound(cmd("look"))),
            IngressClass::Command
        );
        assert_eq!(
            classify_world_msg(&WorldMsg::ReAttach {
                recs: Vec::new().into_boxed_slice(),
            }),
            IngressClass::TransportBookkeeping
        );
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::CreatePlayer {
                name: "Newbie".to_string(),
                reply,
            }),
            IngressClass::WorldMutation
        );
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::NameFree {
                name: "Newbie".to_string(),
                reply,
            }),
            IngressClass::WorldRead
        );
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::InInputContext { actor: GOD, reply }),
            IngressClass::WorldRead
        );
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::ConnectionReady {
                conn: 1,
                actor: GOD,
                reply,
            }),
            IngressClass::WorldRead
        );
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::Barrier { reply }),
            IngressClass::OrderingBarrier
        );
    }

    #[test]
    fn only_transferred_semantic_or_lifecycle_custody_uses_deferred_slot() {
        assert!(!must_defer_behind_commit(&WorldMsg::Inbound(cmd("look"))));
        let (reply, _rx) = oneshot::channel();
        assert!(!must_defer_behind_commit(&WorldMsg::CreatePlayer {
            name: "RetryMe".to_string(),
            reply,
        }));
        assert!(must_defer_behind_commit(&WorldMsg::Inbound(
            Inbound::ConnDown {
                conn: 9,
                actor: GOD,
                reason: DisconnectKind::Quit,
            }
        )));
        assert!(must_defer_behind_commit(&WorldMsg::Inbound(
            Inbound::Apply(WorldCommand::SlowTestDone {
                actor: GOD,
                correlation: 1,
                text: "done".to_string(),
            })
        )));
    }

    #[test]
    fn writer_and_staff_classifiers_cover_every_variant() {
        assert_eq!(
            classify_writer_signal(&WriterSignal::StoreStatus {
                healthy: true,
                detail: String::new(),
            }),
            IngressClass::WriterRecovery
        );
        assert_eq!(
            classify_writer_signal(&WriterSignal::LedgerBreach {
                detail: "breach".to_string(),
                seal: None,
            }),
            IngressClass::WriterRecovery
        );
        assert_eq!(
            classify_writer_signal(&WriterSignal::DeterministicFailure {
                detail: "rejected".to_string(),
            }),
            IngressClass::WriterRecovery
        );

        let ip = "127.0.0.1".parse().unwrap();
        let cases = [
            (
                StaffOpKind::SetBan {
                    ip,
                    until_unix: None,
                    reason: "test".to_string(),
                },
                IngressClass::StaffMutation,
            ),
            (StaffOpKind::ClearBan { ip }, IngressClass::StaffMutation),
            (StaffOpKind::ListBans, IngressClass::StaffStatus),
            (StaffOpKind::Dump, IngressClass::StaffMutation),
            (StaffOpKind::Shutdown, IngressClass::Shutdown),
            (
                StaffOpKind::Boot {
                    target: GOD,
                    message: String::new(),
                },
                IngressClass::StaffMutation,
            ),
            (
                StaffOpKind::NewPassword {
                    target: GOD,
                    name: "God".to_string(),
                    by: "God".to_string(),
                    password: crate::staff::Secret("secret".to_string()),
                },
                IngressClass::StaffMutation,
            ),
            (
                StaffOpKind::AddSshKey {
                    actor: GOD,
                    openssh: "ssh-ed25519 AAAA".to_string(),
                    fingerprint: "SHA256:test".to_string(),
                },
                IngressClass::StaffMutation,
            ),
            (
                StaffOpKind::ListSshKeys { actor: GOD },
                IngressClass::StaffStatus,
            ),
            (
                StaffOpKind::DelSshKey {
                    actor: GOD,
                    which: "1".to_string(),
                },
                IngressClass::StaffMutation,
            ),
            (
                StaffOpKind::NewChar {
                    actor: GOD,
                    name: "Alt".to_string(),
                    max_characters: 2,
                },
                IngressClass::StaffMutation,
            ),
            (StaffOpKind::Ps, IngressClass::StaffStatus),
            (StaffOpKind::Stats, IngressClass::StaffStatus),
        ];
        for (op, expected) in cases {
            assert_eq!(classify_staff_op(&op), expected);
        }
    }

    #[test]
    fn writer_mailbox_coalesces_without_losing_mandatory_facts() {
        let (tx, mut rx) = writer_signal_channel();
        tx.store_status(false, "first failure".to_string());
        tx.store_status(true, String::new());
        tx.deterministic_failure("deterministic root cause".to_string());
        assert!(tx.ledger_breach("ledger freeze".to_string(), None));
        assert!(tx.ledger_breach(
            "exact resolution".to_string(),
            Some((
                SmolStr::new("econ"),
                SmolStr::new("log:cash:0"),
                "a".repeat(64),
                true,
            )),
        ));

        let signals: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(signals.iter().any(|signal| matches!(
            signal,
            WriterSignal::StoreStatus {
                healthy: true,
                detail
            } if detail.is_empty()
        )));
        assert!(signals.iter().any(|signal| matches!(
            signal,
            WriterSignal::DeterministicFailure { detail }
                if detail == "deterministic root cause"
        )));
        assert!(
            signals
                .iter()
                .any(|signal| matches!(signal, WriterSignal::LedgerBreach { seal: None, .. }))
        );
        assert!(signals.iter().any(|signal| matches!(
            signal,
            WriterSignal::LedgerBreach {
                seal: Some((ns, key, hash, true)),
                ..
            } if ns == "econ" && key == "log:cash:0" && hash == &"a".repeat(64)
        )));
        assert_eq!(signals.len(), 4);
    }

    #[test]
    fn writer_mailbox_refuses_exact_overflow_and_accepts_retry_after_drain() {
        let (tx, mut rx) = writer_signal_channel();
        for index in 0..crate::retained::MAX_RETAINED_SEALS {
            assert!(tx.ledger_breach(
                "resolved".to_string(),
                Some((
                    SmolStr::new("econ"),
                    SmolStr::new(format!("log:cash:{index}")),
                    format!("{index:064x}"),
                    false,
                )),
            ));
        }
        let overflow = (
            SmolStr::new("econ"),
            SmolStr::new("log:cash:overflow"),
            "f".repeat(64),
            true,
        );
        assert!(
            !tx.ledger_breach("overflow".to_string(), Some(overflow.clone())),
            "an exact identity must be refused, never shed, when custody is full"
        );

        let first_drain: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            first_drain
                .iter()
                .filter(|signal| matches!(signal, WriterSignal::LedgerBreach { seal: Some(_), .. }))
                .count(),
            crate::retained::MAX_RETAINED_SEALS
        );
        assert!(first_drain.iter().any(|signal| matches!(
            signal,
            WriterSignal::LedgerBreach { seal: None, detail }
                if detail.contains("mailbox saturated")
        )));

        assert!(tx.ledger_breach("overflow retry".to_string(), Some(overflow.clone())));
        let retried = rx.try_recv().expect("retry must arm a fresh wake");
        assert!(matches!(
            retried,
            WriterSignal::LedgerBreach {
                seal: Some((ns, key, hash, true)),
                ..
            } if ns == overflow.0 && key == overflow.1 && hash == overflow.2
        ));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn hard_modes_admit_only_safe_reads_ordering_recovery_and_shutdown() {
        const ALL_CLASSES: [IngressClass; 13] = [
            IngressClass::WorldRead,
            IngressClass::OrderingBarrier,
            IngressClass::Command,
            IngressClass::TransportBookkeeping,
            IngressClass::ConnectionHooks,
            IngressClass::WorldMutation,
            IngressClass::AsyncApply,
            IngressClass::Timer,
            IngressClass::WriterRecovery,
            IngressClass::StaffStatus,
            IngressClass::StaffMutation,
            IngressClass::ExternalEffect,
            IngressClass::Shutdown,
        ];
        let modes = [
            WorldMode::Booting,
            WorldMode::Recovering { incident: None },
            WorldMode::CommitPending {
                token: CommitToken::from_serial(7),
            },
            WorldMode::IntegrityFrozen {
                incident: IncidentId(11),
            },
        ];
        for mode in modes {
            for class in ALL_CLASSES {
                let should_allow = matches!(
                    class,
                    IngressClass::OrderingBarrier
                        | IngressClass::TransportBookkeeping
                        | IngressClass::WriterRecovery
                        | IngressClass::StaffStatus
                        | IngressClass::Shutdown
                ) || (class == IngressClass::WorldRead
                    && !matches!(mode, WorldMode::CommitPending { .. }));
                assert_eq!(
                    admission_for(mode, class) == Admission::Allow,
                    should_allow,
                    "mode={mode:?}, class={class:?}"
                );
            }
        }
    }

    #[test]
    fn commit_pending_world_reads_are_typed_refusals_but_ordering_stays_live() {
        let pending = WorldMode::CommitPending {
            token: CommitToken::from_serial(17),
        };
        assert_eq!(
            admission_for(pending, IngressClass::WorldRead),
            Admission::Refuse(RefusalReason::CommitPending)
        );
        for class in [
            IngressClass::OrderingBarrier,
            IngressClass::TransportBookkeeping,
            IngressClass::WriterRecovery,
            IngressClass::StaffStatus,
            IngressClass::Shutdown,
        ] {
            assert_eq!(admission_for(pending, class), Admission::Allow);
        }
    }

    async fn assert_live_hard_mode_gate(mode: WorldMode) {
        let counts = Arc::new(GateProbeCounts::default());
        let mut world = World::new_minimal();
        let input_actor = world
            .create(ObjKind::Player, "InputProbe", GOD, Some(LIMBO))
            .expect("input probe player");
        world.push_input_context(
            input_actor,
            Some("gate_probe".into()),
            Box::new(ProbeInputContext(Arc::clone(&counts))),
        );
        // Make the registered 1 ms tick overdue, then clear setup dirtiness so
        // any store write below can only have come from a forbidden seam.
        world.set_now(unix_now_s().saturating_sub(1));
        let _ = world.take_dirty();
        let engine = Engine::with_modules(
            world,
            base_registry(),
            vec![Box::new(GateProbeModule {
                counts: Arc::clone(&counts),
                input_actor,
            })],
        )
        .expect("gate probe module composes");

        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        let staff = Arc::new(StaffQueue::new());
        staff.push(StaffOp {
            actor: GOD,
            kind: StaffOpKind::SetBan {
                ip: "127.0.0.77".parse().expect("test IP"),
                until_unix: None,
                reason: "must remain blocked".to_string(),
            },
        });
        let recorder = Arc::new(RecordingGateway::default());
        let sink = Arc::clone(&recorder);
        let handles = spawn_persist_with_gateway_mode(
            engine,
            ServerDeps {
                store: Arc::clone(&store),
                guard: Arc::new(IpGuard::new()),
                staff,
                llm: None,
                metrics: Arc::new(Metrics::new()),
                quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
            },
            Some(mode),
            move |_| -> Gateway { sink },
        );
        let PersistHandles {
            handle,
            world_join,
            writer_join,
            ..
        } = handles;

        // Ordinary command handler + observer + deferred effect.
        assert!(
            handle
                .inbound(Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: "gate-mutate".to_string(),
                })
                .await
        );
        // The same structural command class while an input context owns the
        // line: admission must happen before the context's `feed` callback.
        assert!(
            handle
                .inbound(Inbound::Cmd {
                    conn: 2,
                    actor: input_actor,
                    line: "captured payload".to_string(),
                })
                .await
        );
        // Connection lifecycle hooks, typed world mutation, async completion,
        // and staff control all enter through distinct production seams.
        assert!(
            handle
                .inbound(Inbound::ConnUp {
                    conn: 3,
                    actor: input_actor,
                    caps_structured: false,
                    from_linkdead: false,
                })
                .await
        );
        assert_eq!(
            handle.create_player("BlockedPlayer").await,
            Err(CreatePlayerErr::StoreUnavailable),
            "CreatePlayer must be refused in {mode:?}"
        );
        assert!(
            handle
                .inbound(Inbound::Apply(WorldCommand::ModuleAsyncDone {
                    module: "gate_probe".into(),
                    actor: GOD,
                    correlation: 11,
                    payload: "{}".to_string(),
                }))
                .await
        );
        assert!(handle.inbound(Inbound::Control(ControlMsg::Dump)).await);

        // Barriers are order bookkeeping, not gameplay reads. Two round trips
        // ensure the first iteration's staff-queue drain has also completed.
        assert!(
            handle.barrier().await,
            "first barrier remains live in {mode:?}"
        );
        assert!(
            handle.barrier().await,
            "second barrier remains live in {mode:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(
            counts.total(),
            0,
            "a forbidden engine/module callback executed in {mode:?}"
        );
        assert_eq!(
            handle.name_free("BlockedPlayer").await,
            Ok(true),
            "refused creation must not change the player index in {mode:?}"
        );
        assert_eq!(
            handle.in_input_context(input_actor).await,
            Ok(true),
            "refused input must not consume the context in {mode:?}"
        );

        let query_id = 700 + u64::from(input_actor.0);
        assert!(
            handle
                .inbound(Inbound::Query {
                    conn: 9,
                    actor: GOD,
                    id: query_id,
                    name: "gate_probe.snapshot".into(),
                    args: serde_json::json!({}),
                })
                .await
        );
        assert!(handle.barrier().await, "query delivery barrier in {mode:?}");
        assert_eq!(
            recorder.reply_text(query_id).as_deref(),
            Some("objects=3 mutations=0 input_depth=1"),
            "world and module snapshots changed in {mode:?}"
        );
        assert!(
            store.load_world().await.expect("read test world").is_none(),
            "forbidden world state reached the store in {mode:?}"
        );
        assert!(
            store
                .module_json_get("gate_probe", "command")
                .await
                .expect("read probe module doc")
                .is_none(),
            "forbidden module state reached the store in {mode:?}"
        );
        assert!(
            store.get_bans().await.expect("read test bans").is_empty(),
            "forbidden staff mutation reached the store in {mode:?}"
        );

        // Exercise the final external-effect launch gate independently of the
        // command/tick producers above. If it launches, the zero-delay test
        // effect re-enters this live channel as an Apply message.
        let (effect_tx, mut effect_rx) = mpsc::channel(1);
        let effect_weak = effect_tx.downgrade();
        release_classified_effects(
            mode,
            vec![(
                DeliveryClass::BestEffort,
                Effect::SlowTest {
                    actor: GOD,
                    correlation: 123,
                    seconds: 0,
                },
            )],
            DeliveryOutcome::NoWrite,
            &effect_weak,
            None,
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            matches!(effect_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "an external effect launched in {mode:?}"
        );

        // Transport-only reattach and shutdown remain live. An empty roster is
        // enough to prove the production message is admitted without adding
        // intentional connection bookkeeping to the state-equality checks.
        assert!(handle.reattach(Vec::new()).await);
        assert!(handle.barrier().await, "reattach ordering in {mode:?}");
        assert!(handle.inbound(Inbound::Control(ControlMsg::Shutdown)).await);
        tokio::time::timeout(Duration::from_secs(2), world_join)
            .await
            .expect("hard-mode world shutdown timed out")
            .expect("hard-mode world task panicked");
        tokio::time::timeout(Duration::from_secs(2), writer_join)
            .await
            .expect("hard-mode writer shutdown timed out")
            .expect("hard-mode writer task panicked");
    }

    #[tokio::test]
    async fn live_hard_modes_block_every_mutating_execution_seam() {
        for mode in [
            WorldMode::Booting,
            WorldMode::Recovering {
                incident: Some(IncidentId(41)),
            },
            WorldMode::IntegrityFrozen {
                incident: IncidentId(42),
            },
        ] {
            assert_live_hard_mode_gate(mode).await;
        }
    }

    fn pending_for_custody_test() -> PendingCommit {
        let (_receipt_tx, receipt) = oneshot::channel();
        PendingCommit {
            token: CommitToken::from_serial(7),
            receipt: PendingReceipt::Legacy {
                expected_serial: 7,
                receiver: receipt,
            },
            sends: vec![(
                DeliveryClass::OnCommit,
                SendTarget::Char(u64::from(GOD.0)),
                Arc::new(OutputEvent::Sys(RichText::plain("held output"))),
            )],
            effects: vec![(
                DeliveryClass::OnCommit,
                Effect::SlowTest {
                    actor: GOD,
                    correlation: 9,
                    seconds: 0,
                },
            )],
            failure_actor: Some(GOD),
            create: None,
            prepared_installation: None,
            barriers: Vec::new(),
            coalesced_reattach: None,
            exit_after_ack: false,
        }
    }

    #[tokio::test]
    async fn prepared_action_installs_only_after_its_exact_receipt() {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut transaction = engine
            .action_transaction(&handle)
            .expect("test transaction begins");
        transaction
            .insert(&handle, &41, PreparedLaneState { amount: 7 })
            .expect("test state is valid");
        let prepared = transaction.prepare().expect("test transaction prepares");
        let guards = empty_guards(&engine);
        engine
            .stage_prepared_action(prepared, guards)
            .expect("clean engine accepts prepared action");

        let (writer, mut writer_rx) = mpsc::channel(1);
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
            &mut serial,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
        )
        .await;
        let FlushOutcome::Queued {
            receipt: Some(receipt),
            prepared_installation: Some(installation),
        } = outcome
        else {
            panic!("prepared action must queue with its receipt and install image");
        };
        let StoreJob::Write {
            batch,
            notify: Some(notify),
        } = writer_rx.recv().await.expect("prepared write was queued")
        else {
            panic!("prepared lane must request an exact writer receipt");
        };
        assert_eq!(batch.serial, serial);
        assert_eq!(batch.module_ops.len(), 1);

        assert_eq!(engine.action_state().revision(), 0);
        assert!(engine.action_state().has_prepared());
        assert!(
            engine
                .action_state()
                .get(&handle, &41)
                .expect("state lookup succeeds")
                .is_none(),
            "queue acceptance must not expose the typed post-image"
        );

        notify
            .send(Ok(batch.serial))
            .expect("world still owns the exact receipt");
        let mut receipt = receipt;
        let ack = receipt.receive().await;
        let pending = pending_with_prepared_installation(batch.serial, installation);
        let gateway: Gateway = Arc::new(BlackholeGateway);
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);
        let completed = complete_pending_commit(
            pending,
            ack,
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
        );

        assert_eq!(completed.outcome, DeliveryOutcome::Committed);
        assert_eq!(engine.action_state().revision(), 1);
        assert!(!engine.action_state().has_prepared());
        assert_eq!(
            engine
                .action_state()
                .get(&handle, &41)
                .expect("state lookup succeeds"),
            Some(&PreparedLaneState { amount: 7 })
        );
        assert!(!persistence_integrity.is_latched());
    }

    #[tokio::test]
    async fn authoritative_prepared_action_queues_exact_bundle_and_installs_only_on_exact_receipt()
    {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut transaction = engine.action_transaction(&handle).unwrap();
        transaction
            .insert(&handle, &42, PreparedLaneState { amount: 9 })
            .unwrap();
        let prepared = transaction.prepare().unwrap();
        let bundle = prepared_lane_bundle(&prepared, 91);
        let expected_token = bundle.token();
        let expected_hash = bundle.bundle_hash();
        let guards = empty_guards(&engine);
        engine
            .stage_authoritative_action(prepared, guards, bundle.clone())
            .expect("matching bundle stages");

        let (writer, mut writer_rx) = mpsc::channel(1);
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
            &mut serial,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
        )
        .await;
        let FlushOutcome::Queued {
            receipt: Some(mut pending_receipt),
            prepared_installation: Some(installation),
        } = outcome
        else {
            panic!("authoritative action must queue exact receipt custody");
        };
        assert_eq!(serial, 0, "K3 tokens do not consume legacy serials");
        assert_eq!(
            pending_receipt.token(),
            CommitToken::from_authoritative(expected_token)
        );
        let StoreJob::Authoritative {
            bundle: queued,
            notify,
        } = writer_rx.recv().await.expect("exact bundle queued")
        else {
            panic!("K3 staging must not use legacy WriteBatch");
        };
        assert_eq!(queued.as_ref(), &bundle);

        let store = InMemoryStore::new();
        store
            .reserve_authoritative_intent((*queued).clone())
            .await
            .unwrap();
        let exact = store
            .reconcile_authoritative(expected_token, expected_hash)
            .await
            .unwrap();
        notify.send(Ok(exact)).unwrap();
        let ack = pending_receipt.receive().await;

        let mut pending = pending_with_prepared_installation(0, installation);
        pending.token = CommitToken::from_authoritative(expected_token);
        let gateway: Gateway = Arc::new(BlackholeGateway);
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);
        let completed = complete_pending_commit(
            pending,
            ack,
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
        );
        assert_eq!(completed.outcome, DeliveryOutcome::Committed);
        assert_eq!(
            engine.action_state().get(&handle, &42).unwrap(),
            Some(&PreparedLaneState { amount: 9 })
        );
        assert!(!persistence_integrity.is_latched());
    }

    #[test]
    fn definitive_pre_intent_refusal_aborts_typed_image_without_integrity_freeze() {
        let (mut engine, handle) = prepared_lane_fixture();
        let installation = prepare_lane_installation(&mut engine, &handle, 73, 12);
        let pending = pending_with_prepared_installation(9, installation);
        let gateway: Gateway = Arc::new(BlackholeGateway);
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);

        let completed = complete_pending_commit(
            pending,
            Err(AuthoritativeCommitFailure::Refused(
                "authoritative capacity refused before intent".to_owned(),
            )),
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
        );

        assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
        assert_eq!(engine.action_state().get(&handle, &73).unwrap(), None);
        assert!(!engine.action_state().has_prepared());
        assert!(!persistence_integrity.is_latched());
    }

    #[test]
    fn authoritative_stage_rejects_bundle_that_does_not_match_typed_post_images() {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut first = engine.action_transaction(&handle).unwrap();
        first
            .insert(&handle, &51, PreparedLaneState { amount: 1 })
            .unwrap();
        let first = first.prepare().unwrap();
        let mismatched_bundle = prepared_lane_bundle(&first, 101);
        let (_, first_installation) = first.into_parts();
        engine.abort_prepared_action(first_installation);

        let mut second = engine.action_transaction(&handle).unwrap();
        second
            .insert(&handle, &51, PreparedLaneState { amount: 2 })
            .unwrap();
        let second = second.prepare().unwrap();
        let guards = empty_guards(&engine);
        assert_eq!(
            engine.stage_authoritative_action(second, guards, mismatched_bundle),
            Err(mush_core::StagePreparedActionError::AuthoritativeWriteSetMismatch)
        );
        assert!(!engine.has_staged_action());
        assert!(!engine.action_state().has_prepared());
        assert_eq!(engine.action_state().revision(), 0);
        assert!(engine.action_transaction(&handle).is_ok());
    }

    #[tokio::test]
    async fn stale_prepared_action_is_refused_before_writer_custody() {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut transaction = engine
            .action_transaction(&handle)
            .expect("test transaction begins");
        transaction
            .insert(&handle, &43, PreparedLaneState { amount: 8 })
            .expect("test state is valid");
        let prepared = transaction.prepare().expect("test transaction prepares");
        let guards = prepared_lane_guards(&engine, 1);
        engine
            .stage_prepared_action(prepared, guards)
            .expect("clean engine accepts prepared action");

        engine
            .world
            .module_state_mut::<PreparedLaneGeneration>()
            .expect("generation state is installed")
            .0 = 2;
        assert!(
            !engine.has_legacy_persistence_work(),
            "revision changes used by pure validation are not legacy writes"
        );

        let (writer, mut writer_rx) = mpsc::channel(1);
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
            &mut serial,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
        )
        .await;

        match outcome {
            FlushOutcome::Refused(detail) => assert!(
                detail.contains("guards became stale"),
                "unexpected refusal: {detail}"
            ),
            _ => panic!("stale guards must refuse the prepared submission"),
        }
        assert_eq!(serial, 0, "a stale action must consume no commit serial");
        assert_eq!(health.mode, WorldMode::Writable);
        assert!(health.last_write_unix.is_none());
        assert!(
            matches!(writer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "no stale action write may reach writer custody"
        );
        assert!(!engine.has_staged_action());
        assert!(!engine.action_state().has_prepared());
        assert_eq!(engine.action_state().revision(), 0);
        assert!(
            engine
                .action_state()
                .get(&handle, &43)
                .expect("state lookup succeeds")
                .is_none()
        );
        assert!(
            engine.action_transaction(&handle).is_ok(),
            "refusal must release the typed-state reservation"
        );
    }

    #[test]
    fn prepared_action_receipt_mismatch_or_rejection_aborts_installation() {
        enum BadReceipt {
            WrongSerial,
            StoreRejected,
        }

        for bad_receipt in [BadReceipt::WrongSerial, BadReceipt::StoreRejected] {
            let (mut engine, handle) = prepared_lane_fixture();
            let installation = prepare_lane_installation(&mut engine, &handle, 17, 3);
            let pending = pending_with_prepared_installation(7, installation);
            let ack = match bad_receipt {
                BadReceipt::WrongSerial => Err(AuthoritativeCommitFailure::Integrity(
                    "commit receipt identity mismatch: expected serial 7, received 8".to_owned(),
                )),
                BadReceipt::StoreRejected => Err(AuthoritativeCommitFailure::Integrity(
                    "deterministic store rejection".to_owned(),
                )),
            };
            let gateway: Gateway = Arc::new(BlackholeGateway);
            let mut health = StoreHealth::new();
            let persistence_integrity = PersistenceIntegrityLatch::new();
            let (tx, _rx) = mpsc::channel(1);

            let completed = complete_pending_commit(
                pending,
                ack,
                &mut engine,
                &gateway,
                &mut health,
                &persistence_integrity,
                &tx.downgrade(),
                None,
            );

            assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
            assert_eq!(engine.action_state().revision(), 0);
            assert!(!engine.action_state().has_prepared());
            assert!(
                engine
                    .action_state()
                    .get(&handle, &17)
                    .expect("state lookup succeeds")
                    .is_none(),
                "a bad receipt must discard the complete typed post-image"
            );
            assert!(health.mode.integrity_frozen());
            assert!(persistence_integrity.is_latched());
            assert!(
                engine.action_transaction(&handle).is_ok(),
                "abort must release the state-store reservation"
            );
        }
    }

    #[tokio::test]
    async fn prepared_action_writer_loss_aborts_before_receipt_custody() {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut transaction = engine
            .action_transaction(&handle)
            .expect("test transaction begins");
        transaction
            .insert(&handle, &23, PreparedLaneState { amount: 5 })
            .expect("test state is valid");
        let prepared = transaction.prepare().expect("test transaction prepares");
        let guards = empty_guards(&engine);
        engine
            .stage_prepared_action(prepared, guards)
            .expect("clean engine accepts prepared action");

        let (writer, writer_rx) = mpsc::channel(1);
        drop(writer_rx);
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
            &mut serial,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
        )
        .await;

        assert!(matches!(outcome, FlushOutcome::WriterGone));
        assert_eq!(engine.action_state().revision(), 0);
        assert!(!engine.action_state().has_prepared());
        assert!(
            engine
                .action_state()
                .get(&handle, &23)
                .expect("state lookup succeeds")
                .is_none()
        );
        assert!(engine.action_transaction(&handle).is_ok());
    }

    #[tokio::test]
    async fn prepared_action_refuses_legacy_work_added_after_staging() {
        enum LateLegacyWork {
            WorldDirty,
            ModuleOp,
        }

        for late_work in [LateLegacyWork::WorldDirty, LateLegacyWork::ModuleOp] {
            let (mut engine, handle) = prepared_lane_fixture();
            let mut transaction = engine
                .action_transaction(&handle)
                .expect("test transaction begins");
            transaction
                .insert(&handle, &29, PreparedLaneState { amount: 9 })
                .expect("test state is valid");
            let prepared = transaction.prepare().expect("test transaction prepares");
            let guards = empty_guards(&engine);
            engine
                .stage_prepared_action(prepared, guards)
                .expect("clean engine accepts prepared action");

            match late_work {
                LateLegacyWork::WorldDirty => {
                    engine
                        .world
                        .create(ObjKind::Thing, "late dirty object", GOD, Some(LIMBO))
                        .expect("test object creation succeeds");
                }
                LateLegacyWork::ModuleOp => {
                    let mut out = Outbox::new();
                    let mut effects = Effects::new();
                    engine
                        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut effects)
                        .expect("test module queues its boot persistence op");
                }
            }
            assert!(engine.has_legacy_persistence_work());

            let (writer, mut writer_rx) = mpsc::channel(1);
            let mut serial = 0;
            let mut health = StoreHealth::new();
            let metrics = Metrics::new();
            let mut retained = RetainedSeals::new();
            let high_water = AtomicU64::new(0);
            let outcome = flush_prepared_action(
                &mut engine,
                &writer,
                &mut serial,
                &mut health,
                &metrics,
                &mut retained,
                &high_water,
            )
            .await;

            match outcome {
                FlushOutcome::Rejected(detail) => assert!(
                    detail.contains("alongside legacy persistence work"),
                    "unexpected refusal: {detail}"
                ),
                _ => panic!("late legacy work must reject the prepared submission"),
            }
            assert_eq!(serial, 0, "a refused mixed step must consume no serial");
            assert!(
                matches!(writer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
                "no prefix of a mixed step may reach writer custody"
            );
            assert!(!engine.has_staged_action());
            assert!(!engine.action_state().has_prepared());
            assert_eq!(engine.action_state().revision(), 0);
            assert!(
                engine
                    .action_state()
                    .get(&handle, &29)
                    .expect("state lookup succeeds")
                    .is_none()
            );
            assert!(
                engine.has_legacy_persistence_work(),
                "the rejected prepared image must not discard independent legacy work"
            );
        }
    }

    #[tokio::test]
    async fn aborted_exact_receipt_refuses_barrier_and_suppresses_held_success() {
        let mut pending = pending_for_custody_test();
        let (barrier_tx, barrier_rx) = oneshot::channel();
        assert!(pending.retain_barrier(barrier_tx));

        let mut engine = Engine::new(World::new_minimal(), base_registry());
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);
        let completed = complete_pending_commit(
            pending,
            Err(AuthoritativeCommitFailure::Integrity(
                "deterministic rejection".to_owned(),
            )),
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
        );
        finish_pending_operational(&mut engine, &gateway, completed);

        assert_eq!(
            barrier_rx.await,
            Ok(Err(WorldReadError::Unavailable)),
            "an aborted receipt must never be promoted into a barrier ack"
        );
        let sends = recorder.sends.lock().expect("recording gateway poisoned");
        assert!(
            sends.iter().any(|(_, event)| matches!(
                event.as_ref(),
                OutputEvent::Sys(text)
                    if text.to_plain_text().contains("database rejected your last action")
            )),
            "the requester must receive the commit-failure refusal"
        );
        assert!(
            sends.iter().all(|(_, event)| !matches!(
                event.as_ref(),
                OutputEvent::Sys(text) if text.to_plain_text() == "held output"
            )),
            "a deterministic receipt failure must suppress held OnCommit success"
        );
    }

    #[test]
    fn prolonged_commit_pending_operational_traffic_has_fixed_custody() {
        let mut pending = pending_for_custody_test();
        let baseline_sends = pending.sends.len();
        let baseline_effects = pending.effects.len();

        // Repeated full snapshots replace one another; they never append room
        // renders to the authoritative batch while its receipt is unresolved.
        for conn in 1..=10_000_u64 {
            let replaced = pending
                .coalesce_reattach(
                    vec![ReAttachRec {
                        conn,
                        actor: GOD,
                        caps_structured: conn % 2 == 0,
                    }]
                    .into_boxed_slice(),
                )
                .expect("one-record roster is within the protocol cap");
            assert_eq!(replaced, conn > 1);
        }

        let mut refused = 0;
        for _ in 0..(MAX_PENDING_BARRIERS + 10_000) {
            let (reply, _rx) = oneshot::channel();
            if !pending.retain_barrier(reply) {
                refused += 1;
            }
        }

        let custody = pending.custody();
        assert_eq!(custody.barriers, MAX_PENDING_BARRIERS);
        assert_eq!(custody.reattach_records, 1);
        assert!(custody.operational_bytes <= MAX_PENDING_OPERATIONAL_BYTES);
        assert_eq!(custody.sends, baseline_sends);
        assert_eq!(custody.effects, baseline_effects);
        assert_eq!(refused, 10_000);
        assert_eq!(
            pending
                .coalesced_reattach
                .as_deref()
                .expect("latest roster")[0]
                .conn,
            10_000,
            "only the newest full snapshot remains"
        );
    }

    #[test]
    fn oversized_pending_reattach_is_atomic_and_preserves_previous_snapshot() {
        let mut pending = pending_for_custody_test();
        pending
            .coalesce_reattach(
                vec![ReAttachRec {
                    conn: 41,
                    actor: GOD,
                    caps_structured: false,
                }]
                .into_boxed_slice(),
            )
            .expect("initial bounded roster");

        let oversized = (0..=MAX_PENDING_REATTACH_RECORDS)
            .map(|i| ReAttachRec {
                conn: u64::try_from(i).expect("test index fits u64") + 100,
                actor: GOD,
                caps_structured: true,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let (_rejected, overload) = pending
            .coalesce_reattach(oversized)
            .expect_err("oversized roster must not enter pending custody");
        assert_eq!(overload.records, MAX_PENDING_REATTACH_RECORDS + 1);
        assert_eq!(
            pending.coalesced_reattach.as_deref().expect("old roster")[0].conn,
            41,
            "no prefix of a malformed replacement may reach server-side custody"
        );
    }

    #[tokio::test]
    async fn world_handle_refuses_oversized_reattach_before_enqueue() {
        let (tx, mut rx) = mpsc::channel(2);
        let handle = WorldHandle { tx, shutdown: None };
        let mut sparse = Vec::with_capacity(100_000);
        sparse.push(ReAttachRec {
            conn: 41,
            actor: GOD,
            caps_structured: false,
        });
        assert!(handle.reattach(sparse).await);
        let WorldMsg::ReAttach { recs } = rx.recv().await.expect("bounded roster enqueued") else {
            panic!("expected reattach message");
        };
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].conn, 41);
        // The message type is `Box<[ReAttachRec]>`, so the caller's 100,000
        // element spare Vec capacity cannot cross into channel custody.
        assert_eq!(
            std::mem::size_of_val(recs.as_ref()),
            std::mem::size_of::<ReAttachRec>()
        );

        let oversized = (0..=MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS)
            .map(|i| ReAttachRec {
                conn: u64::try_from(i).expect("test index fits u64"),
                actor: GOD,
                caps_structured: false,
            })
            .collect();

        assert!(!handle.reattach(oversized).await);
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "an oversized roster must be refused as one unit, never partially enqueued"
        );
    }

    #[tokio::test]
    async fn refused_command_does_not_block_pending_reattach_or_barrier() {
        let engine = boot_engine();
        let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = WorldHandle {
            tx: tx.clone(),
            shutdown: Some(shutdown_tx),
        };
        let (writer, mut writer_rx) = mpsc::channel(2);
        let (staff_tx, _staff_rx) = mpsc::channel(STAFF_OPS_CAPACITY);
        let (_signal_tx, signal_rx) = writer_signal_channel();
        let metrics = Arc::new(Metrics::new());
        let mut deps = ServerDeps::in_memory();
        deps.metrics = Arc::clone(&metrics);
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let world_join = tokio::spawn(run(
            engine,
            rx,
            tx.downgrade(),
            writer,
            deps,
            staff_tx,
            gateway,
            Arc::new(AtomicU64::new(0)),
            signal_rx,
            shutdown_rx,
            PersistenceIntegrityLatch::new(),
            None,
        ));

        assert!(
            handle
                .inbound(Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: "@create receipt-anchor".to_string(),
                })
                .await
        );
        let (serial, receipt) = match tokio::time::timeout(Duration::from_secs(2), writer_rx.recv())
            .await
            .expect("dirty command must enqueue a write")
            .expect("writer channel open")
        {
            StoreJob::Write {
                batch,
                notify: Some(receipt),
            } => (batch.serial, receipt),
            _ => panic!("expected an exact-receipt write"),
        };
        let (baseline_sends, baseline_effects) =
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let snapshot = metrics.snapshot();
                    if snapshot.commit_pending_sends > 0 {
                        break (
                            snapshot.commit_pending_sends,
                            snapshot.commit_pending_effects,
                        );
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("world must publish initial exact-receipt custody");

        // This command is retryable input, not accepted semantic custody. It
        // must receive Busy immediately and leave the FIFO available for the
        // operational messages behind it.
        assert!(
            handle
                .inbound(Inbound::Cmd {
                    conn: 2,
                    actor: GOD,
                    line: "@create must-not-exist".to_string(),
                })
                .await
        );
        for conn in 10_000..11_000_u64 {
            assert!(
                handle
                    .reattach(vec![ReAttachRec {
                        conn,
                        actor: GOD,
                        caps_structured: false,
                    }])
                    .await
            );
        }
        let barrier = tokio::spawn({
            let handle = handle.clone();
            async move { handle.barrier().await }
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = metrics.snapshot();
                if snapshot.commit_pending_barriers == 1
                    && snapshot.commit_pending_reattach_records == 1
                    && snapshot.commit_pending_reattach_coalesced_total == 999
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("operational traffic must remain live behind a refused command");
        assert!(
            !barrier.is_finished(),
            "barrier must still wait for the receipt"
        );
        assert_eq!(metrics.snapshot().commit_pending_sends, baseline_sends);
        assert_eq!(metrics.snapshot().commit_pending_effects, baseline_effects);

        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        assert!(
            tokio::time::timeout(Duration::from_secs(2), barrier)
                .await
                .expect("barrier must complete after receipt")
                .expect("barrier task")
        );
        assert_eq!(handle.name_free("must-not-exist").await, Ok(true));
        assert!(
            recorder
                .sends
                .lock()
                .expect("recording gateway poisoned")
                .iter()
                .any(|(_, event)| matches!(
                    event.as_ref(),
                    OutputEvent::Sys(text)
                        if text.to_plain_text().contains("committing another action")
                )),
            "the refused command must receive a retryable Busy response"
        );
        assert_eq!(metrics.snapshot().commit_pending_barriers, 0);
        assert_eq!(metrics.snapshot().commit_pending_reattach_records, 0);

        assert!(handle.inbound(Inbound::Control(ControlMsg::Shutdown)).await);
        tokio::time::timeout(Duration::from_secs(2), world_join)
            .await
            .expect("world shutdown timed out")
            .expect("world task panicked");
    }

    #[tokio::test]
    async fn authoritative_abort_launches_no_external_effect_even_if_best_effort() {
        let (tx, mut rx) = mpsc::channel(1);
        let weak = tx.downgrade();
        let (_receipt_tx, receipt) = oneshot::channel();
        let pending = PendingCommit {
            token: CommitToken::from_serial(7),
            receipt: PendingReceipt::Legacy {
                expected_serial: 7,
                receiver: receipt,
            },
            sends: Vec::new(),
            effects: vec![(
                DeliveryClass::BestEffort,
                Effect::SlowTest {
                    actor: GOD,
                    correlation: 77,
                    seconds: 0,
                },
            )],
            failure_actor: None,
            create: None,
            prepared_installation: None,
            barriers: Vec::new(),
            coalesced_reattach: None,
            exit_after_ack: false,
        };
        let mut engine = Engine::new(World::new_minimal(), base_registry());
        let gateway: Gateway = Arc::new(BlackholeGateway);
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        complete_pending_commit(
            pending,
            Err(AuthoritativeCommitFailure::Integrity(
                "deterministic rejection".to_owned(),
            )),
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &weak,
            None,
        );
        tokio::task::yield_now().await;
        assert!(health.mode.integrity_frozen());
        assert!(
            persistence_integrity.is_latched(),
            "a rejected exact receipt must publish the process-wide snapshot barrier"
        );
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "an aborted dirty step must not launch even a BestEffort external effect"
        );
    }

    #[test]
    fn integrity_freeze_never_uses_command_text_or_context_allowlist() {
        let frozen = WorldMode::IntegrityFrozen {
            incident: IncidentId(12),
        };
        let mut world = World::new_minimal();
        for line in ["look", "WHO", "say hi", "@shutdown", ""] {
            assert!(matches!(
                admit_world_msg(frozen, &world, &WorldMsg::Inbound(cmd(line))),
                Admission::Refuse(RefusalReason::IntegrityFrozen)
            ));
        }
        world.push_input_context(GOD, None, Box::new(Sink));
        assert!(matches!(
            admit_world_msg(frozen, &world, &WorldMsg::Inbound(cmd("~q"))),
            Admission::Refuse(RefusalReason::IntegrityFrozen)
        ));
    }

    /// The conservative allowlist really is a pin: module verbs — comsys
    /// aliases, `@mail`, `+jobs` — mutate module state (seq bumps, boxes,
    /// cursors) and MUST stay refused while read-only. If a future
    /// allowlist edit lets one through, this fails.
    #[test]
    fn read_only_refuses_module_verbs() {
        let world = World::new_minimal();
        for line in [
            "pub Hello there",
            "@mail/quick Nick=hi/there",
            "@mail",
            "addcom pub=Public",
            "comlist",
            "+request T=text",
            "+job/approve 1",
            "@channel/recall pub",
        ] {
            assert!(
                refuse_store_read_only(&world, &cmd(line)).is_some(),
                "{line:?} must be refused while read-only"
            );
        }
        for line in ["look", "WHO", "say hi", "\"hi", ":waves", "help mail"] {
            assert!(
                refuse_store_read_only(&world, &cmd(line)).is_none(),
                "{line:?} must pass while read-only"
            );
        }
    }

    struct Sink;

    impl InputContext for Sink {
        fn prompt(&self, _w: &World) -> PromptSpec {
            PromptSpec {
                mode: InputMode::Normal,
                text: None,
            }
        }

        fn feed(&mut self, _ctx: &mut Ctx<'_>, _line: &str) -> ContextOutcome {
            ContextOutcome::Consumed
        }
    }

    /// A player inside an input context (mail compose) must never be
    /// trapped by read-only mode: every line — the hard escape `~q`, the
    /// send terminator `--`, and arbitrary body text that happens to spell
    /// a refused verb — passes the gate and feeds the context.
    #[test]
    fn read_only_passes_input_context_lines() {
        let mut world = World::new_minimal();
        world.push_input_context(GOD, None, Box::new(Sink));
        for line in ["~q", "--", "body text", "@create orb", "pub Hello", "l"] {
            assert!(
                refuse_store_read_only(&world, &cmd(line)).is_none(),
                "{line:?} must pass while composing"
            );
        }
        // Popped: the gate applies again.
        world.pop_input_context(GOD);
        assert!(refuse_store_read_only(&world, &cmd("@create orb")).is_some());
    }

    // ------------------------------------------------------------------
    // M11 per-observer projection substrate (D1/D2/D3)
    // ------------------------------------------------------------------

    /// A dummy `Perceive` hook that conceals exactly one `(observer, subject)`
    /// pair — the test double proving the game-agnostic projection substrate
    /// works before any ruleset verdict (Obfuscate/Auspex) exists.
    struct CloakPair {
        observer: ObjId,
        subject: ObjId,
    }

    impl mush_core::module::EventHook for CloakPair {
        fn perceives(
            &self,
            _w: &World,
            observer: ObjId,
            subject: ObjId,
        ) -> mush_core::module::Perceive {
            if observer == self.observer && subject == self.subject {
                mush_core::module::Perceive::Conceal
            } else {
                mush_core::module::Perceive::Abstain
            }
        }
    }

    /// The plain text of the `Contents:` line `examine` sent to `actor`
    /// (empty when `examine` emitted none — i.e. every occupant was hidden).
    fn examine_contents(out: &Outbox, actor: ObjId) -> String {
        out.events()
            .iter()
            .find_map(|(aud, ev)| match (aud, ev) {
                (Audience::Actor(a), OutputEvent::Sys(t)) if *a == actor => {
                    let s = t.to_plain_text();
                    s.starts_with("Contents:").then_some(s)
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// With a `Perceive` hook concealing (Alice, Wraith): the subject is
    /// absent from Alice's `room_view`, `examine`, and the resolved
    /// `Audience::Room` fan-out — while Bob, a different observer, perceives
    /// Wraith through every one of those paths.
    #[test]
    fn perceive_substrate_gates_look_examine_and_fanout() {
        use mush_core::module::{HookPoint, Registrar};
        use mush_core::{dispatch, obj_ref, register_core_commands, room_view};
        use mush_proto::{NarrativeKind, RichText};

        let mut world = World::new_minimal();
        let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let alice = world
            .create(ObjKind::Player, "Alice", GOD, Some(hall))
            .unwrap();
        let bob = world
            .create(ObjKind::Player, "Bob", GOD, Some(hall))
            .unwrap();
        let wraith = world
            .create(ObjKind::Player, "Wraith", GOD, Some(hall))
            .unwrap();
        for p in [alice, bob, wraith] {
            world.set_connected(p, true);
        }
        // Alice and Bob must be able to `examine` the room (Wizard).
        world.set_flag(alice, Flags::WIZARD, true).unwrap();
        world.set_flag(bob, Flags::WIZARD, true).unwrap();

        let mut reg = Registry::new();
        register_core_commands(&mut reg).unwrap();
        Registrar::new(&mut reg, "cloaktest").hook(
            HookPoint::Perceive,
            Arc::new(CloakPair {
                observer: alice,
                subject: wraith,
            }),
        );

        // D1: the gate itself. Only the one concealed pair fails.
        assert!(!can_perceive(&reg, &world, alice, wraith));
        assert!(can_perceive(&reg, &world, bob, wraith));
        assert!(can_perceive(&reg, &world, alice, bob));
        assert!(can_perceive(&reg, &world, wraith, wraith));

        // D2(a): room_view drops the concealed occupant for Alice only.
        let names = |looker| -> Vec<String> {
            room_view(&reg, &world, looker, hall)
                .unwrap()
                .contents
                .into_iter()
                .map(|r| r.name)
                .collect()
        };
        let alice_sees = names(alice);
        assert!(
            !alice_sees.contains(&"Wraith".to_string()),
            "look leaked Wraith to Alice: {alice_sees:?}"
        );
        assert!(alice_sees.contains(&"Bob".to_string()));
        assert!(
            names(bob).contains(&"Wraith".to_string()),
            "Bob must still perceive Wraith by look"
        );

        // D2(c): examine dumps raw contents — it too must drop Wraith for
        // Alice while still listing him for Bob.
        let mut out = Outbox::new();
        dispatch(
            &reg,
            &mut world,
            alice,
            &format!("examine {hall}"),
            &mut out,
        );
        let alice_ex = examine_contents(&out, alice);
        assert!(
            !alice_ex.contains("Wraith"),
            "examine leaked Wraith to Alice: {alice_ex:?}"
        );
        assert!(
            alice_ex.contains("Bob"),
            "examine still lists Bob: {alice_ex:?}"
        );
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, bob, &format!("examine {hall}"), &mut out);
        assert!(
            examine_contents(&out, bob).contains("Wraith"),
            "Bob's examine must still show Wraith"
        );

        // D2(b)+D3: the live Room fan-out. Wraith speaks; the event's subject
        // is Wraith, so Alice (who cannot perceive him) is dropped from the
        // resolved recipients while Bob and Wraith himself receive it.
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, wraith, "say boo", &mut out);
        let sends = resolve_sends(&reg, &world, &mut out);
        let recipients: std::collections::HashSet<ObjId> = sends
            .iter()
            .filter_map(|(t, _)| match t {
                SendTarget::Char(ck) => Some(ObjId(u32::try_from(*ck).unwrap_or(u32::MAX))),
                _ => None,
            })
            .collect();
        assert!(
            !recipients.contains(&alice),
            "cloaked Wraith's speech must not reach Alice: {recipients:?}"
        );
        assert!(recipients.contains(&bob), "Bob overhears Wraith");
        assert!(recipients.contains(&wraith), "Wraith hears his own speech");

        // D3: event_subject extracts the attributed actor and abstains on
        // events that name no one.
        assert_eq!(
            event_subject(&OutputEvent::Narrative {
                kind: NarrativeKind::Say,
                actor: Some(obj_ref(&world, wraith)),
                text: RichText::plain("hi"),
            }),
            Some(wraith)
        );
        assert_eq!(
            event_subject(&OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: None,
                text: RichText::plain("mist swirls"),
            }),
            None
        );
        assert_eq!(
            event_subject(&OutputEvent::Sys(RichText::plain("weather"))),
            None
        );
    }

    /// Invariant I1: with NO `Perceive` hook registered the fan-out is
    /// byte-for-byte the old behavior — every connected, non-excluded,
    /// non-ignoring occupant receives a room event, subject or not.
    #[test]
    fn perceive_substrate_is_a_free_pass_without_a_hook() {
        use mush_core::{dispatch, register_core_commands, room_view};

        let mut world = World::new_minimal();
        let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let a = world
            .create(ObjKind::Player, "Ann", GOD, Some(hall))
            .unwrap();
        let b = world
            .create(ObjKind::Player, "Bee", GOD, Some(hall))
            .unwrap();
        for p in [a, b] {
            world.set_connected(p, true);
        }
        let mut reg = Registry::new();
        register_core_commands(&mut reg).unwrap();

        // Everyone is perceivable; look lists the other occupant.
        assert!(can_perceive(&reg, &world, a, b));
        assert!(
            room_view(&reg, &world, a, hall)
                .unwrap()
                .contents
                .iter()
                .any(|r| r.name == "Bee")
        );

        // A say reaches every connected occupant (speaker included).
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, a, "say hi", &mut out);
        let recipients: std::collections::HashSet<ObjId> = resolve_sends(&reg, &world, &mut out)
            .iter()
            .filter_map(|(t, _)| match t {
                SendTarget::Char(ck) => Some(ObjId(u32::try_from(*ck).unwrap_or(u32::MAX))),
                _ => None,
            })
            .collect();
        assert!(recipients.contains(&a) && recipients.contains(&b));
    }
}
